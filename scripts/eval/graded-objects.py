#!/usr/bin/env python3
"""Queue, seed, export and measure human-graded figure/table truth (k1-graded).

Scoring reuses ``evaluate_paper`` from ``object-metrics.py`` unchanged; this
script only builds the truth children that function expects and publishes the
harness input. See ``eval/graded-objects-contract.md``.
"""
import argparse
import datetime
import importlib.util
import json
import math
import random
import re
import statistics
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COLLECTOR = 'graded-objects-v1'
TRUTH_VERSION = 'k1-graded'
LAYOUT = 'k1-graded-v1'
TRUTH_DIR = 'eval/truth/k1-graded'
MANIFEST = TRUTH_DIR + '/objects.json'
OBSERVATION = 'eval/inputs/evidence/graded-objects.json'
INPUT = 'eval/inputs/objects/figure-table.json'
KINDS = ('figure', 'table')
VERDICTS = ('correct', 'region', 'reject')
FIELDS = ('cs', 'econ', 'math', 'other', 'physics', 'stat')
PHYSICS_ARCHIVES = {'astro-ph', 'cond-mat', 'gr-qc', 'hep-ex', 'hep-lat', 'hep-ph', 'hep-th', 'math-ph',
                    'nlin', 'nucl-ex', 'nucl-th', 'physics', 'quant-ph'}
RECT_KEYS = ('x_min', 'y_min', 'x_max', 'y_max')
IMPLEMENTATION_FILES = ('Cargo.lock', 'Cargo.toml', 'examples/object_metrics.rs', 'scripts/eval/graded-objects.py',
                        'scripts/eval/object-metrics.py', 'src/domain.rs', 'src/layout.rs', 'src/library.rs',
                        'src/source_index.rs')
IMPLEMENTATION_TREES = ('src/objects', 'src/source_index')
PDF_NAME = re.compile(r'(?P<arxiv_id>.+)v(?P<version>[0-9]+)\.pdf')
SELECTION_BYTES = 256 * 1024 * 1024


