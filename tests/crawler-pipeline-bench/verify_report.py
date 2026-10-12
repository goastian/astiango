#!/usr/bin/env python3
"""Independently verify C07 quotas, local server accounting and paired artifacts."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import statistics
import sys

ROOT = Path(__file__).resolve().parents[2]
SCENARIOS = {"corpus_single", "corpus_dual", "slow_parser", "slow_storage", "mixed_slow", "byte_pressure", "near_cap"}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(report, base, artifacts=True):
    runs = report["runs"]
    identities = Counter((r["scenario"], r["mode"], r["repetition"]) for r in runs)
    expected = {(s, m, n) for s in SCENARIOS for m in ("legacy", "bounded") for n in range(1, 4)}
    checks = {"complete_three_repetition_matrix": set(identities) == expected and all(n == 1 for n in identities.values()),
        "loopback_only": report["configuration"]["network"].startswith("loopback-only"),
        "paired_configuration_and_archives": True, "server_requests_and_bytes": True,
        "single_extraction": True, "bounded_stage_capacity": True, "released_resources": True,
        "rss_profile": True, "runtime_and_control_below_100ms": True,
        "storage_and_parser_backpressure": True, "byte_quota_effective": True,
        "artifacts_and_sources": True, "summary_distributions": True, "comparison_distributions": True, "complete_summaries": True}
    for run in runs:
        cfg, p = run["configuration"], run["pipeline"]
        paths = Counter(c["path"] for c in cfg["cases"])
        checks["server_requests_and_bytes"] &= (run["server"]["requests"] == run["pages"] == len(cfg["cases"])
            and run["server"]["paths"] == dict(paths)
            and run["server"]["body_bytes_written"] == run["body"]["wire_bytes_received"]
            and run["archive_payload_md5"] == run["expected_payload_md5"])
        checks["single_extraction"] &= run["extraction"]["documents"] == run["pages"]
        checks["server_requests_and_bytes"] &= run["transport"]["requests_started"] == run["pages"]
        checks["server_requests_and_bytes"] &= all(c["host"].endswith(".pipeline.test") and c["host"] in cfg["origins"] for c in cfg["cases"])
        checks["summary_distributions"] &= run["runtime_lag_p99_ms"] == p["runtime_lag_p99_us"] / 1000
        if run["mode"] == "bounded":
            for stage in ("admission", "parse", "network_setup", "storage", "control", "batch_queue"):
                stats = p[stage]
                checks["bounded_stage_capacity"] &= (stats["peak_items"] <= stats["max_items"]
                    and stats["peak_bytes"] <= stats["max_bytes"] and stats["peak_running"] <= stats["workers"])
                checks["released_resources"] &= all(stats[k] == 0 for k in ("items", "bytes", "running", "queued"))
                checks["released_resources"] &= stats["submitted"] == stats["completed"] and stats["failed"] == stats["queue_timeouts"] == stats["input_rejections"] == 0
            checks["released_resources"] &= run["body"]["active_bodies"] == run["body"]["reserved_bytes"] == 0
            checks["rss_profile"] &= run["rss_peak_bytes"] <= cfg["rss_limit_bytes"] == 384 * 1024 * 1024
            checks["runtime_and_control_below_100ms"] &= p["runtime_lag_samples"] > 0 and p["runtime_lag_p99_us"] < 100000 and run["controller_delay_ms"] < 100
            if run["scenario"] in ("slow_parser", "slow_storage", "mixed_slow"):
                checks["storage_and_parser_backpressure"] &= p["admission"]["admission_waits"] > 0 and p["admission"]["peak_items"] == cfg["pipeline"]["max_inflight_documents"]
            if run["scenario"] == "byte_pressure":
                checks["byte_quota_effective"] &= p["parse"]["admission_waits"] > 0 and p["parse"]["peak_items"] == 1 < p["parse"]["max_items"]
        if artifacts:
            result_path, config_path = base / run["result_file"], base / run["configuration_file"]
            checks["artifacts_and_sources"] &= sha(result_path) == run["result_sha256"] and sha(config_path) == run["configuration_sha256"]
            raw = json.loads(result_path.read_text())
            checks["artifacts_and_sources"] &= all(run[k] == v for k, v in raw.items()) and json.loads(config_path.read_text()) == cfg
    for s in SCENARIOS:
        for n in range(1, 4):
            pair = {r["mode"]: r for r in runs if r["scenario"] == s and r["repetition"] == n}
            if len(pair) != 2:
                checks["paired_configuration_and_archives"] = False
                continue
            a, b = pair["legacy"], pair["bounded"]
            strip = lambda r: {k: v for k, v in r["configuration"].items() if k not in ("mode", "output_dir")}
            checks["paired_configuration_and_archives"] &= strip(a) == strip(b) and a["archive_payload_md5"] == b["archive_payload_md5"]
    if artifacts:
        checks["artifacts_and_sources"] &= all(sha(ROOT / name) == digest for name, digest in report["code"]["source_files"].items())
        checks["artifacts_and_sources"] &= sha(Path(report["binary"]["path"])) == report["binary"]["sha256"]
        checks["artifacts_and_sources"] &= all(sha(ROOT / f["path"]) == f["sha256"] for f in report["fixtures"])
    checks["complete_summaries"] = Counter((s["scenario"], s["mode"]) for s in report["summaries"]) == Counter({(s, m): 1 for s in SCENARIOS for m in ("legacy", "bounded")})
    for comparison in report["comparisons"]:
        gains, lags, equivalent = [], [], True
        for n in range(1, 4):
            pair = {r["mode"]: r for r in runs if r["scenario"] == comparison["scenario"] and r["repetition"] == n}
            if len(pair) != 2:
                checks["comparison_distributions"] = False
                continue
            a, b = pair["legacy"], pair["bounded"]
            gains.append(100 * (b["pages_per_second"] / a["pages_per_second"] - 1))
            lags.append(100 * (1 - b["runtime_lag_p99_ms"] / max(a["runtime_lag_p99_ms"], 0.001)))
            equivalent &= a["archive_payload_md5"] == b["archive_payload_md5"]
        if len(gains) == 3:
            for key, data in (("throughput_gain_pct", gains), ("lag_reduction_pct", lags)):
                middle = statistics.median(data)
                checks["comparison_distributions"] &= comparison[key] == {"median": middle, "min": min(data), "max": max(data), "mad": statistics.median(abs(v - middle) for v in data)}
            checks["comparison_distributions"] &= comparison["equivalent"] == equivalent
    for row in report["summaries"]:
        group = [r for r in runs if r["scenario"] == row["scenario"] and r["mode"] == row["mode"]]
        for key, values in row["metrics"].items():
            data = [r["pipeline"]["admission"][key] if key == "admission_waits" else r[key] for r in group]
            if not data:
                checks["summary_distributions"] = False
                continue
            middle = statistics.median(data)
            checks["summary_distributions"] &= values == {"median": middle, "min": min(data), "max": max(data), "mad": statistics.median(abs(v - middle) for v in data)}
    return {"passed": all(checks.values()), "checks": checks}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = verify(json.loads(args.report.read_text()), args.report.resolve().parent)
    content = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(content)
    print(content, end="")
    sys.exit(0 if result["passed"] else 1)


if __name__ == "__main__":
    main()
