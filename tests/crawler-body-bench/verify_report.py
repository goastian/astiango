#!/usr/bin/env python3
"""Independently verify C05 configuration, raw server/client/resources and provenance."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
BANK = __import__("crawler-body-bench")


def verify_provenance(report, check):
    check(report["source_unchanged"], "sources changed during measurement")
    check(report["code"]["source_sha256"] == BANK.code_version()["source_sha256"], "current sources differ from measured sources")
    binary = (ROOT / report["binary"]["path"]).resolve()
    check(binary.is_relative_to(ROOT), "binary escapes repository")
    content = binary.read_bytes()
    check(hashlib.sha256(content).hexdigest() == report["binary"]["sha256"] and len(content) == report["binary"]["bytes"],
          "benchmark binary differs from measured binary")
    check(report["build"]["command"] == BANK.shlex.join(BANK.BUILD), "build command differs")
    check(bool(report["configuration"]["command"]), "reproduction command absent")


def verify_matrix(report, check):
    configuration = report["configuration"]
    check(configuration["profile"] == BANK.PROFILE, "declared memory/timeout profile differs")
    check(configuration["scenarios"] == BANK.SCENARIOS, "scenario matrix differs")
    check(configuration["repetitions"] >= 3, "three repetitions required")
    expected = {(scenario, rep) for scenario in BANK.SCENARIOS for rep in range(1, configuration["repetitions"] + 1)}
    actual = [(run["scenario"], run["repetition"]) for run in report["runs"]]
    check(len(actual) == len(set(actual)), "duplicate run in matrix")
    check(set(actual) == expected, "incomplete or unexpected run matrix")
    artifacts = [run["artifact"] for run in report["runs"]]
    check(len(artifacts) == len(set(artifacts)), "artifact reused across repetitions")


def verify(path):
    report = json.loads(path.read_text())
    checks = 0

    def check(condition, message):
        nonlocal checks
        checks += 1
        if not condition:
            raise AssertionError(message)

    check(report["kind"] == "crawler_body_benchmark" and report["schema_version"] == 1, "wrong C05 schema")
    check(report["configuration"]["acceptance_profile"] == "c05", "wrong C05 profile")
    verify_provenance(report, check)
    verify_matrix(report, check)
    corpus, bodies = BANK.CAPACITY.load_corpus()
    check(report["corpus"] == corpus, "conserved HTML corpus differs")
    for published in report["runs"]:
        directory = (path.parent / published["artifact"]).resolve()
        check(directory.is_relative_to(path.parent.resolve()), "artifact escapes report directory")
        config = json.loads((directory / "config.json").read_text())
        original = json.loads((directory / "result.json").read_text())
        server = json.loads((directory / "server.json").read_text())
        resources = json.loads((directory / "resources.json").read_text())
        scenario = published["scenario"]
        check(config["scenario"] == scenario, "scenario differs")
        check(config["cases"] == BANK.cases(scenario, bodies), "workload/cause/corpus changed")
        check(config["origins"] == sorted({row["host"] for row in config["cases"]}), "controlled origins changed")
        check(config["profile"] == BANK.profile_for(scenario), "per-run memory/timeout limit changed")
        check(config["hold_ms"] == (100 if scenario == "retained_near_cap" else 0), "retention time changed")
        check(config["consumer_copies"] == (2 if scenario == "retained_near_cap" else 0), "consumer copies changed")
        check(server == published["server"], "server ledger differs from raw artifact")
        check(resources == published["resources"], "process resources differ from raw artifact")
        for key, value in original.items():
            check(published[key] == value, f"client measurement changed: {key}")
        for row in server["by_request"].values():
            spec = BANK.fixture(row["path"].lstrip("/"), bodies, config["profile"])
            check(row["body_bytes_planned"] == spec["size"] and row["status"] == spec.get("status", 200),
                  "server planned payload/status differs from controlled fixture")
        checked = BANK.verify_run(dict(original), config, server, resources)
        check(checked["verification"]["passed"], f"run failed: {published['artifact']}: {checked['verification']}")
        check(checked["verification"] == published["verification"], "verification was edited")
    check(BANK.summarize(report["runs"]) == report["summaries"], "medians/dispersion changed")
    check(BANK.acceptance(report) == report["acceptance"] and report["acceptance"]["passed"], "acceptance incomplete or edited")
    return {"passed": True, "checks": checks, "runs": len(report["runs"]),
            "requests": sum(run["requests"] for run in report["runs"]),
            "body_bytes_received": sum(run["body_stats"]["wire_bytes_received"] for run in report["runs"]),
            "server_body_bytes_written": sum(run["server"]["body_bytes_written"] for run in report["runs"]),
            "rss_peak_bytes": max(run["resources"]["rss_peak_bytes"] for run in report["runs"]),
            "source_sha256": report["code"]["source_sha256"], "report_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


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
