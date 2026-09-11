# Citation graph sources for Lysilogy

Checked 2026-09-11. These are candidate integrations; the graph adapters are not yet implemented.

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

## Proposed context pipeline

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
