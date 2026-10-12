import hashlib
import http.client
import importlib.util
from pathlib import Path
import ssl
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
BANK = __import__("crawler-transport-bench")


class TransportLedgerTests(unittest.TestCase):
    def test_workload_preserves_corpus_and_relative_redirect_without_mime(self):
        _manifest, bodies = BANK.CAPACITY.load_corpus()
        specs = BANK.workload(bodies)
        self.assertEqual(len(specs), len(bodies) + 3)
        for spec in specs:
            if spec["page"]:
                body = bodies[spec["path"].rsplit("/", 1)[-1]]
                self.assertEqual(spec["bytes"], len(body))
                self.assertEqual(spec["md5"], hashlib.md5(body).hexdigest())
        status, body, mime, headers = BANK.fixture_response("/redirect", bodies)
        self.assertEqual((status, body, mime), (302, b"", None))
        self.assertTrue(headers["Location"].startswith("/fixture/"))

    def test_server_independently_counts_keep_alive_and_host(self):
        bodies = {"saved": b"<html>conserved</html>"}
        host = "origin-000.transport.test"
        server = BANK.FixtureServer(bodies, [host])
        try:
            client = http.client.HTTPConnection("127.0.0.1", server.port, timeout=5)
            for path in ["/robots.txt", "/fixture/saved", "/sitemap.xml"]:
                client.request("GET", path, headers={"Host": f"{host}:{server.port}"})
                response = client.getresponse()
                self.assertEqual(response.status, 200)
                response.read()
            client.close()
            ledger = server.counts()
            self.assertEqual(ledger["requests"], 3)
            self.assertEqual(ledger["connections"], 1)
            self.assertEqual(list(ledger["requests_by_connection"].values()), [3])
            self.assertEqual(ledger["host_headers"], {f"{host}:{server.port}": 3})
        finally:
            server.close()

    def test_ledger_rejects_unknown_origin(self):
        server = BANK.FixtureServer({"saved": b"<html>conserved</html>"}, ["origin-000.transport.test"])
        try:
            client = http.client.HTTPConnection("127.0.0.1", server.port, timeout=5)
            client.request("GET", "/robots.txt", headers={"Host": "outside.invalid"})
            response = client.getresponse()
            self.assertEqual(response.status, 421)
            response.read()
            client.close()
        finally:
            server.close()

    def test_generated_fixture_certificate_validates_original_sni_hostname(self):
        import socket
        host = "origin-000.transport.test"
        with tempfile.TemporaryDirectory() as directory:
            certificate, key = BANK.make_certificate(Path(directory))
            server = BANK.FixtureServer({"saved": b"<html>conserved</html>"}, [host], str(certificate), str(key))
            try:
                context = ssl.create_default_context(cafile=str(certificate))
                with socket.create_connection(("127.0.0.1", server.port), timeout=5) as raw:
                    with context.wrap_socket(raw, server_hostname=host) as secured:
                        secured.sendall(f"GET /robots.txt HTTP/1.1\r\nHost: {host}:{server.port}\r\nConnection: close\r\n\r\n".encode())
                        response = bytearray()
                        while chunk := secured.recv(4096):
                            response.extend(chunk)
                        self.assertTrue(response.startswith(b"HTTP/1.1 200"))
                self.assertEqual(server.counts()["sni"], {host: 1})
            finally:
                server.close()


