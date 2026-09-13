"""Independent source-text alignment. Production object predictions are never read."""
from collections import Counter
import re
import unicodedata

from archive import UnsupportedSource
from parser import reference_names_verified


def utf16(value):
    return len(value.encode("utf-16-le")) // 2


def folded(char, math=False):
    if math:
        # Case, scripts, grouping and operators can change an equation. Lossy
        # plain PDF extraction withholds alignment rather than erasing them.
        # PDF fonts encode the identical mu glyph as either Greek mu or the
        # compatibility micro sign. Normalize this one encoding alias only;
        # general compatibility folding would erase superscript semantics.
        if char == "µ":
            char = "μ"
        return "".join(item for item in unicodedata.normalize("NFC", char) if not item.isspace())
    value = unicodedata.normalize("NFKD", char).casefold()
    return "".join(item for item in value if item.isalnum())


def normalized(value, math=False):
    return "".join(folded(char, math) for char in value)


class TextAlignment:
    def __init__(self, index):
        self.text = index["text"]
        if len(self.text.encode("utf-8")) > 16 * 1024 * 1024:
            raise UnsupportedSource("PDF index exceeds the alignment byte bound")
        self.tokens = index.get("tokens", [])
        self.maps = {}
        self.utf16_offsets = [0]
        for char in self.text:
            self.utf16_offsets.append(self.utf16_offsets[-1] + utf16(char))
        # Read only source text/token geometry, never index.objects/index.figures.
        for math in (False, True):
            letters, positions = [], []
            for at, char in enumerate(self.text):
                value = folded(char, math)
                letters.append(value)
                positions.extend([at] * len(value))
            self.maps[math] = ("".join(letters), positions)

    def unique(self, authored, math=False):
        needle = normalized(authored, math)
        if len(needle) < 8:
            return None, "insufficient distinctive source text"
        haystack, positions = self.maps[math]
        start = haystack.find(needle)
        if start < 0:
            return None, "source text is absent from PDF index"
        if haystack.find(needle, start + 1) >= 0:
            return None, "source text has multiple PDF matches"
        first, last = positions[start], positions[start + len(needle) - 1] + 1
        # Include terminal authored punctuation when it is actually printed.
        trailing = re.search(r"[^\w\s]+$", authored)
        if trailing:
            wanted = trailing[0].replace("}", "").replace("$", "")
            if wanted and self.text[last:].startswith(wanted):
                last += len(wanted)
        return {"start": self.utf16_offsets[first], "end": self.utf16_offsets[last],
                "codepoint_start": first, "codepoint_end": last, "quality": 1.0,
                "method": "unique source character sequence with presentation folding"}, None

    def geometry(self, span):
        by_page = {}
        for token in self.tokens:
            if token["start"] < span["end"] and span["start"] < token["end"]:
                by_page.setdefault(token["page"], []).extend(token.get("rects", []))
        output = []
        for page, rects in sorted(by_page.items()):
            if rects:
                output.append({"page": page, "rect": {"x_min": min(row["x_min"] for row in rects), "y_min": min(row["y_min"] for row in rects),
                                                         "x_max": max(row["x_max"] for row in rects), "y_max": max(row["y_max"] for row in rects)}})
        return output

    def prefix(self, span, entry):
        start = span["codepoint_start"]
        before = self.text[max(0, start - 80):start]
        key = entry.get("numeric_key_hint")
        if key:
            match = re.search(r"(?:\[" + re.escape(key) + r"\]|\(" + re.escape(key) + r"\)|(?<!\d)" + re.escape(key) + r"\.)\s*$", before)
            if match:
                first = max(0, start - 80) + match.start()
                return {**span, "start": self.utf16_offsets[first], "codepoint_start": first}, key
        return span, None

    def context_matches(self, span, link):
        math = link.get("math_context", False)
        before = normalized(link["context_before"], math)[-80:]
        after = normalized(link["context_after"], math)[:80]
        # One substantial unique context side can establish a location; a short
        # generic word or an empty context never chooses among repeated markers.
        evidence = []
        if len(before) >= 12:
            actual = normalized(self.text[max(0, span[0] - 300):span[0]], math)
            evidence.append(actual.endswith(before))
        if len(after) >= 12:
            actual = normalized(self.text[span[1]:span[1] + 300], math)
            evidence.append(actual.startswith(after))
        return bool(evidence) and all(evidence)


def numeric_values(raw):
    values = []
    for part in re.split(r"\s*[,;]\s*", raw.strip()):
        match = re.fullmatch(r"(\d+)(?:\s*[-–—]\s*(\d+))?", part)
        if not match:
            return None
        start, end = int(match[1]), int(match[2] or match[1])
        if start > end or end - start > 1000:
            return None
        values.extend(range(start, end + 1))
    return sorted(set(values))


