import copy
import json
import os
from pathlib import Path
import unittest
from verify_report import verify

ROOT = Path(__file__).resolve().parents[2]

class VerificationTamperingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(os.environ.get('CRAWLER_PIPELINE_REPORT', ROOT / 'data/crawler-pipeline-bench/report.json'))
        cls.report = json.loads(path.read_text())
        cls.base = path.parent

    def test_real_results_pass(self):
        self.assertTrue(verify(self.report, self.base)['passed'])

    def rejected(self, change, check):
        report = copy.deepcopy(self.report)
        change(report)
        result = verify(report, self.base, artifacts=False)
        self.assertFalse(result['passed'])
        self.assertFalse(result['checks'][check])

    def test_missing_repetition(self):
        self.rejected(lambda r: r['runs'].pop(), 'complete_three_repetition_matrix')

    def test_wrong_server_counter(self):
        self.rejected(lambda r: r['runs'][0]['server'].update(requests=999), 'server_requests_and_bytes')

    def test_changed_paired_limit(self):
        self.rejected(lambda r: r['runs'][0]['configuration']['pipeline'].update(max_inflight_documents=99), 'paired_configuration_and_archives')

    def test_quota_overflow(self):
        self.rejected(lambda r: next(x for x in r['runs'] if x['mode']=='bounded')['pipeline']['storage'].update(peak_bytes=10**12), 'bounded_stage_capacity')

    def test_slow_runtime(self):
        self.rejected(lambda r: next(x for x in r['runs'] if x['mode']=='bounded')['pipeline'].update(runtime_lag_p99_us=100000), 'runtime_and_control_below_100ms')

    def test_rss_over_profile(self):
        self.rejected(lambda r: next(x for x in r['runs'] if x['mode']=='bounded').update(rss_peak_bytes=10**12), 'rss_profile')

    def test_wrong_summary(self):
        self.rejected(lambda r: r['summaries'][0]['metrics']['pages_per_second'].update(median=999), 'summary_distributions')

if __name__ == '__main__':
    unittest.main()
