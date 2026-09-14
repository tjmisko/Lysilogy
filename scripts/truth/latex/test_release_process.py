import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import release_process as process


class ProcessTests(unittest.TestCase):
    def invoke(self, root, program, **kwargs):
        return process.run([sys.executable, '-I', '-B', '-c', program], b'{}', root / 'run',
                           cwd=root, seconds=kwargs.pop('seconds', 3), **kwargs)

    def test_should_retain_exact_boundary_when_both_output_pipes_are_drained(self):
        program = "import sys;sys.stdin.buffer.read();sys.stdout.buffer.write(b'a'*8192);sys.stderr.buffer.write(b'b'*8192)"
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            output, receipt = self.invoke(root, program, stdout_cap=8192)
            self.assertEqual(output, b'a' * 8192)
            self.assertEqual((root / 'run/stderr.bin').read_bytes(), b'b' * 8192)
            self.assertEqual(receipt['status'], 'passed')

    def test_should_retain_failed_prefix_when_output_is_one_byte_over_limit_or_status_is_nonzero(self):
        for length, code in ((8193, 0), (8192, 7)):
            with self.subTest(length=length), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                with self.assertRaises(process.ProcessFailure):
                    self.invoke(root, f"import sys;sys.stdin.buffer.read();sys.stdout.buffer.write(b'a'*{length});sys.exit({code})", stdout_cap=8192)
                receipt = json.loads((root / 'run/receipt.json').read_bytes())
                self.assertEqual(receipt['status'], 'failed')
                self.assertEqual(receipt['stdout']['bytes'], length)
                self.assertFalse(receipt['stdout']['complete'])

    def test_should_stop_when_stderr_overflows_or_the_worker_times_out(self):
        programs = [("import sys;sys.stdin.buffer.read();sys.stderr.buffer.write(b'x'*262145)", 3),
                    ("import sys,time;sys.stdin.buffer.read();time.sleep(10)", .05)]
        for program, seconds in programs:
            with self.subTest(program=program), tempfile.TemporaryDirectory() as raw:
                root = Path(raw); started = time.monotonic()
                with self.assertRaises(process.ProcessFailure):
                    self.invoke(root, program, seconds=seconds)
                self.assertLess(time.monotonic() - started, 2)
                receipt = json.loads((root / 'run/receipt.json').read_bytes())
                self.assertLessEqual(receipt['stderr']['bytes'], 262145)

    def test_should_kill_group_descendants_when_the_leader_exits_with_closed_or_inherited_pipes(self):
        for inherited in (False, True):
            for code in (0, 7):
                program = ("import sys,subprocess;sys.stdin.buffer.read();"
                           "p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(10)'],"
                           + ("stdout=None,stderr=None" if inherited else "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL")
                           + f");print(p.pid,flush=True);sys.exit({code})")
                with self.subTest(inherited=inherited, code=code), tempfile.TemporaryDirectory() as raw:
                    root = Path(raw)
                    try:
                        self.invoke(root, program, seconds=.2)
                    except process.ProcessFailure:
                        pass
                    pid = int((root / 'run/stdout.bin').read_bytes())
                    path = Path('/proc') / str(pid) / 'stat'
                    # A killed child may remain a zombie until its adopter reaps it.
                    def dead():
                        try:
                            return path.read_text().split()[2] == 'Z'
                        except (FileNotFoundError, ProcessLookupError):
                            return True
                    for _ in range(20):
                        if dead():
                            break
                        time.sleep(.01)
                    self.assertTrue(dead())

    def test_should_restore_caller_limits_when_the_bounded_operation_fails(self):
        before = process.resource.getrlimit(process.resource.RLIMIT_AS)
        with self.assertRaisesRegex(ValueError, 'control'):
            with process.address_limit():
                self.assertLessEqual(process.resource.getrlimit(process.resource.RLIMIT_AS)[0], process.MEMORY_BYTES)
                raise ValueError('control')
        self.assertEqual(process.resource.getrlimit(process.resource.RLIMIT_AS), before)

    def test_should_retain_failure_when_cancellation_or_late_spawn_interrupts_transport(self):
        for failure in (KeyboardInterrupt(), ValueError('process exceeded wall deadline')):
            with self.subTest(failure=type(failure)), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                with patch.object(process.subprocess, 'Popen', side_effect=failure):
                    with self.assertRaises((KeyboardInterrupt, process.ProcessFailure)):
                        self.invoke(root, 'unused')
                receipt = json.loads((root / 'run/receipt.json').read_bytes())
                self.assertEqual(receipt['status'], 'failed')
                self.assertFalse(receipt['stdout']['complete'])


if __name__ == '__main__':
    unittest.main()
