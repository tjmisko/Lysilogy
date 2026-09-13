#!/usr/bin/env python3
"""Run all Rust and frontend unit tests inside a new offline network namespace.

Called by `lysilogy eval tests`; no fallback can turn failed isolation into a pass.
Logs and the allowlisted PATH live beneath target/eval-g5, never the user's vault.
"""
import json
import os
from pathlib import Path
import shutil
import shlex
import re
import socket
import subprocess
import sys
import time
import unittest

MODELS = ("codex", "claude", "gemini", "aider", "ollama")
TOOLS = ("sh", "bash", "env", "node", "npm", "python3", "cc", "gcc", "as", "ld", "mold", "ld.mold", "ar",
         "pkg-config", "git", "pdftotext", "pdfinfo", "pdftoppm", "tesseract", "printf", "cat", "sleep",
         "mkdir", "chmod", "cp", "dirname", "basename", "head", "sed", "uname", "nice")


def tooling_tests(directory):
    # Repository tests use should_*_when_* names. Keep standard test_* support
    # too; an empty collection must never establish an offline-test pass.
    loader = unittest.TestLoader()
    loader.testMethodPrefix = ("test", "should_")
    suite = loader.discover(str(directory), pattern="test*.py")
    if suite.countTestCases() == 0:
        print(f"G5 FAIL: no tooling tests collected in {directory}", file=sys.stderr)
        return 1
    return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1


def unregistered_web_tests(web, script_commands):
    # Some frontend fixture suites intentionally have no npm convenience script.
    # Keep package commands intact and also run those otherwise omitted files.
    named = {token.removeprefix("./") for command in script_commands
             for token in shlex.split(command)}
    files = {str(path.relative_to(web)): path for path in sorted((web / "scripts").glob("*.test.mjs"))}
    pending = list(named.intersection(files))
    while pending:
        name = pending.pop()
        # Follow static side-effect imports, such as test:api importing objects
        # tests, without loading modules or reading outside the discovered files.
        imported = re.findall(r"\bimport\s+['\"]([^'\"]+\.test\.mjs)['\"]", files[name].read_text())
        for relative in imported:
            candidate = os.path.normpath(str(Path(name).parent / relative))
            if candidate in files and candidate not in named:
                named.add(candidate)
                pending.append(candidate)
    return [name for name in files if name not in named]


