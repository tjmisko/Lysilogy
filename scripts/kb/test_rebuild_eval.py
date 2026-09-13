import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("rebuild_eval", Path(__file__).with_name("rebuild-eval.py"))
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)


class RebuildCollectorTests(unittest.TestCase):
    def report(self):
        return {"truth_id": "K2", "source_records": 2, "deposited_pairs": 5,
                "summary": {"works": 7, "aliases": 1}, "identical": True,
                "initial_sha256": "a" * 64, "transactional_sha256": "a" * 64, "deleted_sha256": "a" * 64}

    def should_accept_rebuild_evidence_when_entities_and_aliases_are_identical(self):
        collector.validate(self.report())

    def should_reject_empty_truth_when_a_vacuous_rebuild_would_pass(self):
        report = self.report()
        report["source_records"] = 0
        with self.assertRaises(ValueError):
            collector.validate(report)

    def should_reject_different_ids_when_rebuild_hashes_diverge(self):
        report = self.report()
        report["deleted_sha256"] = "b" * 64
        with self.assertRaises(ValueError):
            collector.validate(report)

    def should_reject_missing_alias_coverage_when_only_entities_are_compared(self):
        report = self.report()
        report["summary"]["aliases"] = 0
        with self.assertRaises(ValueError):
            collector.validate(report)
