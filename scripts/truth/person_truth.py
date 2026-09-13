#!/usr/bin/env python3
"""Derive K4 silver labels offline from frozen source-deposited OpenAlex ORCIDs."""
import argparse
import collections
from pathlib import Path
import re
import sys
import unicodedata

from reference_truth import (TruthError, canonical, doi, fingerprint, finish, load_snapshots,
                             openalex_records, read_json, timestamp, write_immutable)

MAX_WORKS = 200_000
MAX_SLOTS = 1_000_000
MAX_NAME_CHARS = 1024
ORCID_SHAPE = re.compile(r"[0-9]{4}-[0-9]{4}-[0-9]{4}-[0-9]{3}[0-9X]")


def orcid(value):
    """Shape/checksum validation is not proof of a registered ORCID assignment."""
    if (not isinstance(value, str) or len(value) > 128
            or any(unicodedata.category(char).startswith("C") for char in value)):
        return None
    value = value.strip(" ")
    value = re.sub(r"^https?://orcid\.org/", "", value, flags=re.IGNORECASE)
    if not ORCID_SHAPE.fullmatch(value):
        return None
    digits = value.replace("-", "")
    total = 0
    for digit in digits[:15]:
        total = (total + int(digit)) * 2
    check = (12 - total % 11) % 11
    if digits[-1] != ("X" if check == 10 else str(check)):
        return None
    return "https://orcid.org/" + value


def missing(value):
    return value is None or (isinstance(value, str) and not value.strip())


def observed(value):
    # Never copy malformed provider objects or unbounded text into a label record.
    if value is None or type(value) in (int, bool) or (isinstance(value, str) and len(value) <= MAX_NAME_CHARS):
        return value
    return {"invalid_value_type": type(value).__name__, "sha256": fingerprint(value)}


def name_problem(value):
    if not isinstance(value, str) or not value.strip():
        return "missing_raw_name"
    if len(value) > MAX_NAME_CHARS:
        return "oversize_raw_name"
    if any(unicodedata.category(char) in {"Cc", "Cs"} for char in value):
        return "invalid_raw_name"
    # Detect visible or compatibility-obfuscated IDs without altering the printed name.
    scan = "".join(char for char in unicodedata.normalize("NFKC", value)
                   if unicodedata.category(char) != "Cf")
    scan = re.sub(r"[\u2010-\u2015\u2212]", "-", scan)
    if re.search(r"orcid\s*(?:[.:/]|\b)|[0-9]{4}[ -]?[0-9]{4}[ -]?[0-9]{4}[ -]?[0-9]{3}[0-9Xx]", scan, re.I):
        return "identifier_in_raw_name"
    if re.search(r"(?:openalex\.org\s*/|openalex\s*:)\s*A[0-9]+", scan, re.I):
        return "profile_identifier_in_raw_name"
    return None


def checked_truth(value, kind):
    if value.get("schema_version") != 1 or value.get("truth_set") != kind:
        raise TruthError(f"Person truth input requires a versioned {kind} truth set")
    original = {key: field for key, field in value.items() if key != "version"}
    expected = f"{kind}-{timestamp(value['built_at']).date()}-{fingerprint(original)[:16]}"
    if value.get("version") != expected:
        raise TruthError(f"{kind} version differs from its content fingerprint")


