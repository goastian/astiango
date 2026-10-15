#!/usr/bin/env python3
"""Archive immutable ingestion history, preserving selectors and consumer pins.

No capture, ledger, checkpoint or ACK is removed. Archives are verified before
retiring a working copy; restore never changes the serving selector.
"""
import argparse
from collections import Counter
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import signal
import subprocess
import tarfile
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]


def read(path, limit=8 * 1024 * 1024):
    with path.open("rb") as source:
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"Metadata exceeds budget: {path}")
    return json.loads(data)


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(f".{path.name}.{uuid.uuid4()}.tmp")
    with temp.open("w") as output:
        json.dump(value, output, separators=(",", ":"))
        output.flush()
        os.fsync(output.fileno())
    temp.replace(path)
    sync_directory(path.parent)


def digest_file(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def inside(path, root):
    resolved = path.resolve()
    if not resolved.is_relative_to(root.resolve()):
        raise ValueError(f"Path escapes storage root: {path}")
    return resolved


def tree_bytes(path):
    seen, total = set(), 0
    for folder, dirs, files in os.walk(path):
        for name in dirs + files:
            p = Path(folder, name)
            if p.is_symlink():
                raise ValueError(f"Unexpected symlink in storage: {p}")
        for name in files:
            stat = Path(folder, name).stat()
            key = (stat.st_dev, stat.st_ino)
            if key not in seen:
                seen.add(key)
                total += stat.st_size
        if len(seen) > 100_000:
            raise ValueError("Storage inventory exceeds budget")
    return total


def inventory(directory, maximum):
    files, total = {}, 0
    for folder, dirs, names in os.walk(directory):
        for name in dirs + names:
            if Path(folder, name).is_symlink():
                raise ValueError("Generation symlinks are unsupported")
        for name in names:
            if name.endswith(".lock"):
                continue
            p = Path(folder, name)
            stat = p.stat()
            total += stat.st_size
            if total > maximum or len(files) >= 20_000:
                raise ValueError("Generation exceeds archive budget")
            files[p.relative_to(directory).as_posix()] = {
                "size": stat.st_size, "sha256": digest_file(p)}
    manifest = read(directory / "manifest.json", 1024 * 1024)
    if manifest.get("state") != "ready" or manifest.get("version") not in (1, 2):
        raise ValueError("Only compatible, ready generations can be archived")
    for name, checksum in manifest.get("files", manifest.get("artifacts", {})).items():
        if files.get(name, {}).get("sha256") != checksum:
            raise ValueError(f"Generation artifact checksum mismatch: {name}")
    return files, total, manifest


def unpack(archive, expected, destination=None):
    """Streaming verification/extraction: no links, traversal or extra members."""
    child = subprocess.Popen(["zstd", "-q", "-d", "-c", str(archive)], stdout=subprocess.PIPE)
    found = set()
    try:
        with tarfile.open(fileobj=child.stdout, mode="r|") as stream:
            for member in stream:
                path = PurePosixPath(member.name)
                if path.is_absolute() or ".." in path.parts:
                    raise ValueError("Unsafe archive path")
                name = path.as_posix()
                if member.isdir():
                    continue
                if not member.isfile() or name in found or name not in expected:
                    raise ValueError(f"Unexpected archive member: {name}")
                entry = expected[name]
                if member.size != entry["size"]:
                    raise ValueError("Archive member size mismatch")
                digest = hashlib.sha256()
                with contextlib.ExitStack() as stack:
                    source = stack.enter_context(stream.extractfile(member))
                    output = None
                    if destination:
                        target = inside(destination / name, destination)
                        target.parent.mkdir(parents=True, exist_ok=True)
                        output = stack.enter_context(target.open("xb"))
                    for block in iter(lambda: source.read(1024 * 1024), b""):
                        digest.update(block)
                        if output:
                            output.write(block)
                    if output:
                        output.flush()
                        os.fsync(output.fileno())
                if digest.hexdigest() != entry["sha256"]:
                    raise ValueError(f"Archive payload checksum mismatch: {name}")
                found.add(name)
        # tar can stop at its end marker before consuming the zstd footer.
        padding = child.stdout.read(65537)
        if len(padding) > 65536 or any(padding):
            raise ValueError("Unexpected archive trailing payload")
        if child.wait() != 0 or found != set(expected):
            raise ValueError("Incomplete archive or decompression failure")
    finally:
        child.stdout.close()
        if child.poll() is None:
            child.terminate()
        child.wait()


class Retention:
    def __init__(self, path):
        self.path = path
        self.config = tomllib.loads(path.read_text())
        c = self.config
        if not (0 < c["target_watermark"] < c["high_watermark"] < 1):
            raise ValueError("Invalid retention watermarks")
        if c["keep_generations"] < 2 or c["minimum_age_seconds"] < 0:
            raise ValueError("Active and rollback generations must be retained")
        self.archive = inside(ROOT / c["archive_directory"], ROOT)
        self.archive.mkdir(parents=True, exist_ok=True)
        self.roots, self.quotas, self.configs = {}, {}, {}
        for component, key, suffix in [("indexer", "indexer_config", "index"), ("webgraph", "webgraph_config", "graph")]:
            config = tomllib.loads((ROOT / c[key]).read_text())
            self.configs[component] = config
            root = inside(ROOT / config["state_directory"] / suffix, ROOT)
            if self.archive.is_relative_to(root) or root.is_relative_to(self.archive):
                raise ValueError("Cold archive must be separate from working storage")
            if root.stat().st_dev != self.archive.stat().st_dev:
                raise ValueError("Atomic retirement requires the same filesystem")
            self.roots[component] = root
            self.quotas[component] = config.get("max_output_bytes", config.get("graph", {}).get("max_output_bytes"))
        self.active_pressure = set()
        self.last_error = None
        self.actions = []
        self.verified = {}

    def verification_key(self, archive, receipt):
        return tuple((p.stat().st_ino, p.stat().st_size, p.stat().st_mtime_ns) for p in (archive, receipt))

    def remember_verified(self, archive, receipt):
        if len(self.verified) >= 24:
            self.verified.pop(next(iter(self.verified)))
        self.verified[str(archive)] = self.verification_key(archive, receipt)

    @contextlib.contextmanager
    def locks(self, component=None):
        with contextlib.ExitStack() as stack:
            for name in ((component,) if component else ("indexer", "webgraph")):
                for filename in ["ingestion.lock", "retention.lock"]:
                    lock = stack.enter_context((self.roots[name] / filename).open("a"))
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield

    def pinned(self, only=None):
        pins = {component: set() for component in self.roots}
        for component, root in self.roots.items():
            selector = "graph-selector.json" if component == "webgraph" else "generation-selector.json"
            for name in [selector, "publication.json", "publication-journal.json"]:
                if not (root / name).exists():
                    continue
                data = read(root / name)
                selected = data.get("selector", data)
                pins[component].update(v for k in ["active", "previous"] if (v := selected.get(k)))
        pin_file = self.archive / "pins.json"
        if pin_file.exists():
            for component, ids in read(pin_file).items():
                pins[component].update(ids)
        serving_file = ROOT / self.config.get("serving_pins", "data/search-api/pins.json")
        if serving_file.exists():
            serving = read(serving_file)
            if serving.get("version") != 1:
                raise ValueError("Unknown serving pin contract; working copies retained")
            for component in pins:
                identities = serving.get(component, [])
                if not isinstance(identities, list) or len(identities) > 128:
                    raise ValueError("Serving pin budget exceeded; working copies retained")
                pins[component].update(str(uuid.UUID(identity)) for identity in identities)
        if only == "indexer":
            return pins
        # Every warm index generation can be loaded directly; preserve its graph.
        for manifest in (self.roots["indexer"] / "generations").glob("*/manifest.json"):
            binding = read(manifest).get("signals", {}).get("graph_generation")
            if binding:
                pins["webgraph"].add(binding["generation"])
        # Hot reload can already have pinned the next graph before index commit.
        current_config = tomllib.loads((ROOT / self.config["indexer_config"]).read_text())
        graph_path = current_config.get("ingestion", {}).get("graph_generation")
        if graph_path:
            pins["webgraph"].add(Path(graph_path).name)
        # A native index worker can still be finishing the previous configuration.
        job = self.roots["indexer"].parent / "current-job.toml"
        if job.exists():
            graph_path = tomllib.loads(job.read_text()).get("ingestion", {}).get("graph_generation")
            if graph_path:
                pins["webgraph"].add(Path(graph_path).name)
        return pins

    def candidates(self, component, pins):
        folder = self.roots[component] / "generations"
        dirs = []
        for p in folder.iterdir():
            if p.is_dir() and not p.is_symlink() and (p / "manifest.json").is_file():
                uuid.UUID(p.name)
                dirs.append(p)
        dirs.sort(key=lambda p: p.stat().st_mtime, reverse=True)
        return [p for p in reversed(dirs[self.config["keep_generations"]:])
                if p.name not in pins and time.time() - p.stat().st_mtime >= self.config["minimum_age_seconds"]]

    def stage(self, component, source):
        folder = self.archive / component
        folder.mkdir(exist_ok=True)
        archive, receipt = folder / f"{source.name}.tar.zst", folder / f"{source.name}.json"
        if receipt.exists():
            record = read(receipt)
            if self.verified.get(str(archive)) != self.verification_key(archive, receipt):
                if digest_file(archive) != record["archive_sha256"]:
                    raise ValueError("Cold archive checksum mismatch; working copy retained")
                unpack(archive, record["files"])
                self.remember_verified(archive, receipt)
            return record
        files, total, manifest = inventory(source, self.config["max_generation_bytes"])
        if tree_bytes(self.archive) + total > self.config["archive_quota_bytes"]:
            raise ValueError("Cold archive quota reached; working copies retained")
        if shutil.disk_usage(self.archive).free < self.config["disk_reserve_bytes"] + total:
            raise ValueError("Disk reserve reached; working copies retained")
        temporary = archive.with_suffix(".partial")
        subprocess.run(["tar", "--format=pax", "--hard-dereference", "--exclude=*.lock",
                        "-I", "zstd -q -T1 -3", "-cf", str(temporary), "-C", str(source), "."], check=True)
        unpack(temporary, files)
        with temporary.open("rb") as saved:
            os.fsync(saved.fileno())
        temporary.replace(archive)
        sync_directory(folder)
        record = {"version": 1, "component": component, "generation": source.name,
                  "state": "verified", "archive_sha256": digest_file(archive),
                  "archive_bytes": archive.stat().st_size, "source_bytes": total,
                  "manifest_sha256": files["manifest.json"]["sha256"],
                  "graph_binding": manifest.get("signals", {}).get("graph_generation"),
                  "files": files, "verified_at": time.time(), "capture_gc_allowed": False}
        atomic(receipt, record)
        self.remember_verified(archive, receipt)
        return record

    def retire(self, component, source, record):
        return bool(self.retire_many(component, [(source, record)]))

    def retire_many(self, component, records):
        folder = self.archive / component
        moved = []
        try:
            with self.locks(component):
                allowed = set(self.candidates(component, self.pinned(only=component)[component]))
                boundary = time.monotonic()
                for source, record in records:
                    if source not in allowed:
                        continue
                    if digest_file(source / "manifest.json") != record["manifest_sha256"]:
                        raise ValueError("Generation changed during archive preparation")
                    receipt = folder / f"{source.name}.json"
                    retired = folder / f"{source.name}.retiring"
                    record["state"] = "retiring"
                    # The archive and verified receipt are already durable.
                    # A .retiring tree is recoverable even if the receipt still
                    # says verified; do not fsync per item under the worker lock.
                    source.rename(retired)
                    moved.append((source, retired, record, receipt))
                    if time.monotonic() - boundary >= .025:
                        break
                if moved:
                    sync_directory(self.roots[component] / "generations")
                    sync_directory(folder)
        except BlockingIOError:
            return 0
        # The verified archive is durable before the redundant working copy moves.
        for source, retired, record, receipt in moved:
            shutil.rmtree(retired)
            record["state"] = "archived"
            atomic(receipt, record)
            self.actions = (self.actions + [{"at": time.time(), "component": component,
                                            "generation": source.name, "action": "archived"}])[-16:]
        return len(moved)

    def recover(self):
        for component in self.roots:
            folder = self.archive / component
            if not folder.exists():
                continue
            for retired in folder.glob("*.retiring"):
                identity = str(uuid.UUID(retired.name.removesuffix(".retiring")))
                receipt = folder / f"{identity}.json"
                record = read(receipt)
                archive = folder / f"{identity}.tar.zst"
                if digest_file(archive) != record["archive_sha256"]:
                    raise ValueError("Interrupted archive is corrupt; redundant copy retained")
                unpack(archive, record["files"])
                # A partially removed retired tree is also safe to finish.
                shutil.rmtree(retired)
                record["state"] = "archived"
                atomic(receipt, record)

    def restore(self, component, identity):
        identity = str(uuid.UUID(identity))
        folder = self.archive / component
        record = read(folder / f"{identity}.json")
        archive = folder / f"{identity}.tar.zst"
        if digest_file(archive) != record["archive_sha256"]:
            raise ValueError("Cold archive checksum mismatch")
        binding = record.get("graph_binding")
        if component == "indexer" and binding:
            graph = self.roots["webgraph"] / "generations" / binding["generation"]
            if not graph.exists():
                self.restore("webgraph", binding["generation"])
            if digest_file(graph / "manifest.json") != binding["manifest_sha256"]:
                raise ValueError("Restored graph binding mismatch")
        destination = self.roots[component] / "generations" / identity
        if destination.exists():
            raise ValueError("Restore destination already exists; selector unchanged")
        if shutil.disk_usage(folder).free < self.config["disk_reserve_bytes"] + record["source_bytes"]:
            raise ValueError("Insufficient restore disk reserve")
        temp = folder / f"{identity}.restore-{uuid.uuid4()}"
        temp.mkdir()
        unpack(archive, record["files"], temp)
        for directory, _, _ in os.walk(temp, topdown=False):
            sync_directory(Path(directory))
        with self.locks():
            if tree_bytes(self.roots[component]) + record["source_bytes"] >= self.quotas[component]:
                raise ValueError("Restore exceeds working quota; verified stage and archive retained")
            pins = read(self.archive / "pins.json") if (self.archive / "pins.json").exists() else {}
            pins[component] = sorted(set(pins.get(component, [])) | {identity})
            atomic(self.archive / "pins.json", pins)
            temp.rename(destination)
            sync_directory(destination.parent)
        return {"restored": identity, "component": component, "selector_changed": False}

    def tick(self):
        for component, root in self.roots.items():
            # Configuration already hot-reloads in both supervisors. Reflect the
            # same effective quota without restarting this maintenance process.
            key = "indexer_config" if component == "indexer" else "webgraph_config"
            current = tomllib.loads((ROOT / self.config[key]).read_text())
            suffix = "index" if component == "indexer" else "graph"
            if (ROOT / current["state_directory"] / suffix).resolve() != root:
                raise ValueError("Live quota reload cannot move working storage")
            self.quotas[component] = current.get("max_output_bytes", current.get("graph", {}).get("max_output_bytes"))
            used = tree_bytes(root)
            if used >= self.quotas[component] * self.config["high_watermark"]:
                try:
                    cleaned = self.prune_incomplete(component)
                    if cleaned["removed"]:
                        self.actions = (self.actions + [{"at": time.time(), "component": component,
                            "action": "uncommitted_scratch_removed", "count": cleaned["removed"], "bytes": cleaned["logical_bytes"]}])[-16:]
                        used = tree_bytes(root)
                except BlockingIOError:
                    pass
            if used >= self.quotas[component] * self.config["high_watermark"]:
                self.active_pressure.add(component)
            if used <= self.quotas[component] * self.config["target_watermark"]:
                self.active_pressure.discard(component)
            choices = self.candidates(component, self.pinned()[component])
            history_due = len(choices) > self.config.get("max_warm_generations", 12) - self.config["keep_generations"]
            if choices and (component in self.active_pressure or history_due):
                # Recent eligible history is usually the largest snapshot. Free
                # headroom promptly instead of spending hours on tiny early runs.
                ordered = list(reversed(choices))
                if component == "indexer":
                    selector = self.roots["webgraph"] / "graph-selector.json"
                    selected = read(selector) if selector.exists() else {}
                    online = {selected.get("active"), selected.get("previous")}
                    # Retiring obsolete index bindings releases graph pins too.
                    # Prefer those over many index snapshots of the same live graph.
                    obsolete = []
                    bindings = {}
                    for candidate in ordered:
                        binding = read(candidate / "manifest.json").get("signals", {}).get("graph_generation")
                        bindings[candidate] = binding["generation"] if binding else None
                        if binding and binding["generation"] not in online:
                            obsolete.append(candidate)
                    counts = Counter(bindings[p] for p in obsolete)
                    # Finish small dependency groups first. Retiring one of forty
                    # snapshots of one graph releases no graph space yet.
                    obsolete.sort(key=lambda p: counts[bindings[p]])
                    ordered = obsolete + [p for p in ordered if p not in obsolete]
                staged = [(source, self.stage(component, source)) for source in ordered[:8]]
                deadline = time.monotonic() + 3
                # Observe the short gap between native jobs without holding a
                # worker lock while waiting. Compression/verification is cached.
                while not self.retire_many(component, staged):
                    if time.monotonic() >= deadline:
                        break
                    time.sleep(.05)
        self.snapshot()

    def prune_incomplete(self, component):
        """Retire uncommitted projection scratch, never a published generation.

        Native prepare() creates UUID directories under the ingestion lock and
        writes manifest.json only after ledger projection, scores and sync pass.
        A stale directory without that manifest has no publication/consumer ACK.
        """
        root = self.roots[component]
        retired = []
        with self.locks(component):
            if not (root / "ledger.redb").is_file():
                raise ValueError("Durable ledger missing; incomplete projections retained")
            pinned = self.pinned(only=component)[component]
            for path in (root / "generations").iterdir():
                if not path.is_dir() or path.is_symlink() or path.name in pinned:
                    continue
                uuid.UUID(path.name)
                if (path / "manifest.json").exists() or not (path / "config.json").is_file():
                    continue
                if time.time() - path.stat().st_mtime < 600:
                    continue
                # Excludes an in-flight writer through the same native lock.
                total = tree_bytes(path)
                destination = root / (".incomplete-" + path.name)
                path.rename(destination)
                retired.append((destination, total))
            if retired:
                sync_directory(root / "generations")
                sync_directory(root)
        # Expensive unlinking is outside the writer lock. No archive is created.
        for path, total in retired:
            shutil.rmtree(path)
        return {"component": component, "scope": "uncommitted_generation_scratch_only",
                "removed": len(retired), "logical_bytes": sum(total for _, total in retired),
                "generations": [path.name.removeprefix(".incomplete-") for path, _ in retired],
                "published_generations_removed": 0, "ledgers_removed": 0}

    def snapshot(self):
        counts = {}
        for component, root in self.roots.items():
            archived = compressed = recoverable = 0
            for receipt in (self.archive / component).glob("*.json"):
                record = read(receipt)
                archived += int(record["state"] == "archived")
                compressed += record["archive_bytes"]
                recoverable += record["source_bytes"]
            counts[component] = {"working_bytes": tree_bytes(root), "working_quota_bytes": self.quotas[component],
                                 "archived_generations": archived,
                                 "archive_bytes": compressed,
                                 "recoverable_bytes": recoverable,
                                 "pinned_generations": len(self.pinned()[component])}
        value = {"version": 1, "updated_at": time.time(), "pid": os.getpid(),
                 "state": "blocked" if self.last_error else "running", "error": self.last_error,
                 "components": counts, "archive_quota_bytes": self.config["archive_quota_bytes"],
                 "disk_free_bytes": shutil.disk_usage(self.archive).free,
                 "disk_reserve_bytes": self.config["disk_reserve_bytes"], "actions": self.actions,
                 "capture_gc_allowed": False}
        atomic(self.archive / "status.json", value)
        dist = ROOT / "tools/crawler-dashboard/dist"
        if dist.exists():
            atomic(dist / "ingestion-retention-live.json", value)
        return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "configs/ingestion/retention-local.toml")
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--restore", nargs=2, metavar=("COMPONENT", "GENERATION"))
    parser.add_argument("--prune-incomplete", choices=["indexer", "webgraph"])
    args = parser.parse_args()
    retention = Retention(args.config)
    if args.prune_incomplete:
        result = retention.prune_incomplete(args.prune_incomplete)
        print(json.dumps(result))
        return
    with (retention.archive / "maintenance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        retention.recover()
        if args.restore:
            component, identity = args.restore
            if component not in retention.roots:
                parser.error("Unknown component")
            print(json.dumps(retention.restore(component, identity)))
            return
        running = True
        def stop(*_):
            nonlocal running
            running = False
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        while running:
            try:
                retention.last_error = None
                retention.tick()
            except Exception as error:
                retention.last_error = str(error)[:512]
                retention.snapshot()
            if not args.run:
                print(json.dumps(retention.snapshot()))
                break
            for _ in range(retention.config["poll_seconds"] * 10):
                if not running:
                    break
                time.sleep(.1)


if __name__ == "__main__":
    main()
