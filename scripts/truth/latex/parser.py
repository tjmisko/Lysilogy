"""Derive evaluation objects and links from source structure, not PDF detectors."""
from collections import Counter
from bisect import bisect_right
import re

from archive import Limits, UnsupportedSource, sha256
from tex import ACCENTS, COMMAND, DROP_ARGUMENT, FORMATTING, SILENT, SYMBOLS, UNVERIFIED_MATH_LAYOUTS, Renderer, comments, definition_regions, expand_project, group, local_style_dependencies, mask_regions, skip_space, token_argument

STANDARD_STATEMENTS = {name: name for name in ("theorem", "lemma", "corollary", "proposition", "definition", "assumption", "remark", "claim", "conjecture", "example")}
ENVIRONMENTS = {"figure": "figure", "table": "table", "equation": "equation", "align": "equation", "gather": "equation", "multline": "equation", "eqnarray": "equation", "proof": "proof", "algorithm": "algorithm", "algorithm2e": "algorithm", "listing": "algorithm", "lstlisting": "algorithm"}
CITES = {"cite", "citep", "citet", "citealt", "citealp", "parencite", "textcite", "autocite"}
REFS = {"ref", "eqref", "autoref", "cref", "Cref", "vref"}
# These exact slots are metadata, literal input, or deferred/repeated content.
# Structural tokens there do not independently establish a live object/link.
# In particular href's second argument and ordinary formatting arguments are
# visible content and deliberately absent from this registry.
STORED_ARGUMENT_ROLES = {
    **{name: (1,) for name in CITES | REFS | {
        "index", "label", "tag", "bibliographystyle", "url", "path", "nolinkurl", "href",
        "documentclass", "documentstyle", "usepackage", "RequirePackage", "LoadClass", "includegraphics",
        "title", "TITLE", "author", "date", "thanks", "bibitem", "bibinfo", "bibfield", "nocite",
        "addbibresource", "vspace", "hspace", "vskip", "hskip"}},
    "setlength": (1, 2), "addcontentsline": (1, 2, 3), "markboth": (1, 2), "newtheorem": (1, 2),
    "DeclareMathOperator": (1, 2),
    "printbibliography": (),
}
OPTIONAL_STORED_ARGUMENTS = CITES | REFS | {"bibitem", "documentclass", "documentstyle", "usepackage", "RequirePackage", "LoadClass", "includegraphics", "href", "printbibliography", "addbibresource", "newtheorem"}
LITERAL_ARGUMENTS = {"url", "path", "nolinkurl"}
# These standard math symbols and scalar dimensions cannot inject object
# environments. This inventory capability does not imply faithful rendering:
# commands absent from Renderer still withhold their enclosing object's text.
INVENTORY_ONLY_PRIMITIVES = {"in", "notin", "ni", "subset", "subseteq", "supset", "supseteq", "sim", "simeq", "approx", "equiv", "cong", "propto", "perp", "parallel", "forall", "exists", "neg", "land", "lor", "cup", "cap", "emptyset", "pm", "mp", "div", "circ", "tilde", "widetilde", "limits", "nolimits", "textwidth", "linewidth", "columnwidth", "hsize", "vsize", "parindent", "parskip", "baselineskip", "abovecaptionskip", "belowcaptionskip", "tabcolsep", "arraycolsep"}
# Finite standard atoms have no object/entry/link side effect. Their glyph,
# delimiter size, script placement and font distinctions remain unverified by
# Renderer. In particular, these are not transparent rendering wrappers.
INVENTORY_ONLY_PRIMITIVES |= {
    "zeta", "eta", "iota", "kappa", "nu", "xi", "rho", "tau", "upsilon", "Xi", "Upsilon",
    "varepsilon", "vartheta", "varpi", "varrho", "varsigma", "varphi",
    "ell", "hbar", "imath", "jmath", "Re", "Im", "aleph", "wp",
    "rightarrow", "leftarrow", "to", "gets", "leftrightarrow", "Rightarrow", "Leftarrow", "Leftrightarrow",
    "longrightarrow", "longleftarrow", "longleftrightarrow", "Longrightarrow", "Longleftarrow", "Longleftrightarrow",
    "mapsto", "longmapsto", "uparrow", "downarrow", "updownarrow", "Uparrow", "Downarrow", "Updownarrow",
    "le", "ge", "ne", "ll", "gg", "prec", "succ", "preceq", "succeq", "asymp", "doteq", "vdash", "dashv",
    "setminus", "uplus", "sqcap", "sqcup", "oplus", "ominus", "otimes", "oslash", "odot", "bullet", "star",
    "cdots", "vdots", "ddots", "prime", "angle", "triangle", "triangleleft", "triangleright", "top", "bot",
    "langle", "rangle", "lbrace", "rbrace", "lfloor", "rfloor", "lceil", "rceil", "vert", "Vert", "|", "mid",
    "big", "Big", "bigg", "Bigg", "bigl", "bigr", "Bigl", "Bigr", "biggl", "biggr", "Biggl", "Biggr",
    "textsuperscript", "textsubscript",
    "tiny", "normalsize", "large", "Large", "LARGE", "huge", "Huge", "newline",
}
LAYOUT_ENVIRONMENTS = {"document", "abstract", "thebibliography", "itemize", "enumerate", "description", "center", "quote", "quotation", "minipage", "tabular", "tabularx", "tabular*", "array", "split", "aligned", "alignedat", "subequations", "subfigure", "subtable", "algorithmic", "algorithmicx", "algorithmic*", "flushleft", "flushright", "IEEEkeywords", "keywords", "tikzpicture", "picture", "adjustbox", "threeparttable", "tablenotes", "multicols", "spacing", "doublespace", "singlespace", "small", "footnotesize", "landscape"}


