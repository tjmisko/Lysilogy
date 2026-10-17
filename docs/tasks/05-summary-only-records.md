# 05 · Summary-only records for papers without a copy

Status: proposed, 2026-10-16. Priority P1. Depends on 03 and 04.

## Problem

Some of the most important Works in a citation graph cannot be obtained: closed
access, no repository copy, a dead link, a book, or a paper whose PDF the user
does not want to fetch. Today such a Work is a graph node with a title and
little else, or an `unavailable` acquisition row. The public information about
it — what it argues, why it matters, how it is received — is still useful to a
reader, and the user wants that available.

The user's requirement: a state where we have asked a chat agent what the paper
contains and can display a high-level summary, backed by public sources. When
this runs is still open; the data model should exist first.

## Goal

A first-class `summary_only` record: a Work we know about, cannot read locally,
and can describe from sourced public information. It is clearly distinct from a
full-text Work and from a plain `unavailable` verdict.

## Scope

1. **Availability model.**
   - Give every Work an explicit availability state, separate from title state:
     `full_text` (a local document is linked), `summary_only`, `metadata_only`,
     `unavailable` (with reason), `unknown`.
   - Acquisition outcomes map onto it; a failed fetch with public information
     can move a Work from `unavailable` to `summary_only`.
   - The state is visible in the graph, ramp and Work panel.

2. **Summary artifact.**
   - A typed record stored beside the Work (Postgres + rebuildable cache):
     thesis or question, central contribution, mechanism, evidence, field
     context, key limitations, and reception where sources support it.
   - Carries: model and version, generation time, a source list, and per-claim
     source IDs — reuse the existing context evidence/admission contract
     ([`context/mod.rs`](../../src/analysis/context/mod.rs)) or a lighter
     variant of it.
   - Every externally checkable claim must be grounded to a source; unsupported
     material is omitted or explicitly labelled as model inference.

3. **Trigger.**
   - Candidate triggers: acquisition fails and a registry returns usable public
     information; the user explicitly asks; the Work is pinned / high-prominence
     and read-path relevant. All are opt-in per Work or per scope — never a
     default batch over the whole graph.
   - A summary is keyed by Work identity plus prompt/model version; regenerating
     is explicit.

4. **Display.**
   - A summary card in the Work panel and the reading ramp: "no local copy",
     the summary, and the exact sources with links.
   - Distinguish clearly between paper-internal summary and external reception,
     and between a sourced claim and inference.
   - The card offers "fetch" / "search registries" and, when a copy appears
     later, the full text takes over while the summary remains viewable.

5. **Search integration.**
   - A registry result of kind `summary` ([04](04-search-registries.md)) can seed
     this record; the summary artifact is what we show.

## Non-goals

- Presenting a summary as a substitute for reading the paper; the UI must say
  otherwise.
- Automatic summarization of every unavailable Work; cost and policy are open.
- Long-form literature review generation.

## Acceptance

- The availability state exists, is persisted, and is shown in the UI.
- A Work with no copy can hold a grounded summary with sources; a claim without
  a source is not published.
- `summary_only` and `unavailable` are visibly different states.
- Regenerating a summary is explicit, cached, and spend-capped.
- A later successful fetch promotes the Work to `full_text` without losing the
  summary record.

## Open questions

1. Which model and profile writes the summary, and does it need web tools for
   reception?
2. Should the summary reuse `ContextNote`/`ContextSource` structures or get a
   new schema for "paper contents" versus "field history"?
3. Do summaries ever aggregate across the graph (e.g. a summarised field), or
   stay per-Work?
