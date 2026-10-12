#!/usr/bin/env python3
"""Independently recompute C06 equivalence, bounds, matrix and CPU target."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parents[2]
FIELDS = ("url", "base_url", "canonical_url", "no_index", "no_follow", "discovery_urls", "internal_urls")

def verify(path):
    report = json.loads(path.read_text())
    manifest = json.loads((ROOT / "tests/crawler-bench/manifest.json").read_text())
    names = {fixture["id"] for fixture in manifest["fixtures"]}
    expected_names = names | {"anchors-300000"}
    runs = report["runs"]
    checks = {}
    checks["complete_matrix"] = len(runs) == len(expected_names) * 6 and {
        (run["scenario"], run["mode"], run["repetition"]) for run in runs
    } == {(name, mode, rep) for name in expected_names for mode in ("legacy", "shared") for rep in (1, 2, 3)}
    checks["sources_unchanged"] = all(hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == digest
        for name, digest in report["code"]["source_files"].items())
    checks["binary_unchanged"] = hashlib.sha256(Path(report["binary"]["path"]).read_bytes()).hexdigest() == report["binary"]["sha256"]
    checks["conserved_corpus"] = all(hashlib.sha256((ROOT / fixture["path"]).read_bytes()).hexdigest() == fixture["sha256"]
        for fixture in manifest["fixtures"])
    checks["results_unchanged"] = True
    for run in runs:
        raw_path = path.parent / run["result_file"]
        raw = json.loads(raw_path.read_text())
        checks["results_unchanged"] &= hashlib.sha256(raw_path.read_bytes()).hexdigest() == run["result_sha256"]
        checks["results_unchanged"] &= all(run[key] == value for key, value in raw.items())
    checks["equivalent_links_metadata"] = True
    checks["one_dom_per_page"] = all(run["dom_parses_per_page"] == 1 for run in runs if run["mode"] == "shared")
    checks["bounded_projections"] = all(len(run["extraction"][field]) <= 512 for run in runs
        for field in ("discovery_urls", "internal_urls"))
    for name in expected_names:
        for rep in (1, 2, 3):
            pair = {run["mode"]: run for run in runs if run["scenario"] == name and run["repetition"] == rep}
            checks["equivalent_links_metadata"] &= set(pair) == {"legacy", "shared"} and all(
                pair["legacy"]["extraction"][field] == pair["shared"]["extraction"][field] for field in FIELDS)
    dense = [run for run in runs if run["scenario"] == "anchors-300000" and run["mode"] == "shared"]
    checks["dense_anchors_cut_early"] = len(dense) == 3 and all(run["extraction"]["anchors_visited"] == 512
        and run["extraction"]["traversal_stopped"] and len(run["extraction"]["internal_urls"]) == 512 for run in dense)
    reductions = []
    for rep in (1, 2, 3):
        totals = {mode: sum(run["cpu_ms_per_page"] for run in runs if run["scenario"] in names
                  and run["mode"] == mode and run["repetition"] == rep) for mode in ("legacy", "shared")}
        reductions.append((1 - totals["shared"] / totals["legacy"]) * 100)
    checks["cpu_target_at_least_30_percent"] = min(reductions) >= 30
    checks["cpu_summary_matches"] = abs(report["corpus_cpu_reduction_pct"]["median"] - statistics.median(reductions)) < 1e-8
    checks["no_network"] = report["configuration"]["network"] == "none"
    checks["allocation_counts_present"] = all(run["allocated_bytes_per_page"] > 0 and run["allocation_calls_per_page"] > 0 for run in runs)
    return {"passed": all(checks.values()), "checks": checks, "runs": len(runs), "cpu_reduction_pct": reductions}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = verify(args.report.resolve())
    if args.output:
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    if not result["passed"]:
        raise SystemExit(1)

if __name__ == "__main__":
    main()