def equation_rows(node, text):
    """Only top-level row separators create separately numbered equations."""
    if node["environment"].rstrip("*") not in ("align", "gather", "eqnarray"):
        raw = text[node["content_start"]:node["content_end"]]
        tagged = bool(list(argument_commands(raw, {"tag"})))
        if node["environment"].rstrip("*") in ("equation", "multline"):
            suppressed = node["environment"].endswith("*") or bool(re.search(r"\\(?:nonumber|notag)\b", raw))
            if suppressed and not tagged:
                if list(argument_commands(raw, {"label"})):
                    return [{**node, "numbering_uncertain": True, "explicit_tag": False}]
                return []
        return [{**node, "explicit_tag": tagged}]
    start, depth, rows, at = node["content_start"], 0, [], node["content_start"]
    for match in COMMAND.finditer(text, node["content_start"], node["content_end"]):
        if match.start() < at:
            continue
        if match[1] in ("begin", "end"):
            _, at = group(text, match.end())
            depth += 1 if match[1] == "begin" else -1
        elif match[1] == "\\" and depth == 0:
            rows.append((start, match.start()))
            _, at = group(text, match.end(), "[", "]", False)
            start = at
    rows.append((start, node["content_end"]))
    output = []
    for start, end in rows:
        raw = text[start:end]
        if not raw.strip():
            continue
        tagged = bool(re.search(r"\\tag\*?\s*\{", raw))
        suppressed = bool(re.search(r"\\(?:nonumber|notag)\b", raw)) or node["environment"].endswith("*")
        if suppressed and not tagged:
            if list(argument_commands(raw, {"label"})):
                output.append({**node, "start": start, "content_start": start, "content_end": end, "end": end,
                               "equation_row": True, "numbering_uncertain": True, "explicit_tag": False})
            continue
        output.append({**node, "start": start, "content_start": start, "content_end": end, "end": end,
                       "equation_row": True, "explicit_tag": tagged})
    return output


def argument_commands(text, names):
    at = 0
    for match in COMMAND.finditer(text):
        if match.start() < at or match[1].rstrip("*") not in names:
            continue
        options = []
        at = match.end()
        for _ in range(2):
            option, at = group(text, at, "[", "]", False)
            if option is None:
                break
            options.append(option)
        value, at = group(text, at)
        yield {"command": match[1], "value": value, "options": options, "start": match.start(), "end": at}


def equation_aliases(text, scan):
    """Recognize only global preamble, zero-argument equation boundary aliases.

    No source is expanded or rewritten: event positions stay at the deposited
    invocation. Parameterized, scoped, repeated, indirect and executable bodies
    retain the ordinary unsupported-macro handling.
    """
    documents = [row for row in argument_commands(scan, {"begin"}) if row["value"] == "document"]
    if len(documents) != 1:
        return {}
    definitions, depths = {}, {}
    previous, depth = 0, 0
    for start, end in definition_regions(text):
        # One cumulative scan, rather than rescanning the entire prefix for
        # every definition in a large deposited preamble.
        at = previous
        for match in COMMAND.finditer(scan, previous, start):
            depth += scan[at:match.start()].count("{") - scan[at:match.start()].count("}")
            if match[1] in {"bgroup", "begingroup"}:
                depth += 1
            elif match[1] in {"egroup", "endgroup"}:
                depth -= 1
            at = match.end()
        depth += scan[at:start].count("{") - scan[at:start].count("}")
        depths[start], previous = depth, start
        command = COMMAND.match(text, start)
        kind, pos = command[1].rstrip("*"), skip_space(text, command.end())
        if kind not in {"newcommand", "renewcommand", "providecommand", "def", "gdef", "edef", "xdef"}:
            continue
        if text[pos:pos + 1] == "{":
            name, pos = group(text, pos)
        else:
            name_match = COMMAND.match(text, pos)
            if not name_match:
                continue
            name, pos = name_match[0], name_match.end()
        if not re.fullmatch(r"\\[A-Za-z@]+", name):
            continue
        definitions.setdefault(name[1:], []).append((kind, start, end, pos))
    output = {}
    for name, rows in definitions.items():
        if len(rows) != 1:
            continue
        kind, start, end, pos = rows[0]
        if kind not in {"newcommand", "def"} or end > documents[0]["start"]:
            continue
        # A literal braced group or explicit group command scopes definitions.
        if depths[start] != 0 or text[skip_space(text, pos):skip_space(text, pos) + 1] != "{":
            continue
        body, stop = group(text, pos)
        match = re.fullmatch(r"\s*\\(begin|end)\s*\{(equation|align|gather|multline|eqnarray)(\*?)\}\s*", body)
        if stop != end or not match or name in (CITES | REFS | set(SYMBOLS) | set(FORMATTING) | set(SILENT) | INVENTORY_ONLY_PRIMITIVES | {"begin", "end", "label", "caption", "bibitem"}):
            continue
        output[name] = {"command": match[1], "value": match[2] + match[3], "definition_start": start, "definition_end": end}
    return output


def environment_commands(scan, aliases, limits):
    rows = list(argument_commands(scan, {"begin", "end"}))
    extra = []
    for match in COMMAND.finditer(scan):
        name = match[1].rstrip('*')
        if name in aliases:
            if any(row['start'] <= match.start() < row['end'] for row in rows):
                raise UnsupportedSource('equation alias inside an environment name is unsupported')
            # The shared scanner recognizes starred LaTeX commands, but these
            # zero-argument aliases consume only the control-word token. A
            # following star is ordinary authored input inside/after the math.
            end = match.end() - int(match[1].endswith('*'))
            extra.append({**aliases[name], "alias": name, "start": match.start(), "end": end, "options": []})
            if len(extra) > limits.expansion_steps:
                raise UnsupportedSource('equation alias invocation count exceeds its bound')
    return sorted(rows + extra, key=lambda row: row['start'])