class TransportVerificationTests(unittest.TestCase):
    def sample(self):
        bodies = {"saved": b"<html>conserved</html>"}
        hosts = ["origin-000.transport.test", "origin-001.transport.test"]
        config = {"origins": hosts, "port": 12345, "scheme": "http", "mode": "pooled",
                  "paths": BANK.workload(bodies), "workers": 8, "max_connections": 16}
        specs = {item["path"]: item for item in config["paths"]}
        counts = {"/robots.txt": 1, "/sitemap.xml": 1, "/fixture/saved": 2, "/redirect": 1}
        responses = {f"{host}{path}": {"count": count, "status": specs[path]["status"],
                                      "bytes": specs[path]["bytes"], "md5": specs[path]["md5"]}
                     for host in hosts for path, count in counts.items()}
        size = sum(item["count"] * item["bytes"] for item in responses.values())
        run = {"requests": 10, "pages": 4, "bytes": size, "responses": responses,
               "body_mismatches": 0, "fd_baseline": 5, "fd_peak": 7, "fd_final": 5, "dns_queries": 2,
               "transport_stats": {"client_slots_peak": 2, "client_slots_active": 0,
                                   "requests_started": 10, "bodies_completed": 10, "dns_lookups": 2}}
        server = {"requests": 10, "connections": 2, "peak_connections": 2, "bytes_sent": size,
                  "requests_by_origin_path": {key: item["count"] for key, item in responses.items()},
                  "requests_by_connection": {"1": 5, "2": 5},
                  "host_headers": {f"{host}:12345": 5 for host in hosts}}
        return run, config, server

    def test_acceptance_requires_real_server_and_client_agreement(self):
        run, config, server = self.sample()
        self.assertTrue(BANK.verify_run(run, config, server)["verification"]["passed"])
        server["requests"] -= 1
        checks = BANK.verify_run(run, config, server)["verification"]["checks"]
        self.assertFalse(checks["request_ledger"])

    def test_verification_detects_changed_body_even_if_error_counter_claims_success(self):
        run, config, server = self.sample()
        run["responses"][next(iter(run["responses"]))]["md5"] = "invented"
        checks = BANK.verify_run(run, config, server)["verification"]["checks"]
        self.assertFalse(checks["body_integrity"])

    def test_reuse_does_not_hide_active_slot_or_file_descriptor_leaks(self):
        run, config, server = self.sample()
        run["transport_stats"]["client_slots_active"] = 1
        run["fd_final"] = 50
        checks = BANK.verify_run(run, config, server)["verification"]["checks"]
        self.assertFalse(checks["bodies_release_slots"])
        self.assertFalse(checks["descriptors_released"])

    def test_transport_client_count_is_not_accepted_as_socket_reuse(self):
        run, config, server = self.sample()
        server["connections"] = 10
        server["requests_by_connection"] = {str(index): 1 for index in range(10)}
        checks = BANK.verify_run(run, config, server)["verification"]["checks"]
        self.assertFalse(checks["keep_alive_reused"])
        self.assertFalse(checks["connections_reduced"])


class TransportReportProvenanceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("transport_report_verifier", Path(__file__).with_name("verify_report.py"))
        cls.verifier = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.verifier)

    @staticmethod
    def require(condition, message):
        if not condition:
            raise AssertionError(message)

    def test_current_source_hash_must_match_even_if_source_unchanged_flag_is_true(self):
        from unittest.mock import patch
        report = {"source_unchanged": True, "code": {"source_sha256": "old"}}
        with patch.object(BANK, "code_version", return_value={"source_sha256": "new"}):
            with self.assertRaisesRegex(AssertionError, "current sources differ"):
                self.verifier.verify_provenance(report, self.require)

    def test_binary_hash_detects_replaced_executable(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "benchmark"
            binary.write_bytes(b"measured binary")
            report = {"source_unchanged": True, "code": {"source_sha256": "same"},
                      "binary": {"path": "benchmark", "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "bytes": binary.stat().st_size},
                      "build": {"command": BANK.shlex.join(BANK.BUILD)}, "configuration": {"command": "python3 scripts/crawler-transport-bench.py --tls"}}
            with patch.object(self.verifier, "ROOT", root), patch.object(BANK, "code_version", return_value={"source_sha256": "same"}):
                self.verifier.verify_provenance(report, self.require)
                binary.write_bytes(b"unmeasured binary")
                with self.assertRaisesRegex(AssertionError, "binary differs"):
                    self.verifier.verify_provenance(report, self.require)

    def matrix(self):
        groups = [("http", count) for count in (2, 32, 128)] + [("https", 2)]
        return {"configuration": {"origins": [2, 32, 128], "repetitions": 3, "tls": True},
                "runs": [{"scheme": scheme, "origins": count, "mode": mode, "repetition": repetition,
                          "artifact": f"{scheme}-{count}-{mode}-{repetition}"}
                         for scheme, count in groups for mode in ("legacy", "pooled") for repetition in range(1, 4)]}

    def test_tls_enabled_requires_all_six_tls_runs(self):
        report = self.matrix()
        self.verifier.verify_matrix(report, self.require)
        report["runs"].pop()
        with self.assertRaisesRegex(AssertionError, "including TLS"):
            self.verifier.verify_matrix(report, self.require)

    def test_duplicate_runs_cannot_replace_a_missing_repetition(self):
        report = self.matrix()
        report["runs"][-1] = dict(report["runs"][-2])
        with self.assertRaisesRegex(AssertionError, "duplicate runs"):
            self.verifier.verify_matrix(report, self.require)


if __name__ == "__main__":
    unittest.main()
