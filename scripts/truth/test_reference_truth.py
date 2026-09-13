"""Small invented metadata fixtures exercise contracts; they are never truth-set claims."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import urllib.parse

import freeze_reference as freeze
import reference_truth as truth

AT = "2026-09-13T00:00:00+00:00"


def snapshot(provider, payload, name="fixture"):
    identities = ([truth.doi(payload["message"]["DOI"])] if provider == "crossref"
                  else [truth.doi(row["doi"]) for row in payload.get("results", [payload])])
    url = f"https://api.{provider}.org/works/" + urllib.parse.quote(identities[0], safe="")
    if len(identities) > 1:
        url = f"https://api.{provider}.org/works?" + urllib.parse.urlencode({"filter": "doi:" + "|".join(identities), "per-page": "100"})
    return {"snapshot_id": hashlib.sha256(name.encode()).hexdigest(), "payload": payload,
            "receipt": {"schema_version": 1, "provider": provider, "url": url,
                        "retrieved_at": "2026-09-12T01:00:00Z", "frozen_at": "2026-09-12T02:00:00Z",
                        "body_sha256": truth.fingerprint(payload), "requested_dois": identities, "missing_dois": []}}


def crossref(references):
    return snapshot("crossref", {"message": {"DOI": "10.1234/citing", "title": ["Fixture citing title"],
                    "author": [{"given": "A.", "family": "Example"}], "published": {"date-parts": [[2021]]},
                    "abstract": "This must never enter derived truth", "reference": references}})


def k2(count=8):
    references = [{"DOI": f"10.1234/target-{index}", "article-title": f"Fixture target {index}", "year": 2001 + index}
                  for index in range(count)]
    return truth.build_k2([crossref(references)], [{"doi": "10.1234/citing", "kind": "fixture"}], AT)


def openalex(count=8):
    records = [{"doi": f"https://doi.org/10.1234/target-{index}", "publication_year": 1980 + 10 * (index % 2),
                "primary_topic": {"field": {"id": f"https://openalex.org/fields/{index % 3}", "display_name": "Fixture"}},
                "open_access": {"is_oa": bool(index % 2), "oa_status": "gold" if index % 2 else "closed"}}
               for index in range(count)]
    return snapshot("openalex", {"results": records})


class ReferenceTruthTests(unittest.TestCase):
    def should_skip_references_without_dois_when_building_k2(self):
        source = crossref([{"unstructured": "No deposited DOI"}, {"DOI": "invalid", "author": "X"},
                           {"DOI": "10.1234/TARGET", "unstructured": "A real-looking invented fixture entry"}])
        result = truth.build_k2([source], [{"doi": "10.1234/citing", "kind": "fixture"}], AT)
        self.assertEqual(1, len(result["cases"]))
        self.assertEqual("10.1234/target", result["cases"][0]["expected_doi"])
        self.assertEqual(2, result["coverage"]["references_without_valid_doi"])
        self.assertNotIn("abstract", truth.canonical(result))
        self.assertNotIn("This must never enter derived truth", truth.canonical(result))

    def should_keep_input_separate_from_labels_when_deposits_have_only_identifiers(self):
        source = crossref([{"DOI": "10.1234/hidden"},
                           {"DOI": "10.1234/plain", "article-title": "Fixture without identifier", "year": "2020a"},
                           {"DOI": "10.1234/visible", "unstructured": "Fixture doi:10.1234/VISIBLE"}])
        result = truth.build_k2([source], [{"doi": "10.1234/citing"}], AT)
        cases = {row["expected_doi"]: row for row in result["cases"]}
        self.assertFalse(cases["10.1234/hidden"]["eligible_resolution"])
        self.assertIsNone(cases["10.1234/hidden"]["input"]["text"])
        self.assertNotIn("10.1234/plain", truth.canonical(cases["10.1234/plain"]["input"]))
        self.assertFalse(cases["10.1234/plain"]["expected_identifier_in_input"])
        self.assertTrue(cases["10.1234/visible"]["expected_identifier_in_input"])

    def should_withhold_bibliography_metrics_when_the_input_was_rendered_from_its_field_labels(self):
        structured = {"DOI": "10.1234/one", "article-title": "Fixture", "author": "Example", "year": "2021"}
        unstructured = {**structured, "DOI": "10.1234/two", "unstructured": "Example. Fixture (2021)."}
        result = truth.build_k2([crossref([structured, unstructured])], [{"doi": "10.1234/citing"}], AT)
        cases = {row["expected_doi"]: row for row in result["cases"]}
        self.assertFalse(cases["10.1234/one"]["eligible_bibliography"])
        self.assertTrue(cases["10.1234/two"]["eligible_bibliography"])
        self.assertEqual({"title": "Fixture", "first_author": "Example", "year": "2021"}, cases["10.1234/two"]["field_labels"])

    def should_preserve_year_label_strings_when_deposits_use_numeric_or_suffixed_years(self):
        for deposited, expected in [(2020, "2020"), (" 2020a ", "2020a"), (True, None), ("undated", None)]:
            with self.subTest(deposited=deposited):
                result = truth.build_k2([crossref([{"DOI": "10.1234/one", "year": deposited}])],
                                        [{"doi": "10.1234/citing"}], AT)
                self.assertEqual(expected, result["cases"][0]["field_labels"]["year"])

    def should_reject_wrong_identity_or_mixed_refreshes_when_crossref_sources_are_frozen(self):
        source = crossref([])
        with self.assertRaisesRegex(truth.TruthError, "identity"):
            truth.build_k2([source], [{"doi": "10.1234/different"}], AT)
        with self.assertRaisesRegex(truth.TruthError, "exactly one"):
            truth.build_k2([source, source], [{"doi": "10.1234/citing"}], AT)
        with self.assertRaisesRegex(truth.TruthError, "unsupported fields"):
            truth.build_k2([source], [{"doi": "10.1234/citing", "abstract": "not a label"}], AT)

    def should_stratify_k5_by_reference_field_and_decade_when_sampling_more_than_one_cell(self):
        result = truth.build_k5(k2(), [openalex()], AT, count=6)
        self.assertEqual(6, len(result["samples"]))
        self.assertEqual(6, len({(row["field_id"], row["decade"]) for row in result["samples"]}))
        self.assertTrue(all(row["selected"] == 1 for row in result["coverage"]["strata"]))
        self.assertEqual({1980, 1990}, {row["decade"] for row in result["samples"]})
        self.assertEqual(result, truth.build_k5(k2(), [openalex()], AT, count=6))

    def should_keep_oa_unknown_when_frozen_provider_evidence_is_missing_or_conflicting(self):
        for value in ({}, {"open_access": {"is_oa": True, "oa_status": "closed"}},
                      {"open_access": {"is_oa": False, "oa_status": "gold"}},
                      {"open_access": {"is_oa": "false", "oa_status": "closed"}}):
            self.assertEqual("unknown", truth.oa_label(value)["status"])
        self.assertEqual("open", truth.oa_label({"open_access": {"is_oa": True, "oa_status": "diamond"}})["status"])
        source = openalex()
        source["payload"]["results"][0].pop("open_access")
        result = truth.build_k5(k2(), [source], AT, count=8)
        self.assertEqual(1, result["coverage"]["oa_status"]["unknown"])

    def should_refuse_to_shrink_k5_when_observed_metadata_cannot_supply_the_requested_sample(self):
        with self.assertRaisesRegex(truth.TruthError, "requires 200"):
            truth.build_k5(k2(), [openalex()], AT)
        source = openalex()
        source["payload"]["results"][0].pop("publication_year")
        result = truth.build_k5(k2(), [source], AT, count=7)
        self.assertEqual(1, result["coverage"]["excluded"]["missing_reference_field_or_year"])
        self.assertNotIn("10.1234/target-0", [row["expected_doi"] for row in result["samples"]])

    def should_deduplicate_reference_targets_when_different_citing_entries_share_a_doi(self):
        labels = k2()
        row = copy.deepcopy(labels["cases"][0])
        row["case_id"] = "duplicate"
        labels["cases"].append(row)
        result = truth.build_k5(labels, [openalex()], AT, count=8)
        self.assertEqual(8, len(result["samples"]))
        self.assertEqual(8, len({row["expected_doi"] for row in result["samples"]}))

    def should_freeze_sources_independently_of_provider_ttl_when_loading_verified_snapshots(self):
        with tempfile.TemporaryDirectory(prefix="lysilogy-truth-fixture-") as directory:
            root = Path(directory)
            (root / "responses").mkdir()
            (root / "receipts").mkdir()
            source = crossref([])
            body = truth.canonical(source["payload"]).encode()
            body_hash = hashlib.sha256(body).hexdigest()
            body_path = root / "responses" / (body_hash + ".json")
            body_path.write_bytes(body)
            receipt = {**source["receipt"], "body_file": f"responses/{body_hash}.json"}
            raw = truth.canonical(receipt).encode()
            digest = hashlib.sha256(raw).hexdigest()
            path = root / "receipts" / (digest + ".json")
            path.write_bytes(raw)
            manifest = {"schema_version": 1, "snapshots": [{"path": f"receipts/{digest}.json", "sha256": digest}]}
            loaded = truth.load_snapshots(root, manifest)
            self.assertEqual(source["payload"], loaded[0]["payload"])
            self.assertEqual(source["receipt"]["retrieved_at"], loaded[0]["receipt"]["retrieved_at"])
            body_path.write_bytes(b"{}")
            with self.assertRaisesRegex(truth.TruthError, "payload fingerprint"):
                truth.load_snapshots(root, manifest)

    def should_reject_forged_snapshot_paths_when_manifest_paths_escape_the_frozen_root(self):
        for descriptor in [{"path": "../outside.json", "sha256": "f" * 64},
                           {"path": "/tmp/outside.json", "sha256": "f" * 64}]:
            with self.assertRaisesRegex(truth.TruthError, "receipt identity"):
                truth.load_snapshots(Path("unused"), {"schema_version": 1, "snapshots": [descriptor]})

    def should_reject_impossible_build_dates_when_truth_predates_a_source(self):
        with self.assertRaisesRegex(truth.TruthError, "predates"):
            truth.build_k2([crossref([])], [{"doi": "10.1234/citing"}], "2026-09-11T00:00:00Z")
        with self.assertRaises(truth.TruthError):
            truth.timestamp("2026-09-13")

    def should_reject_crossref_seed_swaps_when_both_returned_and_requested_dois_are_known(self):
        source = crossref([])
        source["receipt"]["requested_dois"] = ["10.1234/other"]
        source["receipt"]["url"] = "https://api.crossref.org/works/10.1234%2Fother"
        with self.assertRaisesRegex(truth.TruthError, "mismatched"):
            truth.build_k2([source], [{"doi": "10.1234/citing"}, {"doi": "10.1234/other"}], AT)

    def should_reject_foreign_batch_records_when_openalex_response_contains_unrequested_dois(self):
        source = openalex()
        source["payload"]["results"][0]["doi"] = "https://doi.org/10.1234/foreign"
        with self.assertRaisesRegex(truth.TruthError, "mismatched"):
            truth.build_k5(k2(), [source], AT, count=7)
        source = openalex()
        source["receipt"]["url"] += "&api_key=must-not-be-frozen"
        with self.assertRaisesRegex(truth.TruthError, "lookup differs"):
            truth.build_k5(k2(), [source], AT, count=7)

    def should_consume_aggregate_truth_larger_than_one_response_when_loading_a_complete_k2(self):
        cache = Path.home() / ".cache/lysilogy"
        cache.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="truth-aggregate-fixture-", dir=cache) as directory:
            path = Path(directory) / "aggregate.json"
            result = k2()
            result["coverage"]["fixture_padding"] = "x" * truth.MAX_BYTES
            path.write_text(truth.canonical(result))
            self.assertGreater(path.stat().st_size, truth.MAX_BYTES)
            self.assertEqual("K2", truth.read_json(path)["truth_set"])
            with self.assertRaisesRegex(truth.TruthError, "size bound"):
                truth.read_json(path, truth.MAX_BYTES)

    def should_change_cell_order_with_seed_when_the_sample_cannot_cover_every_stratum(self):
        candidates = [{"expected_doi": f"10.1234/{index}", "field_id": str(index), "decade": 2000} for index in range(20)]
        orders = [truth.balanced_sample(candidates, 3, seed) for seed in ("one", "two", "three")]
        self.assertNotEqual(orders[0], orders[1])
        self.assertTrue(any(row["field_id"] not in {"0", "1", "10"} for row in orders[0]))
        self.assertEqual(orders[0], truth.balanced_sample(list(reversed(candidates)), 3, "one"))

    def should_reuse_exact_frozen_bytes_when_upstream_or_ttl_later_changes(self):
        source = crossref([{"DOI": "10.1234/target", "article-title": "Fixture"}])
        response = {**source["receipt"], "body": truth.canonical(source["payload"]), "cache_hit": False,
                    "wall_seconds": 0.01, "model_calls": 0, "model_cost_usd": 0, "provider_cost_usd": None}
        request = {"provider": "crossref", "dois": ["10.1234/citing"]}
        with tempfile.TemporaryDirectory(prefix="truth-freeze-fixture-") as directory:
            root = Path(directory)
            first = freeze.freeze_request(root, request, lambda _: response)
            def fail(_):
                self.fail("An immutable frozen lookup must not contact a provider again")
            self.assertEqual(first, freeze.freeze_request(root, request, fail))
            loaded = truth.load_snapshots(root, {"schema_version": 1, "snapshots": [first]})
            self.assertEqual(source["payload"], loaded[0]["payload"])

    def should_leave_existing_truth_untouched_when_an_output_version_is_reused(self):
        with tempfile.TemporaryDirectory(prefix="truth-write-fixture-") as directory:
            path = Path(directory) / "version.json"
            truth.write_immutable(path, b'{"version":1}')
            truth.write_immutable(path, b'{"version":1}')
            with self.assertRaisesRegex(truth.TruthError, "Existing truth differs"):
                truth.write_immutable(path, b'{"version":2}')
            self.assertEqual(b'{"version":1}', path.read_bytes())

    def should_preserve_unreturned_dois_as_unknown_when_a_frozen_batch_has_partial_coverage(self):
        source = openalex()
        source["payload"]["results"].pop()
        response = {**source["receipt"], "body": truth.canonical(source["payload"]),
                    "body_sha256": truth.fingerprint(source["payload"])}
        _, missing = truth.validate_response({"provider": "openalex", "dois": source["receipt"]["requested_dois"]}, response)
        self.assertEqual(["10.1234/target-7"], missing)

    def should_build_seed_plans_without_abstracts_when_observed_arxiv_metadata_has_dois(self):
        papers = [{"id": f"2001.{index:05}", "doi": f"10.1234/{index}", "categories": [category], "abstract": "not a label"}
                  for index, category in enumerate(["cs.LG", "math.PR", "stat.ML", "cs.CV", "math.ST", "stat.TH"])]
        selection = {"papers": papers, "selection_sha256": truth.fingerprint(papers)}
        result = freeze.seed_plan(selection, 3, "fixture")
        self.assertEqual(3, len(result["seeds"]))
        self.assertNotIn("abstract", truth.canonical(result))
        self.assertNotIn("not a label", truth.canonical(result))

    def should_reject_rehashed_identity_swaps_when_loading_a_frozen_receipt(self):
        source = crossref([])
        source["payload"]["message"]["DOI"] = "10.1234/other"
        body = truth.canonical(source["payload"]).encode()
        digest = hashlib.sha256(body).hexdigest()
        receipt = {**source["receipt"], "body_sha256": digest, "body_file": f"responses/{digest}.json"}
        raw = truth.canonical(receipt).encode()
        receipt_hash = hashlib.sha256(raw).hexdigest()
        with tempfile.TemporaryDirectory(prefix="truth-rehashed-fixture-") as directory:
            root = Path(directory)
            truth.write_immutable(root / receipt["body_file"], body)
            truth.write_immutable(root / f"receipts/{receipt_hash}.json", raw)
            manifest = {"schema_version": 1, "snapshots": [{"path": f"receipts/{receipt_hash}.json", "sha256": receipt_hash}]}
            with self.assertRaisesRegex(truth.TruthError, "mismatched"):
                truth.load_snapshots(root, manifest)

    def should_reject_invalid_identifiers_when_a_freeze_plan_contains_null_or_unnormalized_dois(self):
        for identifiers in ([None], [""], ["10.1234/UPPER"], ["10.1234/a", "10.1234/a"]):
            with self.assertRaises(truth.TruthError):
                truth.validate_request({"provider": "crossref", "dois": identifiers})

    def should_refuse_raw_snapshot_roots_when_the_path_is_in_the_repository_or_tmp(self):
        for path in (Path.cwd() / "raw", Path("/tmp/truth-raw"), Path.home() / ".cache/lysilogy/../../../tmp/truth-raw"):
            with self.assertRaisesRegex(truth.TruthError, "dedicated directory"):
                freeze.external_root(path)

    def should_reject_swapped_valid_descriptors_when_a_frozen_request_reuses_another_lookup(self):
        with tempfile.TemporaryDirectory(prefix="truth-swapped-request-fixture-") as directory:
            root = Path(directory)
            descriptors = []
            for identifier in ("10.1234/a", "10.1234/b"):
                source = snapshot("crossref", {"message": {"DOI": identifier, "reference": []}})
                response = {**source["receipt"], "body": truth.canonical(source["payload"]), "cache_hit": False,
                            "wall_seconds": 0.01, "model_calls": 0, "model_cost_usd": 0, "provider_cost_usd": None}
                descriptors.append(freeze.freeze_request(root, {"provider": "crossref", "dois": [identifier]}, lambda _: response))
            request = {"provider": "crossref", "dois": ["10.1234/a"]}
            path = root / "requests" / (truth.fingerprint(request) + ".json")
            manifest = truth.read_json(path)
            manifest["snapshots"] = [descriptors[1]]
            path.write_text(truth.canonical(manifest))
            with self.assertRaisesRegex(truth.TruthError, "does not belong"):
                freeze.freeze_request(root, request, lambda _: self.fail("must not fetch"))
            for snapshots in ([], descriptors):
                manifest["snapshots"] = snapshots
                path.write_text(truth.canonical(manifest))
                with self.assertRaisesRegex(truth.TruthError, "exactly one"):
                    freeze.freeze_request(root, request, lambda _: self.fail("must not fetch"))
