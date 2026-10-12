"""Prove exact source loading, without any TeX, rendering or corpus access."""
import hashlib
import importlib._bootstrap_external
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from execute import MODULES, load_snapshot, raw_file
from confinement import check_tools
from policy import binding


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


class SnapshotTests(unittest.TestCase):
    def prepare(self, root):
        root.mkdir()
        (root / "modules").mkdir()
        bootstrap = Path(__file__).with_name("execute.py").read_bytes()
        (root / "execute.py").write_bytes(bootstrap)
        modules = {}
        for name in MODULES:
            raw = b"VALUE = 'verified source'\n"
            path = root / "modules" / (name + ".py")
            path.write_bytes(raw)
            modules[name] = digest(raw)
            # A valid timestamp/size cache would be accepted by normal imports,
            # even under -B. Its payload must never execute in this launcher.
            cached = Path(importlib.util.cache_from_source(str(path)))
            cached.parent.mkdir(exist_ok=True)
            code = compile("raise RuntimeError('hostile cached bytecode')", str(path), "exec")
            cached.write_bytes(importlib._bootstrap_external._code_to_timestamp_pyc(code, int(path.stat().st_mtime), len(raw)))
        for name in ("selection.json", "runtime.json"):
            (root / name).write_text("{}\n")
        launch = {"source_commit": "synthetic", "modules": modules,
                  "confinement_tools": check_tools(), "python": binding(Path(sys.executable).resolve()),
                  "python_version": sys.version,
                  "bootstrap_sha256": digest(bootstrap),
                  "inputs": {name: digest((root / name).read_bytes()) for name in ("selection.json", "runtime.json")}}
        (root / "launch.json").write_text(json.dumps(launch))
        launch_sha = digest((root / "launch.json").read_bytes())
        review = {"status": "clear", "launch_sha256": launch_sha, "source_commit": "synthetic"}
        (root / "review.json").write_text(json.dumps(review))
        review_sha = digest((root / "review.json").read_bytes())
        return launch_sha, review_sha

    def should_execute_verified_sources_when_valid_hostile_bytecode_is_present(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "snapshot"
            launch, review = self.prepare(root)
            result = subprocess.run(["python3", "-I", "-S", "-B", str(root / "execute.py"),
                                     "--launch-sha256", launch, "--review", str(root / "review.json"),
                                     "--review-sha256", review, "--check-only"],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["status"], "verified_without_execution")
            self.assertFalse((root / "experiment").exists())

    def should_refuse_changed_code_inputs_or_review_when_frozen_bindings_do_not_match(self):
        for changed in ("modules/policy.py", "selection.json", "runtime.json", "execute.py", "review.json", "launch.json"):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "snapshot"
                launch, review = self.prepare(root)
                with (root / changed).open("ab") as stream:
                    stream.write(b" ")
                with self.assertRaises(ValueError):
                    load_snapshot(root, launch, root / "review.json", review)

    def should_refuse_symlinks_when_the_payload_bytes_would_otherwise_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "snapshot"
            launch, review = self.prepare(root)
            original = root / "modules/policy.py"
            moved = root / "original.py"
            original.rename(moved)
            original.symlink_to(moved)
            with self.assertRaises(ValueError):
                load_snapshot(root, launch, root / "review.json", review)

    def should_refuse_a_different_reviewed_source_when_the_review_hash_is_valid(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "snapshot"
            launch, _ = self.prepare(root)
            path = root / "review.json"
            review = json.loads(path.read_text())
            review["source_commit"] = "other-source"
            path.write_text(json.dumps(review))
            with self.assertRaises(ValueError):
                load_snapshot(root, launch, path, digest(path.read_bytes()))

    def should_refuse_changed_executables_when_reviewed_tool_bytes_or_python_differ(self):
        for changed in ("bwrap", "unshare", "python", "python_version"):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "snapshot"
                self.prepare(root)
                path = root / "launch.json"
                launch = json.loads(path.read_text())
                if changed == "python_version":
                    launch[changed] = "wrong Python version"
                else:
                    expected = launch["python"] if changed == "python" else launch["confinement_tools"][changed]
                    expected["sha256"] = "0" * 64
                path.write_text(json.dumps(launch))
                launch_sha = digest(path.read_bytes())
                review_path = root / "review.json"
                review_path.write_text(json.dumps({"status": "clear", "source_commit": "synthetic", "launch_sha256": launch_sha}))
                with self.assertRaises(ValueError):
                    load_snapshot(root, launch_sha, review_path, digest(review_path.read_bytes()))


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
