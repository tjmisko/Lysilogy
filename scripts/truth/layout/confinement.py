"""A single-process, offline sandbox with a finite set of writable inodes.

No deposited source is run on import. A caller must prepare and independently
review the exact runtime, source selection and command before a live experiment.
"""
import json
import math
import os
from pathlib import Path
import resource
import shutil
import signal
import stat
import subprocess
import time

from policy import Limits, Refused, binding, regular_path, safe_relative, seccomp_filter


def check_tools():
    result = {}
    for name in ("unshare", "bwrap"):
        path = shutil.which(name)
        if path is None:
            raise Refused("required confinement tool unavailable: " + name)
        path = Path(path).resolve(strict=True)
        result[name] = binding(path)
    seccomp_filter()  # Fail before creating a run on an unsupported architecture.
    return result


def prepare_outputs(directory, names):
    """Create a fresh, private directory. No stale files or links are reused."""
    directory = Path(directory).absolute()
    regular_path(directory.parent, directory=True)
    if len(names) > 32 or not names or len(set(names)) != len(names):
        raise Refused("invalid finite output inventory")
    for name in names:
        safe_relative(name)
        if "/" in name:
            raise Refused("output inventory must contain only direct filenames")
    directory.mkdir(mode=0o700, exist_ok=False)
    for name in names:
        with (directory / name).open("xb"):
            pass
    return directory


def readonly_tree(path):
    path = regular_path(path, directory=True)
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            safe_relative(name)
            item = Path(base) / name
            mode = item.lstat().st_mode
            if not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
                raise Refused("only regular source members and directories can be mounted")
    return path


def reject_readwrite_aliases(readonly, run_dir, output, output_ids):
    """Check before creating any controller file, including filter/receipts.

    Path ancestry catches ordinary aliases. Device/inode checks additionally
    catch hard links and host bind aliases with different apparent paths.
    """
    writable_ids = set(output_ids.values())
    for path in (run_dir, output, *run_dir.iterdir()):
        metadata = path.lstat()
        writable_ids.add((metadata.st_dev, metadata.st_ino))
    for source, _ in readonly:
        source = Path(source).absolute()
        source = regular_path(source, directory=source.is_dir())
        for writable_root in (run_dir, output):
            if source.is_relative_to(writable_root) or writable_root.is_relative_to(source):
                raise Refused("read-only and writable roots overlap")
        candidates = [source]
        if source.is_dir():
            for base, dirs, files in os.walk(source, followlinks=False):
                for name in dirs + files:
                    safe_relative(name)
                    candidates.append(Path(base) / name)
        for path in candidates:
            metadata = path.lstat()
            if (metadata.st_dev, metadata.st_ino) in writable_ids:
                raise Refused("read-only path aliases a writable inode")


def sandbox_command(tools, readonly, output, names, command, environment, filter_fd):
    """Construct mounts only from reviewed controller inputs, never source text."""
    if not command or not command[0].startswith("/"):
        raise Refused("an absolute sandbox executable is required")
    argv = [tools["unshare"]["path"], "--user", "--map-root-user", "--net", "--",
            tools["bwrap"]["path"], "--unshare-pid", "--unshare-ipc", "--unshare-uts",
            "--unshare-cgroup", "--die-with-parent", "--new-session", "--clearenv",
            "--cap-drop", "ALL", "--proc", "/proc", "--remount-ro", "/proc",
            "--dir", "/work", "--dir", "/dev", "--chdir", "/work"]
    destinations = set()
    for source, destination in readonly:
        if not destination.startswith("/") or str(Path(destination)) != destination:
            raise Refused("noncanonical mount destination")
        if destination in destinations or not (destination == "/input" or destination.startswith(("/runtime/", "/lib/", "/lib64/", "/usr/lib64/"))):
            raise Refused("reserved or repeated mount destination")
        safe_relative(destination[1:])
        destinations.add(destination)
        source = Path(source)
        regular_path(source, directory=source.is_dir())
        if destination == "/input":
            readonly_tree(source)
        argv.extend(("--ro-bind", str(source), destination))
    # No writable /dev directory or tmpfs. Only this existing harmless device.
    argv.extend(("--ro-bind", "/dev/null", "/dev/null", "--ro-bind", str(output), "/output"))
    for name in names:
        argv.extend(("--bind", str(output / name), "/output/" + name))
    for key, value in sorted(environment.items()):
        argv.extend(("--setenv", key, value))
    argv.extend(("--remount-ro", "/", "--seccomp", str(filter_fd), "--", *command))
    return argv


