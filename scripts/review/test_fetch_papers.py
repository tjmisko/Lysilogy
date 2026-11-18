"""Unit tests for the review-set fetcher's offline logic.

Run from the repository root: python3 -B -m unittest scripts/review/test_fetch_papers.py
"""
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import fetch_papers as f  # noqa: E402

PDF = b"%PDF-1.7\n%fixture\n"


def manifest(**overrides):
    paper = {
        "id": "card1990",
        "title": "The Impact of the Mariel Boatlift",
        "filename": "card1990.pdf",
        "doi": "10.1177/001979399004300205",
        "urls": ["https://example.org/a.pdf", "https://example.org/b.pdf"],
        "neighbours": [
            {"id": "borjas2017", "title": "Reappraisal", "urls": [], "why": "Critique."},
            {"id": "peri2019", "title": "Synthetic control", "arxiv": "1234.5678", "why": "Critique."},
        ],
    }
    paper.update(overrides)
    return {"papers": [paper]}


class ManifestTests(unittest.TestCase):
    def test_should_accept_the_committed_manifest(self):
        loaded = f.load_manifest(f.DEFAULT_MANIFEST)
        self.assertGreaterEqual(len(loaded["papers"]), 9)
        fields = {paper["field"] for paper in loaded["papers"]}
        self.assertEqual(fields, {"ai", "economics"})
        for paper in loaded["papers"]:
            self.assertGreaterEqual(len(paper["neighbours"]), 3, paper["id"])
            self.assertLessEqual(len(paper["neighbours"]), 5, paper["id"])
            self.assertFalse(paper["verified"], paper["id"])

    def test_should_point_oracles_at_files_that_exist(self):
        root = f.DEFAULT_MANIFEST.parents[2]
        for paper in f.load_manifest(f.DEFAULT_MANIFEST)["papers"]:
            if paper.get("oracle"):
                self.assertTrue((root / paper["oracle"]).is_file(), paper["oracle"])

    def test_should_reject_duplicate_ids_and_unsafe_filenames(self):
        duplicate = manifest()
        duplicate["papers"].append(dict(duplicate["papers"][0]))
        with self.assertRaises(f.ManifestError):
            f.validate_manifest(duplicate)
        with self.assertRaises(f.ManifestError):
            f.validate_manifest(manifest(filename="../escape.pdf"))
        with self.assertRaises(f.ManifestError):
            f.validate_manifest(manifest(neighbours=[{"id": "x", "why": ""}]))


class PlanTests(unittest.TestCase):
    def test_should_derive_arxiv_urls_and_skip_insecure_ones(self):
        urls = f.candidate_urls({"urls": ["http://insecure.example/x.pdf", "https://ok.example/x.pdf"], "arxiv": "1706.03762"})
        self.assertEqual(urls, ["https://ok.example/x.pdf", "https://arxiv.org/pdf/1706.03762"])

    def test_should_place_neighbours_under_their_paper(self):
        target = Path("/library/review")
        jobs = f.plan_jobs(manifest(), target, include_neighbours=True)
        self.assertEqual([job.key for job in jobs], ["card1990", "card1990/borjas2017", "card1990/peri2019"])
        self.assertEqual(jobs[0].path, target / "card1990.pdf")
        self.assertEqual(jobs[2].path, target / "neighbours" / "card1990" / "peri2019.pdf")
        self.assertEqual(jobs[2].urls, ["https://arxiv.org/pdf/1234.5678"])
        self.assertEqual(len(f.plan_jobs(manifest(), target, include_neighbours=False)), 1)
        self.assertEqual(f.plan_jobs(manifest(), target, True, only=["other"]), [])


class ObtainTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.target = Path(self.directory.name)

    def tearDown(self):
        self.directory.cleanup()

    def job(self, urls):
        return f.Job("card1990", "Mariel", urls, self.target / "card1990.pdf", "doi:10.1/x")

    def test_should_fall_through_to_the_first_url_that_returns_a_pdf(self):
        responses = {"https://a.example/x": b"<html>login</html>", "https://b.example/x": PDF}

        def fetch(url):
            return responses[url]

        outcome = f.obtain(self.job(["https://a.example/x", "https://b.example/x"]), fetch)
        self.assertEqual(outcome.status, f.FETCHED)
        self.assertEqual(outcome.detail, "https://b.example/x")
        self.assertEqual((self.target / "card1990.pdf").read_bytes(), PDF)
        self.assertEqual(list(self.target.glob(".*.part")), [])

    def test_should_report_failures_and_manual_entries_without_writing(self):
        def fetch(url):
            raise urllib.error.URLError("unreachable")

        failed = f.obtain(self.job(["https://a.example/x"]), fetch)
        self.assertEqual(failed.status, f.FAILED)
        self.assertIn("save it by hand", failed.detail)
        manual = f.obtain(self.job([]), fetch)
        self.assertEqual(manual.status, f.MANUAL)
        self.assertIn("doi:10.1/x", manual.detail)
        self.assertFalse((self.target / "card1990.pdf").exists())

    def test_should_keep_a_pdf_already_in_place(self):
        (self.target / "card1990.pdf").write_bytes(PDF)

        def fetch(url):
            raise AssertionError("must not download")

        self.assertEqual(f.obtain(self.job(["https://a.example/x"]), fetch).status, f.PRESENT)

    def test_should_render_a_table_with_totals(self):
        outcomes = [f.Outcome(self.job([]), f.MANUAL, "drop it in"), f.Outcome(self.job([]), f.FETCHED, "https://x")]
        table = f.format_table(outcomes)
        self.assertIn("needs-manual", table)
        self.assertTrue(table.endswith("1 fetched, 1 needs-manual"))

    def test_should_recognise_pdf_magic(self):
        self.assertTrue(f.is_pdf(PDF))
        self.assertTrue(f.is_pdf(b"\n\n" + PDF))
        self.assertFalse(f.is_pdf(b"<!doctype html>"))


if __name__ == "__main__":
    unittest.main()
