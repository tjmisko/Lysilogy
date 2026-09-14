"""Serial, source-bound publication/replay orchestration for one new K1 format."""
from pathlib import Path
import resource
import sys
import time

import release_layout as layout
import release_process as process

MEMORY_BYTES = 768 * 1024 * 1024
CPU_SECONDS = 60
WALL_SECONDS = 90
HISTORY_SECONDS = 90


def source_paths(repo, names):
    return {name: Path(repo) / ('scripts/corpus/corpus.py' if name == 'corpus.py'
            else 'scripts/truth/latex/' + name) for name in names}


def output_ref(path, cap):
    row = layout.fingerprint(path, cap)
    return dict(row, path=str(path))


def checked_result(row, expected, cap):
    layout.require(row['path'] == str(expected), 'worker returned another output path')
    raw = layout.read(expected, cap)
    layout.require(len(raw) == row['bytes'] and layout.digest(raw) == row['sha256'], 'worker output commitment differs')
    return layout.document(raw, cap)


def evidence(cache, config, release, *, paths=None, timely=lambda: None):
    declared = release.evidence_paths(config)
    selected = declared if paths is None else paths
    layout.require(selected <= declared, 'paper evidence is outside original config')
    result = {}
    for name in sorted(selected):
        timely()
        path = release.safe_file(cache, name)
        result[name] = layout.fingerprint(path, 32 * 1024 * 1024)['sha256']
        layout.require(result[name] == config['evidence_sha256'][name], 'original evidence changed')
    timely()
    return result


def worker(request, modules):
    """Runs only after versioned.py applies isolated roots, source and resource guards."""
    started = time.monotonic()
    cache, repo = Path(request['cache']), Path(request['repo'])
    output = layout.direct(request['output'])
    layout.require(output.is_relative_to(cache / 'k1-bounded-workers') and output.is_dir(), 'worker output root differs')
    release, manual = modules['release'], modules['manual']
    relative = request.get('config_path', 'eval/truth/' + layout.VERSION + '-build.json')
    layout.require(relative.startswith('eval/'), 'worker config is outside eval')
    config_raw = layout.read(release.safe_file(repo, relative))
    layout.require(layout.digest(config_raw) == request['config_sha256'], 'worker config changed')
    config = layout.document(config_raw)
    layout.require(config['version'] == layout.VERSION and 0 < len(config['papers']) <= layout.MAX_PAPERS,
                   'worker selected another version or population')
    inputs_raw = manual.bounded(cache, config['inputs'])
    inputs = manual.document(inputs_raw)
    operation = request['operation']
    if operation == 'history':
        before = evidence(cache, config, release)
        paths = source_paths(repo, ('archive.py', 'tex.py', 'parser.py', 'align.py', 'builder.py'))
        # Retained replay must validate automatic history against retained sources.
        if not request.get('publication'):
            paths = {name: Path(modules[Path(name).stem].__file__) for name in paths}
        sources = {name: layout.read(path) for name, path in paths.items()}
        history = release.validate_automatic_reports(cache, config, inputs, before, sources)
        layout.require(evidence(cache, config, release) == before, 'history evidence changed')
        value = {'history': history, 'evidence_sha256': before}
        if request.get('publication'):
            value['implementation'] = release.fingerprint_sources()
            value['implementation'].update({str(path.relative_to(repo)): layout.digest(layout.read(path))
                                            for path in source_paths(repo, request['current_sources']).values()})
        target, cap = output / 'history.json', layout.MANIFEST_BYTES
    elif operation == 'paper':
        ordinal = request['ordinal']
        layout.require(type(ordinal) is int and 0 <= ordinal < len(config['papers']), 'invalid paper ordinal')
        specification = config['papers'][ordinal]
        subset = dict(config, papers=[specification])
        selected = release.required_evidence_paths(subset)
        before = evidence(cache, config, release, paths=selected)
        candidate = manual.assemble(cache, Path(request['corpus']), Path(request['data']),
            specification['candidate'], specification.get('region_bundle'), specification.get('panel_bundle'),
            config['inputs'], config['indexes'], specification.get('object_bundle'),
            specification.get('bibliography_bundle'), specification.get('tranche_bundle'))
        frozen = {row['arxiv_id']: row for row in inputs['papers']}
        layout.require(len(frozen) == len(inputs['papers']), 'original inputs repeat identity')
        value = release.project_paper(candidate, specification, frozen, retain_ineligible=True)
        layout.require(evidence(cache, config, release, paths=selected) == before, 'paper evidence changed')
        target, cap = output / 'paper.json', layout.PAPER_BYTES
    else:
        raise ValueError('unsupported bounded worker operation')
    layout.require(layout.read(release.safe_file(repo, relative)) == config_raw, 'config changed during worker')
    layout.immutable(target, layout.canonical(value), cap)
    used = resource.getrusage(resource.RUSAGE_SELF)
    return {'schema_version': 1, 'version': layout.VERSION, 'operation': operation,
            'config_sha256': layout.digest(config_raw), 'output': output_ref(target, cap),
            'wall_seconds': time.monotonic() - started, 'cpu_seconds': used.ru_utime + used.ru_stime,
            'peak_rss_kib': used.ru_maxrss, 'network_calls': 0, 'model_calls': 0}


