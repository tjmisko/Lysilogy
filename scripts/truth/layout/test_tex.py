"""Synthetic installed-engine checks; no deposited source or original paper PDF.

An explicit evidence path beneath the external cache retains generated fixtures
for independent review. Normal offline tests clean up their private directory.
"""
import json
import os
from pathlib import Path
import tempfile
import unittest

from confinement import check_tools, prepare_outputs, run_sandbox
from experiment import build_paper, compare_document, page_count
from inputs import materialize
from policy import Limits, binding, engine_command, fixed_environment, output_names, regular_path
import runtime


class TexTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cache = Path.home() / ".cache/lysilogy/source-layout-probe/synthetic"
        cache.mkdir(parents=True, exist_ok=True)
        evidence = os.environ.get("LYSILOGY_LAYOUT_TEST_EVIDENCE")
        cls.temporary = None
        if evidence:
            cls.root = Path(evidence).absolute()
            if not cls.root.is_relative_to(Path.home() / ".cache/lysilogy"):
                raise ValueError("retained synthetic test evidence must use the external cache")
            regular_path(cls.root.parent, directory=True)
            cls.root.mkdir(mode=0o700, exist_ok=False)
        else:
            cls.temporary = tempfile.TemporaryDirectory(prefix="tex-", dir=cache)
            cls.root = Path(cls.temporary.name)
        cls.installed = runtime.discover()
        cls.installed["confinement_tools"] = check_tools()
        (cls.root / "runtime.json").write_text(json.dumps(cls.installed, sort_keys=True, indent=2) + "\n")

    @classmethod
    def tearDownClass(cls):
        runtime.verify(cls.installed)
        if cls.temporary is not None:
            cls.temporary.cleanup()

    def source(self, name, body, *, prefix=b"", preamble=b""):
        directory = self.root / (self._testMethodName + "-" + name)
        directory.mkdir()
        source = directory / "generated.src"
        source.write_bytes(prefix + b"\\documentclass{article}\n" + preamble +
                           b"\\begin{document}\n" + body + b"\n\\end{document}\n")
        source.chmod(0o400)
        paper = {"arxiv_id": "synthetic-" + name, "version": 1, "mapping": {"paper_id": "synthetic"},
                 "frozen_artifacts": {"source": binding(source)}, "source_date_epoch": 0,
                 "date_fidelity": "synthetic fixed epoch", "page_count": 1}
        return directory, paper

    def build(self, name, body, **kwargs):
        directory, paper = self.source(name, body, **kwargs)
        result = build_paper(paper, self.installed, directory / "build")
        self.assertEqual(result["status"], "built", result)
        self.assertEqual(page_count(result["pdf"]["path"], self.installed, directory / "page-count")["count"], 1)
        return directory, paper, result

    def should_reproduce_every_pixel_when_the_same_source_is_built_in_fresh_directories(self):
        body = b"Synthetic mathematics: $x^2$, $\\alpha$, $\\frac{a}{b+c}$."
        _, paper, original = self.build("original", body)
        directory, _, rebuilt = self.build("fresh", body)
        paper["frozen_artifacts"]["pdf"] = original["pdf"]
        result = compare_document(paper, rebuilt["pdf"]["path"], self.installed, directory / "comparison", 1)
        self.assertTrue(result["exact_whole_document"])
        self.assertEqual([p["dpi"] for p in result["pages"]], [96, 192])

    def should_reject_correspondence_when_superscripts_glyphs_or_fractions_change(self):
        _, paper, original = self.build("original", b"$x^2$, $\\alpha$, $\\frac{a}{b+c}$")
        paper["frozen_artifacts"]["pdf"] = original["pdf"]
        for name, body in (("superscript", b"$x2$, $\\alpha$, $\\frac{a}{b+c}$"),
                           ("glyph", b"$x^2$, $A$, $\\frac{a}{b+c}$"),
                           ("fraction", b"$x^2$, $\\alpha$, $\\frac{b+c}{a}$")):
            with self.subTest(name=name):
                directory, _, rebuilt = self.build(name, body)
                result = compare_document(paper, rebuilt["pdf"]["path"], self.installed, directory / "comparison", 1)
                self.assertFalse(result["exact_whole_document"])
                self.assertEqual(len(result["pages"]), 2)
                self.assertTrue(all(not p["exact_whole_page"] for p in result["pages"]))

    def should_keep_the_fixed_engine_and_format_when_source_requests_another_engine(self):
        assertions = (b"\\ifdefined\\directlua\\errmessage{Lua engine escaped policy}\\fi\n"
                      b"\\ifdefined\\XeTeXversion\\errmessage{XeTeX escaped policy}\\fi\n"
                      b"\\ifnum\\pdfoutput=1\\else\\errmessage{PDF mode unavailable}\\fi\n"
                      b"\\def\\wanted{LaTeX2e}\\ifx\\fmtname\\wanted\\else\\errmessage{wrong format}\\fi\n")
        self.build("override", b"Fixed engine and format", prefix=b"%&luatex\n% !TEX program = xelatex\n", preamble=assertions)

    def should_disable_shell_escape_when_source_attempts_a_write_eighteen_command(self):
        body = (b"\\ifnum\\pdfshellescape=0\\else\\errmessage{shell escape enabled}\\fi\n"
                b"\\immediate\\write18{touch /output/escaped}\nShell escape is disabled.")
        directory, _, result = self.build("shell", body)
        self.assertFalse((directory / "build/output/escaped").exists())
        self.assertEqual(result["status"], "built")

    def should_hide_host_sentinels_when_source_attempts_an_absolute_include(self):
        sentinel = self.root / "synthetic-host-sentinel.txt"
        sentinel.write_text("synthetic sentinel must stay unmounted\n")
        body = (b"\\newread\\probe\\openin\\probe=" + str(sentinel).encode() +
                b" \\ifeof\\probe Hidden host file.\\else\\errmessage{host sentinel exposed}\\fi\\closein\\probe")
        self.build("read", body)
        self.assertEqual(sentinel.read_text(), "synthetic sentinel must stay unmounted\n")

    def should_refuse_source_writes_when_an_output_path_traverses_into_the_read_only_input(self):
        directory, paper = self.source("write", b"\\newwrite\\probe\\immediate\\openout\\probe=/output/../input/main.tex ")
        result = build_paper(paper, self.installed, directory / "build")
        self.assertEqual(result["status"], "process_failed")
        self.assertEqual(binding(paper["frozen_artifacts"]["source"]["path"]), paper["frozen_artifacts"]["source"])

    def should_terminate_infinite_expansion_when_the_engine_exceeds_its_wall_budget(self):
        directory, paper = self.source("loop", b"\\loop\\iftrue\\repeat")
        source = materialize(paper["frozen_artifacts"]["source"]["path"], paper["frozen_artifacts"]["source"]["sha256"], directory / "input")
        names = output_names(source["main"])
        output = prepare_outputs(directory / "output", names)
        controller = directory / "controller"
        controller.mkdir()
        result = run_sandbox(run_dir=controller, readonly=[*self.installed["engine_mounts"], (directory / "input", "/input")],
                             output=output, names=names, command=engine_command(source["main"]),
                             environment=fixed_environment(0), limits=Limits(wall_seconds=0.25, cpu_seconds=1),
                             expected_tools=self.installed["confinement_tools"])
        self.assertEqual(result["status"], "wall_timeout")
        self.assertIn("LaTeX", (controller / "stdout.log").read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(result["pid"], 0)


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
