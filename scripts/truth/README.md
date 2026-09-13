# Reference and acquisition truth builders

`reference_truth.py` derives K2 and K5 entirely offline from immutable provider snapshots.
`freeze_reference.py freeze` is the explicit live boundary. It invokes the compiled
`freeze_reference_provider` example, which uses the ordinary E7.1 GraphHttp cache, shared budgets,
credential redaction and fixed HTTPS endpoints. It introduces no proxy or network-policy bypass.

Raw sanitized provider payloads live only in a dedicated directory beneath
`~/.cache/lysilogy/` or `~/Corpora/`; no raw provider response, abstract, PDF or LaTeX source is
committed. A receipt preserves the original provider-fetch timestamp (including cache hits),
freeze time, exact response-byte hash, public request URL, requested/missing DOIs, cache-hit flag,
and elapsed time. Money cost is unknown unless reported, rather than inferred from success;
model calls/cost are zero. No mutable cache TTL is consulted during an offline rebuild.

Immutable request records point to content-addressed receipts and response bytes. A successful
frozen request is never refreshed silently. A failed request leaves earlier completed snapshots
available for resume. Use a new external snapshot directory for a deliberate later upstream build.
The loader verifies bytes, timestamps, fixed endpoint and requested/returned DOI identities;
foreign or duplicate OpenAlex results and swapped Crossref records fail. Missing OpenAlex
results remain explicit missing coverage.

## K2 contract

K2 contains `works` (citing bibliographic metadata), `cases` (deposited reference labels), `sources`
and coverage. Source records retain response hashes and retrieval dates; per-case JSON pointers
and deposited-entry hashes locate the original reference in the external response. It has no
PaperId, WorkId, fabricated local copy or local-ingest dependency. The SQLite G4 collector can
admit these genuine identifiers into its own isolated canonical state through the store API.

Each case separates `input` from `expected_doi`. Input is either actual deposited `unstructured`
text or a documented concatenation of deposited bibliographic fields; the expected DOI is never
inserted into that text. A deposited reference with only a DOI remains an identifier label with
`eligible_resolution=false`. References without valid deposited DOIs are excluded and counted.
If the DOI already occurs in the publisher's input text, `expected_identifier_in_input=true`
records that easier stratum; resolution collectors must report its coverage separately.

