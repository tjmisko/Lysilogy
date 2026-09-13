"""Regression tests for the production G5 tooling-test subprocess."""

from pathlib import Path
import subprocess
import runpy
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ToolingDiscoveryTests(unittest.TestCase):
    def run_fixture(self, directory, body):
        cases = directory / "cases"
        cases.mkdir()
        (cases / "test_fixture.py").write_text(body)
        return subprocess.run(
            [sys.executable, str(ROOT / "src/eval/g5.py"), str(ROOT),
             str(directory / "output"), "--tooling-tests", str(cases)],
            capture_output=True, text=True, check=False,
        )

    def should_execute_both_naming_conventions_when_tooling_suites_are_discovered(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            result = self.run_fixture(directory, """import unittest
from pathlib import Path
class Fixture(unittest.TestCase):
    def should_execute_when_discovered(self):
        Path(__file__).with_name('should-ran').write_text('yes')
    def test_conventional_name(self):
        Path(__file__).with_name('test-ran').write_text('yes')
""")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Ran 2 tests", result.stderr)
            self.assertTrue((directory / "cases/should-ran").is_file())
            self.assertTrue((directory / "cases/test-ran").is_file())

    def should_fail_when_a_should_named_tooling_test_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = self.run_fixture(Path(temporary), """import unittest
class Fixture(unittest.TestCase):
    def should_fail_when_executed(self):
        self.fail('failure must reach G5')
""")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("failure must reach G5", result.stderr)

    def should_include_unregistered_frontend_tests_when_package_commands_omit_them(self):
        discover = runpy.run_path(str(ROOT / "src/eval/g5.py"))["unregistered_web_tests"]
        with tempfile.TemporaryDirectory() as temporary:
            web = Path(temporary)
            (web / "scripts").mkdir()
            for name in ("registered.test.mjs", "objects.test.mjs", "smoke.mjs"):
                (web / "scripts" / name).write_text("fixture")
            self.assertEqual(discover(web, ["node --test ./scripts/registered.test.mjs"]),
                             ["scripts/objects.test.mjs"])
            (web / "scripts/registered.test.mjs").write_text("import './objects.test.mjs';")
            self.assertEqual(discover(web, ["node --test ./scripts/registered.test.mjs"]), [])

    def should_fail_when_a_tooling_suite_collects_no_tests(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = self.run_fixture(Path(temporary), "value = 'no test cases'\n")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("no tooling tests collected", result.stderr)


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
