import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

MODULE = Path(__file__).with_name("verify_report.py")
spec = importlib.util.spec_from_file_location("c06_verifier", MODULE)
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)

class VerificationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        binary = self.root / "binary"
        binary.write_bytes(b"test-binary")
        manifest = json.loads((verifier.ROOT / "tests/crawler-bench/manifest.json").read_text())
        self.report = {"configuration": {"network": "none"},
            "code": {"source_files": {"tests/crawler-extraction-bench/verify_report.py": hashlib.sha256(MODULE.read_bytes()).hexdigest()}},
            "binary": {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()},
            "runs": [], "corpus_cpu_reduction_pct": {"median": 60}}
        for name in [row["id"] for row in manifest["fixtures"]] + ["anchors-300000"]:
            for repetition in (1, 2, 3):
                for mode in ("legacy", "shared"):
                    urls = [f"https://example.org/page-{n}" for n in range(512 if name == "anchors-300000" else 1)]
                    self.report["runs"].append({"scenario": name, "mode": mode, "repetition": repetition,
                        "result_file": f"{name}-{mode}-{repetition}.json", "cpu_ms_per_page": 10 if mode == "legacy" else 4,
                        "allocated_bytes_per_page": 1000 if mode == "legacy" else 400, "allocation_calls_per_page": 100,
                        "dom_parses_per_page": 2 if mode == "legacy" else 1,
                        "extraction": {"url": "https://example.org/", "base_url": "https://example.org/", "canonical_url": None,
                            "no_index": False, "no_follow": False, "discovery_urls": urls, "internal_urls": urls,
                            "anchors_visited": len(urls), "traversal_stopped": name == "anchors-300000"}})
        self.path = self.root / "report.json"

    def check(self):
        for run in self.report["runs"]:
            raw = {key: value for key, value in run.items() if key not in ("scenario", "repetition", "result_file", "result_sha256")}
            path = self.root / run["result_file"]
            path.write_text(json.dumps(raw))
            run["result_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        self.path.write_text(json.dumps(self.report))
        return verifier.verify(self.path)

    def test_complete_equivalent_matrix_passes(self):
        self.assertTrue(self.check()["passed"])

    def test_missing_repetition_is_rejected(self):
        self.report["runs"].pop()
        self.assertFalse(self.check()["checks"]["complete_matrix"])

    def test_matching_artifact_hash_does_not_hide_changed_robots(self):
        self.report["runs"][1]["extraction"]["no_follow"] = True
        self.assertFalse(self.check()["checks"]["equivalent_links_metadata"])

    def test_cpu_goal_is_recomputed_from_runs(self):
        for run in self.report["runs"]:
            if run["mode"] == "shared": run["cpu_ms_per_page"] = 9
        result = self.check()
        self.assertFalse(result["checks"]["cpu_target_at_least_30_percent"])
        self.assertFalse(result["checks"]["cpu_summary_matches"])

    def test_extra_links_are_rejected(self):
        run = next(run for run in self.report["runs"] if run["scenario"] == "anchors-300000" and run["mode"] == "shared")
        run["extraction"]["internal_urls"].append("https://example.org/overflow")
        self.assertFalse(self.check()["checks"]["bounded_projections"])

    def test_binary_tampering_is_rejected(self):
        Path(self.report["binary"]["path"]).write_bytes(b"changed")
        self.assertFalse(self.check()["checks"]["binary_unchanged"])

if __name__ == "__main__":
    unittest.main()