def math_operator_declarations(text, scan, reserved, limits):
    """Prove the inventory effect of a finite AMS preamble declaration.

    amsopn defines a new zero-argument math operator, retaining its literal
    display body; the starred form changes limits placement. Neither form is a
    source-object producer. We do not infer faithful PDF rendering from this
    capability, or execute any deposited command in the display body.
    """
    commands = [match for match in COMMAND.finditer(scan) if match[1].rstrip('*') == 'DeclareMathOperator']
    if len(commands) > limits.expansion_steps:
        raise UnsupportedSource('math operator declaration count exceeds its bound')
    documents = list(argument_commands(scan, {'begin'}))
    document_start = next((row['start'] for row in documents if row['value'] == 'document'), 0)
    ams_loads = [row for row in argument_commands(scan, {'usepackage', 'RequirePackage'})
                 if {name.strip() for name in row['value'].split(',')} & {'amsmath', 'amsopn'}]
    reserved = reserved | set('arccos arcsin arctan arg cos cosh cot coth csc deg det dim exp gcd hom inf injlim ker lg lim liminf limsup ln log max min Pr projlim sec sin sinh sup tan tanh'.split())
    defined = set()
    for start, _ in definition_regions(text):
        match = COMMAND.match(text, start)
        if match[1].rstrip('*') not in {'newcommand', 'renewcommand', 'providecommand', 'def', 'gdef', 'edef', 'xdef'}:
            continue
        pos = skip_space(text, match.end())
        if text[pos:pos + 1] == '{':
            name, _ = group(text, pos)
        else:
            value = COMMAND.match(text, pos)
            name = value[0] if value else ''
        if re.fullmatch(r'\\[A-Za-z@]+', name):
            defined.add(name[1:])
    # Scope is evaluated over the unchanged, definition-masked source once.
    # The declaration's own brace groups are balanced and have zero net depth.
    depth, environment_depth, previous, rows, prior_control = 0, 0, 0, [], False
    for command in commands:
        at = previous
        for match in COMMAND.finditer(scan, previous, command.start()):
            depth += scan[at:match.start()].count('{') - scan[at:match.start()].count('}')
            depth += match[1] in {'bgroup', 'begingroup'}
            depth -= match[1] in {'egroup', 'endgroup'}
            environment_depth += match[1] == 'begin'
            environment_depth -= match[1] == 'end'
            prior_control |= ((match[1].startswith('if') and match[1] != 'iff')
                              or match[1] in {'else', 'fi', 'unless', 'newif', 'csname', 'endcsname', 'let', 'futurelet'})
            at = match.end()
        depth += scan[at:command.start()].count('{') - scan[at:command.start()].count('}')
        previous = command.start()
        row = {'start': command.start(), 'end': command.end(), 'name': None, 'reason': None}
        try:
            name, pos = token_argument(scan, command.end())
            body, end = group(scan, pos, limit=limits.group_depth)
            row.update(end=end, body=body, starred=command[1].endswith('*'))
            if re.fullmatch(r'\\[A-Za-z]+', name):
                row['name'] = name[1:]
            if row['name'] is None:
                row['reason'] = 'operator name is not one ordinary control word'
            elif end > document_start or depth != 0 or environment_depth != 0:
                row['reason'] = 'operator declaration is not a top-level preamble declaration'
            elif prior_control:
                row['reason'] = 'operator declaration follows uninterpreted preamble control flow'
            elif not any(load['end'] <= command.start() for load in ams_loads):
                row['reason'] = 'operator declaration has no preceding explicit AMS package load'
            elif 'DeclareMathOperator' in defined or row['name'] in reserved | defined:
                row['reason'] = 'operator name or declaration primitive has another definition'
            elif not body.strip() or len(body) > 256 or not re.fullmatch(r'(?:[A-Za-z0-9 \t\r\n]|\\[,;! ])+', body):
                row['reason'] = 'operator body is not bounded literal text and standard spacing'
        except UnsupportedSource as error:
            if any(word in str(error) for word in ('bound', 'nesting', 'recursion')):
                raise
            row['reason'] = str(error)
        rows.append(row)
    repeated = Counter(row['name'] for row in rows if row['name'])
    for row in rows:
        if row['name'] and repeated[row['name']] != 1:
            row['reason'] = 'operator name is declared more than once'
    return rows


def render_text(renderer, text):
    """Retain an unrenderable source item without inventing its printed text.

    TeX macros may consume caller tokens beyond their declared arguments. That
    syntax is unsupported by this inert renderer; it does not invalidate the
    structural source span or another independently parsed object. Resource
    limits still abort the paper, including all cumulative expansion limits.
    """
    try:
        return renderer.plain(text)
    except UnsupportedSource as error:
        if any(word in str(error) for word in ("bound", "recursion", "nesting")):
            raise
        renderer.unsupported["unrenderable_text:" + str(error)] += 1
        return ""


def bibtex_fields(files, renderer):
    output, issues = {}, Counter()
    for path, raw in sorted(files.items()):
        if not path.endswith(".bib"):
            continue
        text = comments(raw)
        at = 0
        for match in re.finditer(r"@([A-Za-z]+)\s*([({])", text):
            if match.start() < at:
                continue
            opening = match[2]
            body, at = group(text, match.end() - 1, opening, ")" if opening == "(" else "}")
            if match[1].casefold() in ("comment", "preamble", "string"):
                if match[1].casefold() == "string":
                    issues["bibtex_string_macro"] += 1
                continue
            if "," not in body:
                issues["bibtex_entry_without_fields"] += 1
                continue
            key, rest = body.split(",", 1)
            key = key.strip()
            fields, pos = {}, 0
            while pos < len(rest):
                field = re.match(r"\s*,?\s*([A-Za-z-]+)\s*=\s*", rest[pos:])
                if not field:
                    if rest[pos:].strip(" ,\r\n\t"):
                        issues["bibtex_unsupported_field"] += 1
                    break
                name = field[1].lower()
                pos += field.end()
                if rest[pos:pos + 1] == "{":
                    value, pos = group(rest, pos)
                elif rest[pos:pos + 1] == '"':
                    end = pos + 1
                    while end < len(rest):
                        if rest[end] == '"' and rest[end - 1] != "\\":
                            break
                        end += 1
                    if end == len(rest):
                        raise UnsupportedSource("unterminated quoted BibTeX field")
                    value, pos = rest[pos + 1:end], end + 1
                else:
                    bare = re.match(r"\d+", rest[pos:])
                    if not bare:
                        issues["bibtex_unresolved_field_macro"] += 1
                        next_field = rest.find(",", pos)
                        pos = next_field if next_field >= 0 else len(rest)
                        continue
                    value, pos = bare[0], pos + bare.end()
                if rest[skip_space(rest, pos):].startswith("#"):
                    issues["bibtex_concatenated_field"] += 1
                    next_field = rest.find(",", pos)
                    pos = next_field if next_field >= 0 else len(rest)
                    continue
                fields[name] = value
            labels, provenance = {}, {}
            for target, source in (("title", "title"), ("first_author", "author"), ("year", "year")):
                if source not in fields:
                    continue
                value = fields[source]
                if source == "author":
                    value = re.split(r"\s+and\s+", value, maxsplit=1)[0]
                before = renderer.unsupported.copy()
                rendered = render_text(renderer, value)
                if rendered and renderer.unsupported == before and (source != "year" or re.fullmatch(r"(?:18|19|20)\d{2}[a-z]?", rendered)):
                    labels[target] = rendered
                    provenance[target] = {"path": path, "key": key, "field": source}
            if key in output:
                issues["duplicate_bibtex_key"] += 1
                output[key] = None
            else:
                output[key] = {"labels": labels, "provenance": provenance}
    return output, issues


