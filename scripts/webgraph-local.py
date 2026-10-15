#!/usr/bin/env python3
"""Observe native per-input webgraphs and bounded centrality without changing live signals."""
import argparse
import csv
import fcntl
import hashlib
import importlib.util
import json
import math
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
spec = importlib.util.spec_from_file_location("indexing_helpers", ROOT / "scripts/indexer-local.py")
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)


def configuration(path):
    raw = tomllib.loads(path.read_text())
    keys = {"source_spool", "state_directory", "binary", "poll_seconds", "scan_seconds", "admission_seconds", "max_pending", "max_input_bytes", "max_output_bytes", "minimum_disk_free_bytes", "memory_guard_bytes", "job_timeout_seconds", "max_centrality_edges", "consumer"}
    if set(raw) != keys:
        raise ValueError("Invalid webgraph pilot configuration keys")
    for key in keys - {"source_spool", "state_directory", "binary", "consumer"}:
        if type(raw[key]) is not int or raw[key] <= 0:
            raise ValueError(f"{key} must be a positive integer")
    if raw["poll_seconds"] > 5 or raw["scan_seconds"] < raw["poll_seconds"] or raw["max_pending"] > 64:
        raise ValueError("Invalid observation/queue budget")
    consumer = raw["consumer"]
    if set(consumer) != {"record_bytes", "decoded_bytes", "header_bytes", "batch_bytes", "workers", "stack_bytes"} or any(type(v) is not int or v <= 0 for v in consumer.values()):
        raise ValueError("Invalid consumer budget")
    if consumer["workers"] != 1 or not 15 * 1024 * 1024 <= consumer["batch_bytes"] <= 64 * 1024 * 1024 or consumer["decoded_bytes"] > consumer["batch_bytes"]:
        raise ValueError("Pilot requires one writer, 15..64 MiB batch, decoded <= batch")
    for key in ["source_spool", "state_directory", "binary"]:
        raw[key] = (ROOT / raw[key]).resolve()
        if not raw[key].is_relative_to(ROOT):
            raise ValueError(f"{key} must remain in this workspace")
    if raw["state_directory"].is_relative_to(raw["source_spool"]) or raw["source_spool"].is_relative_to(raw["state_directory"]):
        raise ValueError("Graph output and input spool must be separate")
    if not raw["binary"].is_file():
        raise ValueError("Native graph binary missing")
    return raw


def restrict():
    helpers.restricted_child()
    os.nice(5)


