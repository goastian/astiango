#!/usr/bin/env python3
"""C04: measure the production transport against an allowlisted loopback server."""

import argparse
from collections import Counter
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import socket
import shlex
import sys
import ssl
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("crawler_capacity_bank", ROOT / "scripts/crawler-bench.py")
CAPACITY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CAPACITY)
BUILD = ["cargo", "build", "--locked", "--release", "--no-default-features", "--offline",
         "-p", "astiango", "--features", "crawler-bench", "--example", "crawler_transport_bench"]


def fixture_response(path, bodies):
    first = next(iter(bodies))
    if path == "/robots.txt":
        return 200, b"User-agent: *\nDisallow: /blocked\n", "text/plain", {}
    if path == "/sitemap.xml":
        return 200, b'<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"></urlset>', "application/xml", {}
    if path == "/redirect":
        return 302, b"", None, {"Location": f"/fixture/{first}"}
    if path.startswith("/fixture/") and path.rsplit("/", 1)[-1] in bodies:
        return 200, bodies[path.rsplit("/", 1)[-1]], "text/html; charset=utf-8", {}
    if path == "/slow-headers":
        return 200, b"slow headers", "text/plain", {}
    if path == "/slow-body":
        return 200, b"slow body", "text/plain", {}
    return 404, b"Unknown endpoint", "text/plain", {}


def serve_fixture(connection, bodies, hosts, certificate=None, key=None):
    lock = threading.Lock()
    requests, origins, by_connection, host_headers, sni = (Counter() for _ in range(5))
    sockets, active, peak, bytes_sent = 0, 0, 0, 0
    connection_ids = {}

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def setup(self):
            super().setup()
            self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

        def finish(self):
            nonlocal active
            try:
                super().finish()
            finally:
                with lock:
                    active -= 1

        def log_message(self, *_args):
            pass

        def do_GET(self):
            nonlocal bytes_sent
            host_header = self.headers.get("Host", "")
            host = host_header.partition(":")[0]
            path = self.path.partition("?")[0]
            status, body, content_type, headers = fixture_response(path, bodies)
            if host not in hosts:
                status, body, content_type, headers = 421, b"Unregistered origin", "text/plain", {}
            with lock:
                requests[path] += 1
                origins[f"{host}{path}"] += 1
                by_connection[str(connection_ids[self.client_address])] += 1
                host_headers[host_header] += 1
            try:
                if path == "/slow-headers":
                    time.sleep(0.4)
                self.send_response(status)
                if content_type:
                    self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                for name, value in headers.items():
                    self.send_header(name, value)
                self.end_headers()
                if path == "/slow-body":
                    self.wfile.write(body[:1])
                    self.wfile.flush()
                    time.sleep(0.4)
                    self.wfile.write(body[1:])
                else:
                    self.wfile.write(body)
                self.wfile.flush()
                with lock:
                    bytes_sent += len(body)
            except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
                pass

    context = None
    if certificate:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certificate, key)

        def record_sni(_connection, server_name, _context):
            with lock:
                sni[server_name or "<missing>"] += 1
        context.sni_callback = record_sni

    class Server(ThreadingHTTPServer):
        request_queue_size = 256
        daemon_threads = True

        def get_request(self):
            nonlocal sockets, active, peak
            client, address = super().get_request()
            with lock:
                sockets += 1
                connection_ids[address] = sockets
                active += 1
                peak = max(peak, active)
            if context:
                try:
                    client.settimeout(5)
                    client = context.wrap_socket(client, server_side=True)
                except Exception:
                    client.close()
                    with lock:
                        active -= 1
                    raise
            return client, address

    server = Server(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.02}, daemon=True)
    thread.start()
    connection.send({"port": server.server_port})
    try:
        while True:
            command = connection.recv()
            if command == "counts":
                with lock:
                    connection.send({"requests": sum(requests.values()), "connections": sockets,
                                     "active_connections": active, "peak_connections": peak,
                                     "bytes_sent": bytes_sent, "requests_by_path": dict(requests),
                                     "requests_by_origin_path": dict(origins),
                                     "requests_by_connection": dict(by_connection),
                                     "host_headers": dict(host_headers), "sni": dict(sni)})
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
    def __init__(self, bodies, hosts, certificate=None, key=None):
        context = multiprocessing.get_context("spawn")
        self.connection, child = context.Pipe()
        self.process = context.Process(target=serve_fixture,
                                       args=(child, bodies, hosts, certificate, key), daemon=True)
        self.process.start()
        child.close()
        if not self.connection.poll(15):
            self.close()
            raise RuntimeError("Transport fixture failed to start")
        self.port = self.connection.recv()["port"]

    def counts(self):
        self.connection.send("counts")
        if not self.connection.poll(10):
            raise RuntimeError("Transport fixture did not return its ledger")
        return self.connection.recv()

    def close(self):
        if self.process.is_alive():
            self.connection.send("stop")
            self.process.join(5)
        if self.process.is_alive():
            self.process.terminate()
            self.process.join(5)
        self.connection.close()


