#!/usr/bin/env python3
"""Recheck C04 reports against the saved request/socket ledgers and raw client runs."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
BANK = __import__("crawler-transport-bench")


def verify_provenance(report, check):
    check(report["source_unchanged"], "sources changed during the run")
    check(report["code"]["source_sha256"] == BANK.code_version()["source_sha256"], "current sources differ from the measured sources")
    binary = (ROOT / report["binary"]["path"]).resolve()
    check(binary.is_relative_to(ROOT), "binary escapes repository")
    payload = binary.read_bytes()
    check(hashlib.sha256(payload).hexdigest() == report["binary"]["sha256"] and len(payload) == report["binary"]["bytes"],
          "current benchmark binary differs from the measured binary")
    check(report["build"]["command"] == BANK.shlex.join(BANK.BUILD), "build command differs")
    check(bool(report["configuration"]["command"]), "missing reproduction command")


def verify_matrix(report, check):
    configuration = report["configuration"]
    check(configuration["repetitions"] >= 3, "at least three repetitions are required")
    check(sorted(configuration["origins"]) == [2, 32, 128], "missing 2/32/128 origin matrix")
    groups = [("http", count) for count in [2, 32, 128]]
    if configuration["tls"]:
        groups.append(("https", 2))
    expected = {(scheme, count, mode, repetition) for scheme, count in groups
                for mode in ["legacy", "pooled"] for repetition in range(1, configuration["repetitions"] + 1)}
    actual = [(run["scheme"], run["origins"], run["mode"], run["repetition"]) for run in report["runs"]]
    check(len(actual) == len(set(actual)), "duplicate runs in the measured matrix")
    check(set(actual) == expected, "incomplete or unexpected run matrix, including TLS/SNI repetitions")
    artifacts = [run["artifact"] for run in report["runs"]]
    check(len(artifacts) == len(set(artifacts)), "run artifacts are reused")


def verify(path):
    report = json.loads(path.read_text())
    checks = 0

    def check(condition, message):
        nonlocal checks
        checks += 1
        if not condition:
            raise AssertionError(message)

    check(report["kind"] == "crawler_transport_benchmark" and report["schema_version"] == 1, "wrong C04 report schema")
    check(report["configuration"]["acceptance_profile"] == "c04", "wrong acceptance profile")
    verify_provenance(report, check)
    verify_matrix(report, check)
    corpus, bodies = BANK.CAPACITY.load_corpus()
    check(report["corpus"] == corpus, "conserved HTML corpus differs")
    runs = []
    for published in report["runs"]:
        directory = (path.parent / published["artifact"]).resolve()
        check(directory.is_relative_to(path.parent.resolve()), "run escapes report directory")
        config = json.loads((directory / "config.json").read_text())
        original = json.loads((directory / "result.json").read_text())
        server = json.loads((directory / "server.json").read_text())
        check(config["paths"] == BANK.workload(bodies), "workload differs from the preserved HTML corpus")
        check(server == published["server"], "published server ledger differs from the original")
        for key, value in original.items():
            check(published[key] == value, f"published client measurement changed: {key}")
        checked = BANK.verify_run(dict(original), config, server)
        check(checked["verification"]["passed"], f"invalid run {published['artifact']}: {checked['verification']}")
        check(checked["verification"] == published["verification"], "published verification differs")
        check(config["mode"] == published["mode"] and config["scheme"] == published["scheme"], "transport mode differs")
        check(len(config["origins"]) == published["origins"], "origin count differs")
        for key in ("workers", "max_connections", "max_connections_per_origin", "dns_ttl_seconds", "min_delay_ms", "timeout_seconds"):
            check(config[key] == report["configuration"][key], f"changed workload limit: {key}")
        runs.append(published)
    comparisons = BANK.comparisons(runs)
    check(comparisons == report["comparisons"], "comparison medians/dispersions changed")
    check(BANK.summarize(runs) == report["summaries"], "summary medians/dispersions changed")
    for comparison in comparisons:
        check(comparison["equivalent"], "legacy and pooled returned different responses")
        check(comparison["connections_per_page_reduction_pct"]["min"] > 0, "no connection reuse improvement")
        check(comparison["dns_queries_per_page_reduction_pct"]["min"] > 0, "no DNS reuse improvement")
    check(report["acceptance"]["passed"] and all(report["acceptance"]["checks"].values()), "acceptance is incomplete")
    return {"passed": True, "checks": checks, "runs": len(runs),
            "requests": sum(run["requests"] for run in runs), "pages": sum(run["pages"] for run in runs),
            "server_connections": sum(run["server"]["connections"] for run in runs),
            "tls_runs": sum(run["scheme"] == "https" for run in runs),
            "source_sha256": report["code"]["source_sha256"],
            "report_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = verify(args.report)
    if args.output:
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
