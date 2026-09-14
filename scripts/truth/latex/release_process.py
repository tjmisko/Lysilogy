"""Finite subprocess transport for the new serial K1 path only."""
import hashlib
from contextlib import contextmanager
import json
import os
from pathlib import Path
import selectors
import resource
import signal
import subprocess
import time

from release_layout import MANIFEST_BYTES, PAPER_BYTES, canonical, direct, require

STDERR_BYTES = 256 * 1024
SCRATCH_BYTES = 8192
MEMORY_BYTES = 768 * 1024 * 1024


@contextmanager
def address_limit():
    """Bound the new coordinator and its children; restore the caller's soft limit."""
    before = resource.getrlimit(resource.RLIMIT_AS)
    soft = MEMORY_BYTES if before[0] == resource.RLIM_INFINITY else min(before[0], MEMORY_BYTES)
    resource.setrlimit(resource.RLIMIT_AS, (soft, before[1]))
    try:
        yield
    finally:
        resource.setrlimit(resource.RLIMIT_AS, before)


class ProcessFailure(ValueError):
    def __init__(self, receipt):
        self.receipt = receipt
        super().__init__(receipt['error'])


def run(argv, request, directory, *, cwd, seconds=90, stdout_cap=MANIFEST_BYTES, environment=None):
    """Retain exact bounded request/output prefixes and kill all owned group members."""
    require(type(seconds) in (int, float) and 0 < seconds <= 90,
            'invalid per-paper process deadline')
    require(type(stdout_cap) is int and 0 < stdout_cap <= PAPER_BYTES, 'invalid process output cap')
    require(type(request) is bytes and len(request) <= MANIFEST_BYTES, 'process request exceeds bound')
    directory = direct(directory); directory.mkdir(parents=True)
    (directory / 'request.json').write_bytes(request)
    started = time.monotonic(); deadline = started + seconds
    output = {'stdout': bytearray(), 'stderr': bytearray()}
    limits = {'stdout': stdout_cap, 'stderr': STDERR_BYTES}
    receipt = {'schema_version': 1, 'argv': list(map(str, argv)), 'cwd': str(cwd),
               'seconds_limit': seconds, 'stdout_limit': stdout_cap, 'stderr_limit': STDERR_BYTES,
               'request_sha256': hashlib.sha256(request).hexdigest(), 'status': 'failed',
               'network_calls': 0, 'model_calls': 0}
    child = None; selector = selectors.DefaultSelector(); cursor = 0

    def timely():
        require(time.monotonic() < deadline, 'process exceeded wall deadline')

    try:
        timely()
        child = subprocess.Popen(argv, cwd=cwd, env=environment, stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        timely()
        for stream, name in ((child.stdout, 'stdout'), (child.stderr, 'stderr')):
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ, name)
        os.set_blocking(child.stdin.fileno(), False)
        if request:
            selector.register(child.stdin, selectors.EVENT_WRITE, 'stdin')
        else:
            child.stdin.close()
        while selector.get_map():
            timely()
            for key, _ in selector.select(min(.05, max(0, deadline - time.monotonic()))):
                name = key.data
                if name == 'stdin':
                    try:
                        written = os.write(key.fileobj.fileno(), request[cursor:cursor + SCRATCH_BYTES])
                    except BlockingIOError:
                        continue
                    cursor += written
                    if cursor == len(request):
                        selector.unregister(key.fileobj); key.fileobj.close()
                    continue
                try:
                    part = os.read(key.fileobj.fileno(), min(SCRATCH_BYTES, limits[name] + 1 - len(output[name])))
                except BlockingIOError:
                    continue
                if not part:
                    selector.unregister(key.fileobj); key.fileobj.close()
                    continue
                output[name].extend(part)
                require(len(output[name]) <= limits[name], name + ' exceeded byte bound')
        timely()
        child.wait(timeout=max(.001, deadline - time.monotonic()))
        timely()
        require(cursor == len(request), 'worker did not consume complete request')
        require(child.returncode == 0, 'worker returned nonzero status')
        receipt['status'] = 'passed'
    except (OSError, ValueError, MemoryError, subprocess.SubprocessError) as error:
        receipt['error'] = str(error)[:4096]
    finally:
        selector.close()
        if child is not None:
            # Leader exit is not proof that inherited descendants exited.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                child.wait(timeout=1)
            except subprocess.TimeoutExpired:
                receipt['status'] = 'failed'; receipt['error'] = 'worker cleanup exceeded one second'
            for stream in (child.stdin, child.stdout, child.stderr):
                stream.close()
            receipt['exit_code'] = child.returncode
        if time.monotonic() >= deadline:
            receipt['status'] = 'failed'; receipt['error'] = 'process exceeded wall deadline'
        receipt['wall_seconds'] = time.monotonic() - started
        for name, raw in output.items():
            path = directory / (name + '.bin')
            path.write_bytes(raw)
            receipt[name] = {'path': str(path), 'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest(),
                             'complete': receipt['status'] == 'passed'}
        (directory / 'receipt.json').write_bytes(canonical(receipt))
    if receipt['status'] != 'passed':
        raise ProcessFailure(receipt)
    return bytes(output['stdout']), receipt
