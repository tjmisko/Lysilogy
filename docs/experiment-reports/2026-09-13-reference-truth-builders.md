# Reference truth builders — 2026-09-13 partial checkpoint

Issue #71 has working offline K2/K5 builders, an explicit provider-freeze command through E7.1,
and deterministic K7 withholding mechanics. **The issue remains incomplete:** genuine K2/K5
provider snapshots and the mapped scale/local K7 graph have not been built. This report records
implementation validation; it does not establish any truth-dependent hard gate or objective.

Provider snapshots preserve the original fetch time, including cache hits, and the exact frozen
serialized sanitized JSON bytes. Immutable request records bind the requested DOI lookup to
one content-addressed response receipt. Offline rebuilds verify hashes, timestamps, fixed
endpoints and requested/returned identities without consulting mutable provider TTLs. Raw
responses stay outside the repository. The transport/proxy policy is unchanged.

K2 separates deposited input from expected DOI labels and flags identifiers already visible in
the input. DOI-only deposits remain labels with unavailable resolver inputs; references without
valid deposited DOIs are counted and excluded. O9 fields are nullable strings independently
deposited alongside actual unstructured text. Structured fields rendered into input text cannot
establish bibliography accuracy. A volume/container title alone is not promoted to the cited
work's title. K2 needs no PaperId, WorkId, local ingest or fabricated PDF.

K5 samples exactly 200 unique eligible K2 references, stratifying the referenced work's frozen
OpenAlex field and publication decade. Seeded order and round-robin allocation handle sparse
cells. Insufficient coverage fails instead of shrinking the sample. Explicit consistent OA
labels are frozen; absent or conflicting status stays unknown with coverage counts.

K7 normalizes explicit aliases and removes a held-out paper's outgoing edges across every
bibliography/provider view, including incoming-query duplicates. Adjacency and degrees are
recomputed afterward. Expected targets and raw evidence locations stay outside ranker features.
Compact deterministic fold records reference one frozen base graph. Publication still needs
independently verified mapped candidates and citation provenance from the actual 10k scale and
local graphs; fixture graphs provide no read-next recall measurement.

## Validation and measurements

Relevant before evaluations ran at clean source `2c3a557`; final integrated checks ran at
`83b2244`, including main's #91 endpoint fix and progress checkpoint `e221bb6` through ordinary
merges. The first resolution evaluation was clean in each run. Later suites truthfully record
generated-scorecard changes with `dirty=true`. No targets or baselines changed.

| Check | Result |
| --- | --- |
| Formatting; strict all-target/all-feature Clippy | Pass |
| All-target Rust tests | 309 pass |
| New Rust provenance/request tests | 4 pass |
| New offline Python truth tests | 33 pass |
| G5 isolated Rust / Python / Node tests | 309 / 120 / 85 pass |
| O30 provider-budget violations | 0 / 10,000 requests |
| Resolution, acquisition, read-next, citations, bibliography `eval … --check` | Pass before and after; truth-owned values unavailable |
| Genuine K2 / K5 / K7 truth publication | Pending |

The Python G5 total includes seven separately merged source-endpoint tests and 33 new truth
tests. Tests verify snapshot identity and immutable reuse, swapped-valid-receipt rejection,
missing/duplicate provider identities, aggregate input bounds, label/input separation, sampling
coverage and unknown OA status, and all-view holdout leakage guards. Tests run without network
or models; G5 confirms namespace isolation and absent model CLIs.

The [retained evidence](../../eval/evidence/reference-truth-builders.json) contains exact before
and after eval results, tested implementation hashes, commands, timing/log hashes, full isolated
test outputs and the bounded access failure. Passing eval commands while metrics are unavailable
does not satisfy system acceptance or establish a scorecard target.

## Live boundary and remaining work

One authorized Crossref lookup for DOI `10.3842/sigma.2022.071` through existing GraphHttp failed
as `Unavailable` in 0.002 seconds before a provider response. The root agent separately confirmed
that the runtime host allowlist rejects `api.crossref.org` and `api.openalex.org`. The exact
permission request remains unanswered; no missing credential is confirmed. No access bypass or
further provider request was attempted. Model calls and cost are zero; provider billing cost is
unknown. This probe produced no truth label.

After provider access works through E7.1, freeze genuine Crossref records with local/K0 seed
coverage, build and inspect K2, then freeze OpenAlex status and build the actual 200-reference
K5 sample. Complete K7 only after #25 bibliography extraction and the actual 10k scale/local
mapping are available, binding source hashes and measuring holdout coverage. Keep the PR draft
until those truth requirements and independent review are complete.
