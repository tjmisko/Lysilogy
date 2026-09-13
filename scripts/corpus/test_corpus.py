"""Offline contract tests; generated payloads are tiny and never real papers."""

import base64
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import types
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


class CorpusTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="lysilogy-corpus-unit-")
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)

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
        first = corpus.select(self.root, config)
        with db:
            db.execute("DELETE FROM papers")
        db.close()
        self.assertEqual(first, corpus.select(self.root, config))
        config["seed"] = "new"
        with self.assertRaisesRegex(corpus.CorpusError, "different config"):
            corpus.select(self.root, config)

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

    def should_pin_matching_source_version_and_resume_when_full_download_completes(self):
        config = small_config()
        paper = {"id": "2001.00001", "tiers": ["eval", "scale"], "categories": ["cs.LG"]}
        selection = {"papers": [paper], "config_sha256": corpus.fingerprint(config), "selection_sha256": corpus.fingerprint([paper])}
        corpus.atomic_json(self.root / "selection.json", selection)
        pdf = b"%PDF-1.7 fixture"
        md5 = base64.b64encode(hashlib.md5(pdf, usedforsecurity=False).digest()).decode()
        listing = {"items": [{"name": "arxiv/arxiv/pdf/2001/2001.00001v2.pdf", "size": str(len(pdf)), "md5Hash": md5, "generation": "123"}]}
        client = FakeHttp([json.dumps(listing).encode(), pdf, b"\\documentclass{article}"])
        with contextlib.redirect_stdout(io.StringIO()):
            corpus.download(self.root, config, client)
            self.assertTrue(corpus.status(self.root, verify=True))
        manifest = corpus.read_manifest(self.root)
        self.assertEqual(2, manifest[paper["id"]]["version"])
        self.assertEqual("https://export.arxiv.org/e-print/2001.00001v2", client.calls[-1])
        self.assertIn("?generation=123", client.calls[-2])
        offline = FakeHttp([])
        with contextlib.redirect_stdout(io.StringIO()):
            corpus.download(self.root, config, offline)
        self.assertEqual([], offline.calls)

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
        corpus.atomic_json(self.root / "selection.json", {"papers": [{"id": "2001.00001", "tiers": ["eval", "scale"]}]})
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertFalse(corpus.status(self.root, verify=True))
        report = json.loads(output.getvalue())
        self.assertEqual(1, report["tiers"]["scale"]["selected"])
        self.assertIn("2001.00001: missing source", report["problems"])

    def should_reject_escape_when_manifest_path_points_outside_corpus(self):
        paper = {"id": "2001.00001", "tiers": ["scale"], "pdf": {"path": "../outside.pdf"}}
        corpus.atomic_json(self.root / "selection.json", {"papers": [paper]})
        corpus.write_manifest(self.root, {paper["id"]: paper})
        with self.assertRaisesRegex(corpus.CorpusError, "leaves corpus root"):
            corpus.status(self.root, verify=True)


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader)
