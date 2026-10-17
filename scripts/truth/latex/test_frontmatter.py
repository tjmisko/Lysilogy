"""Synthetic front-matter fixtures for the identity/abstract oracle."""
import unittest

from frontmatter import front_matter


def document(preamble, body="", documentclass="article"):
    return (
        "\\documentclass{" + documentclass + "}\n" + preamble
        + "\n\\begin{document}\n\\maketitle\n" + body + "\n\\end{document}"
    )


def read(preamble, body="", documentclass="article"):
    return front_matter({"main.tex": document(preamble, body, documentclass)})


class TitleTests(unittest.TestCase):
    def test_should_read_the_long_title_when_a_short_title_is_given(self):
        result = read(r"\title[Short]{A Long Title for Testing}")
        self.assertEqual(result["title"], "A Long Title for Testing")

    def test_should_drop_thanks_and_line_breaks_when_reading_the_title(self):
        result = read(r"\title{Deep Learning\thanks{Funded by X.} \\ for Everyone}")
        self.assertEqual(result["title"], "Deep Learning for Everyone")

    def test_should_expand_user_macros_when_the_title_uses_one(self):
        result = read("\\newcommand{\\method}{FooNet}\n\\title{\\method: Fast Nets}")
        self.assertEqual(result["title"], "FooNet: Fast Nets")

    def test_should_keep_the_tex_branch_when_the_title_uses_texorpdfstring(self):
        result = read(r"\title{Bounds for \texorpdfstring{$\ell_1$}{l1} Norms}")
        self.assertIn("Bounds for", result["title"])
        self.assertNotIn("l1 Norms", result["title"].replace("ℓ", ""))

    def test_should_withhold_the_title_when_there_is_none(self):
        result = read(r"\author{Ann Lee}")
        self.assertIsNone(result["title"])
        self.assertIn("title", result["withheld"])

    def test_should_drop_spacing_colour_and_font_arguments_when_they_precede_the_title(self):
        result = read(
            "\\newcommand{\\pkg}[1]{{\\fontseries{b}\\selectfont #1}}\n"
            r"\title{\vspace*{0.5cm}\Large\color{blue} \pkg{Tool}: \textcolor{red}{Fast} \fontsize{9}{11}Nets}"
        )
        self.assertEqual(result["title"], "Tool: Fast Nets")

    def test_should_ignore_a_commented_out_title(self):
        result = read("% \\title{Old}\n\\title{New}")
        self.assertEqual(result["title"], "New")


