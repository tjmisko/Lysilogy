#!/usr/bin/env python3
"""Replay a fixed K1 release with its exact reviewed implementation.

The fixed manifest selects application-owned code, never deposited TeX or a
caller-supplied module. The worker has a separate module namespace and cannot
publish or revise the historical release.
"""
import argparse
import hashlib
import importlib
import importlib.abc
import importlib.util
import json
from pathlib import Path
import resource
import subprocess
import sys
import time

VERSION = 'k1-limited-v1'
MANIFEST_SHA256 = '770ba066d41931f3fae8682ead5216214de780c2d3349ff12da1f35b685dd118'
MODULES = {'align.py', 'annotations.py', 'archive.py', 'bibliography_annotations.py',
           'builder.py', 'corpus.py', 'manual.py', 'object_annotations.py', 'panel.py',
           'parser.py', 'release.py', 'tex.py'}
CURRENT_VERSION = 'k1-limited-v2'
CURRENT_MODULES = MODULES | {'native_exports.py', 'tranche.py'}
# Filled only after the separately reviewed immutable release is constructed.
# An unpublished version cannot be selected successfully.
CURRENT_MANIFEST_SHA256 = 'fdd6d2c5e9ec5f03f6461b15fe01c92ab93151a68ed695a661183616379446ae'
VISUAL_VERSION = 'k1-limited-v3'
VISUAL_MODULES = CURRENT_MODULES | {'tranche_visual.py'}
VISUAL_MANIFEST_SHA256 = 'd02b71b0f9604bdd3e2535825d0d257b587ce1219c5552eb978c6cd9886a682b'
VISUAL_ONLY_VERSION = 'k1-limited-v4'
VISUAL_ONLY_MODULES = VISUAL_MODULES | {'tranche_visual_only.py'}
VISUAL_ONLY_MANIFEST_SHA256 = None  # Remains disabled until separately reviewed immutable publication.
MAX_DOCUMENT = 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def read(path, cap=MAX_DOCUMENT):
    path = Path(path).absolute()
    require('..' not in path.parts, 'parent traversal is not a verifier path')
    require(not any(p.is_symlink() for p in (path, *path.parents)), 'symlinked verifier input')
    require(path.is_file() and path.stat().st_size <= cap, 'missing or oversized verifier input')
    with path.open('rb') as stream:
        raw = stream.read(cap + 1)
    require(len(raw) <= cap, 'verifier input grew beyond its bound')
    return raw


def document(raw):
    def unique(pairs):
        value = {}
        for key, item in pairs:
            require(key not in value, 'duplicate verifier JSON key')
            value[key] = item
        return value
    return json.loads(raw, object_pairs_hook=unique,
                      parse_constant=lambda _: require(False, 'nonfinite verifier JSON'))


def release_spec(version):
    require(version in (VERSION, CURRENT_VERSION, VISUAL_VERSION, VISUAL_ONLY_VERSION), 'unsupported retained truth version')
    if version == VERSION:
        return MANIFEST_SHA256, MODULES
    if version == VISUAL_VERSION:
        require(VISUAL_MANIFEST_SHA256 is not None, 'truth version is not published')
        return VISUAL_MANIFEST_SHA256, VISUAL_MODULES
    if version == VISUAL_ONLY_VERSION:
        require(VISUAL_ONLY_MANIFEST_SHA256 is not None, 'truth version is not published')
        return VISUAL_ONLY_MANIFEST_SHA256, VISUAL_ONLY_MODULES
    require(CURRENT_MANIFEST_SHA256 is not None, 'truth version is not published')
    return CURRENT_MANIFEST_SHA256, CURRENT_MODULES


def verified_bundle(repo, version):
    manifest_hash, modules = release_spec(version)
    bundle = Path(repo) / 'eval/implementations' / version
    raw = read(bundle / 'manifest.json')
    require(digest(raw) == manifest_hash, 'retained verifier manifest differs')
    manifest = document(raw)
    require(type(manifest['schema_version']) is int and manifest['schema_version'] == 1
            and manifest['release'] == version and set(manifest['files']) == modules,
            'retained verifier inventory differs')
    for name, row in manifest['files'].items():
        expected = 'scripts/corpus/corpus.py' if name == 'corpus.py' else 'scripts/truth/latex/' + name
        require(row['source_path'] == expected, 'verifier source path differs')
        raw = read(bundle / name)
        require(digest(raw) == row['sha256'] and type(row['bytes']) is int and len(raw) == row['bytes'],
                'retained verifier module differs: ' + name)
    return bundle, manifest


