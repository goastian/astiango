#!/usr/bin/env python3
"""Continuous exact graph ingestion; legacy services stay up during the upgrade."""
import argparse
import csv
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("helpers", ROOT / "scripts/indexer-local.py")
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)


def configuration(path):
    raw = tomllib.loads(path.read_text())
    expected = {"source_spool", "state_directory", "binary", "poll_seconds", "scan_seconds", "max_pending", "max_input_bytes", "memory_guard_bytes", "job_timeout_seconds", "publication_seconds", "graph", "consumer"}
    if set(raw) != expected:
        raise ValueError("Invalid graph ingestion configuration keys")
    for key in expected - {"source_spool", "state_directory", "binary", "graph", "consumer"}:
        if type(raw[key]) is not int or raw[key] <= 0:
            raise ValueError(f"{key} must be a positive integer")
    if raw["poll_seconds"] > 5 or raw["max_pending"] > 64:
        raise ValueError("Graph observation/queue exceeds budget")
    for key in ["source_spool", "state_directory", "binary"]:
        raw[key] = (ROOT / raw[key]).resolve()
        if not raw[key].is_relative_to(ROOT):
            raise ValueError("Graph paths must stay in this workspace")
    if raw["state_directory"].is_relative_to(raw["source_spool"]) or raw["source_spool"].is_relative_to(raw["state_directory"]):
        raise ValueError("Graph output and capture archive must be separate")
    if raw["consumer"].get("workers") != 1:
        raise ValueError("Continuous graph profile requires one bounded worker")
    return raw