class Coordinator:
    def __init__(self, repo, cache, corpus, data, config_raw, source_hashes=None, config_path=None):
        self.repo, self.cache = Path(repo), Path(cache)
        self.config_raw = config_raw; self.config = layout.document(config_raw)
        layout.require(self.config['version'] == layout.VERSION
                       and 0 < len(self.config['papers']) <= layout.MAX_PAPERS, 'invalid bounded release population')
        self.started = time.monotonic()
        self.seconds = HISTORY_SECONDS + WALL_SECONDS * len(self.config['papers']) + 90
        self.deadline = self.started + self.seconds
        self.directory = self.cache / 'k1-bounded-workers' / (layout.digest(config_raw) + '-' + str(time.time_ns()))
        layout.direct(self.directory).mkdir(parents=True)
        self.dispatch = self.repo / 'scripts/truth/latex/versioned.py'
        self.dispatch_hash = layout.digest(layout.read(self.dispatch))
        self.request = {'repo': str(repo), 'cache': str(cache), 'corpus': str(corpus), 'data': str(data),
                        'version': layout.VERSION, 'config_sha256': layout.digest(config_raw)}
        if source_hashes is not None:
            self.request.update(publication=True, current_sources=source_hashes, config_path=config_path)
        self.receipts = []; self.cpu = 0; self.peak = 0

    def timely(self):
        layout.require(time.monotonic() < self.deadline, 'bounded replay exceeded aggregate wall deadline')
        layout.require(layout.digest(layout.read(self.dispatch)) == self.dispatch_hash, 'worker dispatcher changed')

    def invoke(self, operation, ordinal=None):
        self.timely()
        name = operation if ordinal is None else operation + '-' + str(ordinal)
        directory = self.directory / name
        request = dict(self.request, operation=operation, output=str(directory))
        if ordinal is not None:
            request['ordinal'] = ordinal
        request_raw = layout.canonical(request, 8192)
        raw, transport = process.run([sys.executable, '-I', '-B', str(self.dispatch), '--worker'],
            request_raw, directory, cwd=self.repo,
            seconds=min(WALL_SECONDS, self.deadline - time.monotonic()))
        result = layout.document(raw)
        layout.require(result['version'] == layout.VERSION and result['operation'] == operation
                       and result['config_sha256'] == self.request['config_sha256']
                       and result['network_calls'] == result['model_calls'] == 0, 'worker receipt identity differs')
        target = directory / ('history.json' if operation == 'history' else 'paper.json')
        value = checked_result(result['output'], target,
                               layout.MANIFEST_BYTES if operation == 'history' else layout.PAPER_BYTES)
        layout.require(type(result['cpu_seconds']) in (int, float) and 0 <= result['cpu_seconds'] <= CPU_SECONDS
                       and type(result['peak_rss_kib']) is int and 0 <= result['peak_rss_kib'] <= MEMORY_BYTES // 1024,
                       'worker resource receipt differs')
        self.cpu += result['cpu_seconds']; self.peak = max(self.peak, result['peak_rss_kib'])
        layout.require(self.cpu <= CPU_SECONDS * (len(self.config['papers']) + 1), 'aggregate worker CPU exceeds limit')
        detail = directory / 'worker.json'
        layout.immutable(detail, layout.canonical({'transport': transport, 'result': result}), layout.MANIFEST_BYTES)
        self.receipts.append({'operation': operation, 'ordinal': ordinal,
                              'receipt': output_ref(detail, layout.MANIFEST_BYTES)})
        self.timely()
        return value

    def receipt(self, outputs):
        self.timely()
        receipt = {'schema_version': 1, 'version': layout.VERSION, 'layout': layout.LAYOUT,
                   'outputs': outputs, 'reproduced': True, 'workers': self.receipts,
                   'wall_seconds': time.monotonic() - self.started, 'wall_limit': self.seconds,
                   'cpu_seconds': self.cpu, 'peak_worker_rss_kib': self.peak,
                   'memory_limit': MEMORY_BYTES, 'per_worker_cpu_limit': CPU_SECONDS,
                   'per_worker_wall_limit': WALL_SECONDS, 'network_calls': 0, 'model_calls': 0, 'cost_usd': 0}
        path = self.directory / 'complete.json'
        layout.immutable(path, layout.canonical(receipt), layout.MANIFEST_BYTES)
        self.timely()
        # Only compact commitments cross the caller response/evidence boundary.
        return {'schema_version': 1, 'version': layout.VERSION, 'outputs': outputs, 'reproduced': True,
                'receipt': output_ref(path, layout.MANIFEST_BYTES), 'wall_seconds': receipt['wall_seconds'],
                'cpu_seconds': self.cpu, 'peak_worker_rss_kib': self.peak,
                'network_calls': 0, 'model_calls': 0, 'cost_usd': 0}


