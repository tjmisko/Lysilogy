#!/usr/bin/env python3
"""Generate reproducible, small PDFs for the isolated library scale benchmark."""
import argparse
from contextlib import contextmanager
import errno
import hashlib
import json
import os
from pathlib import Path
import stat
import uuid

SCHEMA_VERSION = 1
DEFAULT_COUNT = 10_000
DEFAULT_SEED = 19
TOPICS = ("Causal inference", "Robotics", "Graph learning", "Probability",
          "Economic history", "Optimization", "Language models", "Vision",
          "Networks", "Statistical physics", "Decision theory", "Information")
AUTHORS = ("Ada Researcher", "B. Analyst", "Chen, Li", "De la Cruz, Ana",
           "Muller, Max", "Okafor, Nia", "Singh, Ravi", "Yamada, Ren")


def encoded(value):
    """PDF metadata strings use explicit Unicode, independent of locale."""
    return "<" + (b"\xfe\xff" + value.encode("utf-16-be")).hex() + ">"


def literal(value):
    return value.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")


def specimen(seed, index):
    entropy = hashlib.sha256(f"lysilogy-bench-v1:{seed}:{index}".encode()).digest()
    topic = TOPICS[entropy[0] % len(TOPICS)]
    author = AUTHORS[entropy[1] % len(AUTHORS)]
    year = 2000 + entropy[2] % 26
    title = f"{topic} study {index:05d}"
    filename = f"{author} - {year} - {title}.pdf"
    nesting = entropy[3] % 4
    directories = [f"collection-{entropy[4] % 12:02d}", str(year), "selected"][:nesting]
    relative = str(Path(*directories, filename))
    pages = 1 + entropy[5] % 3
    objects = ["<< /Type /Catalog /Pages 2 0 R >>", "",
               "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    children = []
    for page in range(pages):
        page_number = len(objects) + 1
        children.append(f"{page_number} 0 R")
        lines = [title, f"{author}, {year}", f"Synthetic benchmark page {page + 1}."]
        lines.extend(f"Observation {line}: sample {index}, seed {seed}, token {entropy[line % 32]}."
                     for line in range(1, 25))
        content = "BT /F1 12 Tf 48 750 Td " + " 0 -24 Td ".join(
            f"({literal(line)}) Tj" for line in lines) + " ET"
        objects.append("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
                       f"/Resources << /Font << /F1 3 0 R >> >> /Contents {page_number + 1} 0 R >>")
        objects.append(f"<< /Length {len(content.encode())} >>\nstream\n{content}\nendstream")
    objects[1] = f"<< /Type /Pages /Kids [{' '.join(children)}] /Count {pages} >>"
    objects.append(f"<< /Title {encoded(title)} /Author {encoded(author)} "
                   f"/Subject {encoded(topic)} /CreationDate (D:{year}0101000000Z) >>")
    pdf = bytearray(b"%PDF-1.4\n")
    offsets = []
    for number, item in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{number} 0 obj\n{item}\nendobj\n".encode())
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets:
        pdf.extend(f"{offset:010d} 00000 n \n".encode())
    pdf.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R /Info {len(objects)} 0 R >>"
               f"\nstartxref\n{xref}\n%%EOF\n".encode())
    return relative, bytes(pdf), dict(title=title, authors=[author], year=year, pages=pages)


def payload(value):
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


@contextmanager
def directory_fd(path, create=False):
    """Pin each no-follow directory before opening its child; never resolve a raceable full path."""
    path = Path(path).absolute()
    if ".." in path.parts:
        raise ValueError("benchmark storage cannot contain parent traversal")
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for name in path.parts[1:]:
            try:
                child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            except FileNotFoundError:
                if not create:
                    raise
                try:
                    os.mkdir(name, mode=0o700, dir_fd=descriptor)
                except FileExistsError:
                    pass
                child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        yield descriptor
    except OSError as error:
        if error.errno in (errno.ELOOP, errno.ENOTDIR):
            raise ValueError("benchmark storage must not traverse symlinks or non-directories") from error
        raise
    finally:
        os.close(descriptor)


