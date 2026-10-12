"""Complete per-paper K1 transport; no real release is activated by this module."""
from collections import Counter
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil

LAYOUT = 'k1-per-paper-v1'
VERSION = 'k1-limited-v5'
MANIFEST_BYTES = 1024 * 1024
PAPER_BYTES = 8 * 1024 * 1024
TOTAL_BYTES = 1024 * 1024 * 1024
MAX_PAPERS = 1000
MAX_DEPTH = 64
MAX_NODES = 200000
MAX_METRIC_VALUES = 100000
FREE_BYTES = 20 * 1024 * 1024 * 1024
METRICS = tuple('O' + str(n) for n in range(1, 12))
KINDS = {'figure', 'table', 'equation', 'statement', 'proof', 'algorithm', 'bib_entry'}
LIMITS = {'manifest_bytes': MANIFEST_BYTES, 'paper_bytes': PAPER_BYTES,
          'total_bytes': TOTAL_BYTES, 'max_papers': MAX_PAPERS,
          'max_depth': MAX_DEPTH, 'max_nodes': MAX_NODES, 'max_metric_values': MAX_METRIC_VALUES}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def canonical(value, cap=PAPER_BYTES):
    # Incremental encoding rejects at the byte boundary before joining the result.
    chunks = []; size = 1
    encoder = json.JSONEncoder(sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False)
    for chunk in encoder.iterencode(value):
        raw = chunk.encode(); size += len(raw)
        require(size <= cap, 'layout output exceeds limit')
        chunks.append(raw)
    return b''.join(chunks) + b'\n'


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def direct(path):
    path = Path(path).absolute()
    require('..' not in path.parts and not any(p.is_symlink() for p in (path, *path.parents)),
            'redirected layout path')
    require(not any(p == '.secrets' or p == '.env' or p.startswith('.env.') for p in path.parts),
            'forbidden layout path')
    return path


def read(path, cap=MANIFEST_BYTES):
    path = direct(path)
    require(path.is_file() and path.stat().st_size <= cap, 'missing or oversized layout file')
    with path.open('rb') as stream:
        raw = stream.read(cap + 1)
    require(len(raw) <= cap, 'layout file grew beyond bound')
    return raw


def fingerprint(path, cap):
    path = direct(path)
    require(path.is_file() and path.stat().st_size <= cap, 'missing or oversized layout input')
    result = hashlib.sha256(); count = 0
    with path.open('rb') as stream:
        while part := stream.read(min(1024 * 1024, cap + 1 - count)):
            count += len(part)
            require(count <= cap, 'layout input grew beyond bound')
            result.update(part)
    return {'sha256': result.hexdigest(), 'bytes': count}


def document(raw, cap=MANIFEST_BYTES):
    require(isinstance(raw, bytes) and len(raw) <= cap, 'layout JSON exceeds byte limit')
    # Bound nesting and structural tokens before allocating the decoded graph.
    depth = nodes = 0; quoted = escaped = False
    for char in raw:
        if quoted:
            if escaped:
                escaped = False
            elif char == 92:
                escaped = True
            elif char == 34:
                quoted = False
        elif char == 34:
            quoted = True; nodes += 1
        elif char in (123, 91):
            depth += 1; nodes += 1
            require(depth <= MAX_DEPTH, 'layout JSON exceeds depth limit')
        elif char in (125, 93):
            depth -= 1
        elif char in (44, 58):
            nodes += 1
        require(nodes <= MAX_NODES, 'layout JSON exceeds node limit')

    def pairs(values):
        row = {}
        for key, value in values:
            require(key not in row, 'duplicate layout JSON field')
            row[key] = value
        return row

    value = json.loads(raw, object_pairs_hook=pairs,
                       parse_constant=lambda _: require(False, 'nonfinite layout JSON'))
    require(type(value) is dict, 'layout document is not an object')
    pending = [value]
    while pending:
        item = pending.pop()
        if isinstance(item, dict):
            pending.extend(item.values())
        elif isinstance(item, list):
            pending.extend(item)
        elif isinstance(item, float):
            require(math.isfinite(item), 'nonfinite layout JSON number')
    return value


def unique_ids(rows, label):
    require(type(rows) is list and len(rows) <= MAX_NODES, 'invalid ' + label + ' inventory')
    ids = [row['id'] for row in rows]
    require(all(type(ident) is str and ident for ident in ids)
            and len(ids) == len(set(ids)), 'duplicate or empty ' + label + ' identity')


