"""Bounded byte-preserving source materialization; never run TeX here."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile

from policy import Refused, binding, regular_path, safe_relative

COMPRESSED_BYTES = 64 * 1024 ** 2
EXPANDED_BYTES = 128 * 1024 ** 2
MEMBER_BYTES = 16 * 1024 ** 2
MEMBER_COUNT = 2048


def source_members(raw):
    if len(raw) > COMPRESSED_BYTES:
        raise Refused("compressed source exceeds its bound")
    if raw.startswith(b"\x1f\x8b"):
        try:
            with gzip.GzipFile(fileobj=io.BytesIO(raw)) as stream:
                raw = stream.read(EXPANDED_BYTES + 1)
        except (OSError, EOFError) as error:
            raise Refused("invalid compressed source") from error
    if len(raw) > EXPANDED_BYTES:
        raise Refused("expanded source exceeds its bound")
    if raw.lstrip().startswith(b"%PDF-"):
        raise Refused("deposited source is a PDF")
    try:
        archive = tarfile.open(fileobj=io.BytesIO(raw), mode="r:")
    except tarfile.ReadError:
        if len(raw) > MEMBER_BYTES or b"\x00" in raw or not re.search(rb"\\(?:documentclass|documentstyle)\b", raw):
            raise Refused("unsupported single-file source") from None
        return {"main.tex": raw}
    files, seen, total = {}, set(), 0
    try:
        with archive:
            for number, member in enumerate(archive, 1):
                if number > MEMBER_COUNT:
                    raise Refused("source member count exceeds its bound")
                name = member.name
                while name.startswith("./"):
                    name = name[2:]
                if member.isdir() and not name:
                    continue
                if member.isdir():
                    name = name.rstrip("/")
                safe_relative(name)
                if name in seen:
                    raise Refused("repeated canonical source member")
                seen.add(name)
                if member.isdir():
                    continue
                if not member.isfile() or member.sparse:
                    raise Refused("source links, devices and sparse files are unsupported")
                if not 0 <= member.size <= MEMBER_BYTES:
                    raise Refused("source member exceeds its bound")
                total += member.size
                if total > EXPANDED_BYTES:
                    raise Refused("aggregate source members exceed their bound")
                stream = archive.extractfile(member)
                if stream is None:
                    raise Refused("unreadable source member")
                with stream:
                    data = stream.read(MEMBER_BYTES + 1)
                if len(data) != member.size:
                    raise Refused("truncated source member")
                files[name] = data
    except (tarfile.TarError, OSError, EOFError) as error:
        raise Refused("invalid source archive") from error
    if not files:
        raise Refused("empty source archive")
    for name in files:
        if any(str(parent) in files for parent in Path(name).parents if str(parent) != "."):
            raise Refused("source file is also another member's parent")
    return files


def uncomment(raw):
    """Mask ordinary comments for root discovery, preserving source bytes."""
    result = bytearray(raw)
    escaped, comment = False, False
    for index, value in enumerate(raw):
        if value in (10, 13):
            comment, escaped = False, False
        elif comment:
            result[index] = 32
        elif value == 37 and not escaped:
            comment = True
            result[index] = 32
        else:
            escaped = not escaped if value == 92 else False
    return bytes(result)


def main_source(files):
    candidates = [name for name, raw in files.items() if name.endswith(".tex") and
                  re.search(rb"\\(?:documentclass|documentstyle)\s*(?:\[|\{)", uncomment(raw))]
    if len(candidates) != 1:
        raise Refused("source does not have exactly one declared main TeX root")
    return candidates[0]


def materialize(source_path, expected_sha256, destination):
    source_path = regular_path(source_path)
    if source_path.stat().st_size > COMPRESSED_BYTES:
        raise Refused("compressed source exceeds its bound")
    raw = source_path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected_sha256:
        raise Refused("source identity mismatch")
    files = source_members(raw)
    main = main_source(files)
    destination = Path(destination).absolute()
    regular_path(destination.parent, directory=True)
    destination.mkdir(mode=0o700, exist_ok=False)
    evidence = []
    for name, data in sorted(files.items()):
        path = destination / name
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        with path.open("xb") as stream:
            stream.write(data)
        path.chmod(0o400)
        evidence.append({"name": name, **binding(path)})
    if source_path.read_bytes() != raw:
        raise Refused("original source changed during materialization")
    return {"original": binding(source_path), "main": main, "members": evidence,
            "source_bytes_changed": False, "deposited_source_executed": False}


def select_ten(prior):
    """Balanced deterministic prefix of the pre-outcome availability-only pool."""
    selected = prior["selected"]
    first = [paper for paper in selected if paper["category_rank"] == 1]
    second = sorted((paper for paper in selected if paper["category_rank"] == 2), key=lambda p: p["input_position"])
    if len(first) != 7 or len({p["stratum"][0] for p in first}) != 7 or len(second) != 7:
        raise Refused("unexpected prior availability pool")
    result = sorted(first + second[:3], key=lambda p: p["input_position"])
    if len({(p["arxiv_id"], p["version"]) for p in result}) != 10:
        raise Refused("selection repeats a paper")
    if any(p["status"] != "selected" or not 0 < p["page_count"] <= 8 for p in result):
        raise Refused("selection contains an unavailable or oversized original")
    # Copy through JSON so subsequent caller annotations cannot mutate the pool.
    return json.loads(json.dumps(result))
