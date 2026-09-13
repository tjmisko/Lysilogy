#!/usr/bin/env python3
"""Generate reproducible, small PDFs for the isolated library scale benchmark."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

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


def ensure_directory(path):
    # Reject symlinks at every component before creating anything below them.
    for component in [*reversed(path.parents), path]:
        if component.is_symlink():
            raise ValueError("benchmark storage must not traverse symlinks")
        if component.exists() and not component.is_dir():
            raise ValueError("benchmark directory component is not a directory")
    path.mkdir(parents=True, exist_ok=True)


def retain_file(path, data):
    """Resume exact generated files; never overwrite an existing different file."""
    if path.is_symlink():
        raise ValueError("benchmark output must not be a symlink")
    if path.exists():
        if not path.is_file() or path.read_bytes() != data:
            raise ValueError(f"existing benchmark output differs: {path.name}")
        return
    ensure_directory(path.parent)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(prefix=".bench-", dir=path.parent, delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        # Hard-link publication is atomic and fails if another writer won.
        try:
            os.link(temporary, path)
        except FileExistsError:
            if path.is_symlink() or not path.is_file() or path.read_bytes() != data:
                raise ValueError("concurrent benchmark output differs") from None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def generate(vault, count=DEFAULT_COUNT, seed=DEFAULT_SEED):
    """Internal fixture generator. The command validates its designated storage root."""
    if not 1 <= count <= DEFAULT_COUNT:
        raise ValueError("paper count must be between 1 and 10000")
    vault = Path(vault).absolute()
    marker = vault / "generation.json"
    config = dict(schema_version=SCHEMA_VERSION, count=count, seed=seed)
    if vault.exists() and not marker.is_file():
        raise ValueError("existing directory is not an owned benchmark vault")
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
