"""Inert serial-collector controls; no release activation or production calls."""
from contextlib import ExitStack
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from test_object_metrics import m, fixture

api = m.truth_verifier(m.ROOT).bounded_api(m.ROOT)
layout = api['release_layout']


def population(count=6):
    papers = []; decisions = []; artifacts = {}; natives = {}
    for position in range(count):
        paper, artifact, native = fixture(); ident = f'{position:016x}'
        paper.update(paper_id=ident, arxiv_id='synthetic-' + str(position), source_sha256='b'*64)
        artifact['paper_id'] = ident
        mode = position % 6
        paper['metric_eligibility'] = dict.fromkeys(layout.METRICS, False)
        paper['metric_eligibility'].update(O1=mode in (0, 1, 3), O2=mode in (0, 1, 2))
        if mode == 4:
            paper['metric_eligibility']['O3'] = True
        if mode == 1:
            paper.update(objects=[], counts={'figure': 0}, reviewed_absent_kinds=['figure', 'table'])
        if mode in (1, 2):
            artifact['objects'] = []
        if mode in (4, 5):
            paper['unscored_inventory'] = {'raw': ['unknown math', 'unresolved reference'], 'complete': False}
        decision = {'paper_id': ident, 'metric_eligibility': paper['metric_eligibility'],
                    'status': 'not_in_visual_cohort', 'outcomes': []}
        if mode < 4:
            metric = m.evaluate_paper(paper, artifact, native, m.BOUNDED_TRUTH_VERSION)
            decision.update(status='measured', metrics=metric, outcomes=metric['outcomes'])
        papers.append(paper); decisions.append(decision); artifacts[ident] = artifact; natives[ident] = native
    descriptors = [layout.descriptor(p, n, layout.canonical(p)) for n, p in enumerate(papers)]
    coverage = {'cohort_papers': {key: [p['paper_id'] for p in papers if p['metric_eligibility'][key]] for key in layout.METRICS},
                'denominators': {'O1': {'truth_objects': sum(p['counts'].get('figure', 0) for p in papers if p['metric_eligibility']['O1'])},
                                 'O2': {'all_annotated_truth_objects': sum(p['counts'].get('figure', 0) for p in papers if p['metric_eligibility']['O2'])}}}
    truth = {'schema_version': 2, 'version': layout.VERSION, 'layout': layout.LAYOUT, 'truth_set': 'K1',
             'limits': layout.LIMITS, 'papers': descriptors, 'paper_bytes': sum(r['bytes'] for r in descriptors), 'coverage': coverage}
    return papers, decisions, artifacts, natives, truth


