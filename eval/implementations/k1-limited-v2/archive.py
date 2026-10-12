"""Read bounded arXiv source archives in memory; never extract or execute them."""
from dataclasses import dataclass
import gzip
import hashlib
import io
from pathlib import PurePosixPath
import tarfile


class UnsupportedSource(ValueError):
    """A paper has an explicit unsupported or unsafe source representation."""


@dataclass(frozen=True)
class Limits:
    compressed_bytes: int = 64 * 1024 * 1024
    expanded_bytes: int = 128 * 1024 * 1024
    member_bytes: int = 16 * 1024 * 1024
    members: int = 2048
    text_bytes: int = 24 * 1024 * 1024
    include_depth: int = 32
    expansion_steps: int = 20000
    group_depth: int = 64


def sha256(raw):
    return hashlib.sha256(raw).hexdigest()


def safe_name(name):
    if not isinstance(name, str) or not name or len(name) > 1024 or "\\" in name or "\x00" in name:
        raise UnsupportedSource("invalid archive/include path")
    # arXiv archives commonly use a harmless leading './'. All other dot or
    # parent components are rejected, and the canonical name detects duplicates.
    while name.startswith("./"):
        name = name[2:]
    path = PurePosixPath(name)
    if path.is_absolute() or any(part in ("", ".", "..", ".env", ".secrets") or part.startswith(".env.") for part in name.split("/")):
        raise UnsupportedSource("archive/include path is outside its safe namespace")
    return str(path)


def decode_text(raw):
    if b"\x00" in raw:
        raise UnsupportedSource("source member contains binary NUL bytes")
    try:
        return raw.decode("utf-8-sig"), "utf-8"
    except UnicodeDecodeError:
        return raw.decode("latin-1"), "latin-1"


def read_archive(raw, limits=Limits()):
    if len(raw) > limits.compressed_bytes:
        raise UnsupportedSource("compressed source exceeds its byte bound")
    if raw.startswith(b"\x1f\x8b"):
        try:
            with gzip.GzipFile(fileobj=io.BytesIO(raw)) as stream:
                raw = stream.read(limits.expanded_bytes + 1)
        except (OSError, EOFError) as error:
            raise UnsupportedSource("invalid gzip source") from error
    if len(raw) > limits.expanded_bytes:
        raise UnsupportedSource("expanded source exceeds its byte bound")
    if raw.lstrip().startswith(b"%PDF-"):
        raise UnsupportedSource("deposited source is a PDF; TeX source is unavailable")
    try:
        archive = tarfile.open(fileobj=io.BytesIO(raw), mode="r:")
    except tarfile.ReadError:
        if len(raw) > limits.member_bytes:
            raise UnsupportedSource("single source exceeds its byte bound") from None
        text, encoding = decode_text(raw)
        if "\\documentclass" not in text and "\\documentstyle" not in text and "\\begin{document}" not in text:
            raise UnsupportedSource("source is neither a tar archive nor a LaTeX document") from None
        return {"main.tex": text}, [{"path": "main.tex", "sha256": sha256(raw), "bytes": len(raw), "encoding": encoding}]
    files, evidence, seen, total = {}, [], set(), 0
    try:
        for number, member in enumerate(archive, 1):
            if number > limits.members:
                raise UnsupportedSource("archive member count exceeds its bound")
            if member.isdir() and member.name in (".", "./"):
                continue
            name = safe_name(member.name.rstrip("/") if member.isdir() else member.name)
            if name in seen:
                raise UnsupportedSource("archive repeats a canonical member path")
            seen.add(name)
            if member.isdir():
                continue
            if not member.isfile() or member.issym() or member.islnk() or member.sparse:
                raise UnsupportedSource("archive contains a link, sparse file or special member")
            if member.size < 0 or member.size > limits.member_bytes:
                raise UnsupportedSource("archive member exceeds its byte bound")
            total += member.size
            if total > limits.expanded_bytes:
                raise UnsupportedSource("archive members exceed the aggregate byte bound")
            stream = archive.extractfile(member)
            if stream is None:
                raise UnsupportedSource("archive member cannot be read")
            with stream:
                data = stream.read(limits.member_bytes + 1)
            if len(data) != member.size:
                raise UnsupportedSource("archive member has an inconsistent size")
            encoding = None
            if PurePosixPath(name).suffix.lower() in (".tex", ".bbl", ".bib", ".aux", ".sty", ".cls", ".ltx"):
                text, encoding = decode_text(data)
                files[name] = text
            evidence.append({"path": name, "sha256": sha256(data), "bytes": len(data), "encoding": encoding})
    except (tarfile.TarError, OSError, EOFError) as error:
        raise UnsupportedSource("invalid tar source") from error
    finally:
        archive.close()
    if sum(item["bytes"] for item in evidence if item["encoding"] is not None) > limits.text_bytes:
        raise UnsupportedSource("source text exceeds its aggregate byte bound")
    if not files:
        raise UnsupportedSource("archive contains no supported text members")
    return files, sorted(evidence, key=lambda item: item["path"])
