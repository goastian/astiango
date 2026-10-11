#!/usr/bin/env python3
"""Run the C01 crawler matrix exclusively against controlled loopback origins."""

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import multiprocessing
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys
import threading
import time


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "tests/crawler-bench/manifest.json"
DEFAULT_OUTPUT = ROOT / "data/crawler-bench"
BUILD = ["cargo", "build", "--locked", "--release", "--no-default-features",
         "-p", "astiango", "--features", "crawler-bench", "--example", "crawler_bench"]
SERVER_CONFIG = {"header_delay_ms": 80, "body_chunk_delay_ms": 20,
                 "body_chunks": 4, "large_bytes": 2 * 1024 * 1024,
                 "oversized_bytes": 33 * 1024 * 1024, "robots_disallow": "/blocked",
                 "listen_backlog": 256}


def sha256(content):
    return hashlib.sha256(content).hexdigest()


def load_corpus(path=CORPUS):
    raw = path.read_bytes()
    manifest = json.loads(raw)
    bodies = {}
    for fixture in manifest["fixtures"]:
        source = (ROOT / fixture["path"]).resolve()
        if not source.is_relative_to(ROOT):
            raise ValueError("Corpus path escapes repository")
        body = source.read_bytes()
        if len(body) != fixture["bytes"] or sha256(body) != fixture["sha256"]:
            raise ValueError(f"Corpus changed: {fixture['path']}")
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", fixture["id"]) or fixture["id"] in bodies:
            raise ValueError("Invalid or duplicate fixture ID")
        bodies[fixture["id"]] = body
    if not bodies:
        raise ValueError("Empty corpus")
    return {"sha256": sha256(raw), "files": manifest["fixtures"]}, bodies


