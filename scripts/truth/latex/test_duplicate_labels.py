"""Source naming failures retain occurrences, never hypothetical truth edges."""
from copy import deepcopy
import unittest
from unittest.mock import patch

from align import align_paper
from archive import Limits, UnsupportedSource
from parser import parse_project
from test_latex import document


def parse(body, preamble='', files=None, limits=Limits()):
    return parse_project({'main.tex': document(body, preamble), **(files or {})}, limits)


def equation(body, labels=''):
    return r'\begin{equation}' + labels + body + r'\end{equation}'


class DuplicateLabelTests(unittest.TestCase):
    def test_should_keep_both_source_objects_when_two_equations_share_a_label(self):
        parsed = parse(equation('alpha+beta=12345', r'\label{shared}')
                       + equation('gamma+delta=67890', r'\label{shared}')
                       + r'This distinctive comparison uses equation \ref{shared} in its conclusion.')
        objects = parsed['objects']
        self.assertEqual(len({row['id'] for row in objects}), 2)
        self.assertEqual({row['provisional_id'] for row in objects}, {'object:shared'})
        self.assertNotIn('shared', parsed['label_targets'])
        self.assertEqual(parsed['ambiguous_labels']['shared']['candidate_count'], 2)
        self.assertEqual(parsed['links'][0]['targets'], ['shared'])
        aligned = align_paper(parsed, {'text': 'alpha+beta=12345 gamma+delta=67890 This distinctive comparison uses equation 1 in its conclusion.', 'tokens': []})
        self.assertTrue(aligned['metric_eligibility']['O3'])
        self.assertFalse(aligned['metric_eligibility']['O4'])
        self.assertEqual(aligned['kind_coverage']['equation']['expected'], 2)
        self.assertEqual(aligned['references'], [])
        self.assertEqual(aligned['excluded_references'][0]['ambiguous_labels'], {'shared': 2})

    def test_should_preserve_unique_aliases_when_only_a_secondary_label_is_duplicated(self):
        parsed = parse(equation('alpha+beta=12345', r'\label{first}\label{shared}')
                       + equation('gamma+delta=67890', r'\label{second}\label{shared}'))
        self.assertEqual([row['id'] for row in parsed['objects']], ['object:first', 'object:second'])
        self.assertEqual(parsed['label_targets'], {'first': 'object:first', 'second': 'object:second'})
        self.assertEqual(len(parsed['label_occurrences']), 4)

    def test_should_withhold_resolution_when_repeated_commands_share_one_object(self):
        for source in (r'\begin{table}\label{same}\caption{A distinctive complete table caption.}\label{same}\end{table}',
                       equation('abcdefghij=12345', r'\label{same}\label{same}')):
            with self.subTest(source=source):
                parsed = parse(source)
                self.assertEqual(len(parsed['objects']), 1)
                self.assertEqual(parsed['objects'][0]['id'], 'object:same')
                self.assertEqual(parsed['ambiguous_labels']['same']['candidate_object_ids'], ['object:same'])
                self.assertEqual(parsed['label_targets'], {})

    def test_should_record_object_section_and_item_claims_when_names_collide(self):
        for tail, role in ((r'\section{End}\label{shared}', 'section'),
                           (r'\begin{enumerate}\item\label{shared}A condition.\end{enumerate}', 'item'),
                           (r'\begin{subfigure}\label{shared}A panel.\end{subfigure}', 'subfigure')):
            with self.subTest(role=role):
                parsed = parse(equation('abcdefghij=12345', r'\label{shared}') + tail)
                self.assertEqual(parsed['ambiguous_labels']['shared']['roles'], sorted(['object', role]))
                self.assertNotIn('shared', parsed['label_targets'])

    def test_should_keep_nested_item_labels_separate_when_an_assumption_contains_a_list(self):
        parsed = parse(r'\begin{assumption}\label{whole}A complete assumption.\begin{enumerate}'
                       r'\item\label{first}One condition.\item\label{second}Another condition.'
                       r'\end{enumerate}\end{assumption}')
        self.assertEqual(parsed['label_targets'], {'whole': 'object:whole'})
        self.assertEqual([row['owner']['role'] for row in parsed['label_occurrences']], ['object', 'item', 'item'])
        self.assertLess(parsed['label_occurrences'][1]['owner']['end'], parsed['objects'][0]['source_span']['end'])

    def test_should_keep_explicit_proofs_unresolved_when_the_named_target_is_ambiguous(self):
        body = (r'\begin{theorem}\label{shared}The first theorem has a distinctive complete assertion.\end{theorem}'
                r'\begin{lemma}\label{shared}The second lemma states another complete assertion.\end{lemma}')
        parsed = parse(body + r'\begin{proof}[Proof of \ref{shared}]An explicit argument establishes the conclusion.\end{proof}')
        proof = parsed['objects'][-1]
        self.assertEqual(proof['proof_targets'], [None])
        self.assertEqual(proof['ambiguous_proof_target_labels'], {'shared': 2})
        self.assertNotIn('nearest', proof['proof_linkage'])
        index = {'text': ' '.join(row['text'] for row in parsed['objects']), 'tokens': []}
        self.assertFalse(align_paper(parsed, index)['metric_eligibility']['O6'])
        unnamed = parse(body + r'\begin{proof}An unnamed argument establishes the conclusion.\end{proof}')
        self.assertEqual(unnamed['objects'][-1]['proof_targets'], [unnamed['objects'][-2]['id']])
        self.assertIn('nearest preceding', unnamed['objects'][-1]['proof_linkage'])

    def test_should_retain_every_explicit_target_when_a_group_mixes_unique_and_ambiguous_labels(self):
        parsed = parse(r'\begin{theorem}\label{one}\label{shared}The first distinctive statement.\end{theorem}'
                       r'\begin{lemma}\label{shared}The second distinctive statement.\end{lemma}'
                       r'\begin{proof}[By \cref{one,shared}]The complete supporting argument.\end{proof}')
        self.assertEqual(parsed['objects'][-1]['proof_target_labels'], ['one', 'shared'])
        self.assertEqual(parsed['objects'][-1]['proof_targets'], ['object:one', None])

    def test_should_preserve_complete_kind_counts_when_an_unreferenced_duplicate_is_localized(self):
        parsed = parse(equation('alpha+beta=12345', r'\label{x}') + equation('gamma+delta=67890', r'\label{x}'))
        result = align_paper(parsed, {'text': 'alpha+beta=12345 gamma+delta=67890', 'tokens': []})
        self.assertEqual(result['aligned_objects_by_kind'], {'equation': 2})
        self.assertTrue(result['metric_eligibility']['O3'])
        self.assertTrue(result['metric_eligibility']['O5'])  # Independently supported absent kind.
        self.assertEqual(parsed['coverage']['unsupported_source_semantics'], {})

    def test_should_withhold_all_cohorts_when_distinct_sources_claim_one_native_span(self):
        parsed = parse(equation('abcdefghij=12345', r'\label{x}') + equation('abcdefghij=12345', r'\label{x}'))
        result = align_paper(parsed, {'text': 'abcdefghij=12345', 'tokens': []})
        self.assertEqual(result['duplicate_span_claims'][0]['claims'], 2)
        self.assertFalse(any(result['metric_eligibility'].values()))

    def test_should_preserve_execution_guards_when_duplicates_appear_in_unsupported_content(self):
        obj = equation('abcdefghij=12345', r'\label{x}')
        for body, preamble in ((obj + r'\iffalse' + obj + r'\fi', ''),
                               (obj + r'\hidden{' + obj + '}', r'\newcommand{\hidden}[1]{}'),
                               (obj + obj, r'\renewcommand{\label}[1]{}'),
                               (obj + r'\title{' + obj + '}', '')):
            with self.subTest(body=body, preamble=preamble):
                parsed = parse(body, preamble)
                self.assertTrue(parsed['coverage']['unsupported_source_semantics'])
                self.assertFalse(any(align_paper(parsed, {'text': 'abcdefghij=12345', 'tokens': []})['metric_eligibility'].values()))

    def test_should_omit_inert_definition_and_comment_claims_when_only_one_label_is_active(self):
        parsed = parse(equation('abcdefghij=12345', r'\label{x}') + '\n% \\label{x}\n',
                       r'\newcommand{\unused}{\label{x}}')
        self.assertEqual(len(parsed['label_occurrences']), 1)
        self.assertEqual(parsed['ambiguous_labels'], {})
        self.assertEqual(parsed['label_targets'], {'x': 'object:x'})

    def test_should_preserve_uncertain_numbering_when_starred_equations_repeat_labels(self):
        parsed = parse(r'\begin{equation*}\label{x}abcdefghij=12345\end{equation*}'
                       r'\begin{equation*}\label{x}klmnopqrst=67890\end{equation*}')
        self.assertEqual(len(parsed['objects']), 2)
        self.assertTrue(all(row['numbering_uncertain'] for row in parsed['objects']))
        self.assertFalse(align_paper(parsed, {'text': 'abcdefghij=12345 klmnopqrst=67890', 'tokens': []})['metric_eligibility']['O3'])

    def test_should_distinguish_repeated_include_occurrences_when_original_members_are_identical(self):
        included = equation('abcdefghij=12345', r'\label{x}')
        parsed = parse(r'\input{part}\input{part}', files={'part.tex': included})
        first, second = parsed['objects']
        self.assertEqual(first['source_members'], second['source_members'])
        self.assertNotEqual(first['source_span'], second['source_span'])
        self.assertNotEqual(first['id'], second['id'])
        self.assertEqual(parsed, parse(r'\input{part}\input{part}', files={'part.tex': included}))
        self.assertEqual([row['source_members'] for row in parsed['label_occurrences']],
                         [[{'path': 'part.tex', 'start': included.index(r'\label'), 'end': included.index(r'\label') + len(r'\label{x}')}]] * 2)

    def test_should_disambiguate_every_colliding_id_when_an_authored_label_matches_a_fallback(self):
        parsed = parse(equation('abcdefghij=12345') + equation('klmnopqrst=67890', r'\label{equation:1}'))
        self.assertEqual({row['provisional_id'] for row in parsed['objects']}, {'object:equation:1'})
        self.assertTrue(all(row['id'].startswith('object:source-occurrence:') for row in parsed['objects']))
        self.assertEqual(parsed['label_targets']['equation:1'], parsed['objects'][1]['id'])

    def test_should_refuse_residual_identity_collisions_when_a_digest_cannot_distinguish_occurrences(self):
        with patch('parser.sha256', return_value='0' * 64), self.assertRaisesRegex(UnsupportedSource, 'occurrence identity'):
            parse(equation('abcdefghij=12345') + equation('klmnopqrst=67890'))
        parsed = parse(equation('abcdefghij=12345'))
        parsed['objects'].append(deepcopy(parsed['objects'][0]))
        with self.assertRaisesRegex(UnsupportedSource, 'identities are duplicated'):
            align_paper(parsed, {'text': 'abcdefghij=12345', 'tokens': []})

    def test_should_bound_duplicate_evidence_when_many_commands_share_one_object(self):
        parsed = parse(equation('abcdefghij=12345', r'\label{x}' * 100), limits=Limits(expansion_steps=150))
        self.assertEqual(parsed['ambiguous_labels']['x']['occurrences'], list(range(100)))
        self.assertEqual(parsed['ambiguous_labels']['x']['candidate_object_ids'], ['object:x'])
        with self.assertRaisesRegex(UnsupportedSource, 'bound'):
            parse(equation('abcdefghij=12345', r'\label{x}' * 101), limits=Limits(expansion_steps=100))
        with self.assertRaisesRegex(UnsupportedSource, 'label evidence'):
            parse(equation('abcdefghij=12345', r'\label{x}' * 10), limits=Limits(text_bytes=1200))

    def test_should_keep_bibliography_key_rejection_when_object_labels_are_localized(self):
        with self.assertRaisesRegex(UnsupportedSource, 'bibliography entry key'):
            parse(r'\begin{thebibliography}{9}\bibitem{same}One complete published entry.'
                  r'\bibitem{same}Another complete published entry.\end{thebibliography}')


if __name__ == '__main__':
    unittest.main()
