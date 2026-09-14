"""Inert serial-worker lifecycle and commitment controls; no real admission."""
from contextlib import ExitStack
from copy import deepcopy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import release
import release_layout as layout
import release_replay as replay
import versioned
from test_release import fixture


def environment(folder):
    repo, cache = folder/'repo', folder/'cache'
    code = repo/'scripts/truth/latex'; code.mkdir(parents=True); cache.mkdir()
    for name in ('archive.py', 'tex.py', 'parser.py', 'align.py', 'builder.py', 'versioned.py'):
        (code/name).write_text('# inert application fixture\n')
    candidates, config, inputs, history = fixture(); config['version'] = layout.VERSION
    unscored = {key: value for key, value in deepcopy(candidates[0]).items() if not key.startswith('manual_')}
    unscored.update(paper_id='1111111111111111', arxiv_id='2201.00001', manual_assembly={'candidate_sha256': 'b'*64},
                    source_inventory={'links': [{'unknown': 'unresolved'}]})
    candidates.insert(1, unscored)
    config['papers'].insert(1, {'paper_id': unscored['paper_id'], 'arxiv_id': unscored['arxiv_id'], 'candidate': 'unscored.json'})
    inputs['papers'].insert(1, dict(deepcopy(inputs['papers'][0]), arxiv_id=unscored['arxiv_id']))
    config.update(inputs='inputs.json', indexes='indexes.json', automatic_builds=[])
    (cache/'inputs.json').write_bytes(layout.canonical(inputs)); (cache/'indexes.json').write_text('{}')
    for candidate, spec in zip(candidates, config['papers'], strict=True):
        path = cache/spec['candidate']; path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(layout.canonical(candidate))
    config['evidence_sha256'] = {name: layout.digest((cache/name).read_bytes()) for name in release.required_evidence_paths(config)}
    config_path = repo/'eval/truth'/('k1-limited-v5-build.json'); config_path.parent.mkdir(parents=True)
    raw = layout.canonical(config); config_path.write_bytes(raw)
    calls = []
    def assemble(cache, corpus, data, candidate, *args):
        calls.append(candidate)
        return layout.document(layout.read(cache/candidate, layout.PAPER_BYTES), layout.PAPER_BYTES)
    manual = SimpleNamespace(bounded=lambda root, name: layout.read(root/name, 32*1024*1024),
                             document=json.loads, assemble=assemble)
    return repo, cache, candidates, config, inputs, raw, calls, {'release': release, 'manual': manual}


