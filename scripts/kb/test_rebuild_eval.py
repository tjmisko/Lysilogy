import importlib.util
import copy
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


class K2AdapterTests(unittest.TestCase):
    def setUp(self):
        self.api = collector.load_truth_api()
        if self.api is None:
            self.skipTest("E8.4 frozen loader is not yet in this checkout")
        payload = {"message": {"DOI": "10.1234/citing", "title": ["Invented fixture"],
                              "reference": [{"DOI": "10.1234/target", "unstructured": "Invented reference"}, {"DOI": "invalid"}, "malformed"]}}
        self.snapshots = [{"snapshot_id": "a" * 64, "payload": payload,
                          "receipt": {"schema_version": 1, "provider": "crossref", "url": "https://api.crossref.org/works/10.1234%2Fciting",
                                      "retrieved_at": "2026-09-12T00:00:00Z", "frozen_at": "2026-09-12T01:00:00Z",
                                      "body_sha256": self.api.fingerprint(payload), "requested_dois": ["10.1234/citing"]}}]
        self.truth = self.api.build_k2(self.snapshots, [{"doi": "10.1234/citing", "kind": "fixture"}], "2026-09-13T00:00:00Z")

    def should_preserve_original_records_when_genuine_k2_contract_reconstructs(self):
        result = collector.prepare_input(self.truth, self.snapshots, self.api)
        self.assertEqual(self.snapshots[0]["payload"]["message"], result["records"][0]["crossref"])
        self.assertEqual(self.snapshots[0]["receipt"]["retrieved_at"], result["records"][0]["retrieved_at"])
        self.assertEqual(self.truth["sources"], result["sources"])
        self.assertEqual([{"doi": "10.1234/target", "case_id": self.truth["cases"][0]["case_id"]}], result["records"][0]["deposited_references"])

    def should_reject_rehashed_label_changes_when_frozen_records_disagree(self):
        changed = copy.deepcopy(self.truth)
        changed["cases"][0]["expected_doi"] = "10.1234/wrong"
        changed = self.api.finish("K2", changed["built_at"], {key: value for key, value in changed.items()
                                  if key not in {"schema_version", "truth_set", "built_at", "version"}})
        with self.assertRaisesRegex(ValueError, "reconstruction"):
            collector.prepare_input(changed, self.snapshots, self.api)

    def should_reject_source_drift_when_receipt_identity_or_time_changes(self):
        changed = copy.deepcopy(self.snapshots)
        changed[0]["receipt"]["retrieved_at"] = "2026-09-11T00:00:00Z"
        with self.assertRaisesRegex(ValueError, "sources differ"):
            collector.prepare_input(self.truth, changed, self.api)
