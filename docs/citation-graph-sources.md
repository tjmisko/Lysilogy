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