def run():
    root = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    if len(sys.argv) > 3 and sys.argv[3] == "--tooling-tests":
        return tooling_tests(Path(sys.argv[4]))
    if len(sys.argv) > 3 and sys.argv[3] == "--script-shell":
        # npm adds node_modules/.bin directories to PATH after our outer check.
        if any(shutil.which(name) for name in MODELS):
            raise RuntimeError("npm injected a model CLI into the test PATH")
        os.execv("/bin/sh", ["sh", *sys.argv[4:]])
    if len(sys.argv) == 3:
        # The parent namespace identity is recorded, then checked after unshare.
        command = ["unshare", "--user", "--map-root-user", "--net", sys.executable,
                   str(Path(__file__).resolve()), str(root), str(output),
                   os.readlink("/proc/self/ns/net")]
        return subprocess.call(command)

    started = time.monotonic()
    namespace = os.readlink("/proc/self/ns/net")
    if namespace == sys.argv[3]:
        raise RuntimeError("unshare did not change the network namespace")
    interfaces = sorted(line.split(":", 1)[0].strip() for line in Path("/proc/net/dev").read_text().splitlines()[2:] if ":" in line)
    if interfaces != ["lo"]:
        raise RuntimeError(f"unexpected network interfaces: {interfaces}")
    probe_errno = None
    with socket.socket() as connection:
        connection.settimeout(0.2)
        try:
            connection.connect(("192.0.2.1", 9))  # RFC 5737 documentation address.
        except OSError as error:
            probe_errno = error.errno
        else:
            raise RuntimeError("network isolation probe unexpectedly connected")
    # No routing to any external interface exists. Keep loopback down as created.
    bin_dir = output / "bin"
    bin_dir.mkdir(exist_ok=True)
    for path in bin_dir.iterdir():
        if not path.is_symlink():
            raise RuntimeError(f"unexpected file in generated executable directory: {path}")
        path.unlink()
    for name in TOOLS:
        found = shutil.which(name)
        if found:
            (bin_dir / name).symlink_to(Path(found).resolve())
    for name in ("cargo", "rustc", "rustdoc"):
        resolved = subprocess.check_output(["rustup", "which", name], text=True).strip()
        (bin_dir / name).symlink_to(resolved)
    environment = {name: os.environ[name] for name in ("HOME", "USER", "TMPDIR", "RUSTUP_HOME", "CARGO_HOME", "CARGO_TARGET_DIR") if name in os.environ}
    environment.update(PATH=str(bin_dir), CARGO_NET_OFFLINE="true", CARGO_BUILD_JOBS="1",
                       CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0",
                       CARGO_INCREMENTAL="0", LYSILOGY_EVAL_ISOLATED="1")
    absent = {name: shutil.which(name, path=str(bin_dir)) is None for name in MODELS}
    if not all(absent.values()):
        raise RuntimeError("a model CLI is present on the isolated PATH")
    package = json.loads((root / "web/package.json").read_text())
    scripts = sorted(name for name in package["scripts"] if name == "test" or name.startswith("test:"))
    if not scripts:
        raise RuntimeError("no frontend test scripts were discovered")
    shell_guard = output / "script-shell"
    shell_guard.write_text("#!/usr/bin/python3\nimport os, sys\nos.execv(" + repr(sys.executable) + ", [" + repr(sys.executable) + ", " + repr(str(Path(__file__).resolve())) + ", " + repr(str(root)) + ", " + repr(str(output)) + ", '--script-shell', *sys.argv[1:]])\n")
    shell_guard.chmod(0o700)
    commands = [(root, ["cargo", "test", "--offline", "--all-targets"])]
    # Discover committed tooling tests without traversing hidden data directories.
    for directory, subdirectories, files in os.walk(root / "scripts"):
        subdirectories[:] = sorted(name for name in subdirectories if not name.startswith("."))
        if any(name.startswith("test") and name.endswith(".py") for name in files):
            commands.append((root, ["python3", str(Path(__file__).resolve()), str(root),
                                   str(output), "--tooling-tests", str(directory)]))
    commands.extend((root / "web", ["npm", "--script-shell", str(shell_guard), "run", name]) for name in scripts)
    omitted = unregistered_web_tests(root / "web", [package["scripts"][name] for name in scripts])
    if omitted:
        commands.append((root / "web", ["node", "--experimental-strip-types", "--test", *omitted]))
    results = []
    for index, (cwd, command) in enumerate(commands):
        log = output / f"{index:02d}.log"
        command_start = time.monotonic()
        with log.open("w") as stream:
            result = subprocess.run(command, cwd=cwd, env=environment, stdout=stream, stderr=subprocess.STDOUT, timeout=1800, check=False)
        results.append(dict(command=command, cwd=str(cwd.relative_to(root)), exit_code=result.returncode,
                            wall_seconds=time.monotonic() - command_start, log=str(log.relative_to(root))))
        print(f"G5 {'PASS' if result.returncode == 0 else 'FAIL'}: {' '.join(command)}", file=sys.stderr, flush=True)
    evidence = dict(schema_version=1, network_namespace=namespace, parent_network_namespace=sys.argv[3],
                    interfaces=interfaces, network_probe_errno=probe_errno, model_clis_absent=absent,
                    path=str(bin_dir), commands=results, wall_seconds=time.monotonic() - started,
                    passed=all(result["exit_code"] == 0 for result in results))
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence))
    return 0 if evidence["passed"] else 1


if __name__ == "__main__":
    sys.exit(run())