def make_certificate(directory):
    directory.mkdir(parents=True, exist_ok=True)
    certificate, key = directory / "fixture-ca.pem", directory / "fixture-key.pem"
    subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-sha256", "-nodes",
                    "-days", "2", "-keyout", str(key), "-out", str(certificate),
                    "-subj", "/CN=transport.test", "-addext", "subjectAltName=DNS:*.transport.test",
                    "-addext", "basicConstraints=critical,CA:TRUE"], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    key.chmod(0o600)
    return certificate, key


def code_version():
    result = CAPACITY.code_version()
    digest = hashlib.sha256(result["source_sha256"].encode())
    for path in [Path(__file__).resolve(), *sorted((ROOT / "tests/crawler-transport-bench").glob("*.py"))]:
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    result["source_sha256"] = digest.hexdigest()
    return result


def workload(bodies):
    paths = ["/robots.txt", "/sitemap.xml", *[f"/fixture/{name}" for name in bodies], "/redirect"]
    result = []
    for path in paths:
        status, body, _content_type, headers = fixture_response(path, bodies)
        result.append({"path": path, "status": status, "bytes": len(body),
                       "md5": hashlib.md5(body).hexdigest(), "location": headers.get("Location"),
                       "page": path.startswith("/fixture/")})
    return result


def distribution(values):
    return CAPACITY.distribution(values)


def verify_run(run, config, server):
    origins = len(config["origins"])
    requests_per_origin = len(config["paths"]) + sum(item["location"] is not None for item in config["paths"])
    expected_requests = origins * requests_per_origin
    expected_pages = origins * (sum(item["page"] for item in config["paths"]) + 1)
    expected_paths = Counter(item["path"] for item in config["paths"])
    for item in config["paths"]:
        if item["location"]:
            expected_paths[item["location"]] += 1
    expected_by_origin = {f"{host}{path}": count for host in config["origins"] for path, count in expected_paths.items()}
    specs = {item["path"]: item for item in config["paths"]}
    expected_responses = {f"{host}{path}": {"count": count, "status": specs[path]["status"],
                                          "bytes": specs[path]["bytes"], "md5": specs[path]["md5"]}
                          for host in config["origins"] for path, count in expected_paths.items()}
    expected_bytes = sum(item["count"] * item["bytes"] for item in expected_responses.values())
    checks = {
        "request_ledger": run["requests"] == server["requests"] == expected_requests,
        "page_count": run["pages"] == expected_pages,
        "origin_path_ledger": server["requests_by_origin_path"] == expected_by_origin,
        "host_identity": set(server["host_headers"]) == {f'{host}:{config["port"]}' for host in config["origins"]},
        "body_integrity": run["body_mismatches"] == 0 and run["responses"] == expected_responses,
        "byte_ledger": run["bytes"] == server["bytes_sent"] == expected_bytes,
        "descriptor_peak_bounded": run["fd_peak"] <= run["fd_baseline"] + config["max_connections"] + 4,
        "descriptors_released": run["fd_final"] <= run["fd_baseline"] + 2,
        "server_connection_ledger": sum(server["requests_by_connection"].values()) == expected_requests,
        "bounded_connections": server["peak_connections"] <= config["workers"] + config["max_connections"] + 2,
    }
    if config["mode"] == "pooled":
        checks["keep_alive_reused"] = max(server["requests_by_connection"].values()) > 1
        checks["connections_reduced"] = server["connections"] < expected_requests
        checks["one_connection_per_origin"] = server["connections"] == origins
        checks["dns_cached_per_origin"] = run["dns_queries"] == origins
        checks["client_slots_bounded"] = run["transport_stats"]["client_slots_peak"] <= config["max_connections"]
        checks["bodies_release_slots"] = run["transport_stats"]["client_slots_active"] == 0
        checks["transport_ledger"] = (run["transport_stats"]["requests_started"] == expected_requests and
                                      run["transport_stats"]["bodies_completed"] == expected_requests and
                                      run["transport_stats"]["dns_lookups"] == run["dns_queries"])
    else:
        checks["legacy_one_connection_per_request"] = server["connections"] == expected_requests
        checks["legacy_dns_per_request"] = run["dns_queries"] == expected_requests
    if config["scheme"] == "https":
        checks["sni_identity"] = set(server["sni"]) == set(config["origins"])
        checks["every_tls_socket_has_sni"] = sum(server["sni"].values()) == server["connections"]
    run["verification"] = {"passed": all(checks.values()), "checks": checks}
    run["connections_per_page"] = server["connections"] / expected_pages
    run["server"] = server
    return run