class Service:
    def __init__(self, config_path):
        self.path = config_path
        self.config = configuration(config_path)
        self.signature = config_path.stat().st_mtime_ns
        self.directory = self.config["state_directory"]
        self.directory.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.directory / "jobs.sqlite")
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.execute("PRAGMA cache_size=-2048")
        self.db.execute("CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,file TEXT,sha256 TEXT,records INTEGER,input_bytes INTEGER,sealed_at INTEGER,state TEXT,error TEXT,attempts INTEGER NOT NULL DEFAULT 0,next_at REAL NOT NULL DEFAULT 0,elapsed_seconds REAL NOT NULL DEFAULT 0,edges INTEGER NOT NULL DEFAULT 0)")
        self.db.execute("UPDATE jobs SET state='pending' WHERE state='running'")
        self.db.commit()
        self.binary_sha = h.checksum(self.config["binary"])
        self.child = self.current = self.log = None
        self.draining = False
        self.started_at, self.run_id = h.timestamp(), str(uuid.uuid4())
        self.phase, self.reason = "idle", "waiting_for_segments"
        self.last_scan = self.last_publication = self.last_projection = 0
        self.source = {"sealed": 0, "open": 0, "oversized": 0, "scan_errors": 0}
        self.runtime, self.history, self.events = {}, [], []
        self.interval_rss = 0
        self.generation = None
        self.publication_error = None
        self.last_attempt = 0
        self.projected_edges = 0
        self.projected_revision = 0
        self.output_bytes = 0
        self.latest_input = None
        progress=self.directory / "native-progress.json"
        if progress.exists():
            try:self.runtime=h.limited_json(progress,131072)
            except (OSError,ValueError):pass
        selector=self.directory / "graph/graph-selector.json"
        if selector.exists():
            try:
                selected=h.limited_json(selector)
                self.generation=h.limited_json(self.directory / "graph/generations" / selected["active"] / "manifest.json",1048576)
                self.projected_revision=self.generation.get("revision",0)
                self.projected_edges=self.generation["edges"]
            except (OSError,ValueError,KeyError):pass

    def event(self, kind, message):
        self.events.append({"at": h.timestamp(), "kind": kind, "message": str(message)[:512]})
        self.events = self.events[-16:]

    def reload(self):
        if self.child:
            return
        try:
            mtime = self.path.stat().st_mtime_ns
            if mtime == self.signature:
                return
            candidate = configuration(self.path)
            if any(candidate[key] != self.config[key] for key in ["source_spool", "state_directory"]):
                raise ValueError("Live reload cannot move source/ledger paths")
            if candidate["binary"] != self.config["binary"]:
                subprocess.run([str(candidate["binary"]), "--build-info"], check=True, timeout=5, stdout=subprocess.DEVNULL)
            self.binary_sha = h.checksum(candidate["binary"])
            self.config, self.signature = candidate, mtime
            self.event("configuration", "Graph configuration applied at job boundary")
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            self.event("error", error)

    def scan(self):
        counts = {"sealed": 0, "open": 0, "oversized": 0, "scan_errors": 0}
        candidates = []
        folder = self.config["source_spool"]
        if not (folder / "state").is_dir():
            raise OSError("Input spool unavailable")
        for i, path in enumerate((folder / "state").glob("*.json")):
            if i >= 4096:
                raise ValueError("Input descriptor scan exceeds budget")
            try:
                row = h.limited_json(path)
                identity = str(uuid.UUID(row["id"]))
                if row["schema_version"] not in [1, 2] or row["file"] != f"{identity}.warc.gz" or path.name != f"{identity}.json":
                    raise ValueError("Invalid input identity")
                if row["state"] == "open":
                    counts["open"] += 1
                    continue
                if row["state"] not in ["local", "pending", "published"] or not row.get("sealed_at") or not row.get("records"):
                    continue
                counts["sealed"] += 1
                if row["bytes"] > self.config["max_input_bytes"]:
                    counts["oversized"] += 1
                    continue
                file = folder / "segments" / row["file"]
                if file.is_symlink() or file.stat().st_size != row["bytes"] or len(row["sha256"]) != 64:
                    raise ValueError("Input checksum descriptor mismatch")
                candidates.append(row)
            except (OSError, ValueError, KeyError, TypeError):
                counts["scan_errors"] += 1
        used = self.db.execute("SELECT count(*) FROM jobs WHERE state IN ('pending','running','deferred')").fetchone()[0]
        for row in sorted(candidates, key=lambda r: (r["sealed_at"], r["id"])):
            old = self.db.execute("SELECT sha256 FROM jobs WHERE id=?", (row["id"],)).fetchone()
            if old:
                if old[0] != row["sha256"]:
                    raise ValueError("Previously admitted immutable capture changed")
                continue
            if used >= self.config["max_pending"]:
                break
            self.db.execute("INSERT INTO jobs(id,file,sha256,records,input_bytes,sealed_at,state) VALUES(?,?,?,?,?,?,'pending')", (row["id"], row["file"], row["sha256"], row["records"], row["bytes"], row["sealed_at"]))
            used += 1
        self.db.commit()
        self.source = counts
        self.output_bytes = h.file_bytes(self.directory)
        self.last_scan = time.monotonic()

    def start_job(self, row=None):
        c = self.config
        graph = {**c["graph"], "output_path": str(self.directory / "graph"), "source_directory": str(c["source_spool"] / "segments"), "files": [row["file"]] if row else [], "publish_generations": False, "report_path": str(self.directory / "native-progress.json")}
        path = self.directory / "current-job.toml"
        text = "\n".join(f"{k}={json.dumps(v)}" for k, v in graph.items())
        text += "\n[consumer]\n" + "\n".join(f"{k}={v}" for k, v in c["consumer"].items())
        path.write_text(text + "\n")
        if row and h.checksum(c["source_spool"] / "segments" / row["file"]) != row["sha256"]:
            self.db.execute("UPDATE jobs SET state='quarantined',error=? WHERE id=?",("Input checksum mismatch; source retained",row["id"]))
            self.db.commit()
            raise ValueError("Input checksum mismatch; source retained")
        logs = self.directory / "logs"
        logs.mkdir(exist_ok=True)
        self.log_path = logs / f"{row['id'] if row else 'generation'}.log"
        self.log = self.log_path.open("ab")
        argv = [str(c["binary"]), str(path)] + ([] if row else ["--prepare-generation"])
        self.child = subprocess.Popen(argv, cwd=ROOT, env={**os.environ, "RUST_LOG": "error", "RAYON_NUM_THREADS": "1"}, stdout=self.log, stderr=self.log, stdin=subprocess.DEVNULL, preexec_fn=h.restricted_child)
        self.current = dict(row) if row else None
        self.job_started = time.monotonic()
        self.peak = 0
        self.child_error = None
        self.phase, self.reason = ("extracting" if row else "publishing"), "processing_segment"
        if row:
            self.db.execute("UPDATE jobs SET state='running',error=NULL WHERE id=?", (row["id"],))
            self.db.commit()
        else:
            self.last_attempt = time.monotonic()
        self.event("started", row["file"] if row else "Graph generation preparation")

    def admit(self):
        policy = self.config["graph"]
        if self.output_bytes >= policy["max_output_bytes"]:
            self.phase, self.reason = "blocked", "output_quota"
            return
        disk = os.statvfs(self.directory)
        if disk.f_bavail * disk.f_frsize < policy["disk_reserve_bytes"]:
            self.phase, self.reason = "blocked", "disk_reserve"
            return
        pending = self.db.execute("SELECT * FROM jobs WHERE state='pending' OR (state='deferred' AND next_at<=? AND attempts<3) ORDER BY sealed_at,id LIMIT 1", (time.time(),)).fetchone()
        edges = self.runtime.get("current_edges", 0)
        due = time.monotonic()-self.last_attempt >= self.config["publication_seconds"]
        if self.runtime.get("revision", 0) != self.projected_revision and due and (not self.publication_error or time.monotonic()-self.last_attempt >= 60):
            self.start_job()
        elif pending:
            self.start_job(pending)
        else:
            self.phase, self.reason = "idle", "waiting_for_segments"

    def poll(self):
        sample = h.process_sample(self.child.pid)
        if sample:
            self.peak = max(self.peak, sample["rss_bytes"])
            self.interval_rss = max(self.interval_rss, sample["rss_bytes"])
        if self.current:
            try:
                self.runtime = h.limited_json(self.directory / "native-progress.json", 131072)
            except (OSError, ValueError):
                pass
        if self.child.poll() is None and self.child_error is None:
            if self.peak > self.config["memory_guard_bytes"]:
                self.child_error = "Memory guard reached; graph ledger/input retained"
            elif time.monotonic()-self.job_started > self.config["job_timeout_seconds"]:
                self.child_error = "Graph job deadline reached; ledger/input retained"
            if self.child_error:
                self.child.terminate()
        code = self.child.poll()
        if code is None:
            return
        self.log.close()
        with self.log_path.open("rb") as source:
            source.seek(max(0, self.log_path.stat().st_size-262144))
            lines = source.read().decode(errors="replace").splitlines()
        result = None
        for line in reversed(lines):
            try:
                result = json.loads(line)
                break
            except ValueError:
                pass
        error = self.child_error or ("\n".join(lines)[-1024:] if code else None)
        if self.current:
            row = self.current
            valid = code == 0 and error is None and result and result.get("source_complete") and result.get("source_records") == row["records"]
            state = "completed" if valid else "quarantined" if "quarantined" in str(error) else "deferred"
            if valid:
                self.runtime = result
                self.latest_input = row["id"]
            else:
                error = error or "Native source/cursor did not reconcile; retained for recovery"
            self.db.execute("UPDATE jobs SET state=?,error=?,attempts=attempts+?,next_at=?,elapsed_seconds=elapsed_seconds+?,edges=? WHERE id=?", (state, error, int(not valid), time.time()+120, time.monotonic()-self.job_started, self.runtime.get("current_edges", 0), row["id"]))
            self.db.commit()
        else:
            if code == 0 and error is None and result and result.get("state") == "ready":
                self.generation = result
                self.projected_edges = result["edges"]
                self.projected_revision = result.get("revision", 0)
                self.last_publication = time.monotonic()
                self.publication_error = None
            else:
                self.publication_error = error or "Partial generation is not ready"
        self.event("completed" if error is None else "error", error or ("Source confirmed" if self.current else "Generation ready"))
        self.child = self.current = None
        self.phase, self.reason = "idle", "waiting_for_segments"

    def snapshot(self):
        aggregate = self.db.execute("SELECT count(*),sum(state='completed'),sum(state IN ('pending','running','deferred')),sum(state='quarantined'),sum(CASE WHEN state='completed' THEN records ELSE 0 END) FROM jobs").fetchone()
        rows = []
        for row in self.db.execute("SELECT * FROM jobs ORDER BY sealed_at DESC,id LIMIT 64"):
            row = dict(row)
            row.update(score_state="generation", scores=0, peak_rss_bytes=0, state="failed" if row["state"] == "quarantined" else row["state"])
            rows.append(row)
        generation = self.generation
        selector = self.directory / "graph/graph-selector.json"
        if selector.exists():
            try:
                selected = h.limited_json(selector)
                generation = h.limited_json(self.directory / "graph/generations" / selected["active"] / "manifest.json", 1048576)
                self.generation = generation
            except (OSError, ValueError, KeyError):
                pass
        if generation:
            generation={key:generation[key] for key in ["version","revision","id","state","logical_sha256","documents","edges","partitions","reused_buckets","rebuilt_buckets","elapsed_ms","node_identity","exact_identity","scores_state","build_id"] if key in generation}
        sample = h.process_sample(self.child.pid) if self.child else None
        worker = {"pid": self.child.pid, "input": self.current["id"] if self.current else "generation", "stage": self.phase, "elapsed_seconds": time.monotonic()-self.job_started, **(sample or {})} if self.child else None
        totals = dict(zip(["inputs", "completed", "pending", "failed", "source_records"], (v or 0 for v in aggregate)))
        totals.update(stored_edges=self.runtime.get("current_edges", 0), scored_inputs=int(bool(generation and generation.get("scores_state") == "calculated")), output_bytes=self.output_bytes)
        config = {"minimum_disk_free_bytes": self.config["graph"]["disk_reserve_bytes"], **self.config["graph"], "workers": 1, "priority": 10, "memory_guard_bytes": self.config["memory_guard_bytes"], "max_input_bytes": self.config["max_input_bytes"], "max_pending": self.config["max_pending"], "binary_sha256": self.binary_sha, "scope": "current_version_graph"}
        top_scores=[]
        if generation and generation.get("scores_state")=="calculated":
            score_path=self.directory / "graph/generations" / generation["id"] / "scores/harmonic.csv"
            try:
                with score_path.open() as input:
                    for row in csv.reader(input):
                        top_scores.append({"host":row[0],"score":float(row[1])})
                        if len(top_scores)>=10:break
            except (OSError,ValueError,IndexError):pass
        snapshot = {"schema_version": 1, "component": "webgraph", "run_id": self.run_id, "pid": os.getpid(), "start_ticks": h.process_sample(os.getpid())["start_ticks"], "started_at": self.started_at, "updated_at": h.timestamp(), "phase": self.phase, "reason": self.reason, "source": self.source, "configuration": config, "totals": totals, "worker": worker, "inputs": rows, "history": self.history, "runtime": self.runtime, "generation": generation, "publication_error": self.publication_error, "latest_input": self.latest_input, "top_scores": top_scores, "scope": "current_version_graph", "signals_adopted": False}
        return {"snapshot": snapshot, "meta": {"read_at": snapshot["updated_at"], "process_alive": True, "source": "versioned_graph_ledger"}}

    def run(self):
        signal.signal(signal.SIGTERM, lambda *_: setattr(self, "draining", True))
        signal.signal(signal.SIGINT, lambda *_: setattr(self, "draining", True))
        while not self.draining or self.child:
            self.reload()
            now = time.monotonic()
            if now-self.last_scan >= self.config["scan_seconds"]:
                try:
                    self.scan()
                except (OSError, ValueError) as error:
                    self.db.rollback()
                    self.phase, self.reason = "blocked", "source_unavailable"
                    self.last_scan = now
                    self.event("error", error)
            if self.child:
                self.poll()
            elif not self.draining and self.reason != "source_unavailable":
                try:
                    self.admit()
                except (OSError, ValueError, subprocess.SubprocessError) as error:
                    self.phase, self.reason = "blocked", "index_pressure"
                    self.event("error", error)
            if now-self.last_projection >= self.config["poll_seconds"]:
                response = self.snapshot()
                s = response["snapshot"]
                self.history.append({"at": s["updated_at"], "edges": s["totals"]["stored_edges"], "rss_mib": max(self.interval_rss, (s["worker"] or {}).get("rss_bytes", 0))/1048576, "completed": s["totals"]["completed"]})
                self.history = self.history[-300:]
                s["history"] = self.history
                self.interval_rss = 0
                for destination in [self.directory / "status.json", ROOT / "tools/crawler-dashboard/dist/webgraph-live.json"]:
                    try:
                        h.atomic_json(destination, response, durable=False)
                    except OSError as error:
                        print(f"Graph observation unavailable: {error}", flush=True)
                self.last_projection = now
                processes=self.directory / "processes.json"
                try:
                    if processes.exists():
                        ownership=h.limited_json(processes)
                        if ownership.get("pid")!=os.getpid():
                            ownership.update(pid=os.getpid(),start_ticks=h.process_sample(os.getpid())["start_ticks"])
                            h.atomic_json(processes,ownership)
                except (OSError,ValueError):pass
            time.sleep(.1 if self.child else .5)
        self.db.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "run", "status"])
    parser.add_argument("--config", type=Path, default=ROOT / "configs/webgraph/ingestion-local.toml")
    args = parser.parse_args()
    config = configuration(args.config.resolve())
    directory = config["state_directory"]
    directory.mkdir(parents=True, exist_ok=True)
    if args.action == "run":
        with (directory / "worker.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            Service(args.config.resolve()).run()
        return
    runner = h.runner_module()
    manifest = directory / "processes.json"
    record = h.limited_json(manifest) if manifest.exists() else {}
    if args.action == "start":
        if runner.owned_process(record):
            print(f"Graph ingestion already active (PID {record['pid']})")
            return
        record = runner.launch("webgraph-ingestion", [sys.executable, str(Path(__file__).resolve()), "run", "--config", str(args.config.resolve())], "systemd" if runner.systemd_available() else "detached", {})
        h.atomic_json(manifest, record)
        print(f"Graph ingestion started (PID {record['pid']}); existing services unchanged")
    else:
        print(json.dumps({"active": runner.owned_process(record), "pid": record.get("pid")}))


if __name__ == "__main__":
    main()
