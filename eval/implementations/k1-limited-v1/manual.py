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


def attach_objects(cache, corpus_root, data_root, candidate_raw, paper, mapped, bundle_relative):
    from object_annotations import apply_object_overlay
    candidate = document(candidate_raw)
    if candidate['index'] != mapped['index'] or candidate['paper_id'] != mapped['paper_id']:
        raise ValueError('manual object candidate differs from frozen mapped identities')
    artifact_paths = {}
    for kind in ('pdf', 'source'):
        expected = paper[kind]
        path = safe_file(corpus_root, expected['path'])
        actual, count = fingerprint_file(path)
        if actual != expected['sha256'] or count != expected['bytes'] or actual != candidate[kind + '_sha256']:
            raise ValueError('manual object source/PDF differs from frozen receipts')
        artifact_paths[kind] = path
    bundle_root = safe_file(cache, bundle_relative + '/packet.json').parent
    names = ('packet.json', 'root-v1.json', 'independent-v1.json', 'independent-v1-receipt.json', 'reconciliation-independent-v1.json')
    raws = [bounded(bundle_root, name) for name in names]
    packet, root, independent, receipt, _ = [document(raw) for raw in raws]
    # Verify paths the annotators actually used, in addition to the expected
    # frozen corpus names. No hash assertion permits a different unread path.
    inputs = independent['inputs']
    for kind in ('pdf', 'source'):
        if inputs[kind]['path'] != str(artifact_paths[kind]) or packet[kind] != paper[kind]:
            raise ValueError('annotated artifact path differs from the frozen corpus receipt')
    index_path = safe_file(data_root, mapped['index']['path'])
    if inputs['reading_index']['path'] != str(index_path) or inputs['packet']['path'] != str(bundle_root / 'packet.json'):
        raise ValueError('annotator packet/index path differs from the verified input')
    serializer = receipt['serializer']
    serializer_relative = str(Path(serializer['path']).relative_to(cache))
    actual, count = fingerprint_file(safe_file(cache, serializer_relative), cap=32 * 1024 * 1024)
    if actual != serializer['sha256'] or count != serializer['bytes']:
        raise ValueError('independent annotation serializer differs from its receipt')
    if receipt['annotation']['path'] != str(bundle_root / 'independent-v1.json'):
        raise ValueError('independent receipt names another annotation path')
    images = inputs['original_images'] + packet['images'] + root['page_images'] + receipt['detail_crops']
    if root.get('supplemental_image'):
        images.append(root['supplemental_image'])
    verified = {}
    for image in images:
        declared = image['path']
        if not isinstance(declared, str):
            raise ValueError('manual image path must be a string')
        path = Path(declared)
        image_root = cache if path.is_absolute() else bundle_root
        relative = str(path.relative_to(cache)) if path.is_absolute() else declared
        actual, _ = fingerprint_file(safe_file(image_root, relative), cap=32 * 1024 * 1024)
        if actual != image['sha256']:
            raise ValueError('manual original/detail image differs from the declared path hash')
        verified[declared] = actual
    if artifact_paths['source'].stat().st_size > 64 * 1024 * 1024:
        raise ValueError('manual source archive exceeds its compressed byte bound')
    return apply_object_overlay(candidate_raw, bounded(data_root, mapped['index']['path']), artifact_paths['source'].read_bytes(), *raws, verified)


