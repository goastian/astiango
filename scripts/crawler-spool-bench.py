#!/usr/bin/env python3
"""C08 retained-corpus writes, process-crash recovery and controlled S3 publication."""
import argparse
from collections import Counter, defaultdict
import gzip
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import threading
import time
from urllib.parse import parse_qs, urlsplit
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("c01_runner", ROOT / "scripts/crawler-bench.py")
C01 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C01)
BINARY = ROOT / "target/release/examples/crawler_spool_bench"
METRICS = ("pages_per_second", "cpu_ms_per_page", "compressed_bytes_per_page", "sync_calls_per_page",
           "files_per_page", "rss_peak_bytes", "duration_ms", "fds_peak")
CRASH_POINTS = ("create", "append_before", "append_after", "data_sync_before", "data_sync_after",
                "journal_append_before", "journal_sync_before", "journal_sync_after", "seal_before", "seal_after")


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            value.update(block)
    return value.hexdigest()


def warcs(directory):
    result = {}
    for path in sorted(Path(directory).rglob("*.warc.gz")):
        current = None
        with gzip.open(path, "rb") as stream:
            while True:
                version = stream.readline()
                if not version:
                    break
                if version.rstrip() != b"WARC/1.0":
                    raise ValueError(f"Invalid WARC version: {path}")
                headers = {}
                while (line := stream.readline()) != b"\r\n":
                    if not line or len(line) > 16384 or b":" not in line:
                        raise ValueError("Invalid WARC header")
                    key, value = line.decode().split(":", 1)
                    headers[key.lower()] = value.strip()
                length = int(headers["content-length"])
                kind = headers["warc-type"]
                if kind == "request":
                    current = headers["warc-target-uri"]
                    if current in result:
                        raise ValueError("Duplicate stored fixture URL")
                if kind == "response":
                    if current is None or stream.read(4) != b"\r\n\r\n":
                        raise ValueError("Response lost its matching request")
                    length -= 4
                value = hashlib.sha256()
                remaining = length
                while remaining:
                    block = stream.read(min(65536, remaining))
                    if not block:
                        raise ValueError("Truncated WARC body")
                    value.update(block)
                    remaining -= len(block)
                if kind == "response":
                    result[current] = value.hexdigest()
                if stream.read(4) != b"\r\n\r\n":
                    raise ValueError("Invalid WARC record terminator")
    return result


def verify_archive(directory, expected=None, segments=None):
    records = warcs(directory)
    if expected is not None and records != {row["url"]: row["sha256"] for row in expected}:
        raise ValueError("Stored WARC URLs or bodies differ from the retained input")
    if segments is not None:
        if sum(row["records"] for row in segments) != len(records):
            raise ValueError("Segment durable counters differ from WARC records")
        for segment in segments:
            path = Path(directory) / "segments" / segment["file"]
            if path.stat().st_size != segment["bytes"] or digest(path) != segment["sha256"]:
                raise ValueError("Sealed segment checksum/size mismatch")
            journal = path.with_name(segment["id"] + ".commit")
            end = 0
            for line in journal.read_text().splitlines():
                entry = json.loads(line)
                encoded = json.dumps(entry["receipts"], separators=(",", ":"), ensure_ascii=False).encode()
                if entry["start"] != end or hashlib.sha256(encoded).hexdigest() != entry["sha256"]:
                    raise ValueError("Commit journal checksum or ordering mismatch")
                with path.open("rb") as stream:
                    for receipt in entry["receipts"]:
                        if receipt["start"] != end:
                            raise ValueError("Committed ranges are not contiguous")
                        stream.seek(end)
                        value = hashlib.sha256()
                        remaining = receipt["end"] - end
                        while remaining:
                            block = stream.read(min(65536, remaining))
                            if not block:
                                raise ValueError("Committed range is missing")
                            value.update(block)
                            remaining -= len(block)
                        if value.hexdigest() != receipt["sha256"]:
                            raise ValueError("Durable receipt checksum mismatch")
                        end = receipt["end"]
                if entry["end"] != end:
                    raise ValueError("Commit boundary mismatch")
            if end != segment["bytes"]:
                raise ValueError("Segment contains bytes outside durable commits")
    return records


