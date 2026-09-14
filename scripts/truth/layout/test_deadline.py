"""Absolute cutoffs include controller setup and process creation time."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from confinement import prepare_outputs, run_sandbox
from policy import Limits


class DeadlineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.cache = Path.home() / ".cache/lysilogy/source-layout-probe/synthetic"
        cls.cache.mkdir(parents=True, exist_ok=True)

    def invoke(self, root):
        output = prepare_outputs(root / "output", ("a",))
        return run_sandbox(run_dir=root, readonly=[], output=output, names=("a",),
                           command=["/runtime/bin/synthetic-unexecuted"], environment={},
                           limits=Limits(wall_seconds=30), deadline=1)

    def should_never_spawn_when_setup_has_consumed_the_absolute_deadline(self):
        with tempfile.TemporaryDirectory(dir=self.cache) as temporary:
            with patch("confinement.time.monotonic", side_effect=[0, 2, 2]), patch("confinement.subprocess.Popen") as spawn:
                result = self.invoke(Path(temporary))
            spawn.assert_not_called()
            self.assertEqual(result["status"], "wall_timeout")
            self.assertNotIn("pid", result)

    def should_shorten_the_wait_when_setup_and_process_creation_consume_time(self):
        with tempfile.TemporaryDirectory(dir=self.cache) as temporary:
            process = Mock(pid=123456, returncode=0)
            process.wait.return_value = 0
            with patch("confinement.time.monotonic", side_effect=[0, 0.2, 0.4, 0.7]), \
                    patch("confinement.subprocess.Popen", return_value=process), patch("confinement.os.killpg") as kill:
                result = self.invoke(Path(temporary))
            self.assertEqual(process.wait.call_args_list[0].kwargs, {"timeout": 0.6})
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["wall_seconds"], 0.7)
            kill.assert_called_once()

    def should_kill_without_another_wait_budget_when_process_creation_crosses_the_deadline(self):
        with tempfile.TemporaryDirectory(dir=self.cache) as temporary:
            process = Mock(pid=123456, returncode=0)
            with patch("confinement.time.monotonic", side_effect=[0, 0.4, 1.1, 1.2]), \
                    patch("confinement.subprocess.Popen", return_value=process), patch("confinement.os.killpg") as kill:
                result = self.invoke(Path(temporary))
            # Only the final reap is called; no stale positive timeout is used.
            process.wait.assert_called_once_with()
            self.assertEqual(result["status"], "wall_timeout")
            kill.assert_called_once()


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
