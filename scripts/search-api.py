#!/usr/bin/env python3
"""Persistent, bounded local Search/API backed by native LocalSearcher readers.

Generation selection, pins and command receipts are independent of ingestion.
No third-party search results, payment simulation, or per-query process spawn.
"""
import argparse
import asyncio
from collections import Counter, OrderedDict, deque
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import gzip
import hashlib
import hmac
import json
import math
import os
from pathlib import Path
import resource
import secrets
import signal
import sqlite3
import sys
import time
import tomllib
import urllib.parse
import uuid

ROOT = Path(__file__).resolve().parents[1]


def timestamp():
    return datetime.now(timezone.utc).isoformat()


def read_json(path, maximum=1024 * 1024):
    with path.open("rb") as source:
        data = source.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("JSON exceeds byte budget")
    return json.loads(data)


def atomic(path, value, durable=True):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("w") as output:
        json.dump(value, output, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
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


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def configuration(path):
    config = tomllib.loads(path.read_text())
    expected = {"host", "port", "state_directory", "index_root", "graph_root", "binary", "binary_sha256",
                "workers", "max_queue", "max_connections", "deadline_ms", "load_timeout_seconds",
                "max_qps", "cache_bytes", "cache_entries", "cache_seconds", "poll_seconds",
                "memory_guard_bytes", "cpu_index", "auto_promote", "scope"}
    defaults = {"replica_timeout_ms": min(1500, config.get("deadline_ms", 2000)),
                "recovery_base_ms": 250, "recovery_max_ms": 8000, "recovery_attempts_per_minute": 5,
                "keepalive_seconds": 3, "keepalive_requests": 100, "gzip_min_bytes": 1024}
    expected.update(defaults)
    config = {**defaults, **config}
    if set(config) != expected:
        raise ValueError(f"Invalid Search/API options: {sorted(set(config) ^ expected)}")
    if config["host"] != "127.0.0.1" or not 1024 <= config["port"] <= 65535:
        raise ValueError("Local access only; external access requires a separate authenticated profile")
    for key in expected - {"host", "state_directory", "index_root", "graph_root", "binary", "binary_sha256", "scope", "auto_promote"}:
        if type(config[key]) is not int or config[key] <= 0:
            raise ValueError(f"{key} must be a positive integer")
    if config["workers"] > 2 or config["max_queue"] > 32 or config["max_connections"] > 64:
        raise ValueError("Local admission exceeds the combined resource profile")
    if config["deadline_ms"] > 10000 or config["cache_bytes"] > 16 * 1024 * 1024 or config["poll_seconds"] > 5:
        raise ValueError("Invalid deadline, cache or projection budget")
    if type(config["auto_promote"]) is not bool or config["scope"] not in {"local", "isolated_integration"}:
        raise ValueError("Invalid promotion policy or scope")
    if not 50 <= config["replica_timeout_ms"] <= config["deadline_ms"] or not 50 <= config["recovery_base_ms"] <= config["recovery_max_ms"] <= 30000:
        raise ValueError("Invalid replica deadline or recovery backoff")
    if config["recovery_attempts_per_minute"] > 10 or config["keepalive_seconds"] > 10 or config["keepalive_requests"] > 100 or config["gzip_min_bytes"] > 65536:
        raise ValueError("Recovery or HTTP budget exceeds the local profile")
    for key in ["state_directory", "index_root", "graph_root", "binary"]:
        config[key] = (ROOT / config[key]).resolve()
        if not config[key].is_relative_to(ROOT):
            raise ValueError(f"{key} must remain in this workspace")
    if sha(config["binary"]) != config["binary_sha256"]:
        raise ValueError("Search reader checksum mismatch")
    return config


def sample(pid):
    try:
        directory = Path(f"/proc/{pid}")
        fields = (directory / "stat").read_text().rsplit(")", 1)[1].split()
        io = {}
        try:
            io = {key: int(value) for key, value in (line.split(":", 1) for line in (directory / "io").read_text().splitlines())}
        except OSError:
            pass
        return {"rss_bytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"),
                "cpu_seconds": (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK"),
                "open_fds": len(list((directory / "fd").iterdir())),
                "read_bytes": io.get("read_bytes", 0), "written_bytes": io.get("write_bytes", 0),
                "read_chars": io.get("rchar", 0), "written_chars": io.get("wchar", 0)}
    except (OSError, ValueError, IndexError):
        return {"rss_bytes": 0, "cpu_seconds": 0, "open_fds": 0, "read_bytes": 0, "written_bytes": 0,
                "read_chars": 0, "written_chars": 0}


class ApiError(Exception):
    def __init__(self, status, code, message=None):
        self.status, self.code = status, code
        super().__init__(message or code)


class Worker:
    def __init__(self, service, group, ordinal):
        self.service, self.group, self.ordinal = service, group, ordinal
        self.process = None
        self.state, self.requests, self.load_ms = "loading", 0, 0.
        self.enabled, self.busy = True, False
        self.validated = False
        self.failures, self.recoveries = 0, 0
        self.last_error, self.next_retry_at = None, None
        self.repair_task = None
        self.attempts = deque(maxlen=10)

    @property
    def id(self):
        return f"{self.group.id}:{self.ordinal}"

    @property
    def eligible(self):
        return self.enabled and self.validated and self.state == "ready" and self.process is not None and self.process.returncode is None

    async def start(self):
        config = self.service.config
        self.validated = False
        self.state = "loading"
        def limits():
            cpus = sorted(os.sched_getaffinity(0))
            os.sched_setaffinity(0, {cpus[(config["cpu_index"] - 1) % len(cpus)]})
            resource.setrlimit(resource.RLIMIT_NOFILE, (256, 256))
            os.nice(10)
        self.process = await asyncio.create_subprocess_exec(
            str(config["binary"]), "--root", str(config["index_root"]), "--generation", self.group.id,
            "--manifest-sha256", self.group.digest, stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
            limit=512 * 1024 + 1, preexec_fn=limits,
            env={**os.environ, "RUST_LOG": "error", "RAYON_NUM_THREADS": "1"})
        try:
            async with asyncio.timeout(config["load_timeout_seconds"]):
                hello = json.loads(await self.process.stdout.readline())
                if hello.get("generation") != self.group.id or hello.get("manifest_sha256") != self.group.digest or hello.get("state") != "loaded":
                    raise ValueError("Native readiness contract mismatch")
                self.load_ms = hello["load_ms"]
                self.service.capabilities = hello.get("capabilities", {})
                self.state = "warming"
                warm = await self.execute({"id": str(uuid.uuid4()), "q": "search", "count": 1, "page": 0, "safe_search": False})
                if warm.get("outcome") != "complete":
                    raise ValueError("Native warmup failed")
                self.state = "ready" if self.enabled else "drained"
                self.validated = True
                self.last_error, self.next_retry_at = None, None
        except BaseException:
            await self.close()
            raise

    async def execute(self, request):
        if not self.process or self.process.returncode is not None:
            raise ApiError(502, "reader_unavailable")
        self.state = "busy"
        data = json.dumps(request, ensure_ascii=False).encode() + b"\n"
        self.process.stdin.write(data)
        await self.process.stdin.drain()
        data = await self.process.stdout.readline()
        if not data or len(data) > 512 * 1024:
            raise ApiError(502, "reader_response_invalid")
        result = json.loads(data)
        if result.get("id") != request["id"] or result.get("outcome") != "complete":
            raise ApiError(502, "native_query_failed", str(result.get("error", "Native query failed"))[:512])
        if len(result.get("results", [])) > request["count"]:
            raise ApiError(502, "reader_result_budget")
        self.requests += 1
        self.state = "ready" if self.enabled else "draining"
        return result

    async def close(self):
        self.state = "closed"
        self.validated = False
        if self.process and self.process.returncode is None:
            # SIGKILL also reaps a stopped or uncooperative blocking reader.
            try:
                self.process.kill()
            except ProcessLookupError:
                pass
            await self.process.wait()


class Group:
    """Two compatible replicas, one execution per query, bounded recovery tasks."""
    def __init__(self, service, identity, manifest, digest):
        self.service, self.id, self.manifest, self.digest = service, identity, manifest, digest
        self.workers, self.users = [], 0
        self.condition = asyncio.Condition()
        self.retired = False

    @property
    def ready(self):
        return any(worker.enabled and worker.validated and worker.process and worker.process.returncode is None
                   and worker.state in {"ready", "busy"} for worker in self.workers)

    async def notify(self):
        async with self.condition:
            self.condition.notify_all()

    async def load(self):
        try:
            for ordinal in range(self.service.config["workers"]):
                worker = Worker(self.service, self, ordinal)
                self.workers.append(worker)
                worker.enabled = not self.service.db.execute(
                    "SELECT 1 FROM node_policy WHERE generation=? AND ordinal=? AND drained=1", (self.id, ordinal)).fetchone()
                async with self.service.recovery_slots:
                    await worker.start()
        except BaseException:
            await self.close()
            raise

    async def acquire(self):
        async with self.condition:
            while True:
                available = [worker for worker in self.workers if worker.eligible and not worker.busy]
                if available:
                    worker = min(available, key=lambda value: (value.requests, value.ordinal))
                    worker.busy = True
                    return worker
                if not self.ready:
                    raise ApiError(503, "no_compatible_replica")
                await self.condition.wait()

    async def release(self, worker):
        worker.busy = False
        if not worker.enabled and worker.state == "draining":
            worker.state = "drained"
        await self.notify()

    def recover(self, worker, reason):
        worker.last_error = reason
        worker.validated = False
        worker.state = "backoff"
        if worker.repair_task is None or worker.repair_task.done():
            worker.repair_task = self.service.spawn(self.replace(worker))

    async def replace(self, worker):
        await worker.close()
        config = self.service.config
        while worker.enabled and not self.retired and not self.service.stopping:
            now = time.monotonic()
            while worker.attempts and worker.attempts[0] <= now-60:
                worker.attempts.popleft()
            delay = min(config["recovery_max_ms"], config["recovery_base_ms"] * 2**min(worker.failures, 10))/1000
            delay += secrets.randbelow(max(1, int(delay*100)))/1000
            if len(worker.attempts) >= config["recovery_attempts_per_minute"]:
                delay = max(delay, worker.attempts[0]+60-now)
            worker.next_retry_at = datetime.fromtimestamp(time.time()+delay, timezone.utc).isoformat()
            worker.state = "backoff"
            await asyncio.sleep(delay)
            if not worker.enabled or self.retired or self.service.stopping:
                break
            try:
                # One load/warmup at a time across generations and replacements.
                async with self.service.recovery_slots:
                    if not worker.enabled or self.retired:
                        break
                    worker.attempts.append(time.monotonic())
                    await worker.start()
                    worker.recoveries += 1
                    worker.failures = 0
                    self.service.event("replica_recovered", worker.id)
                    await self.notify()
                    return
            except Exception as error:
                worker.failures += 1
                worker.last_error = str(error)[:256]
                self.service.event("replica_recovery_failed", worker.id+": "+worker.last_error)
        worker.state = "closed" if self.retired else "drained"
        await self.notify()

    async def control(self, worker, action):
        if self.retired or self is not self.service.current:
            raise ApiError(409, "node_generation_not_serving")
        others = [other for other in self.workers if other is not worker and other.enabled and other.validated
                  and other.process and other.process.returncode is None and other.state in {"ready", "busy"}]
        if action in {"drain_node", "replace_node"} and not others:
            raise ApiError(409, "last_ready_replica")
        if action == "drain_node":
            worker.enabled = False
            self.service.db.execute("INSERT OR REPLACE INTO node_policy VALUES(?,?,1)", (self.id, worker.ordinal))
            self.service.db.commit()
            worker.state = "draining" if worker.busy else "drained"
            await self.notify()
            async with asyncio.timeout(self.service.config["deadline_ms"]/1000+.5):
                async with self.condition:
                    await self.condition.wait_for(lambda: not worker.busy)
            worker.state = "drained"
        elif action in {"resume_node", "replace_node"}:
            if action == "resume_node" and worker.eligible:
                return
            if sum(at > time.monotonic()-60 for at in worker.attempts) >= self.service.config["recovery_attempts_per_minute"]:
                raise ApiError(429, "recovery_budget_exhausted")
            if worker.busy:
                # Remove from routing immediately, then let its current request finish.
                worker.enabled = False
                await self.notify()
                async with asyncio.timeout(self.service.config["deadline_ms"]/1000+.5):
                    async with self.condition:
                        await self.condition.wait_for(lambda: not worker.busy)
            worker.enabled = True
            self.service.db.execute("DELETE FROM node_policy WHERE generation=? AND ordinal=?", (self.id, worker.ordinal))
            self.service.db.commit()
            if worker.repair_task and not worker.repair_task.done():
                worker.repair_task.cancel()
                await asyncio.gather(worker.repair_task, return_exceptions=True)
            self.recover(worker, "operator_replacement")
            async with asyncio.timeout(self.service.config["load_timeout_seconds"]+config_delay(self.service.config)):
                async with self.condition:
                    await self.condition.wait_for(lambda: worker.eligible or self.retired)
            if not worker.eligible:
                raise ApiError(409, "node_generation_retired")
        self.service.event(action, worker.id)

    async def close(self):
        self.retired = True
        for worker in self.workers:
            if worker.repair_task and worker.repair_task is not asyncio.current_task() and not worker.repair_task.done():
                worker.repair_task.cancel()
                await asyncio.gather(worker.repair_task, return_exceptions=True)
            await worker.close()
        await self.notify()


def config_delay(config):
    return config["recovery_max_ms"]/1000+1


class Service:
    def __init__(self, path):
        self.config = configuration(path)
        self.directory = self.config["state_directory"]
        self.directory.mkdir(parents=True, exist_ok=True)
        self.started, self.run_id = timestamp(), str(uuid.uuid4())
        self.token_path = self.directory / "control-token"
        if not self.token_path.exists():
            descriptor = os.open(self.token_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w") as output:
                output.write(secrets.token_urlsafe(32))
        self.token = self.token_path.read_text().strip()
        self.db = sqlite3.connect(self.directory / "operations.sqlite")
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.execute("PRAGMA cache_size=-1024")
        self.db.execute("CREATE TABLE IF NOT EXISTS commands(id TEXT PRIMARY KEY,request TEXT NOT NULL,action TEXT,generation TEXT,state TEXT,error TEXT,created_at TEXT,completed_at TEXT)")
        self.db.execute("UPDATE commands SET state='interrupted',error='Process interrupted; serving selector is authoritative' WHERE state IN ('accepted','applying')")
        self.db.commit()
        self.db.execute("CREATE TABLE IF NOT EXISTS node_policy(generation TEXT,ordinal INTEGER,drained INTEGER,PRIMARY KEY(generation,ordinal))")
        self.db.commit()
        self.recovery_slots = asyncio.Semaphore(1)
        self.http_totals = Counter()
        self.http_traffic = {name: Counter() for name in ["user", "probe", "benchmark"]}
        self.current = self.candidate = None
        self.previous, self.desired, self.hold = None, None, None
        self.draining, self.control_busy, self.stopping = False, False, False
        self.groups = []
        self.pins = {"indexer": set(), "webgraph": set()}
        self.last_pins = None
        self.selector_path = self.directory / "serving-selector.json"
        self.last_error, self.last_attempt, self.last_refresh = None, {}, 0.
        self.totals, self.traffic = Counter(), {name: Counter() for name in ["user", "probe", "benchmark"]}
        self.samples, self.history, self.events, self.rate = deque(maxlen=2048), deque(maxlen=300), deque(maxlen=32), deque()
        self.active, self.queued, self.admitted, self.connections = 0, 0, 0, 0
        self.cache, self.cache_size = OrderedDict(), 0
        self.capabilities, self.stages = {}, {}
        self.tasks = set()
        self.last_cpu, self.last_cpu_at, self.cpu_percent = 0., time.monotonic(), 0.
        self.generation_rows = []
        self.last_probe = None
        self.last_user_query_at = None
        self.recover_selection()

    def recover_selection(self):
        if self.selector_path.exists():
            selected = read_json(self.selector_path, 65536)
            if selected.get("version") != 1:
                raise ValueError("Unknown serving selector version")
            self.desired = str(uuid.UUID(selected["active"]))
            self.previous = selected.get("previous")
            self.hold = selected.get("hold_ingestion_generation")

    def event(self, reason, detail):
        self.events.append({"at": timestamp(), "reason": reason, "detail": str(detail)[:512]})

    def spawn(self, coroutine):
        task = asyncio.create_task(coroutine)
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)
        return task

    @contextmanager
    def storage_locks(self):
        handles = []
        try:
            for root in [self.config["index_root"], self.config["graph_root"]]:
                root.mkdir(parents=True, exist_ok=True)
                # A reader of an immutable generation need not wait for a long
                # ingestion batch. Coordinate only with retirement of its files.
                handle = (root / "retention.lock").open("a")
                handles.append(handle)
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield
        finally:
            for handle in reversed(handles):
                handle.close()

    def manifest(self, identity):
        if str(uuid.UUID(identity)) != identity:
            raise ValueError("Invalid generation identity")
        path = self.config["index_root"] / "generations" / identity / "manifest.json"
        value = read_json(path)
        if value.get("version") != 1 or value.get("state") != "ready" or not value.get("validation", {}).get("passed"):
            raise ValueError("Generation is not compatible and validated")
        return value, sha(path)

    def write_pins(self):
        current = tuple((key, tuple(sorted(value))) for key, value in self.pins.items())
        if current == self.last_pins:
            return
        atomic(self.directory / "pins.json", {"version": 1, "updated_at": timestamp(),
               **{key: sorted(value) for key, value in self.pins.items()}})
        self.last_pins = current

    def bind(self, identity):
        # Retention takes these same short locks and rechecks pins before retiring.
        with self.storage_locks():
            manifest, digest = self.manifest(identity)
            self.pins["indexer"].add(identity)
            graph = manifest.get("signals", {}).get("graph_generation")
            if graph and graph.get("generation"):
                graph_id = str(uuid.UUID(graph["generation"]))
                graph_path = self.config["graph_root"] / "generations" / graph_id / "manifest.json"
                if not graph_path.is_file():
                    raise ValueError("Pinned graph generation is not available")
                self.pins["webgraph"].add(graph_id)
            self.write_pins()
        return Group(self, identity, manifest, digest)

    def refresh_pins(self):
        identities = {group.id for group in self.groups if not group.retired or group.users}
        identities.update(value for value in [self.previous, self.desired] if value)
        with self.storage_locks():
            pins = {"indexer": set(), "webgraph": set()}
            for identity in identities:
                manifest, _ = self.manifest(identity)
                pins["indexer"].add(identity)
                graph = manifest.get("signals", {}).get("graph_generation")
                if graph and graph.get("generation"):
                    pins["webgraph"].add(graph["generation"])
            self.pins = pins
            self.write_pins()

    async def warm(self, identity):
        if self.candidate and self.candidate.id == identity and self.candidate.ready:
            return self.candidate
        if self.candidate:
            await self.candidate.close()
            self.candidate = None
        group = self.bind(identity)
        self.groups.append(group)
        self.candidate = group
        await group.load()
        self.event("generation_warmed", identity)
        return group

    def fault(self, name):
        if self.config["scope"] == "isolated_integration" and os.environ.get("SEARCH_FAILPOINT") == name:
            os._exit(72)

    async def promote(self, identity, manual=False):
        group = await self.warm(identity)
        old = self.current
        self.previous = old.id if old and old.id != identity else self.previous
        self.desired = identity
        if manual:
            try:
                self.hold = read_json(self.config["index_root"] / "generation-selector.json", 65536)["active"]
            except OSError:
                pass
        atomic(self.directory / "publication-intent.json", {"version": 1, "active": identity, "previous": self.previous,
               "manifest_sha256": group.digest, "state": "applying"})
        if manual:
            self.fault("after_intent")
        atomic(self.selector_path, {"version": 1, "active": identity, "previous": self.previous,
               "manifest_sha256": group.digest, "hold_ingestion_generation": self.hold,
               "updated_at": timestamp(), "public_rollout": False})
        if manual:
            self.fault("after_selector")
        self.current, self.candidate = group, None
        self.cache.clear()
        self.cache_size = 0
        atomic(self.directory / "publication-intent.json", {"version": 1, "active": identity, "previous": self.previous,
               "manifest_sha256": group.digest, "state": "applied"})
        self.event("generation_serving", identity)
        self.last_error = None
        if old and old is not group:
            old.retired = True
            if not old.users:
                await old.close()
        try:
            self.refresh_pins()
        except BlockingIOError:
            # bind() already persisted both generations; cleanup of unused pins
            # may wait without changing an already committed serving receipt.
            self.event("pin_refresh_deferred", identity)

    def authorized(self, headers):
        return hmac.compare_digest(headers.get("authorization", ""), f"Bearer {self.token}")

    def validate_query(self, parameters):
        allowed = {"q", "count", "page", "safe_search", "language"}
        if set(parameters) - allowed or any(len(values) != 1 for values in parameters.values()):
            raise ApiError(400, "unsupported_or_repeated_parameter")
        q = parameters.get("q", [""])[0].strip()
        try:
            count, page = int(parameters.get("count", ["10"])[0]), int(parameters.get("page", ["0"])[0])
        except ValueError:
            raise ApiError(400, "invalid_pagination")
        safe = parameters.get("safe_search", ["false"])[0]
        language = parameters.get("language", ["en"])[0]
        if not q or len(q.encode()) > 4096 or len(q.split()) > 16 or not 1 <= count <= 20 or not 0 <= page <= 20:
            raise ApiError(400, "query_budget_exceeded")
        if any(ord(char) < 32 for char in q):
            raise ApiError(400, "query_control_character")
        if safe not in {"true", "false"} or language not in {"en", "de", "pt", "es", "it", "fr", "zh-CN"}:
            raise ApiError(400, "unsupported_search_option")
        request = {"q": q, "count": count, "page": page, "safe_search": safe == "true"}
        if len(json.dumps({"id": "0"*36, **request}, ensure_ascii=False).encode()) + 1 > 8192:
            raise ApiError(400, "query_message_budget")
        return request, language

    async def query(self, parameters, headers):
        arrival = time.monotonic()
        traffic = headers.get("x-search-traffic", "user") if self.authorized(headers) else "user"
        if traffic not in self.traffic:
            traffic = "user"
        counter = self.traffic[traffic]
        counter["requests"] += 1
        try:
            request, language = self.validate_query(parameters)
        except ApiError:
            counter["rejected"] += 1
            raise
        while self.rate and self.rate[0] < arrival - 1:
            self.rate.popleft()
        group = self.current
        if self.draining or not group or not group.ready:
            counter["rejected"] += 1
            raise ApiError(503, "no_ready_generation" if not self.draining else "admission_draining")
        if len(self.rate) >= self.config["max_qps"] or self.admitted >= self.config["workers"] + self.config["max_queue"]:
            counter["rejected"] += 1
            raise ApiError(429, "admission_limit")
        self.rate.append(arrival)
        counter["admitted"] += 1
        counter["pending"] += 1
        self.admitted += 1
        group.users += 1
        request["id"] = str(uuid.uuid4())
        key = hashlib.sha256(json.dumps([group.digest, request["q"], request["count"], request["page"], request["safe_search"], language], ensure_ascii=False).encode()).hexdigest()
        use_cache = not (self.authorized(headers) and headers.get("x-search-cache") == "off")
        worker, queued, native_active, healthy = None, False, False, True
        try:
            async with asyncio.timeout(self.config["deadline_ms"] / 1000):
                cached = self.cache.get(key) if use_cache else None
                if cached and cached[0] > arrival:
                    result = json.loads(cached[1])
                    self.cache.move_to_end(key)
                    counter["cache_hits"] += 1
                    result.update(native_cpu_ms=0, allocated_bytes=0, native_wall_ms=0, stages_ms={"cache_lookup": (time.monotonic()-arrival)*1000})
                    cache = "hit"
                else:
                    counter["cache_misses"] += 1
                    self.queued += 1
                    queued = True
                    worker = await group.acquire()
                    self.queued -= 1
                    queued = False
                    self.active += 1
                    native_active = True
                    queue_ms = (time.monotonic() - arrival) * 1000
                    healthy = False
                    async with asyncio.timeout(self.config["replica_timeout_ms"]/1000):
                        result = await worker.execute(request)
                    healthy = True
                    result["stages_ms"]["queue"] = queue_ms
                    result["stages_ms"]["decode"] = max(0, (time.monotonic()-arrival)*1000 - queue_ms - result["native_wall_ms"])
                    cache = "miss"
                    if use_cache:
                        encoded = json.dumps(result, ensure_ascii=False).encode()
                        if len(encoded) <= self.config["cache_bytes"]:
                            if key in self.cache:
                                self.cache_size -= len(self.cache.pop(key)[1])
                            while self.cache and (len(self.cache) >= self.config["cache_entries"] or self.cache_size + len(encoded) > self.config["cache_bytes"]):
                                _, (_, removed) = self.cache.popitem(last=False)
                                self.cache_size -= len(removed)
                            self.cache[key] = (arrival + self.config["cache_seconds"], encoded)
                            self.cache_size += len(encoded)
                result.update(id=request["id"], version=1, generation=group.id, manifest_sha256=group.digest,
                              documents=group.manifest["documents"], cache=cache, outcome="complete",
                              traffic=traffic, shards={"expected": 1, "complete": 1, "partial": False},
                              routing={"node": worker.id if worker else None, "attempts": 1 if worker else 0, "generation": group.id},
                              language=language, language_scope="query text preserved; no language-only filtering",
                              wall_ms=(time.monotonic()-arrival)*1000)
                counter["complete"] += 1
                if not result["results"]:
                    counter["no_results"] += 1
                if traffic == "user":
                    self.samples.append((time.monotonic(), result["wall_ms"]))
                    self.stages = result["stages_ms"]
                    self.last_user_query_at = timestamp()
                return result
        except TimeoutError:
            counter["timeouts"] += 1
            error = ApiError(504, "replica_deadline" if native_active else "query_deadline")
            error.generation = group.id
            error.routing = {"node": worker.id if worker else None, "attempts": 1 if worker else 0, "generation": group.id}
            raise error
        except asyncio.CancelledError:
            counter["cancelled"] += 1
            raise
        except Exception as error:
            counter["failed"] += 1
            if isinstance(error, ApiError):
                raise
            raise ApiError(502, "native_query_failed", str(error)[:512])
        finally:
            if queued:
                self.queued -= 1
            if native_active:
                self.active -= 1
            self.admitted -= 1
            counter["pending"] -= 1
            group.users -= 1
            if worker:
                if not healthy:
                    worker.failures += 1
                    group.recover(worker, "query_interrupted_or_failed")
                await group.release(worker)
            if group.retired and not group.users:
                await group.close()

    def command(self, body):
        if set(body) - {"id", "action", "generation", "node"} or body.get("action") not in {"warm", "promote", "rollback", "drain", "resume", "drain_node", "resume_node", "replace_node"}:
            raise ApiError(400, "invalid_control")
        identity = str(uuid.UUID(body["id"]))
        encoded = json.dumps(body, sort_keys=True)
        previous = self.db.execute("SELECT request,state,error FROM commands WHERE id=?", (identity,)).fetchone()
        if previous:
            if previous[0] != encoded:
                raise ApiError(409, "idempotency_conflict")
            return {"id": identity, "action": body["action"], "state": previous[1], "error": previous[2]}
        if self.control_busy:
            raise ApiError(409, "generation_operation_busy")
        generation = body.get("generation")
        if generation:
            generation = str(uuid.UUID(generation))
        if body["action"] in {"warm", "promote"} and not generation:
            raise ApiError(400, "generation_required")
        node = body.get("node")
        if body["action"].endswith("_node"):
            if not isinstance(node, str) or not self.current or node not in {worker.id for worker in self.current.workers}:
                raise ApiError(409, "node_generation_not_serving")
        elif node is not None:
            raise ApiError(400, "unexpected_node")
        self.db.execute("INSERT INTO commands VALUES(?,?,?,?,?,?,?,?)", (identity, encoded, body["action"], generation, "accepted", None, timestamp(), None))
        self.db.commit()
        self.control_busy = True
        self.spawn(self.apply_command(identity, body["action"], generation, node))
        return {"id": identity, "action": body["action"], "state": "accepted"}

    async def apply_command(self, identity, action, generation, node=None):
        error, state = None, "applied"
        self.db.execute("UPDATE commands SET state='applying' WHERE id=?", (identity,))
        self.db.commit()
        try:
            if action == "warm":
                await self.warm(generation)
            elif action in {"promote", "rollback"}:
                generation = self.previous if action == "rollback" else generation
                if not generation:
                    raise ValueError("No retained rollback generation")
                await self.promote(generation, manual=True)
            elif action == "drain":
                self.draining = True
            elif action == "resume":
                self.draining = False
            elif action.endswith("_node"):
                group = self.current
                worker = next((worker for worker in group.workers if worker.id == node), None) if group else None
                if not worker:
                    raise ApiError(409, "node_generation_not_serving")
                await group.control(worker, action)
        except Exception as exception:
            error, state = str(exception)[:512], "failed"
            self.event("control_failed", error)
        finally:
            self.db.execute("UPDATE commands SET state=?,error=?,completed_at=? WHERE id=?", (state, error, timestamp(), identity))
            self.db.commit()
            self.control_busy = False

    def window(self):
        now = time.monotonic()
        values = sorted(value for at, value in self.samples if at >= now - 60)
        def percentile(p):
            return values[max(0, math.ceil(len(values)*p)-1)] if values else 0.
        span = min(60, max(1, now-self.last_refresh)) if not values else 60
        return {"qps": len(values)/span, "p50_ms": percentile(.5), "p95_ms": percentile(.95),
                "p99_ms": percentile(.99), "samples": len(values)}

    def snapshot(self):
        window = self.window()
        resources = sample(os.getpid())
        for group in self.groups:
            for worker in group.workers:
                if worker.process and worker.process.returncode is None:
                    value = sample(worker.process.pid)
                    for key in resources:
                        resources[key] += value[key]
        now = time.monotonic()
        delta = now-self.last_cpu_at
        if delta >= .5:
            self.cpu_percent = max(0, (resources["cpu_seconds"]-self.last_cpu)*100/delta)
            self.last_cpu, self.last_cpu_at = resources["cpu_seconds"], now
        resources["cpu_percent"] = self.cpu_percent
        ready = bool(self.current and self.current.ready and not self.draining)
        compatible = sum(worker.enabled and worker.validated and worker.state in {"ready", "busy"}
                         for worker in self.current.workers) if self.current else 0
        phase = "draining" if self.draining else "ready" if ready else "warming" if self.control_busy else "waiting"
        reason = "admission_draining" if self.draining else "serving_generation" if ready else "waiting_for_validated_generation"
        if ready and compatible < self.config["workers"]:
            phase, reason = "degraded", "replica_capacity_reduced"
        if self.last_error and not ready:
            phase, reason = "degraded", self.last_error
        generation = None if not self.current else {"id": self.current.id, "manifest_sha256": self.current.digest,
            "documents": self.current.manifest["documents"], "signals": self.current.manifest.get("signals", {}).get("graph_generation")}
        commands = [{"id": r[0], "action": r[1], "generation": r[2], "state": r[3], "error": r[4], "node": json.loads(r[5]).get("node")}
                    for r in self.db.execute("SELECT id,action,generation,state,error,request FROM commands ORDER BY created_at DESC LIMIT 32")]
        config = {key: str(value) if isinstance(value, Path) else value for key, value in self.config.items()}
        rows = [{**row, "selected": bool(self.current and row["id"] == self.current.id)} for row in self.generation_rows]
        return {"schema_version": 1, "component": "search-api", "run_id": self.run_id, "pid": os.getpid(),
                "started_at": self.started, "updated_at": timestamp(), "phase": phase, "reason": reason, "ready": ready,
                "generation": generation, "previous": self.previous, "desired": self.desired,
                "totals": {**dict.fromkeys(["requests", "admitted", "complete", "no_results", "failed", "timeouts", "cancelled", "rejected", "cache_hits", "cache_misses"], 0), **self.traffic["user"]},
                "traffic": {name: dict(value) for name, value in self.traffic.items()},
                "cache_bytes": self.cache_size, "window": window, "active": self.active, "queued": self.queued,
                "http": {"connections": self.connections, **dict(self.http_totals), "traffic": {key: dict(value) for key, value in self.http_traffic.items()}},
                "replicas": {"configured": self.config["workers"], "ready": sum(worker.eligible for worker in self.current.workers) if self.current else 0,
                             "compatible": compatible,
                             "policy": "one shard; complete results required; one execution per request; no speculative retry"},
                "resources": resources, "history": list(self.history), "stages_ms": self.stages,
                "configuration": config, "capabilities": {**self.capabilities, "payments": "planned_only", "public_access": False,
                    "protocol": 1, "internal_auth": "private worker pipes; authenticated local control", "query_options": ["q", "count", "page", "safe_search", "language"],
                    "http_compression": "gzip level 1; negotiated; bounded JSON", "http_connections": "sequential HTTP/1.1; bounded keep-alive; no pipelining",
                    "shard_policy": "all required; no partial results", "query_retries": 0},
                "nodes": [{"id": f"{group.id}:{worker.ordinal}", "pid": worker.process.pid, "generation": group.id,
                           "state": "warming" if worker.state == "busy" and not worker.validated else worker.state, "load_ms": worker.load_ms, "requests": worker.requests,
                           "enabled": worker.enabled, "failures": worker.failures, "recoveries": worker.recoveries,
                           "last_error": worker.last_error, "next_retry_at": worker.next_retry_at,
                           "generation_compatible": group is self.current and not group.retired,
                           "recovery_attempts_last_minute": sum(at > time.monotonic()-60 for at in worker.attempts)}
                          for group in self.groups for worker in group.workers if worker.process],
                "generations": rows, "commands": commands, "events": list(self.events), "last_probe": self.last_probe,
                "last_user_query_at": self.last_user_query_at}

    def status(self):
        return {"snapshot": self.snapshot(), "meta": {"read_at": timestamp(), "process_alive": True}}

    async def refresh(self):
        if self.control_busy:
            return
        target = self.desired if not self.current else None
        try:
            path = self.config["index_root"] / "generation-selector.json"
            selected = read_json(path, 65536) if path.exists() else {}
            incoming = selected.get("active")
            if not target and self.config["auto_promote"] and incoming != self.hold and incoming != (self.current.id if self.current else None):
                target = incoming
            if target and time.monotonic()-self.last_attempt.get(target, 0) >= 10:
                self.last_attempt[target] = time.monotonic()
                self.control_busy = True
                try:
                    await self.promote(target)
                finally:
                    self.control_busy = False
            rows = []
            for directory in sorted((self.config["index_root"] / "generations").glob("*"), key=lambda path: path.stat().st_mtime, reverse=True)[:32]:
                if (directory / "manifest.json").is_file():
                    value = read_json(directory / "manifest.json")
                    rows.append({"id": directory.name, "state": value.get("state", "unknown"), "documents": value.get("documents", 0), "created_at": value.get("created_at")})
            self.generation_rows = rows
            self.groups = [group for group in self.groups if not group.retired or group.users]
            self.refresh_pins()
        except (OSError, ValueError, RuntimeError) as error:
            self.last_error = str(error)[:256]
            self.event("generation_pending", self.last_error)

    async def project(self):
        while not self.stopping:
            snapshot = self.snapshot()
            self.history.append({"at": snapshot["updated_at"], "qps": snapshot["window"]["qps"],
                                 "p95_ms": snapshot["window"]["p95_ms"] if snapshot["window"]["samples"] else None,
                                 "rss_bytes": snapshot["resources"]["rss_bytes"]})
            atomic(self.directory / "status.json", {"snapshot": snapshot, "meta": {"read_at": timestamp(), "process_alive": True}}, durable=False)
            if self.config["scope"] == "local":
                atomic(ROOT / "tools/crawler-dashboard/dist/search-live.json", self.status(), durable=False)
            for group in self.groups:
                for worker in group.workers:
                    if worker.process and worker.process.returncode is None and sample(worker.process.pid)["rss_bytes"] > self.config["memory_guard_bytes"]:
                        await worker.close()
                        self.event("reader_memory_guard", group.id)
                    if worker.enabled and not worker.busy and not group.retired and (not worker.process or worker.process.returncode is not None):
                        if worker.repair_task is None or worker.repair_task.done():
                            group.recover(worker, "reader_process_exited")
            await asyncio.sleep(1)

    async def maintenance(self):
        while not self.stopping:
            await self.refresh()
            await asyncio.sleep(self.config["poll_seconds"])

    async def probe(self):
        await asyncio.sleep(5)
        while not self.stopping:
            if self.current and self.current.ready and not self.draining:
                writer = None
                try:
                    async with asyncio.timeout(self.config["deadline_ms"] / 1000 + .5):
                        reader, writer = await asyncio.open_connection(self.config["host"], self.config["port"])
                        wire = (f"GET /v1/web/search?q=search&count=1 HTTP/1.1\r\nHost: {self.config['host']}\r\n"
                                f"Authorization: Bearer {self.token}\r\nX-Search-Traffic: probe\r\n\r\n")
                        writer.write(wire.encode())
                        await writer.drain()
                        header = await reader.readuntil(b"\r\n\r\n")
                        status = int(header.split(b" ", 2)[1])
                        fields = dict(line.split(b":", 1) for line in header.split(b"\r\n")[1:] if b":" in line)
                        length = int(fields[b"Content-Length"].strip())
                        if length > 512 * 1024:
                            raise ValueError("Probe response budget")
                        value = json.loads(await reader.readexactly(length))
                        self.last_probe = {"at": timestamp(), "state": "passed" if status == 200 else "failed",
                            "status": status, "generation": value.get("generation"), "results": len(value.get("results", [])),
                            "wall_ms": value.get("wall_ms"), "code": value.get("code")}
                except Exception as error:
                    self.last_probe = {"at": timestamp(), "state": "failed", "error": str(error)[:256]}
                finally:
                    if writer:
                        writer.close()
                        try:
                            await writer.wait_closed()
                        except ConnectionError:
                            pass
            await asyncio.sleep(30)

    @staticmethod
    def encoding(header):
        """RFC 9110 q-values: never gzip a response explicitly refused by a client."""
        if not header:
            return False, True
        weights = {}
        for item in header.lower().split(','):
            parts = item.strip().split(';')
            coding, weight = parts[0].strip(), 1.
            if not coding:
                raise ApiError(400, "invalid_accept_encoding")
            for parameter in parts[1:]:
                key, separator, value = parameter.strip().partition('=')
                if key != 'q' or not separator:
                    raise ApiError(400, "invalid_accept_encoding")
                try:
                    weight = float(value)
                except ValueError:
                    raise ApiError(400, "invalid_accept_encoding")
                if not math.isfinite(weight) or not 0 <= weight <= 1:
                    raise ApiError(400, "invalid_accept_encoding")
            weights[coding] = weight
        gzip_weight = weights.get('gzip', weights.get('*', 0))
        identity_weight = weights.get('identity', 0 if weights.get('*') == 0 else 1)
        if gzip_weight <= 0 and identity_weight <= 0:
            raise ApiError(406, "encoding_not_acceptable")
        return gzip_weight > 0 and gzip_weight >= identity_weight, identity_weight > 0

    async def http(self, reader, writer):
        self.connections += 1
        self.http_totals['connections_opened'] += 1
        started = time.monotonic()
        try:
            if self.connections > self.config['max_connections']:
                await self.respond(writer, 429, {'code': 'connection_limit'})
                return
            for sequence in range(self.config['keepalive_requests']):
                operation = disconnect = None
                deadline = None
                headers, keepalive = {}, False
                try:
                    async with asyncio.timeout(min(self.config['keepalive_seconds'], max(.001, started+30-time.monotonic()))):
                        raw = await reader.readuntil(b'\r\n\r\n')
                        if len(raw) > 8192:
                            raise ApiError(431, 'headers_too_large')
                        lines = raw.decode('iso-8859-1').split('\r\n')
                        method, target, version = lines[0].split(' ')
                        if version != 'HTTP/1.1' or not target.startswith('/'):
                            raise ApiError(400, 'http_version_or_target')
                        for line in lines[1:]:
                            if not line:
                                continue
                            key, value = line.split(':', 1)
                            if key != key.strip() or not key or any(not (char.isascii() and (char.isalnum() or char in "!#$%&'*+-.^_`|~")) for char in key):
                                raise ApiError(400, 'invalid_header_name')
                            key = key.lower()
                            if key in headers or any(ord(char) < 32 and char != '\t' for char in value):
                                raise ApiError(400, 'invalid_or_duplicate_header')
                            headers[key] = value.strip()
                        if not headers.get('host') or 'transfer-encoding' in headers or 'expect' in headers:
                            raise ApiError(400, 'unsupported_framing')
                        length = int(headers.get('content-length', '0'))
                        if not 0 <= length <= 8192:
                            raise ApiError(413, 'body_limit')
                        if method == 'GET' and length:
                            raise ApiError(400, 'unexpected_get_body')
                        body = await reader.readexactly(length) if length else b''
                    compress, identity = self.encoding(headers.get('accept-encoding', ''))
                    url = urllib.parse.urlsplit(target)
                    if url.scheme or url.netloc or url.fragment:
                        raise ApiError(400, 'invalid_target')
                    keepalive = ('close' not in [value.strip() for value in headers.get('connection', '').lower().split(',')]
                                 and sequence+1 < self.config['keepalive_requests'] and time.monotonic()-started < 30)
                    self.http_totals['requests'] += 1
                    if sequence:
                        self.http_totals['reused_requests'] += 1
                    if method == 'GET' and url.path == '/healthz':
                        result, status = {'status': 'alive', 'component': 'search-api', 'pid': os.getpid()}, 200
                    elif method == 'GET' and url.path == '/readyz':
                        result = {'ready': self.snapshot()['ready'], 'generation': self.current.id if self.current else None}
                        status = 200 if result['ready'] else 503
                    elif method == 'GET' and url.path in {'/status', '/metrics', '/nodes', '/generations'}:
                        result, status = self.status(), 200
                    elif method == 'GET' and url.path == '/v1/web/search':
                        deadline = time.monotonic()+self.config['deadline_ms']/1000
                        operation = self.spawn(self.query(urllib.parse.parse_qs(url.query, keep_blank_values=True, max_num_fields=8), headers))
                        # Sequential HTTP/1.1 only. Bytes arriving during execution
                        # are pipelining (unsupported); EOF cancels native work.
                        disconnect = asyncio.create_task(reader.read(1))
                        done, _ = await asyncio.wait([operation, disconnect], return_when=asyncio.FIRST_COMPLETED)
                        if disconnect in done and operation not in done:
                            operation.cancel()
                            await asyncio.gather(operation, return_exceptions=True)
                            return
                        result, status = await operation, 200
                    elif method == 'POST' and url.path == '/control':
                        if not self.authorized(headers):
                            raise ApiError(403, 'control_forbidden')
                        if headers.get('content-type', '').split(';')[0] != 'application/json':
                            raise ApiError(415, 'control_json_required')
                        result, status = self.command(json.loads(body)), 202
                    else:
                        raise ApiError(404, 'route_not_found')
                    if disconnect:
                        disconnect.cancel()
                        await asyncio.gather(disconnect, return_exceptions=True)
                        disconnect = None
                    if not await self.respond(writer, status, result, deadline, keepalive, compress, identity):
                        return
                    if not keepalive:
                        return
                except ApiError as error:
                    await self.respond(writer, error.status, {'version': 1, 'outcome': 'rejected' if error.status in {429, 503} else 'failed',
                        'code': error.code, 'error': str(error), 'generation': getattr(error, 'generation', None),
                        'routing': getattr(error, 'routing', None), 'shards': {'expected': 1, 'complete': 0, 'partial': False}})
                    return
                except TimeoutError:
                    # An idle persistent connection ends silently; no fake query.
                    return
                except (ValueError, UnicodeError, TypeError, KeyError, AttributeError, asyncio.LimitOverrunError):
                    await self.respond(writer, 400, {'code': 'invalid_request', 'error': 'Invalid or oversized HTTP request'})
                    return
                except (asyncio.IncompleteReadError, ConnectionError):
                    return
                finally:
                    if disconnect:
                        disconnect.cancel()
                        await asyncio.gather(disconnect, return_exceptions=True)
                    if operation and not operation.done():
                        operation.cancel()
                        await asyncio.gather(operation, return_exceptions=True)
        finally:
            self.connections -= 1
            writer.close()
            try:
                async with asyncio.timeout(.5):
                    await writer.wait_closed()
            except (ConnectionError, TimeoutError):
                pass

    async def respond(self, writer, status, value, deadline=None, keepalive=False, compress=False, identity=True):
        encode, cpu = time.perf_counter(), time.process_time()
        data = json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(',', ':')).encode()
        serialization_ms, serialization_cpu = (time.perf_counter()-encode)*1000, (time.process_time()-cpu)*1000
        if len(data) > 512*1024:
            status, data, keepalive = 502, b'{"code":"response_budget"}', False
        raw_bytes, compression_ms, compression_cpu, coding = len(data), 0., 0., None
        if compress and (raw_bytes >= self.config['gzip_min_bytes'] or not identity):
            begin, cpu = time.perf_counter(), time.process_time()
            encoded = gzip.compress(data, compresslevel=1, mtime=0)
            compression_ms, compression_cpu = (time.perf_counter()-begin)*1000, (time.process_time()-cpu)*1000
            if len(encoded) < raw_bytes or not identity:
                data, coding = encoded, 'gzip'
        if deadline and time.monotonic() >= deadline:
            self.http_totals['response_deadlines'] += 1
            return False
        header = (f'HTTP/1.1 {status} Response\r\nContent-Type: application/json; charset=utf-8\r\n'
                  f'Content-Length: {len(data)}\r\nConnection: {"keep-alive" if keepalive else "close"}\r\n'
                  'Cache-Control: no-store\r\nVary: Accept-Encoding\r\nX-Content-Type-Options: nosniff\r\n'
                  f'Server-Timing: http_serialization;dur={serialization_ms:.4f}, compression;dur={compression_ms:.4f}\r\n')
        if keepalive:
            header += f'Keep-Alive: timeout={self.config["keepalive_seconds"]}, max={self.config["keepalive_requests"]}\r\n'
        if coding:
            header += 'Content-Encoding: gzip\r\n'
        if status in {429, 503}:
            header += 'Retry-After: 1\r\n'
        if isinstance(value, dict) and value.get('generation'):
            header += f'X-Search-Generation: {value["generation"]}\r\n'
        if isinstance(value, dict) and value.get('id'):
            header += f'X-Request-ID: {value["id"]}\r\n'
        wire = header.encode()+b'\r\n'+data
        writer.write(wire)
        try:
            async with asyncio.timeout(max(.001, deadline-time.monotonic()) if deadline else .5):
                await writer.drain()
        except (ConnectionError, TimeoutError):
            self.http_totals['response_aborted'] += 1
            return False
        traffic = value.get('traffic') if isinstance(value, dict) else None
        if traffic in self.http_traffic:
            counter = self.http_traffic[traffic]
            counter.update(responses=1, raw_bytes=raw_bytes, payload_bytes=len(data), wire_bytes=len(wire),
                           compressed=bool(coding), serialization_cpu_ms=serialization_cpu, compression_cpu_ms=compression_cpu)
            if traffic == 'user':
                self.stages.update(http_serialization=serialization_ms, compression=compression_ms)
        return True

    async def run(self):
        loop = asyncio.get_running_loop()
        stopped = asyncio.Event()
        for signum in [signal.SIGINT, signal.SIGTERM]:
            loop.add_signal_handler(signum, stopped.set)
        # Bind first: health and not-ready remain available while readers load.
        server = await asyncio.start_server(self.http, self.config["host"], self.config["port"], limit=8193)
        self.spawn(self.project())
        self.spawn(self.maintenance())
        self.spawn(self.probe())
        self.event("service_alive", "Private native readers; payments planned only")
        async with server:
            await stopped.wait()
        self.stopping, self.draining = True, True
        for task in list(self.tasks):
            task.cancel()
        await asyncio.gather(*list(self.tasks), return_exceptions=True)
        for group in self.groups:
            await group.close()
        self.db.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "configs/search/local.toml")
    parser.add_argument("--check-config", action="store_true")
    args = parser.parse_args()
    if args.check_config:
        config = configuration(args.config)
        print(json.dumps({"valid": True, "binary_sha256": config["binary_sha256"], "scope": config["scope"]}))
    else:
        service = Service(args.config)
        with (service.directory / "service.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            asyncio.run(service.run())


if __name__ == "__main__":
    main()
