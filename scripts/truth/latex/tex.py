"""Small, deliberately non-executing TeX reader with explicit unsupported coverage."""
from collections import Counter
from dataclasses import dataclass
from pathlib import PurePosixPath
import re
import unicodedata

from archive import Limits, UnsupportedSource, safe_name

COMMAND = re.compile(r"\\([A-Za-z@]+\*?|[^\n])")


def comments(text):
    # Preserve positions and newlines. An escaped percent is authored text.
    output = list(text)
    at = 0
    while at < len(text):
        if text[at] == "\\":
            command = COMMAND.match(text, at)
            at = command.end() if command else at + 1
        elif text[at] == "%":
            end = text.find("\n", at)
            end = len(text) if end < 0 else end
            output[at:end] = " " * (end - at)
            at = end
        else:
            at += 1
    return "".join(output)


def skip_space(text, at):
    while at < len(text) and text[at].isspace():
        at += 1
    return at


def definition_regions(text):
    """Keep definitions inert when scanning commands that occur in the document."""
    regions, end = [], 0
    for match in COMMAND.finditer(text):
        if match.start() < end:
            continue
        name = match[1].rstrip("*")
        if name in ("newcommand", "renewcommand", "providecommand", "newenvironment", "renewenvironment"):
            pos = skip_space(text, match.end())
            if text[pos:pos + 1] == "{":
                _, pos = group(text, pos)
            else:
                command = COMMAND.match(text, pos)
                if not command:
                    raise UnsupportedSource("invalid macro definition name")
                pos = command.end()
            _, pos = group(text, pos, "[", "]", False)
            _, pos = group(text, pos, "[", "]", False)
            _, end = group(text, pos)
            if name.endswith("environment"):
                _, end = group(text, end)
            regions.append((match.start(), end))
        elif name in ("def", "gdef", "edef", "xdef"):
            pos = text.find("{", match.end())
            if pos < 0 or pos - match.end() > 256:
                raise UnsupportedSource("unsupported macro definition syntax")
            _, end = group(text, pos)
            regions.append((match.start(), end))
    return regions


def mask_regions(text, regions):
    value = list(text)
    for start, end in regions:
        value[start:end] = ["\n" if char == "\n" else " " for char in text[start:end]]
    return "".join(value)


def group(text, at, opening="{", closing="}", required=True, limit=64):
    at = skip_space(text, at)
    if at >= len(text) or text[at] != opening:
        if required:
            raise UnsupportedSource("expected a balanced TeX argument")
        return None, at
    start, depth = at + 1, 1
    at += 1
    while at < len(text):
        if text[at] == "\\":
            command = COMMAND.match(text, at)
            at = command.end() if command else at + 1
            continue
        if text[at] == opening:
            depth += 1
            if depth > limit:
                raise UnsupportedSource("TeX argument nesting exceeds its bound")
        elif text[at] == closing:
            depth -= 1
            if depth == 0:
                return text[start:at], at + 1
        at += 1
    raise UnsupportedSource("unterminated TeX argument")


def token_argument(text, at):
    """TeX formatting takes one token when its argument is not braced."""
    at = skip_space(text, at)
    if at >= len(text):
        raise UnsupportedSource("missing formatting token argument")
    if text[at] == "{":
        return group(text, at)
    match = COMMAND.match(text, at)
    return (match[0], match.end()) if match else (text[at], at + 1)


@dataclass
class Expanded:
    text: str
    main: str
    pieces: list
    files: dict
    coverage: dict

    def origins(self, start, end):
        return [{"path": row["path"], "start": row["source_start"] + max(start - row["start"], 0),
                 "end": row["source_start"] + min(end, row["end"]) - row["start"]}
                for row in self.pieces if row["start"] < end and start < row["end"]]


def main_candidates(files):
    return sorted(name for name, value in files.items() if PurePosixPath(name).suffix.lower() in (".tex", ".ltx")
                  and re.search(r"\\(?:documentclass|documentstyle)\b|\\begin\s*\{document\}", mask_regions(comments(value), definition_regions(comments(value)))))


def local_style_dependencies(files, main, text):
    """Find used deposited packages/classes without interpreting their code."""
    output = set()
    scan = mask_regions(comments(text), definition_regions(comments(text)))
    for command in COMMAND.finditer(scan):
        if command[1] not in {"usepackage", "RequirePackage", "documentclass", "documentstyle", "LoadClass"}:
            continue
        _, pos = group(scan, command.end(), "[", "]", False)
        names, _ = group(scan, pos)
        suffix = ".sty" if command[1] in {"usepackage", "RequirePackage"} else ".cls"
        for name in names.split(","):
            name = safe_name(name.strip())
            if "\\" in name or any(char in name for char in "{}#~"):
                raise UnsupportedSource("dynamic package/class name is unsupported")
            name += suffix
            output.update(path for path in {str(PurePosixPath(main).parent / name), name} if path in files)
    return sorted(output)