def run_child(config, path, expected_exit=0, timeout=360):
    C01.atomic_json(path, config)
    peaks = {}
    start = time.monotonic()
    log_path = path.with_suffix(".log")
    with log_path.open("wb") as log:
        process = subprocess.Popen([str(BINARY), "--config", str(path)], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        try:
            while True:
                for key, value in C01.proc_sample(process.pid).items():
                    peaks[key] = max(peaks.get(key, 0), value)
                pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    break
                if time.monotonic() - start > timeout:
                    raise TimeoutError(f"C08 case timed out: {path}")
                time.sleep(0.01)
        finally:
            if process.returncode is None:
                process.kill()
                process.wait()
    if process.returncode != expected_exit:
        raise RuntimeError(f"C08 exited {process.returncode}, expected {expected_exit}: {log_path}")
    result = json.loads(log_path.read_text()) if expected_exit == 0 else {}
    result.update(rss_peak_bytes=usage.ru_maxrss * 1024, fds_peak=peaks.get("fds", 0),
                  process_cpu_seconds=usage.ru_utime + usage.ru_stime,
                  process_wall_ms=(time.monotonic() - start) * 1000,
                  configuration_path=str(path), log_path=str(log_path), exit_code=process.returncode)
    return result


class S3Fixture:
    def __init__(self, directory):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True)
        self.mode = "healthy"
        self.uploads = {}
        self.objects = {}
        self.requests = Counter()
        self.lock = threading.Lock()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_):
                pass

            def send(self, status, body=b"", headers=None):
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                for key, value in (headers or {}).items():
                    self.send_header(key, value)
                self.end_headers()
                if self.command != "HEAD":
                    try:
                        self.wfile.write(body)
                    except (BrokenPipeError, ConnectionResetError):
                        with fixture.lock:
                            fixture.requests["client_disconnect"] += 1

            def parts(self):
                url = urlsplit(self.path)
                with fixture.lock:
                    fixture.requests[self.command] += 1
                return url.path, parse_qs(url.query, keep_blank_values=True)

            def read_body(self):
                size = int(self.headers.get("Content-Length", "0"))
                if size > 6 * 1024 * 1024:
                    raise ValueError("Fixture upload exceeded one bounded part")
                body = self.rfile.read(size)
                if len(body) != size:
                    raise ValueError("Truncated fixture upload")
                return body

            def do_HEAD(self):
                key, _ = self.parts()
                if fixture.mode == "down":
                    return self.send(503)
                row = fixture.objects.get(key)
                if row is None:
                    return self.send(404)
                checksum = "0" * 64 if fixture.mode == "head_mismatch" else row["sha256"]
                self.send_response(200)
                self.send_header("Content-Length", str(Path(row["path"]).stat().st_size))
                self.send_header("x-amz-meta-sha256", checksum)
                self.send_header("ETag", '"' + row["sha256"] + '"')
                self.end_headers()

            def do_PUT(self):
                key, query = self.parts()
                body = self.read_body()
                if fixture.mode == "down":
                    return self.send(503, b"<Error><Code>ServiceUnavailable</Code></Error>")
                if "uploadId" in query:
                    upload = fixture.uploads[query["uploadId"][0]]
                    number = int(query["partNumber"][0])
                    path = fixture.directory / (query["uploadId"][0] + f".part{number}")
                    path.write_bytes(body)
                    upload["parts"][number] = str(path)
                else:
                    path = fixture.directory / (hashlib.sha256(key.encode()).hexdigest() + ".object")
                    path.write_bytes(body)
                    fixture.objects[key] = {"path": str(path), "sha256": self.headers.get("x-amz-meta-sha256")}
                if fixture.mode in ["slow", "slow_online"]:
                    time.sleep(0.2 if fixture.mode == "slow" else 2)
                self.send(200, headers={"ETag": '"' + hashlib.md5(body).hexdigest() + '"'})

            def do_POST(self):
                key, query = self.parts()
                self.read_body()
                if fixture.mode == "down":
                    return self.send(503, b"<Error><Code>ServiceUnavailable</Code></Error>")
                if "uploads" in query:
                    upload_id = str(uuid.uuid4())
                    fixture.uploads[upload_id] = {"key": key, "parts": {}, "sha256": self.headers.get("x-amz-meta-sha256")}
                    return self.send(200, f"<InitiateMultipartUploadResult><Bucket>fixture</Bucket><Key>{key.lstrip('/').split('/', 1)[1]}</Key><UploadId>{upload_id}</UploadId></InitiateMultipartUploadResult>".encode())
                if fixture.mode == "completion_error":
                    return self.send(200, b"<Error><Code>InternalError</Code><Message>controlled failure</Message></Error>")
                upload = fixture.uploads.pop(query["uploadId"][0])
                path = fixture.directory / (hashlib.sha256(key.encode()).hexdigest() + ".object")
                with path.open("wb") as output:
                    for number in sorted(upload["parts"]):
                        with Path(upload["parts"][number]).open("rb") as source:
                            shutil.copyfileobj(source, output, 65536)
                fixture.objects[key] = {"path": str(path), "sha256": upload["sha256"]}
                self.send(200, f"<CompleteMultipartUploadResult><Location>local</Location><Bucket>fixture</Bucket><Key>{key}</Key><ETag>fixture</ETag></CompleteMultipartUploadResult>".encode())

            def do_DELETE(self):
                _, query = self.parts()
                if fixture.mode == "down":
                    return self.send(503, b"<Error><Code>ServiceUnavailable</Code></Error>")
                fixture.uploads.pop(query.get("uploadId", [""])[0], None)
                self.send(204)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def config(self, folder):
        return dict(bucket="fixture", folder=folder, access_key="local-fixture", secret_key="local-fixture",
                    endpoint=f"http://127.0.0.1:{self.server.server_port}")

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(5)


