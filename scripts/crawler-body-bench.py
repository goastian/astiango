#!/usr/bin/env python3
"""C05 loopback-only body admission, cancellation and memory acceptance bank."""
import argparse
from datetime import datetime, timezone
import gzip
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("capacity_bank", ROOT / "scripts/crawler-bench.py")
CAPACITY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CAPACITY)
BUILD = ["cargo", "build", "--locked", "--release", "--no-default-features", "--offline",
         "-p", "astiango", "--features", "crawler-bench", "--example", "crawler_body_bench"]
PROFILE = {"wire_limit_bytes": 2 * 1024 * 1024, "decoded_limit_bytes": 6 * 1024 * 1024,
           "max_reserved_bytes": 64 * 1024 * 1024, "max_inflight_bodies": 8, "workers": 8,
           "rss_limit_bytes": 256 * 1024 * 1024, "connect_timeout_ms": 500,
           "header_timeout_ms": 500, "idle_timeout_ms": 120, "total_timeout_ms": 800,
           "budget_wait_timeout_ms": 2000}
CHUNK_BYTES = 16 * 1024
SCENARIOS = ["corpus", "mime", "status", "declared_size", "gzip_bomb", "chunked_size",
             "decoded_size", "slow_headers", "idle_body", "total_body", "cancel_body", "retained_near_cap"]


def code_version():
    result = CAPACITY.code_version()
    digest = hashlib.sha256(result["source_sha256"].encode())
    for path in [Path(__file__).resolve(), *sorted((ROOT / "tests/crawler-body-bench").glob("*.py"))]:
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    result["source_sha256"] = digest.hexdigest()
    return result


def cgroup_memory():
    measurements = {}
    for name in ("memory.current", "memory.max", "memory.peak", "memory.events"):
        try:
            measurements[name] = (Path("/sys/fs/cgroup") / name).read_text().strip()
        except OSError:
            measurements[name] = None
    measurements["scope"] = "inherited cgroup, possibly shared; no new hard limit installed"
    return measurements


def fixture(scenario, bodies, profile):
    wire = profile["wire_limit_bytes"]
    if scenario.startswith("corpus/"):
        body = bodies[scenario.partition("/")[2]]
        return {"body": body, "size": len(body), "mime": "text/html; charset=utf-8"}
    if scenario == "feed":
        body = b'<?xml version="1.0"?><rss version="2.0"><channel><title>Controlled feed</title></channel></rss>'
        return {"body": body, "size": len(body), "mime": "application/rss+xml; charset=utf-8"}
    if scenario == "sitemap":
        body = b'<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"></urlset>'
        return {"body": body, "size": len(body), "mime": "application/xml; charset=utf-8"}
    if scenario == "robots":
        body = b"User-agent: *\nDisallow: /blocked\n"
        return {"body": body, "size": len(body), "mime": "text/plain; charset=utf-8"}
    result = {"size": wire * 4, "mime": "text/html; charset=utf-8", "delay_ms": 2}
    if scenario == "mime":
        result["mime"] = "image/png"
    elif scenario == "status":
        result["status"] = 503
    elif scenario == "gzip_bomb":
        body = gzip.compress(b"x" * (64 * 1024 * 1024), compresslevel=9, mtime=0)
        result.update({"body": body, "size": len(body), "encoding": "gzip", "inflated_bytes": 64 * 1024 * 1024})
    elif scenario == "chunked_size":
        result["chunked"] = True
    elif scenario in ("decoded_size", "retained_near_cap"):
        result.update({"size": wire - 64, "byte": b"\x80", "mime": "text/html; charset=windows-1252", "delay_ms": 2 if scenario == "decoded_size" else 0})
    elif scenario == "slow_headers":
        result.update({"size": 1024, "headers_delay_ms": 900})
    elif scenario in ("idle_body", "cancel_body"):
        result.update({"size": 256 * 1024, "first_delay_ms": 350})
    elif scenario == "total_body":
        result.update({"size": 1024 * 1024, "delay_ms": 60})
    return result


