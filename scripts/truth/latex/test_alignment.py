"""Independent alignment/coverage regressions, never real truth labels."""
import unittest

from align import TextAlignment, align_paper, utf16
from parser import parse_project
from test_latex import document


def fixture():
    left = "The independent experimental result supports"
    right = "our interpretation of the observed measurements."
    body = left + " [1] " + right
    bibliography = "[1] Smith, A. A uniquely identifiable synthetic study. 2020."
    text = "😀 " + body + "\n\nReferences\n" + bibliography
    source = document(left + r" \cite{one} " + right + r"\begin{thebibliography}{9}\bibitem{one}Smith, A. A uniquely identifiable synthetic study. 2020.\end{thebibliography}")
    return parse_project({"main.tex": source}), {"text": text, "tokens": [], "objects": {"malicious_prediction": "not read"}, "figures": [{"should_not_supply_truth": True}]}


class AlignmentTests(unittest.TestCase):
    def test_should_align_independent_entries_when_citations_and_pdf_text_match(self):
        parsed, index = fixture()
        result = align_paper(parsed, index)
        self.assertTrue(result["accepted"])
        self.assertTrue(result["bibliography_eligible"])
        self.assertEqual(result["alignment"]["quality"], 1)
        self.assertEqual(result["entries"][0]["spans"][0]["start"], utf16(index["text"][:index["text"].index("[1] Smith")]))
        at = index["text"].index("[1]")
        self.assertEqual(result["mentions"], [{"start": utf16(index["text"][:at]), "end": utf16(index["text"][:at]) + 3, "target": "one", "source_link": 0}])

    def test_should_exclude_a_paper_when_alignment_confidence_is_below_its_threshold(self):
        parsed, index = fixture()
        index["text"] = index["text"].replace("Smith, A. A uniquely identifiable synthetic study. 2020.", "Different paper content.")
        result = align_paper(parsed, index)
        self.assertFalse(result["accepted"])
        self.assertFalse(result["bibliography_eligible"])
        self.assertEqual(result["alignment"]["total_items"], 2)
        self.assertEqual(result["alignment"]["aligned_items"], 0)

    def test_should_withhold_bibliography_truth_when_an_unknown_citation_macro_is_used(self):
        parsed, index = fixture()
        parsed["coverage"]["unsupported_citation_commands"] = {"cites": 1}
        self.assertFalse(align_paper(parsed, index)["bibliography_eligible"])
        raw = document(r"A citation \mycitation{one}.", r"\newcommand{\mycitation}[1]{\cite{#1}}")
        self.assertEqual(parse_project({"main.tex": raw})["coverage"]["unsupported_citation_commands"], {"mycitation": 1})

    def test_should_keep_missing_occurrences_in_coverage_when_only_some_citations_align(self):
        parsed, index = fixture()
        parsed["links"].append({**parsed["links"][0], "context_before": "A completely different statement", "context_after": "with no counterpart in the supplied PDF"})
        result = align_paper(parsed, index)
        self.assertFalse(result["bibliography_eligible"])
        self.assertEqual(result["alignment"]["total_items"], 3)
        self.assertEqual(result["alignment"]["aligned_items"], 2)

    def test_should_reject_ambiguous_matches_when_the_same_authored_text_occurs_twice(self):
        alignment = TextAlignment({"text": "A distinctive caption.\nA distinctive caption."})
        span, reason = alignment.unique("A distinctive caption.")
        self.assertIsNone(span)
        self.assertIn("multiple", reason)

    def test_should_preserve_math_operators_when_equations_have_similar_letter_sequences(self):
        alignment = TextAlignment({"text": "abcdefgh < 12345"})
        self.assertIsNone(alignment.unique("abcdefgh > 12345", math=True)[0])

    def test_should_preserve_variable_case_when_equation_letters_are_semantic(self):
        alignment = TextAlignment({"text": "alpha=beta+gamma"})
        self.assertIsNone(alignment.unique("Alpha=Beta+Gamma", math=True)[0])
        self.assertIsNotNone(alignment.unique("alpha=beta+gamma", math=True)[0])
        for actual, authored in (("abcdefgh_2=12345", "abcdefgh^2=12345"),
                                 ("f(alpha,beta)=12345", "f(alphabeta)=12345"),
                                 ("abcdefgh2=12345", "abcdefgh²=12345")):
            self.assertIsNone(TextAlignment({"text": actual}).unique(authored, math=True)[0])

    def test_should_exclude_partial_math_when_the_renderer_does_not_know_an_operator(self):
        parsed = parse_project({"main.tex": document(r"\begin{equation}\unknownoperator abcdefghi=12345\end{equation}")})
        aligned = align_paper(parsed, {"text": "abcdefghi=12345"})
        self.assertEqual(aligned["objects"], [])
        self.assertIn("unsupported math", aligned["excluded_objects"][0]["reason"])

    def test_should_not_publish_caption_boxes_when_full_figure_regions_are_unknown(self):
        source = document(r"\begin{figure}\caption{A sufficiently distinctive caption.}\end{figure}")
        result = align_paper(parse_project({"main.tex": source}), {"text": "Figure 1: A sufficiently distinctive caption.", "tokens": []})
        self.assertIsNone(result["objects"][0]["region"])
        self.assertEqual(result["objects"][0]["region_status"], "independent visual annotation required")

    def test_should_keep_distinct_equations_when_align_rows_have_distinct_labels(self):
        raw = document(r"\begin{align}abcdefghi&=12345\label{eq:first}\\jklmnopqr&=67890\label{eq:second}\end{align}"
                       r"The first distinct equation is \eqref{eq:first} and completes the first argument. "
                       r"The second distinct equation is \eqref{eq:second} and completes the second argument.")
        parsed = parse_project({"main.tex": raw})
        self.assertEqual(len(parsed["objects"]), 2)
        self.assertNotEqual(parsed["label_targets"]["eq:first"], parsed["label_targets"]["eq:second"])
        self.assertEqual(parsed["objects"][0]["labels"], ["eq:first"])
        self.assertEqual(parsed["objects"][1]["labels"], ["eq:second"])
        text = ("abcdefghi=12345 (1)\njklmnopqr=67890 (2)\n"
                "The first distinct equation is (1) and completes the first argument. "
                "The second distinct equation is (2) and completes the second argument.")
        aligned = align_paper(parsed, {"text": text})
        self.assertEqual([row["target"] for row in aligned["references"]], ["object:eq:first", "object:eq:second"])

    def test_should_skip_unnumbered_rows_when_align_uses_notag_and_keep_nested_aligned_together(self):
        raw = document(r"\begin{align}a&=b\notag\\c&=d\label{eq:kept}\end{align}"
                       r"\begin{equation}\begin{aligned}e&=f\\g&=h\end{aligned}\label{eq:one}\end{equation}")
        parsed = parse_project({"main.tex": raw})
        self.assertEqual(len(parsed["objects"]), 2)
        self.assertEqual(parsed["objects"][0]["labels"], ["eq:kept"])
        self.assertEqual(parsed["objects"][1]["labels"], ["eq:one"])

    def test_should_count_an_explicit_tag_when_a_starred_equation_group_would_be_unnumbered(self):
        raw = document(r"\begin{align*}a&=b\\c&=d\tag{A}\label{eq:tag}\end{align*}")
        parsed = parse_project({"main.tex": raw})
        self.assertEqual(len(parsed["objects"]), 1)
        self.assertEqual(parsed["objects"][0]["labels"], ["eq:tag"])

    def test_should_withhold_exhaustive_object_truth_when_unknown_environments_might_hide_objects(self):
        parsed, index = fixture()
        parsed["coverage"]["unsupported_object_environments"] = {"customthm": 1}
        result = align_paper(parsed, index)
        self.assertFalse(result["accepted"])
        self.assertFalse(result["bibliography_eligible"])


if __name__ == "__main__":
    unittest.main()