def load_object_metrics():
    path = Path(__file__).with_name('object-metrics.py')
    spec = importlib.util.spec_from_file_location('object_metrics', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


object_metrics = load_object_metrics()
require = object_metrics.require
digest = object_metrics.digest
canonical = object_metrics.canonical
read = object_metrics.read
document = object_metrics.document
atomic_json = object_metrics.atomic_json


# --- registry -----------------------------------------------------------------

def load_registry(data):
    registry = document(read(data / 'paper-identities.json'))
    require(registry.get('schema_version') == 1 and isinstance(registry.get('records'), dict), 'unsupported paper registry')
    return registry


def identity(registry, paper_id):
    """arxiv_id, arxiv_version and pdf_sha256 of an active registry record, else None."""
    record = registry['records'].get(paper_id)
    if not isinstance(record, dict) or record.get('active') is not True:
        return None
    match = PDF_NAME.fullmatch(record.get('relative_path', ''))
    if match is None:
        return None
    return {'arxiv_id': match['arxiv_id'], 'arxiv_version': int(match['version']), 'pdf_sha256': record['content_hash']}


def registry_by_arxiv_id(registry):
    result = {}
    for paper_id in registry['records']:
        ident = identity(registry, paper_id)
        if ident is not None:
            result.setdefault(ident['arxiv_id'], []).append((ident['arxiv_version'], paper_id))
    return result


# --- queue --------------------------------------------------------------------

def field_of(category):
    archive = category.split('.')[0]
    if archive == 'math':
        return 'math'
    if archive == 'cs':
        return 'cs'
    if archive in PHYSICS_ARCHIVES:
        return 'physics'
    if archive == 'stat':
        return 'stat'
    if archive in ('econ', 'q-fin'):
        return 'econ'
    return 'other'


def build_queue(selection, registry, tier, seed, limit=None):
    """Seeded shuffle inside each field group, then round-robin so every prefix is stratified."""
    versions = registry_by_arxiv_id(registry)
    groups = {field: [] for field in FIELDS}
    skipped = []
    for paper in selection['papers']:
        if tier not in paper.get('tiers', []):
            continue
        candidates = versions.get(paper['id'])
        if not candidates:
            skipped.append(paper['id'])
            continue
        wanted = (paper.get('remote_pdf') or {}).get('version')
        chosen = next((paper_id for version, paper_id in candidates if version == wanted), max(candidates)[1])
        stratum = paper.get('strata', {}).get(tier) or paper.get('categories') or ['']
        field = field_of(stratum[0])
        groups[field].append({'paper_id': chosen, 'arxiv_id': paper['id'], 'stratum': field})
    rng = random.Random(seed)
    for field in FIELDS:
        groups[field].sort(key=lambda row: row['arxiv_id'])
        rng.shuffle(groups[field])
    ordered = []
    for position in range(max((len(group) for group in groups.values()), default=0)):
        for field in FIELDS:
            if position < len(groups[field]):
                ordered.append(groups[field][position])
    if limit is not None:
        ordered = ordered[:limit]
    return {'schema_version': 1, 'seed': seed, 'tier': tier, 'papers': ordered}, skipped


def queue(data, selection_path, tier, seed, limit=None):
    selection = document(read(selection_path, SELECTION_BYTES))
    queue_document, skipped = build_queue(selection, load_registry(data), tier, seed, limit)
    atomic_json(data / 'grading-queue.json', queue_document)
    strata = {}
    for row in queue_document['papers']:
        strata[row['stratum']] = strata.get(row['stratum'], 0) + 1
    return {'path': str(data / 'grading-queue.json'), 'papers': len(queue_document['papers']),
            'strata': strata, 'skipped_unregistered': len(skipped)}


# --- truth children -----------------------------------------------------------

def child_path(repo, paper_id):
    return repo / TRUTH_DIR / 'papers' / (paper_id + '.json')


def inventory(objects):
    counts = {kind: sum(o['kind'] == kind for o in objects) for kind in KINDS}
    return {kind: n for kind, n in counts.items() if n}, [kind for kind in KINDS if not counts[kind]]


def rect(value):
    if not isinstance(value, dict) or not all(key in value for key in RECT_KEYS):
        return None
    if not all(type(value[key]) in (int, float) and math.isfinite(value[key]) for key in RECT_KEYS):
        return None
    return {key: value[key] for key in RECT_KEYS}


def truth_object(ident, kind, printed_label, span, page, box):
    return {'id': ident, 'kind': kind, 'printed_label': printed_label,
            'spans': [{'start': span['start'], 'end': span['end']}],
            'region': [{'page': page, 'rect': box}]}


def printed_label(value, kind):
    """The detector label without its kind prefix, keeping the printed case (``IV`` stays ``IV``)."""
    if not isinstance(value, str):
        return None
    prefix = r'(?:fig(?:ure)?\.?)' if kind == 'figure' else r'table'
    stripped = re.sub(r'^' + prefix + r'\s*', '', value.strip(), flags=re.I).strip(' .:()')
    return stripped if object_metrics.label(stripped, kind) is not None else None


def seed_release(repo, source):
    """Import the figure/table cohort of an earlier release as children with its provenance."""
    source = Path(source) if Path(source).is_absolute() else repo / source
    manifest_raw = read(source / 'objects.json')
    manifest = document(manifest_raw)
    written, kept, dropped = [], [], 0
    for row in manifest['papers']:
        eligibility = row['metric_eligibility']
        if not (eligibility.get('O1') or eligibility.get('O2')):
            continue
        raw = read(source / row['path'])
        require(digest(raw) == row['sha256'], 'release child differs from its manifest')
        paper = document(raw)
        target = child_path(repo, paper['paper_id'])
        if target.exists() and document(read(target)).get('provenance', {}).get('source') == 'graded':
            kept.append(paper['paper_id'])
            continue
        objects = []
        for source_object in paper['objects']:
            if source_object['kind'] not in KINDS:
                continue
            spans = source_object.get('spans')
            regions = source_object.get('region')
            if isinstance(regions, dict):
                regions = [regions]  # earlier releases stored a singleton {page, rect}
            if not spans or not (isinstance(regions, list) and regions):
                dropped += 1
                continue
            objects.append({'id': source_object['id'], 'kind': source_object['kind'],
                            'printed_label': source_object['printed_label'],
                            'spans': [{'start': span['start'], 'end': span['end']} for span in spans],
                            'region': [{'page': region['page'], 'rect': rect(region['rect'])} for region in regions]})
        counts, absent = inventory(objects)
        child = {'paper_id': paper['paper_id'], 'arxiv_id': paper['arxiv_id'], 'arxiv_version': paper['arxiv_version'],
                 'pdf_sha256': paper['pdf_sha256'], 'index': {'path': paper['index']['path'], 'sha256': paper['index']['sha256']},
                 'provenance': {'source': manifest['version'], 'release_sha256': digest(manifest_raw), 'child_sha256': row['sha256']},
                 'metric_eligibility': {'O1': True, 'O2': all(o['region'] for o in objects)},
                 'counts': counts, 'reviewed_absent_kinds': absent, 'objects': objects}
        atomic_json(target, child)
        written.append(paper['paper_id'])
    coverage = rebuild_manifest(repo)['coverage']
    return {'source': manifest['version'], 'written': written, 'kept_graded': kept, 'dropped_objects': dropped, 'coverage': coverage}


def derive_truth(grades, artifact):
    """Truth objects from one complete grades document; blockers name what stops export."""
    blockers, objects = [], []
    verdicts = grades.get('verdicts') or {}
    for detected in artifact['objects']:
        kind = detected.get('kind')
        if kind not in KINDS:
            continue
        ident = detected['id']
        verdict = verdicts.get(ident)
        if not isinstance(verdict, dict):
            blockers.append('missing_verdict:' + ident)
            continue
        decision = verdict.get('verdict')
        if decision == 'reject':
            continue
        if decision not in VERDICTS:
            blockers.append('unknown_verdict:' + ident)
            continue
        box = rect(detected.get('region') if decision == 'correct' else verdict.get('region'))
        if box is None:
            blockers.append(('correct_without_region:' if decision == 'correct' else 'region_without_box:') + ident)
            continue
        label = printed_label(detected.get('label'), kind)
        if label is None:
            blockers.append('unparseable_label:' + ident)
            continue
        objects.append(truth_object('graded:' + ident, kind, label, detected['anchor'], detected['page'], box))
    for addition in grades.get('additions') or []:
        ident = str(addition.get('id'))
        caption = addition.get('caption')
        if not isinstance(caption, dict):
            blockers.append('addition_without_caption:' + ident)
            continue
        if addition.get('kind') not in KINDS:
            blockers.append('addition_kind:' + ident)
            continue
        box = rect(addition.get('region'))
        if box is None:
            blockers.append('addition_without_region:' + ident)
            continue
        objects.append(truth_object('graded:' + ident, addition['kind'], addition.get('printed_label'), caption, addition.get('page'), box))
    if len({o['id'] for o in objects}) != len(objects):
        blockers.append('duplicate_truth_id')
    return objects, blockers


def cached_artifact(data, paper_id, index_sha256):
    path = data / 'papers' / paper_id / 'objects.json'
    if not path.is_file():
        return None
    artifact = document(read(path))
    if artifact.get('reading_index_generation') != '"' + index_sha256 + '"':
        return None
    return artifact


def bridge_predictions(executable, repo, corpus, data, papers):
    """One production bridge run; returns {paper_id: objects artifact}."""
    if not papers:
        return {}
    request = {'corpus_root': str(corpus / 'pdf'), 'data_root': str(data), 'papers': papers}
    result = subprocess.run([str(executable)], input=canonical(request), cwd=repo, capture_output=True,
                            timeout=max(60, 40 * len(papers)))
    require(result.returncode == 0, 'bridge failed: ' + result.stderr.decode(errors='replace')[-2000:])
    require(len(result.stdout) <= object_metrics.MAX_JSON, 'bridge response too large')
    response = document(result.stdout)
    require(response.get('schema_version') == 1 and response.get('network_calls') == 0 and response.get('model_calls') == 0,
            'invalid bridge receipt')
    require([row['paper_id'] for row in response['papers']] == [p['paper_id'] for p in papers], 'bridge omitted or duplicated a paper')
    artifacts = {}
    for wanted, row in zip(papers, response['papers']):
        require(row['index_sha256'] == wanted['index_sha256'], 'bridge index differs')
        raw = row['artifact_json'].encode()
        require(digest(raw) == row['object_sha256'], 'object artifact hash differs')
        artifacts[wanted['paper_id']] = document(raw)
    return artifacts


def export(repo, data, corpus, executable=None):
    """Derive one child per complete grades file, then rebuild the release manifest."""
    registry = load_registry(data)
    skipped, candidates, partial = {}, [], 0
    for grades_path in sorted((data / 'papers').glob('*/objects-grades.json')) if (data / 'papers').is_dir() else []:
        paper_id = grades_path.parent.name
        grades_raw = read(grades_path)
        grades = document(grades_raw)
        if grades.get('complete') is not True:
            partial += 1
            continue
        if grades.get('paper_id') != paper_id:
            skipped[paper_id] = ['paper_id_mismatch']
            continue
        ident = identity(registry, paper_id)
        if ident is None:
            skipped[paper_id] = ['unregistered_paper']
            continue
        index_path = data / 'papers' / paper_id / 'reading-index.json'
        if not index_path.is_file():
            skipped[paper_id] = ['missing_index']
            continue
        index_raw = read(index_path)
        if digest(index_raw) != grades.get('index_sha256'):
            skipped[paper_id] = ['stale_index']
            continue
        candidates.append({'paper_id': paper_id, 'grades': grades, 'grades_sha256': digest(grades_raw), 'identity': ident,
                           'index': document(index_raw)['index'], 'index_sha256': grades['index_sha256'],
                           'artifact': cached_artifact(data, paper_id, grades['index_sha256'])})
    pending = [c for c in candidates if c['artifact'] is None]
    if pending and executable is None:
        for candidate in pending:
            skipped[candidate['paper_id']] = ['no_current_objects (rerun export with --executable)']
        candidates = [c for c in candidates if c['artifact'] is not None]
    elif pending:
        request = [{'paper_id': c['paper_id'], 'relative_path': c['identity']['arxiv_id'] + 'v' + str(c['identity']['arxiv_version']) + '.pdf',
                    'index_sha256': c['index_sha256']} for c in pending]
        artifacts = bridge_predictions(executable, repo, corpus, data, request)
        for candidate in pending:
            candidate['artifact'] = artifacts[candidate['paper_id']]
    exported = []
    for candidate in candidates:
        paper_id, grades, artifact = candidate['paper_id'], candidate['grades'], candidate['artifact']
        if artifact.get('paper_id') != paper_id:
            skipped[paper_id] = ['objects_paper_mismatch']
            continue
        objects, blockers = derive_truth(grades, artifact)
        if blockers:
            skipped[paper_id] = blockers
            continue
        counts, absent = inventory(objects)
        child = {'paper_id': paper_id, **candidate['identity'],
                 'index': {'path': 'papers/' + paper_id + '/reading-index.json', 'sha256': candidate['index_sha256']},
                 'provenance': {'source': 'graded', 'grader': grades.get('grader'), 'graded_at': grades.get('updated_at'),
                                'grades_sha256': candidate['grades_sha256'], 'objects_generation': grades.get('objects_generation')},
                 'metric_eligibility': {'O1': True, 'O2': True}, 'counts': counts, 'reviewed_absent_kinds': absent, 'objects': objects}
        try:
            object_metrics.evaluate_paper(child, artifact, candidate['index'])
        except (ValueError, KeyError, TypeError) as error:
            skipped[paper_id] = ['invalid_truth: ' + str(error)]
            continue
        atomic_json(child_path(repo, paper_id), child)
        exported.append(paper_id)
    coverage = rebuild_manifest(repo)['coverage']
    return {'exported': exported, 'skipped': skipped, 'partial': partial, 'coverage': coverage}


def rebuild_manifest(repo):
    """Manifest over every child present; denominators follow evaluate_paper's cohorts."""
    folder = repo / TRUTH_DIR / 'papers'
    rows, cohorts, truth_objects, annotated = [], {'O1': [], 'O2': []}, 0, 0
    for path in sorted(folder.glob('*.json')) if folder.is_dir() else []:
        raw = read(path)
        paper = document(raw)
        paper_id = paper['paper_id']
        require(paper_id == path.stem, 'truth child name differs from its paper')
        eligibility = paper['metric_eligibility']
        objects = [o for o in paper['objects'] if o['kind'] in KINDS]
        require(inventory(objects)[0] == {k: n for k, n in paper['counts'].items() if n}, 'truth child counts differ from its objects')
        rows.append({'paper_id': paper_id, 'path': 'papers/' + path.name, 'sha256': digest(raw), 'bytes': len(raw),
                     'counts': paper['counts'], 'metric_eligibility': eligibility, 'provenance_source': paper['provenance']['source']})
        if eligibility.get('O1') is True:
            cohorts['O1'].append(paper_id)
            truth_objects += len(objects)
        if eligibility.get('O2') is True:
            cohorts['O2'].append(paper_id)
            annotated += sum(1 for o in objects if isinstance(o.get('region'), list) and o['region'])
    manifest = {'schema_version': 1, 'truth_set': 'K1', 'origin': 'graded', 'version': TRUTH_VERSION, 'layout': LAYOUT,
                'build_date': datetime.date.today().isoformat(), 'papers': rows,
                'coverage': {'cohort_papers': cohorts,
                             'denominators': {'O1': {'truth_objects': truth_objects}, 'O2': {'all_annotated_truth_objects': annotated}}}}
    target = repo / MANIFEST
    if target.is_file():
        previous = document(read(target))
        if previous.get('papers') == rows and previous.get('coverage') == manifest['coverage']:
            manifest['build_date'] = previous.get('build_date', manifest['build_date'])
    atomic_json(target, manifest)
    return manifest


# --- measurement --------------------------------------------------------------

def implementation_files(repo):
    files = set(IMPLEMENTATION_FILES)
    for tree in IMPLEMENTATION_TREES:
        folder = repo / tree
        if folder.is_dir():
            files.update(str(p.relative_to(repo)) for p in folder.rglob('*.rs') if p.is_file())
    return sorted(files)


def cohort(repo, truth):
    papers = []
    for row in truth['papers']:
        eligibility = row['metric_eligibility']
        if not (eligibility.get('O1') or eligibility.get('O2')):
            continue
        raw = read(repo / TRUTH_DIR / row['path'])
        require(digest(raw) == row['sha256'], 'truth child differs from its manifest')
        paper = document(raw)
        require(paper['paper_id'] == row['paper_id'] and paper['metric_eligibility'] == eligibility, 'truth child identity differs')
        require(eligibility.get('O1') is True, 'every graded cohort paper must be O1 eligible')
        papers.append(paper)
    for metric in ('O1', 'O2'):
        require([p['paper_id'] for p in papers if p['metric_eligibility'].get(metric) is True] == truth['coverage']['cohort_papers'][metric],
                'manifest cohort differs from its children')
    require(papers, 'graded cohort is empty')
    return papers


def aggregate(truth, results):
    totals = {key: sum(r[key] for r in results) for key in ('tp', 'fp', 'fn', 'truth_objects', 'predictions')}
    values = [v for r in results for v in r['region_values']]
    expected = truth['coverage']['denominators']
    require(totals['truth_objects'] == expected['O1']['truth_objects'] > 0, 'incomplete frozen O1 denominator')
    require(len(values) == expected['O2']['all_annotated_truth_objects'] > 0, 'incomplete frozen O2 denominator')
    return totals, values


def paper_result(paper, artifact, index):
    result = object_metrics.evaluate_paper(paper, artifact, index)
    predictions = [o for o in artifact['objects'] if o['kind'] in KINDS]
    truths = [o for o in paper['objects'] if o['kind'] in KINDS]
    result.update({
        'arxiv_id': paper['arxiv_id'], 'arxiv_version': paper['arxiv_version'],
        'detector': {'version': artifact.get('figure_detector_version'), 'generation': artifact.get('figure_detector_generation')},
        'prediction_rows': [{'position': n, 'id': o['id'], 'kind': o['kind'], 'label': o.get('label'), 'page': o.get('page')}
                            for n, o in enumerate(predictions)],
        'truth_rows': [{'id': o['id'], 'kind': o['kind'], 'printed_label': o['printed_label'],
                        'page': o['region'][0]['page'] if o.get('region') else None} for o in truths]})
    return result


def freeze_foreign_input(repo):
    """Retain a prior input owned by another collector, with its observation, before replacing it."""
    active = repo / INPUT
    if not (active.exists() or active.is_symlink()):
        return None
    old_raw = read(active)
    old = document(old_raw)
    if old.get('collector') == COLLECTOR:
        return None
    trace = old['metrics']['O1']['evidence'][0]['path']
    observation_raw = read(repo / trace)
    require(old['metrics']['O1']['evidence'][0]['sha256'] == digest(observation_raw), 'prior observation differs from its input')
    return object_metrics.freeze_measurement(repo, old_raw, observation_raw, {'input': INPUT, 'trace': trace})


def measure(repo, data, corpus, executable, build=False):
    started = time.monotonic()
    if build:
        subprocess.run(['cargo', 'build', '--example', 'object_metrics'], cwd=repo, check=True, timeout=600)
    executable = Path(executable)
    executable = executable if executable.is_absolute() else repo / executable
    executable_hash = digest(read(executable, 128 * 1024 * 1024))
    truth_raw = read(repo / MANIFEST)
    truth = document(truth_raw)
    require(truth.get('version') == TRUTH_VERSION and truth.get('origin') == 'graded' and truth.get('truth_set') == 'K1', 'unsupported graded truth identity')
    papers = cohort(repo, truth)
    object_metrics.validate_registry(load_registry(data), papers, corpus / 'pdf')
    indexes, request = {}, []
    for paper in papers:
        ident = paper['paper_id']
        require(re.fullmatch('[0-9a-f]{16}', ident), 'invalid paper id')
        require(paper['index']['path'] == 'papers/' + ident + '/reading-index.json', 'wrong canonical index path')
        raw = read(data / paper['index']['path'])
        require(digest(raw) == paper['index']['sha256'], 'index generation changed')
        indexes[ident] = document(raw)['index']
        request.append({'paper_id': ident, 'relative_path': paper['arxiv_id'] + 'v' + str(paper['arxiv_version']) + '.pdf',
                        'index_sha256': paper['index']['sha256']})
    artifacts = bridge_predictions(executable, repo, corpus, data, request)
    results = [paper_result(paper, artifacts[paper['paper_id']], indexes[paper['paper_id']]) for paper in papers]
    totals, values = aggregate(truth, results)
    observation = {'schema_version': 1, 'collector': COLLECTOR, 'truth_version': TRUTH_VERSION, 'truth_sha256': digest(truth_raw),
                   'coverage': truth['coverage'], 'summary': totals,
                   'O1': 2 * totals['tp'] / (2 * totals['tp'] + totals['fp'] + totals['fn']), 'O2': statistics.median(values),
                   'papers': results, 'executable_sha256': executable_hash, 'network_calls': 0, 'model_calls': 0, 'cost_usd': 0,
                   'wall_seconds': time.monotonic() - started}
    observation_ref = {'path': OBSERVATION, 'version': COLLECTOR, 'sha256': digest(canonical(observation) + b'\n')}
    children = [{'path': TRUTH_DIR + '/' + row['path'], 'version': TRUTH_VERSION, 'sha256': row['sha256']} for row in truth['papers']]
    evidence = [observation_ref, *children]
    payload = {'schema_version': 1, 'suite': 'objects', 'collector': COLLECTOR,
               'implementation': [{'path': path, 'version': COLLECTOR, 'sha256': digest(read(repo / path))} for path in implementation_files(repo)],
               'truth_sets': {'K1': {'path': MANIFEST, 'version': TRUTH_VERSION, 'sha256': digest(truth_raw)}},
               'metrics': {'O1': {'sample': {'method': 'f1', 'true_positive': totals['tp'], 'false_positive': totals['fp'],
                                             'false_negative': totals['fn']}, 'cases': totals['truth_objects'], 'evidence': evidence},
                           'O2': {'sample': {'method': 'median', 'values': values}, 'cases': len(values), 'evidence': evidence}},
               'cost_usd': 0, 'wall_seconds': observation['wall_seconds']}
    frozen = freeze_foreign_input(repo)
    atomic_json(repo / OBSERVATION, observation)
    atomic_json(repo / INPUT, payload)
    return {'summary': totals, 'O1': observation['O1'], 'O2': observation['O2'], 'papers': len(results),
            'frozen_prior_input': frozen, 'wall_seconds': observation['wall_seconds']}


def report(repo):
    """Per-paper worklist: unmatched predictions and missed truth objects."""
    observation = document(read(repo / OBSERVATION))
    lines = []
    for paper in sorted(observation['papers'], key=lambda p: (-(p['fp'] + p['fn']), p['paper_id'])):
        lines.append('%s %sv%s tp=%d fp=%d fn=%d' % (paper['paper_id'], paper['arxiv_id'], paper['arxiv_version'], paper['tp'], paper['fp'], paper['fn']))
        predictions = {row['position']: row for row in paper['prediction_rows']}
        for position in paper['unmatched_prediction_positions']:
            row = predictions[position]
            lines.append('  unmatched prediction #%d %s %r page %s' % (position, row['kind'], row['label'], row['page']))
        truths = {row['id']: row for row in paper['truth_rows']}
        for outcome in paper['outcomes']:
            if outcome['prediction_position'] is not None:
                continue
            row = truths[outcome['truth_id']]
            lines.append('  missed truth %s %s %r page %s' % (row['id'], row['kind'], row['printed_label'], row['page']))
    summary = observation['summary']
    lines.append('total papers=%d tp=%d fp=%d fn=%d O1=%.4f O2=%.4f' % (len(observation['papers']), summary['tp'], summary['fp'], summary['fn'], observation['O1'], observation['O2']))
    return '\n'.join(lines)


# --- command line -------------------------------------------------------------

def main(argv=None):
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument('--data', type=Path, default=Path('~/.cache/lysilogy/arxiv-kb-data'), help='reader data root')
    common.add_argument('--corpus', type=Path, default=Path('~/Corpora/arxiv'), help='corpus root holding pdf/ and selection.json')
    common.add_argument('--repo', type=Path, default=ROOT, help='repository root')
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    queue_parser = commands.add_parser('queue', parents=[common], help='write <data>/grading-queue.json')
    queue_parser.add_argument('--selection', type=Path, help='corpus selection.json (default <corpus>/selection.json)')
    queue_parser.add_argument('--tier', default='eval')
    queue_parser.add_argument('--seed', type=int, default=1)
    queue_parser.add_argument('--limit', type=int)
    seed_parser = commands.add_parser('seed', parents=[common], help='import an earlier release as truth children')
    seed_parser.add_argument('--from', dest='source', default='eval/truth/k1-limited-v5', help='release directory holding objects.json')
    export_parser = commands.add_parser('export', parents=[common], help='derive truth children from complete grades files')
    export_parser.add_argument('--executable', type=Path, help='object_metrics bridge used when no current objects.json is cached')
    measure_parser = commands.add_parser('measure', parents=[common], help='measure the detector against the graded release')
    measure_parser.add_argument('--executable', type=Path, default=Path('target/debug/examples/object_metrics'))
    measure_parser.add_argument('--build', action='store_true', help='cargo build --example object_metrics first')
    commands.add_parser('report', parents=[common], help='print the detector-improvement worklist')
    args = parser.parse_args(argv)
    data, corpus, repo = args.data.expanduser(), args.corpus.expanduser(), args.repo.expanduser()
    if args.command == 'queue':
        selection = args.selection.expanduser() if args.selection else corpus / 'selection.json'
        outcome = queue(data, selection, args.tier, args.seed, args.limit)
    elif args.command == 'seed':
        outcome = seed_release(repo, args.source)
    elif args.command == 'export':
        outcome = export(repo, data, corpus, args.executable.expanduser() if args.executable else None)
    elif args.command == 'measure':
        outcome = measure(repo, data, corpus, args.executable.expanduser(), args.build)
    else:
        print(report(repo))
        return
    print(json.dumps(outcome, sort_keys=True))


if __name__ == '__main__':
    main()
