#!/usr/bin/env python3
"""Build independent K1 candidates offline from frozen corpus receipts.

A pilot always stays external. Publication requires the final coverage contract.
"""
import argparse
from collections import Counter, defaultdict, deque
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts/corpus'))
import corpus

from align import align_paper, normalized
from archive import Limits, UnsupportedSource, read_archive, sha256
from parser import argument_commands, parse_project
from tex import Renderer, comments, definition_regions, expand_project, local_style_dependencies, main_candidates, mask_regions

VERSION = 'k1-latex-v1'
POLICY = {'version': VERSION, 'seed': 'k1-latex-strata-v1', 'target_papers': 500, 'alignment_threshold': 0.95,
          'source_limits': vars(Limits()), 'main_selection': 'unique root or byte-equivalent resolved closure; title substring overlap is diagnostic only',
          'bibliography_inventory': 'all deposited entries and printed source citation occurrences must align',
          'region_truth': 'independent full-region annotations only; caption token geometry is never full-region truth'}
IMPLEMENTATION = ['scripts/truth/latex/' + name for name in ('archive.py', 'tex.py', 'parser.py', 'align.py', 'builder.py', 'annotations.py', 'object_annotations.py', 'bibliography_annotations.py', 'native_exports.py', 'panel.py', 'manual.py', 'release.py')] + ['examples/k1_index.rs', 'src/library.rs', 'src/store.rs', 'src/domain.rs', 'src/source_index.rs', 'src/source_index/cache.rs', 'src/source_index/native.rs', 'src/source_index/paragraphs.rs', 'src/source_index/ocr.rs', 'src/source_index/figures.rs', 'scripts/corpus/corpus.py', 'scripts/corpus/selection.json', 'Cargo.toml', 'Cargo.lock']


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