class AuthorTests(unittest.TestCase):
    def test_should_split_on_and_and_drop_affiliation_lines_in_article_style(self):
        result = read(r"\author{Ann Lee \\ MIT \and Bo Chen\thanks{Equal.} \\ Stanford University}")
        self.assertEqual(result["authors"], ["Ann Lee", "Bo Chen"])

    def test_should_keep_printed_order_when_each_author_has_its_own_command(self):
        result = read(
            r"\author{Zed Young}\affiliation{MIT}\author{Amy Adams}\affiliation{CMU}",
            documentclass="revtex4-2",
        )
        self.assertEqual(result["authors"], ["Zed Young", "Amy Adams"])

    def test_should_skip_the_optional_argument_in_elsarticle_style(self):
        result = read(r"\author[1]{Ann Lee}\author[2]{Bo Chen}\address[1]{MIT}")
        self.assertEqual(result["authors"], ["Ann Lee", "Bo Chen"])

    def test_should_drop_llncs_institute_markers(self):
        result = read(r"\author{Ann Lee\inst{1} \and Bo Chen\inst{2}}\institute{MIT}")
        self.assertEqual(result["authors"], ["Ann Lee", "Bo Chen"])

    def test_should_split_a_comma_list_with_superscript_affiliations(self):
        result = read(r"\author{Ann Lee$^{1}$, Bo Chen$^{2,*}$, and Cy Diaz$^{1}$}")
        self.assertEqual(result["authors"], ["Ann Lee", "Bo Chen", "Cy Diaz"])

    def test_should_read_ieee_author_blocks(self):
        result = read(
            r"\author{\IEEEauthorblockN{Ann Lee, Bo Chen}\IEEEauthorblockA{MIT}"
            r"\and \IEEEauthorblockN{Cy Diaz}\IEEEauthorblockA{CMU}}",
            documentclass="IEEEtran",
        )
        self.assertEqual(result["authors"], ["Ann Lee", "Bo Chen", "Cy Diaz"])

    def test_should_read_icml_authors_from_icmlauthor(self):
        result = read(
            r"\icmltitle{T}\begin{icmlauthorlist}\icmlauthor{Ann Lee}{mit}\icmlauthor{Bo Chen}{cmu}\end{icmlauthorlist}"
        )
        self.assertEqual(result["authors"], ["Ann Lee", "Bo Chen"])

    def test_should_keep_diacritics_from_accent_commands(self):
        result = read(r"\author{Jos\'e Garc\'{\i}a \and Ren\'e Thom}")
        self.assertEqual(result["authors"][1], "René Thom")
        self.assertTrue(result["authors"][0].startswith("José"))

    def test_should_withhold_authors_when_an_affiliation_is_inline(self):
        result = read(r"\author{Ann Lee, Massachusetts Institute of Technology}")
        self.assertIsNone(result["authors"])
        self.assertIn("unrecognized author text", result["withheld"]["authors"])

    def test_should_withhold_authors_when_they_are_anonymized_placeholders(self):
        for author in (r"\author{Anonymous ECCV submission}", r"\author{XXX \and XXX}"):
            with self.subTest(author=author):
                self.assertIsNone(read(author)["authors"])

    def test_should_withhold_authors_when_there_is_no_author_command(self):
        result = read(r"\title{T}")
        self.assertIsNone(result["authors"])


class AbstractTests(unittest.TestCase):
    def test_should_read_the_abstract_environment_without_math_or_citations(self):
        result = read(r"\title{T}", r"\begin{abstract}We bound $x^2$ as in \cite{a}. It holds.\end{abstract}")
        self.assertEqual(result["abstract"], "We bound as in . It holds.")

    def test_should_keep_paragraph_breaks_in_the_abstract(self):
        result = read(r"\title{T}", "\\begin{abstract}First part.\n\nSecond part.\\end{abstract}")
        self.assertEqual(result["abstract"], "First part.\n\nSecond part.")

    def test_should_read_the_abstract_command_when_there_is_no_environment(self):
        result = read(r"\abstract{A short summary.}\title{T}")
        self.assertEqual(result["abstract"], "A short summary.")

    def test_should_withhold_the_abstract_when_the_paper_has_none(self):
        result = read(r"\title{T}", "Body text.")
        self.assertIsNone(result["abstract"])
        self.assertIn("abstract", result["withheld"])

    def test_should_drop_keywords_and_classification_typed_inside_the_abstract(self):
        result = read(r"\title{T}", "\\begin{abstract}We prove a bound.\n\nKey words: models, methods. AMS MSC 2010: 65M70.\\end{abstract}")
        self.assertEqual(result["abstract"], "We prove a bound.")
        result = read(r"\title{T}", r"\begin{abstract}We prove a bound. \keywords{billiards, loci} \subclass{37-40}\end{abstract}")
        self.assertEqual(result["abstract"], "We prove a bound.")
        result = read(r"\title{T}", "\\begin{abstract}We prove a bound.\n\nMSC 37-40, 51N20\\end{abstract}")
        self.assertEqual(result["abstract"], "We prove a bound.")

    def test_should_keep_an_abstract_that_mentions_keywords_in_prose(self):
        result = read(r"\title{T}", r"\begin{abstract}We rank keywords by salience.\end{abstract}")
        self.assertEqual(result["abstract"], "We rank keywords by salience.")

    def test_should_drop_display_math_from_the_abstract(self):
        result = read(r"\title{T}", r"\begin{abstract}Show \[ a=b \] and $$c$$ done.\end{abstract}")
        self.assertEqual(result["abstract"], "Show and done.")


if __name__ == "__main__":
    unittest.main()
