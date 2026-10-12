"""Local runner lifecycle contracts; never launch or signal real processes."""

import argparse
from contextlib import ExitStack, redirect_stdout
from copy import deepcopy
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("crawler_local", ROOT / "scripts/crawler-local.py")
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


def arguments(**values):
    defaults = dict(backend="detached", port=None, benchmark_report=None, no_benchmark=False)
    return argparse.Namespace(**(defaults | values))


class DashboardLifecycle(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="crawler-local-test-")))
        data = self.root / "data/crawler-local"
        dashboard = self.root / "tools/crawler-dashboard"
        data.mkdir(parents=True)
        (dashboard / "dist").mkdir(parents=True)
        (dashboard / "dist/index.html").write_text("dashboard")
        binary = self.root / "astiango"
        binary.touch()
        binary.chmod(0o755)
        (data / "status.json").write_text(json.dumps({"pid": 42}))
        for name, value in {"ROOT": self.root, "DATA": data, "MANIFEST": data / "processes.json",
                            "DASHBOARD": dashboard, "BINARY": binary}.items():
            self.stack.enter_context(patch.object(RUNNER, name, value))
        self.stack.enter_context(redirect_stdout(io.StringIO()))
        self.stack.enter_context(patch.object(RUNNER, "bootstrap"))
        self.stack.enter_context(patch.object(RUNNER, "command"))
        self.stack.enter_context(patch.object(RUNNER.shutil, "which", return_value="/usr/bin/node"))
        self.stack.enter_context(patch.object(RUNNER, "owned_process", side_effect=lambda r: r.get("alive", False)))
        self.stack.enter_context(patch.object(RUNNER, "health", return_value=True))
        self.stop = self.stack.enter_context(patch.object(RUNNER, "stop_one"))
        self.launch = self.stack.enter_context(patch.object(
            RUNNER, "launch", side_effect=lambda name, argv, backend, env: {
                "alive": True, "pid": 101, "command": argv, "backend": backend, "log": "unused",
            }))
        self.state = {"root": str(self.root), "port": 3101, "benchmark_report": None, "processes": {
            "dashboard": {"alive": True, "pid": 41, "backend": "detached"},
            "crawler": {"alive": True, "pid": 42, "backend": "detached"},
        }}
        self.crawler = deepcopy(self.state["processes"]["crawler"])

    def assert_crawler_preserved(self):
        self.assertEqual(self.state["processes"]["crawler"], self.crawler)
        self.assertTrue(all(call.args[0] == "dashboard" for call in self.stop.call_args_list))
        self.assertTrue(all(call.args[0] == "dashboard" for call in self.launch.call_args_list))

    def test_new_installation_uses_single_default_port(self):
        self.assertEqual(RUNNER.dashboard_settings({}, arguments()), (3101, None))

    def test_old_registered_port_is_preserved_without_override(self):
        self.assertEqual(RUNNER.dashboard_settings({"port": 3100}, arguments()), (3100, None))

    def test_saved_report_and_port_survive_start(self):
        self.state.update(port=3201, benchmark_report=str(self.root / "report.json"))
        RUNNER.start(self.state, arguments())
        self.stop.assert_not_called()
        self.launch.assert_not_called()
        self.assertEqual(RUNNER.read_manifest(), self.state)
        self.assert_crawler_preserved()

    def test_dashboard_start_tolerates_transient_process_identity_before_health(self):
        self.state["processes"]["dashboard"]["alive"] = False
        checks = []
        def owned(record):
            if record.get("pid") == 101:
                checks.append(record)
                return len(checks) > 1
            return record.get("alive", False)
        with patch.object(RUNNER, "owned_process", side_effect=owned), patch.object(RUNNER.time, "sleep"):
            RUNNER.start(self.state, arguments())
        self.assertEqual(len(checks), 2)
        self.assertEqual(self.launch.call_args.args[0], "dashboard")
        self.assert_crawler_preserved()

    def test_same_configuration_reuses_both_processes(self):
        RUNNER.start(self.state, arguments())
        RUNNER.start(self.state, arguments(port=3101))
        self.stop.assert_not_called()
        self.launch.assert_not_called()
        self.assert_crawler_preserved()

    def test_enabling_report_replaces_only_dashboard_and_persists_settings(self):
        RUNNER.start(self.state, arguments(benchmark_report="data/bank/report.json"))
        report = str(self.root / "data/bank/report.json")
        self.stop.assert_called_once()
        self.launch.assert_called_once()
        self.assertEqual(self.launch.call_args.args[3]["CRAWLER_BENCHMARK_PATH"], report)
        self.assertEqual(self.launch.call_args.args[3]["CRAWLER_STATE_PATH"], str(RUNNER.DATA / "status.json"))
        self.assertEqual(RUNNER.read_manifest()["benchmark_report"], report)
        self.assert_crawler_preserved()
        RUNNER.start(self.state, arguments())
        self.assertEqual(self.launch.call_count, 1)

    def test_changing_port_replaces_only_dashboard(self):
        RUNNER.start(self.state, arguments(port=3201))
        self.stop.assert_called_once()
        self.launch.assert_called_once()
        self.assertEqual(self.launch.call_args.args[3]["PORT"], "3201")
        self.assertEqual(RUNNER.read_manifest()["port"], 3201)
        self.assert_crawler_preserved()

    def test_disabling_report_clears_persisted_and_inherited_configuration(self):
        self.state["benchmark_report"] = str(self.root / "report.json")
        with patch.dict(RUNNER.os.environ, {"CRAWLER_BENCHMARK_PATH": "/inherited/report.json"}):
            RUNNER.start(self.state, arguments(no_benchmark=True))
        self.assertEqual(self.launch.call_args.args[3]["CRAWLER_BENCHMARK_PATH"], "")
        self.assertIsNone(RUNNER.read_manifest()["benchmark_report"])
        self.assert_crawler_preserved()

    def test_dead_dashboard_is_replaced_without_signals(self):
        self.state["processes"]["dashboard"]["alive"] = False
        RUNNER.start(self.state, arguments())
        self.stop.assert_not_called()
        self.launch.assert_called_once()
        self.assert_crawler_preserved()

    def test_invalid_saved_port_is_rejected_before_process_changes(self):
        self.state["port"] = 70000
        with self.assertRaisesRegex(RuntimeError, "puerto"):
            RUNNER.start(self.state, arguments())
        self.stop.assert_not_called()
        self.launch.assert_not_called()
        self.assert_crawler_preserved()

    def test_absolute_report_path_is_preserved(self):
        report = str(self.root / "report.json")
        self.assertEqual(RUNNER.dashboard_settings(self.state, arguments(benchmark_report=report)), (3101, report))


if __name__ == "__main__":
    unittest.main()
