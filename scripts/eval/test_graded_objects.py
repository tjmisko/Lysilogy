"""Offline fixtures for the graded-objects collector; no corpus, Cargo build or production scores."""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('graded_objects', Path(__file__).with_name('graded-objects.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

PAPER_ID = 'e51f6f61ec6d4107'
ARXIV_ID = '2104.01511'
PDF_SHA = 'f' * 64
CAPTIONS = ['Figure 1. First caption.', 'Table 1. Second caption.', 'Figure 2. Third caption.', 'Figure 3. Fourth caption.']
FAKE_BRIDGE = '''#!/usr/bin/env python3
"""Stand-in for examples/object_metrics.rs: echoes the cached objects.json rebound to the requested index."""
import hashlib, json, sys
from pathlib import Path
request = json.load(sys.stdin)
rows = []
for paper in request['papers']:
    artifact = json.loads((Path(request['data_root']) / 'papers' / paper['paper_id'] / 'objects.json').read_bytes())
    artifact['reading_index_generation'] = '"' + paper['index_sha256'] + '"'
    text = json.dumps(artifact)
    rows.append({'paper_id': paper['paper_id'], 'index_sha256': paper['index_sha256'],
                 'object_sha256': hashlib.sha256(text.encode()).hexdigest(), 'artifact_json': text})
print(json.dumps({'schema_version': 1, 'network_calls': 0, 'model_calls': 0, 'papers': rows}))
'''


def rectangle(x0=0, y0=0, x1=10, y1=10):
    return {'x_min': x0, 'y_min': y0, 'x_max': x1, 'y_max': y1}


def caption_spans():
    spans, offset = {}, 0
    for caption in CAPTIONS:
        spans[caption] = {'start': offset, 'end': offset + len(caption)}
        offset += len(caption) + 1
    return spans


def reading_index():
    text = '\n'.join(CAPTIONS)
    return {'schema_version': 6, 'text': text, 'tokens': [], 'objects': {'paragraph': []}, 'figures': [], 'gaps': [],
            'pages': [{'number': 1, 'start': 0, 'end': len(text), 'width': 100, 'height': 100, 'provenance': 'native', 'confidence': None}]}


def detected(ident, kind, label, span, region):
    return {'id': ident, 'kind': kind, 'label': label, 'page': 1, 'text': label, 'anchor': {'page': 1, **span},
            'member_anchors': [], 'region': region, 'confidence': 'high', 'mentions': []}


def objects_artifact(generation):
    spans = caption_spans()
    return {'schema_version': 1, 'paper_id': PAPER_ID, 'reading_index_generation': generation, 'figure_detector_version': 6,
            'figure_detector_generation': 'd' * 64,
            'objects': [detected('fig-1', 'figure', 'Figure 1', spans[CAPTIONS[0]], rectangle()),
                        detected('tab-1', 'table', 'Table 1', spans[CAPTIONS[1]], rectangle()),
                        detected('fig-2', 'figure', 'Figure 2', spans[CAPTIONS[2]], rectangle(20, 20, 30, 30))]}


def grades(index_sha256):
    spans = caption_spans()
    return {'schema_version': 1, 'paper_id': PAPER_ID, 'index_sha256': index_sha256, 'objects_generation': 'd' * 64,
            'grader': 'tjmisko', 'updated_at': '2026-09-15T20:11:00-07:00', 'revision': 'r' * 64, 'complete': True,
            'verdicts': {'fig-1': {'verdict': 'correct', 'region': None, 'note': ''},
                         'tab-1': {'verdict': 'region', 'region': rectangle(5, 0, 15, 10), 'note': ''},
                         'fig-2': {'verdict': 'reject', 'region': None, 'note': 'prose'}},
            'additions': [{'id': 'add-1', 'kind': 'figure', 'printed_label': '3', 'page': 1, 'region': rectangle(50, 50, 60, 60),
                           'caption': spans[CAPTIONS[3]], 'note': ''}]}


def expected_truth_objects():
    spans = caption_spans()
    return [{'id': 'graded:fig-1', 'kind': 'figure', 'printed_label': '1', 'spans': [spans[CAPTIONS[0]]], 'region': [{'page': 1, 'rect': rectangle()}]},
            {'id': 'graded:tab-1', 'kind': 'table', 'printed_label': '1', 'spans': [spans[CAPTIONS[1]]], 'region': [{'page': 1, 'rect': rectangle(5, 0, 15, 10)}]},
            {'id': 'graded:add-1', 'kind': 'figure', 'printed_label': '3', 'spans': [spans[CAPTIONS[3]]], 'region': [{'page': 1, 'rect': rectangle(50, 50, 60, 60)}]}]


class Fixture:
    """A temp repo, data root and corpus holding one gradable paper."""

    def __init__(self, directory):
        root = Path(directory)
        self.repo, self.data, self.corpus = root / 'repo', root / 'data', root / 'corpus'
        (self.corpus / 'pdf').mkdir(parents=True)
        (self.corpus / 'pdf' / (ARXIV_ID + 'v1.pdf')).write_bytes(b'%PDF')
        for path in [*m.IMPLEMENTATION_FILES, 'src/objects/mod.rs', 'src/source_index/figures.rs']:
            (self.repo / path).parent.mkdir(parents=True, exist_ok=True)
            (self.repo / path).write_text('// ' + path + '\n')
        self.executable = self.repo / 'target/debug/examples/object_metrics'
        self.executable.parent.mkdir(parents=True)
        self.executable.write_text(FAKE_BRIDGE)
        self.executable.chmod(self.executable.stat().st_mode | stat.S_IXUSR)
        self.paper = self.data / 'papers' / PAPER_ID
        self.paper.mkdir(parents=True)
        self.write_index(reading_index())
        self.write_registry({PAPER_ID: {'relative_path': ARXIV_ID + 'v1.pdf', 'content_hash': PDF_SHA, 'active': True}})
        self.write_objects(objects_artifact('"' + self.index_sha256 + '"'))
        self.write_grades(grades(self.index_sha256))

    def write_index(self, index):
        raw = json.dumps({'source': 'fixture', 'generation': '1-fixture', 'index': index}).encode()
        (self.paper / 'reading-index.json').write_bytes(raw)
        self.index_sha256 = m.digest(raw)

    def write_registry(self, records):
        m.atomic_json(self.data / 'paper-identities.json',
                      {'schema_version': 1, 'library_root': str(self.corpus / 'pdf'), 'unresolved': {}, 'records': records})

    def write_objects(self, artifact):
        m.atomic_json(self.paper / 'objects.json', artifact)

    def write_grades(self, value):
        m.atomic_json(self.paper / 'objects-grades.json', value)

    def export(self, executable=None):
        return m.export(self.repo, self.data, self.corpus, executable)

    def measure(self, **kwargs):
        return m.measure(self.repo, self.data, self.corpus, self.executable, **kwargs)

    def child(self):
        return m.document(m.read(m.child_path(self.repo, PAPER_ID)))

    def manifest(self):
        return m.document(m.read(self.repo / m.MANIFEST))


def selection_paper(arxiv_id, category, tiers=('eval',), version=1):
    return {'id': arxiv_id, 'categories': [category], 'tiers': list(tiers), 'title': arxiv_id,
            'strata': {tier: [category, 2023] for tier in tiers}, 'remote_pdf': {'version': version}}


def queue_fixture():
    rows = [('cs.LG', 4), ('math.PR', 3), ('hep-th', 2), ('econ.EM', 2), ('stat.ML', 1), ('q-bio.NC', 1)]
    papers, records, n = [], {}, 0
    for category, count in rows:
        for _ in range(count):
            n += 1
            arxiv_id = '2301.%05d' % n
            papers.append(selection_paper(arxiv_id, category))
            records['%016x' % n] = {'relative_path': arxiv_id + 'v1.pdf', 'content_hash': 'a' * 64, 'active': True}
    papers.append(selection_paper('2301.99999', 'cs.CV'))  # no registry record
    papers.append(selection_paper('2302.00001', 'cs.CV', tiers=('scale',)))
    records['%016x' % 999] = {'relative_path': '2302.00001v1.pdf', 'content_hash': 'a' * 64, 'active': True}
    selection = {'schema_version': 1, 'papers': papers}
    registry = {'schema_version': 1, 'library_root': '/corpus/pdf', 'unresolved': {}, 'records': records}
    return selection, registry


class GradedObjectTests(unittest.TestCase):
    def should_interleave_strata_and_repeat_order_when_seed_is_fixed(self):
        selection, registry = queue_fixture()
        first, skipped = m.build_queue(selection, registry, 'eval', 1)
        again, _ = m.build_queue(selection, registry, 'eval', 1)
        self.assertEqual(first, again)
        self.assertEqual(skipped, ['2301.99999'])
        self.assertEqual(first['schema_version'], 1)
        self.assertEqual((first['seed'], first['tier']), (1, 'eval'))
        strata = [row['stratum'] for row in first['papers']]
        self.assertEqual(len(strata), 13)
        self.assertEqual(sorted(strata[:6]), sorted(m.FIELDS))
        self.assertEqual(strata, ['cs', 'econ', 'math', 'other', 'physics', 'stat', 'cs', 'econ', 'math', 'physics', 'cs', 'math', 'cs'])
        self.assertNotIn('2302.00001', {row['arxiv_id'] for row in first['papers']})
        for row in first['papers']:
            self.assertEqual(registry['records'][row['paper_id']]['relative_path'], row['arxiv_id'] + 'v1.pdf')
        cs_orders = {tuple(row['arxiv_id'] for row in m.build_queue(selection, registry, 'eval', seed)[0]['papers'] if row['stratum'] == 'cs')
                     for seed in range(1, 6)}
        self.assertGreater(len(cs_orders), 1)
        limited, _ = m.build_queue(selection, registry, 'eval', 1, limit=3)
        self.assertEqual(limited['papers'], first['papers'][:3])

    def should_choose_the_selected_pdf_version_when_the_registry_holds_several(self):
        selection = {'papers': [selection_paper('2301.00001', 'cs.LG', version=2)]}
        records = {'1' * 16: {'relative_path': '2301.00001v1.pdf', 'content_hash': 'a' * 64, 'active': True},
                   '2' * 16: {'relative_path': '2301.00001v2.pdf', 'content_hash': 'a' * 64, 'active': True},
                   '3' * 16: {'relative_path': '2301.00001v3.pdf', 'content_hash': 'a' * 64, 'active': False}}
        registry = {'schema_version': 1, 'records': records}
        self.assertEqual(m.build_queue(selection, registry, 'eval', 1)[0]['papers'][0]['paper_id'], '2' * 16)
        selection['papers'][0]['remote_pdf'] = None
        self.assertEqual(m.build_queue(selection, registry, 'eval', 1)[0]['papers'][0]['paper_id'], '2' * 16)

    def should_map_archives_to_fields_when_categories_vary(self):
        cases = {'math.PR': 'math', 'math-ph': 'physics', 'cs.LG': 'cs', 'hep-th': 'physics', 'astro-ph.CO': 'physics',
                 'cond-mat.str-el': 'physics', 'quant-ph': 'physics', 'stat.ML': 'stat', 'econ.EM': 'econ', 'q-fin.PR': 'econ',
                 'q-bio.NC': 'other', 'eess.SP': 'other', '': 'other'}
        for category, field in cases.items():
            with self.subTest(category=category):
                self.assertEqual(m.field_of(category), field)

    def should_write_the_queue_file_when_selection_and_registry_are_on_disk(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            selection, registry = queue_fixture()
            m.atomic_json(fixture.corpus / 'selection.json', selection)
            fixture.write_registry(registry['records'])
            outcome = m.queue(fixture.data, fixture.corpus / 'selection.json', 'eval', 7, 5)
            written = m.document(m.read(fixture.data / 'grading-queue.json'))
            self.assertEqual(outcome['papers'], 5)
            self.assertEqual(outcome['skipped_unregistered'], 1)
            self.assertEqual(len(written['papers']), 5)
            self.assertEqual(written['seed'], 7)
            self.assertEqual(set(written['papers'][0]), {'paper_id', 'arxiv_id', 'stratum'})

    def should_seed_children_from_a_release_when_papers_are_visually_eligible(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            release = fixture.repo / 'eval/truth/k1-limited-v5'
            spans = caption_spans()
            eligible = {'paper_id': PAPER_ID, 'arxiv_id': ARXIV_ID, 'arxiv_url': 'x', 'arxiv_version': 1, 'pdf_sha256': PDF_SHA,
                        'index': {'path': 'papers/' + PAPER_ID + '/reading-index.json', 'sha256': fixture.index_sha256},
                        'metric_eligibility': {'O1': True, 'O2': True, 'O3': False}, 'counts': {'bib_entry': 0, 'figure': 1, 'table': 1, 'equation': 2},
                        'reviewed_absent_kinds': [], 'provenance': {'assembly_sha256': 'z' * 64},
                        'objects': [{'id': 'object:one', 'kind': 'figure', 'labels': ['one'], 'printed_label': '1', 'spans': [spans[CAPTIONS[0]]],
                                     'region': [{'page': 1, 'rect': rectangle()}], 'source_members': [], 'text_sha256': 's' * 64},
                                    {'id': 'object:tab', 'kind': 'table', 'printed_label': '1', 'spans': [{**spans[CAPTIONS[1]], 'native_text_sha256': 'n' * 64}],
                                     'region': {'page': 1, 'rect': rectangle(5, 0, 15, 10)}},
                                    {'id': 'object:nospan', 'kind': 'table', 'printed_label': '2', 'spans': [], 'region': [{'page': 1, 'rect': rectangle()}]},
                                    {'id': 'object:eq', 'kind': 'equation', 'printed_label': '1', 'spans': [spans[CAPTIONS[1]]]}]}
            other = {**copy.deepcopy(eligible), 'paper_id': '2' * 16, 'metric_eligibility': {'O1': False, 'O2': False, 'O3': True}}
            rows = []
            for paper in (eligible, other):
                raw = m.canonical(paper) + b'\n'
                (release / 'papers').mkdir(parents=True, exist_ok=True)
                (release / 'papers' / (paper['paper_id'] + '.json')).write_bytes(raw)
                rows.append({'paper_id': paper['paper_id'], 'path': 'papers/' + paper['paper_id'] + '.json', 'sha256': m.digest(raw),
                             'metric_eligibility': paper['metric_eligibility']})
            m.atomic_json(release / 'objects.json', {'schema_version': 2, 'version': 'k1-limited-v5', 'papers': rows})
            outcome = m.seed_release(fixture.repo, 'eval/truth/k1-limited-v5')
            self.assertEqual((outcome['written'], outcome['dropped_objects']), ([PAPER_ID], 1))
            self.assertFalse(m.child_path(fixture.repo, '2' * 16).exists())
            child = fixture.child()
            self.assertEqual(child['provenance']['source'], 'k1-limited-v5')
            self.assertEqual(child['provenance']['child_sha256'], rows[0]['sha256'])
            self.assertEqual((child['arxiv_id'], child['arxiv_version'], child['pdf_sha256']), (ARXIV_ID, 1, PDF_SHA))
            self.assertEqual(child['index'], eligible['index'])
            self.assertEqual(child['metric_eligibility'], {'O1': True, 'O2': True})
            self.assertEqual(child['counts'], {'figure': 1, 'table': 1})
            self.assertEqual(child['reviewed_absent_kinds'], [])
            self.assertEqual(child['objects'], [{'id': 'object:one', 'kind': 'figure', 'printed_label': '1', 'spans': [spans[CAPTIONS[0]]],
                                                 'region': [{'page': 1, 'rect': rectangle()}]},
                                                {'id': 'object:tab', 'kind': 'table', 'printed_label': '1', 'spans': [spans[CAPTIONS[1]]],
                                                 'region': [{'page': 1, 'rect': rectangle(5, 0, 15, 10)}]}])
            # The seeded child scores under the default truth version, exactly as measure will call it.
            scored = m.object_metrics.evaluate_paper(child, objects_artifact('"' + fixture.index_sha256 + '"'), reading_index())
            self.assertEqual((scored['tp'], scored['fp'], scored['fn']), (2, 1, 0))
            self.assertEqual(sorted(scored['region_values']), [1 / 3, 1.0])
            manifest = fixture.manifest()
            self.assertEqual(manifest['coverage']['cohort_papers'], {'O1': [PAPER_ID], 'O2': [PAPER_ID]})
            self.assertEqual(manifest['coverage']['denominators'], {'O1': {'truth_objects': 2}, 'O2': {'all_annotated_truth_objects': 2}})
            self.assertEqual(manifest['papers'][0]['provenance_source'], 'k1-limited-v5')
            # A graded child is never replaced by a seeded one.
            graded = {**child, 'provenance': {'source': 'graded'}, 'objects': [], 'counts': {}, 'reviewed_absent_kinds': ['figure', 'table']}
            m.atomic_json(m.child_path(fixture.repo, PAPER_ID), graded)
            outcome = m.seed_release(fixture.repo, 'eval/truth/k1-limited-v5')
            self.assertEqual((outcome['written'], outcome['kept_graded']), ([], [PAPER_ID]))
            self.assertEqual(fixture.child()['provenance']['source'], 'graded')

    def should_derive_truth_from_verdicts_and_additions_when_grades_are_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            seeded = {'paper_id': PAPER_ID, 'arxiv_id': ARXIV_ID, 'arxiv_version': 1, 'pdf_sha256': PDF_SHA,
                      'index': {'path': 'papers/' + PAPER_ID + '/reading-index.json', 'sha256': 'old'}, 'provenance': {'source': 'k1-limited-v5'},
                      'metric_eligibility': {'O1': True, 'O2': True}, 'counts': {}, 'reviewed_absent_kinds': ['figure', 'table'], 'objects': []}
            m.atomic_json(m.child_path(fixture.repo, PAPER_ID), seeded)
            outcome = fixture.export()
            self.assertEqual(outcome, {'exported': [PAPER_ID], 'skipped': {}, 'partial': 0,
                                       'coverage': {'cohort_papers': {'O1': [PAPER_ID], 'O2': [PAPER_ID]},
                                                    'denominators': {'O1': {'truth_objects': 3}, 'O2': {'all_annotated_truth_objects': 3}}}})
            child = fixture.child()
            self.assertEqual(child['objects'], expected_truth_objects())
            self.assertEqual(child['counts'], {'figure': 2, 'table': 1})
            self.assertEqual(child['reviewed_absent_kinds'], [])
            self.assertEqual(child['metric_eligibility'], {'O1': True, 'O2': True})
            self.assertEqual((child['arxiv_id'], child['arxiv_version'], child['pdf_sha256']), (ARXIV_ID, 1, PDF_SHA))
            self.assertEqual(child['index'], {'path': 'papers/' + PAPER_ID + '/reading-index.json', 'sha256': fixture.index_sha256})
            grades_sha = m.digest(m.read(fixture.paper / 'objects-grades.json'))
            self.assertEqual(child['provenance'], {'source': 'graded', 'grader': 'tjmisko', 'graded_at': '2026-09-15T20:11:00-07:00',
                                                   'grades_sha256': grades_sha, 'objects_generation': 'd' * 64})
            raw = m.read(m.child_path(fixture.repo, PAPER_ID))
            self.assertEqual(raw, m.canonical(child) + b'\n')
            manifest = fixture.manifest()
            self.assertEqual({k: manifest[k] for k in ('schema_version', 'truth_set', 'origin', 'version', 'layout')},
                             {'schema_version': 1, 'truth_set': 'K1', 'origin': 'graded', 'version': 'k1-graded', 'layout': 'k1-graded-v1'})
            self.assertEqual(manifest['papers'], [{'paper_id': PAPER_ID, 'path': 'papers/' + PAPER_ID + '.json', 'sha256': m.digest(raw),
                                                   'bytes': len(raw), 'counts': {'figure': 2, 'table': 1},
                                                   'metric_eligibility': {'O1': True, 'O2': True}, 'provenance_source': 'graded'}])
            build_date = manifest['build_date']
            self.assertEqual(fixture.export()['exported'], [PAPER_ID])
            self.assertEqual(fixture.manifest()['build_date'], build_date)

    def should_keep_printed_case_when_detector_labels_carry_a_kind_prefix(self):
        for value, kind, expected in [('Figure 3', 'figure', '3'), ('Fig. 2.1', 'figure', '2.1'), ('Table IV', 'table', 'IV'),
                                      ('TABLE S1:', 'table', 'S1'), ('Figure', 'figure', None), ('Figure 1(a)', 'figure', None), (None, 'figure', None)]:
            with self.subTest(value=value):
                self.assertEqual(m.printed_label(value, kind), expected)

    def should_skip_papers_with_a_reason_when_grades_cannot_be_exported(self):
        spans = caption_spans()
        blockers = {
            'missing_verdict:tab-1': lambda f, g: g['verdicts'].pop('tab-1'),
            'correct_without_region:fig-1': lambda f, g: f.write_objects({**objects_artifact('"' + f.index_sha256 + '"'),
                                                                          'objects': [{**o, 'region': None} for o in objects_artifact('x')['objects']]}),
            'region_without_box:tab-1': lambda f, g: g['verdicts']['tab-1'].update(region={'x_min': 0}),
            'unknown_verdict:fig-1': lambda f, g: g['verdicts']['fig-1'].update(verdict='maybe'),
            'addition_without_caption:add-1': lambda f, g: g['additions'][0].update(caption=None),
            'addition_without_region:add-1': lambda f, g: g['additions'][0].update(region=None),
            'addition_kind:add-1': lambda f, g: g['additions'][0].update(kind='equation'),
            'stale_index': lambda f, g: g.update(index_sha256='0' * 64),
            'duplicate_truth_id': lambda f, g: g['additions'][0].update(id='fig-1', caption=spans[CAPTIONS[3]]),
            'unregistered_paper': lambda f, g: f.write_registry({}),
        }
        for reason, mutate in blockers.items():
            with self.subTest(reason=reason), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory)
                value = grades(fixture.index_sha256)
                mutate(fixture, value)
                fixture.write_grades(value)
                outcome = fixture.export()
                self.assertEqual(outcome['exported'], [])
                self.assertEqual(outcome['skipped'], {PAPER_ID: [reason]})
                self.assertFalse(m.child_path(fixture.repo, PAPER_ID).exists())
                self.assertEqual(fixture.manifest()['coverage']['denominators']['O1']['truth_objects'], 0)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            value = grades(fixture.index_sha256)
            value['additions'][0]['caption'] = {'start': spans[CAPTIONS[0]]['end'], 'end': spans[CAPTIONS[0]]['end'] + 1}
            fixture.write_grades(value)
            outcome = fixture.export()
            self.assertEqual(list(outcome['skipped']), [PAPER_ID])
            self.assertRegex(outcome['skipped'][PAPER_ID][0], '^invalid_truth: caption has no authored membership')
            fixture.write_grades({**grades(fixture.index_sha256), 'complete': False})
            outcome = fixture.export()
            self.assertEqual((outcome['exported'], outcome['skipped'], outcome['partial']), ([], {}, 1))

    def should_request_predictions_from_the_bridge_when_cached_objects_are_stale(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.write_objects(objects_artifact('"stale"'))
            outcome = fixture.export()
            self.assertEqual(outcome['skipped'], {PAPER_ID: ['no_current_objects (rerun export with --executable)']})
            requests = []
            real_run = m.subprocess.run

            def observed(command, **kwargs):
                requests.append((command, m.document(kwargs['input']), kwargs['cwd']))
                return real_run(command, **kwargs)
            with patch.object(m.subprocess, 'run', side_effect=observed):
                outcome = fixture.export(fixture.executable)
            self.assertEqual(outcome['exported'], [PAPER_ID])
            self.assertEqual(fixture.child()['objects'], expected_truth_objects())
            command, request, cwd = requests[0]
            self.assertEqual(command, [str(fixture.executable)])
            self.assertEqual(cwd, fixture.repo)
            self.assertEqual(request, {'corpus_root': str(fixture.corpus / 'pdf'), 'data_root': str(fixture.data),
                                       'papers': [{'paper_id': PAPER_ID, 'relative_path': ARXIV_ID + 'v1.pdf', 'index_sha256': fixture.index_sha256}]})

    def should_reject_bridge_output_when_its_receipt_or_hashes_are_wrong(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            request = [{'paper_id': PAPER_ID, 'relative_path': ARXIV_ID + 'v1.pdf', 'index_sha256': fixture.index_sha256}]
            good = m.document(m.subprocess.run([str(fixture.executable)], input=m.canonical({'corpus_root': '', 'data_root': str(fixture.data), 'papers': request}),
                                               capture_output=True, check=True).stdout)
            bad = [{**good, 'network_calls': 1}, {**good, 'papers': []}, {**good, 'papers': [{**good['papers'][0], 'index_sha256': 'x'}]},
                   {**good, 'papers': [{**good['papers'][0], 'object_sha256': '0' * 64}]}]
            for response in bad:
                with self.subTest(response=response), patch.object(m.subprocess, 'run', return_value=m.subprocess.CompletedProcess([], 0, m.canonical(response), b'')):
                    with self.assertRaises(ValueError):
                        m.bridge_predictions(fixture.executable, fixture.repo, fixture.corpus, fixture.data, request)
            with patch.object(m.subprocess, 'run', return_value=m.subprocess.CompletedProcess([], 1, b'', b'boom')):
                with self.assertRaisesRegex(ValueError, 'bridge failed: boom'):
                    m.bridge_predictions(fixture.executable, fixture.repo, fixture.corpus, fixture.data, request)

    def should_measure_known_scores_when_the_bridge_echoes_prepared_objects(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.export()
            outcome = fixture.measure()
            self.assertEqual(outcome['summary'], {'tp': 2, 'fp': 1, 'fn': 1, 'truth_objects': 3, 'predictions': 3})
            self.assertAlmostEqual(outcome['O1'], 2 / 3)
            self.assertAlmostEqual(outcome['O2'], 1 / 3)
            self.assertEqual((outcome['papers'], outcome['frozen_prior_input']), (1, None))
            observation_raw = m.read(fixture.repo / m.OBSERVATION)
            observation = m.document(observation_raw)
            self.assertEqual({k: observation[k] for k in ('schema_version', 'collector', 'truth_version', 'network_calls', 'model_calls', 'cost_usd')},
                             {'schema_version': 1, 'collector': 'graded-objects-v1', 'truth_version': 'k1-graded', 'network_calls': 0, 'model_calls': 0, 'cost_usd': 0})
            truth_raw = m.read(fixture.repo / m.MANIFEST)
            self.assertEqual(observation['truth_sha256'], m.digest(truth_raw))
            self.assertEqual(observation['executable_sha256'], m.digest(fixture.executable.read_bytes()))
            paper = observation['papers'][0]
            self.assertEqual((paper['paper_id'], paper['arxiv_id'], paper['tp'], paper['fp'], paper['fn']), (PAPER_ID, ARXIV_ID, 2, 1, 1))
            self.assertEqual(paper['unmatched_prediction_positions'], [2])
            self.assertEqual(sorted(paper['region_values']), [0.0, 1 / 3, 1.0])
            self.assertEqual(paper['detector'], {'version': 6, 'generation': 'd' * 64})
            self.assertEqual([row['id'] for row in paper['prediction_rows']], ['fig-1', 'tab-1', 'fig-2'])
            self.assertEqual([row['page'] for row in paper['truth_rows']], [1, 1, 1])
            payload = m.document(m.read(fixture.repo / m.INPUT))
            self.assertEqual({k: payload[k] for k in ('schema_version', 'suite', 'collector', 'cost_usd')},
                             {'schema_version': 1, 'suite': 'objects', 'collector': 'graded-objects-v1', 'cost_usd': 0})
            self.assertEqual(payload['wall_seconds'], observation['wall_seconds'])
            self.assertEqual(payload['truth_sets'], {'K1': {'path': m.MANIFEST, 'version': 'k1-graded', 'sha256': m.digest(truth_raw)}})
            paths = [row['path'] for row in payload['implementation']]
            self.assertEqual(paths, sorted(paths))
            self.assertEqual(set(paths), {*m.IMPLEMENTATION_FILES, 'src/objects/mod.rs', 'src/source_index/figures.rs'})
            for row in payload['implementation']:
                self.assertEqual((row['version'], row['sha256']), ('graded-objects-v1', m.digest((fixture.repo / row['path']).read_bytes())))
            o1, o2 = payload['metrics']['O1'], payload['metrics']['O2']
            self.assertEqual(o1['sample'], {'method': 'f1', 'true_positive': 2, 'false_positive': 1, 'false_negative': 1})
            self.assertEqual(o1['cases'], 3)
            self.assertEqual(o2['sample']['method'], 'median')
            self.assertEqual(sorted(o2['sample']['values']), [0.0, 1 / 3, 1.0])
            self.assertEqual(o2['cases'], 3)
            child_raw = m.read(m.child_path(fixture.repo, PAPER_ID))
            expected_evidence = [{'path': m.OBSERVATION, 'version': 'graded-objects-v1', 'sha256': m.digest(observation_raw)},
                                 {'path': m.TRUTH_DIR + '/papers/' + PAPER_ID + '.json', 'version': 'k1-graded', 'sha256': m.digest(child_raw)}]
            self.assertEqual(o1['evidence'], expected_evidence)
            self.assertEqual(o2['evidence'], expected_evidence)

    def should_refuse_measurement_when_the_manifest_denominator_or_child_differs(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.export()
            manifest = fixture.manifest()
            manifest['coverage']['denominators']['O2']['all_annotated_truth_objects'] = 2
            m.atomic_json(fixture.repo / m.MANIFEST, manifest)
            with self.assertRaisesRegex(ValueError, 'incomplete frozen O2 denominator'):
                fixture.measure()
            manifest['coverage']['denominators']['O2']['all_annotated_truth_objects'] = 3
            manifest['coverage']['denominators']['O1']['truth_objects'] = 4
            m.atomic_json(fixture.repo / m.MANIFEST, manifest)
            with self.assertRaisesRegex(ValueError, 'incomplete frozen O1 denominator'):
                fixture.measure()
            manifest['coverage']['denominators']['O1']['truth_objects'] = 3
            manifest['papers'][0]['sha256'] = '0' * 64
            m.atomic_json(fixture.repo / m.MANIFEST, manifest)
            with self.assertRaisesRegex(ValueError, 'truth child differs'):
                fixture.measure()
            fixture.export()
            fixture.write_registry({PAPER_ID: {'relative_path': ARXIV_ID + 'v2.pdf', 'content_hash': PDF_SHA, 'active': True}})
            with self.assertRaisesRegex(ValueError, 'registry identity'):
                fixture.measure()
            self.assertFalse((fixture.repo / m.INPUT).exists())

    def should_freeze_a_foreign_prior_input_when_the_collector_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.export()
            trace = 'eval/inputs/evidence/object-metrics-k1-limited-v5.json'
            old_observation = {'collector': 'figure-table-metrics-v6', 'O1': 0.5}
            m.atomic_json(fixture.repo / trace, old_observation)
            old_input = {'schema_version': 1, 'suite': 'objects', 'collector': 'figure-table-metrics-v6',
                         'truth_sets': {'K1': {'version': 'k1-limited-v5'}},
                         'metrics': {key: {'evidence': [{'path': trace, 'version': 'figure-table-metrics-v6',
                                                          'sha256': m.digest(m.canonical(old_observation) + b'\n')}]} for key in ('O1', 'O2')}}
            m.atomic_json(fixture.repo / m.INPUT, old_input)
            outcome = fixture.measure()
            folder = fixture.repo / outcome['frozen_prior_input']
            self.assertEqual(folder.parent, fixture.repo / 'eval/evidence/object-metrics-history')
            self.assertEqual((folder / 'input.json').read_bytes(), m.canonical(old_input) + b'\n')
            self.assertEqual((folder / 'observation.json').read_bytes(), m.canonical(old_observation) + b'\n')
            manifest = m.document((folder / 'manifest.json').read_bytes())
            self.assertEqual(manifest['original_paths'], {'input': m.INPUT, 'observation': trace})
            self.assertEqual(m.document(m.read(fixture.repo / m.INPUT))['collector'], 'graded-objects-v1')
            self.assertEqual([p.name for p in (fixture.repo / 'eval/inputs/objects').glob('*.json')], ['figure-table.json'])
            self.assertIsNone(fixture.measure()['frozen_prior_input'])
            (fixture.repo / trace).write_bytes(b'{"changed":true}')
            m.atomic_json(fixture.repo / m.INPUT, old_input)
            with self.assertRaisesRegex(ValueError, 'prior observation differs'):
                fixture.measure()
            self.assertEqual(m.document(m.read(fixture.repo / m.INPUT)), old_input)

    def should_build_the_bridge_first_when_measure_is_asked_to_build(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.export()
            commands = []
            real_run = m.subprocess.run

            def observed(command, **kwargs):
                commands.append(command)
                if command[0] == 'cargo':
                    self.assertEqual(kwargs['cwd'], fixture.repo)
                    return m.subprocess.CompletedProcess(command, 0, b'', b'')
                return real_run(command, **kwargs)
            with patch.object(m.subprocess, 'run', side_effect=observed):
                fixture.measure(build=True)
            self.assertEqual(commands[0], ['cargo', 'build', '--example', 'object_metrics'])
            self.assertEqual(commands[1], [str(fixture.executable)])

    def should_list_unmatched_predictions_and_missed_truth_when_reporting(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.export()
            fixture.measure()
            text = m.report(fixture.repo)
            lines = text.splitlines()
            self.assertEqual(lines[0], PAPER_ID + ' ' + ARXIV_ID + 'v1 tp=2 fp=1 fn=1')
            self.assertEqual(lines[1], "  unmatched prediction #2 figure 'Figure 2' page 1")
            self.assertEqual(lines[2], "  missed truth graded:add-1 figure '3' page 1")
            self.assertEqual(lines[3], 'total papers=1 tp=2 fp=1 fn=1 O1=0.6667 O2=0.3333')
            self.assertEqual(len(lines), 4)

    def should_dispatch_subcommands_when_run_from_the_command_line(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            roots = ['--repo', str(fixture.repo), '--data', str(fixture.data), '--corpus', str(fixture.corpus)]
            selection, registry = queue_fixture()
            m.atomic_json(fixture.corpus / 'selection.json', selection)
            fixture.write_registry({**registry['records'], PAPER_ID: {'relative_path': ARXIV_ID + 'v1.pdf', 'content_hash': PDF_SHA, 'active': True}})
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                m.main(['queue', *roots, '--limit', '2'])
                m.main(['export', *roots])
                m.main(['measure', *roots, '--executable', str(fixture.executable)])
                m.main(['report', *roots])
            lines = printed.getvalue().splitlines()
            self.assertEqual(m.document(lines[0].encode())['papers'], 2)
            self.assertEqual(m.document(lines[1].encode())['exported'], [PAPER_ID])
            self.assertEqual(m.document(lines[2].encode())['summary']['tp'], 2)
            self.assertTrue(lines[3].startswith(PAPER_ID))
            self.assertTrue((fixture.data / 'grading-queue.json').is_file())
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                m.main(['unknown'])

    def should_expand_home_directories_when_defaults_are_used(self):
        parser_args = []
        with patch.object(m, 'report', side_effect=lambda repo: parser_args.append(repo) or ''), contextlib.redirect_stdout(io.StringIO()):
            m.main(['report'])
        self.assertEqual(parser_args, [m.ROOT])
        with patch.object(m, 'export', side_effect=lambda repo, data, corpus, executable: parser_args.append((data, corpus)) or {}), contextlib.redirect_stdout(io.StringIO()):
            m.main(['export'])
        data, corpus = parser_args[1]
        self.assertEqual(data, Path(os.path.expanduser('~/.cache/lysilogy/arxiv-kb-data')))
        self.assertEqual(corpus, Path(os.path.expanduser('~/Corpora/arxiv')))


def load_tests(loader, tests, pattern):
    return unittest.TestSuite(GradedObjectTests(name) for name in dir(GradedObjectTests) if name.startswith('should_'))


if __name__ == '__main__':
    unittest.main()