class Summary:
    """Small exact aggregate; full paper records are never accumulated here."""
    def __init__(self):
        self.ids = []; self.arxiv = set(); self.totals = Counter(); self.strata = Counter()
        self.cohorts = {metric: [] for metric in METRICS}
        self.negative = 0; self.metric_cases = Counter(); self.eligible_kinds = Counter()
        self.ineligible = []

    def add(self, paper):
        ident = paper['paper_id']
        require(type(ident) is str and re.fullmatch('[0-9a-f]{16}', ident), 'invalid paper identity')
        require(type(paper['arxiv_id']) is str and paper['arxiv_id']
                and type(paper['arxiv_version']) is int and paper['arxiv_version'] > 0,
                'invalid pinned arXiv identity')
        require(len(self.ids) < MAX_PAPERS and ident not in self.ids
                and paper['arxiv_id'] not in self.arxiv, 'duplicate or excessive paper population')
        eligibility = paper['metric_eligibility']
        require(set(eligibility) == set(METRICS) and all(type(v) is bool for v in eligibility.values()),
                'incomplete metric eligibility')
        unique_ids(paper['objects'], 'object'); unique_ids(paper['entries'], 'entry')
        require(not ({r['id'] for r in paper['objects']} & {r['id'] for r in paper['entries']}),
                'object and entry identities collide')
        kinds = {row['id']: row['kind'] for row in paper['objects']}
        require(all(kind in KINDS - {'bib_entry'} for kind in kinds.values()), 'unknown object kind')
        counts = Counter(kinds.values()); counts['bib_entry'] = len(paper['entries'])
        require(all(type(v) is int and v >= 0 for v in paper['counts'].values())
                and paper['counts'] == dict(counts), 'paper kind counts differ from complete inventory')
        absent = paper['reviewed_absent_kinds']
        require(type(absent) is list and len(absent) == len(set(absent))
                and set(absent) <= KINDS and not any(counts[kind] for kind in absent),
                'negative kind inventory conflicts with positive objects')
        object_refs = 0
        for reference in paper['references']:
            require(reference['target'] in kinds, 'reference target is outside paper inventory')
            object_refs += kinds[reference['target']] in ('equation', 'statement')
        self.ids.append(ident); self.arxiv.add(paper['arxiv_id'])
        self.totals.update(counts); self.strata[str(paper['stratum'])] += 1
        for metric, eligible in eligibility.items():
            if eligible:
                self.cohorts[metric].append(ident)
        if not any(eligibility.values()):
            self.ineligible.append(ident)
        self.negative += eligibility['O1'] and set(absent) == {'figure', 'table'}
        cases = {'O1': counts['figure'] + counts['table'], 'O2': counts['figure'] + counts['table'],
                 'O3': counts['equation'], 'O4': object_refs, 'O5': counts['statement'],
                 'O6': counts['proof'], 'O7': counts['algorithm'], 'O8': len(paper['entries']),
                 'O9': sum(len(row['field_labels']) for row in paper['entries']),
                 'O10': len(paper['mentions']), 'O11': 9}
        for metric, count in cases.items():
            if eligibility[metric]:
                self.metric_cases[metric] += count
        for kind, metric in [('figure', 'O1'), ('table', 'O1'), ('equation', 'O3'),
                             ('statement', 'O5'), ('proof', 'O6'), ('algorithm', 'O7'), ('bib_entry', 'O8')]:
            if eligibility[metric]:
                self.eligible_kinds[kind] += counts[kind]
        require(paper['bibliography_eligible'] is (eligibility['O8'] and eligibility['O10']),
                'bibliography eligibility differs')

    def coverage(self, config, inputs, history):
        require(config['target_papers'] == 500
                and config['coverage_followup'] == 'https://github.com/tjmisko/Lysilogy/issues/97',
                'coverage target or follow-up differs')
        require(config['selection_bias'].strip() and config['publication_deviation'].strip(),
                'release omits selection limitations')
        require(all(self.eligible_kinds[kind] > 0 for kind in KINDS) and all(self.cohorts.values()),
                'release omits a required positive kind or metric cohort')
        count = len(inputs['papers'])
        require(0 < count <= MAX_PAPERS and len({p['arxiv_id'] for p in inputs['papers']}) == count,
                'original eval population is invalid')
        require(self.arxiv <= {p['arxiv_id'] for p in inputs['papers']}, 'paper is outside original eval')
        denominators = {
            'O1': {'truth_objects': self.metric_cases['O1'],
                   'papers': len(self.cohorts['O1']), 'negative_papers': self.negative},
            'O2': {'all_annotated_truth_objects': self.metric_cases['O2']},
            'O3': {'equations': self.metric_cases['O3']}, 'O4': {'object_reference_pairs': self.metric_cases['O4']},
            'O5': {'statements': self.metric_cases['O5']}, 'O6': {'proofs': self.metric_cases['O6']},
            'O7': {'algorithms': self.metric_cases['O7']}, 'O8': {'entries': self.metric_cases['O8']},
            'O9': {'known_fields': self.metric_cases['O9']}, 'O10': {'citation_target_pairs': self.metric_cases['O10']},
            'O11': {'papers': len(self.cohorts['O11']),
                    'panelist_top3_opportunities': 9 * len(self.cohorts['O11'])}}
        admitted = len(self.ids) - len(self.ineligible)
        return {'release_papers': admitted, 'retained_papers': len(self.ids),
                'ineligible_papers': self.ineligible, 'target_papers': 500, 'target_met': admitted >= 500,
                'frozen_eval_papers': count, 'cohort_papers': self.cohorts, 'denominators': denominators,
                'excluded_eval_papers_by_metric': {m: count - len(ids) for m, ids in self.cohorts.items()},
                'frozen_eval_strata': dict(Counter(str(p['stratum']) for p in inputs['papers'])),
                'positive_object_counts': dict(self.eligible_kinds),
                'retained_object_counts': dict(self.totals), 'strata': dict(self.strata),
                'selection_bias': config['selection_bias'], 'publication_deviation': config['publication_deviation'],
                'followup': config['coverage_followup'], 'automatic_builds': history,
                'system_acceptance_claimed': False}


