"""C01 orchestration contracts. Run with python3 tests/crawler-bench/test_runner.py."""

from contextlib import contextmanager, redirect_stdout
from copy import deepcopy
import http.client
import importlib.util
import io
import json
import multiprocessing
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("crawler_bench_runner", ROOT / "scripts/crawler-bench.py")
RUNNER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RUNNER
SPEC.loader.exec_module(RUNNER)


@contextmanager
def controlled_server(bodies, hosts):
    """Use the real HTTP handler with its control pipe, without a child build."""
    parent, child = multiprocessing.Pipe()
    thread = threading.Thread(target=RUNNER.serve_fixture, args=(child, bodies, hosts), daemon=True)
    thread.start()
    try:
        if not parent.poll(5):
            raise RuntimeError("Controlled HTTP server did not start")
        port = parent.recv()

        def counts():
            parent.send("counts")
            if not parent.poll(5):
                raise RuntimeError("Controlled HTTP server did not return its ledger")
            return parent.recv()

        yield port, counts
    finally:
        if thread.is_alive():
            parent.send("stop")
        thread.join(5)
        parent.close()
        if thread.is_alive():
            raise RuntimeError("Controlled HTTP server did not stop")


def local_get(port, path, host="origin-000.test"):
    # http.client never follows Location: no fixture URL can escape this socket.
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    try:
        connection.request("GET", path, headers={"Host": host})
        response = connection.getresponse()
        return response.status, dict(response.getheaders()), response.read()
    finally:
        connection.close()


class CorpusContracts(unittest.TestCase):
    def test_committed_corpus_has_distinct_verified_real_snapshots(self):
        metadata, bodies = RUNNER.load_corpus()
        self.assertEqual(len(bodies), 8)
        self.assertEqual(sum(map(len, bodies.values())), 1_980_316)
        self.assertEqual({item["language"].split("-")[0] for item in metadata["files"]},
                         {"en", "es", "da", "ja", "zh"})

    def test_manifest_rejects_changed_bytes_size_duplicate_and_escaping_path(self):
        original = json.loads(RUNNER.CORPUS.read_text())
        mutations = {
            "hash": lambda value: value["fixtures"][0].update(sha256="0" * 64),
            "size": lambda value: value["fixtures"][0].update(bytes=1),
            "duplicate": lambda value: value["fixtures"].append(value["fixtures"][0]),
            "escape": lambda value: value["fixtures"][0].update(path="../../../../etc/passwd"),
            "empty": lambda value: value.update(fixtures=[]),
            "unsafe_id": lambda value: value["fixtures"][0].update(id="../escape"),
        }
        with tempfile.TemporaryDirectory(prefix="crawler-corpus-test-") as temporary:
            manifest = Path(temporary) / "manifest.json"
            for name, mutate in mutations.items():
                with self.subTest(mutation=name):
                    value = deepcopy(original)
                    mutate(value)
                    manifest.write_text(json.dumps(value))
                    with self.assertRaises(ValueError):
                        RUNNER.load_corpus(manifest)


class ControlledEndpointContracts(unittest.TestCase):
    def test_http_robots_errors_redirects_and_ledgers_stay_on_local_server(self):
        body = "<html><body>日本語 y español</body></html>".encode()
        hosts = ["origin-000.test", "origin-001.test"]
        with controlled_server({"snapshot": body}, hosts) as (port, counts):
            status, headers, actual = local_get(port, "/fixture/snapshot")
            self.assertEqual((status, actual), (200, body))
            self.assertEqual(headers["Content-Type"], "text/html; charset=utf-8")
            self.assertEqual(int(headers["Content-Length"]), len(body))

            status, _, robots = local_get(port, "/robots.txt")
            self.assertEqual(status, 200)
            self.assertIn(b"Disallow: /blocked", robots)
            status, headers, _ = local_get(port, "/error/429")
            self.assertEqual((status, headers["Retry-After"]), (429, "0"))
            for path, location in [
                ("/redirect/relative", "/fixture/snapshot"),
                ("/redirect/loop", "/redirect/loop"),
                ("/redirect/external", "https://outside.invalid/escape"),
            ]:
                status, headers, actual = local_get(port, path)
                self.assertEqual((status, headers["Location"], actual), (302, location, b""))

            self.assertEqual(local_get(port, "/fixture/snapshot", hosts[1])[0], 200)
            self.assertEqual(local_get(port, "/fixture/snapshot", "unregistered.test")[0], 421)
            ledger = counts()
            self.assertEqual(ledger["requests"], 8)
            self.assertEqual(sum(ledger["by_path"].values()), 8)
            self.assertEqual(sum(ledger["by_origin_path"].values()), 8)
            self.assertEqual(ledger["by_path"]["/fixture/snapshot"], 3)
            self.assertEqual(ledger["statuses"], {"200": 3, "429": 1, "302": 3, "421": 1})
            self.assertNotIn("/blocked", ledger["by_path"])
            self.assertNotIn("/escape", ledger["by_path"])

    def test_large_bodies_equal_recorded_configuration(self):
        # Small configured limits make the wire-size contract cheap to verify.
        with patch.dict(RUNNER.SERVER_CONFIG, {"large_bytes": 4096, "oversized_bytes": 8192}):
            with controlled_server({"snapshot": b"<html></html>"}, ["origin-000.test"]) as (port, counts):
                for path, expected in [("/large", 4096), ("/oversized", 8192)]:
                    with self.subTest(path=path):
                        status, headers, body = local_get(port, path)
                        self.assertEqual(status, 200)
                        self.assertEqual(len(body), expected)
                        self.assertEqual(int(headers["Content-Length"]), expected)
                self.assertEqual(counts()["requests"], 2)