def run_sandbox(*, run_dir, readonly, output, names, command, environment, limits=Limits(), expected_tools=None, deadline=None):
    """One bounded invocation; interruption kills the complete namespace job.

    All writable mounts name pre-existing regular inodes. Their parent mount,
    root, inputs, runtime and procfs are read-only. RLIMIT_FSIZE bounds each
    inode, including captured stdout/stderr, so no monitor-latency estimate is
    needed. An engine cannot create new files, rename a mount, or use fallocate,
    ioctl reflinks, io_uring, process creation, or a socket to evade this bound.
    """
    limits.validate()
    started = time.monotonic()
    if deadline is not None and (type(deadline) not in (int, float) or not math.isfinite(deadline)):
        raise Refused("invalid absolute process deadline")
    cutoff = min(started + limits.wall_seconds, deadline) if deadline is not None else started + limits.wall_seconds
    tools = check_tools()
    if expected_tools is not None and tools != expected_tools:
        raise Refused("confinement tool identity changed since snapshot review")
    run_dir = regular_path(run_dir, directory=True)
    output = regular_path(output, directory=True)
    if shutil.disk_usage(run_dir).free < limits.free_bytes:
        raise Refused("free storage is below the experiment floor")
    if len(names) > 32 or not names or len(set(names)) != len(names):
        raise Refused("invalid finite output inventory")
    if set(p.name for p in output.iterdir()) != set(names):
        raise Refused("unexpected output inventory")
    output_ids = {}
    for name in names:
        safe_relative(name)
        if "/" in name:
            raise Refused("output must be a direct regular file")
        path = regular_path(output / name)
        metadata = path.stat()
        if metadata.st_nlink != 1 or metadata.st_size > limits.file_bytes:
            raise Refused("linked or oversized output file")
        output_ids[name] = (metadata.st_dev, metadata.st_ino)
    if len(set(output_ids.values())) != len(names):
        raise Refused("output inventory aliases an inode")
    reject_readwrite_aliases(readonly, run_dir, output, output_ids)
    filter_path = run_dir / "filter.bpf"
    if not filter_path.exists():
        with filter_path.open("xb") as stream:
            stream.write(seccomp_filter())
    if regular_path(filter_path).read_bytes() != seccomp_filter():
        raise Refused("unexpected seccomp program")
    record = {"schema_version": 1, "tools": tools, "filter": binding(filter_path),
              "command": command, "environment": environment, "limits": vars(limits),
              "deadline_monotonic": cutoff,
              "writable_file_count": len(names) + 2,
              "aggregate_output_byte_bound": (len(names) + 2) * limits.file_bytes,
              "network_calls": 0, "model_calls": 0, "external_cost_usd": 0,
              "status": "prepared"}
    with (run_dir / "before.json").open("x") as stream:
        json.dump(record, stream, sort_keys=True, indent=2)
    process = None
    previous_signals = {}

    def interrupted(signum, _frame):
        raise InterruptedError("controller received signal " + str(signum))

    def resource_limits():
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        resource.setrlimit(resource.RLIMIT_CPU, (limits.cpu_seconds, limits.cpu_seconds))
        resource.setrlimit(resource.RLIMIT_AS, (limits.address_bytes, limits.address_bytes))
        resource.setrlimit(resource.RLIMIT_FSIZE, (limits.file_bytes, limits.file_bytes))
        resource.setrlimit(resource.RLIMIT_NOFILE, (64, 64))

    try:
        for signum in (signal.SIGTERM, signal.SIGHUP):
            previous_signals[signum] = signal.signal(signum, interrupted)
        with filter_path.open("rb") as filt, (run_dir / "stdout.log").open("xb") as out, (run_dir / "stderr.log").open("xb") as err:
            argv = sandbox_command(tools, readonly, output, names, command, environment, filt.fileno())
            record["argv"] = argv
            # Tool verification, tree walks and mount construction consume the
            # same deadline as execution. Never launch using a stale duration.
            if time.monotonic() >= cutoff:
                record["status"] = "wall_timeout"
            else:
                process = subprocess.Popen(argv, env={"PATH": "/usr/bin"}, stdin=subprocess.DEVNULL,
                                           stdout=out, stderr=err, close_fds=True,
                                           pass_fds=(filt.fileno(),), start_new_session=True,
                                           preexec_fn=resource_limits)
                record["pid"] = process.pid
                remaining = cutoff - time.monotonic()
                if remaining <= 0:
                    record["status"] = "wall_timeout"
                else:
                    try:
                        record["exit_code"] = process.wait(timeout=remaining)
                        record["status"] = "passed" if process.returncode == 0 else "process_failed"
                    except subprocess.TimeoutExpired:
                        record["status"] = "wall_timeout"
    except BaseException as error:
        record["status"] = "interrupted" if isinstance(error, (KeyboardInterrupt, SystemExit, InterruptedError)) else "failed"
        record["error"] = type(error).__name__ + ": " + str(error)
        raise
    finally:
        if process is not None:
            # unshare/bwrap may have exited while a descendant remains; always
            # kill the launch group. die-with-parent kills the isolated job.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        for signum, previous in previous_signals.items():
            signal.signal(signum, previous)
        record["wall_seconds"] = time.monotonic() - started
        record["controller_lifetime_child_peak_rss_kib"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        record["outputs"] = [binding(output / name) for name in names]
        record["logs"] = [binding(run_dir / name) for name in ("stdout.log", "stderr.log") if (run_dir / name).is_file()]
        record["inodes_unchanged"] = all((p.stat().st_dev, p.stat().st_ino) == output_ids[p.name] for p in output.iterdir())
        record["output_bound_passed"] = all(p["bytes"] <= limits.file_bytes for p in record["outputs"] + record["logs"])
        with (run_dir / "receipt.json").open("x") as stream:
            json.dump(record, stream, sort_keys=True, indent=2)
            stream.write("\n")
    if not record["inodes_unchanged"] or not record["output_bound_passed"]:
        raise Refused("output confinement invariant failed")
    return record