def universe(k2=None, selection=None):
    """Membership identifies observed works, not downloaded or mapped corpus artifacts."""
    if k2 is None and selection is None:
        raise TruthError("K4 work membership requires K2 or K0 selection metadata")
    records, inputs, counts = collections.defaultdict(list), [], collections.Counter()

    def add(value, origin):
        kind = origin["kind"]
        counts[kind + "_rows"] += 1
        identifier = doi(value)
        if identifier is None:
            counts[kind + ("_missing_doi" if missing(value) else "_invalid_doi")] += 1
            return
        records[identifier].append(origin)
        if len(records) > MAX_WORKS:
            raise TruthError("K4 work membership exceeds its work bound")

    if k2 is not None:
        checked_truth(k2, "K2")
        inputs.append({"kind": "K2", "version": k2["version"], "built_at": k2["built_at"],
                       "content_sha256": fingerprint(k2)})
        if not isinstance(k2.get("works"), list) or not isinstance(k2.get("cases"), list):
            raise TruthError("K2 work and reference membership must be arrays")
        seen = set()
        for row in k2["works"]:
            identifier = doi(row.get("doi"))
            if identifier in seen and identifier is not None:
                raise TruthError("K2 citing membership contains duplicate work identities")
            seen.add(identifier)
            add(row.get("doi"), {"kind": "K2_citing", "k2_version": k2["version"],
                                 "snapshot_id": row["snapshot_id"]})
        seen = set()
        for row in k2["cases"]:
            case_id = row.get("case_id")
            if not isinstance(case_id, str) or not case_id or case_id in seen:
                raise TruthError("K2 reference membership requires unique case IDs")
            seen.add(case_id)
            add(row.get("expected_doi"), {"kind": "K2_reference", "k2_version": k2["version"],
                                          "case_id": case_id, "snapshot_id": row["snapshot_id"]})
    if selection is not None:
        papers = selection.get("papers")
        if (selection.get("schema_version") not in {1, 2} or not isinstance(papers, list)
                or selection.get("selection_sha256") != fingerprint(papers)):
            raise TruthError("K0 selection fingerprint differs from its observed papers")
        inputs.append({"kind": "K0_selection_metadata", "content_sha256": fingerprint(selection),
                       "selection_sha256": selection["selection_sha256"]})
        seen = set()
        for paper in papers:
            identifier = paper.get("id")
            if not isinstance(identifier, str) or not identifier or identifier in seen:
                raise TruthError("K0 selection requires unique arXiv identities")
            seen.add(identifier)
            tiers = paper.get("tiers")
            if not isinstance(tiers, list) or not tiers or set(tiers) - {"eval", "scale"}:
                raise TruthError("K0 selected membership requires an eval or scale tier")
            add(paper.get("doi"), {"kind": "K0_selected_metadata", "arxiv_id": identifier,
                                   "tiers": sorted(set(tiers)), "metadata_sha256": fingerprint(paper),
                                   "selection_sha256": selection["selection_sha256"]})
    result = {"schema_version": 1, "kind": "K4_work_universe", "inputs": inputs,
              "works": [{"doi": key, "origins": sorted(records[key], key=canonical)} for key in sorted(records)],
              "coverage": {**dict(sorted(counts.items())), "unique_dois": len(records)}}
    result["content_sha256"] = fingerprint(result)
    return result


def validate_universe(membership):
    if membership.get("schema_version") != 1 or membership.get("kind") != "K4_work_universe":
        raise TruthError("Unsupported K4 work universe")
    content = {key: value for key, value in membership.items() if key != "content_sha256"}
    if membership.get("content_sha256") != fingerprint(content):
        raise TruthError("K4 work universe fingerprint mismatch")
    works = membership.get("works")
    if not isinstance(works, list) or len(works) > MAX_WORKS:
        raise TruthError("K4 work universe exceeds its work bound")
    identifiers = [row.get("doi") for row in works]
    if any(doi(value) is None or doi(value) != value for value in identifiers) or len(set(identifiers)) != len(identifiers):
        raise TruthError("K4 work universe requires unique normalized DOIs")
    return set(identifiers)


def request_plan(membership, batch_size=25, offset=0, limit=100):
    identifiers = sorted(validate_universe(membership))
    if (type(batch_size) is not int or not 1 <= batch_size <= 100 or type(offset) is not int or offset < 0
            or type(limit) is not int or not 1 <= limit <= 100):
        raise TruthError("K4 lookup plans require bounded batches and request windows")
    regular = [value for value in identifiers if not any(char in value for char in "|,")]
    special = [value for value in identifiers if any(char in value for char in "|,")]
    requests = [{"provider": "openalex", "dois": regular[start:start + batch_size]}
                for start in range(0, len(regular), batch_size)]
    requests += [{"provider": "openalex", "dois": [value]} for value in special]
    return {"schema_version": 1, "universe_sha256": membership["content_sha256"], "request_offset": offset,
            "total_requests": len(requests), "requests": requests[offset:offset + limit]}


def completeness(record, count):
    truncated = record.get("is_authors_truncated")
    total = record.get("authors_count")
    total_valid = type(total) is int and total >= 0
    if (truncated is not None and type(truncated) is not bool) or (total is not None and not total_valid):
        status = "invalid_completeness_metadata"
    elif total_valid and (total < count or (total > count and truncated is False)):
        status = "contradictory_completeness_metadata"
    elif truncated is True or (total_valid and total > count):
        status = "observed_truncated"
    elif truncated is False or (total_valid and total == count):
        status = "reported_complete"
    else:
        status = "possible_provider_cap" if count >= 100 else "unknown"
    return {"status": status, "observed_slots": count,
            "reported_authors_count": observed(total), "reported_is_authors_truncated": observed(truncated)}


