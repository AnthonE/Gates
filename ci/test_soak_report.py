"""A truncated or shortened run must never become a successful soak report."""
import copy
import json
import unittest

from soak_report import summarize


class SoakReportTest(unittest.TestCase):
    def setUp(self):
        self.rows = [
            dict(kind='start', clients=1, seconds=3600),
            dict(kind='jitter', elapsed_us=3_600_000_000,
                 minimum_clients_polled_100ms=1, client_ended_early=False,
                 absolute_error_us_log2_bins=[1, 1, 0, 2] + [0] * 28),
            dict(kind='client', client=0, elapsed_us=3_600_000_000,
                 dg_in_bytes=3600, stream_in_bytes=3600,
                 dg_out_bytes=1800, stream_out_bytes=0, decode_errors=0,
                 event_decode_errors=0, walk_truncated=False),
            dict(kind='complete', clients=1, passed=True),
        ]

    def report(self, rows):
        return summarize(map(json.dumps, rows))

    def test_units_and_histogram_bounds(self):
        report = self.report(self.rows)
        self.assertEqual(report['bandwidth']['downstream']['bytes_per_second_median'], 2)
        self.assertEqual(report['absolute_jitter_percentile_bounds']['99'],
                         dict(lower_us=4, upper_exclusive_us=8))

    def test_incomplete_run_is_rejected(self):
        for index in range(len(self.rows)):
            with self.subTest(missing=index), self.assertRaises(ValueError):
                self.report(self.rows[:index] + self.rows[index + 1:])
        for index, key, value in [(1, 'elapsed_us', 100),
                                  (1, 'minimum_clients_polled_100ms', 0),
                                  (1, 'client_ended_early', True),
                                  (2, 'decode_errors', 1),
                                  (2, 'elapsed_us', 100),
                                  (3, 'passed', False)]:
            rows = copy.deepcopy(self.rows)
            rows[index][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.report(rows)


if __name__ == '__main__':
    unittest.main()