def expand_project(files, limits=Limits(), selected_main=None):
    cleaned = {name: comments(value) for name, value in files.items()}
    candidates = main_candidates(files)
    if selected_main is not None and selected_main not in candidates:
        raise UnsupportedSource("selected main is not a deposited document candidate")
    if selected_main is None and len(candidates) != 1:
        raise UnsupportedSource("main file is ambiguous or absent: " + ", ".join(candidates[:20]))
    main = selected_main or candidates[0]
    output, pieces, length, byte_length, steps = [], [], 0, 0, 0
    visited, bibliography_files, ignored = set(), set(), Counter()

    def append(name, start, end):
        nonlocal length, byte_length
        value = cleaned[name][start:end]
        byte_length += len(value.encode("utf-8"))
        if byte_length > limits.text_bytes:
            raise UnsupportedSource("expanded TeX exceeds its byte bound")
        pieces.append({"path": name, "source_start": start, "start": length, "end": length + len(value)})
        output.append(value)
        length += len(value)

    def resolve(parent, value, suffix):
        if "\\" in value or any(char in value for char in "{}#~"):
            raise UnsupportedSource("dynamic include path is unsupported")
        value = safe_name(value.strip())
        if not PurePosixPath(value).suffix:
            value += suffix
        names = {safe_name(str(PurePosixPath(parent).parent / value)), value}
        present = sorted(name for name in names if name in files)
        if len(present) != 1:
            raise UnsupportedSource("include is missing or ambiguous: " + value)
        return present[0]

    def visit(name, stack):
        nonlocal steps
        if name in stack or len(stack) >= limits.include_depth:
            raise UnsupportedSource("include cycle or recursion bound reached")
        visited.add(name)
        text = cleaned[name]
        scan = mask_regions(text, definition_regions(text))
        at = 0
        for command in COMMAND.finditer(scan):
            if command.start() < at:
                continue
            kind = command[1]
            if kind not in ("input", "include", "bibliography", "includeonly"):
                continue
            steps += 1
            if steps > limits.expansion_steps:
                raise UnsupportedSource("include count exceeds its bound")
            if kind == "includeonly":
                raise UnsupportedSource("includeonly changes source visibility and is unsupported")
            if kind in ("input", "include"):
                pos = skip_space(text, command.end())
                if pos < len(text) and text[pos] == "{":
                    value, end = group(text, pos)
                else:
                    bare = re.match(r"[^\s{}%]+", text[pos:])
                    if not bare:
                        raise UnsupportedSource("empty include path")
                    value, end = bare[0], pos + bare.end()
                child = resolve(name, value, ".tex")
            else:
                databases, end = group(text, command.end())
                for database in databases.split(","):
                    try:
                        bibliography_files.add(resolve(name, database, ".bib"))
                    except UnsupportedSource:
                        ignored["bibliography_database_missing_or_ambiguous"] += 1
                local = str(PurePosixPath(main).with_suffix(".bbl"))
                available = [local] if local in files else sorted(path for path in files if path.endswith(".bbl"))
                if len(available) != 1:
                    raise UnsupportedSource("bibliography has no unique deposited .bbl")
                child = available[0]
            append(name, at, command.start())
            visit(child, stack + [name])
            at = end
        append(name, at, len(text))

    visit(main, [])
    return Expanded("".join(output), main, pieces, files,
                    {"main_selection": "explicit root requiring independent PDF evidence" if selected_main else "unique document root", "main_candidates": candidates,
                     "expanded_files": sorted(visited), "bibliography_files": sorted(bibliography_files), "ignored": dict(ignored)})


ACCENTS = {"'": "\u0301", '`': "\u0300", '^': "\u0302", '"': "\u0308", '~': "\u0303", '=': "\u0304", '.': "\u0307", 'c': "\u0327", 'v': "\u030c", 'u': "\u0306", 'H': "\u030b"}
SYMBOLS = {"alpha": "α", "beta": "β", "gamma": "γ", "delta": "δ", "epsilon": "ε", "theta": "θ", "lambda": "λ", "mu": "μ", "pi": "π", "sigma": "σ", "phi": "φ", "psi": "ψ", "omega": "ω", "Gamma": "Γ", "Delta": "Δ", "Theta": "Θ", "Lambda": "Λ", "Pi": "Π", "Sigma": "Σ", "Phi": "Φ", "Psi": "Ψ", "Omega": "Ω", "times": "×", "cdot": "·", "leq": "≤", "geq": "≥", "neq": "≠", "infty": "∞", "ldots": "…", "dots": "…", "LaTeX": "LaTeX", "TeX": "TeX", "&": "&", "%": "%", "_": "_", "#": "#", "$": "$", "{": "{", "}": "}", "textendash": "–", "textemdash": "—", "textasciitilde": "~", "textbackslash": "\\", "ss": "ß", "ae": "æ", "oe": "œ", "o": "ø", "l": "ł"}
FORMATTING = {"textbf", "textit", "texttt", "textrm", "textsf", "textsc", "emph", "mbox", "hbox", "text", "mathrm", "mathbf", "mathit", "mathbb", "mathcal", "mathsf", "operatorname", "ensuremath", "url", "path", "nolinkurl", "enquote", "MakeUppercase", "MakeLowercase", "bibnamefont", "bibfnamefont", "citenamefont"}
SILENT = {"bf", "it", "em", "rm", "sc", "sf", "tt", "bfseries", "itshape", "scshape", "normalfont", "newblock", "protect", "relax", "noindent", "leavevmode", "small", "footnotesize", "scriptsize", "displaystyle", "textstyle", "left", "right", "centering", "hfill", "vfill", "unskip", "ignorespaces", "allowbreak", "penalty", "nobreak", "quad", "qquad", ",", ";", ":", "!", " ", "/", "\\", "bgroup", "egroup"}
DROP_ARGUMENT = {"label", "tag", "tag*", "index", "vspace", "hspace", "vskip", "hskip", "bibliographystyle", "setlength", "addcontentsline"}


