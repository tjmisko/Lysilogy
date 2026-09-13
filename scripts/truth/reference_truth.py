#!/usr/bin/env python3
"""Offline K2/K5 derivation from immutable, content-verified provider snapshots.

This module never imports an HTTP client or invokes a model. Provider payloads stay
outside the repository; outputs contain bibliographic labels and provenance only.
"""
import argparse
import collections
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import urllib.parse

MAX_BYTES = 8 * 1024 * 1024
MAX_AGGREGATE_BYTES = 128 * 1024 * 1024
REFERENCE_FIELDS = ("author", "year", "article-title", "volume-title", "journal-title",
                    "series-title", "volume", "issue", "first-page")
OA_OPEN = {"diamond", "gold", "green", "hybrid", "bronze"}


class TruthError(ValueError):
    pass


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False)


def fingerprint(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def timestamp(value):
    try:
        date = dt.datetime.fromisoformat(value)
    except (TypeError, ValueError) as error:
        raise TruthError("Truth timestamps must be ISO 8601 with a timezone") from error
    if date.tzinfo is None:
        raise TruthError("Truth timestamps require a timezone")
    return date.astimezone(dt.timezone.utc)


def doi(value):
    if not isinstance(value, str):
        return None
    value = value.strip()
    if value.lower().startswith(("https://doi.org/", "http://doi.org/")):
        value = urllib.parse.unquote(value.split("/", 3)[3])
    elif value.lower().startswith("doi:"):
        value = value[4:].strip()
    if len(value) > 512 or not re.fullmatch(r"10\.[0-9]{4,9}/[^\s\x00-\x1f\x7f]+", value):
        return None
    return value.lower()


def text(value):
    return value.strip() if isinstance(value, str) and value.strip() else None


def year(value):
    if type(value) is int and 1000 <= value <= 2999:
        return value
    if isinstance(value, str) and re.fullmatch(r"[12][0-9]{3}[a-z]?", value.strip()):
        return int(value.strip()[:4])
    return None


def first(values):
    return next((value for value in values if value is not None), None)


def publication_year(record):
    for name in ("published", "published-print", "published-online", "issued"):
        dates = record.get(name, {})
        parts = dates.get("date-parts") if isinstance(dates, dict) else None
        if isinstance(parts, list) and parts and isinstance(parts[0], list) and parts[0]:
            if (result := year(parts[0][0])) is not None:
                return result
    return None


def read_json(path, maximum=MAX_AGGREGATE_BYTES):
    path = Path(path)
    reject_symlinks(path)
    with path.open("rb") as source:
        raw = source.read(maximum + 1)
    if len(raw) > maximum:
        raise TruthError("Truth input exceeds its size bound")
    try:
        return json.loads(raw)
    except (ValueError, UnicodeError) as error:
        raise TruthError("Malformed truth JSON") from error


def reject_symlinks(path):
    if any(parent.is_symlink() for parent in (path, *path.absolute().parents)):
        raise TruthError("Truth storage cannot use symlinks")


def write_immutable(path, raw):
    """Publish complete bytes without replacing an earlier snapshot or truth version."""
    path = Path(path)
    reject_symlinks(path)
    if path.exists():
        if path.read_bytes() != raw:
            raise TruthError("Existing truth differs; choose a new version path")
        return
    missing = [directory for directory in (path.parent, *path.parent.parents) if not directory.exists()]
    for directory in reversed(missing):
        directory.mkdir()
        descriptor = os.open(directory.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".truth-", delete=False) as output:
            temporary = Path(output.name)
            output.write(raw)
            output.flush()
            os.fsync(output.fileno())
        os.link(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)


def validate_request(request):
    if set(request) != {"provider", "dois"} or request["provider"] not in {"crossref", "openalex"}:
        raise TruthError("Frozen requests support Crossref and OpenAlex DOI lookups only")
    identifiers = request["dois"]
    if (not isinstance(identifiers, list) or not 1 <= len(identifiers) <= 100
            or any(doi(value) is None or doi(value) != value for value in identifiers)
            or len(set(identifiers)) != len(identifiers)):
        raise TruthError("Frozen requests require unique, normalized DOIs")
    if request["provider"] == "crossref" and len(identifiers) != 1:
        raise TruthError("Crossref requires one citing DOI per request")


def validate_lookup_identity(request, response, payload):
    validate_request(request)
    if not isinstance(payload, dict):
        raise TruthError("Truth provider payload must be an object")
    if response.get("schema_version") != 1 or response.get("provider") != request["provider"]:
        raise TruthError("Transport response differs from its requested provider")
    host = "api." + request["provider"] + ".org"
    endpoint = urllib.parse.urlsplit(response["url"])
    if endpoint.scheme != "https" or endpoint.netloc != host or endpoint.fragment:
        raise TruthError("Transport response must identify the fixed HTTPS provider endpoint")
    decoded_path = urllib.parse.unquote(endpoint.path)
    if len(request["dois"]) == 1:
        if endpoint.query or not decoded_path.startswith("/works/") or doi(decoded_path[len("/works/"):]) != request["dois"][0]:
            raise TruthError("Transport lookup differs from the requested DOI")
    else:
        query = urllib.parse.parse_qs(endpoint.query)
        if decoded_path != "/works" or query != {"filter": ["doi:" + "|".join(request["dois"])], "per-page": ["100"]}:
            raise TruthError("Transport lookup differs from the requested DOI batch")
    if timestamp(response["retrieved_at"]) > timestamp(response["frozen_at"]):
        raise TruthError("Transport response predates its provider fetch")
    records = ([payload.get("message")] if request["provider"] == "crossref"
               else payload.get("results", [payload]))
    if not isinstance(records, list):
        raise TruthError("Truth response has no work records")
    identities = [doi(record.get("DOI" if request["provider"] == "crossref" else "doi"))
                  if isinstance(record, dict) else None for record in records]
    if any(identifier not in request["dois"] for identifier in identities) or len(set(identities)) != len(identities):
        raise TruthError("Truth response contains a mismatched or duplicate work identity")
    if request["provider"] == "crossref" and identities != request["dois"]:
        raise TruthError("Crossref returned no matching citing record")
    return sorted(set(request["dois"]) - set(identities))


def validate_response(request, response):
    body = response.get("body")
    if not isinstance(body, str) or len(body.encode()) > MAX_BYTES:
        raise TruthError("Transport payload exceeds the provider response bound")
    if hashlib.sha256(body.encode()).hexdigest() != response.get("body_sha256"):
        raise TruthError("Transport payload fingerprint mismatch")
    payload = json.loads(body)
    if not isinstance(payload, dict):
        raise TruthError("Truth provider payload must be an object")
    missing = validate_lookup_identity(request, response, payload)
    return body.encode(), missing


def load_snapshots(root, manifest):
    """Ignore mutable provider TTL; require the exact frozen receipt and body hashes."""
    root = Path(root)
    reject_symlinks(root)
    if manifest.get("schema_version") != 1 or not isinstance(manifest.get("snapshots"), list):
        raise TruthError("Unsupported frozen snapshot manifest")
    loaded = []
    seen = set()
    for descriptor in manifest["snapshots"]:
        digest = descriptor.get("sha256", "")
        if not re.fullmatch(r"[0-9a-f]{64}", digest) or descriptor.get("path") != f"receipts/{digest}.json":
            raise TruthError("Invalid frozen receipt identity")
        if digest in seen:
            raise TruthError("Duplicate snapshot receipt")
        seen.add(digest)
        path = root / descriptor["path"]
        if any(parent.is_symlink() for parent in (root, path.parent, path)):
            raise TruthError("Frozen input paths cannot be symlinks")
        with path.open("rb") as source:
            raw = source.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES or hashlib.sha256(raw).hexdigest() != digest:
            raise TruthError("Frozen receipt fingerprint mismatch")
        receipt = json.loads(raw)
        body_hash = receipt.get("body_sha256", "")
        if not re.fullmatch(r"[0-9a-f]{64}", body_hash) or receipt.get("body_file") != f"responses/{body_hash}.json":
            raise TruthError("Invalid frozen response identity")
        body_path = root / receipt["body_file"]
        if body_path.is_symlink() or body_path.parent.is_symlink():
            raise TruthError("Frozen response paths cannot be symlinks")
        with body_path.open("rb") as source:
            body = source.read(MAX_BYTES + 1)
        if len(body) > MAX_BYTES or hashlib.sha256(body).hexdigest() != body_hash:
            raise TruthError("Frozen provider payload fingerprint mismatch")
        endpoint = urllib.parse.urlsplit(receipt["url"])
        host = {"crossref": "api.crossref.org", "openalex": "api.openalex.org"}.get(receipt["provider"])
        if (not host or endpoint.scheme != "https" or endpoint.netloc != host
                or endpoint.fragment or not endpoint.path.startswith("/works")):
            raise TruthError("Frozen response has an unexpected provider endpoint")
        if timestamp(receipt["retrieved_at"]) > timestamp(receipt["frozen_at"]):
            raise TruthError("A frozen response cannot predate its provider fetch")
        payload = json.loads(body)
        missing = validate_lookup_identity({"provider": receipt["provider"], "dois": receipt["requested_dois"]}, receipt, payload)
        if receipt["missing_dois"] != missing:
            raise TruthError("Frozen coverage differs from its returned identities")
        loaded.append({"snapshot_id": digest, "receipt": receipt, "payload": payload})
    return loaded


def source_record(snapshot, built_at):
    receipt = snapshot["receipt"]
    validate_lookup_identity({"provider": receipt["provider"], "dois": receipt["requested_dois"]}, receipt, snapshot["payload"])
    if timestamp(receipt["frozen_at"]) > timestamp(built_at):
        raise TruthError("Truth build predates its frozen source")
    return {"snapshot_id": snapshot["snapshot_id"],
            **{key: receipt[key] for key in ("provider", "url", "retrieved_at", "frozen_at", "body_sha256")}}


def finish(kind, built_at, content):
    timestamp(built_at)
    result = {"schema_version": 1, "truth_set": kind, "built_at": built_at, **content}
    result["version"] = f"{kind}-{timestamp(built_at).date()}-{fingerprint(result)[:16]}"
    return result


def build_k2(snapshots, seeds, built_at):
    """Retain deposited entries separately from expected DOIs; never synthesize input from labels."""
    seed_map = {}
    for seed in seeds:
        if set(seed) - {"doi", "kind", "arxiv_id", "metadata_sha256", "selection_sha256", "local_source_sha256"}:
            raise TruthError("K2 seed provenance contains unsupported fields")
        identifier = doi(seed.get("doi"))
        if identifier is None:
            raise TruthError("K2 seeds require one valid citing DOI each")
        seed_map.setdefault(identifier, []).append(seed)
    works, cases, sources = {}, [], []
    coverage = collections.Counter()
    for snapshot in sorted(snapshots, key=lambda item: item["snapshot_id"]):
        if snapshot["receipt"]["provider"] != "crossref":
            continue
        record = snapshot["payload"].get("message")
        if not isinstance(record, dict) or (identifier := doi(record.get("DOI"))) not in seed_map:
            raise TruthError("Crossref citing identity does not match a supplied seed")
        if identifier in works:
            raise TruthError("K2 must freeze exactly one Crossref record per citing DOI")
        sources.append(source_record(snapshot, built_at))
        authors = record.get("author", [])
        if not isinstance(authors, list):
            raise TruthError("Crossref author field must be an array")
        names = []
        for author in authors:
            if isinstance(author, dict):
                name = " ".join(filter(None, (text(author.get("given")), text(author.get("family")))))
                if name or text(author.get("name")):
                    names.append(name or text(author["name"]))
        titles = record.get("title", [])
        works[identifier] = {"doi": identifier, "title": first(map(text, titles)) if isinstance(titles, list) else None,
                             "authors": names, "year": publication_year(record),
                             "origins": sorted(seed_map[identifier], key=canonical),
                             "snapshot_id": snapshot["snapshot_id"]}
        if "reference" not in record:
            coverage["citing_records_without_reference_list"] += 1
            continue
        if not isinstance(record["reference"], list):
            raise TruthError("Deposited references must be an array")
        for index, reference in enumerate(record["reference"]):
            coverage["deposited_references"] += 1
            if not isinstance(reference, dict):
                coverage["malformed_references"] += 1
                continue
            expected = doi(reference.get("DOI"))
            if expected is None:
                coverage["references_without_valid_doi"] += 1
                continue
            fields = {key: str(reference[key]).strip() for key in REFERENCE_FIELDS
                      if type(reference.get(key)) in (str, int) and str(reference[key]).strip()}
            unstructured = text(reference.get("unstructured"))
            input_text = unstructured or ". ".join(fields.values()) or None
            kind = "deposited_unstructured" if unstructured else "deposited_structured" if fields else "missing"
            exposed = bool(input_text and expected in urllib.parse.unquote(input_text).lower())
            case_id = "K2-" + fingerprint([identifier, index, reference])[:24]
            cases.append({"case_id": case_id, "citing_doi": identifier, "expected_doi": expected,
                          "input": {"text": input_text, "fields": fields, "kind": kind},
                          "expected_identifier_in_input": exposed, "eligible_resolution": input_text is not None,
                          "snapshot_id": snapshot["snapshot_id"], "json_pointer": f"/message/reference/{index}",
                          "deposited_entry_sha256": fingerprint(reference)})
            coverage["doi_labels"] += 1
            coverage["eligible_resolution" if input_text else "doi_only_labels"] += 1
            coverage["identifier_present_in_input" if exposed else "identifier_not_in_input"] += 1
    missing = sorted(set(seed_map) - set(works))
    if not works:
        raise TruthError("K2 requires actual frozen Crossref citing records")
    return finish("K2", built_at, {"sources": sources, "works": [works[key] for key in sorted(works)],
                                  "cases": sorted(cases, key=lambda case: case["case_id"]),
                                  "coverage": {**dict(sorted(coverage.items())), "missing_seed_dois": missing}})


def openalex_records(snapshots, built_at):
    indexed, sources = {}, []
    for snapshot in sorted(snapshots, key=lambda item: item["snapshot_id"]):
        if snapshot["receipt"]["provider"] != "openalex":
            continue
        sources.append(source_record(snapshot, built_at))
        payload = snapshot["payload"]
        records = payload.get("results") if "results" in payload else [payload]
        if not isinstance(records, list):
            raise TruthError("OpenAlex records must be an array")
        for index, record in enumerate(records):
            if not isinstance(record, dict):
                raise TruthError("OpenAlex returned a malformed work")
            identifier = doi(record.get("doi"))
            if identifier is None:
                continue
            if identifier in indexed:
                raise TruthError("K5 requires one frozen OpenAlex record per DOI, without conflicting refreshes")
            indexed[identifier] = (record, snapshot, f"/results/{index}" if "results" in payload else "")
    return indexed, sources


def oa_label(record):
    oa = record.get("open_access")
    if not isinstance(oa, dict) or type(oa.get("is_oa")) is not bool:
        return {"status": "unknown", "reason": "missing_explicit_oa_status"}
    flag, color = oa["is_oa"], oa.get("oa_status")
    if (color not in OA_OPEN | {"closed"}) or flag != (color in OA_OPEN):
        return {"status": "unknown", "reason": "unrecognized_or_conflicting_oa_status"}
    url = oa.get("oa_url")
    return {"status": "open" if flag else "closed", "is_oa": flag, "oa_status": color,
            "oa_url": url if isinstance(url, str) else None, "reason": None}


def balanced_sample(candidates, count, seed):
    """Deterministic round-robin over observed field/decade strata, redistributing sparse cells."""
    if type(count) is not int or count <= 0:
        raise TruthError("Sample count must be a positive integer")
    groups = collections.defaultdict(list)
    for row in candidates:
        groups[(row["field_id"], row["decade"])].append(row)
    if sum(map(len, groups.values())) < count:
        raise TruthError(f"K5 has {len(candidates)} eligible unique references; requires {count}")
    for key in groups:
        groups[key].sort(key=lambda row: (fingerprint([seed, row["expected_doi"]]), row["expected_doi"]))
    cell_order = sorted(groups, key=lambda key: (fingerprint([seed, "stratum", *key]), key))
    selected, index = [], 0
    while len(selected) < count:
        for key in cell_order:
            if index < len(groups[key]):
                selected.append(groups[key][index])
                if len(selected) == count:
                    break
        index += 1
    return selected


def build_k5(k2, snapshots, built_at, count=200, seed="reference-truth-v1"):
    if k2.get("truth_set") != "K2" or timestamp(k2["built_at"]) > timestamp(built_at):
        raise TruthError("K5 requires a previously built K2 truth set")
    records, sources = openalex_records(snapshots, built_at)
    representatives = {}
    for case in k2["cases"]:
        if case["eligible_resolution"]:
            identifier = case["expected_doi"]
            # Prefer a deposited input without an explicit target identifier, then stable case ID.
            rank = (case["expected_identifier_in_input"], case["case_id"])
            if identifier not in representatives or rank < representatives[identifier][0]:
                representatives[identifier] = (rank, case)
    candidates, excluded = [], collections.Counter()
    for identifier, (_, case) in sorted(representatives.items()):
        if identifier not in records:
            excluded["no_frozen_openalex_record"] += 1
            continue
        record, snapshot, pointer = records[identifier]
        topic = record.get("primary_topic")
        field = topic.get("field") if isinstance(topic, dict) else None
        published = year(record.get("publication_year"))
        if not isinstance(field, dict) or not text(field.get("id")) or published is None:
            excluded["missing_reference_field_or_year"] += 1
            continue
        candidates.append({"k2_case_id": case["case_id"], "expected_doi": identifier,
                           "field_id": field["id"], "field_name": text(field.get("display_name")),
                           "year": published, "decade": (published // 10) * 10,
                           "oa": oa_label(record), "snapshot_id": snapshot["snapshot_id"],
                           "retrieved_at": snapshot["receipt"]["retrieved_at"], "json_pointer": pointer})
    selected = balanced_sample(candidates, count, seed)
    available = collections.Counter((row["field_id"], row["decade"]) for row in candidates)
    taken = collections.Counter((row["field_id"], row["decade"]) for row in selected)
    return finish("K5", built_at, {"k2_version": k2["version"], "k2_sha256": fingerprint(k2),
                                  "seed": seed, "requested_count": count, "sources": sources,
                                  "samples": sorted(selected, key=lambda row: row["expected_doi"]),
                                  "coverage": {"eligible_candidates": len(candidates), "excluded": dict(sorted(excluded.items())),
                                               "oa_status": dict(sorted(collections.Counter(row["oa"]["status"] for row in selected).items())),
                                               "strata": [{"field_id": field, "decade": decade, "available": available[(field, decade)],
                                                           "selected": taken[(field, decade)]} for field, decade in sorted(available)]}})


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frozen-root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--built-at", required=True)
    parser.add_argument("--output", type=Path, required=True)
    sub = parser.add_subparsers(dest="command", required=True)
    k2 = sub.add_parser("k2")
    k2.add_argument("--seeds", type=Path, required=True)
    k5 = sub.add_parser("k5")
    k5.add_argument("--k2", type=Path, required=True)
    k5.add_argument("--count", type=int, default=200)
    k5.add_argument("--seed", default="reference-truth-v1")
    args = parser.parse_args(argv)
    try:
        snapshots = load_snapshots(args.frozen_root, read_json(args.manifest))
        result = (build_k2(snapshots, read_json(args.seeds)["seeds"], args.built_at) if args.command == "k2"
                  else build_k5(read_json(args.k2), snapshots, args.built_at, args.count, args.seed))
        encoded = canonical(result) + "\n"
        write_immutable(args.output, encoded.encode())
        print(canonical({"truth_set": result["truth_set"], "version": result["version"], "coverage": result["coverage"]}))
        return 0
    except (OSError, KeyError, TypeError, ValueError) as error:
        print(f"reference truth: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