def serve_fixture(pipe, bodies, hosts, profile):
    lock = threading.Lock()
    ledger = {}
    fixtures = {}
    for name in SCENARIOS:
        if name != "corpus":
            fixtures[name] = fixture(name, bodies, profile)
    for name in bodies:
        fixtures[f"corpus/{name}"] = fixture(f"corpus/{name}", bodies, profile)
    for name in ("feed", "sitemap", "robots"):
        fixtures[name] = fixture(name, bodies, profile)

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def setup(self):
            super().setup()
            self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.connection.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 16 * 1024)
            self.connection.settimeout(5)

        def log_message(self, *_args):
            pass

        def do_GET(self):
            host = self.headers.get("Host", "").partition(":")[0]
            path = self.path.partition("?")[0]
            name = path.lstrip("/")
            if host not in hosts or name not in fixtures:
                self.send_error(421, "Destination outside controlled body fixtures")
                return
            spec = fixtures[name]
            key = f"{host}{path}"
            with lock:
                ledger.setdefault(key, {"requests": 0, "path": path, "host": host, "status": spec.get("status", 200),
                               "body_bytes_planned": spec["size"], "body_bytes_written": 0,
                               "completed": False, "disconnected": False})
                ledger[key]["requests"] += 1
            try:
                time.sleep(spec.get("headers_delay_ms", 0) / 1000)
                self.send_response(spec.get("status", 200))
                self.send_header("Content-Type", spec["mime"])
                if spec.get("chunked"):
                    self.send_header("Transfer-Encoding", "chunked")
                else:
                    self.send_header("Content-Length", str(spec["size"]))
                if spec.get("encoding"):
                    self.send_header("Content-Encoding", spec["encoding"])
                self.end_headers()
                self.wfile.flush()
                time.sleep(spec.get("first_delay_ms", 0) / 1000)
                for offset in range(0, spec["size"], CHUNK_BYTES):
                    chunk_size = min(CHUNK_BYTES, spec["size"] - offset)
                    chunk = spec["body"][offset:offset + chunk_size] if "body" in spec else spec.get("byte", b"x") * chunk_size
                    if spec.get("chunked"):
                        self.wfile.write(f"{len(chunk):x}\r\n".encode())
                    self.wfile.write(chunk)
                    if spec.get("chunked"):
                        self.wfile.write(b"\r\n")
                    self.wfile.flush()
                    with lock:
                        ledger[key]["body_bytes_written"] += len(chunk)
                    time.sleep(spec.get("delay_ms", 0) / 1000)
                if spec.get("chunked"):
                    self.wfile.write(b"0\r\n\r\n")
                    self.wfile.flush()
                with lock:
                    ledger[key]["completed"] = True
            except (BrokenPipeError, ConnectionResetError, TimeoutError):
                with lock:
                    ledger[key]["disconnected"] = True

    class Server(ThreadingHTTPServer):
        daemon_threads = True
        request_queue_size = 64

    server = Server(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.02}, daemon=True)
    thread.start()
    pipe.send(server.server_port)
    try:
        while True:
            command = pipe.recv()
            if command == "counts":
                with lock:
                    rows = {key: dict(value) for key, value in ledger.items()}
                pipe.send({"requests": sum(row["requests"] for row in rows.values()),
                           "body_bytes_planned": sum(row["body_bytes_planned"] for row in rows.values()),
                           "body_bytes_written": sum(row["body_bytes_written"] for row in rows.values()),
                           "by_request": rows, "chunk_bytes": CHUNK_BYTES,
                           "socket_send_buffer_requested_bytes": 16 * 1024,
                           "byte_semantics": "body payload accepted by server write; excludes headers/framing; not an acknowledgement of client consumption"})
            elif command == "stop":
                break
    except EOFError:
        pass
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
        pipe.close()


class FixtureServer:
    def __init__(self, bodies, hosts, profile):
        context = multiprocessing.get_context("spawn")
        self.connection, child = context.Pipe()
        self.process = context.Process(target=serve_fixture, args=(child, bodies, hosts, profile), daemon=True)
        self.process.start()
        child.close()
        if not self.connection.poll(15):
            self.close()
            raise RuntimeError("Body fixture failed to start")
        self.port = self.connection.recv()

    def counts(self):
        self.connection.send("counts")
        if not self.connection.poll(5):
            raise RuntimeError("Body fixture did not return its independent ledger")
        return self.connection.recv()

    def close(self):
        if self.process.is_alive():
            self.connection.send("stop")
            self.process.join(5)
        if self.process.is_alive():
            self.process.terminate()
            self.process.join(5)
        self.connection.close()