class Renderer:
    def __init__(self, text="", limits=Limits()):
        self.limits = limits
        self.macros = {}
        self.unsupported = Counter()
        self.definitions = []
        self.definition_counts = Counter()
        self.budget = [limits.expansion_steps, limits.text_bytes]
        self.math_seen = 0
        for match in COMMAND.finditer(text):
            if match[1].rstrip("*") not in ("newcommand", "renewcommand", "providecommand"):
                continue
            try:
                pos = skip_space(text, match.end())
                if text[pos:pos + 1] == "{":
                    name, pos = group(text, pos)
                else:
                    command = COMMAND.match(text, pos)
                    if not command:
                        continue
                    name, pos = command[0], command.end()
                if not re.fullmatch(r"\\[A-Za-z@]+", name):
                    continue
                self.definition_counts[name[1:]] += 1
                count, pos = group(text, pos, "[", "]", False)
                default, pos = group(text, pos, "[", "]", False)
                body, end = group(text, pos)
                arguments = int(count) if count is not None and count.isdigit() else 0
                if arguments > 3 or default is not None or len(body) > 16384:
                    self.unsupported["macro_definition:" + name] += 1
                    continue
                self.macros[name[1:]] = (arguments, body)
                self.definitions.append((match.start(), end))
            except (UnsupportedSource, ValueError):
                self.unsupported["macro_definition"] += 1

    def plain(self, text, depth=0, budget=None):
        if depth > 24:
            raise UnsupportedSource("macro expansion recursion exceeds its bound")
        if budget is None:
            budget = self.budget
        output, at = [], 0
        text = comments(text)
        text = mask_regions(text, definition_regions(text))
        while at < len(text):
            if text[at] != "\\":
                char = text[at]
                self.math_seen += char == "$"
                output.append(" " if char in "~&" else "" if char in "{}$" else char)
                at += 1
                continue
            match = COMMAND.match(text, at)
            if not match:
                at += 1
                continue
            name, at = match[1], match.end()
            budget[0] -= 1
            if budget[0] < 0:
                raise UnsupportedSource("macro expansion count exceeds its bound")
            if name in ACCENTS:
                pos = skip_space(text, at)
                if text[pos:pos + 1] == "{":
                    value, at = group(text, pos)
                else:
                    value, at = text[pos:pos + 1], pos + 1
                value = self.plain(value, depth + 1, budget)
                output.append(unicodedata.normalize("NFC", value + ACCENTS[name]))
            elif name in self.macros:
                count, value = self.macros[name]
                arguments = []
                for _ in range(count):
                    argument, at = token_argument(text, at)
                    arguments.append(argument)
                for number, argument in reversed(list(enumerate(arguments, 1))):
                    value = value.replace("#" + str(number), argument)
                output.append(self.plain(value, depth + 1, budget))
            elif name in SYMBOLS:
                output.append(SYMBOLS[name])
            elif name in FORMATTING:
                self.math_seen += name == "ensuremath"
                value, at = token_argument(text, at)
                output.append(self.plain(value, depth + 1, budget))
            elif name in ("href", "bibinfo", "bibfield"):
                _, at = group(text, at)
                value, at = group(text, at)
                output.append(self.plain(value, depth + 1, budget))
            elif name in ("begin", "end"):
                environment, at = group(text, at)
                self.math_seen += environment.rstrip("*") in {"equation", "align", "aligned", "gather", "multline", "eqnarray", "split"}
            elif name in DROP_ARGUMENT:
                _, at = group(text, at, required=False)
            elif name in SILENT:
                output.append(" " if name in ("newblock", "quad", "qquad", "\\") else "")
            elif name in ("newcommand", "renewcommand", "providecommand", "def", "write", "write18", "special", "input", "include"):
                self.unsupported[name] += 1
                raise UnsupportedSource("unsupported executable/definition command in rendered content: " + name)
            else:
                self.unsupported[name] += 1
                # Keep literal arguments; unknown commands never run. The caller
                # records this loss of presentation knowledge in coverage.
            if sum(map(len, output[-8:])) > self.limits.text_bytes:
                raise UnsupportedSource("rendered TeX exceeds its byte bound")
        value = "".join(output).replace("---", "—").replace("--", "–").replace("``", '"').replace("''", '"')
        budget[1] -= len(value.encode("utf-8"))
        if budget[1] < 0:
            raise UnsupportedSource("rendered expansion exceeds its aggregate byte bound")
        return re.sub(r"\s+", " ", value).strip()
