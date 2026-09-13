"""Historical replay must not inherit changing parser modules or bytecode."""
import importlib.util
import json
import marshal
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


def imports_in_worker(repo, contaminate=False):
    code = '''import importlib.util,json,pathlib,sys,types
repo=pathlib.Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('versioned',repo/'scripts/truth/latex/versioned.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
original_path=sys.path[:]
if sys.argv[2]=='yes':sys.modules['parser']=types.ModuleType('parser')
loaded=module.load_modules(repo,module.VERSION)
assert sys.path==original_path
print(json.dumps({name:str(value.__file__) for name,value in loaded.items()},sort_keys=True))
'''
    return subprocess.run([sys.executable, '-I', '-B', '-c', code, str(repo),
                           'yes' if contaminate else 'no'], capture_output=True, timeout=10)


class VersionedTests(unittest.TestCase):
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
