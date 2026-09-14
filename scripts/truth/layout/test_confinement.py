"""Required real kernel checks, using only the committed synthetic C probe.

Missing compilers, isolation tools or kernel support fail these tests. G5 runs
them in its own offline namespace too. No deposited source or TeX is executed.
"""
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from confinement import prepare_outputs, run_sandbox
from policy import Limits


class ConfinementTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cache = Path.home() / ".cache/lysilogy/source-layout-probe/synthetic"
        cache.mkdir(parents=True, exist_ok=True)
        cls.temporary = tempfile.TemporaryDirectory(prefix="kernel-", dir=cache)
        cls.root = Path(cls.temporary.name)
        cls.binary = cls.root / "probe"
        # Trusted fixture plus only its native loader/libc; no shell or /usr tree.
        result = subprocess.run(["cc", "-std=c11", "-O0", "-Wall", "-Wextra", "-Werror",
                                 str(Path(__file__).with_name("probe.c")), "-o", str(cls.binary)],
                                capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise AssertionError(result.stderr)
        cls.runtime = [(Path("/lib/ld-linux-aarch64.so.1").resolve(), "/lib/ld-linux-aarch64.so.1"),
                       (Path("/lib64/libc.so.6").resolve(), "/lib64/libc.so.6")]
        cls.source = cls.root / "input"
        cls.source.mkdir()
        (cls.source / "input.txt").write_text("synthetic input\n")
        (cls.root / "host-only-sentinel").write_text("unmounted synthetic sentinel\n")

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def invoke(self, mode, **limits):
        run = self.root / self._testMethodName
        run.mkdir()
        output = prepare_outputs(run / "output", ("a", "b"))
        return run_sandbox(run_dir=run, readonly=[(self.binary, "/runtime/bin/probe"), (self.source, "/input"), *self.runtime],
                           output=output, names=("a", "b"), command=["/runtime/bin/probe", mode],
                           environment={"PATH": "/runtime/bin"},
                           limits=Limits(file_bytes=4096, **limits))

    def should_confine_paths_processes_network_and_descriptors_when_running_untrusted_instructions(self):
        with (self.root / "host-only-sentinel").open("rb") as sentinel:
            os.dup2(sentinel.fileno(), 99, inheritable=True)
            try:
                with patch.dict(os.environ, {"SYNTHETIC_SENTINEL": "must not arrive"}):
                    receipt = self.invoke("isolation")
            finally:
                os.close(99)
        self.assertEqual(receipt["status"], "passed", Path(receipt["logs"][1]["path"]).read_text())
        self.assertEqual((self.source / "input.txt").read_text(), "synthetic input\n")
        self.assertEqual((self.root / "host-only-sentinel").read_text(), "unmounted synthetic sentinel\n")

    def should_bound_total_file_bytes_when_a_program_cycles_through_every_output(self):
        receipt = self.invoke("quota")
        self.assertEqual(receipt["status"], "passed", Path(receipt["logs"][1]["path"]).read_text())
        self.assertEqual(sum(item["bytes"] for item in receipt["outputs"]), 8192)
        self.assertEqual(receipt["aggregate_output_byte_bound"], 16384)

    def should_deny_large_mappings_when_address_space_is_exhausted(self):
        receipt = self.invoke("memory", address_bytes=128 * 1024 ** 2)
        self.assertEqual(receipt["status"], "passed")

    def should_kill_a_busy_loop_when_cpu_time_is_exhausted(self):
        receipt = self.invoke("cpu", cpu_seconds=1, wall_seconds=5)
        self.assertEqual(receipt["status"], "process_failed")
        self.assertIn("probe started", Path(receipt["logs"][0]["path"]).read_text())
        self.assertGreater(receipt["wall_seconds"], 0.5)
        self.assertLess(receipt["wall_seconds"], 5)

    def should_kill_and_reap_a_sleeper_when_the_wall_deadline_expires(self):
        receipt = self.invoke("wall", wall_seconds=0.25)
        self.assertEqual(receipt["status"], "wall_timeout")
        with self.assertRaises(ProcessLookupError):
            os.kill(receipt["pid"], 0)

    def should_preserve_an_interruption_receipt_when_the_controller_is_interrupted(self):
        # Deliver a signal to this trusted controller, then verify its cleanup.
        previous = signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
        signal.setitimer(signal.ITIMER_REAL, 0.25)
        try:
            with self.assertRaises(KeyboardInterrupt):
                self.invoke("wall", wall_seconds=5)
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, previous)
        import json
        receipt = json.loads((self.root / self._testMethodName / "receipt.json").read_text())
        self.assertEqual(receipt["status"], "interrupted")
        with self.assertRaises(ProcessLookupError):
            os.kill(receipt["pid"], 0)


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
