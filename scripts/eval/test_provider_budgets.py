"""Offline checks that the independent O30 audit detects invalid traces."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("provider_budgets", Path(__file__).with_name("provider-budgets.py"))
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.fixture = {"references": 2, "providers": ["openalex"], "policy": {"spacing_ms": 1100, "window_ms": 3000, "requests_per_window": 1}, "cooldown_seconds": 1}
        self.events = [{"provider": "openalex", "reference": reference, "at_ms": at, "event": event, "until_ms": until} for reference, at, event, until in
                       [(0, 0, "granted", None), (0, 0, "cooldown", 1000), (0, 0, "restart", None), (1, 0, "deferred", 3000), (1, 3000, "granted", None)]]

    def should_measure_zero_violations_when_every_observed_grant_respects_the_budget(self):
        self.assertEqual(0, collector.validate_trace(self.events, self.fixture)["violations"])

    def should_detect_violations_when_grants_ignore_spacing_window_and_cooldown(self):
        self.events[-1]["at_ms"] = 500
        self.assertEqual(3, collector.validate_trace(self.events, self.fixture)["violations"])

    def should_reject_incomplete_measurement_when_a_reference_never_receives_admission(self):
        with self.assertRaisesRegex(ValueError, "Incomplete trace"):
            collector.validate_trace(self.events[:-1], self.fixture)

    def should_count_duplicate_admission_when_one_reference_is_granted_twice(self):
        repeated = copy.deepcopy(self.events[-1])
        repeated["at_ms"] = 6000
        self.events.append(repeated)
        self.assertEqual(1, collector.validate_trace(self.events, self.fixture)["violations"])


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader)