class LayoutCollectorTests(unittest.TestCase):
    def test_should_preserve_exact_global_metrics_when_500_mixed_papers_include_zeros_negatives_and_ineligible_rows(self):
        papers, decisions, _, _, truth = population(500)
        totals, values, matched = m.aggregate_layout_metrics(truth, iter(decisions))
        self.assertEqual(totals['tp'], 167)
        self.assertEqual((totals['fp'], totals['fn']), (0, 0))
        self.assertEqual(values, [value for p in decisions if p['metric_eligibility']['O2'] for value in p['metrics']['region_values']])
        self.assertEqual(len(values), 167)
        self.assertEqual(values.count(0), 83)
        self.assertEqual(matched, [1.0] * 84)
        self.assertEqual(len(papers), 500)

    def test_should_reject_population_or_denominator_loss_when_the_last_paper_has_no_visual_objects(self):
        _, decisions, _, _, truth = population()
        mutations = [decisions[:-1], decisions + [decisions[-1]], list(reversed(decisions)),
                     [decisions[0], *decisions[2:], decisions[0]]]
        for rows in mutations:
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                m.aggregate_layout_metrics(truth, iter(rows))
        for value in ([], [float('nan')], [True]):
            rows = deepcopy(decisions); rows[2]['metrics']['region_values'] = value
            with self.assertRaises(ValueError):
                m.aggregate_layout_metrics(truth, iter(rows))

    def exercise(self, folder, fail_last=False, mutate_after=False, invalid_json=None):
        papers, _, artifacts, natives, truth = population()
        repo, cache, corpus, data = (folder / name for name in ('repo', 'cache', 'corpus', 'data'))
        for path in (repo, cache, corpus/'pdf', corpus/'source', data):
            path.mkdir(parents=True)
        root = repo/'eval/truth'/layout.VERSION; (root/'papers').mkdir(parents=True)
        paths = m.release_paths(layout.VERSION); expected = repo/'target/debug/examples/object_metrics'
        expected.parent.mkdir(parents=True); expected.write_bytes(b'inert selected executable')
        (repo/'target/object-metrics-build.json').write_text('{"inert":true}')
        records = {}
        for position, p in enumerate(papers):
            ident = p['paper_id']; name = p['arxiv_id']+'v1'
            for kind, suffix in (('pdf', 'pdf'), ('source', 'src')):
                raw = (kind + ident).encode(); (corpus/kind/(name+'.'+suffix)).write_bytes(raw)
                p[kind+'_sha256'] = m.digest(raw)
            index_path = data/'papers'/ident/'reading-index.json'; index_path.parent.mkdir(parents=True)
            raw = m.canonical({'index': natives[ident]}); index_path.write_bytes(raw)
            p['index'] = {'path': str(index_path.relative_to(data)), 'sha256': m.digest(raw)}
            artifacts[ident]['reading_index_generation'] = '"'+p['index']['sha256']+'"'
            records[ident] = {'active': True, 'relative_path': name+'.pdf', 'content_hash': p['pdf_sha256']}
            raw = layout.canonical(p); (root/'papers'/(ident+'.json')).write_bytes(raw)
            truth['papers'][position] = layout.descriptor(p, position, raw)
        truth['paper_bytes'] = sum(r['bytes'] for r in truth['papers'])
        truth_raw = layout.canonical(truth); (root/'objects.json').write_bytes(truth_raw)
        registry = {'schema_version': 1, 'library_root': str(corpus/'pdf'), 'records': records, 'unresolved': {}}
        (data/'paper-identities.json').write_bytes(m.canonical(registry))
        (repo/'source.py').write_text('inert source')
        sources = {'source.py': m.digest(b'inert source')}
        old_paths = m.release_paths('k1-limited-v4'); old_observation = {'synthetic': 'prior'}
        old_ref = {'path': old_paths['trace'], 'sha256': m.digest(m.canonical(old_observation)+b'\n')}
        old_payload = {'suite': 'objects', 'truth_sets': {'K1': {'version': 'k1-limited-v4'}},
                       'metrics': {key: {'evidence': [old_ref]} for key in ('O1', 'O2')}}
        m.publish_measurement(repo, old_paths, old_observation, old_payload)
        original_active = (repo/m.INPUT).read_bytes(); original_observation = (repo/old_paths['trace']).read_bytes()
        calls = []
        def bridge(argv, request_raw, directory, **kwargs):
            request = json.loads(request_raw); ident = request['papers'][0]['paper_id']; calls.append(ident)
            self.assertEqual(len(request['papers']), 1)
            if fail_last and len(calls) == 4:
                raise ValueError('inert late bridge failure')
            artifact = deepcopy(artifacts[ident]); metadata = []
            if invalid_json and 'depth' in invalid_json:
                for _ in range(65): metadata = [metadata]
            elif invalid_json:
                metadata = [0]*200001
            if invalid_json and invalid_json.startswith('artifact'):
                artifact['extra'] = metadata
            artifact_raw = m.canonical(artifact).decode()
            response = {'schema_version': 1, 'network_calls': 0, 'model_calls': 0,
                'papers': [{'paper_id': ident, 'index_sha256': papers[int(ident,16)]['index']['sha256'],
                            'artifact_json': artifact_raw, 'object_sha256': m.digest(artifact_raw.encode())}]}
            if invalid_json and invalid_json.startswith('response'):
                response['extra'] = metadata
            return m.canonical(response), {'inert': True}
        def final_replay(*args):
            if mutate_after:
                (root/truth['papers'][-1]['path']).write_bytes(b'{}')
            return truth, {}, {'inert': 'after'}
        transport = SimpleNamespace(run=bridge)
        with ExitStack() as stack:
            stack.enter_context(patch.object(m, 'ROOT', repo))
            stack.enter_context(patch.object(layout, 'FREE_BYTES', 0))
            stack.enter_context(patch.object(m, 'validate_derivation', return_value={}))
            stack.enter_context(patch.object(m, 'validate_graphics', return_value={k: {} for k in ('trace_hashes','mask_hashes','vector_hashes')}))
            stack.enter_context(patch.object(m, 'validate_truth', side_effect=final_replay))
            invoke = lambda: m._collect_layout(truth, {}, {'inert': 'before'}, truth_raw, paths, sources,
                m.digest(expected.read_bytes()), expected, time.monotonic(),
                {'release_layout': layout, 'release_process': transport}, cache, corpus, data)
            if fail_last or mutate_after or invalid_json:
                with self.assertRaises(ValueError):
                    invoke()
                self.assertEqual((repo/m.INPUT).read_bytes(), original_active)
                self.assertEqual((repo/old_paths['trace']).read_bytes(), original_observation)
                ledger = next((cache/'object-metrics-layout').glob('*/ledger.jsonl'))
                self.assertEqual(json.loads(ledger.read_text().splitlines()[-1])['status'], 'failed')
            else:
                invoke()
                active = json.loads((repo/m.INPUT).read_bytes()); observation = json.loads((repo/paths['trace']).read_bytes())
                self.assertEqual(len(observation['papers']), 6)
                self.assertEqual(len(calls), 4)
                self.assertEqual(active['metrics']['O2']['sample']['values'], [1.0, 0.0])
                self.assertEqual(active['metrics']['O1']['sample']['true_positive'], 2)
                for metric in active['metrics'].values():
                    self.assertEqual(len(metric['evidence']), 13)
                    for evidence in metric['evidence']:
                        self.assertEqual(m.digest((repo/evidence['path']).read_bytes()), evidence['sha256'])
                final = observation['papers'][-1]
                retained = json.loads((repo/observation['decision_root']/final['path']).read_bytes())
                self.assertEqual(retained['status'], 'not_in_visual_cohort')
                self.assertEqual(layout.paper(root, truth['papers'][-1])['unscored_inventory'], papers[-1]['unscored_inventory'])
                self.assertEqual([p.name for p in (repo/'eval/inputs/objects').glob('*.json')], ['figure-table.json'])

    def test_should_commit_all_children_and_one_owner_when_serial_collection_completes(self):
        with tempfile.TemporaryDirectory() as folder:
            self.exercise(Path(folder))

    def test_should_preserve_active_measurement_when_late_work_or_final_child_verification_fails(self):
        for failure in ('bridge', 'child'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as folder:
                self.exercise(Path(folder), fail_last=failure=='bridge', mutate_after=failure=='child')

    def test_should_reject_before_publication_when_bridge_or_nested_artifact_json_exceeds_structure_limits(self):
        for kind in ('response-depth','response-nodes','artifact-depth','artifact-nodes'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as folder:
                self.exercise(Path(folder), invalid_json=kind)


if __name__ == '__main__':
    unittest.main()
