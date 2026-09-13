"""Derive evaluation objects and links from source structure, not PDF detectors."""
from collections import Counter
import re

from archive import Limits, UnsupportedSource, sha256
from tex import COMMAND, Renderer, comments, definition_regions, expand_project, group, local_style_dependencies, mask_regions, skip_space

STANDARD_STATEMENTS = {name: name for name in ("theorem", "lemma", "corollary", "proposition", "definition", "assumption", "remark", "claim", "conjecture", "example")}
ENVIRONMENTS = {"figure": "figure", "table": "table", "equation": "equation", "align": "equation", "gather": "equation", "multline": "equation", "eqnarray": "equation", "proof": "proof", "algorithm": "algorithm", "algorithm2e": "algorithm", "listing": "algorithm", "lstlisting": "algorithm"}
CITES = {"cite", "citep", "citet", "citealt", "citealp", "parencite", "textcite", "autocite"}
REFS = {"ref", "eqref", "autoref", "cref", "Cref", "vref"}
LAYOUT_ENVIRONMENTS = {"document", "abstract", "thebibliography", "itemize", "enumerate", "description", "center", "quote", "quotation", "minipage", "tabular", "tabularx", "tabular*", "array", "split", "aligned", "alignedat", "subequations", "subfigure", "subtable", "algorithmic", "algorithmicx", "algorithmic*", "flushleft", "flushright", "IEEEkeywords", "keywords", "tikzpicture", "picture", "adjustbox", "threeparttable", "tablenotes", "multicols", "spacing", "doublespace", "singlespace", "small", "footnotesize", "landscape"}


def equation_rows(node, text):
    """Only top-level row separators create separately numbered equations."""
    if node["environment"].rstrip("*") not in ("align", "gather", "eqnarray"):
        return [{**node, "explicit_tag": bool(list(argument_commands(text[node["content_start"]:node["content_end"]], {"tag"})))}]
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
                raise UnsupportedSource("unnumbered equation row has an ambiguous label")
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
                rendered = renderer.plain(value)
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
            value = renderer.plain(value)
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

    structural = CITES | REFS | {"begin", "end", "label", "caption", "bibitem", "input", "include", "bibliography", "newtheorem"}

    macro_structure = {}

    def structural_macro(name):
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
            if commands & structural or any("ref" in command.casefold() or "cite" in command.casefold() for command in commands):
                macro_structure[name] = True
                return True
            pending.extend(commands - seen)
        macro_structure[name] = False
        return False

    for command in COMMAND.finditer(scan):
        name = command[1].rstrip("*")
        if structural_macro(name):
            source_semantics["structural_macro:" + name] += 1
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
            if name and name[1] in reachable:
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
    for command in argument_commands(scan, {"begin", "end"}):
        name = command["value"]
        if command["command"] == "begin":
            if len(stack) >= limits.group_depth:
                raise UnsupportedSource("environment nesting exceeds its bound")
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
        if kind == "equation" and environment.endswith("*") and not node.get("explicit_tag"):
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
        rendered = renderer.plain(selected)
        unknown = dict(renderer.unsupported - before)
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
               "statement_type": statements.get(environment, statements.get(base)), "proof_target_labels": [],
               "number_hint": str(numbering[kind]) if kind in ("figure", "table", "algorithm") else None}
        if kind == "proof" and node["option"]:
            row["proof_heading"] = renderer.plain(node["option"])
            row["proof_target_labels"] = [item["value"].strip() for item in argument_commands(node["option"], REFS)]
        objects.append(row)
    bibliographies = [node for node in nodes if node["environment"] == "thebibliography"]
    entries, entry_keys, occupied = [], set(), []
    database, bib_issues = bibtex_fields({path: files[path] for path in expanded.coverage["bibliography_files"]}, renderer)
    ignored.update(bib_issues)
    for bibliography in bibliographies:
        raw = text[bibliography["content_start"]:bibliography["content_end"]]
        markers = list(argument_commands(mask_regions(raw, definition_regions(raw)), {"bibitem"}))
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
            rendered = renderer.plain(body)
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
            entries.append({"id": key, "text": rendered, "text_sha256": sha256(rendered.encode()),
                            "printed_key": str(number + 1) if not marker["options"] else renderer.plain(marker["options"][0]),
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
        previous = list(argument_commands(left, boundaries))
        following = next((command for command in COMMAND.finditer(right) if command[1].rstrip("*") in boundaries), None)
        if previous:
            left = left[previous[-1]["end"]:]
        if following:
            right = right[:following.start()]
        before = renderer.unsupported.copy()
        before_math = renderer.math_seen
        context_before, context_after = renderer.plain(left)[-200:], renderer.plain(right)[:200]
        unknown_context = dict(renderer.unsupported - before)
        math_context = renderer.math_seen > before_math
        if math_context and any(char in context_before + context_after for char in "^_"):
            unknown_context["unverified_script_binding"] = 1
        links.append({"kind": "citation" if row["command"].rstrip("*") in CITES else "reference",
                      "command": row["command"], "targets": keys, "options": row["options"],
                      "source_span": {"start": row["start"], "end": row["end"]},
                      "source_members": expanded.origins(row["start"], row["end"]),
                      "context_before": context_before, "context_after": context_after,
                      "math_context": math_context,
                      "unsupported_context_commands": unknown_context})
    label_targets = {}
    for row in objects:
        for label in row["labels"]:
            if label in label_targets:
                raise UnsupportedSource("object label is duplicated")
            label_targets[label] = row["id"]
    for row in objects:
        row["proof_targets"] = [label_targets.get(label) for label in row["proof_target_labels"]]
    unsupported_environments = Counter(node["environment"] for node in nodes if node["environment"] not in statements and node["environment"].rstrip("*") not in statements and node["environment"].rstrip("*") not in ENVIRONMENTS)
    unknown_environments = {name: count for name, count in unsupported_environments.items() if name not in LAYOUT_ENVIRONMENTS}
    return {"main": expanded.main, "source_map": expanded.pieces, "objects": objects, "entries": entries, "links": links,
            "label_targets": label_targets, "statement_definitions": definitions,
            "coverage": {**expanded.coverage, "unsupported_commands": dict(renderer.unsupported),
                         "other_environments": dict(unsupported_environments), "issues": dict(ignored),
                         "unsupported_object_environments": unknown_environments,
                         "unsupported_source_semantics": dict(source_semantics),
                         "objects_by_kind": dict(Counter(row["kind"] for row in objects)),
                         "bibliography_entries": len(entries), "citation_commands": sum(row["kind"] == "citation" for row in links),
                         "unsupported_citation_commands": dict(unsupported_citations),
                         "unsupported_reference_commands": unsupported_references}}