def replay(repo, cache, corpus, data, manifest, truth_raw, config_raw):
    coordinator = Coordinator(repo, cache, corpus, data, config_raw)
    config = coordinator.config
    history = coordinator.invoke('history')
    root = Path(repo) / 'eval/truth' / layout.VERSION
    original = layout.document(truth_raw); rows = layout.check_manifest(original)
    layout.require(history['evidence_sha256'] == config['evidence_sha256']
                   == original['provenance']['evidence_sha256'], 'original evidence provenance differs')
    layout.require([r['paper_id'] for r in rows] == [r['paper_id'] for r in config['papers']], 'replay order differs')
    summary = layout.Summary()
    for ordinal, row in enumerate(rows):
        rebuilt = coordinator.invoke('paper', ordinal)
        raw = layout.canonical(rebuilt)
        layout.require(layout.descriptor(rebuilt, ordinal, raw) == row
                       and layout.read(root / row['path'], layout.PAPER_BYTES) == raw,
                       'retained worker cannot reproduce complete paper bytes')
        summary.add(rebuilt)
    inputs = layout.document(layout.read(Path(cache) / config['inputs'], 32 * 1024 * 1024), 32 * 1024 * 1024)
    expected = layout.manifests(rows, summary, config, inputs, history['history'], original['provenance'])
    layout.require({name: layout.digest(raw) for name, raw in expected.items()} == manifest['outputs'],
                   'reproduced manifest outputs differ')
    layout.verify(root, config, inputs, history['history'], original['provenance'])
    for name, expected_hash in history['evidence_sha256'].items():
        coordinator.timely()
        layout.require(layout.fingerprint(Path(cache) / name, 32 * 1024 * 1024)['sha256'] == expected_hash,
                       'original evidence changed after complete replay')
    coordinator.timely()
    return original, config, coordinator.receipt(manifest['outputs'])


def publish_current(repo, cache, corpus, data, config_path, versioned):
    """Explicit producer command; the new replay pin remains disabled after construction."""
    with process.address_limit():
        return _publish_current(repo, cache, corpus, data, config_path, versioned)


def _publish_current(repo, cache, corpus, data, config_path, versioned):
    config_path = layout.direct(config_path); repo = Path(repo)
    layout.require(config_path.is_relative_to(repo / 'eval'), 'publication config is outside eval')
    config_raw = layout.read(config_path); config = layout.document(config_raw)
    sources = {name: layout.digest(layout.read(path))
               for name, path in source_paths(repo, versioned.BOUNDED_MODULES).items()}
    coordinator = Coordinator(repo, cache, corpus, data, config_raw, sources, str(config_path.relative_to(repo)))
    history = coordinator.invoke('history')
    inputs = layout.document(layout.read(Path(cache) / config['inputs'], 32 * 1024 * 1024), 32 * 1024 * 1024)
    implementation = history['implementation']
    layout.require(history['evidence_sha256'] == config['evidence_sha256'], 'publication evidence closure differs')
    layout.require(all(implementation[str(path.relative_to(repo))] == sources[name]
                       for name, path in source_paths(repo, sources).items()), 'publication implementation differs')
    provenance = {'build_config_sha256': layout.digest(config_raw), 'implementation': implementation,
                  'evidence_sha256': history['evidence_sha256']}

    def boundary():
        coordinator.timely()
        layout.require(layout.read(config_path) == config_raw, 'publication config changed')
        layout.require(all(layout.digest(layout.read(repo / path)) == expected
                       for path, expected in implementation.items()), 'publication source changed')
        for name, expected in history['evidence_sha256'].items():
            coordinator.timely()
            layout.require(layout.fingerprint(Path(cache) / name, 32 * 1024 * 1024)['sha256'] == expected,
                           'publication evidence changed')

    records = (coordinator.invoke('paper', ordinal) for ordinal in range(len(config['papers'])))
    output = repo / 'eval/truth' / layout.VERSION
    stage = output.parent / ('.' + layout.VERSION + '-staging-' + layout.digest(config_raw)[:16])
    root = layout.publish(output, stage, records, config, inputs, history['history'], provenance,
                          boundary, coordinator.directory / 'publication.jsonl')
    outputs = {name: layout.digest(layout.read(output / name)) for name in ('objects.json', 'bibliography.json')}
    return root, coordinator.receipt(outputs)
