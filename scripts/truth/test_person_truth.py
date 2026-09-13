"""Invented, tiny provider fixtures exercise K4 contracts; they are never actual truth."""
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import freeze_reference as freeze
import person_truth as person
import reference_truth as truth
from test_reference_truth import AT, k2, snapshot

ALPHA = "0000-0002-1825-0097"
BETA = "0000-0001-5109-3700"
GAMMA = "0000-0002-1694-233X"


def authorship(name="A. Example", raw=ALPHA, profile=ALPHA, author_id="https://openalex.org/A1", position="middle"):
    return {"raw_author_name": name, "raw_orcid": raw, "author_position": position,
            "author": {"id": author_id, "display_name": "Resolved Name Is Not The Input", "orcid": profile}}


def selection(identifiers):
    papers = [{"id": f"2501.{index:05}", "doi": identifier, "tiers": ["eval", "scale"],
               "abstract": "Not a person feature"} for index, identifier in enumerate(identifiers)]
    return {"schema_version": 2, "selection_sha256": truth.fingerprint(papers), "papers": papers}


def build(bylines):
    records = [{"doi": f"10.1234/work-{index}", "authorships": row} for index, row in enumerate(bylines)]
    membership = person.universe(selection=selection([row["doi"] for row in records]))
    return person.build_k4(membership, [snapshot("openalex", {"results": records})], AT)