def summarize(runs):
    summaries = []
    groups = sorted({(run["scheme"], run["origins"], run["mode"]) for run in runs})
    for scheme, origins, mode in groups:
        group = [run for run in runs if (run["scheme"], run["origins"], run["mode"]) == (scheme, origins, mode)]
        metrics = {key: distribution([run[key] for run in group]) for key in
                   ("duration_ms", "requests", "pages", "connections_per_page", "dns_queries", "dns_queries_per_page",
                    "fd_baseline", "fd_peak", "fd_final")}
        metrics["connections"] = distribution([run["server"]["connections"] for run in group])
        summaries.append({"scheme": scheme, "origins": origins, "mode": mode,
                          "repetitions": len(group), "metrics": metrics})
    return summaries


def comparisons(runs):
    result = []
    for scheme, origins in sorted({(run["scheme"], run["origins"]) for run in runs}):
        group = [run for run in runs if run["scheme"] == scheme and run["origins"] == origins]
        old = {run["repetition"]: run for run in group if run["mode"] == "legacy"}
        new = {run["repetition"]: run for run in group if run["mode"] == "pooled"}
        equivalent = set(old) == set(new) and all(
            old[rep]["responses"] == new[rep]["responses"] and
            old[rep]["server"]["requests_by_origin_path"] == new[rep]["server"]["requests_by_origin_path"] and
            old[rep]["pages"] == new[rep]["pages"] and old[rep]["bytes"] == new[rep]["bytes"] for rep in old)
        result.append({"scheme": scheme, "origins": origins, "equivalent": equivalent,
                       "connections_per_page_reduction_pct": distribution([
                           100 * (1 - new[rep]["connections_per_page"] / old[rep]["connections_per_page"]) for rep in old]),
                       "dns_queries_per_page_reduction_pct": distribution([
                           100 * (1 - new[rep]["dns_queries_per_page"] / old[rep]["dns_queries_per_page"]) for rep in old])})
    return result


