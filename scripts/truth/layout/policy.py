"""Fixed, fail-closed limits for the explicitly invoked layout experiment."""
from dataclasses import dataclass
import errno
import hashlib
import os
from pathlib import Path, PurePosixPath
import platform
import stat
import struct
import math
import re


class Refused(ValueError):
    """A required confinement or input condition was not established."""


@dataclass(frozen=True)
class Limits:
    wall_seconds: float = 90
    cpu_seconds: int = 45
    address_bytes: int = 768 * 1024 * 1024
    file_bytes: int = 16 * 1024 * 1024
    free_bytes: int = 20 * 1024 ** 3

    def validate(self):
        for name in self.__dataclass_fields__:
            value = getattr(self, name)
            if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
                raise Refused("limits must be positive numbers")
            if name != "wall_seconds" and type(value) is not int:
                raise Refused("kernel and storage limits must be integers")
        if self.wall_seconds > 90 or self.cpu_seconds > 45 or self.address_bytes > 768 * 1024 ** 2:
            raise Refused("experiment resource ceiling exceeded")
        if self.file_bytes > 16 * 1024 ** 2 or self.free_bytes < 20 * 1024 ** 3:
            raise Refused("output or storage policy weakened")


OUTPUT_NAMES = tuple("layout." + suffix for suffix in (
    "pdf", "log", "aux", "out", "toc", "lof", "lot", "nav", "snm",
    "fls", "synctex", "synctex.gz", "synctex(busy)", "synctex.gz(busy)",
))


def safe_relative(value):
    if not isinstance(value, str) or not value or len(value) > 1024:
        raise Refused("invalid relative path")
    if "\\" in value or any(ord(c) < 32 for c in value):
        raise Refused("invalid path characters")
    parts = value.split("/")
    if any(p in ("", ".", "..", ".env", ".secrets") or p.startswith(".env.") for p in parts):
        raise Refused("unsafe relative path")
    return PurePosixPath(value)


def regular_path(path, directory=False):
    """Reject symlinks in every component before inspecting any file contents."""
    path = Path(path).absolute()
    for component in reversed((path, *path.parents)):
        if component.name in (".env", ".secrets") or component.name.startswith(".env."):
            raise Refused("private filename in input or output path")
        mode = component.lstat().st_mode
        if stat.S_ISLNK(mode):
            raise Refused("symlink in input or output path")
    mode = path.stat().st_mode
    if not (stat.S_ISDIR(mode) if directory else stat.S_ISREG(mode)):
        raise Refused("expected a regular file or directory")
    return path


def binding(path):
    path = regular_path(path)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": digest.hexdigest()}


# Linux aarch64's native syscall ABI, from asm-generic/unistd.h. A positive
# allowlist also rejects unknown future syscalls, compat ABIs, io_uring, sockets,
# ptrace, namespace/mount changes and every process/thread creation syscall.
# execve is required for bubblewrap's one initial engine exec; the mounted
# executable inventory and inherited resource limits also constrain later exec.
AARCH64_CALLS = {
    "getcwd": 17, "dup": 23, "dup3": 24, "fcntl": 25,
    "flock": 32, "unlinkat": 35, "statfs": 43, "fstatfs": 44,
    "truncate": 45, "ftruncate": 46, "faccessat": 48, "chdir": 49,
    "openat": 56, "close": 57, "getdents64": 61, "lseek": 62,
    "read": 63, "write": 64, "readv": 65, "writev": 66, "pread64": 67,
    "pwrite64": 68, "readlinkat": 78, "newfstatat": 79, "fstat": 80,
    "fsync": 82, "fdatasync": 83, "exit": 93, "exit_group": 94, "waitid": 95,
    "set_tid_address": 96, "futex": 98, "set_robust_list": 99,
    "nanosleep": 101, "clock_gettime": 113, "clock_getres": 114,
    "clock_nanosleep": 115, "sched_yield": 124, "sigaltstack": 132,
    "rt_sigaction": 134, "rt_sigprocmask": 135, "rt_sigreturn": 139,
    "getpriority": 141, "times": 153, "uname": 160, "getrlimit": 163,
    "getrusage": 165, "umask": 166, "gettimeofday": 169, "getpid": 172,
    "getppid": 173, "getuid": 174, "geteuid": 175, "getgid": 176,
    "getegid": 177, "gettid": 178, "sysinfo": 179, "brk": 214,
    "munmap": 215, "mremap": 216, "execve": 221, "mmap": 222,
    "mprotect": 226, "madvise": 233, "wait4": 260,
    "renameat2": 276, "getrandom": 278, "membarrier": 283,
    "statx": 291, "rseq": 293, "faccessat2": 439,
}


