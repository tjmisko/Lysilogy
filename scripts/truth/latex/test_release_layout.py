"""Inert complete-record tests: these fixtures establish no real truth admission."""
from copy import deepcopy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import release_layout as layout


def fixture(count=3):
    papers = []
    for index in range(count):
        negative = index % 3 == 1
        nonvisual = index % 3 == 2
        kinds = [] if negative else ['equation', 'statement', 'proof', 'algorithm']
        if not negative and not nonvisual:
            kinds += ['figure', 'table']
        entries = [] if negative else [{'id': 'bib', 'field_labels': {'author': 'Name', 'year': '2000'}}]
        eligibility = {metric: not negative for metric in layout.METRICS}
        eligibility.update(O1=not nonvisual, O2=not nonvisual)
        paper = {'paper_id': f'{index:016x}', 'arxiv_id': 'synthetic-' + str(index), 'arxiv_version': 1,
                 'pdf_sha256': 'a' * 64, 'source_sha256': 'b' * 64, 'stratum': 'synthetic',
                 'objects': [{'id': kind, 'kind': kind} for kind in kinds], 'entries': entries,
                 'references': [] if negative else [{'target': 'equation'}, {'target': 'statement'}],
                 'mentions': [] if negative else [{'target': 'bib'}],
                 'metric_eligibility': eligibility, 'reviewed_absent_kinds': ['figure', 'table'] if negative or nonvisual else [],
                 'bibliography_eligible': not negative, 'unscored_inventory': {'retained': 'unknown'},
                 'provenance': {'synthetic_fixture_only': True},
                 'counts': dict({kind: 1 for kind in kinds}, bib_entry=len(entries))}
        papers.append(paper)
    config = {'version': layout.VERSION, 'target_papers': 500,
              'coverage_followup': 'https://github.com/tjmisko/Lysilogy/issues/97',
              'selection_bias': 'inert fixtures', 'publication_deviation': 'not real truth',
              'build_date': '2000-01-01', 'papers': [{key: p[key] for key in ('paper_id', 'arxiv_id')} for p in papers]}
    inputs = {'papers': [{'arxiv_id': p['arxiv_id'], 'version': 1, 'stratum': 'synthetic',
                         'pdf': {'sha256': 'a' * 64}, 'source': {'sha256': 'b' * 64}} for p in papers]}
    return papers, config, inputs


