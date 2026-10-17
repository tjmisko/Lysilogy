# Backend task list

Status: 2026-10-16. These documents describe the next backend workstreams needed
to bring Lysilogy in line with the intended product. They are deliberately high
level: problem, evidence, goal, scope, acceptance, open questions. Detailed
implementation plans belong in the individual task once it starts.

They come out of the 2026-10-16 audit of the dev database and the extraction
pipeline. Two findings drive the ordering:

- **362 of 3,461 Works have no title** ("Untitled" in the graph). 354 have
  OpenAlex identifiers; 352 of those identifiers no longer resolve at OpenAlex
  (verified live against the API), and hydration silently skips provider
  omissions. The remaining 8 are local stubs from bibliography entries that were
  split or parsed wrong. See [03](03-work-identity-and-hydration.md) and
  [02](02-bibliography-integrity.md).
- **The bibliography pipeline fails on real papers.** One local bibliography
  record contains the whole paper body as 2 enormous "entries"; another splits
  "Preference-based policy learning. Machine" from "learning and knowledge
  discovery in databases, pages 12–27, 2011." and creates an untitled stub from
  the tail. See [02](02-bibliography-integrity.md).

## Order

| # | Task | Priority | Depends on | Why now |
| --- | --- | --- | --- | --- |
| 01 | [Text extraction fidelity](01-text-extraction-fidelity.md) | P0 | — | Every downstream artifact trusts extraction; loss is currently silent. |
| 02 | [Bibliography integrity](02-bibliography-integrity.md) | P0 | — | Demonstrably wrong today; produces untitled Works and bad citations. |
| 03 | [Work identity and hydration](03-work-identity-and-hydration.md) | P0 | 01, 02 | Fixes the visible "Untitled" population and prevents recurrence. |
| 04 | [Search registries](04-search-registries.md) | P1 | — | Identity resolution must not be hard-wired to four providers. |
| 05 | [Summary-only records](05-summary-only-records.md) | P1 | 03, 04 | Key papers without a copy still deserve a sourced record. |
| 06 | [Analysis agent harness](06-analysis-agent-harness.md) | P1 | 01–03 | Rebuild stage harnesses around structured tools and OpenRouter model routing. |

Dependencies are directional, not blocking: 01 and 02 can proceed together; 04
is independent of the extraction work and can start in parallel.

## Cross-cutting rules

- **Only fail when the source is truly unreadable.** Every other failure is an
  explicit state with a reason, surfaced to the user.
- **Determinism first, quality second, but quality wins ties.** Do what can be
  done deterministically; use a model to check or repair when determinism is
  uncertain; never let a model silently overwrite source text.
- **Models propose, the application admits.** Grounding, provenance and trust
  tiers stay application-owned.
- **No silent drops.** A dropped entry, field, mention, reference or title is a
  counted, reportable event.
- **Budget everything.** Per-pass call limits, concurrency, timeouts and the
  shared JSONL spend ledger.
- **Custom agents on OpenRouter by default.** Plan-backed harnesses are
  available; metered first-party Anthropic/OpenAI is opt-in only. Record the
  funding source on every model call. See
  [agent harnesses and model routing](../agent-harnesses.md).

## References

- [Agent harnesses, plans and model routing](../agent-harnesses.md)
- [Architecture](../architecture.md)
- [Reading index](../reading-index.md)
- [Reading pipeline plan](../reading-pipeline-plan.md)
- [Literature viewer status](../literature-viewer-status.md)
- [Citation graph sources](../citation-graph-sources.md)