def seccomp_filter(machine=None):
    if (machine or platform.machine()) != "aarch64":
        raise Refused("only the reviewed native aarch64 syscall ABI is supported")
    # sock_filter: BPF_LD|BPF_W|BPF_ABS, JEQ, RET. Wrong architecture kills;
    # a disallowed native syscall returns EPERM, never falls back to host tools.
    instructions = [(0x20, 0, 0, 4), (0x15, 1, 0, 0xC00000B7),
                    (0x06, 0, 0, 0x80000000), (0x20, 0, 0, 0)]
    for number in sorted(set(AARCH64_CALLS.values())):
        instructions.extend(((0x15, 0, 1, number), (0x06, 0, 0, 0x7FFF0000)))
    # glibc's getrlimit uses prlimit64(pid, resource, NULL, old_limit).
    # Permit only querying; reject either nonzero half of the new-limit pointer.
    instructions.extend(((0x15, 0, 5, 261), (0x20, 0, 0, 32),
                         (0x15, 0, 3, 0), (0x20, 0, 0, 36),
                         (0x15, 0, 1, 0), (0x06, 0, 0, 0x7FFF0000)))
    instructions.append((0x06, 0, 0, 0x00050000 | errno.EPERM))
    return b"".join(struct.pack("=HBBI", *instruction) for instruction in instructions)


def fixed_environment(epoch):
    if type(epoch) is not int or not 0 <= epoch <= 4102444800:
        raise Refused("a bounded, frozen source date is required")
    return {
        "PATH": "/runtime/bin", "HOME": "/unmounted", "LANG": "C", "LC_ALL": "C",
        "TZ": "UTC", "SOURCE_DATE_EPOCH": str(epoch), "FORCE_SOURCE_DATE": "1",
        "TEXMFCNF": "/runtime/config", "TEXMF": "/runtime/texmf",
        "TEXMFDBS": "/runtime/texmf", "TEXMFHOME": "/unmounted",
        "TEXMFVAR": "/unmounted", "TEXMFCONFIG": "/unmounted",
        "TEXINPUTS": "/input//:/runtime/texmf/tex//",
        "TEXFONTMAPS": "/runtime/maps:/runtime/texmf/fonts/map//",
        "TEXFORMATS": "/runtime/format", "TEXMFOUTPUT": "/output",
        "TEXMFTEMP": "/unmounted",
        "openin_any": "p", "openout_any": "p", "shell_escape": "0",
        "MKTEXFMT": "0", "MKTEXTEX": "0", "MKTEXPK": "0", "MKTEXTFM": "0",
    }


def job_name(main):
    safe_relative(main)
    if not main.endswith(".tex") or not re.fullmatch(r"[A-Za-z0-9_-]{1,80}", PurePosixPath(main).stem):
        raise Refused("unsupported literal TeX job name")
    return PurePosixPath(main).stem


def output_names(main):
    job = job_name(main)
    # This fixed bubblewrap topology has reaper PID1 and engine PID2. pdfTeX
    # opens its recorder before processing -jobname. No general filename slot
    # or writable directory is introduced; another topology simply fails.
    return tuple(job + name.removeprefix("layout") for name in OUTPUT_NAMES) + ("pdflatex2.fls",)


def engine_command(main):
    safe_relative(main)
    if not main.endswith(".tex"):
        raise Refused("the unique source root must be a TeX file")
    return ["/runtime/bin/pdftex", "-progname=pdflatex", "-fmt=/runtime/format/pdflatex.fmt",
            "-no-shell-escape", "-no-parse-first-line", "-interaction=nonstopmode",
            "-halt-on-error", "-file-line-error", "-recorder", "-synctex=1",
            "-jobname=" + job_name(main), "-output-directory=/output", "/input/" + main]