class PersonTruthTests(unittest.TestCase):
    def should_cluster_mentions_when_they_share_a_source_deposited_orcid(self):
        labels = build([[authorship("A. Example")], [authorship("Alice Example", author_id="https://openalex.org/A999")]])
        self.assertEqual(1, len(labels["clusters"]))
        self.assertEqual(2, len(labels["clusters"][0]["mention_ids"]))
        self.assertEqual("https://orcid.org/" + ALPHA, labels["clusters"][0]["expected_orcid"])
        self.assertEqual(1, labels["coverage"]["same_person_pairs"])
        self.assertEqual(1, labels["coverage"]["multi_work_clusters"])

    def should_exclude_a_mention_when_its_position_carries_conflicting_valid_orcids(self):
        labels = build([[authorship(raw=ALPHA, profile=BETA)]])
        slot = labels["mentions"][0]
        self.assertFalse(slot["eligible"])
        self.assertIsNone(slot["expected_orcid"])
        self.assertEqual(["conflicting_raw_profile_orcids"], slot["exclusion_reasons"])
        self.assertEqual(ALPHA, slot["observed"]["raw_orcid"])
        self.assertEqual(BETA, slot["observed"]["profile_orcid"])

    def should_keep_array_positions_distinct_when_several_authors_are_middle_authors(self):
        labels = build([[authorship(raw=ALPHA, profile=ALPHA), authorship(raw=BETA, profile=BETA)]])
        self.assertEqual([0, 1], [row["input"]["authorship_index"] for row in labels["mentions"]])
        self.assertEqual(2, len({row["mention_id"] for row in labels["mentions"]}))
        self.assertEqual(2, labels["coverage"]["singleton_clusters"])

    def should_exclude_every_affected_slot_when_one_orcid_is_repeated_for_coauthors(self):
        labels = build([[authorship("A. First"), authorship("B. Second"), authorship("C. Third", BETA, BETA)]])
        self.assertEqual(1, labels["coverage"]["eligible_mentions"])
        self.assertEqual(2, labels["coverage"]["exclusion_reasons"]["duplicate_orcid_across_coauthors"])
        conflict = labels["conflicts"]["duplicate_orcid_across_coauthors"][0]
        self.assertEqual(2, len(conflict["mention_ids"]))
        self.assertEqual(1, len(labels["clusters"]))

    def should_reject_profile_only_labels_when_orcid_was_not_deposited_on_the_authorship(self):
        labels = build([[authorship(raw=None), authorship("Independent Source", BETA, None)]])
        self.assertEqual(["profile_only_orcid"], labels["mentions"][0]["exclusion_reasons"])
        self.assertTrue(labels["mentions"][1]["eligible"])
        self.assertEqual("https://orcid.org/" + BETA, labels["mentions"][1]["expected_orcid"])

    def should_validate_orcid_shape_and_checksum_when_reading_public_identifiers(self):
        for value in (ALPHA, BETA, GAMMA, "https://orcid.org/" + ALPHA, "http://orcid.org/" + ALPHA,
                      " HTTPS://ORCID.ORG/" + ALPHA + " "):
            with self.subTest(value=value):
                self.assertIsNotNone(person.orcid(value))
        invalid = [None, 123, True, {}, [], ALPHA[:-1] + "8", GAMMA.lower(), ALPHA.replace("-", ""),
                   "https://orcid.org.evil/" + ALPHA, "https://evil@orcid.org/" + ALPHA,
                   "https://orcid.org/" + ALPHA + "?extra=1", "https://orcid.org/" + ALPHA + "/",
                   "https://orcid.org/" + ALPHA + "#x", "https://orcid.org:443/" + ALPHA,
                   "orcid:" + ALPHA, ALPHA + "\n", ALPHA.replace("0", "０"), "x" * 200]
        for value in invalid:
            with self.subTest(value=value):
                self.assertIsNone(person.orcid(value))

    def should_distinguish_invalid_identifiers_from_conflicts_when_bad_deposits_are_observed(self):
        labels = build([[authorship(raw=ALPHA[:-1] + "8"), authorship(raw=BETA, profile="bad-profile"),
                        authorship(raw=None, profile=None), authorship(raw=GAMMA, profile=ALPHA)]])
        self.assertEqual({"invalid_raw_orcid": 1, "invalid_profile_orcid": 1, "missing_raw_orcid": 1,
                          "conflicting_raw_profile_orcids": 1}, labels["coverage"]["exclusion_reasons"])
        self.assertEqual("no_eligible_labels", labels["coverage"]["label_status"])

    def should_preserve_original_names_and_observations_when_labels_are_normalized(self):
        raw = " http://orcid.org/" + ALPHA + " "
        labels = build([[authorship("  Łukasz García 王  ", raw, None)]])
        slot = labels["mentions"][0]
        self.assertEqual(raw, slot["observed"]["raw_orcid"])
        self.assertEqual("  Łukasz García 王  ", slot["input"]["raw_name"])
        self.assertEqual("https://orcid.org/" + ALPHA, slot["expected_orcid"])
        self.assertEqual("/results/0/authorships/0", slot["provenance"]["json_pointer"])
        self.assertEqual("2026-09-12T01:00:00Z", slot["provenance"]["retrieved_at"])

    def should_keep_same_name_people_distinct_when_their_deposited_orcids_differ(self):
        labels = build([[authorship("Alex Example", ALPHA, ALPHA)], [authorship("Alex Example", BETA, BETA)]])
        self.assertEqual(2, len(labels["clusters"]))
        self.assertEqual(0, labels["coverage"]["same_person_pairs"])
        self.assertEqual(1, labels["coverage"]["provider_profile_conflicts"])
        self.assertEqual(2, labels["coverage"]["eligible_mentions"])

    def should_keep_ids_and_label_provenance_out_when_projecting_clustering_features(self):
        labels = build([[authorship()], [authorship("A. Other")]])
        output = person.features(labels)
        serialized = truth.canonical(output)
        for forbidden in (ALPHA, "orcid", "A1", "Resolved Name", "snapshot", "retrieved_at", "authorship_sha256"):
            self.assertNotIn(forbidden, serialized)
        self.assertEqual({"mention_id", "raw_name", "work_doi", "authorship_index"}, set(output["mentions"][0]))
        changed = copy.deepcopy(labels)
        for slot in changed["mentions"]:
            slot["expected_orcid"] = "Changed private evaluator label"
            slot["observed"]["provider_author_id"] = "Changed profile"
        self.assertEqual(output, person.features(changed))
        other = build([[authorship(raw=BETA, profile=BETA)], [authorship("A. Other", BETA, BETA)]])
        self.assertEqual(output, person.features(other))

    def should_exclude_leaking_names_when_identifiers_appear_in_raw_author_text(self):
        names = ["Example " + ALPHA, "Example " + ALPHA.replace("-", ""), "Example https://orcid.org/" + ALPHA,
                 "Example " + ALPHA.replace("-", "–"), "Example " + ALPHA.replace("0", "０"),
                 "Example " + ALPHA.replace("-", "-\u200b")]
        for name in names:
            with self.subTest(name=name):
                labels = build([[authorship(name)]])
                self.assertIn("identifier_in_raw_name", labels["mentions"][0]["exclusion_reasons"])
                self.assertEqual([], person.features(labels)["mentions"])
        labels = build([[authorship()]])
        labels["mentions"][0]["input"]["raw_name"] = ALPHA
        with self.assertRaisesRegex(truth.TruthError, "leaking"):
            person.features(labels)

    def should_report_unknown_slots_when_provider_names_or_bylines_are_missing(self):
        labels = build([[None, authorship(None, None, None), authorship("x" * 1025, BETA, BETA),
                        authorship("A.\nExample", GAMMA, GAMMA)]])
        self.assertEqual(4, labels["coverage"]["excluded_mentions"])
        self.assertEqual(1, labels["coverage"]["exclusion_reasons"]["malformed_authorship"])
        self.assertEqual(1, labels["coverage"]["exclusion_reasons"]["oversize_raw_name"])
        self.assertNotIn("x" * 1025, truth.canonical(labels))
        membership = person.universe(selection=selection(["10.1234/a", "10.1234/b", "10.1234/c"]))
        source = snapshot("openalex", {"results": [{"doi": "10.1234/a"}]})
        source["receipt"]["requested_dois"] += ["10.1234/b"]
        source["receipt"]["missing_dois"] = ["10.1234/b"]
        source["receipt"]["url"] = "https://api.openalex.org/works?filter=doi:10.1234/a%7C10.1234/b&per-page=100"
        labels = person.build_k4(membership, [source], AT)
        self.assertEqual({"missing_authorships": 1, "missing_provider_record": 1, "not_requested": 1}, labels["coverage"]["work_status"])
        self.assertEqual(2, labels["coverage"]["requested_works"])

    def should_report_observed_truncation_when_work_completeness_is_uncertain(self):
        for record, count, status in [({}, 99, "unknown"), ({}, 100, "possible_provider_cap"),
                                     ({"is_authors_truncated": True}, 100, "observed_truncated"),
                                     ({"authors_count": 200}, 100, "observed_truncated"),
                                     ({"authors_count": 2}, 2, "reported_complete"),
                                     ({"is_authors_truncated": False}, 2, "reported_complete"),
                                     ({"authors_count": 1}, 2, "contradictory_completeness_metadata"),
                                     ({"authors_count": 3, "is_authors_truncated": False}, 2, "contradictory_completeness_metadata"),
                                     ({"authors_count": True}, 1, "invalid_completeness_metadata")]:
            with self.subTest(record=record):
                result = person.completeness(record, count)
                self.assertEqual(status, result["status"])
                self.assertEqual(record.get("authors_count"), result["reported_authors_count"])

    def should_bind_each_membership_origin_when_k2_and_k0_refer_to_the_same_work(self):
        references = k2(2)
        selected = selection(["10.1234/citing", "10.1234/target-0", None, "bad-doi"])
        membership = person.universe(references, selected)
        self.assertEqual(3, membership["coverage"]["unique_dois"])
        works = {row["doi"]: row for row in membership["works"]}
        self.assertEqual({"K2_citing", "K0_selected_metadata"}, {row["kind"] for row in works["10.1234/citing"]["origins"]})
        self.assertEqual({"K2_reference", "K0_selected_metadata"}, {row["kind"] for row in works["10.1234/target-0"]["origins"]})
        self.assertEqual(1, membership["coverage"]["K0_selected_metadata_missing_doi"])
        self.assertEqual(1, membership["coverage"]["K0_selected_metadata_invalid_doi"])
        self.assertEqual(truth.fingerprint(references), membership["inputs"][0]["content_sha256"])
        self.assertEqual(truth.fingerprint(selected), membership["inputs"][1]["content_sha256"])
        self.assertNotIn("abstract", truth.canonical(membership))

    def should_reject_drifted_membership_when_source_or_universe_fingerprints_change(self):
        selected = selection(["10.1234/a"])
        selected["papers"][0]["doi"] = "10.1234/b"
        with self.assertRaisesRegex(truth.TruthError, "fingerprint"):
            person.universe(selection=selected)
        references = k2()
        references["cases"][0]["expected_doi"] = "10.1234/drift"
        with self.assertRaisesRegex(truth.TruthError, "fingerprint"):
            person.universe(references)
        membership = person.universe(selection=selection(["10.1234/a"]))
        membership["works"][0]["doi"] = "10.1234/b"
        with self.assertRaisesRegex(truth.TruthError, "fingerprint"):
            person.request_plan(membership)

    def should_use_fixed_bounded_lookup_plans_when_dois_require_special_filter_handling(self):
        membership = person.universe(selection=selection(["10.1234/a", "10.1234/a|b", "10.1234/a,b", "10.1234/b"]))
        plan = person.request_plan(membership, batch_size=2, limit=2)
        self.assertEqual(3, plan["total_requests"])
        self.assertEqual(["10.1234/a", "10.1234/b"], plan["requests"][0]["dois"])
        self.assertEqual(1, len(plan["requests"][1]["dois"]))
        self.assertEqual(1, len(person.request_plan(membership, 2, offset=2)["requests"]))
        for options in ({"batch_size": 0}, {"limit": 101}, {"offset": -1}, {"batch_size": True}):
            with self.subTest(options=options), self.assertRaises(truth.TruthError):
                person.request_plan(membership, **options)

    def should_reject_foreign_or_refreshed_records_when_building_from_frozen_requests(self):
        membership = person.universe(selection=selection(["10.1234/a", "10.1234/b"]))
        source = snapshot("openalex", {"doi": "10.1234/a", "authorships": [authorship()]})
        with self.assertRaisesRegex(truth.TruthError, "once"):
            person.build_k4(membership, [source, source], AT)
        foreign = snapshot("openalex", {"doi": "10.1234/outside"})
        with self.assertRaisesRegex(truth.TruthError, "foreign"):
            person.build_k4(membership, [foreign], AT)
        source["payload"]["doi"] = "10.1234/b"
        with self.assertRaisesRegex(truth.TruthError, "mismatched"):
            person.build_k4(membership, [source], AT)

    def should_reject_impossible_builds_when_membership_or_sources_are_newer(self):
        membership = person.universe(k2())
        source = snapshot("openalex", {"doi": "10.1234/citing"})
        with self.assertRaisesRegex(truth.TruthError, "predate"):
            person.build_k4(membership, [source], "2026-09-12T03:00:00Z")
        membership = person.universe(selection=selection(["10.1234/citing"]))
        with self.assertRaisesRegex(truth.TruthError, "predates"):
            person.build_k4(membership, [source], "2026-09-12T01:00:00Z")
        with self.assertRaisesRegex(truth.TruthError, "frozen OpenAlex"):
            person.build_k4(membership, [], AT)

    def should_rebuild_identically_when_receipts_are_reused_offline(self):
        source = snapshot("openalex", {"doi": "10.1234/a", "authorships": [authorship()]})
        response = {**source["receipt"], "body": truth.canonical(source["payload"]), "cache_hit": False,
                    "wall_seconds": 0.01, "model_calls": 0, "model_cost_usd": 0, "provider_cost_usd": None}
        request = {"provider": "openalex", "dois": ["10.1234/a"]}
        membership = person.universe(selection=selection(request["dois"]))
        with tempfile.TemporaryDirectory(prefix="person-truth-fixture-") as directory:
            root = Path(directory)
            descriptor = freeze.freeze_request(root, request, lambda _: response)
            manifest = {"schema_version": 1, "snapshots": [descriptor]}
            labels = person.build_k4(membership, truth.load_snapshots(root, manifest), AT)
            def unavailable(_):
                self.fail("An offline rebuild must not call a provider")
            self.assertEqual(descriptor, freeze.freeze_request(root, request, unavailable))
            self.assertEqual(labels, person.build_k4(membership, truth.load_snapshots(root, manifest), AT))
            body_path = root / "responses" / (response["body_sha256"] + ".json")
            body_path.write_text("{}")
            with self.assertRaisesRegex(truth.TruthError, "payload fingerprint"):
                truth.load_snapshots(root, manifest)

    def should_stop_without_truncating_when_work_or_slot_bounds_are_exceeded(self):
        with patch.object(person, "MAX_WORKS", 1), self.assertRaisesRegex(truth.TruthError, "work bound"):
            person.universe(selection=selection(["10.1234/a", "10.1234/b"]))
        with patch.object(person, "MAX_SLOTS", 1), self.assertRaisesRegex(truth.TruthError, "slot bound"):
            build([[authorship(), authorship(raw=BETA, profile=BETA)]])

    def should_write_labels_and_safe_features_when_cli_inputs_have_verified_frozen_provenance(self):
        source = snapshot("openalex", {"doi": "10.1234/a", "authorships": [authorship()]})
        response = {**source["receipt"], "body": truth.canonical(source["payload"]), "cache_hit": False,
                    "wall_seconds": 0.01, "model_calls": 0, "model_cost_usd": 0, "provider_cost_usd": None}
        with tempfile.TemporaryDirectory(prefix="person-cli-fixture-") as directory:
            root = Path(directory)
            descriptor = freeze.freeze_request(root, {"provider": "openalex", "dois": ["10.1234/a"]}, lambda _: response)
            membership = root / "universe.json"
            manifest = root / "manifest.json"
            output = root / "labels.json"
            projected = root / "features.json"
            membership.write_text(truth.canonical(person.universe(selection=selection(["10.1234/a"]))))
            manifest.write_text(truth.canonical({"schema_version": 1, "snapshots": [descriptor]}))
            command = ["build", "--universe", str(membership), "--frozen-root", str(root), "--manifest", str(manifest),
                       "--built-at", AT, "--output", str(output)]
            with patch("builtins.print"):
                self.assertEqual(0, person.main(command))
                self.assertEqual(0, person.main(command))
                original = output.read_bytes()
                self.assertEqual(0, person.main(["features", "--k4", str(output), "--output", str(projected)]))
                self.assertNotIn(ALPHA, projected.read_text())
                source_path = root / "responses" / (response["body_sha256"] + ".json")
                source_path.write_text("{}")
                self.assertEqual(1, person.main(command))
                self.assertEqual(original, output.read_bytes())
