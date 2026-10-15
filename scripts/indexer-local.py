#!/usr/bin/env python3
"""Consume sealed crawler captures with the native indexer and project bounded state."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import resource
import signal
import sqlite3
import subprocess
import sys
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "configs/indexer/local-pilot.toml"


def timestamp():
    return datetime.now(timezone.utc).isoformat()


def limited_json(path, limit=65536):
    with path.open("rb") as source:
        value = source.read(limit + 1)
    if len(value) > limit:
        raise ValueError(f"JSON exceeds size budget: {path.name}")
    return json.loads(value)


def checksum(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_json(path, value, durable=True):
    temporary = path.with_suffix(".tmp")
    with temporary.open("w") as output:
        json.dump(value, output, ensure_ascii=False)
        output.write("\n")
        output.flush()
        if durable:
            os.fsync(output.fileno())
    temporary.replace(path)
    if durable:
        descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)


def configuration(path):
    value = tomllib.loads(path.read_text())
    publication_seconds = value.pop("publication_seconds", 0)
    if type(publication_seconds) is not int or publication_seconds < 0:
        raise ValueError("publication_seconds must be a non-negative integer")
    expected = {"source_spool", "state_directory", "binary", "poll_seconds", "scan_seconds", "admission_seconds", "max_shards", "max_output_bytes", "minimum_disk_free_bytes", "memory_guard_bytes", "max_input_bytes", "job_timeout_seconds", "batch_size", "consumer"}
    ingestion=value.pop("ingestion", None)
    if set(value) != expected:
        raise ValueError(f"Invalid pilot keys: missing={sorted(expected-set(value))}, unknown={sorted(set(value)-expected)}")
    for key in expected - {"source_spool", "state_directory", "binary", "consumer", "admission_seconds"}:
        if type(value[key]) is not int or value[key] <= 0:
            raise ValueError(f"{key} must be a positive integer")
    if not 1 <= value["max_shards"] <= 128 or not 1 <= value["batch_size"] <= 64:
        raise ValueError("Pilot supports 1..128 shards and batches of 1..64")
    if type(value["admission_seconds"]) is not int or value["admission_seconds"] < 0 or value["poll_seconds"] > 5 or value["scan_seconds"] < value["poll_seconds"]:
        raise ValueError("Invalid poll, scan or admission interval")
    consumer = value["consumer"]
    if set(consumer) != {"record_bytes", "decoded_bytes", "header_bytes", "batch_bytes", "workers", "stack_bytes"} or any(type(n) is not int or n <= 0 for n in consumer.values()):
        raise ValueError("Invalid consumer limits")
    if consumer["workers"] != 1 or not consumer["decoded_bytes"] <= consumer["batch_bytes"] <= 64 * 1024 * 1024:
        raise ValueError("Pilot requires one worker and decoded <= batch <=64 MiB")
    for key in ["source_spool", "state_directory", "binary"]:
        value[key] = (ROOT / value[key]).resolve()
        if not value[key].is_relative_to(ROOT):
            raise ValueError(f"{key} must remain in this workspace")
    if value["state_directory"].is_relative_to(value["source_spool"]) or value["source_spool"].is_relative_to(value["state_directory"]):
        raise ValueError("Input spool and index output must be separate")
    value["ingestion"] = ingestion
    value["publication_seconds"] = publication_seconds
    return value


def process_sample(pid):
    try:
        proc = Path(f"/proc/{pid}")
        fields = (proc / "stat").read_text().rsplit(")", 1)[1].split()
        if fields[0] == "Z":
            return None
        io = dict(line.split(":", 1) for line in (proc / "io").read_text().splitlines())
        return {"rss_bytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"), "cpu_seconds": (int(fields[11])+int(fields[12])) / os.sysconf("SC_CLK_TCK"),
                "read_bytes": int(io["read_bytes"]), "written_bytes": int(io["write_bytes"]), "open_fds": len(list((proc / "fd").iterdir())), "start_ticks": fields[19]}
    except (OSError, ValueError, IndexError):
        return None


def index_counts(directory):
    meta = limited_json(directory / "inverted_index/meta.json", 2 * 1024 * 1024)
    return sum(item["max_doc"]-(item.get("deletes") or {}).get("num_deleted_docs", 0) for item in meta["segments"]), len(meta["segments"])


def file_bytes(directory):
    total = 0
    seen = set()
    for parent, folders, files in os.walk(directory):
        folders[:] = [name for name in folders if not Path(parent, name).is_symlink()]
        for name in files:
            path = Path(parent, name)
            if not path.is_symlink():
                try:
                    stat = path.stat()
                    identity = (stat.st_dev, stat.st_ino)
                    if identity not in seen:
                        seen.add(identity)
                        total += stat.st_size
                except FileNotFoundError:
                    pass
    return total


def log_path_size(path):
    return path.stat().st_size


def restricted_child():
    cpus = sorted(os.sched_getaffinity(0))
    # Separate the consumers on a multi-CPU profile; each worker keeps one CPU.
    offset = 1 if "webgraph" in Path(sys.argv[0]).stem else 0
    os.sched_setaffinity(0, {cpus[offset % len(cpus)]})
    os.nice(10)
    resource.setrlimit(resource.RLIMIT_NOFILE, (256, 256))


class Pilot:
    def __init__(self, config, config_path=None):
        self.config = config
        self.config_path = config_path
        self.config_signature = config_path.stat().st_mtime_ns if config_path else None
        self.config_error = None
        self.crawler_alive = False
        self.directory = config["state_directory"]
        self.directory.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.directory / "jobs.sqlite")
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.execute("PRAGMA cache_size=-2048")
        self.db.execute("CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, file TEXT NOT NULL, sha256 TEXT NOT NULL, records INTEGER NOT NULL, input_bytes INTEGER NOT NULL, sealed_at INTEGER NOT NULL, state TEXT NOT NULL, documents INTEGER NOT NULL DEFAULT 0, segments INTEGER NOT NULL DEFAULT 0, cursor INTEGER NOT NULL DEFAULT 0, elapsed_seconds REAL NOT NULL DEFAULT 0, cpu_seconds REAL NOT NULL DEFAULT 0, peak_rss_bytes INTEGER NOT NULL DEFAULT 0, error TEXT, completed_at TEXT)")
        columns={row[1] for row in self.db.execute("PRAGMA table_info(jobs)")}
        if "next_eligible_at" not in columns:self.db.execute("ALTER TABLE jobs ADD COLUMN next_eligible_at REAL NOT NULL DEFAULT 0")
        if "attempts" not in columns:self.db.execute("ALTER TABLE jobs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0")
        self.db.execute("UPDATE jobs SET state='deferred',next_eligible_at=? WHERE state='failed' AND error LIKE 'deferred:%'",(time.time()+600,))
        self.db.execute("UPDATE jobs SET state='pending' WHERE state='running'")
        self.db.commit()
        if config["ingestion"] is not None:
            folder=config["source_spool"] / "segments"
            for row in self.db.execute("SELECT id,file,sha256 FROM jobs WHERE state='failed'").fetchall():
                identity=hashlib.sha256(f"local:{folder}:{row['file']}".encode()).hexdigest()
                try:
                    saved=limited_json(self.directory / "index/checkpoints" / f"{identity}.json")
                    if saved.get("complete") and saved["fingerprint"]==row["sha256"]:
                        self.db.execute("UPDATE jobs SET state='pending' WHERE id=?",(row["id"],))
                except (OSError,ValueError,KeyError):pass
            self.db.commit()
        self.binary_sha256 = checksum(config["binary"])
        self.db.execute("CREATE TABLE IF NOT EXISTS samples (at TEXT PRIMARY KEY, value TEXT NOT NULL)")
        self.db.commit()
        self.history = [json.loads(row[0]) for row in self.db.execute("SELECT value FROM samples ORDER BY at")]
        self.run_id, self.started_at = str(uuid.uuid4()), timestamp()
        self.child = self.child_log = self.current = None
        self.draining = False
        self.next_job_at = self.last_scan = self.output_bytes = 0
        self.phase, self.reason = "idle", "waiting_for_segments"
        self.source = {"sealed": 0, "open": 0, "oversized": 0, "available": 0, "scan_errors": 0}
        self.events, self.last_sample = [], None
        self.runtime = {}
        try: self.runtime=limited_json(self.directory / "native-progress.json",131072)
        except (OSError, ValueError): pass
        self.interval_rss = self.interval_cpu = 0
        self.history = []
        try: self.history=limited_json(self.directory / "history.json",131072)[-300:]
        except (OSError, ValueError): pass
        self.job_peak = self.job_cpu = 0

    def event(self, kind, message):
        self.events.append({"at": timestamp(), "kind": kind, "message": message[:512]})
        self.events = self.events[-32:]

    def reload_configuration(self):
        """Apply valid settings between jobs, preserving the running worker and ledger."""
        if self.child or self.config_path is None:
            return
        try:
            signature = self.config_path.stat().st_mtime_ns
            if signature == self.config_signature:
                return
            candidate = configuration(self.config_path)
            for key in ["source_spool", "state_directory"]:
                if candidate[key] != self.config[key]:
                    raise ValueError(f"Live reload cannot change {key}")
            if (candidate["ingestion"] is None) != (self.config["ingestion"] is None):
                raise ValueError("Live reload cannot change ingestion mode")
            binary_hash = checksum(candidate["binary"])
            if candidate["binary"] != self.config["binary"]:
                subprocess.run([str(candidate["binary"]), "--build-info"], check=True,
                               stdout=subprocess.DEVNULL, timeout=5)
            self.config, self.binary_sha256 = candidate, binary_hash
            self.config_signature, self.config_error = signature, None
            self.next_job_at = min(self.next_job_at, time.monotonic()+candidate["admission_seconds"])
            self.last_scan = 0
            self.event("configuration", "Configuration applied at job boundary")
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            message = str(error)[:512]
            if self.config_error != message:
                self.event("error", f"Live configuration rejected; previous settings retained: {message}")
            self.config_error = message

    def scan(self):
        candidates = []
        counts = {"sealed": 0, "open": 0, "oversized": 0, "available": 0, "scan_errors": 0}
        source = self.config["source_spool"]
        try:
            if not (source / "state").is_dir() or not (source / "segments").is_dir():
                raise OSError("Input spool unavailable")
            for i, path in enumerate((source / "state").glob("*.json")):
                if i >= 4096:
                    raise ValueError("Spool scan exceeds 4096 entries")
                try:
                    item = limited_json(path)
                    identity = str(uuid.UUID(item["id"]))
                    if item.get("schema_version") not in [1, 2] or item["file"] != f"{identity}.warc.gz" or path.name != f"{identity}.json":
                        raise ValueError("Invalid segment descriptor")
                    if item["state"] == "open":
                        counts["open"] += 1
                        continue
                    if item["state"] not in ["local", "pending", "published"] or not item.get("sealed_at") or not item.get("records"):
                        continue
                    counts["sealed"] += 1
                    file = source / "segments" / item["file"]
                    if file.is_symlink() or file.stat().st_size != item["bytes"] or not isinstance(item.get("sha256"), str) or len(item["sha256"]) != 64:
                        raise ValueError("Segment file/descriptor mismatch")
                    if item["bytes"] > self.config["max_input_bytes"]:
                        counts["oversized"] += 1
                        continue
                    counts["available"] += 1
                    candidates.append(item)
                except (OSError, ValueError, KeyError, TypeError):
                    counts["scan_errors"] += 1
            used = self.db.execute("SELECT count(*) FROM jobs" + (" WHERE state IN ('pending','running')" if self.config["ingestion"] is not None else "")).fetchone()[0]
            for item in sorted(candidates, key=lambda x: (x["sealed_at"], x["id"])):
                known = self.db.execute("SELECT sha256 FROM jobs WHERE id=?", (item["id"],)).fetchone()
                if known:
                    if known[0] != item["sha256"]:
                        raise ValueError("Previously admitted immutable source changed")
                    continue
                if used >= self.config["max_shards"]:
                    break
                self.db.execute("INSERT INTO jobs(id,file,sha256,records,input_bytes,sealed_at,state) VALUES(?,?,?,?,?,?,'pending')", (item["id"], item["file"], item["sha256"], item["records"], item["bytes"], item["sealed_at"]))
                used += 1
            self.db.commit()
            self.source, self.output_bytes = counts, file_bytes(self.directory / ("index" if self.config["ingestion"] is not None else "shards"))
            self.last_scan = time.monotonic()
            return True
        except (OSError, ValueError) as error:
            self.db.rollback()
            self.phase, self.reason = "blocked", "source_unavailable"
            self.event("error", str(error))
            return False

    def admit(self):
        config = self.config
        policy=config["ingestion"]
        if policy is not None and "merge" in str(self.runtime.get("error") or ""):
            required=2*file_bytes(self.directory / "index/working/inverted_index")+16*1024*1024
            if required>policy.get("merge_temporary_bytes",268435456) or file_bytes(self.directory / "index")+required>=config["max_output_bytes"]:
                self.phase,self.reason="blocked","index_pressure"
                return
        row = self.db.execute("SELECT * FROM jobs WHERE state='pending' OR (state='deferred' AND next_eligible_at<=? AND attempts<3) ORDER BY state='pending' DESC,sealed_at,id LIMIT 1",(time.time(),)).fetchone()
        if not row:
            extra = self.source["available"] > self.db.execute("SELECT count(*) FROM jobs").fetchone()[0]
            self.phase, self.reason = ("blocked", "pilot_limit") if extra else ("idle", "waiting_for_segments")
            return
        if self.output_bytes >= config["max_output_bytes"]:
            self.phase, self.reason = "blocked", "output_quota"
            return
        disk = os.statvfs(self.directory)
        if disk.f_bavail * disk.f_frsize < config["minimum_disk_free_bytes"]:
            self.phase, self.reason = "blocked", "disk_reserve"
            return
        try:
            crawler = limited_json(ROOT / "data/crawler-local/status.json", 16 * 1024 * 1024)
            self.crawler_alive = time.time()-datetime.fromisoformat(crawler["updated_at"]).timestamp() <= 15 and crawler.get("phase") not in ["stopped", "failed", "completed"]
            if self.crawler_alive and (crawler.get("resources") or {}).get("reason") in ["memory_limit", "memory_pressure", "disk_reserve", "inode_reserve"]:
                self.phase, self.reason = "blocked", "crawler_pressure"
                return
        except (OSError, ValueError, KeyError):
            # Sealed inputs remain consumable while their producer is unavailable.
            self.crawler_alive = False
        file = config["source_spool"] / "segments" / row["file"]
        try:
            if checksum(file) != row["sha256"]:
                raise ValueError("Source checksum mismatch; checkpoint retained")
        except (OSError, ValueError) as error:
            self.db.execute("UPDATE jobs SET state='failed',error=? WHERE id=?", (str(error)[:512], row["id"]))
            self.db.commit()
            self.event("error", str(error))
            return
        shared = config["ingestion"] is not None
        output = self.directory / "index" if shared else self.directory / "shards" / row["id"]
        folder = config["source_spool"] / "segments"
        identity = hashlib.sha256(f"local:{folder}:{row['file']}".encode()).hexdigest()
        self.checkpoint_path = Path(str(output)+".consumer-work") / "jobs" / identity / row["file"] / "consumer-checkpoints" / f"{identity}.json"
        if shared: self.checkpoint_path=output / "checkpoints" / f"{identity}.json"
        self.before_documents=self.runtime.get("documents",0)
        native = {"output_path": str(output), "host_centrality_store_path": str(self.directory / "unranked-centrality"), "limit_warc_files": 1, "batch_size": config["batch_size"], "autocommit_after_num_inserts": config["batch_size"]}
        text = "\n".join(f"{key} = {json.dumps(value)}" for key, value in native.items())
        text += "\n\n[warc_source]\ntype = \"Local\"\nfolder = " + json.dumps(str(folder)) + "\nnames = " + json.dumps([row["file"]])
        text += "\n\n[consumer]\n" + "\n".join(f"{key} = {value}" for key, value in config["consumer"].items())
        if shared:
            policy={**config["ingestion"],"report_path":str(self.directory / "native-progress.json")}
            if policy.get("publish_generations") and config["publication_seconds"]:
                selector = output / "generation-selector.json"
                try:
                    publication_due = time.time()-selector.stat().st_mtime >= config["publication_seconds"]
                except FileNotFoundError:
                    publication_due = True
                remaining = self.db.execute("SELECT count(*) FROM jobs WHERE state='pending'").fetchone()[0]
                # A steady stream can catch up after every segment. Preserve the
                # configured publication window instead of rebuilding each time.
                policy["publish_generations"] = publication_due
            text += "\n\n[ingestion]\n" + "\n".join(f"{key} = {json.dumps(value)}" for key,value in policy.items())
        job_path = self.directory / "current-job.toml"
        job_path.write_text(text + "\n")
        (self.directory / "logs").mkdir(exist_ok=True)
        log_path=self.directory / "logs" / f"{row['id']}.log"
        self.child_log=log_path.open("ab")
        self.job_log_path=log_path
        self.current, self.last_sample = dict(row), None
        self.job_peak = self.job_cpu = 0
        self.child_started, self.child_error = time.monotonic(), None
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        self.child_cpu_start = usage.ru_utime + usage.ru_stime
        self.child = subprocess.Popen([str(config["binary"]), *([] if shared else ["indexer", "search"]), str(job_path)], cwd=ROOT, env={**os.environ, "RUST_LOG": "astiango=info"}, stdin=subprocess.DEVNULL, stdout=self.child_log, stderr=subprocess.STDOUT, start_new_session=True, preexec_fn=restricted_child)
        self.db.execute("UPDATE jobs SET state='running',error=NULL WHERE id=?", (row["id"],))
        self.db.commit()
        self.phase, self.reason = "indexing", "processing_segment"
        self.event("started", row["file"])

    def poll_child(self):
        sample = process_sample(self.child.pid)
        if sample:
            self.job_peak = max(self.job_peak, sample["rss_bytes"])
            self.job_cpu = max(self.job_cpu, sample["cpu_seconds"])
            previous = self.last_sample
            sample["cpu_percent"] = max(0, (sample["cpu_seconds"]-previous["cpu_seconds"]) / max(.001, time.monotonic()-previous["at"]) * 100) if previous else 0
            sample["at"], self.last_sample = time.monotonic(), sample
            self.interval_rss=max(self.interval_rss,sample["rss_bytes"])
            self.interval_cpu=max(self.interval_cpu,sample["cpu_percent"])
        if self.config["ingestion"] is not None:
            try: self.runtime=limited_json(self.directory / "native-progress.json",131072)
            except (OSError, ValueError): pass
        elapsed = time.monotonic()-self.child_started
        if self.child.poll() is None and not self.child_error:
            if self.job_peak > self.config["memory_guard_bytes"]:
                self.child_error = "Memory guard exceeded; checkpoint/source retained"
            elif elapsed > self.config["job_timeout_seconds"]:
                self.child_error = "Job deadline exceeded; checkpoint/source retained"
            elif self.output_bytes >= self.config["max_output_bytes"]:
                self.child_error = "Output quota reached; checkpoint/source retained"
            if self.child_error:
                self.child.terminate()
        try:
            checkpoint = limited_json(self.checkpoint_path)
            self.current["cursor"] = checkpoint["records_committed"]
        except (OSError, ValueError, KeyError):
            pass
        result = self.child.poll()
        if result is None:
            return
        self.child_log.close()
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        self.job_cpu = max(self.job_cpu, usage.ru_utime+usage.ru_stime-self.child_cpu_start)
        state, error, documents, segments = "failed", self.child_error, 0, 0
        if result == 0 and not error:
            try:
                checkpoint = limited_json(self.checkpoint_path)
                if not checkpoint.get("complete") or checkpoint["fingerprint"] != self.current["sha256"]:
                    raise ValueError("Complete matching input not confirmed")
                if self.config["ingestion"] is not None:
                    self.runtime=limited_json(self.directory / "native-progress.json",131072)
                    if not self.runtime["source_complete"]: raise ValueError("Native source not complete")
                    documents=max(0,self.runtime["documents"]-self.before_documents)
                    _,segments=index_counts(self.directory / "index/working")
                else: documents, segments = index_counts(self.directory / "shards" / self.current["id"])
                if checkpoint["records_committed"] != self.current["records"]:
                    raise ValueError("Cursor/descriptor counts do not reconcile")
                state = "completed"
            except (OSError, ValueError, KeyError, TypeError) as cause:
                error = str(cause)
        if state=="failed" and self.config["ingestion"] is not None: error=error or self.runtime.get("error")
        if state=="failed" and not error:
            try:
                with self.job_log_path.open("rb") as log:
                    log.seek(max(0,log_path_size(self.job_log_path)-4096))
                    detail=log.read().decode("utf8",errors="replace")
                error=detail[-512:]
            except OSError:pass
        error = error or (f"Indexer exited with code {result}; see last-job.log" if state == "failed" else None)
        if state=="failed" and str(error).startswith("deferred:"):
            state="deferred"
            self.db.execute("UPDATE jobs SET next_eligible_at=?,attempts=attempts+1 WHERE id=?",(time.time()+600,self.current["id"]))
        self.db.execute("UPDATE jobs SET state=?,documents=?,segments=?,cursor=?,elapsed_seconds=?,cpu_seconds=?,peak_rss_bytes=?,error=?,completed_at=? WHERE id=?", (state, documents, segments, self.current.get("cursor", 0), elapsed, self.job_cpu, self.job_peak, error, timestamp(), self.current["id"]))
        self.db.commit()
        self.event(state, error or f"{documents} documents committed from {self.current['file']}")
        self.child = self.current = None
        self.next_job_at = time.monotonic()+self.config["admission_seconds"]
        self.phase, self.reason = "idle", "admission_interval" if self.config["admission_seconds"] else "waiting_for_segments"

    def snapshot(self):
        rows = [dict(row) for row in self.db.execute("SELECT * FROM jobs ORDER BY sealed_at DESC,id LIMIT 128")]
        completed = [row for row in rows if row["state"] == "completed"]
        totals = {"shards_completed": len(completed), "shards_failed": sum(row["state"] == "failed" for row in rows), "shards_pending": sum(row["state"] == "pending" for row in rows), "source_records_completed": sum(row["records"] for row in completed), "indexed_documents": sum(row["documents"] for row in completed), "records_without_document": sum(row["records"]-row["documents"] for row in completed), "source_bytes_completed": sum(row["input_bytes"] for row in completed), "elapsed_seconds": sum(row["elapsed_seconds"] for row in completed), "cpu_seconds": sum(row["cpu_seconds"] for row in completed), "output_bytes": self.output_bytes}
        if self.config["ingestion"] is not None:
            aggregate=self.db.execute("SELECT sum(state='completed'),sum(state='failed'),sum(state='pending'),sum(CASE WHEN state='completed' THEN records ELSE 0 END),sum(CASE WHEN state='completed' THEN documents ELSE 0 END),sum(CASE WHEN state='completed' THEN input_bytes ELSE 0 END),sum(elapsed_seconds),sum(cpu_seconds) FROM jobs").fetchone()
            for key,value in zip(["shards_completed","shards_failed","shards_pending","source_records_completed","indexed_documents","source_bytes_completed","elapsed_seconds","cpu_seconds"],aggregate):totals[key]=value or 0
            totals["records_without_document"]=max(0,totals["source_records_completed"]-totals["indexed_documents"])
            totals["shards_deferred"]=self.db.execute("SELECT count(*) FROM jobs WHERE state='deferred'").fetchone()[0]
            if self.runtime:
                totals["indexed_documents"]=self.runtime["documents"]
                outcomes=self.runtime.get("outcomes",{})
                # Current URL documents and terminal input decisions have
                # different denominators, especially after replay/recovery.
                totals["records_without_document"]=sum(count for outcome,count in outcomes.items() if outcome not in ['indexed','pending'])
                totals["records_decided"]=sum(count for outcome,count in outcomes.items() if outcome!='pending')
                totals["records_indexed"]=outcomes.get('indexed',0)
                totals["records_pending"]=outcomes.get('pending',0)
        generation=None
        try: generation=limited_json(self.directory / "index/generations/status.json",262144)
        except (OSError,ValueError):pass
        generation=None
        try: generation=limited_json(self.directory / "index/generations/status.json",262144)
        except (OSError,ValueError):pass
        worker = {"pid": self.child.pid, "input": self.current["file"], "records_committed": self.current.get("cursor", 0), "records_total": self.current["records"], "elapsed_seconds": time.monotonic()-self.child_started, "peak_rss_bytes": self.job_peak, **(self.last_sample or {})} if self.child else None
        return {"schema_version": 1, "component": "indexer", "run_id": self.run_id, "pid": os.getpid(), "start_ticks": process_sample(os.getpid())["start_ticks"], "started_at": self.started_at, "updated_at": timestamp(), "phase": "draining" if self.draining and self.phase != "stopped" else self.phase, "reason": self.reason,
                "source": {"spool": str(self.config["source_spool"].relative_to(ROOT)), **self.source}, "configuration": {"binary_sha256": self.binary_sha256, "workers": 1, "cpu_affinity": 1, "priority": 10, "memory_guard_bytes": self.config["memory_guard_bytes"], "max_output_bytes": self.config["max_output_bytes"], "max_shards": self.config["max_shards"], "capacity_scope":"pending_inputs" if self.config["ingestion"] is not None else "total_shards", "admission_seconds": self.config["admission_seconds"], "batch_bytes": self.config["consumer"]["batch_bytes"], "ingestion":self.config["ingestion"],"signals": "unranked; graph and ML not configured"}, "history": self.history, "totals": totals, "worker": worker, "shards": rows, "recent_events": self.events, "history":self.history,"runtime":self.runtime or None,"publication": {"search_visible": False, "generation_selected": bool(generation and generation.get("active")), "active_generation":generation.get("active") if generation else None, "publication_error":generation.get("last_error") if generation else None, "cross_shard_dedup": self.config["ingestion"] is not None, "gc_enabled": False}}

    def run(self):
        def drain(signum, frame):
            self.draining = True
        signal.signal(signal.SIGTERM, drain)
        signal.signal(signal.SIGINT, drain)
        last_projection = 0
        try:
            while not self.draining or self.child:
                now = time.monotonic()
                self.reload_configuration()
                healthy = self.scan() if now-self.last_scan >= self.config["scan_seconds"] else self.reason != "source_unavailable"
                if self.child:
                    self.poll_child()
                elif not self.draining and healthy and now >= self.next_job_at:
                    self.admit()
                if now-last_projection >= self.config["poll_seconds"]:
                    observation = self.snapshot()
                    sample = {"at": observation["updated_at"], "documents": observation["totals"]["indexed_documents"], "rss_mib": (observation["worker"] or {}).get("rss_bytes", 0)/1048576, "cpu_percent": (observation["worker"] or {}).get("cpu_percent", 0)}
                    self.history.append(sample)
                    self.history = self.history[-300:]
                    with self.db:
                        self.db.execute("INSERT OR REPLACE INTO samples VALUES(?,?)", (sample["at"], json.dumps(sample)))
                        self.db.execute("DELETE FROM samples WHERE at NOT IN (SELECT at FROM samples ORDER BY at DESC LIMIT 300)")
                    try:
                        snapshot=self.snapshot()
                        worker=snapshot["worker"] or {}
                        self.history.append({"at":snapshot["updated_at"],"documents":snapshot["totals"]["indexed_documents"],"rss_mib":max(self.interval_rss,worker.get("rss_bytes",0))/1048576,"cpu_percent":max(self.interval_cpu,worker.get("cpu_percent",0))})
                        self.history=self.history[-300:]
                        snapshot["history"]=self.history
                        self.interval_rss=self.interval_cpu=0
                        atomic_json(self.directory / "history.json",self.history,durable=False)
                        atomic_json(self.directory / "status.json", snapshot, durable=False)
                    except OSError as error:
                        print(f"Indexing metrics unavailable: {error}", flush=True)
                    last_projection = now
                time.sleep(.1 if self.child else .5)
        finally:
            self.phase, self.reason = "stopped", "shutdown"
            atomic_json(self.directory / "status.json", self.snapshot(), durable=False)
            self.db.close()


def runner_module():
    spec = importlib.util.spec_from_file_location("crawler_local", ROOT / "scripts/crawler-local.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "run", "status", "stop"])
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    args = parser.parse_args()
    config = configuration(args.config.resolve())
    directory = config["state_directory"]
    directory.mkdir(parents=True, exist_ok=True)
    if args.action == "run":
        with (directory / "worker.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            # A supervisor restart changes PID/start ticks; status/start must follow it.
            manifest = directory / "processes.json"
            if manifest.exists():
                runner = runner_module()
                record = limited_json(manifest)
                details = runner.process_details(os.getpid())
                if details and details["cwd"] == str(ROOT):
                    record.update(pid=os.getpid(), start_ticks=details["start_ticks"], command=details["command"])
                    atomic_json(manifest, record)
            Pilot(config, args.config.resolve()).run()
        return
    runner = runner_module()
    manifest = directory / "processes.json"
    with (directory / "runner.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        record = limited_json(manifest) if manifest.exists() else {}
        if args.action == "start":
            if runner.owned_process(record):
                print(f"Indexer already active (PID {record['pid']})")
                return
            if not config["binary"].is_file():
                raise ValueError("Build the native indexer before starting")
            subprocess.run([str(config["binary"]), *([] if config["ingestion"] is not None else ["indexer", "search"]), "--help"], check=True, stdout=subprocess.DEVNULL)
            backend = "systemd" if runner.systemd_available() else "detached"
            record = runner.launch("indexer", [sys.executable, str(Path(__file__).resolve()), "run", "--config", str(args.config.resolve())], backend, {})
            atomic_json(manifest, record)
            print(f"Indexer started (PID {record['pid']}, {backend})")
        elif args.action == "stop":
            runner.stop_one("indexer", record)
        else:
            print(json.dumps({"active": runner.owned_process(record), "pid": record.get("pid"), "state_path": str(directory / "status.json")}, indent=2))


if __name__ == "__main__":
    main()