def descriptor(paper, ordinal, raw):
    return {'ordinal': ordinal, 'paper_id': paper['paper_id'], 'arxiv_id': paper['arxiv_id'],
            'arxiv_version': paper['arxiv_version'], 'path': 'papers/' + paper['paper_id'] + '.json',
            'sha256': digest(raw), 'bytes': len(raw), 'metric_eligibility': paper['metric_eligibility'],
            'counts': paper['counts']}


def manifests(rows, summary, config, inputs, history, provenance):
    require(config['version'] == VERSION, 'unsupported per-paper release version')
    coverage = summary.coverage(config, inputs, history)
    root = {'schema_version': 2, 'layout': LAYOUT, 'limits': LIMITS, 'truth_set': 'K1',
            'version': VERSION, 'origin': 'arxiv-latex', 'alignment_threshold': .95,
            'scope': 'limited independently reviewed release; coverage expansion remains open',
            'build_date': config['build_date'], 'coverage': coverage, 'provenance': provenance,
            'papers': rows, 'paper_bytes': sum(row['bytes'] for row in rows)}
    bibliography = dict(root, version=VERSION + '-bibliography',
                        papers=[row for row in rows if row['metric_eligibility']['O8']
                                and row['metric_eligibility']['O10']])
    return {'objects.json': canonical(root, MANIFEST_BYTES), 'bibliography.json': canonical(bibliography, MANIFEST_BYTES)}


def check_manifest(root):
    require(root.get('schema_version') == 2 and root.get('layout') == LAYOUT
            and root.get('version') == VERSION and root.get('limits') == LIMITS
            and root.get('truth_set') == 'K1', 'unsupported per-paper manifest')
    rows = root['papers']
    require(type(rows) is list and 0 < len(rows) <= MAX_PAPERS, 'invalid manifest population')
    require(all(type(row['ordinal']) is int for row in rows)
            and [row['ordinal'] for row in rows] == list(range(len(rows))), 'paper ordinals differ')
    require(len({row['paper_id'] for row in rows}) == len({row['arxiv_id'] for row in rows}) == len(rows),
            'manifest repeats paper identities')
    for row in rows:
        require(type(row['paper_id']) is str and re.fullmatch('[0-9a-f]{16}', row['paper_id'])
                and row['path'] == 'papers/' + row['paper_id'] + '.json', 'unsafe paper descriptor')
        require(type(row['bytes']) is int and 0 < row['bytes'] <= PAPER_BYTES
                and type(row['sha256']) is str and re.fullmatch('[0-9a-f]{64}', row['sha256']),
                'invalid paper byte commitment')
    require(type(root['paper_bytes']) is int
            and root['paper_bytes'] == sum(row['bytes'] for row in rows) <= TOTAL_BYTES,
            'cumulative truth bytes differ')
    return rows


def decision_descriptor(paper_id, ordinal, raw):
    require(type(paper_id) is str and re.fullmatch('[0-9a-f]{16}', paper_id), 'invalid decision paper')
    require(type(ordinal) is int and 0 <= ordinal < MAX_PAPERS and len(raw) <= PAPER_BYTES,
            'decision exceeds identity or byte bound')
    return {'ordinal': ordinal, 'paper_id': paper_id, 'path': 'papers/' + paper_id + '.json',
            'sha256': digest(raw), 'bytes': len(raw)}


def paper(root, row):
    raw = read(direct(root) / row['path'], PAPER_BYTES)
    require(len(raw) == row['bytes'] and digest(raw) == row['sha256'], 'paper bytes differ')
    value = document(raw, PAPER_BYTES)
    require(raw == canonical(value) and descriptor(value, row['ordinal'], raw) == row,
            'paper identity or canonical encoding differs')
    return value


