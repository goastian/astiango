#!/usr/bin/env python3
"""C06: local extraction only, unchanged C01 HTML, three paired repetitions."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/examples/crawler_extraction_bench"
SOURCES = ["Cargo.lock", "crates/core/Cargo.toml", "crates/core/examples/crawler_extraction_bench.rs",
           "crates/core/src/crawler/extraction.rs", "crates/core/src/crawler/extraction_benchmark.rs",
           "crates/core/src/crawler/worker.rs", "crates/core/src/crawler/local.rs",
           "crates/core/src/crawler/mod.rs", "crates/core/src/webpage/html/mod.rs",
           "crates/core/src/webpage/html/links.rs", "scripts/crawler-extraction-bench.py",
           "tests/crawler-extraction-bench/verify_report.py"]

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def fingerprint():
    return {name: sha(ROOT / name) for name in SOURCES}

def distribution(values):
    middle = statistics.median(values)
    return {"median": middle, "mad": statistics.median(abs(v - middle) for v in values),
            "min": min(values), "max": max(values)}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "data/crawler-extraction-bench")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise RuntimeError("El directorio del banco debe estar vacío.")
    build = ["cargo", "build", "--locked", "--offline", "--release", "--no-default-features",
             "--features", "crawler-bench", "-p", "astiango", "--example", "crawler_extraction_bench"]
    environment = {**os.environ, "CFLAGS": "-std=gnu17", "CARGO_PROFILE_RELEASE_STRIP": "symbols"}
    if not args.skip_build:
        subprocess.run(build, cwd=ROOT, env=environment, check=True)
    manifest = json.loads((ROOT / "tests/crawler-bench/manifest.json").read_text())
    fixtures = []
    for fixture in manifest["fixtures"]:
        path = ROOT / fixture["path"]
        if sha(path) != fixture["sha256"] or path.stat().st_size != fixture["bytes"]:
            raise RuntimeError(f"Fixture modificado: {path}")
        fixtures.append({**fixture, "scenario": fixture["id"], "iterations": 5,
                         "url": fixture["source_url"] or "https://example.org/article", "file": str(path)})
    adversarial = output / "anchors-300000.html"
    with adversarial.open("w") as target:
        target.write("<html><head><base href='/base/'><meta name='robots' content='noindex'></head><body>")
        for number in range(300_000):
            target.write(f"<a href='page-{number}'>anchor {number}</a>")
        target.write("</body></html>")
    fixtures.append({"scenario": "anchors-300000", "iterations": 1, "url": "https://example.org/article",
                     "file": str(adversarial), "sha256": sha(adversarial), "bytes": adversarial.stat().st_size,
                     "category": "synthetic_adversarial", "anchors": 300_000})
    before = fingerprint()
    binary_hash = sha(BINARY)
    runs = []
    for repetition in range(1, 4):
        for fixture in fixtures:
            for mode in (["legacy", "shared"] if repetition % 2 else ["shared", "legacy"]):
                name = f"{fixture['scenario']}-{mode}-{repetition}"
                config_path = output / f"{name}.config.json"
                config_path.write_text(json.dumps({"mode": mode, "path": fixture["file"],
                    "url": fixture["url"], "iterations": fixture["iterations"]}, indent=2) + "\n")
                process = subprocess.run([str(BINARY), "--config", str(config_path)], cwd=ROOT,
                    check=True, capture_output=True, text=True, timeout=180)
                raw = json.loads(process.stdout)
                result_path = output / f"{name}.result.json"
                result_path.write_text(json.dumps(raw, indent=2) + "\n")
                runs.append({"scenario": fixture["scenario"], "repetition": repetition,
                    "result_file": result_path.name, "result_sha256": sha(result_path), **raw})
                print(f"{name}: CPU {raw['cpu_ms_per_page']:.3f} ms/página; asignados {raw['allocated_bytes_per_page']:.0f} B/página", flush=True)
    if fingerprint() != before or sha(BINARY) != binary_hash:
        raise RuntimeError("Fuentes o binario cambiaron durante el banco.")
    keys = ("cpu_ms_per_page", "wall_ms_per_page", "allocated_bytes_per_page", "allocation_calls_per_page", "dom_parses_per_page")
    summaries = []
    comparisons = []
    for fixture in fixtures:
        name = fixture["scenario"]
        for mode in ("legacy", "shared"):
            group = [run for run in runs if run["scenario"] == name and run["mode"] == mode]
            summaries.append({"scenario": name, "mode": mode, "repetitions": 3,
                "metrics": {key: distribution([run[key] for run in group]) for key in keys}})
        reductions = {key: [] for key in ("cpu", "allocated_bytes")}
        equivalent = True
        for repetition in range(1, 4):
            pair = {run["mode"]: run for run in runs if run["scenario"] == name and run["repetition"] == repetition}
            for short, key in (("cpu", "cpu_ms_per_page"), ("allocated_bytes", "allocated_bytes_per_page")):
                reductions[short].append(100 * (1 - pair["shared"][key] / pair["legacy"][key]))
            for key in ("url", "base_url", "canonical_url", "no_index", "no_follow", "discovery_urls", "internal_urls"):
                equivalent = equivalent and pair["legacy"]["extraction"][key] == pair["shared"]["extraction"][key]
        comparisons.append({"scenario": name, "equivalent": equivalent,
            "cpu_reduction_pct": distribution(reductions["cpu"]),
            "allocated_bytes_reduction_pct": distribution(reductions["allocated_bytes"])})
    corpus_names = {fixture["scenario"] for fixture in fixtures[:-1]}
    reductions = []
    for repetition in range(1, 4):
        totals = {mode: sum(run["cpu_ms_per_page"] for run in runs if run["repetition"] == repetition
            and run["mode"] == mode and run["scenario"] in corpus_names) for mode in ("legacy", "shared")}
        reductions.append(100 * (1 - totals["shared"] / totals["legacy"]))
    report = {"schema_version": 1, "kind": "crawler_extraction_benchmark",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "configuration": {"acceptance_profile": "c06", "repetitions": 3, "link_limit": 512,
            "allocation_metric": "requested bytes including realloc new sizes; same-thread extraction only; not RSS",
            "text_decoding": "UTF-8 replacement, identical for both modes",
            "network": "none", "warmup": "both mechanisms before measurement", "cpu_clock": "CLOCK_THREAD_CPUTIME_ID"},
        "code": {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                 "source_files": before, "dirty": True},
        "binary": {"path": str(BINARY), "sha256": binary_hash, "allocator": "jemalloc"},
        "build": {"command": build, "environment": {"CFLAGS": "-std=gnu17", "CARGO_PROFILE_RELEASE_STRIP": "symbols"},
                  "skipped": args.skip_build},
        "hardware": {"platform": platform.platform(), "cpu_logical": os.cpu_count(),
                     "cpu": next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), None)},
        "fixtures": fixtures, "runs": runs, "summaries": summaries, "comparisons": comparisons,
        "corpus_cpu_reduction_pct": distribution(reductions), "acceptance": {"passed": False}}
    path = output / "report.json"
    path.write_text(json.dumps(report, indent=2) + "\n")
    subprocess.run([sys.executable, "-B", str(ROOT / "tests/crawler-extraction-bench/verify_report.py"), str(path),
                    "--output", str(output / "verification.json")], cwd=ROOT, check=True)
    verification = json.loads((output / "verification.json").read_text())
    report["acceptance"] = {"passed": verification["passed"], "checks": verification["checks"]}
    path.write_text(json.dumps(report, indent=2) + "\n")
    lines = ["# C06 · Extracción compartida", "", "Tres repeticiones; mediana ± MAD. Sin solicitudes de red.", "",
             "CPU del corpus: **%.2f %% menos** (MAD %.2f)." % (statistics.median(reductions), report["corpus_cpu_reduction_pct"]["mad"]), "",
             "| Escenario | Modo | CPU ms/página | Bytes asignados/página | DOM/página |", "|---|---|---:|---:|---:|"]
    for row in summaries:
        metrics = row["metrics"]
        cpu, allocations = metrics["cpu_ms_per_page"], metrics["allocated_bytes_per_page"]
        lines.append(f"| {row['scenario']} | {row['mode']} | {cpu['median']:.3f} ± {cpu['mad']:.3f} | {allocations['median']:.0f} ± {allocations['mad']:.0f} | {metrics['dom_parses_per_page']['median']:.0f} |")
    lines += ["", "Asignaciones acumuladas solicitadas, no memoria viva ni RSS. La referencia reproduce el doble parseo completo de C05; no mide HTTP, compresión ni almacenamiento.", "",
              "El DOM sigue procesando todo el HTML. Los vectores y la deduplicación retienen como máximo 512 destinos por proyección. Si una proyección no se llena, el recorrido continúa para conservar la cobertura histórica.", "",
              f"Aceptación: **{verification['passed']}**. Fuentes, binario, hardware, corpus, rangos y resultados individuales se conservan en report.json."]
    (output / "report.md").write_text("\n".join(lines) + "\n")
    print(f"Informe: {path}")

if __name__ == "__main__":
    main()
