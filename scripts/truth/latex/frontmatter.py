#!/usr/bin/env python3
"""Title, ordered authors and abstract from deposited TeX: the identity oracle.

Evaluation only, like the rest of this harness: sources are read in memory and
never executed. A field the reader cannot establish is withheld with a reason
instead of guessed, so the extraction benchmark never scores against a wrong
answer. Math is removed from the abstract (the PDF prints it as glyphs the
source spelling cannot match); citation commands are removed because the
printed marker is not in the source.
"""
import argparse
import json
import re
import sys
import unicodedata
from pathlib import Path

from archive import UnsupportedSource, read_archive
from tex import COMMAND, Renderer, expand_project, group

# Commands whose arguments are never part of a printed title or author name,
# with how many braced arguments each takes. \textcolor keeps its text: only
# the colour argument is dropped.
NOISE = {
    **dict.fromkeys((
        "thanks", "footnote", "footnotemark", "footnotetext", "inst", "textsuperscript", "affiliation",
        "affil", "email", "orcid", "orcidlink", "cite", "citep", "citet", "ref", "label", "IEEEauthorrefmark",
        "url", "address", "institute", "authornote", "corref", "fnref", "tnoteref", "titlenote",
        "vspace", "hspace", "color", "textcolor", "fontseries", "fontshape", "fontfamily", "usefont",
    ), 1),
    "fontsize": 2,
}
CITATIONS = dict.fromkeys(
    ("cite", "citep", "citet", "citealp", "citealt", "citeauthor", "citeyear", "parencite", "textcite", "autocite"), 1
)
PLACEHOLDER = re.compile(r"\b(?:anonymous|submission|x{3,}|author\s*names?|first\s*name|last\s*name)\b", re.IGNORECASE)
INSTITUTIONAL = re.compile(
    r"\b(?:university|universit[äéy]|institut[e]?|department|dept|laboratory|laboratories|lab|school|college|"
    r"cent(?:er|re)|inc|ltd|corporation|research|google|microsoft|faculty|academy|hospital|street|road)\b",
    re.IGNORECASE,
)
NAME_TOKEN = re.compile(r"[^\W\d_]", re.UNICODE)


def drop_commands(text, arities):
    """Remove each named command with its optional and its braced arguments."""
    output, at = [], 0
    for match in COMMAND.finditer(text):
        if match.start() < at:
            continue
        count = arities.get(match[1].rstrip("*"))
        if count is None:
            continue
        output.append(text[at:match.start()])
        end = match.end()
        _, end = group(text, end, "[", "]", required=False)
        for _ in range(count):
            braced, after = group(text, end, required=False)
            if braced is None:
                break
            end = after
        at = end
    output.append(text[at:])
    return "".join(output)


def drop_math(text):
    text = re.sub(r"\\\[.*?\\\]|\\\(.*?\\\)", " ", text, flags=re.DOTALL)
    text = re.sub(r"\$\$.*?\$\$", " ", text, flags=re.DOTALL)
    return re.sub(r"(?<!\\)\$.*?(?<!\\)\$", " ", text, flags=re.DOTALL)


def first_argument(text, name):
    """Every braced argument of `\\name`, after any optional argument, in order."""
    found = []
    for match in COMMAND.finditer(text):
        if match[1] != name:
            continue
        at = match.end()
        _, at = group(text, at, "[", "]", required=False)
        value, _ = group(text, at, required=False)
        if value is not None:
            found.append(value)
    return found


def texorpdf(text):
    """Keep the TeX branch of \\texorpdfstring{tex}{pdf}."""
    output, at = [], 0
    for match in COMMAND.finditer(text):
        if match.start() < at or match[1] != "texorpdfstring":
            continue
        tex, end = group(text, match.end())
        _, end = group(text, end)
        output.extend([text[at:match.start()], tex])
        at = end
    output.append(text[at:])
    return "".join(output)


def clean(renderer, text):
    text = texorpdf(text)
    text = drop_commands(text, NOISE)
    text = text.replace("\\\\", " ").replace("\\newline", " ")
    return unicodedata.normalize("NFC", renderer.plain(text))


