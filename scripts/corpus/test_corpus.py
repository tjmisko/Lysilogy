"""Offline contract tests; generated payloads are tiny and never real papers."""

import base64
import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import types
import traceback
import unittest
from unittest.mock import patch

import corpus


class Response(io.BytesIO):
    def __init__(self, data, length=None):
        super().__init__(data)
        self.headers = {"Content-Length": str(len(data) if length is None else length)}


class FakeHttp:
    def __init__(self, payloads):
        self.payloads = iter(payloads)
        self.calls = []

    @contextlib.contextmanager
    def open(self, url):
        self.calls.append(url)
        payload = next(self.payloads)
        with payload if isinstance(payload, Response) else Response(payload) as response:
            yield response

    def bytes(self, url):
        self.calls.append(url)
        return next(self.payloads)


def oai(records="", token="", error="", date="2026-09-12T00:00:00Z"):
    return f'''<OAI-PMH xmlns="http://www.openarchives.org/OAI/2.0/">
      <responseDate>{date}</responseDate>{error}<ListRecords>{records}
      <resumptionToken>{token}</resumptionToken></ListRecords></OAI-PMH>'''.encode()


def record(identifier="2001.00001", categories="cs.LG stat.ML", deleted=False):
    if deleted:
        return f'<record><header status="deleted"><identifier>oai:arXiv.org:{identifier}</identifier></header></record>'
    return f'''<record><header><identifier>oai:arXiv.org:{identifier}</identifier>
      <datestamp>2026-09-10</datestamp></header><metadata>
      <arXiv xmlns="http://arxiv.org/OAI/arXiv/"><id>{identifier}</id><created>2020-01-02</created>
      <title>A fixture</title><categories>{categories}</categories><license>http://arxiv.org/licenses/nonexclusive-distrib/1.0/</license>
      <authors><author><keyname>Example</keyname><forenames>A.</forenames></author></authors>
      </arXiv></metadata></record>'''


def small_config():
    config = corpus.load_config(Path(__file__).with_name("selection.json"))
    config["sets"] = {"cs.LG": "cs:cs:LG", "math.PR": "math:math:PR"}
    config["tiers"] = {tier: {"count": count, "years": [2020, 2021], "categories": ["cs.LG", "math.PR"]}
                       for tier, count in [("eval", 4), ("scale", 8)]}
    config["free_space_floor_gib"] = 0.000001
    config["estimated_pdf_mib"] = 0.0001
    config["estimated_source_mib"] = 0.0001
    return config


def papers():
    return [{"id": f"{year % 100:02}01.{index:05}", "created": f"{year}-01-02", "categories": [category]}
            for year in (2020, 2021) for category, indices in [("cs.LG", range(1, 7)), ("math.PR", range(7, 13))]
            for index in indices]


def remote(identifier, **changes):
    payload = b"%PDF-1.7 fixture"
    return {"version": 2, "name": f"arxiv/arxiv/pdf/{identifier[:4]}/{identifier}v2.pdf",
            "bytes": len(payload), "md5": base64.b64encode(hashlib.md5(payload, usedforsecurity=False).digest()).decode(),
            "generation": "123", **changes}


class CorpusTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="lysilogy-corpus-unit-")
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)

    def seed_metadata(self):
        config = small_config()
        with contextlib.closing(corpus.connect(self.root)) as db, db:
            for paper in papers():
                db.execute("INSERT INTO papers VALUES (?, ?)", (paper["id"], corpus.canonical(paper)))
            for set_spec in config["sets"].values():
                db.execute("INSERT INTO harvests VALUES (?, ?)", (set_spec, '{"complete":true}'))
        return config

    def seed_inventories(self, missing=(), overrides=None):
        for month in ("2001", "2101"):
            objects = {p["id"]: remote(p["id"]) for p in papers() if p["id"][:4] == month and p["id"] not in missing}
            objects.update({key: value for key, value in (overrides or {}).items() if key[:4] == month})
            corpus.atomic_json(self.root / "inventory" / (month + ".json"), {"complete": True, "token": "", "objects": objects})

    def legacy_selection(self):
        config = self.seed_metadata()
        chosen = corpus.select_papers(papers(), config)
        selection = {"schema_version": 1, "config_sha256": corpus.fingerprint(config),
                     "metadata_sha256": corpus.fingerprint(papers()), "selection_sha256": corpus.fingerprint(chosen),
                     "papers": chosen}
        # Metadata SELECT orders IDs, which is also the canonical hash order.
        selection["metadata_sha256"] = corpus.fingerprint(sorted(papers(), key=lambda p: p["id"]))
        corpus.atomic_json(self.root / "selection.json", selection)
        self.seed_inventories(missing=[chosen[0]["id"]])
        return config, selection

    def should_fill_identical_stratum_quotas_when_ranked_public_pdfs_are_missing(self):
        config = self.seed_metadata()
        original = corpus.select_papers(papers(), config)
        missing = [original[0]["id"], original[-1]["id"]]
        self.seed_inventories(missing)
        chosen = corpus.select(self.root, config, FakeHttp([]))
        self.assertEqual(2, chosen["schema_version"])
        self.assertTrue(set(missing).isdisjoint(p["id"] for p in chosen["papers"]))
        expected = corpus.select_papers([p for p in papers() if p["id"] not in missing], config)
        self.assertEqual([(p["id"], p["tiers"], p["strata"]) for p in expected],
                         [(p["id"], p["tiers"], p["strata"]) for p in chosen["papers"]])
        self.assertEqual(4, sum("eval" in p["tiers"] for p in chosen["papers"]))
        self.assertEqual(8, sum("scale" in p["tiers"] for p in chosen["papers"]))
        self.assertTrue(any(p["tiers"] == ["eval", "scale"] for p in chosen["papers"]))
        self.assertTrue(set(missing) <= {p["id"] for p in chosen["availability"]["exclusions"]})
        corpus.validate_availability(self.root, chosen)

    def should_exclude_invalid_latest_objects_when_inventory_fields_are_unusable(self):
        identifier = "2001.00001"
        cases = [(None, "missing_public_pdf"), ({}, "invalid_object_identity"),
                 (remote(identifier, name="arxiv/arxiv/pdf/2101/2001.00001v2.pdf"), "invalid_object_identity"),
                 (remote(identifier, version=3), "invalid_object_identity"),
                 (remote(identifier, generation="0"), "invalid_object_generation"),
                 (remote(identifier, generation="1?bad"), "invalid_object_generation"),
                 (remote(identifier, bytes=0), "invalid_or_oversized_object_bytes"),
                 (remote(identifier, bytes=True), "invalid_or_oversized_object_bytes"),
                 (remote(identifier, bytes=101), "invalid_or_oversized_object_bytes"),
                 (remote(identifier, md5="invalid"), "invalid_object_md5")]
        for value, expected in cases:
            with self.subTest(value=value):
                self.assertEqual(expected, corpus.remote_problem(value, identifier, 100))
        objects = corpus.gcs_objects({"items": [
            {"name": "arxiv/arxiv/pdf/2001/2001.00001v2.pdf", "size": "bad", "generation": "123"},
            {"name": "arxiv/arxiv/pdf/2001/2001.00001v1.pdf", "size": "10", "generation": "122", "md5Hash": remote(identifier)["md5"]}]}, "2001")
        self.assertEqual(2, objects[identifier]["version"])
        self.assertEqual("invalid_or_oversized_object_bytes", corpus.remote_problem(objects[identifier], identifier, 100))

    def should_record_invalid_object_evidence_when_replacements_keep_full_quotas(self):
        config = self.seed_metadata()
        identifier = corpus.select_papers(papers(), config)[0]["id"]
        self.seed_inventories(overrides={identifier: remote(identifier, md5=None)})
        chosen = corpus.select(self.root, config, FakeHttp([]))
        row = next(p for p in chosen["availability"]["exclusions"] if p["id"] == identifier)
        self.assertEqual("invalid_object_md5", row["reason"])
        self.assertIsNone(row["remote_pdf"]["md5"])
        self.assertTrue(all(p["id"] != identifier for p in chosen["papers"]))

    def should_keep_selection_unpublished_when_available_stratum_capacity_is_insufficient(self):
        config = self.seed_metadata()
        self.seed_inventories(missing=[p["id"] for p in papers() if p["created"].startswith("2020") and p["categories"] == ["cs.LG"]])
        with self.assertRaisesRegex(corpus.CorpusError, "Insufficient available PDFs.*0 available, 1 needed"):
            corpus.select(self.root, config, FakeHttp([]))
        self.assertFalse((self.root / "selection.json").exists())
        evidence = list((self.root / "availability").glob("failed-*.json"))
        self.assertEqual(1, len(evidence))
        self.assertEqual(6, len(corpus.read_json(evidence[0])["exclusions"]))

    def should_preserve_observed_created_year_when_identifier_predates_the_stratum(self):
        config = self.seed_metadata()
        with contextlib.closing(corpus.connect(self.root)) as db, db:
            row = {"id": "1801.00600", "created": "2020-06-30", "categories": ["cs.LG"]}
            db.execute("INSERT INTO papers VALUES (?, ?)", (row["id"], corpus.canonical(row)))
        self.seed_inventories()
        corpus.atomic_json(self.root / "inventory/1801.json", {"complete": True, "token": "", "objects": {}})
        availability = corpus.Availability(self.root, config, FakeHttp([]), "fixture", self.root / "availability/inputs.json")
        self.assertIsNone(availability.qualify(row))
        self.assertEqual("2020-06-30", row["created"])
        self.assertEqual("missing_public_pdf", availability.exclusions[row["id"]]["reason"])

    def should_reuse_preserved_inventory_when_selection_preparation_is_interrupted(self):
        config = self.seed_metadata()
        self.seed_inventories()
        original = corpus.Availability.qualify
        visited = []
        def interrupt(instance, paper):
            qualified = original(instance, paper)
            visited.append(paper["id"])
            if len(visited) == 2:
                raise KeyboardInterrupt()
            return qualified
        with patch.object(corpus.Availability, "qualify", interrupt), self.assertRaises(KeyboardInterrupt):
            corpus.select(self.root, config, FakeHttp([]))
        checkpoint = corpus.read_json(self.root / "availability/selection-inputs.json")
        for month in checkpoint["inventories"]:
            corpus.atomic_json(self.root / "inventory" / (month + ".json"), {"complete": True, "token": "", "objects": {}})
        chosen = corpus.select(self.root, config, FakeHttp([]))
        self.assertEqual(8, sum("scale" in p["tiers"] for p in chosen["papers"]))
        self.assertTrue(set(visited) <= {p["id"] for p in chosen["papers"]})

    def should_archive_exact_original_when_explicit_zero_artifact_recovery_succeeds(self):
        config, legacy = self.legacy_selection()
        original_bytes = (self.root / "selection.json").read_bytes()
        recovered = corpus.recover_selection(self.root, config, FakeHttp([]), "Selected PDF unavailable in public bucket")
        archive = self.root / "selection-recovery" / hashlib.sha256(original_bytes).hexdigest()
        self.assertEqual(original_bytes, (archive / "original-selection.json").read_bytes())
        self.assertNotEqual(legacy["selection_sha256"], recovered["selection_sha256"])
        self.assertEqual(recovered, corpus.recover_selection(self.root, config, FakeHttp([]), "Selected PDF unavailable in public bucket"))
        self.assertEqual(recovered, corpus.select(self.root, config, FakeHttp([])))
        with self.assertRaisesRegex(corpus.CorpusError, "reason differs"):
            corpus.recover_selection(self.root, config, FakeHttp([]), "changed reason")

    def should_resume_staged_recovery_when_publication_is_interrupted(self):
        config, _ = self.legacy_selection()
        old = (self.root / "selection.json").read_bytes()
        atomic = corpus.atomic_json
        def interrupt(path, value):
            if path == self.root / "selection.json":
                raise KeyboardInterrupt()
            return atomic(path, value)
        with patch.object(corpus, "atomic_json", interrupt), self.assertRaises(KeyboardInterrupt):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        self.assertEqual(old, (self.root / "selection.json").read_bytes())
        with patch.object(corpus, "prepare_selection", side_effect=AssertionError("must reuse staged replacement")):
            recovered = corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        self.assertEqual(2, recovered["schema_version"])

    def should_leave_failed_original_intact_when_recovery_capacity_is_insufficient(self):
        config, _ = self.legacy_selection()
        original = (self.root / "selection.json").read_bytes()
        self.seed_inventories(missing=[p["id"] for p in papers()])
        with self.assertRaisesRegex(corpus.CorpusError, "Insufficient available PDFs"):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDFs")
        self.assertEqual(original, (self.root / "selection.json").read_bytes())
        self.assertEqual(original, next((self.root / "selection-recovery").glob("*/original-selection.json")).read_bytes())

    def should_refuse_recovery_before_network_when_any_manifest_or_artifact_is_admitted(self):
        config, _ = self.legacy_selection()
        original = (self.root / "selection.json").read_bytes()
        for name, payload in [("manifest.jsonl", "{}\n"), ("manifest.jsonl.partial", ""),
                              ("pdf/paper.pdf", "%PDF-test"), ("pdf/paper.pdf.partial", "partial"),
                              ("source/paper.src.verified.json", "{}"), ("source/paper.src", "source")]:
            with self.subTest(name=name):
                path = self.root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(payload)
                with self.assertRaises(corpus.CorpusError):
                    corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
                self.assertEqual(payload, path.read_text())
                path.unlink()
        self.assertEqual(original, (self.root / "selection.json").read_bytes())
        self.assertFalse((self.root / "selection-recovery").exists())

    def should_refuse_recovery_when_original_metadata_or_archive_was_changed(self):
        config, _ = self.legacy_selection()
        original = (self.root / "selection.json").read_bytes()
        archive = self.root / "selection-recovery" / hashlib.sha256(original).hexdigest()
        corpus.immutable_text(archive / "original-selection.json", "tampered\n")
        with self.assertRaisesRegex(corpus.CorpusError, "archive differs"):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        self.assertEqual("tampered\n", (archive / "original-selection.json").read_text())
        (archive / "original-selection.json").unlink()
        with contextlib.closing(corpus.connect(self.root)) as db, db:
            db.execute("DELETE FROM papers WHERE id='2001.00001'")
        with self.assertRaisesRegex(corpus.CorpusError, "Metadata changed"):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        self.assertEqual(original, (self.root / "selection.json").read_bytes())

    def should_reject_corrupt_inventory_snapshot_when_frozen_selection_is_downloaded(self):
        config = self.seed_metadata()
        self.seed_inventories()
        chosen = corpus.select(self.root, config, FakeHttp([]))
        proof = next(iter(chosen["availability"]["inventories"].values()))
        (self.root / proof["path"]).write_text("{}")
        with self.assertRaisesRegex(corpus.CorpusError, "snapshot fingerprint"):
            corpus.download(self.root, config, FakeHttp([]), limit=1)

    def should_use_frozen_object_without_listing_when_inventory_changes_after_selection(self):
        config = self.seed_metadata()
        self.seed_inventories()
        chosen = corpus.select(self.root, config, FakeHttp([]))
        self.seed_inventories(overrides={p["id"]: remote(p["id"], version=3, generation="456") for p in papers()})
        client = FakeHttp([b"%PDF-1.7 fixture", b"\\documentclass{article}"])
        corpus.download(self.root, config, client, tier="eval", limit=1)
        self.assertEqual(2, len(client.calls))
        self.assertIn("v2.pdf?generation=123", client.calls[0])
        self.assertTrue(client.calls[1].endswith("v2"))
        entry = next(iter(corpus.read_manifest(self.root).values()))
        paper = next(p for p in chosen["papers"] if p["id"] == entry["id"])
        entry["remote_pdf"]["generation"] = "456"
        with self.assertRaisesRegex(corpus.CorpusError, "availability-pinned"):
            corpus.validate_entry(entry, paper, chosen["selection_sha256"])

    def should_refuse_preparation_before_inventory_when_disk_estimate_breaks_floor(self):
        config = self.seed_metadata()
        with patch.object(corpus, "check_space", side_effect=corpus.CorpusError("Insufficient disk space")):
            with self.assertRaisesRegex(corpus.CorpusError, "Insufficient disk space"):
                corpus.select(self.root, config, FakeHttp([]))
        self.assertFalse((self.root / "selection.json").exists())
        self.assertFalse((self.root / "inventory").exists())

    def should_hash_identical_canonical_metadata_when_streaming_its_fingerprint(self):
        for records in ([], papers(), [{"unicode": "Σø", "lines": "a\nb"}]):
            self.assertEqual(corpus.fingerprint(records), corpus.fingerprint_records(records))

    def should_preserve_original_newlines_when_archiving_a_legacy_selection(self):
        config, legacy = self.legacy_selection()
        original = (json.dumps(legacy, indent=2).replace("\n", "\r\n") + "\r\n").encode()
        (self.root / "selection.json").write_bytes(original)
        corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        archive = self.root / "selection-recovery" / hashlib.sha256(original).hexdigest()
        self.assertEqual(original, (archive / "original-selection.json").read_bytes())

    def should_resume_without_reselection_when_crash_follows_final_publication(self):
        config, _ = self.legacy_selection()
        atomic = corpus.atomic_json
        def interrupt(path, value):
            atomic(path, value)
            if path == self.root / "selection.json":
                raise KeyboardInterrupt()
        with patch.object(corpus, "atomic_json", interrupt), self.assertRaises(KeyboardInterrupt):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        original = (self.root / "selection.json").read_bytes()
        with patch.object(corpus, "prepare_selection", side_effect=AssertionError("must not reselect")):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        self.assertEqual(original, (self.root / "selection.json").read_bytes())
        (self.root / "source").mkdir()
        (self.root / "source/paper.src.partial").write_bytes(b"")
        with self.assertRaisesRegex(corpus.CorpusError, "partial downloads"):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")

    def should_reproduce_selection_bytes_when_metadata_and_inventory_snapshots_match(self):
        config = self.seed_metadata()
        self.seed_inventories(missing=["2001.00001"])
        first = corpus.select(self.root, config, FakeHttp([]))
        root = self.root
        with tempfile.TemporaryDirectory(prefix="lysilogy-corpus-unit-") as other:
            self.root = Path(other)
            try:
                self.seed_metadata()
                self.seed_inventories(missing=["2001.00001"])
                second = corpus.select(self.root, config, FakeHttp([]))
                self.assertEqual(first, second)
                self.assertEqual((root / "selection.json").read_bytes(), (self.root / "selection.json").read_bytes())
            finally:
                self.root = root

    def should_reject_rehashed_version_or_quota_changes_when_selection_is_corrupted(self):
        config = self.seed_metadata()
        self.seed_inventories()
        selected = corpus.select(self.root, config, FakeHttp([]))
        modified = copy.deepcopy(selected)
        modified["papers"][0]["remote_pdf"]["generation"] = "456"
        modified["selection_sha256"] = corpus.fingerprint(modified["papers"])
        corpus.validate_selection(modified, config)
        with self.assertRaisesRegex(corpus.CorpusError, "preserved inventory"):
            corpus.validate_availability(self.root, modified)
        modified = copy.deepcopy(selected)
        modified["papers"].pop()
        modified["selection_sha256"] = corpus.fingerprint(modified["papers"])
        with self.assertRaisesRegex(corpus.CorpusError, "exact stratum quotas"):
            corpus.validate_selection(modified, config)

    def should_leave_other_files_untouched_when_recovery_archive_contains_a_symlink(self):
        config, _ = self.legacy_selection()
        raw = (self.root / "selection.json").read_bytes()
        archive = self.root / "selection-recovery" / hashlib.sha256(raw).hexdigest()
        archive.mkdir(parents=True)
        outside = self.root / "fixture-protected.json"
        outside.write_text("protected")
        (archive / "original-selection.json").symlink_to(outside)
        with self.assertRaisesRegex(corpus.CorpusError, "symlinks"):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        self.assertEqual("protected", outside.read_text())
        self.assertEqual(raw, (self.root / "selection.json").read_bytes())

    def should_reject_changed_archive_when_recovered_selection_is_verified(self):
        config, _ = self.legacy_selection()
        selected = corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")
        archive = self.root / "selection-recovery" / selected["recovery"]["original_file_sha256"]
        (archive / "request.json").write_text("{}")
        with self.assertRaisesRegex(corpus.CorpusError, "archive differs"):
            corpus.validate_availability(self.root, selected)

    def should_route_explicit_recovery_when_cli_selects_a_proxy_and_failure_reason(self):
        config = small_config()
        client = FakeHttp([])
        with patch.object(corpus, "validate_root", return_value=self.root), \
                patch.object(corpus, "load_config", return_value=config), \
                patch.object(corpus, "Http", return_value=client), \
                patch.object(corpus, "recover_selection") as recovery:
            self.assertEqual(0, corpus.main(["--proxy-env", "HTTPS_PROXY", "recover-selection", "--reason", "missing PDF"]))
        recovery.assert_called_once_with(self.root, config, client, "missing PDF")

    def should_refuse_recovery_when_rehashed_legacy_rows_do_not_match_metadata(self):
        config, legacy = self.legacy_selection()
        legacy["papers"][0]["created"] = "2020-09-13"
        legacy["selection_sha256"] = corpus.fingerprint(legacy["papers"])
        corpus.atomic_json(self.root / "selection.json", legacy)
        with self.assertRaisesRegex(corpus.CorpusError, "recorded metadata and config"):
            corpus.recover_selection(self.root, config, FakeHttp([]), "missing PDF")

    def should_select_identical_papers_when_seed_config_and_metadata_match(self):
        config = small_config()
        first = corpus.select_papers(papers(), config)
        self.assertEqual(first, corpus.select_papers(list(reversed(papers())) + papers(), config))
        self.assertEqual(4, sum("eval" in p["tiers"] for p in first))
        self.assertEqual(8, sum("scale" in p["tiers"] for p in first))
        altered = copy.deepcopy(config)
        altered["seed"] = "other-seed"
        self.assertNotEqual(first, corpus.select_papers(papers(), altered))
        for tier, expected in [("eval", 1), ("scale", 2)]:
            for category in ("cs.LG", "math.PR"):
                for year in (2020, 2021):
                    self.assertEqual(expected, sum(p["strata"].get(tier) == [category, year] for p in first))

    def should_use_primary_category_without_duplicates_when_papers_are_cross_listed(self):
        records = papers()
        for paper in records:
            paper["categories"].append("math.PR" if paper["categories"][0] == "cs.LG" else "cs.LG")
        chosen = corpus.select_papers(records, small_config())
        self.assertEqual(8, sum("scale" in p["tiers"] for p in chosen))
        self.assertEqual(len(chosen), len({p["id"] for p in chosen}))
        for paper in chosen:
            for tier in paper["tiers"]:
                self.assertEqual(paper["categories"][0], paper["strata"][tier][0])

    def should_honor_cooldown_and_spacing_when_production_http_retries_429(self):
        current = [1000.0]
        starts = []
        def sleep(delay):
            current[0] += delay
        class Opener:
            def open(self, request, timeout):
                starts.append(current[0])
                if len(starts) == 1:
                    raise corpus.urllib.error.HTTPError(request.full_url, 429, "rate limited", {"Retry-After": "10"}, None)
                return Response(b"fixture")
        client = corpus.Http(cache_root=self.root, sleep=sleep, clock=lambda: current[0])
        client.opener = Opener()
        client.bytes(corpus.EXPORT + "2001.00001v1")
        client.bytes(corpus.OAI)
        self.assertEqual([1000, 1010, 1013], starts)

    def should_ignore_ambient_proxies_when_no_transport_option_is_selected(self):
        with patch.dict(os.environ, {"HTTPS_PROXY": "http://unused:secret@proxy.invalid:8080"}), \
                patch.object(corpus.urllib.request, "build_opener") as build:
            corpus.Http(cache_root=self.root)
        self.assertEqual({}, build.call_args.args[0].proxies)
        self.assertIsInstance(build.call_args.args[1], corpus.NoRedirect)

    def should_use_only_selected_https_proxy_when_transport_is_explicitly_enabled(self):
        selected = "http://fixture-user:fixture-password@proxy.invalid:8080"
        with patch.dict(os.environ, {"HTTPS_PROXY": "http://unselected.invalid", "https_proxy": selected,
                                     "HTTP_PROXY": "http://also-unselected.invalid"}), \
                patch.object(corpus.urllib.request, "build_opener") as build:
            corpus.Http(cache_root=self.root, proxy_env="https_proxy")
        self.assertEqual({"https": selected}, build.call_args.args[0].proxies)
        self.assertIsInstance(build.call_args.args[1], corpus.NoRedirect)

    def should_reject_proxy_configuration_without_secrets_when_selected_value_is_invalid(self):
        values = ["", "proxy.invalid", "ftp://fixture-secret@proxy.invalid",
                  "http://fixture-secret@proxy.invalid:invalid", "http://fixture-secret@proxy.invalid:70000",
                  "http://fixture-secret@proxy.invalid:0", "http://fixture-secret@proxy.invalid/path",
                  "http://proxy.invalid?fixture-secret", "http://proxy.invalid#fixture-secret",
                  "http://proxy.invalid/\nfixture-secret", "http://[fixture-secret"]
        for value in values:
            with self.subTest(value=value), patch.dict(os.environ, {"HTTPS_PROXY": value}):
                with self.assertRaises(corpus.CorpusError) as failure:
                    corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY")
                self.assertNotIn("fixture-secret", str(failure.exception))
        with patch.dict(os.environ, {}, clear=True), self.assertRaises(corpus.CorpusError):
            corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY")
        with self.assertRaises(corpus.CorpusError):
            corpus.Http(cache_root=self.root, proxy_env="UNRELATED_SECRET")

    def should_reject_other_destinations_when_proxy_transport_is_enabled(self):
        with patch.dict(os.environ, {"HTTPS_PROXY": "http://proxy.invalid:8080"}):
            client = corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY")
        with patch.object(client.opener, "open") as send:
            for url in ("https://example.invalid/paper.pdf", "http://oaipmh.arxiv.org/oai",
                        "https://user:password@oaipmh.arxiv.org/oai"):
                with self.assertRaises(corpus.CorpusError):
                    client.bytes(url)
            send.assert_not_called()

    def should_reject_tls_proxy_scheme_when_the_standard_transport_cannot_preserve_it(self):
        with patch.dict(os.environ, {"HTTPS_PROXY": "https://fixture-user:fixture-secret@proxy.invalid:8443"}), \
                patch.object(corpus.urllib.request, "build_opener") as build:
            with self.assertRaisesRegex(corpus.CorpusError, "HTTPS-scheme proxies are unsupported") as failure:
                corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY")
        build.assert_not_called()
        self.assertNotIn("fixture-secret", "".join(traceback.format_exception(failure.exception)))

    def should_redact_transport_failures_when_configured_proxy_has_credentials(self):
        secret = "fixture-user:fixture-password"
        errors = [corpus.urllib.error.URLError(secret),
                  corpus.urllib.error.HTTPError("http://" + secret + "@proxy.invalid", 407, secret, {}, None),
                  corpus.http.client.InvalidURL(secret)]
        for error in errors:
            current = [1000.0]
            def sleep(delay):
                current[0] += delay
            with patch.dict(os.environ, {"HTTPS_PROXY": "http://" + secret + "@proxy.invalid:8080"}):
                client = corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY", sleep=sleep,
                                     clock=lambda: current[0])
            with patch.object(client.opener, "open", side_effect=error):
                with self.assertRaises(corpus.CorpusError) as failure:
                    client.bytes(corpus.OAI)
            rendered = "".join(traceback.format_exception(failure.exception))
            self.assertNotIn("fixture-user", rendered)
            self.assertNotIn("fixture-password", rendered)
            self.assertIn("Configured HTTPS proxy request failed", rendered)
            if isinstance(error, corpus.urllib.error.HTTPError):
                self.assertIn("HTTP 407", rendered)

    def should_share_spacing_and_retry_cooldown_when_proxy_and_direct_clients_alternate(self):
        current = [1000.0]
        starts = []
        def sleep(delay):
            current[0] += delay
        class Opener:
            def open(self, request, timeout):
                starts.append(current[0])
                if len(starts) == 1:
                    raise corpus.urllib.error.HTTPError(request.full_url, 429, "fixture", {"Retry-After": "10"}, None)
                return Response(b"fixture")
        with patch.dict(os.environ, {"HTTPS_PROXY": "http://proxy.invalid:8080"}):
            proxied = corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY", sleep=sleep,
                                  clock=lambda: current[0])
        proxied.opener = Opener()
        proxied.bytes(corpus.OAI)
        direct = corpus.Http(cache_root=self.root, sleep=sleep, clock=lambda: current[0])
        direct.opener = Opener()
        direct.bytes(corpus.EXPORT + "2001.00001v1")
        self.assertEqual([1000, 1010, 1013], starts)

    def should_keep_credentials_out_of_tracebacks_when_proxy_retry_sleep_is_interrupted(self):
        def interrupt(_delay):
            raise KeyboardInterrupt()
        with patch.dict(os.environ, {"HTTPS_PROXY": "http://fixture-secret@proxy.invalid:8080"}):
            client = corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY", sleep=interrupt,
                                 clock=lambda: 4000.0)
        error = corpus.urllib.error.HTTPError(corpus.OAI, 429, "fixture-secret", {"Retry-After": "600"}, None)
        with patch.object(client.opener, "open", side_effect=error):
            with self.assertRaises(KeyboardInterrupt) as failure:
                client.bytes(corpus.OAI)
        self.assertNotIn("fixture-secret", "".join(traceback.format_exception(failure.exception)))
        self.assertEqual(4600, float((self.root / "arxiv-http.lock").read_text()))

    def should_forward_proxy_choice_when_a_live_command_is_dispatched(self):
        with patch.object(corpus, "validate_root", return_value=self.root), \
                patch.object(corpus, "Http") as factory, patch.object(corpus, "harvest") as harvest:
            self.assertEqual(0, corpus.main(["--proxy-env", "HTTPS_PROXY", "harvest"]))
        factory.assert_called_once_with(proxy_env="HTTPS_PROXY")
        self.assertEqual(factory.return_value, harvest.call_args.args[2])

    def should_suppress_transport_context_when_cooldown_persistence_is_interrupted(self):
        with patch.dict(os.environ, {"HTTPS_PROXY": "http://fixture-secret@proxy.invalid:8080"}):
            client = corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY", clock=lambda: 1000.0)
        persist = corpus.persist_rate_deadline
        calls = []
        def interrupted_persist(stream, deadline):
            calls.append(deadline)
            persist(stream, deadline)
            if len(calls) == 2:
                raise KeyboardInterrupt()
        error = corpus.urllib.error.HTTPError(corpus.OAI, 429, "fixture-secret", {"Retry-After": "600"}, None)
        with patch.object(client.opener, "open", side_effect=error), \
                patch.object(corpus, "persist_rate_deadline", side_effect=interrupted_persist):
            with self.assertRaises(KeyboardInterrupt) as failure:
                client.bytes(corpus.OAI)
        self.assertEqual([1003, 1600], calls)
        self.assertEqual(1600, float((self.root / "arxiv-http.lock").read_text()))
        self.assertTrue(error.closed)
        self.assertNotIn("fixture-secret", "".join(traceback.format_exception(failure.exception)))

    def should_redact_response_lifetime_errors_when_proxy_read_or_cleanup_fails(self):
        class BrokenResponse:
            def __init__(self, fail_read):
                self.fail_read = fail_read
            def __enter__(self):
                return self
            def __exit__(self, *_args):
                if not self.fail_read:
                    raise RuntimeError("fixture-secret")
            def read(self, _count):
                if self.fail_read:
                    raise corpus.urllib.error.URLError("fixture-secret")
                return b"fixture"
        class BrokenClose(corpus.urllib.error.HTTPError):
            def close(self):
                super().close()
                raise RuntimeError("fixture-secret")
        with patch.dict(os.environ, {"HTTPS_PROXY": "http://fixture-secret@proxy.invalid:8080"}):
            client = corpus.Http(cache_root=self.root, proxy_env="HTTPS_PROXY")
        for response in (BrokenResponse(True), BrokenResponse(False)):
            with patch.object(client.opener, "open", return_value=response):
                with self.assertRaises(corpus.CorpusError) as failure:
                    client.bytes(corpus.GCS + "/fixture")
            self.assertNotIn("fixture-secret", "".join(traceback.format_exception(failure.exception)))
        error = BrokenClose(corpus.GCS, 429, "fixture-secret", {}, None)
        with patch.object(client.opener, "open", side_effect=error):
            with self.assertRaises(corpus.CorpusError) as failure:
                client.bytes(corpus.GCS + "/fixture")
        self.assertNotIn("fixture-secret", "".join(traceback.format_exception(failure.exception)))

    def should_reject_directory_symlink_when_pdf_storage_leaves_corpus_root(self):
        real_directory = self.root / "external"
        real_directory.mkdir()
        (self.root / "pdf").symlink_to(real_directory, target_is_directory=True)
        with self.assertRaisesRegex(corpus.CorpusError, "symlinks"):
            corpus.fetch_file(self.root / "pdf/paper.pdf", corpus.GCS + "/test", "pdf", FakeHttp([]), 0, 1024)
        self.assertEqual([], list(real_directory.iterdir()))

    def should_fail_instead_of_reducing_tier_when_a_stratum_is_missing(self):
        with self.assertRaisesRegex(corpus.CorpusError, "Insufficient metadata"):
            corpus.select_papers(papers()[:3], small_config())

    def should_preserve_metadata_and_checkpoint_together_when_a_page_finishes(self):
        config = small_config()
        config["sets"] = {"cs.LG": "cs:cs:LG"}
        client = FakeHttp([oai(record(), "next-page"), oai(record("2001.00002"))])
        corpus.harvest(self.root, config, client)
        self.assertEqual(2, len(client.calls))
        self.assertIn("resumptionToken=next-page", client.calls[1])
        self.assertNotIn("metadataPrefix", client.calls[1])
        self.assertIn("metadataPrefix=arXiv", client.calls[0])
        db = corpus.connect(self.root)
        self.addCleanup(db.close)
        self.assertEqual(2, db.execute("SELECT COUNT(*) FROM papers").fetchone()[0])
        corpus.harvest(self.root, config, FakeHttp([]))

    def should_restart_original_window_when_oai_token_expires(self):
        config = small_config()
        config["sets"] = {"cs.LG": "cs:cs:LG"}
        client = FakeHttp([oai(record(), "expired")])
        with self.assertRaises(StopIteration):
            corpus.harvest(self.root, config, client)
        client = FakeHttp([oai(error='<error code="badResumptionToken">expired</error>'), oai(record())])
        corpus.harvest(self.root, config, client)
        self.assertIn("resumptionToken=expired", client.calls[0])
        self.assertIn("from=2020-01-01", client.calls[1])
        self.assertNotIn("resumptionToken", client.calls[1])

    def should_harvest_incrementally_and_apply_deletions_when_refresh_is_requested(self):
        config = small_config()
        config["sets"] = {"cs.LG": "cs:cs:LG"}
        corpus.harvest(self.root, config, FakeHttp([oai(record())]))
        client = FakeHttp([oai(record(deleted=True))])
        corpus.harvest(self.root, config, client, refresh=True)
        self.assertIn("from=2026-09-12", client.calls[0])
        db = corpus.connect(self.root)
        self.addCleanup(db.close)
        self.assertEqual(0, db.execute("SELECT COUNT(*) FROM papers").fetchone()[0])

    def should_reject_entity_expansion_when_metadata_has_xml_declarations(self):
        with self.assertRaisesRegex(corpus.CorpusError, "entity declaration"):
            corpus.parse_oai(b'<!DOCTYPE x [<!ENTITY e "unsafe">]>' + oai())

    def should_parse_provenance_when_arxiv_metadata_arrives(self):
        rows, _, _ = corpus.parse_oai(oai(record()))
        self.assertEqual(["cs.LG", "stat.ML"], rows[0]["categories"])
        self.assertEqual("Example", rows[0]["authors"][0]["keyname"])
        self.assertEqual("https://arxiv.org/abs/2001.00001", rows[0]["arxiv_url"])
        self.assertIn("nonexclusive", rows[0]["license"])

    def should_preserve_frozen_selection_when_local_metadata_later_changes(self):
        config = small_config()
        db = corpus.connect(self.root)
        with db:
            for paper in papers():
                db.execute("INSERT INTO papers VALUES (?, ?)", (paper["id"], corpus.canonical(paper)))
            for set_spec in config["sets"].values():
                db.execute("INSERT INTO harvests VALUES (?, ?)", (set_spec, '{"complete":true}'))
        self.seed_inventories()
        first = corpus.select(self.root, config, FakeHttp([]))
        with db:
            db.execute("DELETE FROM papers")
        db.close()
        self.assertEqual(first, corpus.select(self.root, config, FakeHttp([])))
        config["seed"] = "new"
        with self.assertRaisesRegex(corpus.CorpusError, "different config"):
            corpus.select(self.root, config, FakeHttp([]))

    def should_resume_without_redownloading_when_verified_file_exists(self):
        payload = b"%PDF-1.7\nsynthetic test bytes\n%%EOF"
        client = FakeHttp([payload])
        path = self.root / "paper.pdf"
        receipt, fetched = corpus.fetch_file(path, "https://storage.googleapis.com/test", "pdf", client, 0, 1024)
        self.assertTrue(fetched)
        again, fetched = corpus.fetch_file(path, "https://storage.googleapis.com/test", "pdf", client, 0, 1024)
        self.assertFalse(fetched)
        self.assertEqual(receipt, again)
        self.assertEqual(1, len(client.calls))

    def should_discard_partial_download_when_remote_hash_mismatches(self):
        path = self.root / "paper.pdf"
        expected = {"md5": "wrong", "bytes": 8}
        with self.assertRaisesRegex(corpus.CorpusError, "hash or size"):
            corpus.fetch_file(path, "https://storage.googleapis.com/test", "pdf", FakeHttp([b"%PDF-bad"]), 0, 1024, expected)
        self.assertFalse(path.exists())
        self.assertFalse(path.with_name("paper.pdf.partial").exists())
        self.assertFalse(path.with_name("paper.pdf.verified.json").exists())

    def should_discard_partial_download_when_stream_ends_early(self):
        path = self.root / "paper.pdf"
        with self.assertRaisesRegex(corpus.CorpusError, "Incomplete"):
            corpus.fetch_file(path, "https://storage.googleapis.com/test", "pdf", FakeHttp([Response(b"%PDF-x", 90)]), 0, 1024)
        self.assertFalse(path.exists())
        self.assertFalse(path.with_name("paper.pdf.partial").exists())

    def should_keep_existing_bytes_when_a_completed_file_is_corrupt(self):
        path = self.root / "paper.pdf"
        client = FakeHttp([b"%PDF-original"])
        corpus.fetch_file(path, "https://storage.googleapis.com/test", "pdf", client, 0, 1024)
        path.write_bytes(b"changed locally")
        with self.assertRaisesRegex(corpus.CorpusError, "leaving it untouched"):
            corpus.fetch_file(path, "https://storage.googleapis.com/test", "pdf", client, 0, 1024)
        self.assertEqual(b"changed locally", path.read_bytes())
        self.assertEqual(1, len(client.calls))

    def should_resume_after_manifest_crash_when_a_source_receipt_exists(self):
        path = self.root / "paper.src"
        client = FakeHttp([b"\\documentclass{article}"])
        receipt, _ = corpus.fetch_file(path, "https://export.arxiv.org/e-print/2001.00001v2", "source", client, 0, 1024)
        self.assertEqual("tex", receipt["source_format"])
        # No manifest has been written; independently durable receipt is enough.
        _, fetched = corpus.fetch_file(path, "https://export.arxiv.org/e-print/2001.00001v2", "source", client, 0, 1024)
        self.assertFalse(fetched)

    def should_discard_partial_download_when_response_is_html(self):
        path = self.root / "paper.src"
        with self.assertRaisesRegex(corpus.CorpusError, "source format"):
            corpus.fetch_file(path, "https://export.arxiv.org/e-print/2001.00001v1", "source",
                              FakeHttp([b"<html>error \\trace</html>"]), 0, 1024)
        self.assertFalse(path.exists())

    def should_pace_eprint_requests_when_several_requests_are_queued(self):
        current = [0.0]
        def sleep(delay):
            current[0] += delay
        pacer = corpus.Pacer(interval=0.1, clock=lambda: current[0], sleep=sleep)
        starts = []
        for _ in range(12):
            pacer.wait()
            starts.append(current[0])
        self.assertEqual(list(range(0, 36, 3)), starts)

    def should_parse_server_cooldown_when_retry_after_is_seconds_or_http_date(self):
        self.assertEqual(10, corpus.retry_after("10"))
        self.assertEqual(10, corpus.retry_after("Thu, 01 Jan 1970 00:00:10 GMT", timestamp=0))
        self.assertEqual(0, corpus.retry_after("bad value"))

    def should_refuse_to_start_when_projected_usage_exceeds_free_space_floor(self):
        with self.assertRaisesRegex(corpus.CorpusError, "Insufficient disk space"):
            corpus.check_space(self.root, 400, 700, lambda _: types.SimpleNamespace(free=1000))
        result = corpus.check_space(self.root, 300, 700, lambda _: types.SimpleNamespace(free=1000))
        self.assertEqual(300, result["projected_additional_bytes"])

    def should_stop_and_remove_partial_when_free_space_falls_during_download(self):
        path = self.root / "paper.pdf"
        with patch.object(corpus, "check_space", side_effect=corpus.CorpusError("Insufficient disk space")):
            with self.assertRaisesRegex(corpus.CorpusError, "Insufficient disk space"):
                corpus.fetch_file(path, "https://storage.googleapis.com/test", "pdf", FakeHttp([b"%PDF-test"]), 1, 1024)
        self.assertFalse(path.exists())
        self.assertFalse(path.with_name("paper.pdf.partial").exists())

    def should_pin_highest_version_when_bucket_objects_are_unsorted(self):
        objects = [{"name": f"arxiv/arxiv/pdf/2001/2001.00001v{v}.pdf", "size": "9", "md5Hash": "hash", "generation": str(v)} for v in (3, 1, 2)]
        objects.append({"name": "../../outside", "size": "9"})
        found = corpus.gcs_objects({"items": objects}, "2001")
        self.assertEqual(3, found["2001.00001"]["version"])
        self.assertEqual(1, len(found))

    def should_resume_listing_when_bucket_inventory_has_a_saved_token(self):
        corpus.atomic_json(self.root / "inventory/2001.json", {"complete": False, "token": "resume", "objects": {}})
        client = FakeHttp([json.dumps({"items": [{"name": "arxiv/arxiv/pdf/2001/2001.00001v1.pdf", "size": "9", "md5Hash": "hash", "generation": "1"}]}).encode()])
        first = corpus.inventory(self.root, "2001", client)
        self.assertIn("pageToken=resume", client.calls[0])
        self.assertEqual(first, corpus.inventory(self.root, "2001", FakeHttp([])))

    def should_reject_unsafe_root_when_it_is_in_repo_tmp_library_or_data(self):
        repo = Path(__file__).resolve().parents[2]
        for root in (repo / "corpus", Path("/tmp/corpus"), Path.home() / "local-articles/corpus", Path.home() / "example/.lysilogy/corpus"):
            with self.subTest(root=root), self.assertRaises(corpus.CorpusError):
                corpus.validate_root(root)
        self.assertEqual(Path.home() / "Corpora/arxiv", corpus.validate_root("~/Corpora/arxiv"))

    def should_reject_external_hosts_when_download_url_or_redirect_is_untrusted(self):
        for url in ("http://storage.googleapis.com/file", "https://127.0.0.1/file", "https://storage.googleapis.com.attacker.invalid/", "https://user@export.arxiv.org/e-print/id"):
            with self.subTest(url=url), self.assertRaises(corpus.CorpusError):
                corpus.validate_url(url)

    def should_share_production_pacing_when_metadata_and_sources_use_one_client(self):
        current = [1000.0]
        starts = []
        def sleep(delay):
            current[0] += delay
        class Opener:
            def open(self, request, timeout):
                starts.append(current[0])
                return Response(b"fixture")
        client = corpus.Http(cache_root=self.root, sleep=sleep, clock=lambda: current[0])
        client.opener = Opener()
        for url in (corpus.OAI, corpus.EXPORT + "2001.00001v1", corpus.OAI):
            client.bytes(url)
        self.assertEqual([1000, 1003, 1006], starts)
        # Separate clients/roots still share a policy clock through cache_root.
        another = corpus.Http(cache_root=self.root, sleep=sleep, clock=lambda: current[0])
        another.opener = Opener()
        another.bytes(corpus.OAI)
        self.assertEqual(1009, starts[-1])

    def should_refuse_download_before_network_when_disk_projection_is_unsafe(self):
        config = small_config()
        paper = {"id": "2001.00001", "tiers": ["eval"], "categories": ["cs.LG"]}
        corpus.atomic_json(self.root / "selection.json", {"papers": [paper], "config_sha256": corpus.fingerprint(config), "selection_sha256": corpus.fingerprint([paper])})
        client = FakeHttp([])
        with patch.object(corpus, "check_space", side_effect=corpus.CorpusError("Insufficient disk space")):
            with self.assertRaisesRegex(corpus.CorpusError, "Insufficient disk space"):
                corpus.download(self.root, config, client)
        self.assertEqual([], client.calls)

    def complete_fixture(self):
        config = small_config()
        for spec in config["tiers"].values():
            spec["count"] = 1
        paper = {"id": "2001.00001", "tiers": ["eval", "scale"], "categories": ["cs.LG"]}
        selection = {"papers": [paper], "config_sha256": corpus.fingerprint(config), "selection_sha256": corpus.fingerprint([paper])}
        corpus.atomic_json(self.root / "selection.json", selection)
        pdf = b"%PDF-1.7 fixture"
        md5 = base64.b64encode(hashlib.md5(pdf, usedforsecurity=False).digest()).decode()
        listing = {"items": [{"name": "arxiv/arxiv/pdf/2001/2001.00001v2.pdf", "size": str(len(pdf)), "md5Hash": md5, "generation": "123"}]}
        client = FakeHttp([json.dumps(listing).encode(), pdf, b"\\documentclass{article}"])
        with contextlib.redirect_stdout(io.StringIO()):
            corpus.download(self.root, config, client)
            self.assertTrue(corpus.status(self.root, verify=True, config=config))
        return config, paper, client

    def should_pin_matching_source_version_and_resume_when_full_download_completes(self):
        config, paper, client = self.complete_fixture()
        manifest = corpus.read_manifest(self.root)
        self.assertEqual(2, manifest[paper["id"]]["version"])
        self.assertEqual("https://export.arxiv.org/src/2001.00001v2", client.calls[-1])
        self.assertIn("?generation=123", client.calls[-2])
        offline = FakeHttp([])
        with contextlib.redirect_stdout(io.StringIO()):
            corpus.download(self.root, config, offline)
        self.assertEqual([], offline.calls)

    def should_use_canonical_source_location_when_a_pdf_version_is_pinned(self):
        path, url = corpus.artifact_location({"id": "0812.5080", "version": 5}, "source")
        self.assertEqual("source/0812.5080v5.src", path)
        self.assertEqual("https://export.arxiv.org/src/0812.5080v5", url)

    def should_reuse_legacy_source_receipts_when_an_admitted_corpus_is_upgraded(self):
        config, paper, _ = self.complete_fixture()
        entries = corpus.read_manifest(self.root)
        source = entries[paper["id"]]["source"]
        source["url"] = corpus.LEGACY_EXPORT + paper["id"] + "v2"
        path = self.root / source["path"]
        receipt = {key: value for key, value in source.items() if key != "path"}
        receipt_path = path.with_name(path.name + ".verified.json")
        corpus.atomic_json(receipt_path, receipt)
        corpus.write_manifest(self.root, entries)
        original_bytes, original_receipt = path.read_bytes(), receipt_path.read_bytes()
        offline = FakeHttp([])
        corpus.download(self.root, config, offline)
        self.assertEqual([], offline.calls)
        self.assertEqual(entries, corpus.read_manifest(self.root))
        self.assertEqual(original_bytes, path.read_bytes())
        self.assertEqual(original_receipt, receipt_path.read_bytes())
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertTrue(corpus.status(self.root, verify=True, config=config))

    def should_resume_legacy_source_provenance_when_its_manifest_update_was_interrupted(self):
        config, paper, _ = self.complete_fixture()
        entries = corpus.read_manifest(self.root)
        source = entries[paper["id"]]["source"]
        path = self.root / source["path"]
        receipt = {key: value for key, value in source.items() if key != "path"}
        receipt["url"] = corpus.LEGACY_EXPORT + paper["id"] + "v2"
        corpus.atomic_json(path.with_name(path.name + ".verified.json"), receipt)
        entries[paper["id"]]["source"] = None
        corpus.write_manifest(self.root, entries)
        offline = FakeHttp([])
        corpus.download(self.root, config, offline)
        self.assertEqual([], offline.calls)
        recovered = corpus.read_manifest(self.root)[paper["id"]]["source"]
        self.assertEqual({**receipt, "path": source["path"]}, recovered)
        self.assertEqual(receipt["fetched_at"], recovered["fetched_at"])

    def should_reject_legacy_source_aliases_when_identity_or_route_is_different(self):
        config, paper, _ = self.complete_fixture()
        entry = corpus.read_manifest(self.root)[paper["id"]]
        source = entry["source"]
        expected = corpus.EXPORT + paper["id"] + "v2"
        legacy = corpus.LEGACY_EXPORT + paper["id"] + "v2"
        for url in [corpus.LEGACY_EXPORT + paper["id"] + "v1", corpus.LEGACY_EXPORT + "2001.00002v2",
                    corpus.LEGACY_EXPORT + paper["id"], legacy + "?download=1", legacy + "#fragment",
                    legacy.replace("https://", "http://"), legacy.replace("export.arxiv.org", "arxiv.org"),
                    legacy.replace("export.arxiv.org", "oaipmh.arxiv.org"), legacy.replace("/e-print/", "/other/")]:
            with self.subTest(url=url):
                self.assertFalse(corpus.receipt_url_matches(url, expected, "source"))
                with self.assertRaisesRegex(corpus.CorpusError, "pinned paper version"):
                    corpus.validate_receipt(entry, "source", {**source, "url": url})
        self.assertFalse(corpus.receipt_url_matches(legacy, expected, "pdf"))
        with self.assertRaisesRegex(corpus.CorpusError, "pinned paper version"):
            corpus.validate_receipt(entry, "source", {**source, "url": legacy, "path": "source/2001.00001v1.src"})

    def should_preserve_existing_source_bytes_when_a_legacy_receipt_is_invalid(self):
        config, paper, _ = self.complete_fixture()
        source = corpus.read_manifest(self.root)[paper["id"]]["source"]
        path = self.root / source["path"]
        receipt = {key: value for key, value in source.items() if key != "path"}
        receipt["url"] = corpus.LEGACY_EXPORT + paper["id"] + "v1"
        receipt_path = path.with_name(path.name + ".verified.json")
        corpus.atomic_json(receipt_path, receipt)
        original = path.read_bytes(), receipt_path.read_bytes()
        with self.assertRaisesRegex(corpus.CorpusError, "leaving it untouched"):
            corpus.download(self.root, config, FakeHttp([]))
        self.assertEqual(original, (path.read_bytes(), receipt_path.read_bytes()))

    def should_keep_redirects_disabled_when_a_source_response_moves_again(self):
        with self.assertRaisesRegex(corpus.CorpusError, "Unexpected HTTP redirect"):
            corpus.NoRedirect().redirect_request(None, None, 301, "Moved", {}, corpus.EXPORT + "0812.5080v5")

    def should_refuse_legacy_receipt_reuse_when_its_kind_or_bytes_do_not_match(self):
        path = self.root / "source/2001.00001v2.src"
        legacy = corpus.LEGACY_EXPORT + "2001.00001v2"
        receipt, _ = corpus.fetch_file(path, legacy, "source", FakeHttp([b"\\documentclass{article}"]), 0, 1024)
        receipt_path = path.with_name(path.name + ".verified.json")
        original_bytes = path.read_bytes()
        for changed in ({**receipt, "kind": "pdf"}, {**receipt, "sha256": "0" * 64}):
            with self.subTest(changed=changed):
                corpus.atomic_json(receipt_path, changed)
                with self.assertRaisesRegex(corpus.CorpusError, "leaving it untouched"):
                    corpus.fetch_file(path, corpus.EXPORT + "2001.00001v2", "source", FakeHttp([]), 0, 1024)
                self.assertEqual(original_bytes, path.read_bytes())
                self.assertEqual(changed, corpus.read_json(receipt_path))

    def should_reject_symlink_receipt_when_it_points_outside_corpus(self):
        path = self.root / "paper.pdf"
        outside = self.root / "outside.json"
        outside.write_text("not read")
        path.with_name("paper.pdf.verified.json").symlink_to(outside)
        with self.assertRaisesRegex(corpus.CorpusError, "symlinks"):
            corpus.fetch_file(path, corpus.GCS + "/test", "pdf", FakeHttp([]), 0, 1024)

    def should_reject_persisted_path_escape_when_selection_is_modified(self):
        config = small_config()
        paper = {"id": "../../outside", "tiers": ["scale"]}
        selection = {"papers": [paper], "config_sha256": corpus.fingerprint(config), "selection_sha256": corpus.fingerprint([paper])}
        corpus.atomic_json(self.root / "selection.json", selection)
        client = FakeHttp([])
        with self.assertRaisesRegex(corpus.CorpusError, "Invalid paper ID"):
            corpus.download(self.root, config, client)
        self.assertEqual([], client.calls)

    def should_reject_manifest_version_drift_when_a_saved_entry_changes(self):
        paper = {"id": "2001.00001", "tiers": ["scale"]}
        entry = {**paper, "version": 2, "selection_sha256": "selection",
                 "remote_pdf": {"name": "arxiv/arxiv/pdf/2001/2001.00001v1.pdf", "generation": "1", "bytes": 1, "version": 1}}
        with self.assertRaisesRegex(corpus.CorpusError, "Manifest identity differs"):
            corpus.validate_entry(entry, paper, "selection")

    def should_fail_verification_when_the_selection_has_missing_downloads(self):
        config = small_config()
        for spec in config["tiers"].values():
            spec["count"] = 1
        chosen = [{"id": "2001.00001", "tiers": ["eval", "scale"]}]
        corpus.atomic_json(self.root / "selection.json", {"papers": chosen, "config_sha256": corpus.fingerprint(config),
                                                         "selection_sha256": corpus.fingerprint(chosen)})
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertFalse(corpus.status(self.root, verify=True, config=config))
        report = json.loads(output.getvalue())
        self.assertEqual(1, report["tiers"]["scale"]["selected"])
        self.assertIn("2001.00001: missing source", report["problems"])

    def should_reject_escape_when_manifest_path_points_outside_corpus(self):
        config, paper, _ = self.complete_fixture()
        entries = corpus.read_manifest(self.root)
        entries[paper["id"]]["pdf"]["path"] = "../outside.pdf"
        corpus.write_manifest(self.root, entries)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertFalse(corpus.status(self.root, verify=True, config=config))
        self.assertIn("Manifest path leaves corpus root", output.getvalue())

    def should_reject_manifest_corruption_when_pinned_provenance_is_changed(self):
        config, paper, _ = self.complete_fixture()
        entries = corpus.read_manifest(self.root)
        changes = [
            (("remote_pdf", "name"), "arxiv/arxiv/pdf/2001/2001.00002v2.pdf"),
            (("remote_pdf", "md5"), base64.b64encode(b"x" * 16).decode()),
            (("remote_pdf", "bytes"), 12345),
            (("remote_pdf", "generation"), "124"),
            (("version",), 3),
            (("categories",), ["math.PR"]),
            (("pdf", "path"), "pdf/2001.00002v2.pdf"),
            (("pdf", "url"), corpus.GCS + "/other.pdf"),
            (("source", "path"), "source/2001.00001v1.src"),
            (("source", "url"), corpus.EXPORT + "2001.00001v1"),
            (("source", "kind"), "pdf"),
        ]
        for keys, value in changes:
            with self.subTest(keys=keys):
                corrupted = copy.deepcopy(entries)
                target = corrupted[paper["id"]]
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                corpus.write_manifest(self.root, corrupted)
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertFalse(corpus.status(self.root, verify=True, config=config))
        corpus.write_manifest(self.root, entries)

    def should_reject_frozen_selection_corruption_when_config_or_records_change(self):
        config, _, _ = self.complete_fixture()
        selection = corpus.read_json(self.root / "selection.json")
        for changed in ("config_sha256", "selection_sha256", "records"):
            with self.subTest(changed=changed):
                corrupted = copy.deepcopy(selection)
                if changed == "records":
                    corrupted["papers"][0]["title"] = "modified after selection"
                else:
                    corrupted[changed] = "incorrect"
                corpus.atomic_json(self.root / "selection.json", corrupted)
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    self.assertFalse(corpus.status(self.root, verify=True, config=config))
                self.assertIn("Invalid frozen selection", output.getvalue())
        corpus.atomic_json(self.root / "selection.json", selection)

    def should_reject_unselected_manifest_entry_when_its_artifact_hashes_are_valid(self):
        config, paper, _ = self.complete_fixture()
        entries = corpus.read_manifest(self.root)
        entries["2001.00002"] = {**entries[paper["id"]], "id": "2001.00002"}
        corpus.write_manifest(self.root, entries)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertFalse(corpus.status(self.root, verify=True, config=config))
        self.assertIn("absent from frozen selection", output.getvalue())

    def should_reject_changed_pdf_when_local_receipt_matches_but_pinned_remote_does_not(self):
        config, paper, _ = self.complete_fixture()
        entries = corpus.read_manifest(self.root)
        receipt = entries[paper["id"]]["pdf"]
        path = self.root / receipt["path"]
        path.write_bytes(b"%PDF-1.7 changed")
        receipt.update(corpus.digest_file(path))
        corpus.write_manifest(self.root, entries)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertFalse(corpus.status(self.root, verify=True, config=config))
        self.assertIn("differs from pinned GCS hash or size", output.getvalue())

    def should_overlap_first_response_day_when_harvest_resumes_across_midnight(self):
        config = small_config()
        config["sets"] = {"cs.LG": "cs:cs:LG"}
        first = FakeHttp([oai(record(), "next-page", date="2026-09-12T23:59:00Z")])
        with self.assertRaises(StopIteration):
            corpus.harvest(self.root, config, first)
        corpus.harvest(self.root, config, FakeHttp([oai(record("2001.00002"), date="2026-09-13T00:01:00Z")]))
        refresh = FakeHttp([oai(record("2001.00003"), date="2026-09-14T00:00:00Z")])
        corpus.harvest(self.root, config, refresh, refresh=True)
        self.assertIn("from=2026-09-12", refresh.calls[0])
        self.assertNotIn("from=2026-09-13", refresh.calls[0])

    def should_replay_original_bound_when_legacy_checkpoint_lacks_window_start(self):
        config = small_config()
        config["sets"] = {"cs.LG": "cs:cs:LG"}
        db = corpus.connect(self.root)
        with db:
            state = {"from": "2020-01-01", "token": "", "complete": True, "response_date": "2026-09-13T00:00:00Z"}
            db.execute("INSERT INTO harvests VALUES (?, ?)", ("cs:cs:LG", corpus.canonical(state)))
        db.close()
        refresh = FakeHttp([oai(record())])
        corpus.harvest(self.root, config, refresh, refresh=True)
        self.assertIn("from=2020-01-01", refresh.calls[0])

    def should_persist_final_retry_cooldown_when_a_fresh_client_follows_failure(self):
        current = [3907.0]
        starts = []
        def sleep(delay):
            current[0] += delay
        class Opener:
            def open(self, request, timeout):
                starts.append(current[0])
                if len(starts) <= 6:
                    headers = {"Retry-After": "600"} if len(starts) == 6 else {}
                    raise corpus.urllib.error.HTTPError(request.full_url, 429, "rate limited", headers, None)
                return Response(b"fixture")
        opener = Opener()
        first = corpus.Http(cache_root=self.root, sleep=sleep, clock=lambda: current[0])
        first.opener = opener
        with self.assertRaises(corpus.urllib.error.HTTPError):
            first.bytes(corpus.OAI)
        self.assertEqual(4000, current[0])
        self.assertEqual(4600, float((self.root / "arxiv-http.lock").read_text()))
        second = corpus.Http(cache_root=self.root, sleep=sleep, clock=lambda: current[0])
        second.opener = opener
        second.bytes(corpus.EXPORT + "2001.00001v1")
        self.assertEqual(4600, starts[-1])

    def should_preserve_server_cooldown_when_sleep_is_interrupted(self):
        current = [4000.0]
        starts = []
        def interrupted_sleep(delay):
            raise KeyboardInterrupt()
        class Opener:
            def open(self, request, timeout):
                starts.append(current[0])
                if len(starts) == 1:
                    raise corpus.urllib.error.HTTPError(request.full_url, 429, "rate limited", {"Retry-After": "600"}, None)
                return Response(b"fixture")
        opener = Opener()
        first = corpus.Http(cache_root=self.root, sleep=interrupted_sleep, clock=lambda: current[0])
        first.opener = opener
        with self.assertRaises(KeyboardInterrupt):
            first.bytes(corpus.OAI)
        self.assertEqual(4600, float((self.root / "arxiv-http.lock").read_text()))
        def sleep(delay):
            current[0] += delay
        second = corpus.Http(cache_root=self.root, sleep=sleep, clock=lambda: current[0])
        second.opener = opener
        second.bytes(corpus.OAI)
        self.assertEqual([4000, 4600], starts)


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader)
