"""Failure/provenance boundaries of orchestration, without invoking TeX."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from experiment import build_paper, compare_document
from policy import Refused, binding


class ExperimentTests(unittest.TestCase):
    def make_paper(self, root, raw=b"\\documentclass{article}\\begin{document}x\\end{document}"):
        path = root / "source.src"
        path.write_bytes(raw)
        return {"arxiv_id": "synthetic", "version": 1, "mapping": {"paper_id": "synthetic"},
                "frozen_artifacts": {"source": binding(path)}, "source_date_epoch": 0,
                "date_fidelity": "synthetic"}

    def should_withhold_a_pdf_when_the_second_pass_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paper = self.make_paper(root)
            calls = []

            def run(**kwargs):
                calls.append(kwargs)
                status = "passed" if len(calls) == 1 else "process_failed"
                (kwargs["output"] / "main.pdf").write_bytes(b"%PDF-synthetic first pass")
                (kwargs["run_dir"] / "receipt.json").write_text(json.dumps({"status": status}))
                return {"status": status, "wall_seconds": 0.1}

            with patch("experiment.run_sandbox", side_effect=run):
                result = build_paper(paper, {"engine_mounts": []}, root / "build")
            self.assertEqual(len(calls), 2)
            self.assertEqual(result["status"], "process_failed")
            self.assertNotIn("pdf", result)
            self.assertEqual(binding(root / "source.src"), paper["frozen_artifacts"]["source"])

    def should_retain_a_source_failure_when_no_unique_main_document_exists(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paper = self.make_paper(root, b"no declared TeX source")
            with patch("experiment.run_sandbox") as run:
                result = build_paper(paper, {"engine_mounts": []}, root / "build")
            run.assert_not_called()
            self.assertEqual(result["status"], "unsupported_or_refused")
            self.assertEqual(result["passes"], [])
            self.assertEqual(json.loads((root / "build/receipt.json").read_text())["status"], result["status"])

    def should_record_count_mismatch_when_rendering_would_clamp_a_missing_page(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pdf = root / "synthetic.pdf"
            pdf.write_bytes(b"synthetic placeholder; not parsed")
            paper = {"page_count": 2, "frozen_artifacts": {"pdf": binding(pdf)}}
            with patch("experiment.render_page") as render:
                result = compare_document(paper, pdf, {}, root / "comparison", 1)
            render.assert_not_called()
            self.assertEqual(result["status"], "page_count_mismatch")
            self.assertFalse(result["exact_whole_document"])

    def should_retain_failed_correspondence_when_a_required_raster_is_unavailable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pdf = root / "synthetic.pdf"
            pdf.write_bytes(b"synthetic placeholder; not parsed")
            paper = {"page_count": 1, "frozen_artifacts": {"pdf": binding(pdf)}}
            with patch("experiment.render_page", side_effect=Refused("synthetic renderer failure")), self.assertRaises(Refused):
                compare_document(paper, pdf, {}, root / "comparison", 1)
            result = json.loads((root / "comparison/receipt.json").read_text())
            self.assertEqual(result["status"], "failed")
            self.assertFalse(result["exact_whole_document"])
            self.assertEqual(result["pages"], [])


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
