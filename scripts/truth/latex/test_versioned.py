"""Historical replay must not inherit changing parser modules or bytecode."""
import importlib.util
import json
import marshal
import re
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import versioned

ROOT = Path(__file__).resolve().parents[3]


def fixture(directory):
    repo = Path(directory) / 'repo'
    bundle = repo / 'eval/implementations' / versioned.VERSION
    shutil.copytree(ROOT / 'eval/implementations' / versioned.VERSION, bundle)
    truth = repo / 'eval/truth'
    shutil.copytree(ROOT / 'eval/truth' / versioned.VERSION, truth / versioned.VERSION)
    shutil.copy2(ROOT / 'eval/truth/k1-limited-v1-build.json', truth)
    scripts = repo / 'scripts/truth/latex'
    scripts.mkdir(parents=True)
    shutil.copy2(Path(versioned.__file__), scripts / 'versioned.py')
    return repo, bundle


def imports_in_worker(repo, contaminate=False, version=versioned.VERSION):
    code = '''import importlib.util,json,pathlib,sys,types
repo=pathlib.Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('versioned',repo/'scripts/truth/latex/versioned.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
original_path=sys.path[:]
if sys.argv[2]=='yes':sys.modules['parser']=types.ModuleType('parser')
loaded=module.load_modules(repo,sys.argv[3])
assert sys.path==original_path
print(json.dumps({name:str(value.__file__) for name,value in loaded.items()},sort_keys=True))
'''
    return subprocess.run([sys.executable, '-I', '-B', '-c', code, str(repo),
                           'yes' if contaminate else 'no', version], capture_output=True, timeout=10)


def second_fixture(directory, visual=False):
    """Synthetic publication pins; no real new release or manual labels."""
    repo, _ = fixture(directory)
    version = versioned.VISUAL_VERSION if visual else versioned.CURRENT_VERSION
    modules = versioned.VISUAL_MODULES if visual else versioned.CURRENT_MODULES
    bundle = repo / 'eval/implementations' / version
    bundle.mkdir()
    files = {}
    for name in modules:
        relative = 'scripts/corpus/corpus.py' if name == 'corpus.py' else 'scripts/truth/latex/' + name
        raw = (ROOT / relative).read_bytes()
        (bundle / name).write_bytes(raw)
        files[name] = {'source_path': relative, 'sha256': versioned.digest(raw), 'bytes': len(raw)}
    canonical = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode() + b'\n'
    config = canonical({'version': version})
    provenance = {'build_config_sha256': versioned.digest(config),
                  'implementation': {row['source_path']: row['sha256'] for row in files.values()}}
    payload = canonical({'version': version, 'provenance': provenance, 'papers': []})
    truth = repo / 'eval/truth' / version
    truth.mkdir()
    bibliography = canonical({'version': version + '-bibliography', 'provenance': provenance, 'papers': []})
    (truth / 'objects.json').write_bytes(payload)
    (truth / 'bibliography.json').write_bytes(bibliography)
    (truth.parent / (version + '-build.json')).write_bytes(config)
    manifest = {'schema_version': 1, 'release': version, 'files': files,
                'config_sha256': versioned.digest(config),
                'outputs': {'objects.json': versioned.digest(payload),
                            'bibliography.json': versioned.digest(bibliography)}}
    raw = canonical(manifest)
    (bundle / 'manifest.json').write_bytes(raw)
    manifest_hash = versioned.digest(raw)
    worker = repo / 'scripts/truth/latex/versioned.py'
    pin = 'VISUAL_MANIFEST_SHA256' if visual else 'CURRENT_MANIFEST_SHA256'
    worker.write_text(re.sub('^' + pin + r' = .*$',
                            pin + ' = ' + repr(manifest_hash),
                            worker.read_text(), flags=re.M))
    return repo, bundle, manifest_hash