def verify(root, config, inputs, history, provenance):
    root = direct(root)
    original = read(root / 'objects.json')
    manifest = document(original); rows = check_manifest(manifest)
    require([row['paper_id'] for row in rows] == [row['paper_id'] for row in config['papers']],
            'manifest differs from original selected order')
    actual = set()
    for folder in (root, root / 'papers'):
        with os.scandir(folder) as entries:
            for entry in entries:
                require(not entry.is_symlink(), 'symlinked release child')
                if entry.is_dir(follow_symlinks=False):
                    require(folder == root and entry.name == 'papers', 'extra release directory')
                    continue
                require(entry.is_file(follow_symlinks=False), 'invalid release child')
                actual.add(str(Path(entry.path).relative_to(root)))
                require(len(actual) <= MAX_PAPERS + 2, 'excessive release children')
    require(actual == {'objects.json', 'bibliography.json'} | {row['path'] for row in rows},
            'missing or extra release child')
    summary = Summary()
    frozen = {row['arxiv_id']: row for row in inputs['papers']}
    for row, specification in zip(rows, config['papers'], strict=True):
        value = paper(root, row)
        require(value['arxiv_id'] == specification['arxiv_id'], 'selected arXiv identity differs')
        source = frozen[value['arxiv_id']]
        require(value['arxiv_version'] == source['version'] and value['stratum'] == source['stratum']
                and value['pdf_sha256'] == source['pdf']['sha256']
                and value['source_sha256'] == source['source']['sha256'], 'original source identity differs')
        summary.add(value)
    expected = manifests(rows, summary, config, inputs, history, provenance)
    require(all(read(root / name) == raw for name, raw in expected.items()), 'manifest totals or provenance differ')
    require(read(root / 'objects.json') == original, 'manifest changed during verification')
    return manifest


def immutable(path, raw, cap):
    path = direct(path)
    require(len(raw) <= cap, 'layout output exceeds limit')
    require(shutil.disk_usage(path.parent).free >= FREE_BYTES + len(raw), 'layout free-space floor')
    if path.exists():
        require(read(path, cap) == raw, 'immutable staged bytes differ')
    else:
        with path.open('xb') as stream:
            stream.write(raw); stream.flush(); os.fsync(stream.fileno())


def publish(output, staging, records, config, inputs, history, provenance, boundary, ledger):
    """Stage full children serially. Caller supplies reviewed assembly, never acceptance flags."""
    output, staging, ledger = direct(output), direct(staging), direct(ledger)
    require(output != staging and output.parent == staging.parent, 'staging must be a distinct sibling')
    require(not output.exists(), 'release already exists; verify immutable contents instead')
    staging.mkdir(parents=True, exist_ok=True); (staging / 'papers').mkdir(exist_ok=True)
    ledger.parent.mkdir(parents=True, exist_ok=True)
    summary = Summary(); rows = []; total = 0
    with ledger.open('x') as log:
        def record(value):
            log.write(canonical(value).decode()); log.flush(); os.fsync(log.fileno())
        try:
            boundary()
            specifications = config['papers']
            require(0 < len(specifications) <= MAX_PAPERS, 'invalid selected population')
            for ordinal, value in enumerate(records):
                require(ordinal < len(specifications), 'extra projected paper')
                require(value['paper_id'] == specifications[ordinal]['paper_id']
                        and value['arxiv_id'] == specifications[ordinal]['arxiv_id'], 'projected order differs')
                raw = canonical(value)
                require(len(raw) <= PAPER_BYTES, 'paper output exceeds limit')
                document(raw, PAPER_BYTES); summary.add(value)
                total += len(raw); require(total <= TOTAL_BYTES, 'cumulative paper output exceeds limit')
                row = descriptor(value, ordinal, raw)
                immutable(staging / row['path'], raw, PAPER_BYTES)
                rows.append(row); record({'status': 'paper_staged', 'paper': row})
            require(len(rows) == len(specifications), 'missing projected paper')
            boundary()
            roots = manifests(rows, summary, config, inputs, history, provenance)
            for name, raw in roots.items():
                immutable(staging / name, raw, MANIFEST_BYTES)
            verify(staging, config, inputs, history, provenance)
            boundary()
            os.rename(staging, output)
            descriptor_fd = os.open(output.parent, os.O_RDONLY)
            try:
                os.fsync(descriptor_fd)
            finally:
                os.close(descriptor_fd)
            record({'status': 'complete', 'outputs': {name: digest(raw) for name, raw in roots.items()}})
            return document(roots['objects.json'])
        except BaseException as error:
            record({'status': 'failed', 'error': str(error), 'staged_papers': len(rows)})
            raise