def align_links(parsed, aligner, entry_matches):
    output, excluded = [], []
    numeric_markers = list(re.finditer(r"\[\s*\d[\d,;\s–—-]*\]|\(\s*\d[\d,;\s–—-]*\)", aligner.text))
    entries = {row["id"]: row for row in parsed["entries"]}
    claimed = set()
    for number, link in enumerate(parsed["links"]):
        if link["kind"] != "citation":
            continue
        if link.get("unsupported_context_commands"):
            excluded.append({"link": number, "reason": "unresolved source commands prevent complete citation context"})
            continue
        targets = list(dict.fromkeys(link["targets"]))
        if not all(target in entry_matches for target in targets):
            excluded.append({"link": number, "reason": "citation target entry is not independently aligned"})
            continue
        keys = [entry_matches[target].get("printed_numeric_key") for target in targets]
        candidates = []
        if all(key is not None for key in keys):
            wanted = sorted(set(map(int, keys)))
            candidates = [(match.start(), match.end()) for match in numeric_markers if numeric_values(match[0][1:-1]) == wanted]
        elif len(targets) == 1:
            label = entries[targets[0]].get("printed_key")
            if label and re.search(r"(?:18|19|20)\d{2}", label):
                wanted = normalized(label)
                haystack, positions = aligner.maps[False]
                at = haystack.find(wanted)
                while at >= 0 and len(candidates) < 1000:
                    start, end = positions[at], positions[at + len(wanted) - 1] + 1
                    # Parentheses around author/year are printed occurrence data.
                    if start > 0 and aligner.text[start - 1] == "(":
                        start -= 1
                    if end < len(aligner.text) and aligner.text[end] == ")":
                        end += 1
                    candidates.append((start, end))
                    at = haystack.find(wanted, at + 1)
        candidates = [span for span in candidates if aligner.context_matches(span, link)
                      and not any(row["spans"][0]["start"] <= aligner.utf16_offsets[span[0]] < row["spans"][-1]["end"] for row in entry_matches.values())]
        if len(candidates) != 1:
            excluded.append({"link": number, "reason": "printed citation is absent, ambiguous, or lacks distinctive source context", "candidate_count": len(candidates)})
            continue
        start, end = candidates[0]
        pairs = [(aligner.utf16_offsets[start], aligner.utf16_offsets[end], target) for target in targets]
        if any(pair in claimed for pair in pairs):
            excluded.append({"link": number, "reason": "two source commands claim the same printed occurrence"})
            continue
        claimed.update(pairs)
        for start16, end16, target in pairs:
            output.append({"start": start16, "end": end16, "target": target, "source_link": number})
    return output, excluded


def align_references(parsed, aligner, aligned_objects):
    output, excluded, claimed = [], [], set()
    # Source context identifies the exact printed expansion; the source label
    # supplies its destination. A number is never resolved by its value alone.
    markers = list(re.finditer(r"(?<![\w.])(?:\(?\d+(?:\.\d+)*[a-z]?\)?)(?!\w|\.\d)", aligner.text))
    present = {row["id"] for row in aligned_objects}
    for number, link in enumerate(parsed["links"]):
        if link["kind"] != "reference":
            continue
        ambiguous = {key: parsed.get('ambiguous_labels', {})[key]['candidate_count']
                     for key in dict.fromkeys(link['targets']) if key in parsed.get('ambiguous_labels', {})}
        if ambiguous:
            excluded.append({'link': number, 'reason': 'reference label has multiple source claims',
                             'requested_labels': list(link['targets']), 'ambiguous_labels': ambiguous})
            continue
        if not reference_names_verified(parsed, link['targets']):
            excluded.append({'link': number, 'reason': 'source label naming is unverified',
                             'requested_labels': list(link['targets'])})
            continue
        if link.get("unsupported_context_commands"):
            excluded.append({"link": number, "reason": "unresolved source commands prevent complete reference context"})
            continue
        targets = [parsed["label_targets"].get(key) for key in dict.fromkeys(link["targets"])]
        if len(targets) != 1 or targets[0] not in present:
            excluded.append({"link": number, "reason": "reference target is absent, unaligned or grouped"})
            continue
        candidates = [(match.start(), match.end()) for match in markers if aligner.context_matches((match.start(), match.end()), link)]
        if len(candidates) != 1:
            excluded.append({"link": number, "reason": "reference expansion lacks unique independent context", "candidate_count": len(candidates)})
            continue
        start, end = candidates[0]
        pair = (aligner.utf16_offsets[start], aligner.utf16_offsets[end], targets[0])
        if pair in claimed:
            excluded.append({"link": number, "reason": "source references claim the same printed occurrence"})
            continue
        claimed.add(pair)
        output.append({"start": pair[0], "end": pair[1], "target": pair[2], "source_link": number})
    return output, excluded