def attach_bibliography(cache, corpus_root, data_root, candidate_raw, paper, mapped, bundle_relative):
    from bibliography_annotations import apply_bibliography_overlay
    candidate = document(candidate_raw)
    if candidate['index'] != mapped['index'] or candidate['paper_id'] != mapped['paper_id']:
        raise ValueError('bibliography candidate differs from frozen mapped identities')
    bundle_root = safe_file(cache, bundle_relative + '/bibliography-packet.json').parent
    names = ('packet.json', 'bibliography-packet.json', 'bibliography-root-v1.json', 'bibliography-independent-v1.json', 'bibliography-independent-v1-receipt.json', 'bibliography-reconciliation-independent-v1.json')
    raws = [bounded(bundle_root, name) for name in names]
    _, packet, root, independent, receipt, review = [document(raw) for raw in raws]
    inputs = independent['inputs']
    for kind in ('pdf', 'source'):
        expected = paper[kind]; path = safe_file(corpus_root, expected['path'])
        actual, count = fingerprint_file(path)
        if actual != expected['sha256'] or count != expected['bytes'] or actual != candidate[kind + '_sha256'] or inputs[kind]['path'] != str(path) or packet[kind] != expected:
            raise ValueError('bibliography artifact differs from actual frozen source/PDF')
    index_path = safe_file(data_root, mapped['index']['path'])
    if inputs['reading_index']['path'] != str(index_path) or inputs['packet']['path'] != str(bundle_root / 'bibliography-packet.json') or receipt['annotation']['path'] != str(bundle_root / 'bibliography-independent-v1.json'):
        raise ValueError('bibliography evidence paths differ from verified inputs')
    for declared in [receipt['serializer'], review['review_script'], inputs['bounded_archive_module']]:
        relative = str(Path(declared['path']).relative_to(cache))
        actual, count = fingerprint_file(safe_file(cache, relative), cap=32 * 1024 * 1024)
        if actual != declared['sha256'] or count != declared['bytes']:
            raise ValueError('bibliography review implementation differs from its receipt')
    for declared in review['input_revalidation']:
        path = Path(declared['path'])
        owner = corpus_root if path.is_relative_to(corpus_root) else cache
        actual, count = fingerprint_file(safe_file(owner, str(path.relative_to(owner))))
        if actual != declared['sha256'] or count != declared['bytes']:
            raise ValueError('bibliography reviewed input changed')
    images = packet['images'] + inputs['images'] + [root['bibliography_detail_image']] + receipt['detail_images']
    verified = {}
    for image in images:
        declared = image['path']; path = Path(declared)
        owner = cache if path.is_absolute() else bundle_root
        relative = str(path.relative_to(cache)) if path.is_absolute() else declared
        actual, count = fingerprint_file(safe_file(owner, relative), cap=32 * 1024 * 1024)
        if actual != image['sha256'] or ('bytes' in image and count != image['bytes']):
            raise ValueError('bibliography image differs from its actual declared path')
        verified[declared] = actual
    source_path = safe_file(corpus_root, paper['source']['path'])
    if source_path.stat().st_size > 64 * 1024 * 1024:
        raise ValueError('bibliography source exceeds compressed byte bound')
    return apply_bibliography_overlay(candidate_raw, bounded(data_root, mapped['index']['path']), source_path.read_bytes(), *raws, verified)


def assemble(cache, corpus_root, data_root, candidate_relative, region_relative, panel_relative=None,
             inputs_relative='k1-full-eval-inputs.json', indexes_relative='k1-full-index.json', object_relative=None, bibliography_relative=None):
    inputs_raw, indexes_raw = bounded(cache, inputs_relative), bounded(cache, indexes_relative)
    inputs, indexes = document(inputs_raw), document(indexes_raw)
    candidate_raw = bounded(cache, candidate_relative)
    candidate = document(candidate_raw)
    papers = [paper for paper in inputs['papers'] if paper['arxiv_id'] == candidate['arxiv_id']]
    mappings = [paper for paper in indexes['papers'] if paper['paper_id'] == candidate['paper_id']]
    if len(papers) != 1 or len(mappings) != 1:
        raise ValueError('manual candidate lacks unique frozen paper and mapped identities')
    if bool(region_relative) == bool(object_relative):
        raise ValueError('exactly one manual region/object bundle is required')
    if object_relative:
        if panel_relative:
            raise ValueError('figure-ranking panel requires a figure/table region bundle')
        output = attach_objects(cache, corpus_root, data_root, candidate_raw, papers[0], mappings[0], object_relative)
    else:
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
    if bibliography_relative:
        bibliography = attach_bibliography(cache, corpus_root, data_root, candidate_raw, papers[0], mappings[0], bibliography_relative)
        output['manual_bibliography_overlay'] = bibliography['manual_bibliography_overlay']
    output['manual_assembly'] = {'inputs_sha256': sha256(inputs_raw), 'index_map_sha256': sha256(indexes_raw),
                                 'candidate_sha256': sha256(candidate_raw), 'final_k1_publication': False,
                                 'source_inventory_policy': 'exact historical candidate retained; no automatic-confidence change'}
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', required=True)
    bundles = parser.add_mutually_exclusive_group(required=True)
    bundles.add_argument('--region-bundle')
    bundles.add_argument('--object-bundle')
    parser.add_argument('--panel-bundle')
    parser.add_argument('--bibliography-bundle')
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
    result = assemble(cache, corpus_root, data_root, args.candidate, args.region_bundle, args.panel_bundle, args.inputs, args.indexes, args.object_bundle, args.bibliography_bundle)
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