def serve_fixture(connection, bodies, hosts):
    counters, by_origin, statuses = Counter(), Counter(), Counter()
    lock = threading.Lock()
    first = next(iter(bodies))

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):
            pass

        def do_GET(self):
            host = self.headers.get("Host", "").split(":")[0]
            path = self.path.split("?", 1)[0]
            status, headers, content_type = 200, {}, "text/html; charset=utf-8"
            chunk_delay = 0
            if host not in hosts:
                status, body = 421, b"Unregistered fixture origin"
            elif path == "/robots.txt":
                body = b"User-agent: *\nDisallow: /blocked\n"
                content_type = "text/plain"
            elif path == "/blocked":
                body = b"<html><body>Robots exclusion failed</body></html>"
            elif path.startswith(("/fixture/", "/slow/", "/slow-body/")):
                body = bodies.get(path.rsplit("/", 1)[-1])
                if body is None:
                    status, body = 404, b"Unknown fixture"
                if path.startswith("/slow/"):
                    time.sleep(SERVER_CONFIG["header_delay_ms"] / 1000)
                if path.startswith("/slow-body/"):
                    chunk_delay = SERVER_CONFIG["body_chunk_delay_ms"] / 1000
            elif path in ("/large", "/oversized"):
                length = SERVER_CONFIG["large_bytes" if path == "/large" else "oversized_bytes"]
                prefix, suffix = b"<!doctype html><html><body><p>", b"</p></body></html>"
                body = prefix + b"x" * (length - len(prefix) - len(suffix)) + suffix
            elif path.startswith("/error/"):
                status = int(path.rsplit("/", 1)[-1])
                body = f"<html><body>Controlled HTTP {status}</body></html>".encode()
                if status == 429:
                    headers["Retry-After"] = "0"
            elif path.startswith("/redirect/"):
                status, body = 302, b""
                headers["Location"] = {
                    "/redirect/relative": f"/fixture/{first}",
                    "/redirect/loop": "/redirect/loop",
                    "/redirect/external": "https://outside.invalid/escape",
                }.get(path, f"/fixture/{first}")
            else:
                status, body = 404, b"Unknown endpoint"
            with lock:
                counters[path] += 1
                by_origin[f"{host}{path}"] += 1
                statuses[str(status)] += 1
            try:
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                for key, value in headers.items():
                    self.send_header(key, value)
                self.end_headers()
                size = max(1, (len(body) + SERVER_CONFIG["body_chunks"] - 1) // SERVER_CONFIG["body_chunks"])
                for offset in range(0, len(body), size):
                    if chunk_delay:
                        time.sleep(chunk_delay)
                    self.wfile.write(body[offset:offset + size])
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass

    class Server(ThreadingHTTPServer):
        request_queue_size = SERVER_CONFIG["listen_backlog"]

    server = Server(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
    thread.start()
    connection.send(server.server_port)
    try:
        while True:
            command = connection.recv()
            if command == "counts":
                with lock:
                    connection.send({"requests": sum(counters.values()), "by_path": dict(counters),
                                     "by_origin_path": dict(by_origin), "statuses": dict(statuses)})
            elif command == "stop":
                break
    except EOFError:
        pass
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
        connection.close()


class FixtureServer:
    def __init__(self, bodies, hosts):
        context = multiprocessing.get_context("spawn")
        self.connection, child = context.Pipe()
        self.process = context.Process(target=serve_fixture, args=(child, bodies, hosts), daemon=True)
        self.process.start()
        child.close()
        if not self.connection.poll(15):
            self.close()
            raise RuntimeError("Fixture server failed to start")
        self.port = self.connection.recv()

    def counts(self):
        self.connection.send("counts")
        if not self.connection.poll(10):
            raise RuntimeError("Fixture server did not return counters")
        return self.connection.recv()

    def close(self):
        if self.process.is_alive():
            self.connection.send("stop")
            self.process.join(5)
        if self.process.is_alive():
            self.process.terminate()
            self.process.join(5)
        self.connection.close()


def proc_sample(pid):
    result = {}
    proc = Path(f"/proc/{pid}")
    try:
        for line in (proc / "status").read_text().splitlines():
            if line.startswith(("VmRSS:", "Threads:")):
                key, value = line.split(":", 1)
                result["rss_bytes" if key == "VmRSS" else "threads"] = int(value.split()[0]) * (1024 if key == "VmRSS" else 1)
        result["fds"] = len(list((proc / "fd").iterdir()))
        for line in (proc / "io").read_text().splitlines():
            key, value = line.split(":")
            result[key] = int(value)
    except (OSError, ValueError):
        pass
    return result


def run_child(binary, config_path, timeout):
    peaks = {}
    started = time.monotonic()
    with config_path.with_suffix(".log").open("wb") as log:
        child = subprocess.Popen([str(binary), "--config", str(config_path)], cwd=ROOT,
                                 stdout=log, stderr=subprocess.STDOUT)
        try:
            while True:
                for key, value in proc_sample(child.pid).items():
                    peaks[key] = max(peaks.get(key, 0), value)
                pid, status, usage = os.wait4(child.pid, os.WNOHANG)
                if pid:
                    child.returncode = os.waitstatus_to_exitcode(status)
                    break
                if time.monotonic() - started > timeout:
                    raise TimeoutError(f"Benchmark exceeded {timeout}s: {config_path}")
                time.sleep(0.01)
        finally:
            if child.returncode is None:
                child.kill()
                child.wait()
        if child.returncode:
            raise RuntimeError(f"Benchmark exited {child.returncode}; see {config_path.with_suffix('.log')}")
    return {"cpu_seconds": usage.ru_utime + usage.ru_stime,
            "cpu_user_seconds": usage.ru_utime, "cpu_system_seconds": usage.ru_stime,
            "rss_peak_bytes": usage.ru_maxrss * 1024,
            "process_wall_ms": (time.monotonic() - started) * 1000,
            "voluntary_context_switches": usage.ru_nvcsw,
            "involuntary_context_switches": usage.ru_nivcsw,
            "sampled_peak": peaks}


def distribution(values):
    median = statistics.median(values)
    return {"median": median, "min": min(values), "max": max(values),
            "mad": statistics.median(abs(value - median) for value in values)}


def scenario_paths(bodies, profile="c01"):
    ids = list(bodies)
    paths = {
        "balanced": [f"/fixture/{name}" for name in ids],
        "slow": [f"/fixture/{name}" for name in ids] +
                [f"/slow/{name}" for name in ids[:2]] + [f"/slow-body/{ids[0]}", "/large"],
        "adversarial": [f"/fixture/{name}" for name in ids[:2]] +
                       ["/large", "/oversized", "/error/404", "/error/500", "/error/429",
                        "/redirect/relative", "/redirect/loop", "/redirect/external", "/blocked"],
    }
    if profile == "c02":
        paths["adversarial"].remove("/error/429")
        paths["adversarial"].append("/error/429")
    return paths


def observer_equivalent(off, on):
    if not off or len(off) != len(on):
        return False
    baseline = {run["repetition"]: run for run in off}
    observed = {run["repetition"]: run for run in on}
    if len(baseline) != len(off) or len(observed) != len(on) or baseline.keys() != observed.keys():
        return False
    fields = ("requests_by_path", "requests_by_origin", "job_outcomes", "saved_pages", "saved_bytes", "crawl_policy")
    return all(
        all(field in original and field in observed[repetition]
            and original[field] == observed[repetition][field] for field in fields)
        and original["server"]["by_origin_path"] == observed[repetition]["server"]["by_origin_path"]
        for repetition, original in baseline.items()
    )


def explicit_outcomes_verified(run):
    jobs = run.get("job_outcomes")
    policy = run.get("crawl_policy", {})
    if not isinstance(jobs, dict) or set(jobs) != set(run["requests_by_origin"]):
        return False
    if policy.get("observer_controls_behavior") is not False or policy.get("network_policy") != "public_only":
        return False
    durable = attempts = 0
    for job in jobs.values():
        kinds = [entry["outcome"]["kind"] for entry in job["urls"]]
        if not set(kinds) <= {"durable_success", "skipped", "permanent_failure", "deferred"}:
            return False
        if job["status"] not in ("completed", "deferred"):
            return False
        if ("deferred" in kinds) != (job["status"] == "deferred"):
            return False
        if job["counters"]["useful_pages"] != kinds.count("durable_success"):
            return False
        durable += job["counters"]["useful_pages"]
        attempts += job["counters"]["page_http_attempts"]
    return (durable == run["saved_pages"] and
            attempts + run["requests_by_path"].get("/robots.txt", 0) == run["client_requests"])


def code_version():
    paths = {ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__).resolve(), CORPUS}
    for directory in ("crates/core/src", "crates/core/examples", "crates/robotstxt/src", "crates/kuchiki/src", "crates/simple-wal/src"):
        paths.update(path for path in (ROOT / directory).rglob("*") if path.is_file())
    paths.add(ROOT / "crates/core/Cargo.toml")
    paths.add(ROOT / "crates/simple-wal/Cargo.toml")
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)
    return {"commit": commit, "dirty": bool(status), "source_sha256": digest.hexdigest(),
            "working_tree_status": status.splitlines()}


