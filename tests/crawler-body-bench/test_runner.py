import copy
import hashlib
import http.client
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
BANK = __import__("crawler-body-bench")
SPEC = importlib.util.spec_from_file_location("body_report_verifier", Path(__file__).with_name("verify_report.py"))
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def matrix():
    return {"configuration": {"profile": BANK.PROFILE, "scenarios": BANK.SCENARIOS, "repetitions": 3},
            "runs": [{"scenario": scenario, "repetition": rep, "artifact": f"{scenario}-{rep}"}
                     for scenario in BANK.SCENARIOS for rep in range(1, 4)]}


class IntegrityTests(unittest.TestCase):
    def test_matrix_is_exact_and_three_repetitions(self):
        VERIFY.verify_matrix(matrix(), check)
        for mutate in [lambda report: report["runs"].pop(),
                       lambda report: report["runs"].append(report["runs"][0]),
                       lambda report: report["runs"][1].update(artifact=report["runs"][0]["artifact"]),
                       lambda report: report["configuration"].update(repetitions=2),
                       lambda report: report["configuration"].update(profile={**BANK.PROFILE, "rss_limit_bytes": 999999999})]:
            with self.subTest(mutate=mutate):
                report = copy.deepcopy(matrix())
                mutate(report)
                with self.assertRaises(AssertionError):
                    VERIFY.verify_matrix(report, check)

    def test_source_and_binary_provenance_cannot_be_asserted_by_flag(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            binary = Path(directory) / "body-binary"
            binary.write_bytes(b"controlled fixture executable")
            report = {"source_unchanged": True, "code": {"source_sha256": "original"},
                      "binary": {"path": str(binary.relative_to(ROOT)), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                                 "bytes": binary.stat().st_size},
                      "build": {"command": BANK.shlex.join(BANK.BUILD)}, "configuration": {"command": "python3 scripts/crawler-body-bench.py"}}
            with patch.object(BANK, "code_version", return_value={"source_sha256": "original"}):
                VERIFY.verify_provenance(report, check)
                binary.write_bytes(b"changed")
                with self.assertRaisesRegex(AssertionError, "binary differs"):
                    VERIFY.verify_provenance(report, check)
            with patch.object(BANK, "code_version", return_value={"source_sha256": "changed"}):
                with self.assertRaisesRegex(AssertionError, "sources differ"):
                    VERIFY.verify_provenance(report, check)

    def test_expected_decoding_and_reservation_are_not_wire_size(self):
        rows = BANK.cases("retained_near_cap", {})
        self.assertEqual(len(rows), 8)
        self.assertTrue(all(row["decoded_bytes"] == (2 * 1024 * 1024 - 64) * 3 for row in rows))
        self.assertEqual(BANK.PROFILE["max_reserved_bytes"] // (BANK.PROFILE["wire_limit_bytes"] + 3 * BANK.PROFILE["decoded_limit_bytes"]), 3)
        self.assertEqual(BANK.profile_for("retained_near_cap")["total_timeout_ms"], 2000)
        self.assertEqual(BANK.profile_for("decoded_size")["decoded_limit_bytes"], 1048576)

    def test_ledger_rejects_wrong_cause_lost_bytes_and_memory_overrun(self):
        profile = BANK.profile_for("corpus")
        row = {"host": "origin-000.body.test", "path": "/corpus/tiny", "expected": "admitted",
               "decoded_bytes": 1, "decoded_md5": hashlib.md5(b"x").hexdigest()}
        stats = {"peak_reserved_bytes": 20 * 1048576, "peak_active_bodies": 1, "reserved_bytes": 0,
                 "active_bodies": 0, "bodies_started": 1, "wire_bytes_received": 1,
                 "decoded_bytes_produced": 1, "bodies_completed": 1, "bodies_admitted": 1, "body_failures": 0}
        original = {"configuration": profile, "outcomes": [dict(row, cause="admitted")], "requests": 1,
                    "body_stats": stats, "fd_baseline": 10, "fd_peak": 11, "fd_final": 10,
                    "transport_stats": {"client_slots_active": 0, "requests_started": 1}}
        config = {"scenario": "corpus", "profile": profile, "cases": [row]}
        resources = {"rss_peak_bytes": 64 * 1048576}
        server = {"requests": 1, "body_bytes_written": 1, "body_bytes_planned": 1,
                  "by_request": {row["host"] + row["path"]: {"requests": 1, "body_bytes_written": 1,
                                                          "body_bytes_planned": 1, "completed": True, "disconnected": False}}}
        self.assertTrue(BANK.verify_run(copy.deepcopy(original), config, server, resources)["verification"]["passed"])
        changes = [lambda run: run["outcomes"][0].update(cause="stream_failure"),
                   lambda run: run["body_stats"].update(wire_bytes_received=0),
                   lambda run: run["body_stats"].update(reserved_bytes=20 * 1048576),
                   lambda run: run["body_stats"].update(peak_reserved_bytes=65 * 1048576)]
        for change in changes:
            run = copy.deepcopy(original)
            change(run)
            self.assertFalse(BANK.verify_run(run, config, server, resources)["verification"]["passed"])
        self.assertFalse(BANK.verify_run(copy.deepcopy(original), config, server,
                                        {"rss_peak_bytes": 257 * 1048576})["verification"]["passed"])

    def test_gzip_bomb_fits_wire_limit_and_exceeds_decoded_limit(self):
        fixture = BANK.fixture("gzip_bomb", {}, BANK.PROFILE)
        self.assertLess(fixture["size"], BANK.PROFILE["wire_limit_bytes"])
        self.assertGreater(fixture["inflated_bytes"], BANK.PROFILE["decoded_limit_bytes"])
        self.assertEqual(fixture["encoding"], "gzip")


class ControlledServerTests(unittest.TestCase):
    def setUp(self):
        self.host = "origin-000.body.test"
        self.server = BANK.FixtureServer({"tiny": b"<html>controlled</html>"}, [self.host], BANK.PROFILE)

    def tearDown(self):
        self.server.close()

    def request(self, path):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.port, timeout=3)
        connection.putrequest("GET", path, skip_host=True)
        connection.putheader("Host", self.host)
        connection.endheaders()
        return connection, connection.getresponse()

    def test_conserved_body_matches_independent_server_payload_ledger(self):
        connection, response = self.request("/corpus/tiny")
        self.assertEqual(response.read(), b"<html>controlled</html>")
        connection.close()
        ledger = BANK.settled_counts(self.server)
        self.assertEqual(ledger["requests"], 1)
        self.assertEqual(ledger["body_bytes_written"], ledger["body_bytes_planned"])
        self.assertTrue(ledger["by_request"][self.host + "/corpus/tiny"]["completed"])

    def test_declared_oversize_disconnect_is_measured_before_full_payload(self):
        connection, response = self.request("/declared_size")
        self.assertEqual(int(response.getheader("Content-Length")), 8 * 1024 * 1024)
        response.close()
        connection.close()
        ledger = BANK.settled_counts(self.server)
        self.assertTrue(ledger["by_request"][self.host + "/declared_size"]["disconnected"])
        self.assertLess(ledger["body_bytes_written"], ledger["body_bytes_planned"])

    def test_duplicate_requests_cannot_disappear_from_server_count(self):
        for _ in range(2):
            connection, response = self.request("/corpus/tiny")
            response.read()
            connection.close()
        ledger = BANK.settled_counts(self.server)
        self.assertEqual(ledger["requests"], 2)


if __name__ == "__main__":
    unittest.main()
