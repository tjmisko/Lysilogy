# Citation graph sources for Lysilogy

Implemented and checked against provider documentation on 2026-09-11. The adapters share the `CitationProvider` interface in `src/citation_graph/`; fixture tests cover their response contracts. Live provider requests could not be exercised in the development sandbox because its network allowlist blocks those hosts.

Use OpenAlex to discover a paper's citation neighborhood, Semantic Scholar to retrieve citation
passages where available, and the citing document to verify the relationship. A database edge says
that A cites B. It does not establish that A supports B, improves on B, or was influenced by B in a
particular way.

| Source | What it provides | Suggested use |
| --- | --- | --- |
| [OpenAlex](https://help.openalex.org/how-to/api-recipes/) | `referenced_works` for outgoing links; `works?filter=cites:W…` for incoming links, plus work and author metadata | Broad discovery of predecessors and citing papers. `related_works` represents similarity, not citations. |
| [Semantic Scholar](https://api.semanticscholar.org/api-docs/snippets) | `/graph/v1/paper/{id}/references` and `/citations`; optional `contexts`, `intents`, and `isInfluential` | Retrieve candidate citation passages and rank papers for inspection. |
| [OpenCitations Index](https://api.opencitations.net/index/v2) | `/references/doi:{doi}` and `/citations/doi:{doi}`; also PMID/OMID identifiers and persistent citation identifiers | Cross-check citation edges and retain their provenance. |
| [Crossref](https://www.crossref.org/documentation/retrieve-metadata/rest-api/) | Publisher-deposited DOI metadata, including references when deposited | Resolve bibliography entries and obtain publisher-supplied metadata. Use a graph service for lists of incoming citations. |

OpenAlex currently allows basic requests without a key and offers a larger budget with a free key.
Semantic Scholar allows many unauthenticated requests but recommends a key; shared unauthenticated
capacity can be throttled. OpenCitations recommends an access token for applications. Crossref's
public REST API requires no registration. See [OpenAlex authentication](https://help.openalex.org/api/authentication/),
[Semantic Scholar access](https://www.semanticscholar.org/product/api), and the provider documentation
above before choosing production request limits.

## What counts as ground truth

- **Outgoing citation:** the selected version of the paper's bibliography, linked to its in-text
  citation marker. Retain the original reference string and source page alongside resolved IDs.
- **Incoming citation:** use graph indexes to discover the citing paper, then verify the target in
  that paper's bibliography or publisher-supplied reference list. A missing database edge should
  mean unknown, not that no citation exists.
- **Why it is cited:** inspect the actual surrounding paragraph, sometimes multiple paragraphs.
  Store the passage and location separately from an interpretation such as extension, criticism,
  comparison, or background. Semantic Scholar's influential-citation label is produced by a
  [machine-learning model](https://www.semanticscholar.org/faq/influential-citations), so it can guide
  retrieval but should not serve as the ground-truth label.
- **Version identity:** retain DOI, arXiv ID/version, and provider IDs. Link preprints and published
  versions without silently treating their dates or bibliographies as identical.

## Further context verification

1. Extract local references; resolve the target and bibliography using DOI/arXiv IDs, with
   title/author matching as a fallback whose confidence is recorded.
2. Fetch and paginate outgoing/incoming edges; deduplicate across providers while retaining each
   provider's record and retrieval time. Two indexes may share an upstream source, so agreement is
   not necessarily independent corroboration.
3. Retrieve the important citing/cited documents and verify bibliography entries and citation
   passages. Record unavailable full text and unmatched references as gaps.
4. Freeze the evidence set. Give the context writer these records and require every before/after
   bullet to cite the exact evidence it uses. Keep the independent review of the resulting claims.

Useful metrics are resolved local references / extracted references, document-verified edges /
inspected edges, verified citation passages / inspected citing papers, and evidence-supported
context claims / proposed claims. Report provider-specific incoming counts with their retrieval
dates. Do not claim complete incoming-citation recall: the true universe of citing documents is
unknown.


## Fetching a paper's graph

Fetches are explicit. Ordinary reading, heuristic analysis, and context refresh never start new
citation API requests. Identify the exact paper/version with a DOI, arXiv ID, or provider ID; the
adapter deliberately does not guess a target from a title. The response includes returned target
metadata so identity can be checked. Binding an identifier to a local paper is user-supplied; it is
not document verification.

```sh
./target/release/lysilogy --library local-articles --data .lysilogy citation-graph \
  "paper ID or title fragment" --identifier doi:10.1038/nphys1170 --limit 100

# Restrict the discovery work; provider/direction flags accept comma-separated values.
./target/release/lysilogy --library local-articles --data .lysilogy citation-graph \
  "paper ID or title fragment" --identifier arxiv:1805.00899 \
  --provider semantic-scholar --direction references,citations
```

`POST /api/papers/{id}/citation-graph` accepts:

```json
{
  "identifier": "doi:10.1038/nphys1170",
  "providers": ["openalex", "semantic_scholar", "opencitations", "crossref"],
  "directions": ["references", "citations"],
  "limit": 100
}
```

Omitting providers/directions selects all four/both. The edge limit is 1–500 **per provider and
direction**. `GET /api/papers/{id}/citation-graph` returns the saved snapshot, or JSON `null` when
none exists; GET never contacts a provider. POST replaces `citation-graph.json` in that paper's
artifact directory. All provider failures remain visible in the returned and stored snapshot;
partial failures do not discard already retrieved edges.

Run `refresh-context <paper> --provider codex --force` after fetching to rebuild context with the
snapshot. Up to 12 edges per provider/direction seed the evidence research prompt. The researcher
must still inspect primary passages; neither graph edges nor Semantic Scholar's machine labels
are admitted directly into the context writer's frozen evidence dossier. Fetching a graph does
not regenerate analysis, change author metadata, or initiate model calls.

## Provider implementations and limits

| Adapter | Exact identifiers | Retrieval behavior |
| --- | --- | --- |
| OpenAlex | `doi:`, `pmid:`, `openalex:W…` | Resolves the target; obtains outgoing IDs and enriches metadata in batches of 100. Incoming edges use cursor pagination, newest publication first. |
| Semantic Scholar | `doi:`, `arxiv:`, `pmid:`, `s2:` | Resolves target metadata; paginates references/citations with offsets, preserving available passages, intents, influential labels, and unresolved records. |
| OpenCitations | `doi:`, `pmid:`, `omid:` | Uses Index v2; retains all PIDs, OCI, and raw edge provenance including source-index labels. The API has no pagination contract; one response is bounded by the byte cap and results are capped locally. Metadata such as author names is absent from this index endpoint. |
| Crossref | `doi:` | Reads deposited references from DOI metadata, preserving unresolved/raw entries and full supplied author names. An absent deposited list is `unavailable`, distinct from an explicitly empty list. Incoming citation lists return `unsupported` without a request. |

Raw DOIs and `https://doi.org/…` URLs are accepted. DOI suffixes containing reserved URL characters
are encoded as path data. arXiv versions and multiple provider identifiers are preserved; versions
are never silently merged. Records stay grouped by provider/direction rather than flattening them
into supposedly independent votes. Within paginated OpenAlex/Semantic Scholar responses, repeated
provider IDs are deduplicated. A future cross-provider merge must preserve aliases and every edge's
provenance.

Each report contains `retrieved_at`, `target`, `requests` (without credentials), `edges`, optional
`total_reported`, `coverage`, and optional `failure`. Coverage is `complete`, `capped`, `partial`, or
`unavailable`; **complete means only that provider's available response**, never all papers in the
literature. Unsupported, missing, denied, rate-limited, malformed, and interrupted responses are
explicit errors rather than empty successful graphs. Counts are not harmonized across indexes.

Requests use fixed HTTPS provider hosts, no redirects, a 25-second timeout, an 8 MiB response cap,
and at most 20 pagination requests per direction. Each provider has a shared 1.1-second spacing
slot. `Retry-After` is reported and retained as a provider-specific cooldown; requests during long
cooldowns fail immediately, leaving other providers available. Requests are not retried
automatically. This avoids hidden loops or long sleeps when shared public API capacity is limited.

Optional process environment configuration (no credential files are read):

- `LYSILOGY_OPENALEX_API_KEY`: OpenAlex API query credential.
- `LYSILOGY_SEMANTIC_SCHOLAR_API_KEY`: Semantic Scholar `x-api-key` header.
- `LYSILOGY_OPENCITATIONS_TOKEN`: OpenCitations `authorization` header.
- `LYSILOGY_CITATION_MAILTO`: contact address for Crossref/OpenAlex polite requests.

Credentials are attached only at transport time, omitted from serialized request provenance, and
redacted from errors/debug output. Restart the backend after changing these settings. No key is
required by the adapter; provider-side access limits and policies still apply.

## Shared response cache and batch budgets

Explicit citation requests now use the same provider services intended for resolution and
acquisition. Successful JSON objects/arrays are cached for seven days under
`~/.cache/lysilogy/providers/responses/<provider>/<key>.json`. This storage is separate from
both the library and its data root; local-library and corpus processes share it. Construction
is lazy and ordinary reading creates no provider files. `GraphHttp::from_environment_with_storage`
accepts an explicit shared storage location and per-provider `BudgetPolicy` values for services
and isolated fixtures. Every service using the same provider credentials must use the same root.

Keys hash the provider and canonical public endpoint/path/query before authentication. Public
query ordering is canonicalized while repeated parameter order remains significant. Credentials
and contact parameters are removed from keys. Credential fields and configured secret echoes
(including nested strings, object keys and mixed-case/double percent encoding) are removed from
responses before return or serialization. Cache reads verify schema, key, SHA-256, creation and
expiry timestamps, object/array shape, and the 8 MiB payload cap. Hash-corrupt or expired entries
are misses. Generated storage paths reject symlinks in any ancestor, and writes use an atomic
replacement followed by file/directory synchronization.

`created_ms` and `expires_ms` in each cache entry describe the provider fetch. The graph report's
`retrieved_at` remains the time that local report was assembled; a report can reuse provider
metadata fetched up to seven days earlier. A cache hit does not contact a provider or consume an
admission slot, including while that provider is in cooldown. Concurrent cache fills can
conservatively consume an unused admission slot when a lookup is filled while awaiting its lease.
Failures and credential errors are not cached. A failed cache write preserves the successful
response and emits a generic warning; unavailable budget storage fails closed before a request.

Each provider defaults to 1.1-second spacing, one active connection, and 50 admissions in a
60-second fixed window. A window restarts on the first admission check after the previous window expires.
The quota and window are explicit local limits, not estimates of paid account credits. Configured
spacing cannot be less than 1.1 seconds. A short spacing wait is allowed; exhausted windows and
long server cooldowns return `rate_limited` with a retry interval. Separate processes coordinate
through a provider file lock held until the response body finishes. Admissions and the largest
`Retry-After` deadline persist before the request or failure returns, so restart does not reset
the allowance or shorten a cooldown. Rate state contains only timestamps and counts.

Policies checked on 2026-09-12: [Semantic Scholar](https://webflow.semanticscholar.org/product/api)
documents an introductory keyed allowance of one request per second;
[Crossref](https://www.crossref.org/documentation/retrieve-metadata/rest-api/access-and-authentication/)
documents pool-specific rate and concurrency headers, including one concurrent public request;
[OpenAlex](https://help.openalex.org/api/authentication/) applies both request-rate and daily
credit limits; [OpenCitations](https://opencitations.net/querying/) recommends a token in the
authorization header. The local limits do not claim to exhaust any provider's account allowance.
No automatic retry or paid request purchase is added.

Measure the actual admission state machine over the recorded 10,000-reference fixture:

```sh
python3 scripts/eval/provider-budgets.py
cargo run -- eval scale --check
```

The Rust trace covers all four providers, exhausted windows, injected server cooldowns and
serialized state reloads. An independent Python audit checks observed grants against spacing,
window quotas, cooldown deadlines, duplicate admission and complete case coverage. It writes
`eval/inputs/scale/provider-budgets.json` and a hashed observations file; each scale collector
keeps its own input and dependencies. Four offline audit tests include deliberately invalid
traces to demonstrate that violations are detected. No network or model is called.