def hardware():
    info = {"platform": platform.platform(), "machine": platform.machine(),
            "cpu_logical": os.cpu_count(), "cpu_affinity": sorted(os.sched_getaffinity(0)),
            "python": platform.python_version()}
    for path, key in (("/proc/meminfo", "memory"), ("/proc/cpuinfo", "cpu"),
                      ("/sys/fs/cgroup/cpu.max", "cgroup_cpu_max"),
                      ("/sys/fs/cgroup/memory.max", "cgroup_memory_max")):
        try:
            content = Path(path).read_text()
            if key == "cpu":
                content = next((line.split(":", 1)[1].strip() for line in content.splitlines()
                                if line.startswith("model name")), platform.processor())
            elif key == "memory":
                content = next(line for line in content.splitlines() if line.startswith("MemTotal:"))
            info[key] = content.strip()
        except OSError:
            info[key] = None
    for executable in ("rustc", "cargo"):
        info[executable] = subprocess.check_output([executable, "--version"], text=True).strip()
    return info


def summarize(name, origins, observer, runs):
    metrics = {}
    for key in ("duration_ms", "cpu_seconds", "rss_peak_bytes", "client_requests", "saved_pages",
                "saved_bytes", "process_wall_ms"):
        if all(isinstance(run.get(key), (int, float)) for run in runs):
            metrics["requests" if key == "client_requests" else key] = distribution([run[key] for run in runs])
    metrics["stages_ms"] = {stage: distribution([run["stages_ms"].get(stage, 0) for run in runs])
                            for stage in sorted({key for run in runs for key in run.get("stages_ms", {})})}
    for key in ("read_bytes", "write_bytes", "syscr", "syscw", "rchar", "wchar"):
        metrics[key] = distribution([run.get("resources_io", run["resources"]["sampled_peak"]).get(key, 0) for run in runs])
    for key in ("threads", "fds"):
        metrics[f"{key}_peak"] = distribution([run["resources"]["sampled_peak"].get(key, 0) for run in runs])
    return {"name": name, "origins": origins, "observer": observer, "repetitions": len(runs),
            "metrics": metrics, "requests_match": all(run["requests_match"] for run in runs), "runs": runs}