def markup_fields(raw, renderer):
    labels, provenance = {}, {}
    first_author_seen = False
    raw = mask_regions(raw, definition_regions(raw))
    for match in COMMAND.finditer(raw):
        if match[1] not in ("bibinfo", "bibfield"):
            continue
        name, pos = group(raw, match.end())
        value, _ = group(raw, pos)
        # A bibfield{author} wrapper can contain an entire list. Only the first
        # explicitly delimited bibinfo{author} establishes a first-author label.
        if name == "author" and match[1] == "bibfield":
            continue
        if name == "author":
            if first_author_seen:
                continue
            first_author_seen = True
        field = {"title": "title", "year": "year", "author": "first_author"}.get(name)
        if field and field not in labels:
            before = renderer.unsupported.copy()
            value = render_text(renderer, value)
            if field == "first_author" and re.search(r"\band\b|\bet\s+al\b|;", value, flags=re.IGNORECASE):
                continue
            if value and renderer.unsupported == before and (field != "year" or re.fullmatch(r"(?:18|19|20)\d{2}[a-z]?", value)):
                labels[field] = value
                provenance[field] = {"command": match[1], "field": name, "offset": match.start()}
    return labels, provenance


def parse_project(files, limits=Limits(), selected_main=None):
    expanded = expand_project(files, limits, selected_main)
    text = expanded.text
    renderer = Renderer(text, limits)
    scan = mask_regions(text, definition_regions(text))
    aliases = equation_aliases(text, scan)
    environment_events = environment_commands(scan, aliases, limits)
    # TeX conditionals and scoped/repeated definitions affect what exists, not
    # just its presentation. We do not execute them or certify their inventory.
    source_semantics = Counter()
    for path in local_style_dependencies(files, expanded.main, text):
        source_semantics["uninterpreted_local_style:" + path] += 1
    for command in COMMAND.finditer(scan):
        name = command[1].rstrip("*")
        if (name.startswith("if") and name != "iff") or name in {"else", "fi", "unless", "newif", "csname", "endcsname", "let", "futurelet"}:
            source_semantics["control_flow:" + name] += 1
    for name, count in renderer.definition_counts.items():
        if count > 1:
            source_semantics["redefined_macro:" + name] = count

    structural = CITES | REFS | {"begin", "end", "label", "caption", "bibitem", "input", "include", "bibliography", "newtheorem",
                               "tag", "notag", "nonumber", "numberwithin", "counterwithin", "setcounter", "addtocounter", "stepcounter", "refstepcounter"}

    macro_structure = {}

    def structural_macro(name):
        if name in aliases:
            return False
        if name in macro_structure:
            return macro_structure[name]
        pending, seen = [name], set()
        while pending:
            current = pending.pop()
            if current not in renderer.macros or current in seen:
                continue
            seen.add(current)
            if len(seen) > limits.expansion_steps:
                raise UnsupportedSource("macro dependency closure exceeds its bound")
            commands = {item[1].rstrip("*") for item in COMMAND.finditer(renderer.macros[current][1])}
            if commands & (structural | set(aliases)) or any("ref" in command.casefold() or "cite" in command.casefold() for command in commands):
                macro_structure[name] = True
                return True
            pending.extend(commands - seen)
        macro_structure[name] = False
        return False

    forwarding = {}

    def unverified_forwarding(name):
        if name not in forwarding:
            pending = [name]
            seen = set()
            forwarding[name] = False
            while pending:
                current = pending.pop()
                if current in seen or current not in renderer.macros:
                    continue
                seen.add(current)
                if len(seen) > limits.expansion_steps:
                    raise UnsupportedSource("macro argument dependency closure exceeds its bound")
                dependencies = {item[1].rstrip("*") for item in COMMAND.finditer(renderer.macros[current][1])}
                if any(renderer.macros.get(dependency, (0, ""))[0] or dependency in STORED_ARGUMENT_ROLES
                       for dependency in dependencies):
                    forwarding[name] = True
                    break
                pending.extend(dependencies - seen)
        return forwarding[name]

    def structural_argument_commands(value):
        commands = {item[1].rstrip("*") for item in COMMAND.finditer(value)}
        return sorted(item for item in commands if item in structural or item in aliases
                      or "cite" in item.casefold() or "ref" in item.casefold() or structural_macro(item))

    argument_evidence = []
    argument_steps, argument_bytes = 0, 0
    for command in COMMAND.finditer(scan):
        name = command[1].rstrip("*")
        if structural_macro(name):
            source_semantics["structural_macro:" + name] += 1
        if name not in renderer.macros:
            continue
        # Nested custom or standard stored-argument consumers have no established
        # forwarding contract. A body can discard/repeat/reorder #n, or end in
        # another macro which consumes additional caller tokens. Do not infer
        # visibility from the raw source inventory in any such invocation.
        if unverified_forwarding(name):
            source_semantics["unverified_macro_argument_forwarding:" + name] += 1
            argument_evidence.append({"macro": name, "reason": "unverified nested argument forwarding",
                                      "invocation": expanded.origins(command.start(), command.end() - int(command[1].endswith('*')))})
        pos = command.end() - int(command[1].endswith('*'))
        for number in range(renderer.macros[name][0]):
            start = skip_space(scan, pos)
            try:
                value, pos = token_argument(scan, start)
            except UnsupportedSource as error:
                if any(word in str(error) for word in ("bound", "recursion", "nesting")):
                    raise
                source_semantics["unverified_macro_arguments:" + name] += 1
                argument_evidence.append({"macro": name, "argument": number + 1, "reason": str(error),
                                          "invocation": expanded.origins(command.start(), command.end()),
                                          "source_members": expanded.origins(start, len(scan))})
                break
            argument_steps += 1
            argument_bytes += pos - start
            if argument_steps > limits.expansion_steps or argument_bytes > limits.text_bytes:
                raise UnsupportedSource("macro argument inspection exceeds its cumulative bound")
            affecting = structural_argument_commands(value)
            if affecting:
                source_semantics["structural_macro_argument:" + name] += 1
                argument_evidence.append({"macro": name, "argument": number + 1,
                                          "reason": "unverified structural argument visibility and multiplicity",
                                          "commands": affecting, "invocation": expanded.origins(command.start(), command.end() - int(command[1].endswith('*'))),
                                          "source_members": expanded.origins(start, pos)})
        if len(argument_evidence) > limits.expansion_steps:
            raise UnsupportedSource("macro argument evidence exceeds its cumulative bound")
    stored_evidence = []
    for command in COMMAND.finditer(scan):
        name = command[1].rstrip('*')
        if name not in STORED_ARGUMENT_ROLES:
            continue
        # Only explicit standard starred variants consume that character here.
        pos = command.end() - int(command[1].endswith('*') and name not in {'tag', 'includegraphics', 'vspace', 'hspace', 'newtheorem', 'DeclareMathOperator'} | CITES | REFS)
        invocation = expanded.origins(command.start(), pos)
        arguments = []
        try:
            if name in OPTIONAL_STORED_ARGUMENTS:
                for number in range(2):
                    start = skip_space(scan, pos)
                    value, stop = group(scan, start, '[', ']', False)
                    if value is None:
                        break
                    pos = stop
                    arguments.append(('option:' + str(number + 1), start, pos, value))
            for number in range(1, max(STORED_ARGUMENT_ROLES[name], default=0) + 1):
                start = skip_space(scan, pos)
                if name in LITERAL_ARGUMENTS and scan[start:start + 1] != '{':
                    # Literal delimiter forms do not execute their contents.
                    # Their exact delimiter grammar is not implemented here.
                    raise UnsupportedSource('unverified literal argument delimiter')
                value, pos = token_argument(scan, start)
                if number in STORED_ARGUMENT_ROLES[name]:
                    arguments.append((number, start, pos, value))
                if name == 'newtheorem':
                    start = skip_space(scan, pos)
                    value, stop = group(scan, start, '[', ']', False)
                    if value is not None:
                        pos = stop
                        arguments.append(('counter:' + str(number), start, pos, value))
        except UnsupportedSource as error:
            if any(word in str(error) for word in ('bound', 'recursion', 'nesting')):
                raise
            source_semantics['unverified_stored_arguments:' + name] += 1
            stored_evidence.append({'command': name, 'reason': str(error), 'invocation': invocation})
        for number, start, stop, value in arguments:
            argument_steps += 1
            argument_bytes += stop - start
            if argument_steps > limits.expansion_steps or argument_bytes > limits.text_bytes:
                raise UnsupportedSource('stored argument inspection exceeds its cumulative bound')
            affecting = structural_argument_commands(value)
            if affecting:
                source_semantics['structural_stored_argument:' + name] += 1
                stored_evidence.append({'command': name, 'argument': number, 'commands': affecting,
                                        'reason': 'unverified stored or literal argument visibility and multiplicity',
                                        'invocation': invocation, 'source_members': expanded.origins(start, stop)})
        if len(stored_evidence) + len(argument_evidence) > limits.expansion_steps:
            raise UnsupportedSource('stored argument evidence exceeds its cumulative bound')
    # Unimplemented low-level definitions can hide structure through aliases.
    # Track all referenced macro names transitively, without executing a body.
    reachable = {command[1].rstrip("*") for command in COMMAND.finditer(scan)}
    pending = list(reachable)
    while pending:
        name = pending.pop()
        if name in renderer.macros:
            extra = {command[1].rstrip("*") for command in COMMAND.finditer(renderer.macros[name][1])} - reachable
            reachable.update(extra)
            pending.extend(extra)
        if len(reachable) > limits.expansion_steps:
            raise UnsupportedSource("macro dependency inventory exceeds its bound")
    for start, end in definition_regions(text):
        definition = COMMAND.match(text, start)
        kind = definition[1].rstrip("*")
        if kind in {"def", "gdef", "edef", "xdef"}:
            name = COMMAND.match(text, skip_space(text, definition.end()))
            if name and name[1] in reachable and name[1] not in aliases:
                source_semantics["unsupported_definition:" + kind + ":" + name[1]] += 1
        elif kind in {"newenvironment", "renewenvironment"}:
            name, _ = group(text, definition.end())
            if any(row["value"] == name for row in argument_commands(scan, {"begin"})):
                source_semantics["unsupported_environment_definition:" + name] += 1
    literal_environments = {"verbatim", "Verbatim", "lstlisting", "minted", "alltt", "comment"}
    for row in argument_commands(scan, {"begin"}):
        if row["value"].rstrip("*") in literal_environments:
            source_semantics["literal_environment:" + row["value"]] += 1
    for name in reachable & {"verb", "Verb", "lstinline", "mintinline"}:
        source_semantics["literal_command:" + name] += 1
    unsupported_references = {name: 1 for name in sorted(reachable) if "ref" in name.casefold() and name not in REFS}
    # Unhandled commands cannot certify an empty inventory. Standard math and
    # presentation may be unrenderable while their inventory effect is known;
    # arbitrary deposited commands and hooks remain unsupported.
    inventory_commands = (set(ACCENTS) | set(FORMATTING) | set(SILENT) | set(SYMBOLS) | set(DROP_ARGUMENT) | INVENTORY_ONLY_PRIMITIVES | CITES | REFS | structural | set(renderer.macros) | set(aliases)
                          | {"documentclass", "documentstyle", "usepackage", "RequirePackage", "LoadClass", "title", "TITLE", "author", "date", "maketitle", "thanks", "footnote", "footnotemark", "footnotetext", "section", "subsection", "subsubsection", "paragraph", "subparagraph", "chapter", "part", "appendix", "item", "newpage", "clearpage", "pagebreak", "linebreak", "includegraphics", "bibliographystyle", "bibinfo", "bibfield", "href", "nocite", "newtheorem", "setcounter", "addtocounter", "refstepcounter", "pagestyle", "thispagestyle", "markboth", "tableofcontents", "listoffigures", "listoftables", "frac", "dfrac", "tfrac", "sqrt", "sum", "prod", "int", "iint", "iiint", "oint", "partial", "nabla", "lim", "log", "ln", "exp", "sin", "cos", "tan", "min", "max", "arg", "det", "sup", "inf", "overline", "underline", "hat", "widehat", "bar", "vec", "dot", "ddot", "notag", "nonumber", "hline", "cline", "toprule", "midrule", "bottomrule", "multicolumn", "multirow", "centering", "caption", "(", ")", "[", "]", "crefrange", "Crefrange", "cpageref", "Cpageref", "labelcref", "labelcpageref", "namecref", "nameCref", "lcnamecref", "pageref", "eqrefrange", "autopageref", "vpageref", "vref", "autocites", "parencites", "textcites", "citeauthor", "citeyear", "citeyearpar", "citenum", "citetext", "citealp", "citealt", "printbibliography", "addbibresource"})
    operator_declarations = math_operator_declarations(text, scan, inventory_commands, limits)
    for declaration in operator_declarations:
        if declaration['reason']:
            source_semantics['unverified_math_operator_declaration:' + declaration['reason']] += 1
        else:
            inventory_commands.update({'DeclareMathOperator', declaration['name']})
    for name in sorted(reachable - inventory_commands):
        source_semantics["unknown_inventory_command:" + name] += 1
    unsupported_bibliography = {name: 1 for name in sorted(reachable) if "bib" in name.casefold() and name not in {"bibliography", "bibliographystyle", "bibitem", "bibinfo", "bibfield", "bibnamefont", "bibfnamefont"}}
    statements = dict(STANDARD_STATEMENTS)
    definitions, ignored = [], Counter()
    for match in COMMAND.finditer(scan):
        if match[1].rstrip("*") != "newtheorem":
            continue
        name, pos = group(text, match.end())
        shared, pos = group(text, pos, "[", "]", False)
        title, pos = group(text, pos)
        within, pos = group(text, pos, "[", "]", False)
        statements[name] = renderer.plain(title)
        definitions.append({"environment": name, "title": statements[name], "shared_counter": shared, "within": within, "numbered": not match[1].endswith("*")})
    stack, nodes = [], []
    for command in environment_events:
        name = command["value"]
        if command["command"] == "begin":
            if len(stack) >= limits.group_depth:
                raise UnsupportedSource("environment nesting exceeds its bound")
            if name.rstrip('*') in {"equation", "align", "gather", "multline", "eqnarray"}:
                # These math environments have no optional bracket argument.
                # A leading interval/vector is authored mathematical content.
                option, content_start = None, command["end"]
            else:
                option, content_start = group(text, command["end"], "[", "]", False)
            stack.append({"environment": name, "start": command["start"], "content_start": content_start,
                          "option": option})
        else:
            if not stack or stack[-1]["environment"] != name:
                raise UnsupportedSource("unbalanced or crossing TeX environments")
            node = stack.pop()
            node.update(content_end=command["start"], end=command["end"])
            nodes.append(node)
    if stack:
        raise UnsupportedSource("unterminated TeX environment")
    documents = [node for node in nodes if node["environment"] == "document"]
    if len(documents) != 1:
        raise UnsupportedSource("expected one document environment")
    document = documents[0]
    nodes = [row for node in nodes for row in equation_rows(node, scan)]
    objects, numbering = [], Counter()
    semantic_nodes = [node for node in nodes if node["environment"].rstrip("*") in statements
                      or node["environment"].rstrip("*") in ENVIRONMENTS
                      or node["environment"].rstrip("*") in ("subfigure", "subtable")]

    def owned_commands(node, names):
        raw = scan[node["content_start"]:node["content_end"]]
        for row in argument_commands(raw, names):
            at = node["content_start"] + row["start"]
            if not any(child is not node and node["start"] < child["start"] <= at < child["end"] <= node["end"] for child in semantic_nodes):
                yield row

    for node in sorted(nodes, key=lambda item: item["start"]):
        environment = node["environment"]
        base = environment.rstrip("*")
        kind = "statement" if environment in statements or base in statements else ENVIRONMENTS.get(base)
        if not kind or not document["content_start"] <= node["start"] < document["content_end"]:
            continue
        if kind == "equation" and environment.endswith("*") and not node.get("explicit_tag") and not node.get("numbering_uncertain"):
            continue  # Unnumbered display math is outside numbered-equation truth.
        raw = text[node["content_start"]:node["content_end"]]
        label_rows = list(owned_commands(node, {"label"}))
        label_keys = [row["value"].strip() for row in label_rows]
        numbering[kind] += 1
        identity = label_keys[0] if label_keys else f"{kind}:{numbering[kind]}"
        caption_rows = list(owned_commands(node, {"caption"}))
        selected = caption_rows[0]["value"] if caption_rows else raw
        before = renderer.unsupported.copy()
        before_math = renderer.math_seen
        rendered = render_text(renderer, selected)
        unknown = dict(renderer.unsupported - before)
        if node.get("numbering_uncertain"):
            unknown["ambiguous_equation_numbering"] = 1
        has_math = kind == "equation" or renderer.math_seen > before_math
        if has_math and any(char in rendered for char in "^_"):
            # Flattened PDF text does not establish which tokens belong to a
            # superscript/subscript group. Until independent geometry proves
            # binding, neither braced nor one-token TeX scripts are certifiable.
            unknown["unverified_script_binding"] = 1
        row = {"id": "object:" + identity, "kind": kind, "environment": environment,
               "source_span": {"start": node["start"], "end": node["end"]},
               "source_members": expanded.origins(node["start"], node["end"]),
               "labels": label_keys, "text": rendered, "text_sha256": sha256(rendered.encode()),
               "caption": rendered if caption_rows else None, "unsupported_commands": unknown,
               "contains_math": has_math,
               "numbering_uncertain": node.get("numbering_uncertain", False),
               "statement_type": statements.get(environment, statements.get(base)), "proof_target_labels": [],
               "number_hint": str(numbering[kind]) if kind in ("figure", "table", "algorithm") else None}
        if kind == "proof" and node["option"]:
            row["proof_heading"] = render_text(renderer, node["option"])
            row["unsupported_commands"].update(renderer.unsupported - before)
            row["proof_target_labels"] = [item["value"].strip() for item in argument_commands(node["option"], REFS)]
            row["proof_heading_source"] = node["option"]
        objects.append(row)
    bibliographies = [node for node in nodes if node["environment"] == "thebibliography"]
    source_role_evidence = []
    unsupported_kind_inventory = Counter()
    heading_ranks = {name: rank for rank, name in enumerate(('part', 'chapter', 'section', 'subsection', 'subsubsection', 'paragraph', 'subparagraph'))}
    heading_names = set(heading_ranks)
    headings = list(argument_commands(scan, heading_names | {'textbf', 'textit', 'emph'}))
    formal_headings = [row for row in headings if row['command'].rstrip('*') in heading_names]
    formal_starts = [row['start'] for row in formal_headings]
    # Descendant headings belong to their enclosing section. Precompute the
    # seven bounded rank lists instead of scanning the remaining headings for
    # each role, which would make a heading-heavy source quadratic.
    boundary_starts = {rank: [row['start'] for row in formal_headings
                              if heading_ranks[row['command'].rstrip('*')] <= rank]
                       for rank in heading_ranks.values()}
    bibliography_masked = mask_regions(scan, [(row['start'], row['end']) for row in bibliographies + formal_headings])
    for heading in headings:
        if not document['content_start'] <= heading['start'] < document['content_end']:
            continue
        if any(row['start'] <= heading['start'] < row['end'] for row in bibliographies):
            continue
        formal = heading['command'].rstrip('*') in heading_names
        if not formal:
            # Only an isolated styled line supplies an unparsed heading cue;
            # a word inside a sentence/caption is not a bibliography section.
            line_start = scan.rfind('\n', 0, heading['start']) + 1
            line_end = scan.find('\n', heading['end'])
            if line_end < 0:
                line_end = len(scan)
            prefix = re.sub(r'\\(?:noindent|small|footnotesize|large|Large|bfseries)\b', '', scan[line_start:heading['start']])
            suffix = scan[heading['end']:line_end].replace('\\\\', '')
            if prefix.strip(' {}\t') or suffix.strip(' {}\t'):
                continue
        before = renderer.unsupported.copy()
        title = render_text(renderer, heading['value']).casefold().strip(' :.')
        if renderer.unsupported != before:
            continue  # Uninterpreted text cannot establish a particular role.
        boundaries = boundary_starts[heading_ranks[heading['command'].rstrip('*')]] if formal else formal_starts
        following = bisect_right(boundaries, heading['start'])
        end = min(boundaries[following], document['content_end']) if following < len(boundaries) else document['content_end']
        if title in {'references', 'bibliography', 'literature cited', 'works cited', 'references and notes'}:
            if bibliography_masked[heading['end']:end].strip(' {}\t\n\r'):
                unsupported_bibliography['unparsed_reference_section'] = unsupported_bibliography.get('unparsed_reference_section', 0) + 1
                source_role_evidence.append({'kind': 'bib_entry', 'reason': 'reference section contains content outside parsed bibliography entries',
                                             'heading': title, 'heading_members': expanded.origins(heading['start'], heading['end']),
                                             'source_members': expanded.origins(heading['start'], end)})
        if re.match(r'^(?:algorithm|procedure|pseudocode)(?:\b|(?=[0-9]))', title) and not any(
                row['kind'] == 'algorithm' and row['source_span']['start'] <= heading['start'] < row['source_span']['end'] for row in objects):
            unsupported_kind_inventory['algorithm'] += 1
            source_role_evidence.append({'kind': 'algorithm', 'reason': 'procedural heading has no parsed algorithm container',
                                         'heading_members': expanded.origins(heading['start'], heading['end']),
                                         'source_members': expanded.origins(heading['start'], end)})
    for node in nodes:
        if node['environment'] == 'enumerate' and node['option'] and re.search(r'\bstep(?:\b|(?=[0-9]))', node['option'], re.I):
            if not any(row['kind'] == 'algorithm' and row['source_span']['start'] <= node['start'] < row['source_span']['end'] for row in objects):
                unsupported_kind_inventory['algorithm'] += 1
                source_role_evidence.append({'kind': 'algorithm', 'reason': 'step-labeled procedural list has no parsed algorithm container',
                                             'source_members': expanded.origins(node['start'], node['end'])})
    entries, entry_keys, occupied = [], set(), []
    database, bib_issues = bibtex_fields({path: files[path] for path in expanded.coverage["bibliography_files"]}, renderer)
    ignored.update(bib_issues)
    for bibliography in bibliographies:
        raw = text[bibliography["content_start"]:bibliography["content_end"]]
        markers = list(argument_commands(mask_regions(raw, definition_regions(raw)), {"bibitem"}))
        prefix_end = markers[0]['start'] if markers else len(raw)
        prefix = mask_regions(raw[:prefix_end], definition_regions(raw[:prefix_end]))
        _, prefix_start = group(prefix, 0, required=False)  # thebibliography's label-width argument
        if prefix[prefix_start:].strip(' {}\t\n\r'):
            unsupported_bibliography['unparsed_bibliography_prefix'] = unsupported_bibliography.get('unparsed_bibliography_prefix', 0) + 1
            source_role_evidence.append({'kind': 'bib_entry', 'reason': 'bibliography content precedes its parsed entry markers',
                                         'source_members': expanded.origins(bibliography['content_start'] + prefix_start, bibliography['content_start'] + prefix_end)})
        if not markers:
            ignored["empty_bibliography"] += 1
        for number, marker in enumerate(markers):
            key = marker["value"].strip()
            if not key or key in entry_keys:
                raise UnsupportedSource("bibliography entry key is empty or duplicated")
            entry_keys.add(key)
            end = markers[number + 1]["start"] if number + 1 < len(markers) else len(raw)
            body = raw[marker["end"]:end]
            before = renderer.unsupported.copy()
            before_math = renderer.math_seen
            rendered = render_text(renderer, body)
            unknown = dict(renderer.unsupported - before)
            has_math = renderer.math_seen > before_math
            if has_math and any(char in rendered for char in "^_"):
                unknown["unverified_script_binding"] = 1
            field_labels, field_provenance = markup_fields(body, renderer)
            deposited = database.get(key)
            field_conflicts = []
            if deposited:
                for field, value in deposited["labels"].items():
                    if field in field_labels and field_labels[field] != value:
                        field_conflicts.append({"field": field, "reason": "BibTeX disagrees with explicit deposited bibliography markup", "provenance": deposited["provenance"][field]})
                    elif field not in field_labels:
                        field_conflicts.append({"field": field, "reason": "BibTeX alone does not establish the printed field boundaries and role", "provenance": deposited["provenance"][field]})
            ignored["unverified_bibtex_fields"] += len(field_conflicts)
            start = bibliography["content_start"] + marker["start"]
            source_end = bibliography["content_start"] + end
            printed_key = str(number + 1) if not marker["options"] else render_text(renderer, marker["options"][0])
            unknown.update(renderer.unsupported - before)
            entries.append({"id": key, "text": rendered, "text_sha256": sha256(rendered.encode()),
                            "printed_key": printed_key,
                            "numeric_key_hint": str(number + 1), "field_labels": field_labels,
                            "field_provenance": field_provenance, "source_span": {"start": start, "end": source_end},
                            "field_conflicts": field_conflicts,
                            "contains_math": has_math,
                            "source_members": expanded.origins(start, source_end), "unsupported_commands": unknown})
        occupied.append((bibliography["start"], bibliography["end"]))
    links = []
    unsupported_citations = Counter()
    for command in COMMAND.finditer(scan, document["content_start"], document["content_end"]):
        if any(start <= command.start() < end for start, end in occupied):
            continue
        name = command[1].rstrip("*")
        if name not in CITES and name != "nocite" and ("cite" in name.casefold()
                or (name in renderer.macros and re.search(r"\\(?:cite|textcite|parencite|autocite)", renderer.macros[name][1]))):
            unsupported_citations[name] += 1
    for row in argument_commands(scan, CITES | REFS):
        if not document["content_start"] <= row["start"] < document["content_end"] or any(start <= row["start"] < end for start, end in occupied):
            continue
        keys = [key.strip() for key in row["value"].split(",")]
        if not all(keys):
            raise UnsupportedSource("citation/reference has an empty target")
        left = text[max(document["content_start"], row["start"] - 500):row["start"]]
        right = text[row["end"]:min(document["content_end"], row["end"] + 500)]
        # Trim at TeX paragraph boundaries and commands so contexts do not invent
        # words from labels, macro arguments, or another citation.
        left = re.split(r"\n\s*\n", left)[-1]
        right = re.split(r"\n\s*\n", right)[0]
        boundaries = CITES | REFS | {"label", "begin", "end", "section", "subsection", "subsubsection", "caption"}
        before = renderer.unsupported.copy()
        before_math = renderer.math_seen
        context_error = None
        try:
            previous = list(argument_commands(left, boundaries))
            following = next((command for command in COMMAND.finditer(right) if command[1].rstrip("*") in boundaries), None)
            if previous:
                left = left[previous[-1]["end"]:]
            if following:
                right = right[:following.start()]
            context_before, context_after = renderer.plain(left)[-200:], renderer.plain(right)[:200]
        except UnsupportedSource as error:
            # These are clipped context windows, not complete source commands.
            # A citation inside a braced heading/macro can leave one side open;
            # retain its source occurrence and denominator without inventing a
            # closing brace or rejecting unrelated source inventories.
            if any(word in str(error) for word in ("bound", "recursion")):
                raise
            context_before, context_after = "", ""
            context_error = str(error)
        unknown_context = dict(renderer.unsupported - before)
        if context_error:
            unknown_context["unbalanced_clipped_context"] = 1
        layout_context = []
        for node in nodes:
            if node['environment'].rstrip('*') in UNVERIFIED_MATH_LAYOUTS and node['content_start'] <= row['start'] < node['content_end']:
                unknown_context['unverified_math_layout:' + node['environment']] = 1
                layout_context.append({'environment': node['environment'], 'source_members': expanded.origins(node['start'], node['end'])})
        # Context clipping intentionally removes begin/end commands. Retain
        # enclosing source layout provenance so that clipping cannot erase an
        # unverified row/column relationship from reference/citation evidence.
        math_context = renderer.math_seen > before_math or bool(layout_context)
        if math_context and any(char in context_before + context_after for char in "^_"):
            unknown_context["unverified_script_binding"] = 1
        links.append({"kind": "citation" if row["command"].rstrip("*") in CITES else "reference",
                      "command": row["command"], "targets": keys, "options": row["options"],
                      "source_span": {"start": row["start"], "end": row["end"]},
                      "source_members": expanded.origins(row["start"], row["end"]),
                      "context_before": context_before, "context_after": context_after,
                      "math_context": math_context,
                      "source_layout_context": layout_context,
                      "context_exclusion": context_error,
                      "unsupported_context_commands": unknown_context})
    label_targets = {}
    for row in objects:
        for label in row["labels"]:
            if label in label_targets:
                raise UnsupportedSource("object label is duplicated")
            label_targets[label] = row["id"]
    preceding_statement = None
    for row in objects:
        row["proof_targets"] = [label_targets.get(label) for label in row["proof_target_labels"]]
        if row["kind"] == "proof":
            if row["proof_target_labels"]:
                row["proof_linkage"] = "explicit source label in proof heading"
            elif row.get("proof_heading_source"):
                # An unparsed optional heading might name another statement.
                # Preserve that uncertainty rather than overriding it by order.
                row["proof_linkage"] = "unresolved optional proof heading"
            elif preceding_statement:
                row["proof_targets"] = [preceding_statement]
                row["proof_linkage"] = "unnamed proof: nearest preceding source statement"
            else:
                row["proof_linkage"] = "unlinked: no preceding source statement"
        elif row["kind"] == "statement":
            preceding_statement = row["id"]
    unsupported_environments = Counter(node["environment"] for node in nodes if node["environment"] not in statements and node["environment"].rstrip("*") not in statements and node["environment"].rstrip("*") not in ENVIRONMENTS)
    unknown_environments = {name: count for name, count in unsupported_environments.items() if name not in LAYOUT_ENVIRONMENTS}
    document_probe = None
    if not objects and not entries and not links:
        before, before_math = renderer.unsupported.copy(), renderer.math_seen
        probe = render_text(renderer, scan[document["content_start"]:document["content_end"]])
        document_probe = {"text": probe, "unsupported_commands": dict(renderer.unsupported - before), "contains_math": renderer.math_seen > before_math}
        if document_probe["contains_math"] and any(char in probe for char in "^_"):
            document_probe["unsupported_commands"]["unverified_script_binding"] = 1
    return {"main": expanded.main, "source_map": expanded.pieces, "objects": objects, "entries": entries, "links": links,
            "empty_inventory_document_probe": document_probe,
            "label_targets": label_targets, "statement_definitions": definitions,
            "coverage": {**expanded.coverage, "unsupported_commands": dict(renderer.unsupported),
                         "resolved_equation_aliases": {name: {"command": row["command"], "environment": row["value"],
                             "definition": expanded.origins(row["definition_start"], row["definition_end"]),
                             "invocations": [expanded.origins(event["start"], event["end"]) for event in environment_events if event.get("alias") == name]}
                             for name, row in aliases.items()},
                         "other_environments": dict(unsupported_environments), "issues": dict(ignored),
                         "math_operator_declarations": [{**{key: value for key, value in row.items() if key not in {'start', 'end'}},
                                                        "source_members": expanded.origins(row['start'], row['end']),
                                                        "inventory_verified": row['reason'] is None,
                                                        "rendering_verified": False} for row in operator_declarations],
                         "unverified_macro_arguments": argument_evidence,
                         "unverified_stored_arguments": stored_evidence,
                         "unsupported_object_environments": unknown_environments,
                         "unsupported_source_semantics": dict(source_semantics),
                         "unsupported_kind_inventory": dict(unsupported_kind_inventory),
                         "unparsed_source_roles": source_role_evidence,
                         "objects_by_kind": dict(Counter(row["kind"] for row in objects)),
                         "bibliography_entries": len(entries), "citation_commands": sum(row["kind"] == "citation" for row in links),
                         "unsupported_citation_commands": dict(unsupported_citations),
                         "unsupported_reference_commands": unsupported_references,
                         "unsupported_bibliography_commands": unsupported_bibliography}}
