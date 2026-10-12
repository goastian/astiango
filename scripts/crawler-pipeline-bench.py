#!/usr/bin/env python3
"""C07 paired local HTTP/parse/fsync benchmark; no external requests."""
import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import threading
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/examples/crawler_pipeline_bench"
MIB = 1024 * 1024
SCENARIOS = {
    "corpus_single": (1, 2, 16, 0, 0, 1),
    "corpus_dual": (2, 32, 32, 0, 0, 1),
    "slow_parser": (1, 32, 24, 80, 0, 1),
    "slow_storage": (1, 32, 24, 0, 80, 1),
    "mixed_slow": (2, 32, 24, 60, 80, 2),
    "byte_pressure": (1, 32, 12, 30, 30, 1),
    "near_cap": (2, 128, 128, 0, 0, 2),
}
SOURCES = ["Cargo.lock", "crates/core/Cargo.toml", "crates/core/examples/crawler_pipeline_bench.rs",
    "crates/core/src/config/mod.rs", "scripts/crawler-pipeline-bench.py",
    "tests/crawler-pipeline-bench/verify_report.py"]
SOURCES += [str(p.relative_to(ROOT)) for p in sorted((ROOT / "crates/core/src/crawler").glob("*.rs"))]
SOURCES += ["crates/core/src/live_index/crawler/crawlable_site.rs",
    "crates/core/src/webpage/html/mod.rs", "crates/core/src/webpage/html/links.rs", "crates/core/src/warc.rs"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def distribution(values):
    middle = statistics.median(values)
    return {"median": middle, "min": min(values), "max": max(values),
            "mad": statistics.median(abs(v - middle) for v in values)}


def profile(name):
    threads, origins, pages, parser, storage, workers = SCENARIOS[name]
    limit = 2 * MIB
    stage = lambda count, size: {"workers": count, "max_items": 4, "max_bytes": size}
    return {"scenario": name, "runtime_threads": threads,
        "origins": [f"origin-{n}.pipeline.test" for n in range(origins)],
        "cases": [{"host": f"origin-{n % origins}.pipeline.test",
                   "path": f"/{'near-cap' if name in ('near_cap', 'byte_pressure') else 'fixture-' + str(n % 8)}?n={n}"}
                  for n in range(pages)],
        "body": {**{kind: {"wire_limit_bytes": limit, "decoded_limit_bytes": limit}
                    for kind in ("html", "feed", "sitemap", "robots")},
            "max_inflight_bodies": 4, "max_reserved_bytes": 32 * MIB,
            "connect_timeout_ms": 30000, "header_timeout_ms": 30000,
            "idle_timeout_ms": 30000, "total_timeout_ms": 60000,
            "budget_wait_timeout_ms": 30000},
        "pipeline": {"max_inflight_documents": 4, "max_inflight_bytes": 8 * MIB,
            "network_setup": stage(2, MIB),
            "parse": stage(2, 8 * MIB) if name != "byte_pressure" else
                     {"workers": 2, "max_items": 8, "max_bytes": limit},
            "storage": stage(workers, 8 * MIB), "control": stage(1, 16 * MIB),
            "queue_wait_timeout_ms": 30000, "runtime_sample_interval_ms": 25,
            "batch_segment_bytes": 4 * MIB, "batch_segment_items": 512},
        "async_jobs": 16, "parse_delay_ms": parser, "storage_delay_ms": storage,
        "rss_limit_bytes": 384 * MIB}


class Server(ThreadingHTTPServer):
    daemon_threads = True
    def __init__(self, payloads):
        super().__init__(("127.0.0.1", 0), Handler)
        self.payloads = payloads
        self.lock = threading.Lock()
        self.reset()
    def reset(self):
        with self.lock:
            self.requests = Counter()
            self.wire = 0
    def stats(self):
        with self.lock:
            return {"requests": sum(self.requests.values()), "paths": dict(self.requests),
                    "body_bytes_written": self.wire}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *args):
        pass
    def do_GET(self):
        path = urlsplit(self.path).path
        host = self.headers.get("Host", "").split(":")[0]
        if host not in {f"origin-{n}.pipeline.test" for n in range(128)} or path not in self.server.payloads:
            self.send_error(400)
            return
        body = self.server.payloads[path]
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()
        with self.server.lock:
            self.server.requests[self.path] += 1
            self.server.wire += len(body)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "data/crawler-pipeline-bench")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--scenarios", nargs="+", choices=SCENARIOS, default=list(SCENARIOS))
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise RuntimeError("Benchmark output directory must be empty")
    build = ["cargo", "build", "--locked", "--offline", "--release", "--no-default-features",
             "--features", "crawler-bench", "-p", "astiango", "--example", "crawler_pipeline_bench"]
    if not args.skip_build:
        subprocess.run(build, cwd=ROOT, check=True,
            env={**os.environ, "CFLAGS": "-std=gnu17", "CARGO_PROFILE_RELEASE_STRIP": "symbols"})
    manifest = json.loads((ROOT / "tests/crawler-bench/manifest.json").read_text())
    payloads = {}
    for number, fixture in enumerate(manifest["fixtures"]):
        path = ROOT / fixture["path"]
        if sha(path) != fixture["sha256"]:
            raise RuntimeError(f"Changed fixture: {path}")
        payloads[f"/fixture-{number}"] = path.read_bytes()
    anchors = (b"<a href='/page'>anchor</a>" * 80000)
    prefix = b"<html><head><base href='/base/'></head><body>" + anchors
    payloads["/near-cap"] = prefix + b" " * (2 * MIB - 64 - len(prefix) - 14) + b"</body></html>"
    payload_path = output / "near-cap.html"
    payload_path.write_bytes(payloads["/near-cap"])
    fixtures = manifest["fixtures"] + [{"id": "near-cap", "path": str(payload_path),
        "bytes": len(payloads["/near-cap"]), "sha256": sha(payload_path), "anchors": 80000}]
    sources = {name: sha(ROOT / name) for name in SOURCES}
    binary = sha(BINARY)
    server = Server(payloads)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    runs = []
    try:
        for repetition in range(1, 4):
            for name in args.scenarios:
                for mode in (["legacy", "bounded"] if repetition % 2 else ["bounded", "legacy"]):
                    identity = f"{name}-{mode}-{repetition}"
                    config = {**profile(name), "mode": mode, "port": server.server_port,
                              "output_dir": str(output / identity)}
                    cfg = output / f"{identity}.config.json"
                    cfg.write_text(json.dumps(config, indent=2) + "\n")
                    server.reset()
                    process = subprocess.run([str(BINARY), "--config", str(cfg)], cwd=ROOT,
                        capture_output=True, text=True, timeout=180)
                    (output / f"{identity}.stderr.log").write_text(process.stderr)
                    process.check_returncode()
                    result = json.loads(process.stdout)
                    artifact = output / f"{identity}.result.json"
                    artifact.write_text(json.dumps(result, indent=2) + "\n")
                    expected = {f"http://{c['host']}:{server.server_port}{c['path']}":
                        hashlib.md5(payloads[urlsplit(c['path']).path].decode('utf-8', 'replace').encode()).hexdigest()
                        for c in config["cases"]}
                    runs.append({**result, "runtime_lag_p99_ms": result["pipeline"]["runtime_lag_p99_us"] / 1000, "repetition": repetition, "configuration": config,
                        "configuration_file": cfg.name, "configuration_sha256": sha(cfg),
                        "result_file": artifact.name, "result_sha256": sha(artifact),
                        "expected_payload_md5": expected, "server": server.stats()})
                    print(f"{identity}: {result['pages_per_second']:.2f} páginas/s; "
                          f"p99 {result['pipeline']['runtime_lag_p99_us']/1000:.2f} ms; "
                          f"RSS {result['rss_peak_bytes']/MIB:.1f} MiB", flush=True)
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
    if {name: sha(ROOT / name) for name in SOURCES} != sources or sha(BINARY) != binary:
        raise RuntimeError("Source or binary changed during measurements")
    summaries, comparisons = [], []
    keys = ["duration_ms", "pages_per_second", "cpu_ms_per_page", "rss_peak_bytes",
            "fds_peak", "controller_delay_ms"]
    for name in args.scenarios:
        for mode in ("legacy", "bounded"):
            group = [r for r in runs if r["scenario"] == name and r["mode"] == mode]
            metrics = {key: distribution([r[key] for r in group]) for key in keys}
            metrics.update({"runtime_lag_p99_ms": distribution([r["runtime_lag_p99_ms"] for r in group]),
                "admission_waits": distribution([r["pipeline"]["admission"]["admission_waits"] for r in group])})
            summaries.append({"scenario": name, "mode": mode, "origins": len(profile(name)["origins"]),
                "repetitions": 3, "metrics": metrics})
        gains, lag_reductions, equivalent = [], [], True
        for repetition in range(1, 4):
            pair = {r["mode"]: r for r in runs if r["scenario"] == name and r["repetition"] == repetition}
            gains.append(100 * (pair["bounded"]["pages_per_second"] / pair["legacy"]["pages_per_second"] - 1))
            lag_reductions.append(100 * (1 - pair["bounded"]["runtime_lag_p99_ms"] /
                max(pair["legacy"]["runtime_lag_p99_ms"], 0.001)))
            equivalent &= pair["bounded"]["archive_payload_md5"] == pair["legacy"]["archive_payload_md5"]
        comparisons.append({"scenario": name, "equivalent": equivalent,
            "throughput_gain_pct": distribution(gains), "lag_reduction_pct": distribution(lag_reductions)})
    report = {"schema_version": 1, "kind": "crawler_pipeline_benchmark",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "configuration": {"acceptance_profile": "c07", "repetitions": 3, "scenarios": args.scenarios,
            "network": "loopback-only; allowlisted .test origins; links are extracted but never fetched",
            "reference": "C06 parse and gzip/fsync inline on Tokio; same code, corpus, limits and delays",
            "runtime": "1 or 2 Tokio workers per scenario, identical in paired runs; production has its own runtime",
            "rss_profile_bytes": 384 * MIB, "lag_p99_target_ms": 100},
        "code": {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "dirty": True, "source_files": sources},
        "binary": {"path": str(BINARY), "sha256": binary, "allocator": "jemalloc"},
        "build": {"command": build, "skipped": args.skip_build},
        "hardware": {"platform": platform.platform(), "cpu_logical": os.cpu_count(),
            "cpu": next((s.split(":", 1)[1].strip() for s in Path('/proc/cpuinfo').read_text().splitlines() if s.startswith('model name')), None),
            "cgroup": Path('/proc/self/cgroup').read_text().strip()},
        "fixtures": fixtures, "runs": runs, "summaries": summaries, "comparisons": comparisons,
        "acceptance": {"passed": False}}
    path = output / "report.json"
    path.write_text(json.dumps(report, indent=2) + "\n")
    subprocess.run([sys.executable, "-B", str(ROOT / "tests/crawler-pipeline-bench/verify_report.py"), str(path),
                    "--output", str(output / "verification.json")], cwd=ROOT, check=True)
    report["acceptance"] = json.loads((output / "verification.json").read_text())
    path.write_text(json.dumps(report, indent=2) + "\n")
    lines = ["# C07 · Pipeline acotado", "", "Tres repeticiones por escenario y modo; mediana ± MAD.", "",
        "| Escenario | Páginas/s antes | Páginas/s C07 | Cambio | Lag p99 C07 (ms) | RSS C07 (MiB) |",
        "|---|---:|---:|---:|---:|---:|"]
    for comparison in comparisons:
        pair = {s['mode']: s['metrics'] for s in summaries if s['scenario'] == comparison['scenario']}
        before, after = pair['legacy'], pair['bounded']
        metric = lambda v: f"{v['median']:.2f} ± {v['mad']:.2f}"
        lines.append(f"| {comparison['scenario']} | {metric(before['pages_per_second'])} | {metric(after['pages_per_second'])} | "
            f"{metric(comparison['throughput_gain_pct'])}% | {metric(after['runtime_lag_p99_ms'])} | {after['rss_peak_bytes']['median']/MIB:.1f} |")
    lines += ["", "El banco limita Tokio a 1–2 hilos para reproducir contención. El cambio de throughput es específico de cada escenario; no es una promesa porcentual para producción.",
        "Las demoras de parser/disco son inyectadas; se usa HTTP loopback, el parser real y WARC/fsync reales. El coste de CPU incluye HTTP y escritura, pero las demoras no consumen CPU.",
        "Las cuotas cuentan entradas y tareas, no DOM ni RSS. El RSS máximo se mide con getrusage; el perfil no instala un cgroup nuevo.",
        "El control se programa a los 100 ms desde el inicio. El lag p99 pertenece a un temporizador de 25 ms, no a todos los workers de Tokio.",
        "Se contrastan peticiones y bytes con el servidor, y todos los cuerpos WARC con los MD5 esperados."]
    (output / "report.md").write_text("\n".join(lines) + "\n")
    print(f"Report: {path}")


if __name__ == "__main__":
    main()
