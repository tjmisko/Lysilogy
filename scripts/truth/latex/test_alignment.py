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
    def test_should_distinguish_inventory_support_when_standard_math_is_still_unrenderable(self):
        source = document(r'\begin{figure}\caption{A complete independ\"ent figure caption.}\end{figure}\begin{equation}x\in A+\tilde{x}\end{equation}', r'\setlength{\parindent}{0pt}\setlength{\textwidth}{10cm}')
        parsed = parse_project({'main.tex': source})
        result = align_paper(parsed, {'text': 'A complete independent figure caption. x in A plus x'})
        self.assertFalse(parsed['coverage']['unsupported_source_semantics'])
        self.assertTrue(result['metric_eligibility']['O1'])
        self.assertFalse(result['metric_eligibility']['O3'])
        self.assertIn('in', parsed['objects'][1]['unsupported_commands'])
        self.assertIn('tilde', parsed['objects'][1]['unsupported_commands'])

    def test_should_retain_unknown_hooks_when_an_ordinary_looking_command_can_hide_objects(self):
        for command in (r'\captionof{table}{A hidden table}', r'\phantom{Hidden contents}', r'\customhook'):
            parsed = parse_project({'main.tex': document(command + r'\begin{figure}\caption{A complete independent figure caption.}\end{figure}')})
            self.assertTrue(parsed['coverage']['unsupported_source_semantics'])
            self.assertFalse(align_paper(parsed, {'text': 'A complete independent figure caption.'})['accepted'])

    def test_should_withhold_semantic_math_alphabets_when_a_plain_text_collision_exists(self):
        for command in ('mathbb', 'mathcal', 'mathbf', 'mathsf', 'mathrm', 'mathit', 'operatorname'):
            source = document(r'\begin{equation}' + chr(92) + command + r'{R}+constant=12345\end{equation}')
            parsed = parse_project({'main.tex': source})
            result = align_paper(parsed, {'text': 'ℝ+constant=12345 Other prose R+constant=12345'})
            with self.subTest(command=command):
                self.assertIn('unverified_math_alphabet:' + command, parsed['objects'][0]['unsupported_commands'])
                self.assertFalse(result['metric_eligibility']['O3'])
        prose = parse_project({'main.tex': document(r'\begin{theorem}\textbf{All elements have a complete bounded representation.}\end{theorem}')})
        self.assertTrue(align_paper(prose, {'text': 'All elements have a complete bounded representation.'})['metric_eligibility']['O5'])

    def test_should_fold_the_mu_encoding_alias_when_math_case_and_script_semantics_stay_distinct(self):
        aligner = TextAlignment({'text': 'longvariable+µ=constant'})
        self.assertIsNotNone(aligner.unique('longvariable+μ=constant', math=True)[0])
        for different in ('longvariable+Μ=constant', 'longvariable-μ=constant', 'longvariable+μ²=constant'):
            self.assertIsNone(aligner.unique(different, math=True)[0])

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

    def test_should_withhold_inventory_when_used_macros_hide_objects_or_transitive_links(self):
        visible = r"\begin{theorem}A visible distinctive theorem.\end{theorem}"
        for definitions, invocation in ((r"\newcommand{\hidden}{\begin{theorem}A hidden distinctive theorem.\end{theorem}}", r"\hidden"),
                                        (r"\newcommand{\inner}[1]{\cite{#1}}\newcommand{\outer}[1]{\inner{#1}}", r"\outer{one}"),
                                        (r"\newcommand{\inner}[1]{\ref{#1}}\newcommand{\outer}[1]{\inner{#1}}", r"\outer{one}")):
            with self.subTest(definitions=definitions):
                parsed = parse_project({"main.tex": document(visible + invocation, definitions)})
                aligned = align_paper(parsed, {"text": "A visible distinctive theorem. A hidden distinctive theorem. [1]"})
                self.assertFalse(aligned["accepted"])
                self.assertTrue(parsed["coverage"]["unsupported_source_semantics"])

    def test_should_withhold_conditional_truth_when_false_branch_text_appears_as_ordinary_prose(self):
        source = document(r"\iffalse\begin{theorem}A distinctive hidden theorem.\end{theorem}\fi A distinctive hidden theorem.")
        result = align_paper(parse_project({"main.tex": source}), {"text": "A distinctive hidden theorem."})
        self.assertFalse(result["accepted"])
        self.assertIn("control_flow:iffalse", result["coverage"]["unsupported_source_semantics"])

    def test_should_withhold_redefined_macros_when_scoping_can_change_object_text(self):
        source = document(r"\begin{theorem}\term\end{theorem}\renewcommand{\term}{A second distinctive statement.}\begin{theorem}\term\end{theorem}",
                          r"\newcommand{\term}{A first distinctive statement.}")
        result = align_paper(parse_project({"main.tex": source}), {"text": "A first distinctive statement. A second distinctive statement."})
        self.assertFalse(result["accepted"])
        self.assertEqual(result["coverage"]["unsupported_source_semantics"]["redefined_macro:term"], 2)
        self.assertTrue(result["duplicate_span_claims"])

    def test_should_keep_tagged_single_equations_when_a_starred_environment_has_an_explicit_tag(self):
        source = document(r"\begin{equation*}abcdefghi=12345\tag{A}\label{eq:a}\end{equation*}")
        parsed = parse_project({"main.tex": source})
        self.assertEqual(len(parsed["objects"]), 1)
        self.assertEqual(parsed["objects"][0]["labels"], ["eq:a"])
        self.assertTrue(align_paper(parsed, {"text": "abcdefghi=12345 (A)"})["accepted"])

    def test_should_withhold_script_binding_when_flat_pdf_text_cannot_verify_tex_groups(self):
        for formula in (r"x^{ab}+constant=0", r"x^a b+constant=0", r"x_{ab}+constant=0"):
            parsed = parse_project({"main.tex": document(r"\begin{equation}" + formula + r"\end{equation}")})
            result = align_paper(parsed, {"text": "x^ab+constant=0 x_ab+constant=0"})
            self.assertFalse(result["accepted"])
            self.assertEqual(result["objects"], [])
            self.assertEqual(parsed["objects"][0]["unsupported_commands"], {"unverified_script_binding": 1})

    def test_should_withhold_low_level_macro_truth_when_definitions_hide_structure(self):
        visible = r"\begin{theorem}A visible distinctive theorem.\end{theorem}"
        for kind in ('def', 'gdef', 'edef', 'xdef'):
            definition = '\\' + kind + r'\hidden{\begin{theorem}Hidden theorem content.\end{theorem}}'
            parsed = parse_project({'main.tex': document(visible + r'\outer', definition + r'\newcommand{\outer}{\hidden}')})
            self.assertFalse(align_paper(parsed, {'text': 'A visible distinctive theorem. Hidden theorem content.'})['accepted'])
            self.assertIn('unsupported_definition:' + kind + ':hidden', parsed['coverage']['unsupported_source_semantics'])

    def test_should_withhold_redefined_environments_when_their_body_is_not_interpreted(self):
        source = document(r'\begin{theorem}A visible distinctive theorem.\end{theorem}', r'\renewenvironment{theorem}{\begin{figure}}{\end{figure}}')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'A visible distinctive theorem.'})
        self.assertFalse(result['accepted'])

    def test_should_withhold_literal_code_truth_when_code_contains_fake_tex_objects(self):
        visible = ''.join(r'\begin{theorem}Distinct actual theorem number ' + str(number) + r'.\end{theorem}' for number in range(20))
        code = r'\begin{lstlisting}\begin{theorem}This fake theorem is literal code.\end{theorem}\end{lstlisting}'
        text = ' '.join('Distinct actual theorem number ' + str(number) + '.' for number in range(20)) + ' This fake theorem is literal code.'
        result = align_paper(parse_project({'main.tex': document(visible + code)}), {'text': text})
        self.assertFalse(result['accepted'])
        self.assertIn('literal_environment:lstlisting', result['coverage']['unsupported_source_semantics'])

    def test_should_withhold_incomplete_text_when_unknown_commands_can_add_meaningful_words(self):
        for body in (r'\begin{theorem}\emphclaim{All elements have a unique bounded representation.}\end{theorem}',
                     r'\begin{figure}\caption{\emphclaim{All elements have a unique bounded representation.}}\end{figure}',
                     r'\begin{thebibliography}{9}\bibitem{one}\emphclaim{All elements have a unique bounded representation.}\end{thebibliography}'):
            parsed = parse_project({'main.tex': document(body, r'\usepackage{local}'), 'local.sty': r'\newcommand{\emphclaim}[1]{Not #1}'})
            result = align_paper(parsed, {'text': 'Not All elements have a unique bounded representation.'})
            self.assertFalse(result['accepted'])
            self.assertEqual(result['objects'] + result['entries'], [])

    def test_should_withhold_link_context_when_unresolved_commands_can_change_its_meaning(self):
        parsed, index = fixture()
        parsed['links'][0]['unsupported_context_commands'] = {'unknown': 1}
        result = align_paper(parsed, index)
        self.assertEqual(result['mentions'], [])
        self.assertFalse(result['bibliography_eligible'])

    def test_should_admit_complete_figure_truth_when_a_separate_equation_kind_cannot_align(self):
        source = document(r'\begin{figure}\caption{A complete independently aligned caption.}\end{figure}\begin{equation}\frac{abcdefghi}{12345}\end{equation}')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'A complete independently aligned caption. abcdefghi=12345'})
        self.assertTrue(result['accepted'])
        self.assertTrue(result['metric_eligibility']['O1'])
        self.assertFalse(result['metric_eligibility']['O2'])
        self.assertFalse(result['metric_eligibility']['O3'])
        self.assertEqual(result['alignment']['overall_completeness'], .5)
        self.assertEqual(result['alignment']['quality'], 1)
        self.assertEqual(result['kind_coverage']['equation']['expected'], 1)
        self.assertEqual(result['kind_coverage']['equation']['aligned'], 0)

    def test_should_keep_the_whole_kind_denominator_when_one_of_its_objects_is_missing(self):
        source = document(r'\begin{figure}\caption{First distinctive figure caption.}\end{figure}\begin{figure}\caption{Second distinctive figure caption.}\end{figure}\begin{theorem}A fully aligned distinctive statement.\end{theorem}')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'First distinctive figure caption. A fully aligned distinctive statement.'})
        self.assertTrue(result['accepted'])
        self.assertFalse(result['metric_eligibility']['O1'])
        self.assertTrue(result['metric_eligibility']['O5'])
        self.assertEqual(result['kind_coverage']['figure']['expected'], 2)
        self.assertEqual(result['kind_coverage']['figure']['aligned'], 1)
        self.assertNotIn('figure', result['eligible_kinds'])

    def test_should_retain_exhaustive_bibliography_cohorts_when_unrelated_math_is_unaligned(self):
        parsed, index = fixture()
        extra = parse_project({'main.tex': document(r'\begin{equation}\unknownop abcdefghi=12345\end{equation}')})['objects'][0]
        parsed['objects'].append(extra)
        result = align_paper(parsed, index)
        self.assertTrue(result['bibliography_eligible'])
        self.assertTrue(result['metric_eligibility']['O8'])
        self.assertTrue(result['metric_eligibility']['O10'])
        self.assertFalse(result['metric_eligibility']['O3'])
        self.assertEqual(result['alignment']['overall_completeness'], 2 / 3)

    def test_should_withhold_proof_link_metrics_when_a_proof_has_no_independent_destination(self):
        source = document(r'\begin{theorem}A completely aligned distinctive theorem.\end{theorem}\begin{proof}A completely aligned distinctive proof.\end{proof}')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'A completely aligned distinctive theorem. A completely aligned distinctive proof.'})
        self.assertTrue(result['metric_eligibility']['O5'])
        self.assertFalse(result['metric_eligibility']['O6'])

    def test_should_keep_citation_denominators_when_another_metric_cohort_is_complete(self):
        parsed, index = fixture()
        parsed['links'].append({**parsed['links'][0], 'context_before': 'Another distinctive context that never occurs', 'context_after': 'and its complete unmatched continuation'})
        figure = parse_project({'main.tex': document(r'\begin{figure}\caption{The distinctive independently aligned figure.}\end{figure}')})['objects'][0]
        parsed['objects'].append(figure)
        index['text'] += '\nThe distinctive independently aligned figure.'
        result = align_paper(parsed, index)
        self.assertTrue(result['metric_eligibility']['O1'])
        self.assertFalse(result['bibliography_eligible'])
        self.assertFalse(result['metric_eligibility']['O10'])
        self.assertEqual(len(result['excluded_citations']), 1)

    def test_should_withhold_inventory_when_a_local_style_can_inject_hidden_objects(self):
        source = document(r'\begin{theorem}A visible distinctive theorem.\end{theorem}', r'\usepackage{local}')
        files = {'main.tex': source, 'local.sty': r'\AtBeginDocument{\begin{theorem}An injected hidden theorem.\end{theorem}}'}
        result = align_paper(parse_project(files), {'text': 'An injected hidden theorem. A visible distinctive theorem.'})
        self.assertFalse(result['accepted'])
        self.assertFalse(any(result['metric_eligibility'].values()))
        self.assertIn('uninterpreted_local_style:local.sty', result['coverage']['unsupported_source_semantics'])

    def test_should_withhold_object_links_when_unsupported_reference_variants_add_occurrences(self):
        source = document(r'\begin{theorem}\label{a}A first distinctive theorem.\end{theorem}\begin{theorem}\label{b}A second distinctive theorem.\end{theorem} '
                          r'The independently named statement is \ref{a} with a uniquely matching context.' + '\n\n' + r'Also \crefrange{a}{b}.')
        parsed = parse_project({'main.tex': source})
        text = 'A first distinctive theorem. A second distinctive theorem. The independently named statement is 1 with a uniquely matching context. Also Theorems1–2.'
        result = align_paper(parsed, {'text': text})
        self.assertTrue(result['metric_eligibility']['O5'])
        self.assertEqual(len(result['references']), 1)
        self.assertFalse(result['metric_eligibility']['O4'])
        self.assertIn('crefrange', result['coverage']['unsupported_reference_commands'])

    def test_should_preserve_inline_math_when_operators_inside_prose_change_meaning(self):
        for environment, opening, closing in (('theorem', '', ''), ('figure', r'\caption{', '}'), ('algorithm', '', '')):
            for authored, actual in (('x+y', 'x-y'), ('x=y', 'xy'), ('Alpha=Beta', 'alpha=beta'), ('x^{ab}', 'x^a b')):
                body = 'The sufficient condition $' + authored + '$ ensures a unique bounded outcome.'
                source = document('\\begin{' + environment + '}' + opening + body + closing + '\\end{' + environment + '}')
                parsed = parse_project({'main.tex': source})
                result = align_paper(parsed, {'text': 'The sufficient condition ' + actual + ' ensures a unique bounded outcome.'})
                with self.subTest(environment=environment, authored=authored):
                    self.assertFalse(result['accepted'])
                    self.assertEqual(result['objects'], [])

    def test_should_preserve_inline_math_when_bibliography_text_contains_a_formula(self):
        source = document(r'\begin{thebibliography}{9}\bibitem{one}Smith. A result about $x+y$ with unique consequences. 2020.\end{thebibliography}')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'Smith. A result about x-y with unique consequences. 2020.'})
        self.assertFalse(result['bibliography_eligible'])
        self.assertEqual(result['entries'], [])

    def test_should_preserve_inline_math_when_selecting_a_printed_link_context(self):
        parsed, index = fixture()
        parsed['links'][0]['context_before'] = 'The sufficient condition x+y has a unique meaning'
        parsed['links'][0]['context_after'] = ''
        parsed['links'][0]['math_context'] = True
        index['text'] = 'The sufficient condition x-y has a unique meaning [1]\n[1] Smith, A. A uniquely identifiable synthetic study. 2020.'
        result = align_paper(parsed, index)
        self.assertEqual(result['mentions'], [])
        self.assertFalse(result['bibliography_eligible'])

    def test_should_keep_supported_empty_kinds_when_negative_papers_can_reveal_false_positives(self):
        body = 'This independently aligned plain document contains ordinary prose only.'
        result = align_paper(parse_project({'main.tex': document(body)}), {'text': body})
        for metric in ('O1', 'O3', 'O5', 'O7', 'O8', 'O10'):
            self.assertTrue(result['metric_eligibility'][metric], metric)
        self.assertTrue(result['alignment']['empty_inventory_document_verified'])
        self.assertEqual(result['objects'] + result['entries'] + result['mentions'], [])
        self.assertEqual(result['kind_coverage']['figure']['expected'], 0)

    def test_should_withhold_false_empty_inventories_when_unknown_commands_can_create_objects(self):
        source = document(r'This distinctive plain paragraph is followed by \mysteryobjects.')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'This distinctive plain paragraph is followed by an injected figure.'})
        self.assertFalse(any(result['metric_eligibility'].values()))
        self.assertIn('unknown_inventory_command:mysteryobjects', result['coverage']['unsupported_source_semantics'])

    def test_should_withhold_empty_bibliography_truth_when_an_unsupported_builder_prints_references(self):
        source = document(r'A distinctive introductory paragraph. \printbibliography')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'A distinctive introductory paragraph. References A. Author. A title.'})
        self.assertFalse(result['metric_eligibility']['O8'])
        self.assertFalse(result['metric_eligibility']['O10'])

    def test_should_reject_unverified_negative_papers_when_even_document_prose_does_not_match(self):
        source = document('The independently supplied source document has a distinctive sentence.')
        result = align_paper(parse_project({'main.tex': source}), {'text': 'An entirely different document and unknown content.'})
        self.assertFalse(result['accepted'])
        self.assertFalse(result['alignment']['empty_inventory_document_verified'])

    def test_should_keep_unrelated_cohorts_when_an_unnumbered_equation_label_is_uncertain(self):
        for environment, suppression in [('align*', ''), ('gather*', ''), ('eqnarray*', ''), ('equation*', ''), ('multline*', ''), ('equation', r'\nonumber'), ('multline', r'\notag')]:
            source = document(r'\begin{figure}\caption{A complete independent visual caption.}\end{figure}' + r'\begin{' + environment + r'}abcdefghi=12345\label{ambiguous}' + suppression + r'\end{' + environment + '}')
            result = align_paper(parse_project({'main.tex': source}), {'text': 'A complete independent visual caption. abcdefghi=12345'})
            with self.subTest(environment=environment, suppression=suppression):
                self.assertTrue(result['metric_eligibility']['O1'])
                self.assertFalse(result['metric_eligibility']['O3'])
                self.assertEqual(result['kind_coverage']['equation']['expected'], 1)
                self.assertEqual(result['kind_coverage']['equation']['aligned'], 0)
                self.assertIn('ambiguous_equation_numbering', result['excluded_objects'][0]['unsupported_commands'])

    def test_should_withhold_equation_negatives_when_transitive_macros_change_numbering(self):
        for command in (r'\tag{A}', r'\nonumber', r'\notag'):
            source = document(r'\begin{figure}\caption{An independently complete figure caption.}\end{figure}\begin{equation*}abcdefghi=12345\outer\end{equation*}', r'\newcommand{\inner}{' + command + r'}\newcommand{\outer}{\inner}')
            result = align_paper(parse_project({'main.tex': source}), {'text': 'An independently complete figure caption. abcdefghi=12345 (A)'})
            with self.subTest(command=command):
                self.assertFalse(result['metric_eligibility']['O3'])
                self.assertFalse(result['accepted'])


if __name__ == "__main__":
    unittest.main()