def pinned_release(repo, version):
    bundle, manifest = verified_bundle(repo, version)
    raw = read(Path(repo) / 'eval/truth' / version / 'objects.json')
    bibliography_raw = read(Path(repo) / 'eval/truth' / version / 'bibliography.json')
    config_raw = read(Path(repo) / 'eval/truth' / (version + '-build.json'))
    require(digest(raw) == manifest['outputs']['objects.json']
            and digest(bibliography_raw) == manifest['outputs']['bibliography.json']
            and digest(config_raw) == manifest['config_sha256'], 'immutable release bytes differ')
    truth, config = document(raw), document(config_raw)
    bibliography = document(bibliography_raw)
    require(truth['version'] == config['version'] == version
            and bibliography['version'] == version + '-bibliography',
            'release selects another verifier')
    require(bibliography['provenance'] == truth['provenance'], 'release provenance differs by payload')
    require(truth['provenance']['build_config_sha256'] == manifest['config_sha256'],
            'release configuration provenance differs')
    for row in manifest['files'].values():
        require(truth['provenance']['implementation'][row['source_path']] == row['sha256'],
                'verifier is not the original release implementation')
    return bundle, manifest, raw, bibliography_raw, config_raw


def load_modules(repo, version):
    """Compile verified bytes directly, without searching a bundle or bytecode cache."""
    require(sys.flags.isolated and sys.flags.dont_write_bytecode, 'verifier requires isolated Python')
    bundle, manifest = verified_bundle(repo, version)
    namespace = VISUAL_ONLY_MODULES if version == VISUAL_ONLY_VERSION else CURRENT_MODULES
    require(not any(Path(name).stem in sys.modules for name in namespace),
            'verifier module namespace is contaminated')
    sources = {}
    for name, row in manifest['files'].items():
        raw = read(bundle / name)
        require(digest(raw) == row['sha256'], 'verifier module changed before import')
        sources[Path(name).stem] = raw

    class PinnedLoader(importlib.abc.MetaPathFinder, importlib.abc.Loader):
        def find_spec(self, fullname, path=None, target=None):
            if fullname in sources:
                return importlib.util.spec_from_file_location(fullname, bundle / (fullname + '.py'), loader=self)
            return None

        def create_module(self, spec):
            return None

        def exec_module(self, module):
            exec(compile(sources[module.__name__], module.__file__, 'exec'), module.__dict__)

    loader = PinnedLoader()
    original_path = sys.path[:]
    sys.meta_path.insert(0, loader)
    try:
        # builder's exact old ROOT is correct at eval/implementations/<version>/.
        # The finder takes precedence even while old builder adjusts sys.path.
        modules = {name: importlib.import_module(name) for name in sources}
        require(modules['builder'].ROOT == Path(repo), 'relocated verifier lost its repository root')
        for name, module in modules.items():
            require(Path(module.__file__).absolute() == bundle / (name + '.py'),
                    'verifier imported another module')
    finally:
        sys.path[:] = original_path
        sys.meta_path.remove(loader)
    verified_bundle(repo, version)
    return modules