def build_k4(membership, snapshots, built_at):
    identifiers = validate_universe(membership)
    for source in membership["inputs"]:
        if source["kind"] == "K2" and timestamp(source["built_at"]) > timestamp(built_at):
            raise TruthError("K4 cannot predate its K2 membership")
    relevant = [row for row in snapshots if row["receipt"]["provider"] == "openalex"]
    if not relevant:
        raise TruthError("K4 requires frozen OpenAlex responses, including observed missing-work responses")
    requested = set()
    for snapshot in relevant:
        current = set(snapshot["receipt"]["requested_dois"])
        if current - identifiers or current & requested:
            raise TruthError("K4 requests must cover universe works once without foreign or refreshed identities")
        requested.update(current)
    records, sources = openalex_records(relevant, built_at)
    slots, works, duplicate_orcids = [], [], []
    profile_labels = collections.defaultdict(lambda: collections.defaultdict(list))
    for identifier in sorted(identifiers):
        if identifier not in records:
            works.append({"doi": identifier, "status": "missing_provider_record" if identifier in requested else "not_requested"})
            continue
        record, snapshot, pointer = records[identifier]
        authorships = record.get("authorships")
        if not isinstance(authorships, list):
            works.append({"doi": identifier, "status": "missing_authorships" if authorships is None else "invalid_authorships",
                          "snapshot_id": snapshot["snapshot_id"], "json_pointer": pointer})
            continue
        if len(slots) + len(authorships) > MAX_SLOTS:
            raise TruthError("K4 authorship input exceeds its slot bound")
        works.append({"doi": identifier, "status": "observed_authorships", "snapshot_id": snapshot["snapshot_id"],
                      "json_pointer": pointer, "completeness": completeness(record, len(authorships))})
        work_orcids = collections.defaultdict(list)
        for index, authorship in enumerate(authorships):
            row = authorship if isinstance(authorship, dict) else {}
            author = row.get("author") if isinstance(row.get("author"), dict) else {}
            raw, profile = row.get("raw_orcid"), author.get("orcid")
            raw_id, profile_id = orcid(raw), orcid(profile)
            name = row.get("raw_author_name")
            reasons = []
            if not isinstance(authorship, dict):
                reasons.append("malformed_authorship")
            if problem := name_problem(name):
                reasons.append(problem)
            if raw_id is None:
                reasons.append(("profile_only_orcid" if profile_id else "missing_raw_orcid") if missing(raw) else "invalid_raw_orcid")
            if not missing(profile) and profile_id is None:
                reasons.append("invalid_profile_orcid")
            if raw_id and profile_id and raw_id != profile_id:
                reasons.append("conflicting_raw_profile_orcids")
            mention_id = "K4-mention-" + fingerprint([identifier, index])
            slot = {"mention_id": mention_id, "input": {"raw_name": observed(name), "work_doi": identifier, "authorship_index": index},
                    "observed": {"raw_orcid": observed(raw), "profile_orcid": observed(profile),
                                 "provider_author_id": observed(author.get("id")), "author_position": observed(row.get("author_position"))},
                    "provenance": {"snapshot_id": snapshot["snapshot_id"], "json_pointer": f"{pointer}/authorships/{index}",
                                   "retrieved_at": snapshot["receipt"]["retrieved_at"], "authorship_sha256": fingerprint(authorship)},
                    "expected_orcid": raw_id, "exclusion_reasons": reasons}
            slots.append(slot)
            if raw_id:
                work_orcids[raw_id].append(slot)
                provider_id = author.get("id")
                if isinstance(provider_id, str) and re.fullmatch(r"https://openalex\.org/A[0-9]+", provider_id):
                    profile_labels[provider_id][raw_id].append(mention_id)
        for raw_id, shared in sorted(work_orcids.items()):
            if len(shared) > 1:
                for slot in shared:
                    slot["exclusion_reasons"].append("duplicate_orcid_across_coauthors")
                duplicate_orcids.append({"work_doi": identifier, "orcid": raw_id,
                                         "mention_ids": [slot["mention_id"] for slot in shared]})
    clusters, exclusions = collections.defaultdict(list), collections.Counter()
    for slot in slots:
        slot["exclusion_reasons"] = sorted(slot["exclusion_reasons"])
        slot["eligible"] = not slot["exclusion_reasons"]
        if slot["eligible"]:
            clusters[slot["expected_orcid"]].append(slot["mention_id"])
        else:
            exclusions.update(slot["exclusion_reasons"])
            slot["expected_orcid"] = None
    conflicts = [{"provider_author_id": key,
                  "raw_orcids": [{"orcid": label, "mention_ids": sorted(values)} for label, values in sorted(labels.items())]}
                 for key, labels in sorted(profile_labels.items()) if len(labels) > 1]
    eligible = sum(map(len, clusters.values()))
    return finish("K4", built_at, {"label_policy": "source-deposited-raw-orcid-v1", "universe": membership,
        "sources": sources, "mentions": slots, "works": works,
        "clusters": [{"expected_orcid": key, "mention_ids": sorted(clusters[key])} for key in sorted(clusters)],
        "conflicts": {"duplicate_orcid_across_coauthors": duplicate_orcids, "provider_profile_multiple_raw_orcids": conflicts},
        "coverage": {"universe_works": len(identifiers), "requested_works": len(requested), "returned_works": len(records),
                     "work_status": dict(sorted(collections.Counter(row["status"] for row in works).items())),
                     "completeness": dict(sorted(collections.Counter(row["completeness"]["status"] for row in works if "completeness" in row).items())),
                     "observed_slots": len(slots), "eligible_mentions": eligible, "excluded_mentions": len(slots) - eligible,
                     "exclusion_reasons": dict(sorted(exclusions.items())), "clusters": len(clusters),
                     "singleton_clusters": sum(len(values) == 1 for values in clusters.values()),
                     "multi_work_clusters": sum(len(values) > 1 for values in clusters.values()),
                     "same_person_pairs": sum(len(values) * (len(values) - 1) // 2 for values in clusters.values()),
                     "provider_profile_conflicts": len(conflicts), "label_status": "available" if eligible else "no_eligible_labels"}})


def features(labels):
    """Allowlist projection: neither resolved profile identities nor answer labels are inputs."""
    if labels.get("schema_version") != 1 or labels.get("truth_set") != "K4":
        raise TruthError("Person feature projection requires K4")
    rows, seen = [], set()
    for slot in labels["mentions"]:
        if slot.get("eligible") is not True:
            continue
        value = slot["input"]
        identifier, index = value["work_doi"], value["authorship_index"]
        identity = "K4-mention-" + fingerprint([identifier, index])
        if (identity != slot["mention_id"] or identity in seen or doi(identifier) is None or doi(identifier) != identifier
                or type(index) is not int or index < 0 or name_problem(value["raw_name"])):
            raise TruthError("K4 contains an invalid or leaking person feature")
        seen.add(identity)
        rows.append({"mention_id": identity, "raw_name": value["raw_name"], "work_doi": identifier, "authorship_index": index})
    return {"schema_version": 1, "kind": "person_clustering_features", "mentions": sorted(rows, key=lambda row: row["mention_id"])}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    membership = sub.add_parser("universe")
    membership.add_argument("--k2", type=Path)
    membership.add_argument("--selection", type=Path)
    plan = sub.add_parser("plan")
    plan.add_argument("--universe", type=Path, required=True)
    plan.add_argument("--batch-size", type=int, default=25)
    plan.add_argument("--request-offset", type=int, default=0)
    plan.add_argument("--request-limit", type=int, default=100)
    build = sub.add_parser("build")
    build.add_argument("--universe", type=Path, required=True)
    build.add_argument("--frozen-root", type=Path, required=True)
    build.add_argument("--manifest", type=Path, required=True)
    build.add_argument("--built-at", required=True)
    projection = sub.add_parser("features")
    projection.add_argument("--k4", type=Path, required=True)
    for command in (membership, plan, build, projection):
        command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "universe":
            result = universe(read_json(args.k2) if args.k2 else None, read_json(args.selection) if args.selection else None)
        elif args.command == "plan":
            result = request_plan(read_json(args.universe), args.batch_size, args.request_offset, args.request_limit)
        elif args.command == "build":
            result = build_k4(read_json(args.universe), load_snapshots(args.frozen_root, read_json(args.manifest)), args.built_at)
        else:
            labels = read_json(args.k4)
            checked_truth(labels, "K4")
            result = features(labels)
        write_immutable(args.output, (canonical(result) + "\n").encode())
        print(canonical({key: result[key] for key in ("kind", "truth_set", "version", "coverage", "total_requests") if key in result}))
        return 0
    except (OSError, KeyError, TypeError, ValueError) as error:
        print(f"person truth: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