class Pilot:
    def __init__(self, config):
        self.config, self.directory = config, config["state_directory"]
        self.directory.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.directory / "jobs.sqlite")
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.execute("PRAGMA cache_size=-2048")
        self.db.execute("CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,file TEXT NOT NULL,sha256 TEXT NOT NULL,records INTEGER NOT NULL,input_bytes INTEGER NOT NULL,sealed_at INTEGER NOT NULL,state TEXT NOT NULL,edges INTEGER NOT NULL DEFAULT 0,scores INTEGER NOT NULL DEFAULT 0,score_state TEXT NOT NULL DEFAULT 'pending',elapsed_seconds REAL NOT NULL DEFAULT 0,peak_rss_bytes INTEGER NOT NULL DEFAULT 0,error TEXT,completed_at TEXT)")
        self.db.execute("UPDATE jobs SET state='pending' WHERE state='running'")
        self.db.execute("UPDATE jobs SET state='graph_committed' WHERE state='calculating'")
        self.db.commit()
        self.child = self.current = self.log = None
        self.draining = False
        self.run_id, self.started_at = str(uuid.uuid4()), helpers.timestamp()
        self.binary_sha = helpers.checksum(config["binary"])
        self.phase, self.reason = "idle", "waiting_for_segments"
        self.next_job = self.last_scan = self.output_bytes = 0
        self.source = {"sealed": 0, "open": 0, "oversized": 0, "scan_errors": 0}
        self.history, self.events, self.top_scores = [], [], []
        self.interval_rss = 0
        self.scored_input = None
        self.latest_input = None
        try:
            saved = helpers.limited_json(self.directory / "status.json", 262144)
            self.latest_input, self.top_scores = saved.get("latest_input"), saved.get("top_scores", [])
            self.scored_input = saved.get("scored_input")
            self.history = saved.get("history", [])[-300:]
        except (OSError, ValueError):
            pass

    def event(self, kind, message):
        self.events.append({"at": helpers.timestamp(), "kind": kind, "message": str(message)[:512]})
        self.events = self.events[-16:]

    def scan(self):
        counts = {"sealed": 0, "open": 0, "oversized": 0, "scan_errors": 0}
        candidates = []
        if not (self.config["source_spool"] / "state").is_dir():
            raise OSError("Input spool unavailable")
        for i, path in enumerate((self.config["source_spool"] / "state").glob("*.json")):
            if i >= 4096:
                raise ValueError("Spool scan exceeds 4096 descriptors")
            try:
                item = helpers.limited_json(path)
                identity = str(uuid.UUID(item["id"]))
                if item["schema_version"] not in [1, 2] or item["file"] != f"{identity}.warc.gz" or path.name != f"{identity}.json":
                    raise ValueError("Invalid closed segment descriptor")
                if item["state"] == "open":
                    counts["open"] += 1
                    continue
                if item["state"] not in ["local", "pending", "published"] or not item.get("sealed_at") or not item.get("records"):
                    continue
                counts["sealed"] += 1
                if item["bytes"] > self.config["max_input_bytes"]:
                    counts["oversized"] += 1
                    continue
                file = self.config["source_spool"] / "segments" / item["file"]
                if file.is_symlink() or file.stat().st_size != item["bytes"] or len(item["sha256"]) != 64:
                    raise ValueError("Source/descriptor mismatch")
                candidates.append(item)
            except (OSError, ValueError, KeyError, TypeError):
                counts["scan_errors"] += 1
        used = self.db.execute("SELECT count(*) FROM jobs WHERE state IN ('pending','running','graph_committed','calculating')").fetchone()[0]
        for item in sorted(candidates, key=lambda x: (x["bytes"], x["sealed_at"], x["id"])):
            known = self.db.execute("SELECT sha256 FROM jobs WHERE id=?", (item["id"],)).fetchone()
            if known:
                if known[0] != item["sha256"]:
                    raise ValueError("Previously admitted immutable input changed")
                continue
            if used >= self.config["max_pending"]:
                break
            self.db.execute("INSERT INTO jobs(id,file,sha256,records,input_bytes,sealed_at,state) VALUES(?,?,?,?,?,?,'pending')", (item["id"], item["file"], item["sha256"], item["records"], item["bytes"], item["sealed_at"]))
            used += 1
        self.db.commit()
        self.source = counts
        self.output_bytes = helpers.file_bytes(self.directory)
        self.last_scan = time.monotonic()

    def admit(self):
        if self.output_bytes >= self.config["max_output_bytes"]:
            self.phase, self.reason = "blocked", "output_quota"
            return
        disk = os.statvfs(self.directory)
        if disk.f_bavail * disk.f_frsize < self.config["minimum_disk_free_bytes"]:
            self.phase, self.reason = "blocked", "disk_reserve"
            return
        row = self.db.execute("SELECT * FROM jobs WHERE state IN ('pending','graph_committed') ORDER BY state='graph_committed' DESC,input_bytes,sealed_at,id LIMIT 1").fetchone()
        if not row:
            self.phase, self.reason = "idle", "waiting_for_segments"
            return
        folder = self.directory / "inputs" / row["id"]
        folder.mkdir(parents=True, exist_ok=True)
        graph = folder / "graph"
        if row["state"] == "pending":
            source = self.config["source_spool"] / "segments" / row["file"]
            if helpers.checksum(source) != row["sha256"]:
                self.db.execute("UPDATE jobs SET state='failed',error='Source checksum mismatch; input retained' WHERE id=?", (row["id"],))
                self.db.commit()
                return
            native = {"graph_base_path": str(graph), "shard": 0, "host_centrality_store_path": str(self.directory / "unranked/harmonic"), "host_rank_store_path": str(self.directory / "unranked/harmonic_rank"), "limit_warc_files": 1, "merge_all_segments": False}
            text = "\n".join(f"{k}={json.dumps(v)}" for k, v in native.items())
            text += '\n[warc_source]\ntype="Local"\nfolder=' + json.dumps(str(source.parent)) + '\nnames=' + json.dumps([row["file"]])
            text += '\n[consumer]\n' + "\n".join(f"{k}={v}" for k, v in self.config["consumer"].items())
            config_path = folder / "native.toml"
            config_path.write_text(text + "\n")
            argv, self.stage = [str(self.config["binary"]), "webgraph", "create", str(config_path)], "extracting"
            state = "running"
        else:
            # Fresh attempt avoids treating partial scores as a confirmed version.
            self.score_directory = folder / f"scores-{uuid.uuid4()}"
            argv, self.stage = [str(self.config["binary"]), "centrality", "harmonic", str(graph), str(self.score_directory)], "centrality"
            state = "calculating"
        self.log_path = folder / f"{self.stage}.log"
        self.log = self.log_path.open("ab")
        self.child = subprocess.Popen(argv, cwd=ROOT, env={**os.environ, "RUST_LOG": "astiango=info", "RAYON_NUM_THREADS": "1"}, stdout=self.log, stderr=self.log, stdin=subprocess.DEVNULL, preexec_fn=restrict)
        self.current, self.child_started, self.peak, self.child_error = dict(row), time.monotonic(), 0, None
        self.db.execute("UPDATE jobs SET state=?,error=NULL WHERE id=?", (state, row["id"]))
        self.db.commit()
        self.phase, self.reason = self.stage, "processing_segment"
        self.event("started", f"{self.stage}: {row['file']}")

    def poll(self):
        sample = helpers.process_sample(self.child.pid)
        if sample:
            self.peak = max(self.peak, sample["rss_bytes"])
            self.interval_rss = max(self.interval_rss, sample["rss_bytes"])
        if self.child.poll() is None and self.child_error is None:
            if self.peak > self.config["memory_guard_bytes"]:
                self.child_error = "Memory guard exceeded; input/checkpoints retained"
            elif time.monotonic()-self.child_started > self.config["job_timeout_seconds"]:
                self.child_error = "Stage deadline exceeded; input/checkpoints retained"
            elif self.output_bytes >= self.config["max_output_bytes"]:
                self.child_error = "Output quota reached; input/checkpoints retained"
            if self.child_error:
                self.child.terminate()
        code = self.child.poll()
        if code is None:
            return
        self.log.close()
        row = self.current
        folder = self.directory / "inputs" / row["id"]
        state, error = "failed", self.child_error
        edges, scores, score_state = row["edges"], row["scores"], row["score_state"]
        try:
            if code != 0 or error:
                with self.log_path.open("rb") as source:
                    source.seek(max(0, self.log_path.stat().st_size-1024))
                    detail = source.read().decode(errors="replace")
                raise ValueError(error or f"Native exit {code}: {detail[-512:]}")
            if self.stage == "extracting":
                meta = helpers.limited_json(folder / "graph/edges/meta.json", 2097152)
                edges = sum(v["max_doc"] for v in meta["segments"])
                identity = hashlib.sha256(f"local:{self.config['source_spool'] / 'segments'}:{row['file']}".encode()).hexdigest()
                cursor = helpers.limited_json(folder / "graph/consumer-checkpoints" / f"{identity}.json")
                if not cursor["complete"] or cursor["fingerprint"] != row["sha256"] or cursor["records_committed"] != row["records"]:
                    raise ValueError("Graph checkpoint/descriptor does not reconcile")
                score_state = "empty" if edges == 0 else "budget" if edges > self.config["max_centrality_edges"] else "pending"
                state = "graph_committed" if score_state == "pending" else "completed"
            else:
                best = []
                with (self.score_directory / "harmonic.csv").open(newline="") as source:
                    for host, value in csv.reader(source):
                        value = float(value)
                        if not math.isfinite(value):
                            raise ValueError("Non-finite centrality score")
                        scores += 1
                        best.append({"host": host[:253], "score": value})
                        best = sorted(best, key=lambda v: (-v["score"], v["host"]))[:10]
                helpers.atomic_json(folder / "scores.json", {"version": 1, "input": row["id"], "source_sha256": row["sha256"], "binary_sha256": self.binary_sha, "algorithm": "harmonic-hll64", "scope": "single_input", "path": str(self.score_directory), "count": scores, "adopted": False})
                self.top_scores = best
                self.scored_input = row["id"]
                state, score_state = "completed", "calculated"
            self.latest_input = row["id"]
        except (OSError, ValueError, KeyError, TypeError) as cause:
            error = str(cause)[:512]
        self.db.execute("UPDATE jobs SET state=?,edges=?,scores=?,score_state=?,elapsed_seconds=elapsed_seconds+?,peak_rss_bytes=max(peak_rss_bytes,?),error=?,completed_at=? WHERE id=?", (state, edges, scores, score_state, time.monotonic()-self.child_started, self.peak, error, helpers.timestamp(), row["id"]))
        self.db.commit()
        self.event(state, error or f"{row['file']}: {edges} stored edges; scores {score_state}")
        self.child = self.current = None
        self.next_job = time.monotonic()+self.config["admission_seconds"]
        self.phase, self.reason = "idle", "admission_interval"

    def snapshot(self):
        rows = [dict(v) for v in self.db.execute("SELECT * FROM jobs ORDER BY sealed_at DESC,id LIMIT 64")]
        totals = dict(self.db.execute("SELECT count(*) inputs,coalesce(sum(state='completed'),0) completed,coalesce(sum(state IN ('pending','running','graph_committed','calculating')),0) pending,coalesce(sum(state='failed'),0) failed,coalesce(sum(CASE WHEN state IN ('completed','graph_committed','calculating') THEN records ELSE 0 END),0) source_records,coalesce(sum(edges),0) stored_edges,coalesce(sum(score_state='calculated'),0) scored_inputs FROM jobs").fetchone())
        totals["output_bytes"] = self.output_bytes
        sample = helpers.process_sample(self.child.pid) if self.child else None
        worker = {"pid": self.child.pid, "input": self.current["id"], "stage": self.stage, "elapsed_seconds": time.monotonic()-self.child_started, "peak_rss_bytes": self.peak, **(sample or {})} if self.child else None
        visible = {k: v for k, v in self.config.items() if k not in ["source_spool", "state_directory", "binary"]}
        visible.update(binary_sha256=self.binary_sha, workers=1, priority=15, scope="per_input_graph", signals_adopted=False)
        return {"schema_version": 1, "component": "webgraph", "run_id": self.run_id, "pid": os.getpid(), "start_ticks": helpers.process_sample(os.getpid())["start_ticks"], "started_at": self.started_at, "updated_at": helpers.timestamp(), "phase": "draining" if self.draining and self.phase != "stopped" else self.phase, "reason": self.reason, "source": self.source, "configuration": visible, "totals": totals, "worker": worker, "inputs": rows, "history": self.history, "recent_events": self.events, "latest_input": self.latest_input, "scored_input": self.scored_input, "top_scores": self.top_scores, "signals_adopted": False, "scope": "per_input_graph"}

    def run(self):
        signal.signal(signal.SIGTERM, lambda *_: setattr(self, "draining", True))
        signal.signal(signal.SIGINT, lambda *_: setattr(self, "draining", True))
        projection = 0
        try:
            while not self.draining or self.child:
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
                elif not self.draining and now >= self.next_job and self.reason != "source_unavailable":
                    self.admit()
                if now-projection >= self.config["poll_seconds"]:
                    snapshot = self.snapshot()
                    self.history.append({"at": snapshot["updated_at"], "edges": snapshot["totals"]["stored_edges"], "rss_mib": max(self.interval_rss, (snapshot["worker"] or {}).get("rss_bytes", 0))/1048576, "completed": snapshot["totals"]["completed"]})
                    self.interval_rss = 0
                    self.history = self.history[-300:]
                    snapshot["history"] = self.history
                    try:
                        helpers.atomic_json(self.directory / "status.json", snapshot, durable=False)
                    except OSError as error:
                        print(f"Webgraph metrics unavailable: {error}", flush=True)
                    projection = now
                time.sleep(.1 if self.child else .5)
        finally:
            self.phase, self.reason = "stopped", "shutdown"
            helpers.atomic_json(self.directory / "status.json", self.snapshot(), durable=False)
            self.db.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "run", "status", "stop"])
    parser.add_argument("--config", type=Path, default=ROOT / "configs/webgraph/local-pilot.toml")
    args = parser.parse_args()
    config = configuration(args.config.resolve())
    directory = config["state_directory"]
    directory.mkdir(parents=True, exist_ok=True)
    if args.action == "run":
        with (directory / "worker.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            manifest = directory / "processes.json"
            if manifest.exists():
                runner = helpers.runner_module()
                record = helpers.limited_json(manifest)
                details = runner.process_details(os.getpid())
                if details and details["cwd"] == str(ROOT):
                    record.update(pid=os.getpid(), start_ticks=details["start_ticks"], command=details["command"])
                    helpers.atomic_json(manifest, record)
            Pilot(config).run()
        return
    runner = helpers.runner_module()
    manifest = directory / "processes.json"
    with (directory / "runner.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        record = helpers.limited_json(manifest) if manifest.exists() else {}
        if args.action == "start":
            if runner.owned_process(record):
                print(f"Webgraph already active (PID {record['pid']})")
                return
            record = runner.launch("webgraph", [sys.executable, str(Path(__file__).resolve()), "run", "--config", str(args.config.resolve())], "systemd" if runner.systemd_available() else "detached", {})
            helpers.atomic_json(manifest, record)
            print(f"Webgraph started (PID {record['pid']})")
        elif args.action == "stop":
            runner.stop_one("webgraph", record)
        else:
            print(json.dumps({"active": runner.owned_process(record), "pid": record.get("pid"), "state_path": str(directory / "status.json")}))


if __name__ == "__main__":
    main()