def replay(repo, cache, corpus, data, version=VERSION):
    """Validate in a fresh interpreter; return unchanged historical labels/config."""
    repo = Path(repo).absolute()
    _, manifest, truth_raw, _, config_raw = pinned_release(repo, version)
    manifest_hash, _ = release_spec(version)
    worker = repo / 'scripts/truth/latex/versioned.py'
    worker_hash = digest(read(worker))
    request = {'repo': str(repo), 'cache': str(cache), 'corpus': str(corpus),
               'data': str(data), 'version': version}
    result = subprocess.run([sys.executable, '-I', '-B', str(worker), '--worker'],
                            input=json.dumps(request).encode(), capture_output=True,
                            check=True, timeout=90, cwd=repo)
    require(len(result.stdout) <= MAX_DOCUMENT, 'retained verifier response exceeds its bound')
    receipt = document(result.stdout)
    require(receipt['version'] == version and receipt['manifest_sha256'] == manifest_hash
            and receipt['outputs'] == manifest['outputs'] and receipt['reproduced'] is True,
            'retained verifier returned another release')
    require(digest(read(worker)) == worker_hash, 'verifier worker changed during replay')
    _, _, final_truth, _, final_config = pinned_release(repo, version)
    require(final_truth == truth_raw and final_config == config_raw, 'release changed during replay')
    return document(truth_raw), document(config_raw), receipt


def worker(request):
    """Called only in the isolated interpreter, with fixed application roots."""
    require(sys.flags.isolated and sys.flags.dont_write_bytecode, 'verifier requires isolated Python')
    repo, cache, corpus, data = (Path(request[key]).absolute() for key in ('repo', 'cache', 'corpus', 'data'))
    require(repo == Path(__file__).resolve().parents[3], 'worker repository differs')
    require(cache == Path.home() / '.cache/lysilogy'
            and corpus == Path.home() / 'Corpora/arxiv'
            and data == cache / 'arxiv-kb-data', 'verifier roots differ from dedicated storage')
    for path in (repo, cache, corpus, data):
        require(path.is_dir() and not any(p.is_symlink() for p in (path, *path.parents)),
                'verifier root is not a direct directory')
    resource.setrlimit(resource.RLIMIT_AS, (768 * 1024 * 1024, 768 * 1024 * 1024))
    resource.setrlimit(resource.RLIMIT_CPU, (60, 60))
    started = time.monotonic()
    bundle, manifest, truth_raw, bibliography_raw, config_raw = pinned_release(repo, request['version'])
    modules = load_modules(repo, request['version'])
    release, manual = modules['release'], modules['manual']
    truth, config = document(truth_raw), document(config_raw)
    before = release.verify_evidence(cache, config)
    require(before == config['evidence_sha256'] == truth['provenance']['evidence_sha256'],
            'original evidence provenance differs')
    inputs = document(manual.bounded(cache, config['inputs']))
    assemblies = []
    for p in config['papers']:
        args = (cache, corpus, data, p['candidate'], p.get('region_bundle'), p.get('panel_bundle'),
                config['inputs'], config['indexes'], p.get('object_bundle'), p.get('bibliography_bundle'))
        assemblies.append(manual.assemble(*args) if request['version'] == VERSION
                          else manual.assemble(*args, p.get('tranche_bundle')))
    sources = {name: read(bundle / name)
               for name in ('archive.py', 'tex.py', 'parser.py', 'align.py', 'builder.py')}
    history = release.validate_automatic_reports(cache, config, inputs, before, sources)
    objects, bibliography = release.build_release(assemblies, config, inputs, history)
    objects['provenance'] = truth['provenance']
    bibliography['provenance'] = document(bibliography_raw)['provenance']
    outputs = {'objects.json': digest(release.canonical(objects) + b'\n'),
               'bibliography.json': digest(release.canonical(bibliography) + b'\n')}
    require(outputs == manifest['outputs'], 'retained implementation cannot reproduce original labels')
    require(release.verify_evidence(cache, config) == before, 'review evidence changed during replay')
    pinned_release(repo, request['version'])
    return {'schema_version': 1, 'version': request['version'],
            'manifest_sha256': release_spec(request['version'])[0],
            'outputs': outputs, 'reproduced': True, 'module_hashes': manifest['files'],
            'evidence_hashes': before, 'wall_seconds': time.monotonic() - started,
            'network_calls': 0, 'model_calls': 0, 'cost_usd': 0}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', action='store_true')
    arguments = parser.parse_args()
    require(arguments.worker, 'use the replay API with explicit roots')
    raw = sys.stdin.buffer.read(8193)
    require(len(raw) <= 8192, 'verifier request exceeds its bound')
    print(json.dumps(worker(document(raw)), sort_keys=True))