class LayoutTests(unittest.TestCase):
    def publish(self, folder, papers, config, inputs, boundary=lambda: None, attempt='ledger'):
        with patch.object(layout, 'FREE_BYTES', 0):
            return layout.publish(folder / 'release', folder / 'staging', iter(papers), config,
                                  inputs, [], {'synthetic': True}, boundary, folder / attempt)

    def test_should_stream_complete_populations_when_500_synthetic_papers_include_negative_and_ineligible_rows(self):
        papers, config, inputs = fixture(500)
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            manifest = self.publish(root, papers, config, inputs)
            coverage = manifest['coverage']
            self.assertEqual(coverage['release_papers'], 500)
            self.assertEqual(coverage['cohort_papers']['O1'], [p['paper_id'] for p in papers if p['metric_eligibility']['O1']])
            self.assertEqual(coverage['cohort_papers']['O3'], [p['paper_id'] for p in papers if p['metric_eligibility']['O3']])
            self.assertEqual(coverage['denominators']['O2']['all_annotated_truth_objects'], 334)
            self.assertEqual(coverage['denominators']['O4']['object_reference_pairs'], 666)
            self.assertEqual(coverage['denominators']['O9']['known_fields'], 666)
            for row, expected in zip(manifest['papers'], papers, strict=True):
                self.assertEqual(layout.paper(root / 'release', row), expected)

    def test_should_count_only_each_metric_cohort_when_unscored_and_wholly_ineligible_records_are_retained(self):
        papers, config, inputs = fixture(6)
        papers[3]['metric_eligibility']['O2'] = False
        papers[3]['metric_eligibility']['O3'] = False
        papers[4]['metric_eligibility'] = dict.fromkeys(layout.METRICS, False)
        papers[5]['metric_eligibility'] = dict.fromkeys(layout.METRICS, False)
        papers[5]['bibliography_eligible'] = False
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw); manifest = self.publish(root, papers, config, inputs)
            coverage = manifest['coverage']; cases = coverage['denominators']
            self.assertEqual(cases['O1']['truth_objects'], 4)
            self.assertEqual(cases['O2']['all_annotated_truth_objects'], 2)
            self.assertEqual(cases['O3']['equations'], 2)
            self.assertEqual(cases['O4']['object_reference_pairs'], 6)
            self.assertEqual(coverage['retained_object_counts']['equation'], 4)
            self.assertEqual(coverage['positive_object_counts']['equation'], 2)
            self.assertEqual(coverage['retained_papers'], 6)
            self.assertEqual(coverage['release_papers'], 4)
            self.assertEqual(coverage['ineligible_papers'], [papers[4]['paper_id'], papers[5]['paper_id']])
            self.assertFalse(coverage['target_met'])
            self.assertEqual(layout.paper(root / 'release', manifest['papers'][5]), papers[5])

    def test_should_reject_incomplete_population_when_a_late_projection_fails(self):
        papers, config, inputs = fixture()
        def broken():
            yield papers[0]
            raise ValueError('late failure')
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            with self.assertRaisesRegex(ValueError, 'late failure'):
                self.publish(root, broken(), config, inputs)
            self.assertFalse((root / 'release').exists())
            self.assertFalse((root / 'staging/objects.json').exists())
            ledger = [json.loads(line) for line in (root / 'ledger').read_text().splitlines()]
            self.assertEqual([x['status'] for x in ledger], ['paper_staged', 'failed'])
            self.publish(root, papers, config, inputs, attempt='resumed')
            self.assertEqual(json.loads((root / 'release/papers/0000000000000000.json').read_bytes()), papers[0])

    def test_should_reject_changed_staging_when_resuming_partial_content(self):
        papers, config, inputs = fixture()
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            with self.assertRaises(ValueError):
                self.publish(root, papers[:1], config, inputs)
            (root / 'staging/papers/0000000000000000.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'immutable'):
                self.publish(root, papers, config, inputs, attempt='retry')
            self.assertFalse((root / 'release').exists())

    def test_should_reject_missing_duplicate_extra_or_reordered_papers_when_finalizing(self):
        papers, config, inputs = fixture()
        for changed in (papers[:-1], [papers[0], papers[0], papers[2]], papers + [papers[0]], papers[::-1]):
            with self.subTest(papers=[p['paper_id'] for p in changed]), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                with self.assertRaises(ValueError):
                    self.publish(root, changed, config, inputs)
                self.assertFalse((root / 'release').exists())

    def test_should_reject_mutated_children_or_totals_when_verifying_a_manifest(self):
        for change in ('child', 'missing', 'extra', 'total', 'source', 'ordinal', 'symlink'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as raw:
                papers, config, inputs = fixture()
                root = Path(raw); manifest = self.publish(root, papers, config, inputs)
                directory = root / 'release'; child = directory / manifest['papers'][0]['path']
                if change == 'child':
                    child.write_text('{}')
                elif change == 'missing':
                    child.unlink()
                elif change == 'extra':
                    (directory / 'extra.json').write_text('{}')
                elif change == 'symlink':
                    child.rename(root / 'other'); child.symlink_to(root / 'other')
                elif change == 'source':
                    inputs = deepcopy(inputs); inputs['papers'][0]['pdf']['sha256'] = 'c' * 64
                else:
                    if change == 'total':
                        manifest['coverage']['denominators']['O2']['all_annotated_truth_objects'] -= 1
                    else:
                        manifest['papers'][0]['ordinal'] = False
                    (directory / 'objects.json').write_bytes(layout.canonical(manifest))
                with self.assertRaises(ValueError):
                    layout.verify(directory, config, inputs, [], {'synthetic': True})

    def test_should_reject_conflicting_members_when_projected_records_claim_complete_counts(self):
        original = fixture()[0][0]
        for key, value in [('objects', original['objects'] + [original['objects'][0]]),
                           ('counts', {'figure': True}), ('reviewed_absent_kinds', ['figure'])]:
            with self.subTest(key=key):
                paper = deepcopy(original); paper[key] = value
                with self.assertRaises(ValueError):
                    layout.Summary().add(paper)

    def test_should_reject_invalid_json_when_size_depth_keys_or_numbers_are_unsupported(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"x":' + b'[' * 64 + b']' * 64 + b'}'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                layout.document(raw)
        raw = b'{"a":0}'
        self.assertEqual(layout.document(raw, len(raw)), {'a': 0})
        with self.assertRaises(ValueError):
            layout.document(raw, len(raw) - 1)
        with patch.object(layout, 'MAX_NODES', 2), self.assertRaises(ValueError):
            layout.document(raw)

    def test_should_reject_limit_plus_one_when_a_child_or_cumulative_budget_is_exceeded(self):
        papers, config, inputs = fixture()
        total = sum(len(layout.canonical(p)) for p in papers)
        largest = max(len(layout.canonical(p)) for p in papers)
        for constant, limit in [('PAPER_BYTES', largest - 1), ('TOTAL_BYTES', total - 1)]:
            with self.subTest(constant=constant), tempfile.TemporaryDirectory() as raw:
                with patch.object(layout, constant, limit), self.assertRaises(ValueError):
                    self.publish(Path(raw), papers, config, inputs)

    def test_should_reject_changed_evidence_when_final_boundary_fails(self):
        papers, config, inputs = fixture()
        calls = 0
        def boundary():
            nonlocal calls
            calls += 1
            if calls == 3:
                raise ValueError('evidence changed')
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            with self.assertRaisesRegex(ValueError, 'evidence changed'):
                self.publish(root, papers, config, inputs, boundary)
            self.assertFalse((root / 'release').exists())
            self.assertEqual(json.loads((root / 'ledger').read_text().splitlines()[-1])['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