def base_config(directory, corpus, **updates):
    spool = dict(directory=str(directory), segment_bytes=128 * 1024 * 1024, segment_records=65536,
                 max_spool_bytes=2 * 1024 * 1024 * 1024, max_segments=256, gzip_level=3,
                 commit_records=16, commit_wait_ms=10, rotate_seconds=300, upload_retry_seconds=1,
                 upload_timeout_seconds=20, gc_enabled=False, retention_seconds=86400)
    spool.update(updates.pop("spool", {}))
    return dict(action="write", mode="segments", spool=spool, corpus_files=corpus, pages=128,
                body_bytes=0, fault_point=None, s3=None) | updates


def source_version():
    paths = [ROOT / "Cargo.lock", ROOT / "crates/core/Cargo.toml", Path(__file__),
             ROOT / "crates/core/examples/crawler_spool_bench.rs", ROOT / "crates/core/src/warc.rs",
             ROOT / "crates/core/src/config/mod.rs", ROOT / "tests/crawler-spool-bench/verify_report.py"]
    paths += sorted((ROOT / "crates/core/src/crawler").glob("*.rs"))
    return {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "files": {str(path.relative_to(ROOT)): digest(path) for path in paths},
            "binary_sha256": digest(BINARY)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "data/crawler-spool-bench")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    build = ["cargo", "build", "--locked", "--offline", "--release", "--no-default-features",
             "-p", "astiango", "--features", "crawler-bench", "--example", "crawler_spool_bench"]
    if not args.skip_build:
        subprocess.run(build, cwd=ROOT, check=True,
                       env=os.environ | {"CFLAGS": "-std=gnu17", "CARGO_PROFILE_RELEASE_STRIP": "symbols"})
    metadata, bodies = C01.load_corpus()
    corpus = [str((ROOT / row["path"]).resolve()) for row in metadata["files"]]
    run_id = str(uuid.uuid4())
    run_root = output / "runs" / run_id
    run_root.mkdir(parents=True)
    checks, runs = [], []
    configurations = []
    for group in [1, 16]:
        for mode, level in [("legacy_page", 9), ("segments", 9), ("segments", 3)]:
            configurations.append((f"corpus_group_{group}", mode, level, 128, 16 if args.smoke else 128, 0, group))
    for level in [1, 6]:
        configurations.append(("corpus_group_16", "segments", level, 128, 16 if args.smoke else 128, 0, 16))
    if not args.smoke:
        for size in [64, 128, 256]:
            configurations.append((f"rotation_{size}", "segments", 3, size, int(size * 1024 * 1024 / (2 * 1024 * 1024 * .75)) + 12, 2 * 1024 * 1024, 4))
        for pages in [160, 320]:
            configurations.append((f"archive_scale_{pages}", "segments", 3, 64, pages, 2 * 1024 * 1024, 4))
    for repetition in [1, 2, 3]:
        for scenario, mode, level, size, pages, body_bytes, group in configurations:
            directory = run_root / f"{scenario}-{mode}-gzip{level}-r{repetition}"
            config = base_config(directory, corpus, mode=mode, pages=pages, body_bytes=body_bytes,
                                 spool=dict(gzip_level=level, segment_bytes=size * 1024 * 1024, commit_records=group))
            result = run_child(config, directory.with_suffix(".json"))
            verify_archive(directory, result["expected"], result["segments"] if mode == "segments" else None)
            if scenario.startswith("rotation_") and result["files"] < 2:
                raise ValueError("Size rotation fixture did not rotate")
            if mode == "segments" and any(segment["bytes"] > config["spool"]["segment_bytes"] for segment in result["segments"]):
                raise ValueError("Segment exceeded its configured size")
            result.update(scenario=scenario, mode=mode, gzip_level=level, segment_bytes=size * 1024 * 1024,
                          repetition=repetition, archive_path=str(directory), archive_verified=True)
            runs.append(result)
            print(f"C08 {scenario}/{mode}/gzip{level}/r{repetition}: {result['pages_per_second']:.2f} pages/s, {result['rss_peak_bytes']/1048576:.2f} MiB", flush=True)
    for repetition in [1, 2, 3]:
        for point in CRASH_POINTS:
            directory = run_root / f"crash-{point}-r{repetition}"
            config = base_config(directory, corpus, action="fault", pages=2, fault_point=point)
            result = run_child(config, directory.with_suffix(".json"), expected_exit=86)
            acked = [json.loads(line) for line in (directory / "acknowledged.jsonl").read_text().splitlines()]
            config.update(action="recover", fault_point=None)
            recovered = run_child(config, directory.with_name(directory.name + "-recover.json"))
            records = verify_archive(directory, segments=recovered["segments"])
            receipts = [receipt for segment in recovered["segments"] for line in (directory / "segments" / (segment["id"] + ".commit")).read_text().splitlines() for receipt in json.loads(line)["receipts"]]
            if not acked or not all(receipt in receipts for receipt in acked):
                raise ValueError("A locally confirmed result disappeared after a process crash")
            if len(records) < len(acked):
                raise ValueError("Recovered WARC lost acknowledged documents")
            checks.append(dict(name=f"crash_{point}", repetition=repetition, passed=True,
                               archive_path=str(directory), acked=len(acked), recovered=len(records), exit_code=result["exit_code"]))
    fixture = S3Fixture(run_root / "s3-server")
    try:
        for repetition in [1, 2, 3]:
            for mode in ["down", "completion_error", "head_mismatch", "slow"]:
                directory = run_root / f"s3-{mode}-r{repetition}"
                config = base_config(directory, corpus, s3=fixture.config(directory.name), pages=12,
                                     body_bytes=2 * 1024 * 1024)
                prepared = run_child(config, directory.with_suffix(".json"))
                fixture.mode = mode
                config.update(action="publish")
                first = run_child(config, directory.with_name(directory.name + "-publish.json"))
                if mode != "slow" and not all(row["state"] == "pending" for row in first["segments"]):
                    raise ValueError("S3 failure was incorrectly confirmed as published")
                verify_archive(directory, prepared["expected"], first["segments"])
                fixture.mode = "healthy"
                restored = run_child(config, directory.with_name(directory.name + "-restore.json"))
                if not all(row["state"] == "published" for row in restored["segments"]):
                    raise ValueError("S3 recovery did not publish all retained segments")
                for segment in restored["segments"]:
                    key = "/fixture/" + config["s3"]["folder"] + "/" + segment["file"]
                    if digest(fixture.objects[key]["path"]) != segment["sha256"]:
                        raise ValueError("Server received different bytes than the locally durable segment")
                if max(first["stats"]["upload_buffer_peak_bytes"], restored["stats"]["upload_buffer_peak_bytes"]) > 5 * 1024 * 1024:
                    raise ValueError("Uploader exceeded one bounded part")
                checks.append(dict(name=f"s3_{mode}", repetition=repetition, passed=True,
                                   archive_path=str(directory), first=first, restored=restored))
            for point in ["upload_after_part", "upload_after_transfer", "publish_confirm_before", "publish_confirm_after"]:
                directory = run_root / f"s3-crash-{point}-r{repetition}"
                config = base_config(directory, corpus, s3=fixture.config(directory.name), pages=12,
                                     body_bytes=2 * 1024 * 1024)
                prepared = run_child(config, directory.with_suffix(".json"))
                config.update(action="publish", fault_point=point)
                run_child(config, directory.with_name(directory.name + "-crash.json"), expected_exit=86)
                config["fault_point"] = None
                restored = run_child(config, directory.with_name(directory.name + "-restore.json"))
                verify_archive(directory, prepared["expected"], restored["segments"])
                if not all(row["state"] == "published" for row in restored["segments"]):
                    raise ValueError("Upload crash recovery failed")
                checks.append(dict(name=f"s3_crash_{point}", repetition=repetition, passed=True,
                                   archive_path=str(directory), restored=restored))
            for mode in ["down", "slow"]:
                directory = run_root / f"s3-{mode}-online-r{repetition}"
                config = base_config(directory, corpus, action="online", pages=32,
                                     s3=fixture.config(directory.name), spool=dict(segment_records=4))
                fixture.mode = "slow_online" if mode == "slow" else mode
                online = run_child(config, directory.with_suffix(".json"))
                verify_archive(directory, online["expected"], online["segments"])
                if online["stats"]["records_durable"] != 32 or (mode == "down" and online["stats"]["upload_errors"] == 0):
                    raise ValueError("Actual async writer did not preserve local ACKs while S3 was unavailable")
                fixture.mode = "healthy"
                config["action"] = "publish"
                restored = run_child(config, directory.with_name(directory.name + "-restore.json"))
                if not all(row["state"] == "published" for row in restored["segments"]):
                    raise ValueError("Actual writer outbox did not recover")
                checks.append(dict(name=f"s3_{mode}_online", repetition=repetition, passed=True, archive_path=str(directory),
                                   online=online, restored=restored))
        remote_checksums = {key: digest(row["path"]) for key, row in fixture.objects.items()}
        for check in checks:
            if "restored" not in check:
                continue
            for segment in check["restored"]["segments"]:
                key = "/fixture/" + Path(check["archive_path"]).name + "/" + segment["file"]
                if remote_checksums.get(key) != segment["sha256"]:
                    raise ValueError("Remote object differs from a locally confirmed segment")
            check["remote_checksums_verified"] = True
        server = dict(requests=dict(fixture.requests), objects=len(fixture.objects), remaining_multipart=len(fixture.uploads),
                      endpoint=f"http://127.0.0.1:{fixture.server.server_port}", remote_checksums=remote_checksums)
    finally:
        fixture.close()
    grouped = defaultdict(list)
    for run in runs:
        grouped[(run["scenario"], run["mode"], run["gzip_level"], run["segment_bytes"])].append(run)
    summaries = [dict(scenario=scenario, mode=mode, gzip_level=level, segment_bytes=size, repetitions=3,
                      metrics={key: C01.distribution([row[key] for row in rows]) for key in METRICS})
                 for (scenario, mode, level, size), rows in grouped.items()]
    comparisons = []
    for group in [1, 16]:
        scenario = f"corpus_group_{group}"
        before = {r["repetition"]: r for r in runs if r["scenario"] == scenario and r["mode"] == "legacy_page"}
        for level in [3, 9]:
            after = {r["repetition"]: r for r in runs if r["scenario"] == scenario and r["mode"] == "segments" and r["gzip_level"] == level}
            comparisons.append(dict(scenario=scenario, gzip_level=level, equivalent=True,
                throughput_gain_pct=C01.distribution([(after[n]["pages_per_second"]/before[n]["pages_per_second"]-1)*100 for n in [1,2,3]]),
                sync_reduction_pct=C01.distribution([(1-after[n]["sync_calls"]/before[n]["sync_calls"])*100 for n in [1,2,3]]),
                files_reduction_pct=C01.distribution([(1-after[n]["files"]/before[n]["files"])*100 for n in [1,2,3]])))
    report = dict(schema_version=1, kind="crawler_spool_benchmark", generated_at=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                  configuration=dict(acceptance_profile="c08", repetitions=3, smoke=args.smoke, run_id=run_id,
                                     corpus=metadata, hardware=C01.hardware(), code=source_version(),
                                     command=["python3", "scripts/crawler-spool-bench.py"] + sys.argv[1:]),
                  acceptance=dict(passed=True, full_profile=not args.smoke, crash_cases=len(CRASH_POINTS)*3,
                                  publication_cases=30, stress_external_requests=0),
                  runs=runs, summaries=summaries, comparisons=comparisons, checks=checks, server=server)
    C01.atomic_json(output / "report.json", report)
    print(json.dumps({"report": str(output / "report.json"), "perf_runs": len(runs), "recovery_checks": len(checks), "passed": True}))


if __name__ == "__main__":
    main()