class ReplayTests(unittest.TestCase):
    def test_should_rebuild_every_record_serially_when_history_is_validated_once_and_ineligible_rows_interleave(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            repo, cache, candidates, config, inputs, config_raw, calls, modules = environment(root)
            source_hashes = {}
            def transport(argv, request, directory, **kwargs):
                directory.mkdir(parents=True)
                result = replay.worker(json.loads(request), modules)
                return layout.canonical(result), {'inert': True}
            with ExitStack() as stack:
                stack.enter_context(patch.object(layout, 'FREE_BYTES', 0))
                history = stack.enter_context(patch.object(release, 'validate_automatic_reports', return_value=[{'inert': True}]))
                stack.enter_context(patch.object(replay.process, 'run', side_effect=transport))
                coordinator = replay.Coordinator(repo, cache, root/'corpus', root/'data', config_raw,
                                                 source_hashes, 'eval/truth/k1-limited-v5-build.json')
                retained = coordinator.invoke('history')
                self.assertEqual(history.call_count, 1)
                self.assertEqual(calls, [])
                records = []
                for ordinal in range(3):
                    records.append(coordinator.invoke('paper', ordinal))
                    self.assertEqual(len(calls), ordinal+1)
                self.assertEqual(history.call_count, 1)
                self.assertFalse(any(records[1]['metric_eligibility'].values()))
                self.assertEqual(records[1]['unscored_inventory']['retained_candidate'], candidates[1])
                provenance = {'evidence_sha256': config['evidence_sha256']}
                output = repo/'eval/truth'/layout.VERSION
                published = layout.publish(output, output.parent/'stage', iter(records), config, inputs,
                                           retained['history'], provenance, lambda: None, root/'ledger.jsonl')
                self.assertEqual(published['coverage']['retained_papers'], 3)
                self.assertEqual(published['coverage']['release_papers'], 2)
                outputs = {name: layout.digest(layout.read(output/name)) for name in ('objects.json','bibliography.json')}
                receipt = coordinator.receipt(outputs)
                details = layout.document(layout.read(receipt['receipt']['path']))
                self.assertEqual(len(details['workers']), 4)
                self.assertLess(len(layout.canonical(receipt)), 2048)
                # Exercise the actual replay coordinator loop over the immutable children.
                # Only the process seam is inert; complete projection and verification run.
                def replay_transport(argv, request, directory, **kwargs):
                    request = json.loads(request)
                    request.update(publication=True, current_sources={}, config_path='eval/truth/k1-limited-v5-build.json')
                    return transport(argv, layout.canonical(request), directory, **kwargs)
                stack.enter_context(patch.object(replay.process, 'run', side_effect=replay_transport))
                actual, _, result = replay.replay(repo, cache, root/'corpus', root/'data', {'outputs': outputs},
                                                 layout.read(output/'objects.json'), config_raw)
                self.assertEqual(actual, published)
                self.assertEqual(result['outputs'], outputs)
                self.assertEqual(history.call_count, 2)
                self.assertEqual(calls, [p['candidate'] for p in config['papers']]*2)

    def test_should_reject_wrong_outputs_or_resources_when_a_worker_receipt_is_substituted(self):
        mutations = ('path', 'bytes', 'sha256', 'cpu', 'rss', 'version')
        for mutation in mutations:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as raw:
                root = Path(raw); repo, cache, _, _, _, config_raw, _, modules = environment(root)
                def transport(argv, request, directory, **kwargs):
                    directory.mkdir(parents=True)
                    result = replay.worker(json.loads(request), modules)
                    if mutation in ('path','bytes','sha256'):
                        result['output'][mutation] = {'path': str(root/'outside'), 'bytes': 0, 'sha256': '0'*64}[mutation]
                    elif mutation == 'cpu':
                        result['cpu_seconds'] = 61
                    elif mutation == 'rss':
                        result['peak_rss_kib'] = 900*1024
                    else:
                        result['version'] = 'k1-limited-v1'
                    return layout.canonical(result), {'inert': True}
                with patch.object(layout, 'FREE_BYTES', 0), patch.object(replay.process, 'run', side_effect=transport):
                    coordinator = replay.Coordinator(repo, cache, root/'corpus', root/'data', config_raw,
                                                     {}, 'eval/truth/k1-limited-v5-build.json')
                    with self.assertRaises(ValueError):
                        coordinator.invoke('paper', 0)

    def test_should_reject_changed_original_evidence_when_assembly_returns_after_mutation(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw); repo, cache, candidates, config, _, config_raw, _, modules = environment(root)
            output = cache/'k1-bounded-workers/control'; output.mkdir(parents=True)
            def corrupt(*args):
                (cache/config['papers'][0]['candidate']).write_text('{}')
                return candidates[0]
            modules['manual'].assemble = corrupt
            request = dict(repo=str(repo), cache=str(cache), corpus=str(root/'corpus'), data=str(root/'data'),
                           output=str(output), config_sha256=layout.digest(config_raw), operation='paper', ordinal=0)
            with self.assertRaisesRegex(ValueError, 'evidence changed'):
                replay.worker(request, modules)
            self.assertFalse((output/'paper.json').exists())

    def test_should_refuse_real_activation_when_the_new_version_has_no_publication_pin(self):
        self.assertIsNone(versioned.BOUNDED_MANIFEST_SHA256)
        with self.assertRaisesRegex(ValueError, 'not published'):
            versioned.release_spec(layout.VERSION)


if __name__ == '__main__':
    unittest.main()
