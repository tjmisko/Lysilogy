# Handoff: backend implementation loop

Status: 2026-10-16. Branch `integration/literature-viewer`, uncommitted working
tree. This is the starting point and guardrail document for the phase-by-phase
implementation loop. Read this first, then [README.md](README.md) and
[../agent-harnesses.md](../agent-harnesses.md).

## What is already in the working tree

Uncommitted but complete and gate-clean (clippy, `cargo test --workspace`,
web typecheck/lint/build):

- **Sources is an inline reader tab, no overlay.** `L` switches to a `sources`
  view after Text; `LiteratureGraph` has an `embedded` mode; the histogram is
  the default and `v` toggles to the radial graph.
- **OpenRouter provider is wired and default.** `AnalysisProvider::OpenRouter`
  plus `run_openrouter` (chat completions, `response_format.json_schema`,
  `deepseek/deepseek-v4.1-flash`), all CLI/API/UI defaults updated.
- **Spend ledger.** `src/analysis/spend.rs`, JSONL at
  `.lysilogy/model-spend.jsonl`, per-substage records, `lysilogy spend`, $0.10
  default cap (`LYSILOGY_MODEL_BUDGET_USD`).
- **Substage labeling** through `ReaderToolRequest.purpose` and stage names.
- **Docs:** the task list under `docs/tasks/` and the plan/model reference at
  `docs/agent-harnesses.md`.

Nothing above is committed. Decide whether the loop commits these as a base
before starting phase 01.

## The task list

Order and dependencies are in [README.md](README.md). Summary:

| Phase | Document | Focus |
| --- | --- | --- |
| 01 | [text extraction fidelity](01-text-extraction-fidelity.md) | adversarial audit, mangle detection, token-anchored repair, metadata completeness |
| 02 | [bibliography integrity](02-bibliography-integrity.md) | region detection, entry splitting, field parsing, cheap-model verification |
| 03 | [work identity and hydration](03-work-identity-and-hydration.md) | dead OpenAlex IDs, hydration states, backfill, terminal titles |
| 04 | [search registries](04-search-registries.md) | pluggable query/result contract, user registries, model fallback |
| 05 | [summary-only records](05-summary-only-records.md) | availability model, grounded summary artifact, display |
| 06 | [analysis agent harness](06-analysis-agent-harness.md) | OpenRouter default agents, structured tools, optional planning |

01 and 02 are foundations and can run together. 04 is independent. 03 depends
on 01/02. 05 depends on 03/04. 06 depends on 01–03 and should move stages one at
a time, starting with bibliography verification.

## Findings the loop should treat as settled

Do not re-investigate these; they are verified.

- **Untitled Works:** of 362 untitled (of 3,461 total), 354 have OpenAlex IDs and
  **352 of those IDs no longer resolve at OpenAlex** (verified live: direct 404,
  `filter=ids.openalex:` count 0). Hydration silently skips provider omissions.
  The other 8 are local stubs from mis-split bibliography entries.
- **Bibliography is broken on real papers:** one `local_bibliography` record
  holds the paper body as 2 huge entries (reference list never found); another
  splits "Preference-based policy learning. Machine" from "learning and knowledge
  discovery in databases, pages 12–27, 2011."; the "et al." boundary in
  `bibliography/fields.rs` is wrong. Evidence and examples are in
  [02](02-bibliography-integrity.md) and [03](03-work-identity-and-hydration.md).
- **Plan/auth:** `claude --print` spends the Claude Pro plan, `codex exec` spends
  the ChatGPT plan, OpenRouter is metered; there is no official plan-backed HTTP
  API. `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` are set and must stay out of the
  default path. Details and tool-control flags: [../agent-harnesses.md](../agent-harnesses.md).

## Locked decisions

- **OpenRouter custom agents are the default**, with plan harnesses available and
  metered first-party APIs opt-in only. Revisitable in one place (doc 06 /
  `agent-harnesses.md`).
- **Determinism first, quality wins ties.** Models propose; the application
  admits. Never let a model silently overwrite source text.
- **No silent drops.** A dropped entry, field, mention, reference or title is a
  counted, user-visible state. "Unreadable" is the only acceptable hard failure.
- **Grounding is what makes cheap models safe.** Exact span/quote/substring
  admission stays on every model output.
- **Every model call records its funding and cost** in the spend ledger.

## Verification gates

Run before calling any phase done:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd web && npm run typecheck && npm run lint && npm run build)
```

For model-routing changes, score grounded acceptance rate × cost on a fixed
sample; do not judge by prose. For extraction changes, add the paper to the
audit corpus and report measured loss.

## Decisions the loop needs from the user

1. Commit the current working tree as the loop's base, or start on top of it
   uncommitted?
2. For 02: does the cheap-model bibliography verification run by default with
   local evidence, and when the model disagrees with `bib-v1`, do we insert a
   higher-confidence claim or only flag?
3. For 03: backfill the dev database as part of the phase, or ship the command
   and let the user run it?
4. For 05: which trigger promotes a Work to `summary_only` (acquisition failure
   with public info, explicit request, or prominence), and which model writes it?
5. For 06: which tasks move to the cheapest tier first, and what counts as an
   acceptable grounded-acceptance drop?

## Reference

The full pipeline audit (every model call, lookup, verifier and guarantee) is in
`~/Guides/How Lysilogy Builds Its Structured Artifacts.html`.