def title_of(text, renderer):
    values = first_argument(text, "title") or first_argument(text, "icmltitle")
    titles = list(dict.fromkeys(clean(renderer, value) for value in values))
    titles = [title for title in titles if title]
    if not titles:
        return None, "no title command"
    if len(titles) > 1:
        return None, "conflicting title commands"
    return titles[0], None


def split_names(chunk):
    """A chunk is one name, or a comma/'and' list of names."""
    parts = re.split(r"\s*,\s*(?:and\s+)?|\s+and\s+|\s*&\s*", chunk)
    return [part for part in (name_text(part) for part in parts) if part]


def name_text(value):
    value = re.sub(r"[\*†‡§¶#∗♮♯⋆]+", " ", value)
    value = re.sub(r"\s+", " ", value).strip(" ,;:.")
    # Superscript affiliation markers survive as bare digits or letters after math removal.
    return re.sub(r"(?:\s+\d+)+$", "", value).strip()


def valid_name(name):
    tokens = name.split()
    return (
        1 <= len(tokens) <= 6
        and all(NAME_TOKEN.search(token) for token in tokens)
        and not re.search(r"[\d@/\\{}]", name)
        and not INSTITUTIONAL.search(name)
        and not PLACEHOLDER.search(name)
    )


def author_chunks(text):
    """Raw author chunks in printed order, from the first convention present."""
    icml = first_argument(text, "icmlauthor")
    if icml:
        return icml
    ieee = first_argument(text, "IEEEauthorblockN")
    if ieee:
        return ieee
    chunks = []
    for value in first_argument(text, "author"):
        for part in re.split(r"\\(?:and|AND)\b", value):
            # Lines after a forced break are affiliations or addresses.
            chunks.append(re.split(r"\\\\|\\newline\b", part, maxsplit=1)[0])
    return chunks


def authors_of(text, renderer):
    chunks = author_chunks(text)
    if not chunks:
        return None, "no author command"
    names = []
    for chunk in chunks:
        rendered = clean(renderer, drop_math(chunk))
        names.extend(split_names(rendered))
    if not names:
        return None, "author commands render empty"
    rejected = [name for name in names if not valid_name(name)]
    if rejected:
        return None, "unrecognized author text: " + "; ".join(rejected[:3])
    return names, None


def abstract_of(text, renderer):
    match = re.search(r"\\begin\s*\{abstract\}(.*?)\\end\s*\{abstract\}", text, re.DOTALL)
    if match:
        raw = match[1]
    else:
        values = first_argument(text, "abstract")
        if not values:
            return None, "no abstract environment or command"
        raw = values[0]
    raw = drop_commands(drop_math(raw), {**CITATIONS, "label": 1, "footnote": 1, "thanks": 1})
    paragraphs = [clean(renderer, part) for part in re.split(r"\n\s*\n|\\par\b", raw)]
    abstract = "\n\n".join(part for part in paragraphs if part)
    if not abstract:
        return None, "abstract renders empty"
    return abstract, None


def front_matter_renderer(text):
    """A renderer that also drops noise commands produced by macro expansion."""
    renderer = Renderer(text)
    for name, count in NOISE.items():
        for spelling in (name, name + "*"):
            renderer.macros[spelling] = (count, "")
    return renderer


def front_matter(files, main=None):
    """Front matter for one deposited project; withheld fields carry a reason."""
    expanded = expand_project(files, selected_main=main)
    text = expanded.text
    result = {"main": expanded.main, "withheld": {}}
    for field, reader in (("title", title_of), ("authors", authors_of), ("abstract", abstract_of)):
        try:
            value, reason = reader(text, front_matter_renderer(text))
        except UnsupportedSource as error:
            value, reason = None, "unsupported: " + str(error)
        result[field] = value
        if reason:
            result["withheld"][field] = reason
    return result


def front_matter_from_source(path, main=None):
    try:
        files, _ = read_archive(Path(path).read_bytes())
        return {"status": "ok", **front_matter(files, main)}
    except UnsupportedSource as error:
        return {"status": "unsupported", "reason": str(error)}


def main():
    parser = argparse.ArgumentParser(description="Read title, authors and abstract from deposited TeX.")
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--main", help="main .tex member when the root is ambiguous")
    args = parser.parse_args()
    print(json.dumps(front_matter_from_source(args.source, args.main), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
