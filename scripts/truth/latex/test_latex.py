"""Offline source/alignment fixtures; synthetic examples never become K1 truth."""
import gzip
import io
import tarfile
import unittest

from archive import Limits, UnsupportedSource, read_archive
from parser import parse_project
from tex import Renderer, comments, expand_project, group


def document(body, preamble=""):
    return "\\documentclass{article}\n" + preamble + "\n\\begin{document}\n" + body + "\n\\end{document}"


def tar(members):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as stream:
        for name, value, kind in members:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.size = len(value) if kind == tarfile.REGTYPE else 0
            stream.addfile(member, io.BytesIO(value) if kind == tarfile.REGTYPE else None)
    return gzip.compress(output.getvalue())


class SourceTests(unittest.TestCase):
    def test_should_omit_only_literal_penalty_parameters_when_bibliography_spacing_is_deposited(self):
        self.assertEqual(Renderer().plain(r'12:\penalty0 2121--2159'), '12: 2121–2159')
        self.assertEqual(Renderer().plain(r'12\penalty-100 (1)'), '12 (1)')
        with self.assertRaisesRegex(UnsupportedSource, 'nonliteral TeX penalty'):
            Renderer().plain(r'12:\penalty\custom 2121--2159')

    def test_should_report_unavailable_tex_when_the_deposited_source_is_a_pdf(self):
        for raw in (b'%PDF-1.4\n\x00binary', gzip.compress(b'%PDF-1.4\n\x00binary')):
            with self.subTest(compressed=raw.startswith(b'\x1f\x8b')), self.assertRaisesRegex(UnsupportedSource, 'source is a PDF; TeX source is unavailable'):
                read_archive(raw)

    def test_should_retain_unrenderable_entries_when_a_macro_consumes_undeclared_caller_tokens(self):
        source = document(r'\begin{figure}\caption{A complete independent visual caption.}\end{figure}\begin{thebibliography}{9}\bibitem{one}\wrapper{A source title.}\end{thebibliography}', r'\newcommand{\wrapper}{\textbf}')
        parsed = parse_project({'main.tex': source})
        self.assertEqual(len(parsed['entries']), 1)
        self.assertTrue(parsed['entries'][0]['unsupported_commands'])
        self.assertEqual(parsed['entries'][0]['text'], '')
        self.assertEqual(parsed['objects'][0]['text'], 'A complete independent visual caption.')
        self.assertTrue(parsed['entries'][0]['source_members'])

    def test_should_propagate_resource_limits_when_an_unrenderable_row_exhausts_the_paper_budget(self):
        source = document(r'\begin{theorem}\loop\end{theorem}', r'\newcommand{\loop}{\loop}')
        with self.assertRaisesRegex(UnsupportedSource, 'recursion'):
            parse_project({'main.tex': source})

    def test_should_read_a_standalone_document_when_a_gzip_source_contains_no_tar(self):
        source = document("A single paper.").encode()
        files, evidence = read_archive(gzip.compress(source))
        self.assertEqual(files, {"main.tex": source.decode()})
        self.assertEqual(evidence[0]["bytes"], len(source))

    def test_should_accept_root_directories_when_a_tar_uses_dot_relative_members(self):
        files, _ = read_archive(tar([("./", b"", tarfile.DIRTYPE), ("./main.tex", document("Safe.").encode(), tarfile.REGTYPE)]))
        self.assertEqual(list(files), ["main.tex"])

    def test_should_reject_unsafe_archives_when_members_escape_link_or_repeat_paths(self):
        for members in [
            [("../main.tex", b"x", tarfile.REGTYPE)], [("/main.tex", b"x", tarfile.REGTYPE)],
            [(".env", b"x", tarfile.REGTYPE)], [("main.tex", b"", tarfile.SYMTYPE)],
            [("main.tex", b"x", tarfile.REGTYPE), ("./main.tex", b"y", tarfile.REGTYPE)],
        ]:
            with self.subTest(members=members), self.assertRaises(UnsupportedSource):
                read_archive(tar(members))

    def test_should_bound_decompression_when_a_small_archive_expands_past_its_limit(self):
        with self.assertRaisesRegex(UnsupportedSource, "expanded"):
            read_archive(gzip.compress(b"A" * 5000), Limits(expanded_bytes=1024))
        with self.assertRaisesRegex(UnsupportedSource, "member count"):
            read_archive(tar([("a.tex", b"a", tarfile.REGTYPE), ("b.tex", b"b", tarfile.REGTYPE)]), Limits(members=1))

    def test_should_expand_input_files_when_parsing_a_multifile_source(self):
        files = {"main.tex": document("\\input{sections/body}"), "sections/body.tex": "Before \\input{nested} after", "sections/nested.tex": "nested text"}
        result = expand_project(files)
        self.assertIn("Before nested text after", result.text)
        self.assertEqual(result.coverage["expanded_files"], sorted(files))
        at = result.text.index("nested text")
        self.assertEqual(result.origins(at, at + 11), [{"path": "sections/nested.tex", "start": 0, "end": 11}])

    def test_should_reject_ambiguous_mains_when_two_documents_are_deposited(self):
        with self.assertRaisesRegex(UnsupportedSource, "main file is ambiguous"):
            expand_project({"a.tex": document("A"), "b.tex": document("B")})

    def test_should_reject_dynamic_or_cyclic_includes_when_expansion_is_not_reproducible(self):
        for files in ({"main.tex": document("\\input{main}")}, {"main.tex": document("\\input{../outside}")}, {"main.tex": document("\\input{\\chosen}")}, {"main.tex": document("\\input{missing}")}):
            with self.assertRaises(UnsupportedSource):
                expand_project(files)

    def test_should_keep_definitions_inert_when_their_unused_body_contains_input_or_environments(self):
        raw = document("Visible.", r"\newcommand{\unused}{\input{missing}\begin{figure}\end{figure}}")
        result = parse_project({"main.tex": raw})
        self.assertEqual(result["objects"], [])

    def test_should_preserve_comments_when_an_escaped_percent_precedes_a_real_comment(self):
        raw = "A \\% B % hidden \\input{bad}\nnext"
        clean = comments(raw)
        self.assertEqual(len(clean), len(raw))
        self.assertIn("A \\% B", clean)
        self.assertNotIn("hidden", clean)
        self.assertTrue(clean.endswith("\nnext"))

    def test_should_bound_groups_and_macros_when_source_nesting_or_expansion_recurses(self):
        with self.assertRaisesRegex(UnsupportedSource, "nesting"):
            group("{" * 65 + "x" + "}" * 65, 0)
        renderer = Renderer(r"\newcommand{\loop}{\loop}")
        with self.assertRaisesRegex(UnsupportedSource, "recursion"):
            renderer.plain(r"\loop")
        with self.assertRaisesRegex(UnsupportedSource, "byte bound"):
            expand_project({"main.tex": document("α" * 100)}, Limits(text_bytes=150))

    def test_should_map_custom_newtheorem_when_a_source_defines_its_statement_environment(self):
        raw = document(r"\begin{thm}\label{thm:one}Every item is bounded.\end{thm}", r"\newtheorem{thm}{Theorem}")
        result = parse_project({"main.tex": raw})
        self.assertEqual(result["objects"][0]["kind"], "statement")
        self.assertEqual(result["objects"][0]["statement_type"], "Theorem")
        self.assertEqual(result["objects"][0]["text"], "Every item is bounded.")

    def test_should_link_a_proof_when_its_optional_heading_names_a_statement_label(self):
        raw = document(r"\begin{thm}\label{thm:one}A claim.\end{thm}\begin{proof}[Proof of Theorem~\ref{thm:one}]A derivation.\end{proof}", r"\newtheorem{thm}{Theorem}")
        result = parse_project({"main.tex": raw})
        self.assertEqual(result["objects"][1]["proof_targets"], ["object:thm:one"])

    def test_should_collect_each_e1_kind_when_environments_and_bibliography_are_present(self):
        raw = document(r"""
\begin{figure}\caption{An independent caption.}\label{fig:a}\end{figure}
\begin{table}\caption{An independent table.}\end{table}
\begin{equation}x=1\label{eq:a}\end{equation}
\begin{lemma}A useful property.\end{lemma}
\begin{proof}Its proof.\end{proof}
\begin{algorithm}Initialize and iterate.\end{algorithm}
See \ref{fig:a} and \eqref{eq:a}; evidence \cite{source}.
\begin{thebibliography}{9}\bibitem{source}A. Author. A real-looking synthetic title. 2020.\end{thebibliography}
""")
        result = parse_project({"main.tex": raw})
        self.assertEqual({row["kind"] for row in result["objects"]}, {"figure", "table", "equation", "statement", "proof", "algorithm"})
        self.assertEqual(result["entries"][0]["id"], "source")
        self.assertEqual(len(result["links"]), 3)
        self.assertEqual(result["entries"][0]["field_labels"], {})

    def test_should_withhold_bibtex_only_fields_when_printed_field_boundaries_are_not_explicit(self):
        files = {"main.tex": document(r"\bibliography{sources}"), "main.bbl": r"\begin{thebibliography}{9}\bibitem{one}A. Author. A deposited title. 2020.\end{thebibliography}",
                 "sources.bib": '@article{one, author={Author, A. and Second, B.},title={A deposited title},year={2020}}'}
        result = parse_project(files)
        self.assertEqual(result["entries"][0]["field_labels"], {})
        self.assertEqual(len(result["entries"][0]["field_conflicts"]), 3)

    def test_should_withhold_substring_fields_when_printed_negation_and_year_roles_differ(self):
        files = {"main.tex": document(r"\bibliography{used}"), "main.bbl": r"\begin{thebibliography}{9}\bibitem{one}Smith. Not all events of 1999 were recorded. 2021.\end{thebibliography}",
                 "used.bib": '@article{one, title={All events of 1999 were recorded},year={1999}}'}
        row = parse_project(files)["entries"][0]
        self.assertEqual(row["field_labels"], {})
        self.assertEqual(len(row["field_conflicts"]), 2)

    def test_should_take_explicit_printed_fields_when_bibliography_markup_establishes_their_roles(self):
        source = document(r"\begin{thebibliography}{9}\bibitem{one}\bibinfo{author}{Smith}. \bibinfo{title}{Not all events of 1999 were recorded}. \bibinfo{year}{2021}.\end{thebibliography}")
        row = parse_project({'main.tex': source})['entries'][0]
        self.assertEqual(row['field_labels'], {'title': 'Not all events of 1999 were recorded', 'year': '2021', 'first_author': 'Smith'})

    def test_should_take_one_explicit_author_when_a_bibfield_wrapper_contains_multiple_people(self):
        source = document(r"\begin{thebibliography}{9}\bibitem{one}\bibfield{author}{\bibinfo{author}{A. Smith} and \bibinfo{author}{B. Jones}}. A title.\end{thebibliography}")
        self.assertEqual(parse_project({'main.tex': source})['entries'][0]['field_labels'], {'first_author': 'A. Smith'})
        ambiguous = source.replace(r'\bibinfo{author}{A. Smith} and \bibinfo{author}{B. Jones}', 'A. Smith and B. Jones')
        self.assertEqual(parse_project({'main.tex': ambiguous})['entries'][0]['field_labels'], {})

    def test_should_not_promote_later_authors_when_the_first_author_slot_is_unknown(self):
        for authors in ('A. One and B. Two', 'A. ONE AND B. TWO'):
            source = document(r'\begin{thebibliography}{9}\bibitem{one}\bibinfo{author}{' + authors + r'}\bibinfo{author}{C. Three}. A title.\end{thebibliography}')
            self.assertEqual(parse_project({'main.tex': source})['entries'][0]['field_labels'], {})

    def test_should_withhold_stale_fields_when_active_bibtex_disagrees_with_rendered_evidence(self):
        files = {"main.tex": document(r"\bibliography{used}"), "main.bbl": r"\begin{thebibliography}{9}\bibitem{one}Smith. Correct unique published title. 2021.\end{thebibliography}",
                 "used.bib": '@article{one, author={Wrong Author},title={An unrelated manuscript},year={1999}}'}
        row = parse_project(files)["entries"][0]
        self.assertEqual(row["field_labels"], {})
        self.assertEqual(len(row["field_conflicts"]), 3)

    def test_should_ignore_unrelated_databases_when_an_unused_bibtex_key_matches(self):
        files = {"main.tex": document(r"\bibliography{used}"), "main.bbl": r"\begin{thebibliography}{9}\bibitem{one}Smith. Correct unique published title. 2021.\end{thebibliography}",
                 "used.bib": '', "unrelated.bib": '@article{one, author={Wrong Author},title={An unrelated manuscript},year={1999}}'}
        row = parse_project(files)["entries"][0]
        self.assertEqual(row["field_labels"], {})
        self.assertEqual(row["field_conflicts"], [])

    def test_should_ignore_an_inert_label_when_an_unused_macro_is_defined_inside_a_theorem(self):
        source = document(r"\begin{theorem}\newcommand{\unused}{\label{ghost}}\label{real}A visible distinctive statement.\end{theorem}")
        parsed = parse_project({"main.tex": source})
        self.assertEqual(parsed["objects"][0]["labels"], ["real"])
        self.assertNotIn("ghost", parsed["label_targets"])

    def test_should_record_unknown_commands_when_rendering_an_unsupported_presentation_macro(self):
        renderer = Renderer()
        self.assertEqual(renderer.plain(r"A \unknown{literal} word"), "A literal word")
        self.assertEqual(renderer.unsupported, {"unknown": 1})
        self.assertEqual(renderer.plain(r"Garc\'{i}a and \textit{proof}"), "García and proof")

    def test_should_accept_a_single_token_when_known_formatting_has_no_braces(self):
        renderer = Renderer()
        self.assertEqual(renderer.plain(r"\mathbf a + \mathbb N + \mathcal D"), "a + N + D")
        self.assertEqual(renderer.plain(r"\mathbf\alpha"), "α")
        self.assertEqual(renderer.unsupported, {})

    def test_should_accept_single_tokens_when_a_declared_macro_uses_unbraced_arguments(self):
        renderer = Renderer(r'\newcommand{\format}[1]{\mathbf{#1}}')
        self.assertEqual(renderer.plain(r'\format a + \format\alpha'), 'a + α')

    def test_should_assign_nested_labels_when_a_statement_contains_a_numbered_equation(self):
        raw = document(r"\begin{theorem}\label{thm:a}A statement \begin{equation}x=1\label{eq:a}\end{equation}\end{theorem}")
        rows = parse_project({"main.tex": raw})["objects"]
        self.assertEqual(rows[0]["labels"], ["thm:a"])
        self.assertEqual(rows[1]["labels"], ["eq:a"])

    def test_should_bound_total_rendering_when_many_objects_repeat_one_macro(self):
        source = document(r"\begin{theorem}\term\end{theorem}" * 6, r"\newcommand{\term}{" + "a" * 500 + "}")
        with self.assertRaisesRegex(UnsupportedSource, "aggregate byte bound"):
            parse_project({"main.tex": source}, Limits(text_bytes=1200))

    def test_should_retain_source_links_when_partial_contexts_cut_balanced_authoring_commands(self):
        contexts = (r'\mbox{A distinctive cited statement \cite{one} with its continuation.}',
                    r'\subsubsection{A heading containing \cite{one}}',
                    r'A distinctive cited statement \cite{one} \emph{' + 'extended ' * 100 + '}')
        for context in contexts:
            source = document(r'\begin{figure}\caption{A separately complete figure caption.}\end{figure}' + context + r'\begin{thebibliography}{9}\bibitem{one}A unique complete bibliography entry.\end{thebibliography}')
            parsed = parse_project({'main.tex': source})
            with self.subTest(context=context):
                self.assertEqual(len(parsed['objects']), 1)
                self.assertEqual(len(parsed['entries']), 1)
                self.assertEqual(len(parsed['links']), 1)
                self.assertEqual(parsed['links'][0]['targets'], ['one'])
                self.assertIn('unbalanced_clipped_context', parsed['links'][0]['unsupported_context_commands'])
                self.assertTrue(parsed['links'][0]['source_members'])


if __name__ == "__main__":
    unittest.main()
