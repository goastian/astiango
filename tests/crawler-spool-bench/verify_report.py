#!/usr/bin/env python3
"""Recompute C08 measurements and reconcile durable and remote artifacts."""
import argparse
from collections import defaultdict
import importlib.util
import json
import math
from pathlib import Path
import statistics
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("spool_runner", ROOT / "scripts/crawler-spool-bench.py")
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def distribution(values):
    require(bool(values) and all(math.isfinite(n) for n in values), "Invalid measurement")
    median = statistics.median(values)
    return dict(median=median, min=min(values), max=max(values),
                mad=statistics.median(abs(n - median) for n in values))


def same_distribution(actual, values):
    expected = distribution(values)
    require(set(actual) == set(expected) and all(
        math.isclose(actual[key], value, rel_tol=1e-9, abs_tol=1e-9)
        for key, value in expected.items()), "Published distribution differs from its repetitions")


def verify(report, archives=False, allow_smoke=False):
    require(report["kind"] == "crawler_spool_benchmark" and report["schema_version"] == 1, "Unknown report")
    config = report["configuration"]
    require(config["acceptance_profile"] == "c08" and config["repetitions"] == 3, "Wrong acceptance profile")
    require(allow_smoke or not config["smoke"], "A smoke report cannot complete C08")
    require(report["acceptance"]["passed"] and report["acceptance"]["full_profile"] == (not config["smoke"]), "Acceptance is incomplete")
    require(report["acceptance"]["stress_external_requests"] == 0, "External stress is forbidden")
    for path, checksum in config["code"]["files"].items():
        require(BENCH.digest(ROOT / path) == checksum, "Source fingerprint changed: " + path)
    require(BENCH.digest(BENCH.BINARY) == config["code"]["binary_sha256"], "Benchmark executable changed")
    for fixture in config["corpus"]["files"]:
        path = ROOT / fixture["path"]
        require(path.stat().st_size == fixture["bytes"] and BENCH.digest(path) == fixture["sha256"], "Retained corpus changed")
    groups = defaultdict(list)
    for row in report["runs"]:
        require(row["exit_code"] == 0 and row["archive_verified"], "A performance run was not verified")
        require(all(math.isfinite(row[key]) and row[key] >= 0 for key in BENCH.METRICS), "Invalid run metrics")
        require(row["rss_peak_bytes"] <= 384 * 1024 * 1024, "Archive RSS exceeded its 384 MiB profile")
        require(row["fds_peak"] <= 64, "Archive FD count exceeded its profile")
        expected_key = (row["scenario"], row["mode"], row["gzip_level"], row["segment_bytes"])
        groups[expected_key].append(row)
        require(row["pages"] > 0 and math.isclose(row["pages_per_second"], row["pages"] * 1000 / row["duration_ms"]), "Invalid throughput")
        require(row["sync_calls_per_page"] == row["sync_calls"] / row["pages"] and
                row["files_per_page"] == row["files"] / row["pages"], "Invalid per-page counters")
        if row["mode"] == "segments":
            require(row["stats"]["records_durable"] == row["pages"], "Durable count differs from accepted pages")
            require(sum(s["records"] for s in row["segments"]) == row["pages"], "Segment count differs from accepted pages")
            for segment in row["segments"]:
                require(segment["bytes"] <= row["segment_bytes"], "Segment exceeded configured limit")
                path = Path(row["archive_path"]) / "segments" / segment["file"]
                require(path.stat().st_size == segment["bytes"] and BENCH.digest(path) == segment["sha256"], "Segment checksum mismatch")
        if archives:
            BENCH.verify_archive(row["archive_path"], row["expected"], row.get("segments") if row["mode"] == "segments" else None)
    require(len(groups) == len(report["summaries"]), "Duplicate or missing summary")
    for summary in report["summaries"]:
        key = (summary["scenario"], summary["mode"], summary["gzip_level"], summary["segment_bytes"])
        rows = groups[key]
        require(summary["repetitions"] == 3 and sorted(r["repetition"] for r in rows) == [1, 2, 3], "Missing repetition")
        for metric in BENCH.METRICS:
            same_distribution(summary["metrics"][metric], [row[metric] for row in rows])
    for comparison in report["comparisons"]:
        rows = [r for r in report["runs"] if r["scenario"] == comparison["scenario"]]
        before = {r["repetition"]: r for r in rows if r["mode"] == "legacy_page"}
        after = {r["repetition"]: r for r in rows if r["mode"] == "segments" and r["gzip_level"] == comparison["gzip_level"]}
        require(comparison["equivalent"], "Non-equivalent corpus comparison")
        for n in [1, 2, 3]:
            require(before[n]["expected"] == after[n]["expected"], "Compared runs changed accepted documents")
        same_distribution(comparison["throughput_gain_pct"], [(after[n]["pages_per_second"] / before[n]["pages_per_second"] - 1) * 100 for n in [1, 2, 3]])
        for metric, counter in [("sync_reduction_pct", "sync_calls"), ("files_reduction_pct", "files")]:
            same_distribution(comparison[metric], [(1 - after[n][counter] / before[n][counter]) * 100 for n in [1, 2, 3]])
    cases = defaultdict(list)
    for row in report["checks"]:
        require(row["passed"], "Recovery case failed")
        cases[row["name"]].append(row["repetition"])
        if row["name"].startswith("crash_"):
            require(row["exit_code"] == 86 and row["recovered"] >= row["acked"] > 0, "Confirmed result lost")
            if archives:
                folder = Path(row["archive_path"])
                segments = [json.loads(p.read_text()) for p in (folder / "state").glob("*.json")]
                BENCH.verify_archive(folder, segments=segments)
                receipts = [receipt for p in (folder / "segments").glob("*.commit") for line in p.read_text().splitlines() for receipt in json.loads(line)["receipts"]]
                require(all(json.loads(line) in receipts for line in (folder / "acknowledged.jsonl").read_text().splitlines()), "Durable ACK is absent from its recovered journal")
        else:
            require(row["remote_checksums_verified"], "Remote verification missing")
            for segment in row["restored"]["segments"]:
                key = "/fixture/" + Path(row["archive_path"]).name + "/" + segment["file"]
                require(segment["state"] == "published" and report["server"]["remote_checksums"][key] == segment["sha256"], "False publication confirmation")
            for step in ["first", "online", "restored"]:
                if step in row:
                    require(row[step]["stats"]["upload_buffer_peak_bytes"] <= 5 * 1024 * 1024, "Upload buffer exceeded one part")
                    require(row[step]["rss_peak_bytes"] <= 384 * 1024 * 1024, "Publication RSS exceeded its profile")
    require(all(sorted(repetitions) == [1, 2, 3] for repetitions in cases.values()), "Recovery case lacks three repetitions")
    required = {"crash_" + name for name in BENCH.CRASH_POINTS} | {"s3_" + name for name in [
        "down", "completion_error", "head_mismatch", "slow", "down_online", "slow_online",
        "crash_upload_after_part", "crash_upload_after_transfer", "crash_publish_confirm_before", "crash_publish_confirm_after"]}
    require(set(cases) == required and len(report["checks"]) == 60, "Failure matrix is incomplete")
    require(report["server"]["endpoint"].startswith("http://127.0.0.1:") and report["server"]["remaining_multipart"] == 0, "S3 fixture leaked multipart state")
    if not config["smoke"]:
        require(len(report["runs"]) == 39, "Performance matrix is incomplete")
        for size in [64, 128, 256]:
            rows = groups[(f"rotation_{size}", "segments", 3, size * 1024 * 1024)]
            require(all(row["files"] >= 2 for row in rows), "Rotation size was not exercised")
        smaller = statistics.median(r["rss_peak_bytes"] for r in report["runs"] if r["scenario"] == "archive_scale_160")
        larger = statistics.median(r["rss_peak_bytes"] for r in report["runs"] if r["scenario"] == "archive_scale_320")
        require(larger <= smaller + 32 * 1024 * 1024, "RSS grew with total archive size")
    return dict(passed=True, performance_runs=len(report["runs"]), recovery_cases=len(report["checks"]),
                full_profile=not config["smoke"], archives_reconciled=archives,
                rss_profile_bytes=384 * 1024 * 1024, source_and_binary_verified=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--archives", action="store_true")
    parser.add_argument("--allow-smoke", action="store_true")
    args = parser.parse_args()
    print(json.dumps(verify(json.loads(args.report.read_text()), args.archives, args.allow_smoke)))