class VersionedTests(unittest.TestCase):
    def test_should_keep_visual_release_disabled_when_its_independent_publication_pin_is_absent(self):
        with patch.object(versioned, 'VISUAL_MANIFEST_SHA256', None):
            with self.assertRaisesRegex(ValueError, 'not published'):
                versioned.pinned_release(ROOT, versioned.VISUAL_VERSION)
        self.assertEqual(len(versioned.release_spec(versioned.VERSION)[1]), 12)
        self.assertEqual(len(versioned.release_spec(versioned.CURRENT_VERSION)[1]), 14)

    def test_should_load_exact_fifteen_modules_when_visual_release_is_explicitly_selected(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, bundle, digest = second_fixture(directory, visual=True)
            with patch.object(versioned, 'VISUAL_MANIFEST_SHA256', digest):
                manifest = versioned.pinned_release(repo, versioned.VISUAL_VERSION)[1]
                self.assertEqual(set(manifest['files']), versioned.CURRENT_MODULES | {'tranche_visual.py'})
                (repo / 'scripts/truth/latex/tranche_visual.py').write_text('raise RuntimeError("unreviewed current source")')
                result = imports_in_worker(repo, version=versioned.VISUAL_VERSION)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                loaded = json.loads(result.stdout)
                self.assertEqual(len(loaded), 15)
                self.assertEqual(loaded['tranche_visual'], str(bundle / 'tranche_visual.py'))
                (bundle / 'tranche_visual.py').write_bytes(b'raise RuntimeError("replaced retained source")')
                with self.assertRaisesRegex(ValueError, 'module differs'):
                    versioned.pinned_release(repo, versioned.VISUAL_VERSION)

    def test_should_reject_unpublished_truth_when_no_reviewed_release_pin_exists(self):
        with patch.object(versioned, 'CURRENT_MANIFEST_SHA256', None):
            with self.assertRaisesRegex(ValueError, 'not published'):
                versioned.pinned_release(ROOT, versioned.CURRENT_VERSION)

    def test_should_select_separate_retained_sources_when_a_second_release_is_requested(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, bundle, manifest_hash = second_fixture(directory)
            with patch.object(versioned, 'CURRENT_MANIFEST_SHA256', manifest_hash):
                _, manifest, _, _, _ = versioned.pinned_release(repo, versioned.CURRENT_VERSION)
                self.assertEqual(set(manifest['files']), versioned.CURRENT_MODULES)
                # The old release still selects its original 12 source files.
                self.assertEqual(len(versioned.pinned_release(repo, versioned.VERSION)[1]['files']), 12)
                (repo / 'scripts/truth/latex/parser.py').write_text('raise RuntimeError("mutable source")')
                loaded = imports_in_worker(repo, version=versioned.CURRENT_VERSION)
                self.assertEqual(loaded.returncode, 0, loaded.stderr.decode())
                self.assertTrue(all(Path(path).parent == bundle for path in json.loads(loaded.stdout).values()))
                (bundle / 'tranche.py').write_bytes(b'changed')
                with self.assertRaisesRegex(ValueError, 'module differs'):
                    versioned.pinned_release(repo, versioned.CURRENT_VERSION)

    def test_should_reject_cross_version_payloads_when_new_release_files_are_replaced(self):
        for relative in ('objects.json', 'bibliography.json', 'config'):
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as directory:
                repo, _, manifest_hash = second_fixture(directory)
                truth = repo / 'eval/truth'
                source = truth / ('k1-limited-v1-build.json' if relative == 'config' else 'k1-limited-v1/' + relative)
                target = truth / ('k1-limited-v2-build.json' if relative == 'config' else 'k1-limited-v2/' + relative)
                target.write_bytes(source.read_bytes())
                with patch.object(versioned, 'CURRENT_MANIFEST_SHA256', manifest_hash):
                    with self.assertRaisesRegex(ValueError, 'immutable release bytes differ'):
                        versioned.pinned_release(repo, versioned.CURRENT_VERSION)

    def test_should_pin_original_implementation_and_outputs_when_the_release_is_selected(self):
        _, manifest, objects, bibliography, config = versioned.pinned_release(ROOT, versioned.VERSION)
        self.assertEqual(len(manifest['files']), 12)
        self.assertEqual(sum(row['bytes'] for row in manifest['files'].values()), 257691)
        self.assertEqual(versioned.digest(objects), manifest['outputs']['objects.json'])
        self.assertEqual(versioned.digest(bibliography), manifest['outputs']['bibliography.json'])
        self.assertEqual(versioned.digest(config), manifest['config_sha256'])

    def test_should_reject_an_unknown_version_when_a_caller_requests_other_truth(self):
        for name in ('k1-expanded-v2', '../k1-limited-v1', '', None):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'unsupported retained truth version'):
                versioned.verified_bundle(ROOT, name)

    def test_should_reject_module_or_manifest_drift_when_historical_code_is_modified(self):
        for name in ('parser.py', 'manifest.json'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                repo, bundle = fixture(directory)
                path = bundle / name
                path.write_bytes(path.read_bytes() + b'\n')
                with self.assertRaisesRegex(ValueError, 'differs'):
                    versioned.pinned_release(repo, versioned.VERSION)

    def test_should_reject_changed_labels_or_config_when_a_historical_release_is_rewritten(self):
        for name in ('k1-limited-v1/objects.json', 'k1-limited-v1/bibliography.json', 'k1-limited-v1-build.json'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                repo, _ = fixture(directory)
                path = repo / 'eval/truth' / name
                path.write_bytes(path.read_bytes() + b'\n')
                with self.assertRaisesRegex(ValueError, 'immutable release bytes differ'):
                    versioned.pinned_release(repo, versioned.VERSION)

    def test_should_reject_symlink_ancestors_when_bundle_storage_is_redirected(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, _ = fixture(directory)
            actual = Path(directory) / 'outside'
            (repo / 'eval').rename(actual)
            (repo / 'eval').symlink_to(actual, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, 'symlinked verifier input'):
                versioned.pinned_release(repo, versioned.VERSION)

    def test_should_compile_only_pinned_source_when_current_modules_and_cached_bytecode_disagree(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, bundle = fixture(directory)
            marker = Path(directory) / 'executed'
            poison = 'from pathlib import Path\nPath(' + repr(str(marker)) + ').write_text("executed")\nraise RuntimeError("wrong code")\n'
            (repo / 'scripts/truth/latex/parser.py').write_text(poison)
            (repo / 'scripts/corpus').mkdir()
            (repo / 'scripts/corpus/corpus.py').write_text(poison)
            # A valid timestamp/size header would fool a normal cached import.
            source = bundle / 'archive.py'
            cache = Path(importlib.util.cache_from_source(str(source)))
            cache.parent.mkdir()
            cache.write_bytes(importlib.util.MAGIC_NUMBER + struct.pack('<III', 0, int(source.stat().st_mtime), source.stat().st_size)
                              + marshal.dumps(compile(poison, str(source), 'exec')))
            result = imports_in_worker(repo)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            loaded = json.loads(result.stdout)
            self.assertEqual(set(loaded), {Path(name).stem for name in versioned.MODULES})
            self.assertTrue(all(Path(path).parent == bundle for path in loaded.values()))
            self.assertFalse(marker.exists())

    def test_should_reject_namespace_contamination_when_a_module_was_preloaded(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, _ = fixture(directory)
            result = imports_in_worker(repo, True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b'namespace is contaminated', result.stderr)

    def test_should_reject_a_different_worker_release_when_subprocess_output_is_mismatched(self):
        receipt = {'version': 'other', 'manifest_sha256': versioned.MANIFEST_SHA256,
                   'outputs': {}, 'reproduced': True}
        result = subprocess.CompletedProcess([], 0, json.dumps(receipt).encode(), b'')
        with patch.object(versioned.subprocess, 'run', return_value=result) as run:
            with self.assertRaisesRegex(ValueError, 'another release'):
                versioned.replay(ROOT, '/cache', '/corpus', '/data')
        self.assertEqual(run.call_args.args[0][1:3], ['-I', '-B'])
        self.assertEqual(json.loads(run.call_args.kwargs['input'])['repo'], str(ROOT))

    def test_should_reject_duplicate_or_nonfinite_fields_when_worker_json_is_malformed(self):
        for raw in (b'{"version":1,"version":2}', b'{"value":NaN}'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                versioned.document(raw)


if __name__ == '__main__':
    unittest.main()
