"""Inert policy and bytecode tests; no network, model or deposited source."""
import errno
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from confinement import check_tools, prepare_outputs, readonly_tree, reject_readwrite_aliases, sandbox_command
from policy import Limits, Refused, engine_command, fixed_environment, regular_path, safe_relative, seccomp_filter


def interpret(program, number, architecture=0xC00000B7, new_limit=0):
    words = {0: number, 4: architecture, 32: new_limit & 0xFFFFFFFF, 36: new_limit >> 32}
    rows = list(struct.iter_unpack("=HBBI", program))
    position, accumulator = 0, None
    while position < len(rows):
        code, yes, no, value = rows[position]
        if code == 0x20:
            accumulator = words[value]
        elif code == 0x15:
            position += yes if accumulator == value else no
        elif code == 0x06:
            return value
        else:
            raise AssertionError("unexpected BPF opcode")
        position += 1
    raise AssertionError("BPF must return")


class PolicyTests(unittest.TestCase):
    def should_reject_unsafe_members_when_paths_escape_or_name_private_files(self):
        for name in ("../x", "/x", "a//b", "a/./b", "a/../b", "a\\b", "x\x00", ".env", "a/.env.local", "a/.secrets/x"):
            with self.subTest(name=name), self.assertRaises(Refused):
                safe_relative(name)
        self.assertEqual(str(safe_relative("figures/a-b_2.pdf")), "figures/a-b_2.pdf")

    def should_refuse_weakened_limits_when_overrides_exceed_the_frozen_ceiling(self):
        for kwargs in ({"wall_seconds": 91}, {"wall_seconds": float("nan")}, {"file_bytes": 20 * 1024 ** 2},
                       {"cpu_seconds": 46}, {"address_bytes": 1024 ** 3}, {"free_bytes": 1}, {"cpu_seconds": 1.5}):
            with self.subTest(kwargs=kwargs), self.assertRaises(Refused):
                Limits(**kwargs).validate()
        Limits(wall_seconds=0.5, cpu_seconds=1, file_bytes=4096).validate()

    def should_refuse_an_unknown_abi_when_a_filter_would_use_wrong_syscalls(self):
        with self.assertRaises(Refused):
            seccomp_filter("x86_64")
        self.assertEqual(interpret(seccomp_filter("aarch64"), 63, architecture=0x40000028), 0x80000000)

    def should_deny_process_network_and_namespace_calls_when_sandboxed(self):
        program = seccomp_filter("aarch64")
        # Independent Linux ABI examples: clone, clone3, socket, connect,
        # unshare, mount, ptrace, io_uring_setup, execveat, setrlimit, ioctl.
        for number in (220, 435, 198, 203, 97, 40, 117, 425, 281, 164, 29, 9999):
            with self.subTest(number=number):
                self.assertEqual(interpret(program, number), 0x50000 | errno.EPERM)
        for number in (63, 64, 56, 57, 221, 222):
            self.assertEqual(interpret(program, number), 0x7FFF0000)

    def should_only_query_limits_when_prlimit_receives_a_new_limit_pointer(self):
        program = seccomp_filter("aarch64")
        self.assertEqual(interpret(program, 261), 0x7FFF0000)
        for pointer in (1, 0x100000000, 0xFFFFFFFFFFFFFFFF):
            self.assertEqual(interpret(program, 261, new_limit=pointer), 0x50000 | errno.EPERM)

    def should_refuse_missing_tools_when_isolation_cannot_start(self):
        with patch("confinement.shutil.which", return_value=None), self.assertRaises(Refused):
            check_tools()

    def should_ignore_host_environment_when_constructing_engine_configuration(self):
        with patch.dict(os.environ, {"TEXINPUTS": "/synthetic-secret", "LD_PRELOAD": "/synthetic-preload", "TOKEN": "sentinel"}):
            env = fixed_environment(1600000000)
        self.assertNotIn("LD_PRELOAD", env)
        self.assertNotIn("TOKEN", env)
        self.assertNotIn("synthetic", env["TEXINPUTS"])
        self.assertEqual(env["MKTEXFMT"], "0")

    def should_preserve_the_original_job_name_when_constructing_a_fixed_engine_command(self):
        command = engine_command("paper/original-name.tex")
        self.assertEqual(command[-1], "/input/paper/original-name.tex")
        self.assertIn("-jobname=original-name", command)
        self.assertIn("-no-shell-escape", command)
        self.assertIn("-no-parse-first-line", command)
        with self.assertRaises(Refused):
            engine_command("paper/$(touch sentinel).tex")

    def should_reject_stale_outputs_and_links_when_preparing_a_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            out = prepare_outputs(root / "out", ("a", "b"))
            with self.assertRaises(FileExistsError):
                prepare_outputs(out, ("a", "b"))
            (root / "alias").symlink_to(out, target_is_directory=True)
            with self.assertRaises(Refused):
                regular_path(root / "alias/a")
            source = root / "source"
            source.mkdir()
            (source / "link").symlink_to(root / "out/a")
            with self.assertRaises(Refused):
                readonly_tree(source)

    def should_refuse_reserved_mounts_when_controller_input_would_overlay_isolation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "input"
            path.write_text("synthetic")
            tools = {n: {"path": "/usr/bin/" + n} for n in ("unshare", "bwrap")}
            for destination in ("/proc/1", "/dev/other", "/output/a", "/home", "/runtime/../proc"):
                with self.subTest(destination=destination), self.assertRaises(Refused):
                    sandbox_command(tools, [(path, destination)], root, [], ["/runtime/bin/probe"], {}, 3)

    def should_reject_readwrite_aliases_when_a_source_or_runtime_mount_contains_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run = root / "run"
            run.mkdir()
            output = prepare_outputs(run / "output", ("a", "b"))
            ids = {p.name: (p.stat().st_dev, p.stat().st_ino) for p in output.iterdir()}
            for source in (output, output / "a", run, root):
                with self.subTest(source=source), self.assertRaises(Refused):
                    reject_readwrite_aliases([(source, "/input")], run, output, ids)
            outside = root / "readonly-runtime"
            outside.mkdir()
            (outside / "alias").hardlink_to(output / "a")
            with self.assertRaises(Refused):
                reject_readwrite_aliases([(outside, "/runtime/files")], run, output, ids)
            self.assertEqual((output / "a").read_bytes(), b"")
            self.assertFalse((run / "before.json").exists())


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
