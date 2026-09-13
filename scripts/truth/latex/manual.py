#!/usr/bin/env python3
"""Assemble externally retained, independently reviewed K1 manual candidates.

This never publishes final K1 or measures a production ranking. All path options
other than corpus/data roots are relative to ~/.cache/lysilogy.
"""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import time

from annotations import document
from archive import sha256
from builder import atomic_json, attach_manual_regions, canonical, fingerprint_file, fingerprint_sources, safe_file
from panel import bind_panel


def bounded(root, relative):
    path = safe_file(root, relative)
    if path.stat().st_size > 32 * 1024 * 1024:
        raise ValueError('manual evidence exceeds its byte bound')
    return path.read_bytes()


def verify_panel_images(cache, packet_raw):
    verified = {}
    for page in document(packet_raw)['pages']:
        declared = page['image_path']
        if not isinstance(declared, str) or not Path(declared).is_absolute():
            raise ValueError('panel image path must be absolute within the dedicated cache')
        relative = str(Path(declared).relative_to(cache))
        actual, _ = fingerprint_file(safe_file(cache, relative), cap=32 * 1024 * 1024)
        if actual != page['image_sha256']:
            raise ValueError('actual panel image bytes differ from its packet hash')
        verified[declared] = actual
    return verified


def assemble(cache, corpus_root, data_root, candidate_relative, region_relative, panel_relative=None,
             inputs_relative='k1-full-eval-inputs.json', indexes_relative='k1-full-index.json'):
    inputs_raw, indexes_raw = bounded(cache, inputs_relative), bounded(cache, indexes_relative)
    inputs, indexes = document(inputs_raw), document(indexes_raw)
    candidate_raw = bounded(cache, candidate_relative)
    candidate = document(candidate_raw)
    papers = [paper for paper in inputs['papers'] if paper['arxiv_id'] == candidate['arxiv_id']]
    mappings = [paper for paper in indexes['papers'] if paper['paper_id'] == candidate['paper_id']]
    if len(papers) != 1 or len(mappings) != 1:
        raise ValueError('manual candidate lacks unique frozen paper and mapped identities')
    region_root = safe_file(cache, region_relative + '/regions-root-v1.json').parent
    output = attach_manual_regions(candidate_raw, papers[0], mappings[0], corpus_root, data_root, region_root)
    if panel_relative:
        read_panel = lambda name: bounded(cache, panel_relative + '/' + name)
        packet_raw = read_panel('packet.json')
        verified_images = verify_panel_images(cache, packet_raw)
        output = bind_panel(output, packet_raw, read_panel('prompt.txt'),
                            [read_panel(f'vote-evaluator-{number}.json') for number in range(1, 4)],
                            read_panel('panel-root-review-v1.json'), read_panel('scoring-policy-v1.json'),
                            bounded(data_root, candidate['index']['path']), bounded(region_root, 'regions-root-v1.json'), verified_images)
    output['manual_assembly'] = {'inputs_sha256': sha256(inputs_raw), 'index_map_sha256': sha256(indexes_raw),
                                 'candidate_sha256': sha256(candidate_raw), 'final_k1_publication': False,
                                 'source_inventory_policy': 'exact historical candidate retained; no automatic-confidence change'}
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--region-bundle', required=True)
    parser.add_argument('--panel-bundle')
    parser.add_argument('--inputs', default='k1-full-eval-inputs.json')
    parser.add_argument('--indexes', default='k1-full-index.json')
    parser.add_argument('--corpus-root', type=Path, default=Path.home() / 'Corpora/arxiv')
    parser.add_argument('--data-root', type=Path, default=Path.home() / '.cache/lysilogy/arxiv-kb-data')
    args = parser.parse_args()
    cache = (Path.home() / '.cache/lysilogy').resolve(strict=True)
    corpus_root, data_root = args.corpus_root.resolve(strict=True), args.data_root.resolve(strict=True)
    if not corpus_root.is_relative_to((Path.home() / 'Corpora').resolve(strict=True)) or not data_root.is_relative_to(cache):
        raise ValueError('manual evidence must remain within the dedicated external corpus/cache roots')
    implementation = fingerprint_sources()
    started = time.monotonic()
    result = assemble(cache, corpus_root, data_root, args.candidate, args.region_bundle, args.panel_bundle, args.inputs, args.indexes)
    if implementation != fingerprint_sources():
        raise ValueError('manual implementation bytes changed during assembly')
    result['manual_assembly']['implementation'] = implementation
    raw = canonical(result) + b'\n'
    identity = sha256(raw)
    output = cache / 'k1-manual-candidates' / identity
    if (output / 'candidate.json').exists() and bounded(cache, str((output / 'candidate.json').relative_to(cache))) != raw:
        raise ValueError('existing manual candidate is not immutable')
    atomic_json(output / 'candidate.json', result)
    receipt = {'schema_version': 1, 'candidate_path': str(output / 'candidate.json'), 'candidate_sha256': identity,
               'implementation': implementation, 'assembled_at': datetime.now(timezone.utc).isoformat(),
               'wall_seconds': time.monotonic() - started, 'network_calls': 0, 'model_calls': 0,
               'external_call_cost_usd': 0, 'panel_agent_judgment_cost_usd': None,
               'panel_agent_cost_note': 'Independent prior agent judgment cost is not exposed by the harness.',
               'final_k1_publication': False, 'O11_measured': False}
    atomic_json(output / 'assembly-receipt.json', receipt)
    print(json.dumps(receipt, sort_keys=True))


if __name__ == '__main__':
    main()