def ensure_directory(path):
    with directory_fd(path, create=True):
        pass


def read_at(descriptor, name, limit):
    try:
        file = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=descriptor)
    except OSError as error:
        if error.errno == errno.ELOOP:
            raise ValueError("benchmark file must not be a symlink") from error
        raise
    with os.fdopen(file, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError("benchmark file must be regular")
        content = stream.read(limit + 1)
        if len(content) > limit:
            raise ValueError("benchmark file exceeds expected bound")
        return content


def read_regular(path, limit=8 * 1024 * 1024):
    with directory_fd(path.parent) as descriptor:
        return read_at(descriptor, path.name, limit)


def retain_file(path, data):
    """Resume exact generated files; never overwrite an existing different file."""
    write_owned(path, data, replace=False)


def write_owned(path, data, replace=False):
    with directory_fd(path.parent, create=True) as descriptor:
        try:
            current = read_at(descriptor, path.name, max(len(data), 8 * 1024 * 1024) if replace else len(data))
        except FileNotFoundError:
            current = None
        if current is not None and not replace:
            if current != data:
                raise ValueError(f"existing benchmark output differs: {path.name}")
            return
        temporary = f".bench-{uuid.uuid4().hex}"
        file = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                       0o600, dir_fd=descriptor)
        try:
            with os.fdopen(file, "wb") as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            if replace:
                os.replace(temporary, path.name, src_dir_fd=descriptor, dst_dir_fd=descriptor)
            else:
                try:
                    os.link(temporary, path.name, src_dir_fd=descriptor, dst_dir_fd=descriptor, follow_symlinks=False)
                except FileExistsError:
                    if read_at(descriptor, path.name, len(data)) != data:
                        raise ValueError("concurrent benchmark output differs") from None
            os.fsync(descriptor)
        finally:
            try:
                os.unlink(temporary, dir_fd=descriptor)
            except FileNotFoundError:
                pass


def generate(vault, count=DEFAULT_COUNT, seed=DEFAULT_SEED):
    """Internal fixture generator. The command validates its designated storage root."""
    if not 1 <= count <= DEFAULT_COUNT:
        raise ValueError("paper count must be between 1 and 10000")
    vault = Path(vault).absolute()
    marker = vault / "generation.json"
    config = dict(schema_version=SCHEMA_VERSION, count=count, seed=seed)
    try:
        with directory_fd(vault):
            try:
                read_regular(marker)
            except FileNotFoundError:
                raise ValueError("existing directory is not an owned benchmark vault") from None
    except FileNotFoundError:
        pass
    ensure_directory(vault)
    retain_file(marker, payload(config))
    records = []
    for index in range(count):
        relative, pdf, metadata = specimen(seed, index)
        retain_file(vault / "papers" / relative, pdf)
        records.append(dict(path=relative, sha256=hashlib.sha256(pdf).hexdigest(),
                            bytes=len(pdf), metadata=metadata))
    manifest = dict(**config, papers=records)
    retain_file(vault / "manifest.json", payload(manifest))
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    designated = Path.home() / ".cache/lysilogy/bench-vault"
    parser.add_argument("--root", type=Path, default=designated,
                        help="designated bench-vault directory or a subdirectory below it")
    parser.add_argument("--count", type=int, default=DEFAULT_COUNT)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    args = parser.parse_args()
    vault = args.root.expanduser().absolute()
    if not vault.is_relative_to(designated) or ".." in vault.parts:
        parser.error("generated PDFs must stay under ~/.cache/lysilogy/bench-vault/")
    manifest = generate(vault, args.count, args.seed)
    print(json.dumps(dict(vault=str(vault), papers=len(manifest["papers"]),
                          bytes=sum(item["bytes"] for item in manifest["papers"]))))


if __name__ == "__main__":
    main()