class SummaryContracts(unittest.TestCase):
    def test_median_and_mad_do_not_hide_outlier_range(self):
        self.assertEqual(RUNNER.distribution([10, 12, 1000]),
                         {"median": 12, "min": 10, "max": 1000, "mad": 2})
        self.assertEqual(RUNNER.distribution([4, 4, 4])["mad"], 0)

    def test_summary_includes_stage_dispersion_and_counter_failure(self):
        runs = []
        for index, duration in enumerate([10, 12, 1000]):
            runs.append({"duration_ms": duration, "client_requests": 2,
                         "stages_ms": {"http_headers": duration / 2},
                         "requests_match": index != 2,
                         "resources_io": {"write_bytes": duration},
                         "resources": {"sampled_peak": {"threads": 3, "fds": 4}}})
        row = RUNNER.summarize("slow", 2, True, runs)
        self.assertFalse(row["requests_match"])
        self.assertEqual(row["repetitions"], 3)
        self.assertEqual(row["metrics"]["requests"]["median"], 2)
        self.assertEqual(row["metrics"]["stages_ms"]["http_headers"]["mad"], 1)
        self.assertEqual(row["metrics"]["write_bytes"]["max"], 1000)


class ReportAcceptanceContracts(unittest.TestCase):
    def run_report(self, extra_args, *, mismatch=False, source_changes=False, collapsed_origins=False,
                   observer_decision_changes=False):
        class FakeServer:
            current = None

            def __init__(self, _bodies, hosts):
                self.hosts, self.port = hosts, 12345
                self.ledger = None
                FakeServer.current = self

            def counts(self):
                return self.ledger

            def close(self):
                pass

        def fake_child(_binary, config_path, _timeout):
            config = json.loads(config_path.read_text())
            paths = [path for path in config["paths"] if path != "/blocked"]
            hosts = [origin["host"] for origin in config["origins"]]
            if collapsed_origins:
                hosts = hosts[:1]
            by_path = {path: len(hosts) for path in paths}
            by_origin = {host: len(paths) for host in hosts}
            total = sum(by_path.values())
            FakeServer.current.ledger = {
                "requests": total, "by_path": by_path,
                "by_origin_path": {host + path: 1 for host in hosts for path in paths},
                "statuses": {"200": total},
            }
            result = {
                "duration_ms": 10, "stages_ms": {"http_headers": 5},
                "client_requests": total + int(mismatch), "requests_by_path": by_path,
                "requests_by_origin": by_origin, "blocked_outbound": int("/redirect/external" in paths),
                "saved_pages": len(hosts), "saved_bytes": 100 * len(hosts), "warc_verified": True,
                "resources_io": {"write_bytes": 100},
                "crawl_policy": {"network_policy": "public_only", "max_redirect_hops": 10,
                                 "observer_controls_behavior": False},
                "job_outcomes": {host: {
                    "status": "completed",
                    "counters": {"useful_pages": 1, "page_http_attempts": len(paths), "redirect_hops": 0},
                    "urls": [
                        {"url": f"http://{host}{path}", "outcome": (
                            {"kind": "durable_success", "final_url": f"https://{host}{path}"}
                            if index == 0 else {"kind": "skipped", "reason": (
                                "observer_changed_decision" if observer_decision_changes and config["observer"] else "fixture_skip"
                            )})}
                        for index, path in enumerate(config["paths"])
                    ],
                } for host in hosts},
            }
            config_path.with_name("result.json").write_text(json.dumps(result))
            return {"cpu_seconds": 0.01, "rss_peak_bytes": 1_048_576,
                    "process_wall_ms": 12, "sampled_peak": {"threads": 2, "fds": 3}}

        version = {"commit": "test-commit", "dirty": True, "source_sha256": "same-source"}
        versions = [version, {**version, "source_sha256": "changed" if source_changes else "same-source"}]
        with tempfile.TemporaryDirectory(prefix="crawler-report-test-") as temporary:
            output = Path(temporary) / "report"
            args = ["crawler-bench.py", "--skip-build", "--binary", str(ROOT / "scripts/crawler-bench.py"),
                    "--output", str(output), *extra_args]
            with patch.object(sys, "argv", args), patch.object(RUNNER, "FixtureServer", FakeServer), \
                    patch.object(RUNNER, "run_child", fake_child), patch.object(RUNNER, "hardware", return_value={}), \
                    patch.object(RUNNER, "code_version", side_effect=versions), redirect_stdout(io.StringIO()):
                exit_code = RUNNER.main()
            report = json.loads((output / "report.json").read_text())
            self.assertTrue((output / "report.md").is_file())
            return exit_code, report

    def test_full_matrix_requires_three_runs_for_each_observer_mode(self):
        code, report = self.run_report([])
        self.assertEqual(code, 0)
        self.assertTrue(report["acceptance"]["passed"])
        self.assertEqual(len(report["scenarios"]), 18)
        self.assertEqual(sum(len(row["runs"]) for row in report["scenarios"]), 54)
        self.assertTrue(all(row["repetitions"] == 3 for row in report["scenarios"]))

    def test_partial_matrix_is_saved_but_cannot_claim_c01_acceptance(self):
        code, report = self.run_report(["--origins", "2", "--scenarios", "balanced"])
        self.assertEqual(code, 1)
        self.assertFalse(report["acceptance"]["full_matrix"])
        self.assertFalse(report["acceptance"]["passed"])

    def test_c02_accepts_18_equivalent_runs_without_claiming_full_c01_matrix(self):
        code, report = self.run_report(["--acceptance-profile", "c02"])
        self.assertEqual(code, 0)
        self.assertTrue(report["acceptance"]["passed"])
        self.assertFalse(report["acceptance"]["full_matrix"])
        self.assertTrue(report["acceptance"]["c02_matrix"])
        self.assertTrue(report["acceptance"]["observer_equivalent"])
        self.assertTrue(report["acceptance"]["explicit_outcomes"])
        self.assertEqual(report["configuration"]["origins"], [2])
        self.assertEqual(sum(len(row["runs"]) for row in report["scenarios"]), 18)
        self.assertEqual(report["configuration"]["scenario_paths"]["adversarial"][-1], "/error/429")
        bodies = {entry["id"]: b"" for entry in report["corpus"]["files"]}
        self.assertNotEqual(RUNNER.scenario_paths(bodies)["adversarial"][-1], "/error/429")
        self.assertIn("# C02", RUNNER.markdown_report(report))
        self.assertNotIn("diferencias actuales", RUNNER.markdown_report(report))

    def test_c02_rejects_observer_decisions_even_when_request_totals_match(self):
        code, report = self.run_report(["--acceptance-profile", "c02"], observer_decision_changes=True)
        self.assertEqual(code, 1)
        self.assertFalse(report["acceptance"]["observer_equivalent"])
        self.assertTrue(report["acceptance"]["requests_reconciled"])
        self.assertTrue(report["acceptance"]["explicit_outcomes"])

    def test_c02_requires_all_workloads_and_two_origins(self):
        for arguments in (["--scenarios", "balanced"], ["--origins", "32"]):
            with self.subTest(arguments=arguments):
                code, report = self.run_report(["--acceptance-profile", "c02", *arguments])
                self.assertEqual(code, 1)
                self.assertFalse(report["acceptance"]["c02_matrix"])

    def test_disagreeing_server_count_blocks_full_matrix_acceptance(self):
        code, report = self.run_report([], mismatch=True)
        self.assertEqual(code, 1)
        self.assertFalse(report["acceptance"]["requests_reconciled"])
        self.assertFalse(report["acceptance"]["passed"])

    def test_changed_sources_block_comparison_acceptance(self):
        code, report = self.run_report([], source_changes=True)
        self.assertEqual(code, 1)
        self.assertFalse(report["acceptance"]["source_unchanged"])

    def test_cli_matrix_cannot_replace_observed_origin_coverage(self):
        code, report = self.run_report([], collapsed_origins=True)
        self.assertEqual(code, 1)
        self.assertTrue(report["acceptance"]["full_matrix"])
        self.assertTrue(report["acceptance"]["requests_reconciled"])
        self.assertFalse(report["acceptance"]["origins_verified"])
        self.assertFalse(report["acceptance"]["passed"])


if __name__ == "__main__":
    unittest.main()
