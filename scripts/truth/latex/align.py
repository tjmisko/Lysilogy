"""Independent source-text alignment. Production object predictions are never read."""
from collections import Counter
import re
import unicodedata

from archive import UnsupportedSource


def utf16(value):
    return len(value.encode("utf-16-le")) // 2


def folded(char, math=False):
    if math:
        # Case, scripts, grouping and operators can change an equation. Lossy
        # plain PDF extraction withholds alignment rather than erasing them.
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
        before = normalized(link["context_before"])[-80:]
        after = normalized(link["context_after"])[:80]
        # One substantial unique context side can establish a location; a short
        # generic word or an empty context never chooses among repeated markers.
        evidence = []
        if len(before) >= 12:
            actual = normalized(self.text[max(0, span[0] - 300):span[0]])
            evidence.append(actual.endswith(before))
        if len(after) >= 12:
            actual = normalized(self.text[span[1]:span[1] + 300])
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
    aligner = TextAlignment(index)
    objects, entries, excluded = [], [], []
    entry_matches = {}
    for row in parsed["objects"]:
        if row.get("unsupported_commands"):
            reason = "unsupported math commands prevent complete equation alignment" if row["kind"] == "equation" else "unsupported commands prevent complete object text alignment"
            excluded.append({"kind": row["kind"], "id": row["id"], "reason": reason, "unsupported_commands": row["unsupported_commands"]})
            continue
        span, reason = aligner.unique(row["text"], row["kind"] == "equation")
        if span is None:
            excluded.append({"kind": row["kind"], "id": row["id"], "reason": reason})
            continue
        objects.append({"id": row["id"], "kind": row["kind"], "labels": row["labels"],
                        "spans": [{"start": span["start"], "end": span["end"]}], "quality": span["quality"],
                        "source_members": row["source_members"], "text_sha256": row["text_sha256"],
                        "caption": row["caption"], "statement_type": row["statement_type"],
                        "proof_targets": row["proof_targets"], "text_geometry": aligner.geometry(span),
                        "region": None, "region_status": "independent visual annotation required" if row["kind"] in ("figure", "table") else "not applicable"})
    for row in parsed["entries"]:
        if row.get("unsupported_commands"):
            excluded.append({"kind": "bib_entry", "id": row["id"], "reason": "unsupported commands prevent complete bibliography text alignment", "unsupported_commands": row["unsupported_commands"]})
            continue
        span, reason = aligner.unique(row["text"])
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
    quality = aligned / total if total else 0.0
    # Bibliography evaluation requires exhaustive source inventories: partial
    # entry or citation alignment must never reduce a detector's denominator.
    bibliography_complete = bool(parsed["entries"]) and len(entries) == len(parsed["entries"]) and len(mentions) == expected_links and not missing_links and not parsed["coverage"].get("unsupported_citation_commands")
    spans = Counter((row["spans"][0]["start"], row["spans"][-1]["end"]) for row in objects + entries)
    duplicate_spans = [{"start": start, "end": end, "claims": count} for (start, end), count in sorted(spans.items()) if count > 1]
    accepted = (bool(total) and quality >= threshold and not parsed["coverage"].get("unsupported_object_environments")
                and not parsed["coverage"].get("unsupported_source_semantics") and not duplicate_spans)
    return {"accepted": accepted, "bibliography_eligible": accepted and bibliography_complete,
            "alignment": {"quality": quality, "method": "unique independent LaTeX text and source-context alignment", "threshold": threshold,
                          "aligned_items": aligned, "total_items": total, "bibliography_exhaustive": bibliography_complete},
            "duplicate_span_claims": duplicate_spans,
            "objects": objects, "entries": entries, "mentions": mentions, "excluded_objects": excluded,
            "references": references, "excluded_references": missing_references,
            "excluded_citations": missing_links, "coverage": parsed["coverage"],
            "accepted_objects_by_kind": dict(Counter(row["kind"] for row in objects))}
