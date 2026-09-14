"""Synthetic archives test exact bytes, refusals, and pre-outcome selection."""
import gzip
import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import inputs
from inputs import main_source, materialize, select_ten, source_members
from policy import Refused


def archive(entries):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w") as tar:
        for name, data, kind in entries:
            item = tarfile.TarInfo(name)
            item.type = kind
            item.size = len(data) if kind == tarfile.REGTYPE else 0
            if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                item.linkname = "synthetic-target"
            tar.addfile(item, io.BytesIO(data))
    return stream.getvalue()


class InputTests(unittest.TestCase):
    def should_preserve_binary_and_text_bytes_when_materializing_a_safe_archive(self):
        text = b"% comment\r\n\\documentclass{article}\r\n\\begin{document}x^2\\end{document}\r\n"
        image = bytes(range(256))
        raw = gzip.compress(archive([("./main.tex", text, tarfile.REGTYPE), ("figures/a.png", image, tarfile.REGTYPE)]))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.src"
            source.write_bytes(raw)
            report = materialize(source, hashlib.sha256(raw).hexdigest(), root / "input")
            self.assertEqual((root / "input/main.tex").read_bytes(), text)
            self.assertEqual((root / "input/figures/a.png").read_bytes(), image)
            self.assertEqual(source.read_bytes(), raw)
            self.assertEqual(report["main"], "main.tex")
            self.assertFalse(report["deposited_source_executed"])

    def should_refuse_links_devices_duplicates_and_traversal_when_archive_members_are_unsafe(self):
        for entries in (
            [("../escape", b"x", tarfile.REGTYPE)], [("/escape", b"x", tarfile.REGTYPE)],
            [("x", b"", tarfile.SYMTYPE)], [("x", b"", tarfile.LNKTYPE)],
            [("x", b"", tarfile.CHRTYPE)], [(".env", b"synthetic", tarfile.REGTYPE)],
            [("a", b"x", tarfile.REGTYPE), ("./a", b"y", tarfile.REGTYPE)],
            [("a", b"x", tarfile.REGTYPE), ("a/b", b"y", tarfile.REGTYPE)],
        ):
            with self.subTest(entries=entries), self.assertRaises(Refused):
                source_members(archive(entries))

    def should_stop_decompression_when_expansion_exceeds_the_cap(self):
        with patch.object(inputs, "EXPANDED_BYTES", 1024), self.assertRaises(Refused):
            source_members(gzip.compress(b"x" * 2000))

    def should_reject_oversized_members_when_archive_headers_exceed_the_cap(self):
        with patch.object(inputs, "MEMBER_BYTES", 4), self.assertRaises(Refused):
            source_members(archive([("main.tex", b"12345", tarfile.REGTYPE)]))

    def should_refuse_ambiguous_roots_when_two_documents_are_deposited(self):
        with self.assertRaises(Refused):
            main_source({"a.tex": b"\\documentclass{article}", "b.tex": b"\\documentclass{article}"})
        self.assertEqual(main_source({"a.tex": b"%\\documentclass{ignored}\n\\documentclass{article}",
                                      "b.tex": b"% \\documentclass{not a root}"}), "a.tex")

    def should_refuse_stale_trees_and_wrong_identities_when_materialization_is_repeated(self):
        raw = b"\\documentclass{article}"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "paper.src"
            source.write_bytes(raw)
            with self.assertRaises(Refused):
                materialize(source, "0" * 64, root / "out")
            self.assertFalse((root / "out").exists())
            materialize(source, hashlib.sha256(raw).hexdigest(), root / "out")
            with self.assertRaises(FileExistsError):
                materialize(source, hashlib.sha256(raw).hexdigest(), root / "out")

    def should_select_the_same_balanced_ten_when_later_outcome_fields_change(self):
        papers = [{"arxiv_id": str(100 + position), "version": 1, "stratum": [str(position % 7), 2020],
                   "category_rank": position // 7 + 1, "input_position": position,
                   "status": "selected", "page_count": 5} for position in range(21)]
        first = select_ten({"selected": papers})
        for paper in papers:
            paper["later_parser_outcome"] = "failed" if paper["input_position"] < 10 else "passed"
        second = select_ten({"selected": list(reversed(papers))})
        self.assertEqual([p["arxiv_id"] for p in first], [p["arxiv_id"] for p in second])
        self.assertEqual([p["input_position"] for p in first], list(range(10)))


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