EXPECTED = {"corpus": "admitted", "mime": "unsupported_mime", "status": "status_rejected",
            "declared_size": "wire_limit_exceeded", "gzip_bomb": "unsupported_encoding",
            "chunked_size": "wire_limit_exceeded", "decoded_size": "decoded_limit_exceeded",
            "slow_headers": "headers_timeout", "idle_body": "idle_timeout",
            "total_body": "total_timeout", "cancel_body": "cancelled", "retained_near_cap": "admitted"}


def profile_for(scenario):
    profile = dict(PROFILE)
    if scenario == "decoded_size":
        profile["decoded_limit_bytes"] = 1024 * 1024
    if scenario == "retained_near_cap":
        profile["total_timeout_ms"] = 2000
    return profile


def cases(scenario, bodies):
    host = "origin-000.body.test"
    if scenario == "corpus":
        rows = [{"path": f"/corpus/{name}", "kind": "html", "expected": "admitted",
                 "decoded_bytes": len(body.decode("utf-8", errors="replace").encode()),
                 "decoded_md5": hashlib.md5(body.decode("utf-8", errors="replace").encode()).hexdigest()}
                for name, body in bodies.items()]
        rows += [{"path": f"/{name}", "kind": name, "expected": "admitted",
                  "decoded_bytes": fixture(name, bodies, PROFILE)["size"],
                  "decoded_md5": hashlib.md5(fixture(name, bodies, PROFILE)["body"]).hexdigest()}
                 for name in ("feed", "sitemap", "robots")]
        return [dict(row, host=host) for row in rows]
    if scenario == "retained_near_cap":
        decoded = "€".encode() * (PROFILE["wire_limit_bytes"] - 64)
        return [{"host": f"origin-{index:03}.body.test", "path": "/retained_near_cap", "kind": "html",
                 "expected": "admitted", "decoded_bytes": len(decoded), "decoded_md5": hashlib.md5(decoded).hexdigest()}
                for index in range(PROFILE["workers"])]
    return [{"host": host, "path": f"/{scenario}", "kind": "html", "expected": EXPECTED[scenario],
             "decoded_bytes": None, "decoded_md5": None}]


