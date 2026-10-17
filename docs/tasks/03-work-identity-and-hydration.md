# 03 · Work identity, hydration and the "Untitled" population

Status: proposed, 2026-10-16. Priority P0.

## Problem

362 of 3,461 Works in the dev database have no selected title. In the graph,
ramp and histogram they render as "Untitled" with a short ID. The user's
question was whether this comes from the deterministic base. The answer is yes,
in two distinct ways, and neither is currently detected.

## Diagnosis

### Cause A — dead OpenAlex reference IDs (352 of 354 with an OpenAlex ID)

- Stub Works are created from a citing Work's `referenced_works` array, and the
  OpenAlex identifier is attached at creation
  ([`ingest.rs`](../../crates/lysilogy-ingest/src/ingest.rs),
  [`literature.rs`](../../crates/lysilogy-ingest/src/literature.rs)).
- Hydration then batch-fetches OpenAlex summaries and silently skips any ID the
  provider does not return:
  [`hydrate_stub_works`](../../crates/lysilogy-ingest/src/ingest.rs) and
  [`fetch_summaries_cached`](../../crates/lysilogy-ingest/src/openalex.rs).
  There is no record of "we asked and the provider omitted it".
- Live verification on 2026-10-16: all 354 untitled OpenAlex IDs were checked in
  chunks of 50 against `api.openalex.org`; **352 resolve to nothing** (direct
  `/works/W…` returns 404, `filter=ids.openalex:W…` returns `count 0`). Two
  resolve and were simply never cached.
- These IDs are stale entries in OpenAlex's own `referenced_works` for the
  citing paper (merged or deleted Works). The stub Work then exists forever with
  an identifier that points nowhere and `title_state = 'unknown'`.

### Cause B — bibliography mis-parses (8 without any OpenAlex ID)

- These stubs come from mis-split bibliography entries that lost the title
  ([02](02-bibliography-integrity.md)). Their claims contain venue text as
  authors (`["Austin"]`, `["Conference on Learning Representations"]`) and a
  year, but no title, so the Work can never be titled from local evidence.

### Why it is invisible

- `hydrate_stub_works` counts only successes. Omitted IDs produce no counter, no
  warning and no terminal state.
- `title_state` only distinguishes `known | unknown`; `unknown` is used both for
  "not fetched yet" and "the provider has nothing", and there is no last-checked
  timestamp on the identifier.
- The UI has a display fallback (`Untitled (short-id)`) that reads as a normal
  node rather than a data-quality state.

## Goal

Every Work either has a selected title or an explicit terminal state with a
reason; stale identifiers are reconciled or retired; the graph never shows an
anonymous "Untitled" without explanation.

## Scope

1. **Hydration completeness.**
   - Track, per Work and per identifier, whether a summary was requested and
     what came back: `fetched`, `provider_omitted`, `not_attempted`.
   - Retry `not_attempted`; treat `provider_omitted` as a state, not a no-op.
   - Record `checked_at` and `status` on `graph.work_identifier` (active /
     dead / merged / unknown), plus the redirect target when OpenAlex reports a
     merged Work.

2. **Fallback title resolution.**
   - Dead OpenAlex ID → try DOI and arXiv identifiers already attached.
   - Still nothing → title search by the citing paper's bibliography entry text
     (when local evidence exists).
   - Still nothing → mark the Work `title_unresolvable` with the reason. Do not
     leave it silently `unknown` forever.

3. **Backfill job.**
   - One command that walks all untitled Works, performs the fallback chain and
     reports counts by outcome. Idempotent and resumable, with the usual spend
     ledger and provider rate limits.
   - Backfill the current 362 and prove the count of indefinite `unknown` drops
     to zero.

4. **Terminal states and user control.**
   - A Work with no resolvable title gets a stable short label plus a visible
     state ("identifier retired", "no printed title", "unavailable").
   - The user can set a title manually (existing `user` trust tier already
     outranks everything).

5. **Prevent recurrence.**
   - At stub creation, fail the run loudly if the reference ID is known dead?
     No — record it and continue, but never count it as hydrated.
   - A follow-up `lit` run must re-check `provider_omitted` IDs once, not fetch
     them every time.

## Non-goals

- Guessing identities from fuzzy titles; that is [04](04-search-registries.md)
  and the deferred fuzzy matching work.
- Deleting stub Works; an assertion may still point at them.

## Acceptance

- After backfill: zero Works with indefinite `unknown`; all untitled Works have
  `title_unresolvable`, a retired identifier, or a real title.
- Hydration summaries report `fetched`, `provider_omitted`, `not_attempted` and
  `retitled` counts.
- The graph shows a state badge and stable label for unresolvable Works; manual
  title entry works and survives re-ingest.
- Rerunning `lit` on a paper with dead references is stable and cheap.

## Open questions

1. Do we store the OpenAlex merge target when the API exposes it, and rewrite
   the identifier automatically, or only record it?
2. Should a `title_unresolvable` Work be excluded from prominence and counting,
   or shown as a warning?
3. How much of the dev database should be backfilled as part of this task versus
   left for the user to run?
