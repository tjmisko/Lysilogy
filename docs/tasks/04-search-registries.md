# 04 · Search registries and the identity-resolution contract

Status: proposed, 2026-10-16. Priority P1.

## Problem

Identity resolution today is hard-wired to OpenAlex, Crossref, arXiv and
Unpaywall, with title matching as the fallback. That is the right default, but
it is not enough:

- A user at a university may want results from their own catalogue — where the
  physical copy is, the call number, whether it is checked out.
- Institutions, labs and private servers have their own indexes that a user
  should be able to attach.
- When the deterministic providers fail, the answer is often still available
  from a web search; there should be very few queries that return nothing
  helpful.

## Goal

A stable, pluggable registry contract: partial semi-structured query in,
search results out, with deterministic registries searched first and a bounded
model web-search fallback. Our providers ship as defaults; users can add their
own.

## The query side

A partial, semi-structured query — not a full citation record. Any subset of:

- title (and subtitle)
- authors (any order)
- year or range
- DOI, arXiv ID, PMID, OpenAlex ID
- venue / journal / conference
- raw citation string, as printed
- local paper context (citing paper, page, key)

Registries must tolerate missing fields, return ranked candidates, and say when
a field is unsupported rather than inventing it.

## The result side

A result is a typed record, not just a link. At minimum the contract must
express these modes:

| Result kind | Meaning |
| --- | --- |
| `metadata` | Identity record: title, authors, year, venue, identifiers |
| `link` | A landing page or DOI URL for this Work |
| `full_text` | A PDF or HTML copy, with access status (open / licensed / unknown) |
| `holding` | A physical copy: library, location, call number, availability, catalogue URL |
| `library_record` | A catalogue record without holdings detail |
| `summary` | Enough public information to describe the Work without a copy — see [05](05-summary-only-records.md) |
| `not_found` | Explicit, with the reason and what was searched |

Every result carries the registry name, retrieval time, a confidence or match
explanation, and the raw record reference where one exists. Two registries
agreeing is *not* independent corroboration; keep each record separate and
deduplicate on identifiers while preserving provenance.

## Scope

1. **Registry interface.**
   - A trait/contract with capabilities (which query fields, which result
     kinds), authentication, rate limits, timeout, and whether it is
     deterministic or model-backed.
   - Credentials from environment or a user config file, never committed.
   - Adapters may be HTTP, SRU/Z39.50, OAI-PMH, or a local database.

2. **Default registries.**
   - OpenAlex, Crossref, arXiv, Unpaywall (existing adapters) wrapped in the
     contract, plus a generic "web search" model registry.
   - One concrete institutional example to prove the contract: an SRU or
     Ex Libris/OCLC-style catalogue adapter, fixture-tested.

3. **Search orchestration.**
   - Deterministic registries in parallel, best-candidate merge by identifier
     then by score; failures are per-registry and never discard other results.
   - If all deterministic registries return nothing usable, run the bounded
     model fallback (web-search tools, JSON schema, direct URLs only). The
     fallback never guesses; it returns candidates with sources.
   - Cache per registry + canonical query; expose a `refresh` mode.

4. **User registries.**
   - A small manifest (name, endpoint, protocol, auth key, enabled) under the
     config directory, plus CLI/API to add, list, test and disable.
   - Per-user private; no global registries in the local workspace.

5. **Surfacing.**
   - The reader's "find reference", acquisition and lit flows consume the same
     contract; result kinds the UI does not support yet (e.g. `holding`) are
     still returned and displayed as text.
   - Every result shown links to the record that produced it.

## Non-goals

- Building a discovery UI for arbitrary catalogues in this task; contract and
  adapters first.
- Piracy or bypassing access controls. Licensed links are surfaced, not
  scraped.

## Acceptance

- The contract is documented, implemented, and covered by fixture tests.
- Existing OpenAlex/Crossref/arXiv/Unpaywall calls are migrated behind it with
  no behavior regression.
- One institutional catalogue adapter works against a recorded fixture.
- A query that OpenAlex cannot answer produces a `summary` or model-found
  candidate with a source, or an explicit `not_found` with reasons.
- A user can add a registry, test it, and see its results in the UI.

## Open questions

1. Which catalogue protocol first: SRU, OAI-PMH, or a plain JSON HTTP adapter?
2. Does the model fallback run automatically or only when the user asks? What
   is the per-query budget?
3. How do user registries interact with the provenance/trust tiers — is a
   library catalogue a new origin, or `library_metadata`?