def align_paper(parsed, index, threshold=0.95):
    if type(threshold) not in (float, int) or not 0 < threshold <= 1:
        raise ValueError("alignment threshold must be finite and in (0,1]")
    if len({row['id'] for row in parsed['objects']}) != len(parsed['objects']):
        raise UnsupportedSource('source object identities are duplicated before alignment')
    occurrences = [row['source_occurrence_id'] for row in parsed['objects'] if 'source_occurrence_id' in row]
    if len(set(occurrences)) != len(occurrences):
        raise UnsupportedSource('source object occurrences are duplicated before alignment')
    aligner = TextAlignment(index)
    objects, entries, excluded = [], [], []
    entry_matches = {}
    for row in parsed["objects"]:
        if row.get("unsupported_commands"):
            reason = "unsupported math commands prevent complete equation alignment" if row["kind"] == "equation" else "unsupported commands prevent complete object text alignment"
            excluded.append({"kind": row["kind"], "id": row["id"], "reason": reason, "unsupported_commands": row["unsupported_commands"]})
            continue
        span, reason = aligner.unique(row["text"], row["kind"] == "equation" or row.get("contains_math", False))
        if span is None:
            excluded.append({"kind": row["kind"], "id": row["id"], "reason": reason})
            continue
        objects.append({"id": row["id"], "kind": row["kind"], "labels": row["labels"],
                        "spans": [{"start": span["start"], "end": span["end"]}], "quality": span["quality"],
                        "source_members": row["source_members"], "text_sha256": row["text_sha256"],
                        "caption": row["caption"], "statement_type": row["statement_type"],
                        "proof_targets": row["proof_targets"], "proof_linkage": row.get("proof_linkage"), "text_geometry": aligner.geometry(span),
                        "region": None, "region_status": "independent visual annotation required" if row["kind"] in ("figure", "table") else "not applicable"})
    for row in parsed["entries"]:
        if row.get("unsupported_commands"):
            excluded.append({"kind": "bib_entry", "id": row["id"], "reason": "unsupported commands prevent complete bibliography text alignment", "unsupported_commands": row["unsupported_commands"]})
            continue
        span, reason = aligner.unique(row["text"], row.get("contains_math", False))
        if span is None:
            excluded.append({"kind": "bib_entry", "id": row["id"], "reason": reason})
            continue
        span, numeric_key = aligner.prefix(span, row)
        entry = {"id": row["id"], "spans": [{"start": span["start"], "end": span["end"]}],
                 "field_labels": row["field_labels"], "field_provenance": row["field_provenance"],
                 "source_members": row["source_members"], "printed_numeric_key": numeric_key}
        entries.append(entry)
        entry_matches[row["id"]] = entry
    mentions, missing_links = align_links(parsed, aligner, entry_matches)
    references, missing_references = align_references(parsed, aligner, objects)
    expected_links = sum(len(set(row["targets"])) for row in parsed["links"] if row["kind"] == "citation")
    expected_references = sum(len(set(row["targets"])) for row in parsed["links"] if row["kind"] == "reference")
    total = len(parsed["objects"]) + len(parsed["entries"]) + expected_links + expected_references
    aligned = len(objects) + len(entries) + len(mentions) + len(references)
    overall_completeness = aligned / total if total else 1.0
    probe = parsed.get("empty_inventory_document_probe")
    empty_source_verified = bool(probe and not probe["unsupported_commands"] and aligner.unique(probe["text"], probe["contains_math"])[0] is not None)
    negative_evidence = aligned > 0 or (total == 0 and empty_source_verified)
    # Bibliography evaluation requires exhaustive source inventories: partial
    # entry or citation alignment must never reduce a detector's denominator.
    bibliography_complete = len(entries) == len(parsed["entries"]) and len(mentions) == expected_links and not missing_links and not parsed["coverage"].get("unsupported_citation_commands") and not parsed["coverage"].get("unsupported_bibliography_commands")
    spans = Counter((row["spans"][0]["start"], row["spans"][-1]["end"]) for row in objects + entries)
    duplicate_spans = [{"start": start, "end": end, "claims": count} for (start, end), count in sorted(spans.items()) if count > 1]
    inventory_known = (not parsed["coverage"].get("unsupported_object_environments")
                       and not parsed["coverage"].get("unsupported_source_semantics") and not duplicate_spans)
    expected_kinds = Counter(row["kind"] for row in parsed["objects"])
    aligned_kinds = Counter(row["kind"] for row in objects)
    eligible_kinds, kind_coverage = [], {}
    for kind in ("figure", "table", "equation", "statement", "proof", "algorithm"):
        expected, count = expected_kinds[kind], aligned_kinds[kind]
        kind_inventory_unknown = parsed['coverage'].get('unsupported_kind_inventory', {}).get(kind, 0)
        eligible = inventory_known and not kind_inventory_unknown and count == expected and (expected > 0 or negative_evidence)
        if eligible:
            eligible_kinds.append(kind)
        kind_coverage[kind] = {"expected": expected, "aligned": count, "eligible": eligible,
                               "reason": "complete independent inventory" if eligible else "source inventory semantics are unsupported" if not inventory_known else "source inventory of this kind is unsupported" if kind_inventory_unknown else "no source objects of this kind" if not expected else "not every source object aligns"}
    figure_tables = all(kind in eligible_kinds for kind in ("figure", "table"))
    bibliography_eligible = inventory_known and bibliography_complete and (bool(parsed["entries"]) or expected_links > 0 or negative_evidence)
    object_kinds = {row["id"]: row["kind"] for row in parsed["objects"]}
    relevant_reference_links = []
    unknown_reference_targets = False
    for number, link in enumerate(parsed["links"]):
        if link["kind"] != "reference":
            continue
        targets = [parsed["label_targets"].get(key) for key in set(link["targets"])]
        unknown_reference_targets |= (not reference_names_verified(parsed, link['targets'])
                                      or any(target is None for target in targets)
                                      or any(key in parsed.get('ambiguous_labels', {}) for key in link['targets']))
        relevant_reference_links.extend((number, target) for target in targets if object_kinds.get(target) in ("equation", "statement"))
    reference_pairs = {(row["source_link"], row["target"]) for row in references}
    object_links_complete = (inventory_known and (bool(relevant_reference_links) or negative_evidence) and not unknown_reference_targets and not parsed["coverage"].get("unsupported_reference_commands")
                             and all(pair in reference_pairs for pair in relevant_reference_links)
                             and all(kind in eligible_kinds for kind in {object_kinds[target] for _, target in relevant_reference_links}))
    proof_links_complete = ("proof" in eligible_kinds and "statement" in eligible_kinds
                            and all(reference_names_verified(parsed, row['proof_target_labels']) for row in parsed['objects'] if row['kind'] == 'proof' and row['proof_target_labels'])
                            and not any(label in parsed.get('ambiguous_labels', {}) for row in parsed['objects'] if row['kind'] == 'proof' for label in row['proof_target_labels'])
                            and all(row["proof_targets"] and all(target in object_kinds and object_kinds[target] == "statement" for target in row["proof_targets"]) for row in objects if row["kind"] == "proof"))
    metrics = {"O1": figure_tables, "O2": False, "O3": "equation" in eligible_kinds,
               "O4": object_links_complete, "O5": "statement" in eligible_kinds, "O6": proof_links_complete,
               "O7": "algorithm" in eligible_kinds, "O8": bibliography_eligible, "O9": bibliography_eligible,
               "O10": bibliography_eligible, "O11": False}
    accepted = any(metrics.values())
    # Exact uniquely aligned spans have confidence1. The fraction of all source
    # items aligned is coverage, retained independently; it cannot dilute or
    # shorten the complete per-metric inventories above.
    quality = min((row["quality"] for row in objects if row["kind"] in eligible_kinds), default=1.0) if accepted else overall_completeness
    accepted = accepted and quality >= threshold
    return {"accepted": accepted, "bibliography_eligible": accepted and bibliography_eligible,
            "eligible_kinds": eligible_kinds if accepted else [], "metric_eligibility": {key: accepted and value for key, value in metrics.items()},
            "kind_coverage": kind_coverage,
            "alignment": {"quality": quality, "method": "unique independent LaTeX text and source-context alignment", "threshold": threshold,
                          "aligned_items": aligned, "total_items": total, "overall_completeness": overall_completeness,
                          "empty_inventory_document_verified": empty_source_verified,
                          "bibliography_exhaustive": bibliography_complete},
            "duplicate_span_claims": duplicate_spans,
            "objects": objects, "entries": entries, "mentions": mentions, "excluded_objects": excluded,
            "references": references, "excluded_references": missing_references,
            "excluded_citations": missing_links, "coverage": parsed["coverage"],
            "aligned_objects_by_kind": dict(aligned_kinds),
            "accepted_objects_by_kind": {kind: aligned_kinds[kind] for kind in eligible_kinds if accepted}}