def atomic_json(path, value):
    if path.is_symlink() or any(parent.is_symlink() for parent in path.parents):
        raise ValueError('derived evidence paths cannot contain symlinks')
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode='wb', dir=path.parent, prefix=path.name + '.', delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(canonical(value) + b'\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)


def safe_file(root, relative):
    if not isinstance(relative, str) or '\\' in relative or '\0' in relative or any(part in ('', '.', '..') for part in relative.split('/')):
        raise ValueError('unsafe evidence path')
    path = Path(relative)
    if path.is_absolute() or not path.parts or any(part in ('.', '..', '.env', '.secrets') or part.startswith('.env.') for part in path.parts):
        raise ValueError('unsafe evidence path')
    candidate = root
    for part in path.parts:
        candidate /= part
        if candidate.is_symlink():
            raise ValueError('evidence paths cannot contain symlinks')
    resolved = candidate.resolve(strict=True)
    if not resolved.is_relative_to(root.resolve()) or not resolved.is_file():
        raise ValueError('evidence path escapes its root or is not a file')
    return resolved


def fingerprint_file(path, cap=512 * 1024 * 1024):
    digest, total = hashlib.sha256(), 0
    with path.open('rb') as stream:
        while chunk := stream.read(1024 * 1024):
            total += len(chunk)
            if total > cap:
                raise ValueError('evidence file exceeds its byte bound')
            digest.update(chunk)
    return digest.hexdigest(), total


def fingerprint_sources():
    return {name: sha256(safe_file(ROOT, name).read_bytes()) for name in IMPLEMENTATION}


def ordered_papers(papers, seed):
    strata = defaultdict(list)
    for paper in papers:
        strata[tuple(paper['stratum'])].append(paper)
    categories = defaultdict(list)
    for stratum in sorted(strata):
        queue = sorted(strata[stratum], key=lambda paper: sha256((seed + '\0' + paper['arxiv_id']).encode()))
        categories[stratum[0]].append(deque(queue))
    queues = []
    for category in sorted(categories):
        queue = deque()
        while any(categories[category]):
            for year in categories[category]:
                if year:
                    queue.append(year.popleft())
        queues.append(queue)
    output = []
    while any(queues):
        for queue in queues:
            if queue:
                output.append(queue.popleft())
    return output


def freeze_inputs(root, config, limit=None):
    raw_selection = safe_file(root, 'selection.json').read_bytes()
    selection = json.loads(raw_selection)
    corpus.validate_selection(selection, config)
    corpus.validate_availability(root, selection)
    manifest = corpus.read_manifest(root)
    complete, missing = [], []
    selected = [paper for paper in selection['papers'] if 'eval' in paper['tiers']]
    for paper in selected:
        entry = manifest.get(paper['id'])
        if not entry or not entry.get('pdf') or not entry.get('source'):
            missing.append(paper['id'])
            continue
        corpus.validate_entry(entry, paper, selection['selection_sha256'])
        row = {'arxiv_id': paper['id'], 'version': entry['version'], 'stratum': paper['strata']['eval'],
               'title': paper['title'], 'arxiv_url': paper['arxiv_url']}
        for kind in ('pdf', 'source'):
            receipt = entry[kind]
            corpus.validate_receipt(entry, kind, receipt)
            path = safe_file(root, receipt['path'])
            sidecar = json.loads(safe_file(root, receipt['path'] + '.verified.json').read_text())
            if {**sidecar, 'path': receipt['path']} != receipt:
                raise ValueError('manifest and per-file receipt disagree')
            if path.stat().st_size != receipt['bytes']:
                raise ValueError('admitted corpus file size changed')
            row[kind] = receipt
        complete.append(row)
    ordered = ordered_papers(complete, POLICY['seed'])
    if limit is not None:
        ordered = ordered[:limit]
    return {'schema_version': 1, 'selection_sha256': selection['selection_sha256'],
            'selection_file_sha256': sha256(raw_selection), 'config_sha256': selection['config_sha256'],
            'metadata_sha256': selection['metadata_sha256'], 'expected_eval_papers': len(selected),
            'available_pairs': len(complete), 'missing_pairs': missing, 'papers': ordered,
            'policy': POLICY, 'policy_sha256': sha256(canonical(POLICY))}


def choose_main(files, index, members=None):
    candidates = main_candidates(files)
    if len(candidates) == 1:
        return None, {'method': 'unique document root', 'selected': candidates[0], 'candidates': candidates}
    if not candidates:
        raise UnsupportedSource('no deposited document root')
    first = next((page for page in index.get('pages', []) if page['number'] == 1), None)
    first_text = index['text'].encode('utf-16-le')[2 * first['start']:2 * first['end']].decode('utf-16-le') if first else ''
    pdf_title_text = normalized(first_text)
    matches, titles = [], {}
    for name in candidates:
        text = comments(files[name])
        text = mask_regions(text, definition_regions(text))
        title_rows = list(argument_commands(text, {'title', 'TITLE'}))
        if len(title_rows) == 1:
            renderer = Renderer(text)
            title = renderer.plain(title_rows[0]['value'])
            titles[name] = title
            if not renderer.unsupported and len(normalized(title)) >= 20 and normalized(title) in pdf_title_text:
                matches.append(name)
    # First-page substring matches are diagnostic only: a shorter source title
    # may occur inside a different printed title (including added negation).
    # Without independently bounded title/source evidence, do not select a
    # non-equivalent root from that overlap.
    # Equivalent roots must expand to identical text and have identical deposited
    # bibliography content; a same-named or same-title file alone is insufficient.
    closures = {}
    for name in candidates:
        try:
            expanded = expand_project(files, selected_main=name)
            if local_style_dependencies(files, name, expanded.text):
                raise UnsupportedSource('local package/class semantics prevent complete root equivalence')
            bib = sorted(sha256(files[path].encode()) for path in expanded.coverage['bibliography_files'])
            resources = []
            inventory = {row['path']: row['sha256'] for row in members or []}
            for command in argument_commands(expanded.text, {'includegraphics'}):
                value = command['value']
                if '\\' in value or any(part in ('', '.', '..') for part in value.split('/')) or value.startswith('/'):
                    raise UnsupportedSource('dynamic or unsafe graphics resource')
                origins = expanded.origins(command['start'], command['end'])
                if not origins:
                    raise UnsupportedSource('graphics resource has no source origin')
                base = Path(origins[0]['path']).parent
                choices = {str(base / value), value}
                if not Path(value).suffix:
                    choices = {path + suffix for path in choices for suffix in ('.pdf', '.png', '.jpg', '.jpeg', '.eps')}
                present = choices & inventory.keys()
                if len(present) != 1:
                    raise UnsupportedSource('graphics resource is absent or ambiguous')
                resources.append(inventory[present.pop()])
            closures[name] = sha256(canonical({'expanded_text': expanded.text, 'bib_members': bib, 'graphics': resources}))
        except UnsupportedSource:
            closures[name] = None
    if None not in closures.values() and len(set(closures.values())) == 1:
        return candidates[0], {'method': 'identical fully resolved source and bibliography closures', 'selected': candidates[0], 'equivalent_roots': candidates, 'closure_sha256': next(iter(closures.values())), 'titles': titles}
    raise UnsupportedSource('multiple non-equivalent document roots lack unique PDF evidence: ' + ', '.join(candidates))


def build_backend():
    environment = dict(os.environ, CARGO_BUILD_JOBS='1', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0', CARGO_INCREMENTAL='0')
    build = subprocess.run(['cargo', 'build', '--offline', '--example', 'k1_index', '--target-dir', str(ROOT / 'target'), '--message-format=json'], cwd=ROOT, env=environment, capture_output=True, text=True, check=True)
    choices = set()
    for line in build.stdout.splitlines():
        row = json.loads(line)
        target = row.get('target', {})
        if row.get('reason') == 'compiler-artifact' and target.get('name') == 'k1_index' and target.get('kind') == ['example'] and row.get('executable') and Path(target['src_path']).resolve() == ROOT / 'examples/k1_index.rs':
            choices.add(Path(row['executable']).resolve(strict=True))
    if len(choices) != 1:
        raise ValueError('Cargo did not identify one exact K1 indexing executable')
    executable = choices.pop()
    return executable, sha256(executable.read_bytes())


def derive_paper(paper, mapped, root, data_root):
    for kind in ('pdf', 'source'):
        path = safe_file(root, paper[kind]['path'])
        actual, count = fingerprint_file(path)
        if actual != paper[kind]['sha256'] or count != paper[kind]['bytes']:
            raise ValueError('frozen corpus artifact bytes changed')
    index_path = safe_file(data_root, mapped['index']['path'])
    raw_index = index_path.read_bytes()
    if sha256(raw_index) != mapped['index']['sha256'] or mapped['pdf_sha256'] != paper['pdf']['sha256']:
        raise ValueError('index or mapped source fingerprint changed')
    index = json.loads(raw_index)['index']
    source_path = safe_file(root, paper['source']['path'])
    raw_source = source_path.read_bytes()
    if sha256(raw_source) != paper['source']['sha256'] or len(raw_source) != paper['source']['bytes']:
        raise ValueError('source bytes read for parsing disagree with the frozen receipt')
    files, members = read_archive(raw_source)
    selected, main_evidence = choose_main(files, index, members)
    parsed = parse_project(files, selected_main=selected)
    result = align_paper(parsed, index, POLICY['alignment_threshold'])
    result['source_inventory'] = parsed
    result['source_inventory_sha256'] = sha256(canonical(parsed))
    if sha256(index_path.read_bytes()) != mapped['index']['sha256']:
        raise ValueError('index bytes changed during alignment')
    result.update(arxiv_id=paper['arxiv_id'], paper_id=mapped['paper_id'], pdf_sha256=paper['pdf']['sha256'],
                  source_sha256=paper['source']['sha256'], index=mapped['index'], stratum=paper['stratum'],
                  main_evidence=main_evidence, source_members=members)
    return result


def attach_manual_regions(candidate_raw, paper, mapped, root, data_root, bundle_root):
    """Rehash every external input before validating a reviewed manual overlay."""
    from annotations import apply_regions, document
    candidate = document(candidate_raw)
    if candidate['arxiv_id'] != paper['arxiv_id'] or candidate['paper_id'] != mapped['paper_id'] or candidate['index'] != mapped['index']:
        raise ValueError('manual candidate differs from frozen paper/index identities')
    for kind in ('pdf', 'source'):
        expected = paper[kind]
        actual, count = fingerprint_file(safe_file(root, expected['path']))
        if candidate[kind + '_sha256'] != actual or actual != expected['sha256'] or count != expected['bytes']:
            raise ValueError('manual source/PDF bytes differ from the frozen receipts')
    index_path = safe_file(data_root, mapped['index']['path'])
    if index_path.stat().st_size > 32 * 1024 * 1024:
        raise ValueError('manual index exceeds its byte bound')
    index_raw = index_path.read_bytes()
    names = ('regions-root-v1.json', 'review-independent-v1.json', 'source-associations-root-v1.json', 'source-associations-independent-review-v1.json')
    receipts = []
    for name in names:
        path = safe_file(bundle_root, name)
        if path.stat().st_size > 32 * 1024 * 1024:
            raise ValueError('manual receipt exceeds its byte bound')
        receipts.append(path.read_bytes())
    annotation = document(receipts[0])
    for relative, expected in annotation['page_render_sha256'].items():
        if fingerprint_file(safe_file(bundle_root, relative), cap=32 * 1024 * 1024)[0] != expected:
            raise ValueError('manual page render differs from the reviewed bytes')
    return apply_regions(candidate_raw, index_raw, *receipts)


def publication_problems(snapshot, results):
    accepted = [row for row in results if row.get('accepted')]
    problems = []
    if snapshot['missing_pairs']:
        problems.append('full eval source/PDF tier is not yet available')
    if len(accepted) < POLICY['target_papers']:
        problems.append('fewer than 500 papers satisfy the independent alignment policy')
    kinds = {kind for row in accepted for kind in row['eligible_kinds'] if row['kind_coverage'][kind]['expected'] > 0}
    if not {'figure', 'table', 'equation', 'statement', 'proof', 'algorithm'} <= kinds or not any(row.get('bibliography_eligible') and row['entries'] for row in accepted):
        problems.append('accepted truth does not cover every E1 object kind')
    strata = {tuple(row['stratum']) for row in accepted}
    if not {tuple(row['stratum']) for row in snapshot['papers']} <= strata:
        problems.append('an input category/year stratum has no accepted paper')
    problems.append('three independent O11 panel judgments and full-region annotation receipt not attached')
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--corpus-root', type=Path, default=Path.home() / 'Corpora/arxiv')
    parser.add_argument('--data-root', type=Path, default=Path.home() / '.cache/lysilogy/arxiv-kb-data')
    parser.add_argument('--pilot', type=int, help='bounded exploratory paper count; never publishes K1')
    args = parser.parse_args()
    if args.pilot is not None and not 1 <= args.pilot <= 1000:
        raise ValueError('pilot size must be between 1 and 1000')
    started = time.monotonic()
    corpus_root = args.corpus_root.expanduser().resolve(strict=True)
    allowed_corpus = (Path.home() / 'Corpora').resolve(strict=True)
    allowed_cache = (Path.home() / '.cache/lysilogy').resolve(strict=True)
    if not corpus_root.is_relative_to(allowed_corpus):
        raise ValueError('corpus must remain under ~/Corpora')
    # The parent exists before creation, so a symlink cannot create cache files
    # outside the explicitly permitted cache root before canonical validation.
    if not args.data_root.parent.resolve(strict=True).is_relative_to(allowed_cache):
        raise ValueError('data root must remain under ~/.cache/lysilogy')
    args.data_root.mkdir(exist_ok=True)
    data_root = args.data_root.resolve(strict=True)
    if not data_root.is_relative_to(allowed_cache):
        raise ValueError('data root resolves outside the cache root')
    implementation = fingerprint_sources()
    config = corpus.load_config(ROOT / 'scripts/corpus/selection.json')
    snapshot = freeze_inputs(corpus_root, config, args.pilot)
    if not snapshot['papers']:
        raise ValueError('no fully admitted eval PDF/source pair is available')
    if args.pilot is None and snapshot['missing_pairs']:
        raise ValueError('complete K1 build waits for the full eval tier; use an explicit pilot for exploratory work')
    build_id = sha256(canonical({'snapshot': snapshot, 'implementation': implementation}))
    out = allowed_cache / 'k1-builds' / build_id
    if out.is_symlink() or out.parent.is_symlink():
        raise ValueError('derived build directories cannot be symlinks')
    out.mkdir(parents=True, exist_ok=True)
    atomic_json(out / 'inputs.json', snapshot)
    executable, executable_hash = build_backend()
    request = {'corpus_pdf_root': str(corpus_root / 'pdf'), 'data_root': str(data_root),
               'papers': [{'relative_path': Path(row['pdf']['path']).name, 'pdf_sha256': row['pdf']['sha256']} for row in snapshot['papers']]}
    run = subprocess.run([str(executable)], input=json.dumps(request), capture_output=True, text=True, check=True)
    mapped = json.loads(run.stdout)
    atomic_json(out / 'indexes.json', mapped)
    by_path = {row['relative_path']: row for row in mapped['papers']}
    results = []
    for paper in snapshot['papers']:
        began = time.monotonic()
        index = by_path[Path(paper['pdf']['path']).name]
        try:
            if index.get('error'):
                raise UnsupportedSource('native index build failed: ' + index['error'])
            result = derive_paper(paper, index, corpus_root, data_root)
        except (UnsupportedSource, ValueError) as error:
            result = {'arxiv_id': paper['arxiv_id'], 'stratum': paper['stratum'], 'accepted': False,
                      'bibliography_eligible': False, 'exclusion': str(error)}
        result['wall_seconds'] = time.monotonic() - began
        atomic_json(out / 'papers' / (paper['arxiv_id'] + '.json'), result)
        results.append(result)
        print(json.dumps({'paper': paper['arxiv_id'], 'accepted': result['accepted'], 'bibliography_eligible': result['bibliography_eligible'], 'quality': result.get('alignment', {}).get('quality'), 'exclusion': result.get('exclusion')}), flush=True)
    if fingerprint_sources() != implementation or sha256(executable.read_bytes()) != executable_hash:
        raise ValueError('implementation or executable changed during truth construction')
    problems = publication_problems(snapshot, results)
    report = {'schema_version': 1, 'version': VERSION, 'build_id': build_id, 'built_at': datetime.now(timezone.utc).isoformat(),
              'mode': 'exploratory' if args.pilot is not None else 'incomplete-final-build', 'inputs_sha256': sha256(canonical(snapshot) + b'\n'),
              'implementation': implementation, 'executable_sha256': executable_hash, 'coverage': {'input_papers': len(results),
                  'accepted_papers': sum(row['accepted'] for row in results), 'bibliography_papers': sum(row['bibliography_eligible'] for row in results)},
              'publication_problems': problems, 'wall_seconds': time.monotonic() - started, 'model_calls': 0, 'network_calls': 0, 'model_cost_usd': 0}
    atomic_json(out / 'report.json', report)
    print(json.dumps({'report': str(out / 'report.json'), 'coverage': report['coverage'], 'publication_problems': problems}), flush=True)
    # Publication will be added only with independently reviewed panel receipts.
    return 0 if args.pilot is not None else 1


if __name__ == '__main__':
    raise SystemExit(main())