`field_labels` separately exposes nullable string `title`, `first_author`, and `year` labels
from deposited reference fields. Crossref defines its citation `author` field as the
[first author](https://www.crossref.org/documentation/schema-library/markup-guide-metadata-segments/references/).
Numeric deposited years become strings; valid suffixes such as `2020a` remain intact.
The title label uses `article-title`; a `volume-title` alone may name the container of a chapter
and therefore stays an input field without being promoted to a known cited-work title.
Only cases with actual unstructured deposited text have `eligible_bibliography=true`; a rendered
structured input cannot establish field accuracy by repeating its own labels. Unknown labels are
excluded individually with counts, while missing predictions for known labels count wrong.

The seed planner uses DOI-bearing observed arXiv metadata, with deterministic diversity across
primary-category families. Those seeds do not establish that PDFs have downloaded or mapped.
Local bibliography provenance can be supplied as `kind` and `local_source_sha256` once extracted
entries are available. Raw abstracts and arbitrary seed fields are refused.

## K5 contract

K5 chooses 200 unique K2 target DOIs with usable deposited inputs and frozen OpenAlex field/year
metadata. It stratifies the **referenced work** by `primary_topic.field.id` and publication decade.
Within cells, a seeded hash orders DOI candidates; a seeded cell order avoids always favoring
lexically early fields. Round-robin allocation redistributes sparse cells and reports every
observed cell's candidate and selected counts. A short sample fails instead of reducing 200.

OA status is frozen from explicit, consistent `open_access.is_oa`/`oa_status` values. Missing,
unrecognized or conflicting evidence is `unknown`, never guessed from a URL or download outcome.
Unknowns remain in the sample and its coverage; the OA denominator includes only explicit open
labels. OpenAlex's taxonomy includes diamond, gold, green, hybrid, bronze and closed.
See [OpenAlex's OA fields](https://help.openalex.org/data/works/open-access/) and
[authentication and batch limits](https://help.openalex.org/api/authentication/), checked
2026-09-13. Crossref deposited references come from its
[public metadata API](https://www.crossref.org/documentation/retrieve-metadata/rest-api/).

## K7 fold mechanics

`read_next_truth.py` provides offline normalization and withholding, pending the actual mapped
10k scale and local bibliography inputs. Candidates must be independently mapped papers from
those cohorts; targets discovered only through a held-out reference do not enlarge the candidate
set. Coverage counts references outside that universe and papers without an eligible reference.
Explicit DOI aliases and arXiv versions identify duplicate views of the same paper. Conflicting
aliases fail rather than guessing a merge from a title or name.

Every edge view is oriented **citing → cited**, including incoming provider queries. Before
constructing fold features, the builder removes every edge whose canonical source is the held-out
paper from all bibliography/provider views. It then recomputes the union, incoming/outgoing
adjacency and degrees. Other papers' edges into the query remain available. Raw provider payloads
and cached features are refused as observation inputs. Evidence locations also stay out of feature
output so the ranker cannot reopen an unfiltered provider record. The future ranking adapter must consume
only `features_for_fold` output, never the retained base graph or the fold's expected targets.

`fold_plan` creates deterministic, seed-ordered compact fold records against one fingerprinted
base graph, avoiding a separate 10k-node graph on disk for every fold. `expected_targets` are
separate from feature output. Self-references are excluded from labels and counted. These mechanics
have generated graph tests, but no actual K7 version or read-next metric exists at this checkpoint.

## Commands and current boundary

Build the transport without network, then create a modest explicit request plan:

```sh
cargo build --offline --example freeze_reference_provider
python3 scripts/truth/freeze_reference.py seed-plan \
  --selection "$HOME/Corpora/arxiv/selection.json" --count 30 \
  --output "$HOME/.cache/lysilogy/reference-truth-seeds.json"
python3 scripts/truth/freeze_reference.py freeze \
  --plan "$HOME/.cache/lysilogy/reference-truth-seeds.json" \
  --output "$HOME/.cache/lysilogy/reference-truth-crossref.json"
```

Use the returned frozen manifest and a fixed timezone-aware `--built-at` to build K2:

```sh
python3 scripts/truth/reference_truth.py \
  --frozen-root "$HOME/.cache/lysilogy/reference-truth" \
  --manifest "$HOME/.cache/lysilogy/reference-truth-crossref.json" \
  --built-at '2026-09-13T12:00:00Z' --output eval/truth/K2-2026-09-13.json \
  k2 --seeds "$HOME/.cache/lysilogy/reference-truth-seeds.json"
python3 scripts/truth/freeze_reference.py oa-plan \
  --k2 eval/truth/K2-2026-09-13.json --batch-size 25 \
  --output "$HOME/.cache/lysilogy/reference-truth-oa-plan.json"
```

Freeze that OA plan in the same way, then run `reference_truth.py ... k5 --k2 <K2 file>` with the
OA manifest. Dates above illustrate syntax; the actual build date must follow every frozen source.
Truth versions contain the build date and content fingerprint. Existing differing output bytes
are never replaced. Per-response bodies/receipts are bounded to 8 MiB; aggregate manifests and
derived truth accept up to 128 MiB. Seed plans use at most 100 citing works per explicit build;
freeze plans use at most 100 requests and batches at most 100 DOIs (25 by default for OA).

At the initial implementation checkpoint, provider host access was unavailable and **no actual
K2/K5 labels had been built**. Offline fixture tests prove builder contracts, not scorecard
precision. No collector or target is changed by these builders. K7 still requires the actual
mapped 10k scale citation graph and local bibliographies; it is not complete at this checkpoint.