def atomic_json(path, value):
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def markdown_report(report):
    profile = report["configuration"].get("acceptance_profile", "c01")
    lines = [f"# {profile.upper()} · Resultados locales", "", f"Generado: {report['generated_at']}", "",
             f"Código: `{report['code']['commit']}`; árbol modificado: `{report['code']['dirty']}`.",
             f"SHA-256 de fuentes: `{report['code']['source_sha256']}`.",
             f"Corpus: `{report['corpus']['sha256']}`.", "",
             "Tiempos en ms; dispersión = MAD (desviación absoluta mediana). RSS corresponde al pico del proceso crawler.", "",
             "| Escenario | Orígenes | Observador | Mediana ms | MAD ms | CPU s | RSS MiB | Peticiones | Archivo verificado |",
             "|---|---:|---|---:|---:|---:|---:|---:|---|"]
    for row in report["scenarios"]:
        metric = row["metrics"]
        lines.append(f"| {row['name']} | {row['origins']} | {'sí' if row['observer'] else 'no'} | "
                     f"{metric['duration_ms']['median']:.2f} | {metric['duration_ms']['mad']:.2f} | "
                     f"{metric['cpu_seconds']['median']:.3f} | {metric['rss_peak_bytes']['median']/1048576:.1f} | "
                     f"{metric['requests']['median']:g} | {'sí' if all(r.get('warc_verified') for r in row['runs']) else 'no'} |")
    comparison_note = (
        "C02 compara solicitudes, decisiones, páginas y bytes por repetición con y sin observador. Las pruebas Rust verifican las pausas permitidas con reloj inyectado; el tiempo de pared real puede variar."
        if profile == "c02" else
        "La matriz C01 mide capacidad. Comparar tiempos requiere revisar la igualdad de solicitudes y resultados; el perfil C02 verifica esa equivalencia explícitamente."
    )
    lines += ["", "Las etapas concurrentes se acumulan por tarea y no suman el tiempo de pared. HTTP excluye TLS y DNS público.",
              comparison_note,
              "El JSON conserva cada repetición, contadores del servidor, configuración, recursos y rangos completos.", ""]
    if profile == "c02":
        lines += ["El perfil C02 sitúa HTTP 429 al final del escenario adversarial para ejercer redirects y robots antes del aplazamiento. El corpus HTML y los límites permanecen iguales.", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--acceptance-profile", choices=["c01", "c02"], default="c01")
    parser.add_argument("--origins", type=int, nargs="+")
    parser.add_argument("--scenarios", nargs="+", choices=["balanced", "slow", "adversarial"],
                        default=["balanced", "slow", "adversarial"])
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/examples/crawler_bench")
    args = parser.parse_args()
    if args.origins is None:
        args.origins = [2] if args.acceptance_profile == "c02" else [2, 32, 128]
    if args.repetitions < 3 or not 1 <= args.workers <= 128 or args.timeout <= 0 or any(count < 1 or count > 128 for count in args.origins):
        parser.error("Use at least three repetitions, 1–128 workers/origins and a positive timeout")
    if len(set(args.origins)) != len(args.origins) or len(set(args.scenarios)) != len(args.scenarios):
        parser.error("Origins and scenarios must be unique")
    output = args.output.resolve()
    production = (ROOT / "data/crawler-local").resolve()
    if output.is_relative_to(production) or production.is_relative_to(output):
        parser.error("Benchmark output must be isolated from data/crawler-local")
    if output.exists() and any(output.iterdir()):
        parser.error("Output must be empty; choose a new --output to preserve previous evidence")
    corpus, bodies = load_corpus()
    if not args.skip_build:
        subprocess.run(BUILD, cwd=ROOT, check=True, env={**os.environ,
                       "CFLAGS": os.environ.get("CFLAGS", "-std=gnu17"),
                       "CARGO_PROFILE_RELEASE_STRIP": "symbols"})
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"Missing benchmark binary: {binary}")
    output.mkdir(parents=True, exist_ok=True)
    version, paths = code_version(), scenario_paths(bodies, args.acceptance_profile)
    disk = os.statvfs(output)
    report = {"schema_version": 1, "kind": "crawler-capacity-benchmark",
              "generated_at": datetime.now(timezone.utc).isoformat(), "code": version,
              "corpus": corpus, "hardware": hardware(),
              "configuration": {"acceptance_profile": args.acceptance_profile,
                                "repetitions": args.repetitions, "origins": args.origins,
                                "workers": args.workers, "min_crawl_delay_ms": 5, "max_crawl_delay_ms": 50,
                                "timeout_seconds": 1, "server": SERVER_CONFIG,
                                "scenario_paths": {name: paths[name] for name in args.scenarios},
                                "command": [sys.executable, *sys.argv], "build": BUILD,
                                "binary_sha256": sha256(binary.read_bytes()),
                                "sampling_interval_ms": 10, "transport": "injected DNS; HTTP loopback only; no TLS",
                                "resource_scope": "CPU and IO during crawl; RSS/FD/thread peaks cover fresh crawler process including verification; server/build/dashboard excluded",
                                "storage": {"device_id": output.stat().st_dev, "block_size": disk.f_frsize,
                                            "total_bytes": disk.f_blocks * disk.f_frsize,
                                            "available_bytes_before": disk.f_bavail * disk.f_frsize}},
              "scenarios": [], "comparisons": [], "acceptance": {"passed": False}}
    for name in args.scenarios:
        for count in args.origins:
            groups = {False: [], True: []}
            for repetition in range(args.repetitions):
                for observer in ([False, True] if repetition % 2 == 0 else [True, False]):
                    label = f"{name}-{count}-{'on' if observer else 'off'}-{repetition + 1}"
                    directory = output / "runs" / label
                    directory.mkdir(parents=True)
                    hosts = [f"origin-{index:03}.test" for index in range(count)]
                    server = FixtureServer(bodies, hosts)
                    config = {"origins": [{"host": host, "port": server.port} for host in hosts],
                              "paths": paths[name], "observer": observer, "output_dir": str(directory),
                              "workers": args.workers, "min_crawl_delay_ms": 5,
                              "max_crawl_delay_ms": 50, "timeout_seconds": 1}
                    config_path = directory / "config.json"
                    atomic_json(config_path, config)
                    print(f"[{label}] local fixture :{server.port}", flush=True)
                    try:
                        resources = run_child(binary, config_path, args.timeout)
                        counts = server.counts()
                    finally:
                        server.close()
                    run = json.loads((directory / "result.json").read_text())
                    run.update(repetition=repetition + 1, resources=resources, server=counts)
                    run.update({key: resources[key] for key in ("cpu_seconds", "rss_peak_bytes", "process_wall_ms")})
                    if "crawl_cpu_seconds" in run:
                        run["cpu_seconds"] = run["crawl_cpu_seconds"]
                    elif isinstance(run.get("resources_crawl"), dict):
                        run["cpu_seconds"] = (run["resources_crawl"]["cpu_user_seconds"] +
                                              run["resources_crawl"]["cpu_system_seconds"])
                    observed_origins = Counter()
                    for origin_path, requests in counts["by_origin_path"].items():
                        observed_origins[origin_path.split("/", 1)[0]] += requests
                    run["requests_match"] = (run["client_requests"] == counts["requests"] and
                                             run["requests_by_path"] == counts["by_path"] and
                                             run["requests_by_origin"] == dict(observed_origins))
                    run["origins_verified"] = set(observed_origins) == set(hosts)
                    run["robots_respected"] = counts["by_path"].get("/blocked", 0) == 0
                    atomic_json(directory / "measurement.json", run)
                    groups[observer].append(run)
            rows = [summarize(name, count, observer, groups[observer]) for observer in (False, True)]
            report["scenarios"].extend(rows)
            off, on = (row["metrics"] for row in rows)
            report["comparisons"].append({"name": name, "origins": count,
                "duration_overhead_pct": 100 * (on["duration_ms"]["median"] / off["duration_ms"]["median"] - 1),
                "requests_equal": all(a["requests_by_path"] == b["requests_by_path"] for a, b in zip(groups[False], groups[True])),
                "saved_pages_equal": all(a["saved_pages"] == b["saved_pages"] for a, b in zip(groups[False], groups[True])),
                "behavior_equivalent": observer_equivalent(groups[False], groups[True])})
            atomic_json(output / "report.json", report)
    runs = [run for scenario in report["scenarios"] for run in scenario["runs"]]
    report["acceptance"] = {
        "full_matrix": set(args.origins) == {2, 32, 128} and set(args.scenarios) == set(paths),
        "three_repetitions": args.repetitions >= 3,
        "requests_reconciled": all(run["requests_match"] for run in runs),
        "origins_verified": all(run["origins_verified"] for run in runs),
        "robots_respected": all(run["robots_respected"] for run in runs),
        "warc_verified": all(run.get("warc_verified", False) for run in runs),
        "saved_content": all(run["saved_pages"] > 0 for run in runs),
        "source_unchanged": version["source_sha256"] == code_version()["source_sha256"],
        "loopback_only": True,
    }
    if args.acceptance_profile == "c02":
        report["acceptance"].update(
            c02_matrix=2 in args.origins and set(args.scenarios) == set(paths),
            observer_equivalent=all(row["behavior_equivalent"] for row in report["comparisons"]),
            explicit_outcomes=all(explicit_outcomes_verified(run) for run in runs),
        )
        required = [value for key, value in report["acceptance"].items() if key != "full_matrix"]
    else:
        required = list(report["acceptance"].values())
    report["acceptance"]["passed"] = all(required)
    atomic_json(output / "report.json", report)
    (output / "report.md").write_text(markdown_report(report))
    print(f"Report: {output / 'report.json'}; acceptance={report['acceptance']['passed']}")
    return 0 if report["acceptance"]["passed"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"C01: {error}", file=sys.stderr)
        sys.exit(1)