def verify_run(run, config, server, resources):
    scenario, profile = config["scenario"], config["profile"]
    rows = {row["host"] + row["path"]: row for row in run["outcomes"]}
    expected = {row["host"] + row["path"]: row for row in config["cases"]}
    stats = run["body_stats"]
    requests = len(expected)
    checks = {
        "profile_unchanged": run["configuration"] == profile == profile_for(scenario),
        "request_ledger": (set(rows) == set(expected) == set(server["by_request"]) and
                           len(run["outcomes"]) == requests == run["requests"] == server["requests"] and
                           all(row["requests"] == 1 for row in server["by_request"].values())),
        "explicit_causes": all(rows.get(key, {}).get("cause") == spec["expected"] for key, spec in expected.items()),
        "decoded_identity": all(rows.get(key, {}).get("decoded_bytes") == spec["decoded_bytes"] and
                                rows[key]["decoded_md5"] == spec["decoded_md5"]
                                for key, spec in expected.items() if spec["expected"] == "admitted"),
        "reservations_bounded": 0 <= stats["peak_reserved_bytes"] <= profile["max_reserved_bytes"],
        "concurrency_bounded": 0 <= stats["peak_active_bodies"] <= profile["max_inflight_bodies"],
        "reservations_released": stats["reserved_bytes"] == stats["active_bodies"] == 0,
        "rss_profile": 0 < resources["rss_peak_bytes"] <= profile["rss_limit_bytes"],
        "fd_released": run["fd_final"] <= run["fd_baseline"],
        "fd_bounded": run["fd_peak"] <= run["fd_baseline"] + 16,
        "transport_slots_released": run["transport_stats"]["client_slots_active"] == 0,
        "transport_request_ledger": run["transport_stats"]["requests_started"] == requests,
        "server_ledger_totals": (server["body_bytes_written"] == sum(row["body_bytes_written"] for row in server["by_request"].values()) and
                                 server["body_bytes_planned"] == sum(row["body_bytes_planned"] for row in server["by_request"].values())),
        "server_write_bounds": all(0 <= row["body_bytes_written"] <= row["body_bytes_planned"] for row in server["by_request"].values()),
        "body_started_ledger": stats["bodies_started"] == (0 if scenario == "slow_headers" else requests),
    }
    if scenario in ("corpus", "retained_near_cap"):
        checks["complete_bytes"] = (stats["wire_bytes_received"] == server["body_bytes_written"] == server["body_bytes_planned"] and
                                    stats["decoded_bytes_produced"] == sum(row["decoded_bytes"] for row in rows.values()))
        checks["completed_bodies"] = stats["bodies_completed"] == stats["bodies_admitted"] == requests and stats["body_failures"] == 0
        checks["server_completed"] = all(row["completed"] and not row["disconnected"] for row in server["by_request"].values())
    else:
        checks["early_server_cut"] = all(row["disconnected"] and row["body_bytes_written"] < row["body_bytes_planned"] for row in server["by_request"].values())
        checks["no_completed_body"] = stats["bodies_completed"] == 0
        checks["body_failure_ledger"] = stats["body_failures"] == (0 if scenario in ("slow_headers", "cancel_body") else 1)
    if scenario in ("mime", "status", "declared_size", "gzip_bomb", "slow_headers"):
        checks["no_body_polled"] = stats["wire_bytes_received"] == stats["decoded_bytes_produced"] == stats["bodies_admitted"] == 0
    if scenario == "declared_size":
        checks["declared_rejection_counter"] = stats["rejected_content_length"] == stats["wire_limit_exceeded"] == 1
    if scenario == "chunked_size":
        # Hyper may deliver the crossing chunk as a single block. It is counted,
        # then rejected before decoding; the controlled server quantum is 16 KiB.
        checks["wire_crossing_bounded"] = profile["wire_limit_bytes"] < stats["wire_bytes_received"] <= profile["wire_limit_bytes"] + 256 * 1024
        checks["server_overshoot_bounded"] = server["body_bytes_written"] <= profile["wire_limit_bytes"] + 512 * 1024
        checks["decoded_below_wire_limit"] = stats["decoded_bytes_produced"] <= profile["wire_limit_bytes"]
    if scenario == "decoded_size":
        checks["decoded_cut_bounded"] = 0 < stats["decoded_bytes_produced"] <= profile["decoded_limit_bytes"]
        checks["wire_below_wire_limit"] = stats["wire_bytes_received"] <= profile["wire_limit_bytes"]
    if scenario in ("slow_headers", "idle_body", "total_body", "cancel_body"):
        deadline = {"slow_headers": profile["header_timeout_ms"], "idle_body": profile["idle_timeout_ms"],
                    "total_body": profile["total_timeout_ms"], "cancel_body": 50}[scenario]
        checks["timeout_window"] = all(deadline * 0.9 <= row["duration_ms"] <= deadline + 500 for row in rows.values())
    if scenario == "retained_near_cap":
        reservation = profile["wire_limit_bytes"] + 3 * profile["decoded_limit_bytes"]
        capacity = min(profile["max_reserved_bytes"] // reservation, profile["max_inflight_bodies"])
        checks["retained_lease_budget"] = (run["held_bodies_peak"] == capacity and
                                           stats["peak_reserved_bytes"] == capacity * reservation and
                                           stats["peak_active_bodies"] == capacity and
                                           run["retained_stats"]["reserved_bytes"] == capacity * reservation)
        checks["backpressure_exercised"] = stats["budget_waits"] > 0 and stats["budget_wait_ms"] >= config["hold_ms"]
        checks["consumer_copies_exercised"] = run["consumer_copies"] == config["consumer_copies"] == 2 and run["hold_ms"] == config["hold_ms"] == 100
    checks["received_not_above_server_write"] = stats["wire_bytes_received"] <= server["body_bytes_written"]
    run.update({"server": server, "resources": resources,
                "verification": {"passed": all(checks.values()), "checks": checks}})
    return run


def summarize(runs):
    rows = []
    for scenario in SCENARIOS:
        group = [run for run in runs if run["scenario"] == scenario]
        if not group:
            continue
        values = {"duration_ms": [run["duration_ms"] for run in group],
                  "server_body_bytes_written": [run["server"]["body_bytes_written"] for run in group],
                  "body_bytes_received": [run["body_stats"]["wire_bytes_received"] for run in group],
                  "body_bytes_decoded": [run["body_stats"]["decoded_bytes_produced"] for run in group],
                  "peak_reserved_bytes": [run["body_stats"]["peak_reserved_bytes"] for run in group],
                  "rss_peak_bytes": [run["resources"]["rss_peak_bytes"] for run in group],
                  "cpu_seconds": [run["resources"]["cpu_seconds"] for run in group],
                  "fd_peak": [run["fd_peak"] for run in group]}
        rows.append({"scenario": scenario, "expected_outcome": EXPECTED[scenario], "repetitions": len(group),
                     "passed": all(run["verification"]["passed"] for run in group),
                     "metrics": {key: CAPACITY.distribution(data) for key, data in values.items()}})
    return rows


def acceptance(report):
    expected = {(scenario, repetition) for scenario in SCENARIOS for repetition in range(1, report["configuration"]["repetitions"] + 1)}
    observed = [(run["scenario"], run["repetition"]) for run in report["runs"]]
    checks = {"three_repetitions": report["configuration"]["repetitions"] >= 3,
              "complete_unique_matrix": len(observed) == len(set(observed)) and set(observed) == expected,
              "all_runs_verified": all(run["verification"]["passed"] for run in report["runs"]),
              "source_unchanged": report["source_unchanged"],
              "profile_declared": report["configuration"]["profile"] == PROFILE}
    return {"passed": all(checks.values()), "checks": checks}


def markdown(report):
    lines = ["# C05 · Admisión de cuerpos y memoria", "", f'Generado: {report["generated_at"]}', "",
             f'Fuentes: `{report["code"]["source_sha256"]}`. Binario: `{report["binary"]["sha256"]}`.', "",
             "Perfil previo: wire 2 MiB, UTF-8 6 MiB, reservas 64 MiB, 8 tareas, RSS máximo 256 MiB. "
             "Cada cuerpo reserva 20 MiB; se admiten tres a la vez. Decoded-size limita UTF-8 a 1 MiB. "
             "La cohorte retiene texto y dos copias durante 100 ms, con plazo total 2 s; los otros escenarios usan 800 ms.", "",
             "Mediana ± MAD y rango de tres repeticiones; bytes del servidor son escrituras aceptadas, no lectura del cliente.", "",
             "| Escenario | Causa esperada | Duración ms ± MAD (rango) | RSS MiB ± MAD (rango) | Reserva pico MiB | Pasa |",
             "|---|---|---:|---:|---:|---|"]
    for row in report["summaries"]:
        duration, rss = row["metrics"]["duration_ms"], row["metrics"]["rss_peak_bytes"]
        lines.append(f'| {row["scenario"]} | {row["expected_outcome"]} | '
                     f'{duration["median"]:.2f} ± {duration["mad"]:.2f} ({duration["min"]:.2f}–{duration["max"]:.2f}) | '
                     f'{rss["median"]/1048576:.2f} ± {rss["mad"]/1048576:.2f} ({rss["min"]/1048576:.2f}–{rss["max"]/1048576:.2f}) | '
                     f'{row["metrics"]["peak_reserved_bytes"]["median"]/1048576:g} | {row["passed"]} |')
    lines += ["", f'Aceptación: **{report["acceptance"]["passed"]}**.', "",
              "Transporte y BodyReader de producción, DNS inyectado y HTTP controlado exclusivamente en loopback. "
              "Sin parseo HTML ni persistencia WARC. Gzip se rechaza por Content-Encoding antes de leerlo: no se habilita descompresión. "
              "RSS pertenece al proceso cliente; cgroup heredado puede incluir otros procesos y se publica como contexto, no como límite instalado.", ""]
    return "\n".join(lines)


def settled_counts(server, timeout=2):
    deadline = time.monotonic() + timeout
    while True:
        result = server.counts()
        if result["by_request"] and all(row["completed"] or row["disconnected"] for row in result["by_request"].values()):
            return result
        if time.monotonic() >= deadline:
            return result
        time.sleep(0.02)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "data/crawler-body-bench")
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--timeout", type=int, default=30)
    args = parser.parse_args()
    if args.repetitions < 3 or args.timeout < 5:
        parser.error("Use at least three repetitions and a timeout >=5s")
    args.output = args.output.resolve()
    production = (ROOT / "data/crawler-local").resolve()
    if args.output.is_relative_to(production) or production.is_relative_to(args.output):
        parser.error("Body fixtures must remain outside the production dashboard data")
    if args.output.exists() and any(args.output.iterdir()):
        parser.error("Output must be empty; preserve previous evidence")
    args.output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment.setdefault("CFLAGS", "-std=gnu17")
    environment.setdefault("CARGO_PROFILE_RELEASE_STRIP", "symbols")
    if not args.skip_build:
        subprocess.run(BUILD, cwd=ROOT, env=environment, check=True)
    binary = ROOT / "target/release/examples/crawler_body_bench"
    binary_bytes = binary.read_bytes()
    corpus, bodies = CAPACITY.load_corpus()
    code = code_version()
    report = {"kind": "crawler_body_benchmark", "schema_version": 1, "generated_at": datetime.now(timezone.utc).isoformat(),
              "code": code, "hardware": CAPACITY.hardware(), "corpus": corpus,
              "binary": {"path": str(binary.relative_to(ROOT)), "sha256": hashlib.sha256(binary_bytes).hexdigest(), "bytes": len(binary_bytes)},
              "build": {"command": shlex.join(BUILD), "executed_by_runner": not args.skip_build,
                        "environment": {name: environment[name] for name in ("CFLAGS", "CARGO_PROFILE_RELEASE_STRIP")}},
              "configuration": {"acceptance_profile": "c05", "profile": PROFILE, "scenarios": SCENARIOS,
                                "repetitions": args.repetitions,
                                "command": shlex.join(["python3", "scripts/crawler-body-bench.py", *sys.argv[1:]])},
              "cgroup_before": cgroup_memory(), "runs": []}
    # Persist the memory profile before creating the first fixture or client.
    CAPACITY.atomic_json(args.output / "report.json", report)
    for scenario in SCENARIOS:
        for repetition in range(1, args.repetitions + 1):
            name = f"{scenario}-{repetition}"
            directory = args.output / name
            directory.mkdir()
            workload = cases(scenario, bodies)
            origins = sorted({row["host"] for row in workload})
            profile = profile_for(scenario)
            server = FixtureServer(bodies, origins, profile)
            try:
                config = {"scenario": scenario, "origins": origins, "port": server.port, "cases": workload,
                          "profile": profile, "output_dir": str(directory),
                          "hold_ms": 100 if scenario == "retained_near_cap" else 0,
                          "consumer_copies": 2 if scenario == "retained_near_cap" else 0}
                path = directory / "config.json"
                CAPACITY.atomic_json(path, config)
                resources = CAPACITY.run_child(binary, path, args.timeout)
                resources["cgroup_after"] = cgroup_memory()
                CAPACITY.atomic_json(directory / "resources.json", resources)
                run = json.loads((directory / "result.json").read_text())
                server_ledger = settled_counts(server)
                CAPACITY.atomic_json(directory / "server.json", server_ledger)
                run = verify_run(run, config, server_ledger, resources)
                run.update({"repetition": repetition, "artifact": name})
                report["runs"].append(run)
                CAPACITY.atomic_json(args.output / "report.json", report)
                print(f'{name}: {run["outcomes"][0]["cause"]}; received={run["body_stats"]["wire_bytes_received"]}; '
                      f'RSS={resources["rss_peak_bytes"]/1048576:.2f}MiB; verified={run["verification"]["passed"]}', flush=True)
                if not run["verification"]["passed"]:
                    raise RuntimeError(f'Body acceptance failed: {name}: {run["verification"]}')
            finally:
                server.close()
    report["summaries"] = summarize(report["runs"])
    report["source_unchanged"] = code_version()["source_sha256"] == code["source_sha256"]
    report["cgroup_after"] = cgroup_memory()
    report["acceptance"] = acceptance(report)
    CAPACITY.atomic_json(args.output / "report.json", report)
    (args.output / "report.md").write_text(markdown(report))
    print(f'Report: {args.output / "report.json"}', flush=True)
    return 0 if report["acceptance"]["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
