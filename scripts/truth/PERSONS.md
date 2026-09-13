# K4 person silver labels

`person_truth.py` is an offline builder. It imports the frozen-response validation and immutable
storage from E8.4 (#71 / draft PR #93); that dependency must merge first. No actual K4 is published
by this tooling change. OpenAlex/Crossref access remains blocked by the runtime host allowlist.
The tests use invented small records and cannot establish G1 or O13.

## Evidence and label policy

OpenAlex distinguishes source-deposited `authorship.raw_orcid` from `author.orcid`, the ORCID on
the resolved author profile that OpenAlex propagates to associated works. K4 requires the former
to avoid evaluating a new resolver against OpenAlex's previous clustering. Profile-only slots are
counted and excluded. See the official [ORCID semantics](https://help.openalex.org/data/authors/orcid/).

A mention is the canonical work DOI plus zero-based `authorships` array index. `author_position`
is a first/middle/last role, so two middle authors never share a position. The original
`raw_author_name` supplies the name input; resolved profile display names do not. Official
[authorship documentation](https://help.openalex.org/data/authorships/) describes these fields and
the provider's author-list cap. Explicit truncation/count fields and unknown completeness are
reported separately; unseen authors receive no labels.

The validator accepts the hyphenated bare ORCID and HTTP/HTTPS `orcid.org` URI forms, optionally
surrounded by ASCII spaces. It emits the canonical HTTPS URI, validates ASCII shape and MOD 11-2
check digit (including capital X), and preserves the original observed string separately. URI
credentials, query/fragment, port, trailing slash, compact digits, other hosts and control
characters are rejected. A valid checksum does not prove registry assignment. The official
[ORCID identifier specification](https://support.orcid.org/hc/en-us/articles/360006897674-Structure-of-the-ORCID-Identifier)
defines its canonical form and check digit.

Valid raw/profile disagreement excludes the slot. Repeating one raw ORCID on several coauthor
positions excludes every affected slot, including when another slot also has a different
problem. Invalid raw IDs, invalid profile IDs, missing raw IDs, profile-only evidence, malformed
names and contradictions have separate reason counts; reasons can overlap, while each excluded
mention is counted once. Names containing an ORCID-like identifier or explicit OpenAlex author
URL/prefix are excluded from evaluation inputs, including compatibility-width and control
obfuscation. Ordinary names starting with A are not treated as provider IDs.

Across works, a provider profile associated with multiple valid raw ORCIDs is reported as a
profile-clustering conflict. The profile ID neither merges these people nor invalidates otherwise
consistent raw labels. Multiple provider profiles with the same valid raw ORCID cluster together.
Same-name mentions with distinct ORCIDs stay distinct. These are silver labels with recorded
source limitations, not independently audited person identities.

## Offline workflow

All paths below point to dedicated external cache directories. Raw sanitized provider response
bytes and their original fetch timestamps remain in the E8.4 content-addressed freeze store.
Derived labels retain snapshot IDs, JSON pointers and source-authorship hashes. They never copy
abstracts or complete provider records into the repository. Immutable output writes refuse to
replace a different existing version.

```sh
python3 scripts/truth/person_truth.py universe \
  --k2 ~/.cache/lysilogy/reference-truth/K2.json \
  --selection ~/Corpora/arxiv/selection.json \
  --output ~/.cache/lysilogy/person-truth/universe-v1.json
python3 scripts/truth/person_truth.py plan \
  --universe ~/.cache/lysilogy/person-truth/universe-v1.json \
  --output ~/.cache/lysilogy/person-truth/requests-v1-000.json
```

Either K2 or K0 can be supplied while the other is unavailable. K2 includes both citing and
reference work DOIs. K0 contributes only selected DOI metadata membership; this does not assert
that PDFs/sources were downloaded or mapped. Each membership origin and input content fingerprint
is retained. Missing and invalid DOI counts are explicit; no title-based identity is invented.
Rebuild the universe with both actual inputs before final K4 acceptance.

For the first live build, freeze Crossref and derive genuine K2 first. Then form the combined
K2+K0 universe and freeze its OpenAlex plan once. Reuse those same OpenAlex snapshots for both
K4 and K5, rather than independently generating overlapping OA and person lookup batches. The
current K0-only 73-request plan is a preparatory checkpoint, not this final combined plan.

The request plan uses the E8.4 `freeze_reference.py freeze` command and Rust GraphHttp helper,
with the existing cache, budget and fixed provider endpoint. The plan alone makes no network
call. Live freezing remains a separate explicit operation after host access is available.
Each plan holds at most 100 requests of at most 100 DOIs (25 by default); `--request-offset` and
`--request-limit` divide a larger universe into bounded windows. Filter-delimiter DOIs get
singleton requests. Reuse existing frozen DOI receipts instead of refetching them. A combined
manifest must contain one receipt per requested DOI; overlapping/refreshed responses are refused.

```sh
python3 scripts/truth/person_truth.py build \
  --universe ~/.cache/lysilogy/person-truth/universe-v1.json \
  --frozen-root ~/.cache/lysilogy/reference-truth \
  --manifest ~/.cache/lysilogy/person-truth/frozen-v1.json \
  --built-at 2026-09-13T00:00:00Z \
  --output ~/.cache/lysilogy/person-truth/K4-v1.json
python3 scripts/truth/person_truth.py features \
  --k4 ~/.cache/lysilogy/person-truth/K4-v1.json \
  --output ~/.cache/lysilogy/person-truth/features-v1.json
```

Use the actual build timestamp, after all source fetch/freeze and K2 build times. The loader
requires exact source bytes, endpoints, requested/returned DOI identity, and immutable dates.
The `features` file boundary validates K4's content-derived version before projecting. Its pure
projection function remains independent of label values. `features` emits only opaque mention
ID, raw name, work DOI and array index. Expected ORCIDs,
observed ORCIDs, resolved profile IDs, source pointers and clusters stay in evaluator-only truth.
Consumers must use this projection rather than giving the complete K4 document to a resolver.

Coverage includes requested/returned/unrequested works, missing bylines, observed slots,
exclusions, singleton and multi-work clusters, same-person pairs, and provider-profile conflicts.
Zero labels retain their observed coverage but cannot establish a metric; neither can isolated
fixture success. No collector input or scorecard truth is registered automatically. Before
publication, check genuine K2/K0 membership, both origin coverages, cluster diversity, frozen
source provenance and resolver feature separation, then run the actual G1/O13 collector.

The shared reader bounds individual provider bodies to 8 MiB and aggregates to 128 MiB. K4 also
bounds unique work membership to 200,000, observed slots to 1,000,000, and names to 1,024 characters.
Work/slot overflow fails rather than silently truncating; oversize names are excluded with a hash.