def markdown(report):
    lines = ["# C04 · Banco local de transporte", "", f'Generado: {report["generated_at"]}', "",
             f'Código: `{report["code"]["commit"]}` · Fuentes: `{report["code"]["source_sha256"]}`.', "",
             "Cada fila publica mediana, MAD y rango de tres repeticiones. Las conexiones son sockets aceptados por el servidor.", "",
             "| Protocolo | Orígenes | Transporte | Conexiones/página (mediana ± MAD) | Rango | FD pico |",
             "|---|---:|---|---:|---|---:|"]
    for item in report["summaries"]:
        ratio = item["metrics"]["connections_per_page"]
        lines.append(f'| {item["scheme"]} | {item["origins"]} | {item["mode"]} | '
                     f'{ratio["median"]:.4f} ± {ratio["mad"]:.4f} | {ratio["min"]:.4f}–{ratio["max"]:.4f} | '
                     f'{item["metrics"]["fd_peak"]["median"]:g} |')
    lines += ["", f'Aceptación: **{report["acceptance"]["passed"]}**.', "",
              "Sólo servidores loopback controlados; HTML conservado C01 sin seguir enlaces externos. "
              "Este banco mide transporte y lectura íntegra del cuerpo, no parseo ni persistencia WARC.", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "data/crawler-transport-bench")
    parser.add_argument("--origins", nargs="+", type=int, default=[2, 32, 128])
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--tls", action="store_true", help="also run two-origin TLS/SNI validation")
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if args.repetitions < 3 or len(set(args.origins)) != len(args.origins) or any(count not in (2, 32, 128) for count in args.origins):
        parser.error("Use at least three repetitions and origins 2, 32 and/or 128")
    if args.output.exists() and any(args.output.iterdir()):
        parser.error("Output directory must be empty; preserve previous reports")
    args.output.mkdir(parents=True, exist_ok=True)
    if not args.skip_build:
        environment = os.environ.copy()
        environment.setdefault("CFLAGS", "-std=gnu17")
        environment.setdefault("CARGO_PROFILE_RELEASE_STRIP", "symbols")
        subprocess.run(BUILD, cwd=ROOT, env=environment, check=True)
    binary = ROOT / "target/release/examples/crawler_transport_bench"
    corpus, bodies = CAPACITY.load_corpus()
    code = code_version()
    binary_bytes = binary.read_bytes()
    report = {"schema_version": 1, "kind": "crawler_transport_benchmark", "generated_at": datetime.now(timezone.utc).isoformat(),
              "code": code, "hardware": CAPACITY.hardware(), "corpus": corpus,
              "binary": {"path": str(binary.relative_to(ROOT)), "sha256": hashlib.sha256(binary_bytes).hexdigest(),
                         "bytes": len(binary_bytes)},
              "build": {"command": shlex.join(BUILD), "executed_by_runner": not args.skip_build,
                        "environment": {"CFLAGS": "-std=gnu17", "CARGO_PROFILE_RELEASE_STRIP": "symbols"}},
              "configuration": {"acceptance_profile": "c04", "origins": args.origins, "repetitions": args.repetitions, "workers": 8,
                                "max_connections": 16, "max_connections_per_origin": 2, "dns_ttl_seconds": 300,
                                "min_delay_ms": 5, "timeout_seconds": 5, "tls": args.tls,
                                "command": shlex.join(["python3", "scripts/crawler-transport-bench.py", *sys.argv[1:]])},
              "runs": []}
    certificate = key = None
    if args.tls:
        certificate, key = make_certificate(args.output / "tls")
    groups = [("http", count) for count in args.origins]
    if args.tls:
        groups.append(("https", 2))
    for scheme, origins in groups:
        hosts = [f"origin-{index:03}.transport.test" for index in range(origins)]
        for repetition in range(1, args.repetitions + 1):
            # Alternate ordering so OS cache/warmup does not always favor the candidate.
            modes = ["legacy", "pooled"] if repetition % 2 else ["pooled", "legacy"]
            for mode in modes:
                name = f"{scheme}-{origins:03}-{mode}-{repetition}"
                directory = args.output / name
                directory.mkdir()
                server = FixtureServer(bodies, hosts, str(certificate) if scheme == "https" else None,
                                       str(key) if scheme == "https" else None)
                try:
                    config = {"origins": hosts, "port": server.port, "scheme": scheme, "mode": mode,
                              "paths": workload(bodies), "output_dir": str(directory), "workers": 8,
                              "max_connections": 16, "max_connections_per_origin": 2, "dns_ttl_seconds": 300,
                              "min_delay_ms": 5, "timeout_seconds": 5,
                              "certificate": str(certificate) if scheme == "https" else None}
                    config_path = directory / "config.json"
                    CAPACITY.atomic_json(config_path, config)
                    resources = CAPACITY.run_child(binary, config_path, args.timeout)
                    run = json.loads((directory / "result.json").read_text())
                    run.update({"scheme": scheme, "origins": origins, "mode": mode, "repetition": repetition,
                                "artifact": str(directory.relative_to(args.output)), "resources": resources})
                    server_ledger = server.counts()
                    CAPACITY.atomic_json(directory / "server.json", server_ledger)
                    run = verify_run(run, config, server_ledger)
                    report["runs"].append(run)
                    CAPACITY.atomic_json(args.output / "report.json", report)
                    print(f'{name}: {run["server"]["connections"]} sockets / {run["pages"]} pages; '
                          f'verified={run["verification"]["passed"]}', flush=True)
                    if not run["verification"]["passed"]:
                        raise RuntimeError(f"Failed transport verification: {name}: {run['verification']}")
                finally:
                    server.close()
    report["summaries"] = summarize(report["runs"])
    report["comparisons"] = comparisons(report["runs"])
    report["source_unchanged"] = code_version()["source_sha256"] == code["source_sha256"]
    checks = {"all_runs_verified": all(run["verification"]["passed"] for run in report["runs"]),
              "same_responses_and_limits": all(pair["equivalent"] for pair in report["comparisons"]),
              "connections_per_page_improved": all(pair["connections_per_page_reduction_pct"]["min"] > 0 for pair in report["comparisons"]),
              "dns_queries_per_page_improved": all(pair["dns_queries_per_page_reduction_pct"]["min"] > 0 for pair in report["comparisons"]),
              "three_repetitions": args.repetitions >= 3, "origin_matrix": set(args.origins) == {2, 32, 128},
              "source_unchanged": report["source_unchanged"]}
    report["acceptance"] = {"passed": all(checks.values()), "checks": checks}
    CAPACITY.atomic_json(args.output / "report.json", report)
    (args.output / "report.md").write_text(markdown(report))
    print(f'Report: {args.output / "report.json"}', flush=True)
    return 0 if report["acceptance"]["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
