# Handoff: core pipeline and the unified analysis backend

Status: 2026-10-21. Progress and open questions from folding the extraction
spike into the integration line and moving every model reader onto one
analysis pipeline.

## What landed

1. **Extraction spike folded in.** The E0-E13 `paper-artifacts` backend, the
   printed-reference resolver, and the benchmark harness are now on
   `integration/literature-viewer` (`69d15bd`).
2. **Provider seam.** `paper_artifacts::run::Backend` and `extract_with` run one
   pipeline over either OpenRouter HTTP or a Codex/Claude plan harness
   (`a3431ba`).
3. **Plan harnesses on the core.** `LocalCliAnalyzer::plan_completion` runs a
   structured call with no tools and no web; core calls use low effort. Codex,
   Claude, and OpenRouter all route through `analyze_core_artifacts`.
4. **Plan DAG.** `RunConfig` calls carry `requires`. Parsing rejects duplicate
   names, unknown and self requires, and cycles. Ready calls run in parallel
   batches (`c6792a0`).
5. **One pipeline.** `default_pipeline(core_mode, features)` builds the core
   and optional feature nodes, and `extract_with` always runs a plan. The old
   `core_pass` / `fanout_core` / `feature_passes` path is removed (`3bef2f1`).
6. **Component refresh.** Structure and abstract refresh run the core pipeline
   and keep only the requested field. Context refresh is disabled and its reader
   affordance is gone (`642551c`). Revision dropped web (`42be402`).

## Findings that shaped it

- The core/optional split is already real: nine core fields in
  `core_fields.rs`, seven independently requestable features in `features.rs`.
- Raw page text beat markdown+priors on sections and graph and cost less (E1),
  though the comparison was confounded. E13's winner is raw text plus the
  deterministic page-1 title prior and the `ceil(pages/4)-ceil(pages/2)`
  sections guidance.
- A combined core call at low effort, a schema-validated self-check, and
  verbatim quote admission are the parts worth keeping.

## Open questions

1. **Refresh cost.** Structure and abstract refresh re-run the whole core. Should
   refresh run only the subgraph that produces its field, with per-node caching?
2. **Reader mapping.** `draft_from_artifacts` fills only thesis, sections, and
   the abstract. Glossary, claims, caveats, reading path, and context stay
   empty, so several tabs are thin.
3. **Feature opt-in.** The pipeline supports `objects`, `supercut`, `analysis`,
   and `glossary` nodes, but the reader always asks for `none`. No UI opts in.
4. **Graph fields.** `references`, `citations`, and `relations` come back from
   the core call but are not wired into the reference resolver.
5. **Feedback path.** `revise` still uses the old pipeline and the old
   deterministic abstract extraction, so feedback can diverge from analyze.
6. **`:analyze` command.** It accepts only codex, claude, or heuristic, so the
   no-argument default (OpenRouter) and `:analyze openrouter` both error. The
   toolbar button works.
7. **Context research.** It is disabled. Rebuild it as a no-web node, or delete
   it.
8. **Dead code.** `AnalysisService::refresh_structure`, `refresh_context`, and
   the `local_cli.analyze` orchestration are unreachable but still compiled.
9. **Live validation.** The core path is unit-tested and stub-tested, not run
   against a real model since the rewrite. A live run needs `OPENROUTER_API_KEY`
   and a budget.
10. **Duplicate extraction.** `load_or_extract` extracts the PDF, then
    `paper_artifacts::resources::build` extracts it again.
11. **Provider and budget defaults.** `analyze_core_artifacts` leaves
    `providers: []` and `paper_budget_usd: 0.15`, matching the spike defaults,
    not the app's spend plumbing.

## Merge state

- `main` (`3164c0d`) is an ancestor of `integration/literature-viewer`
  (`642551c`). Integration is 108 commits ahead and fast-forwards into main.

## Gates

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --lib` (1005 tests)
- web `typecheck`, `lint`, `build`
