# Handoff: core pipeline and the unified analysis backend

Status: 2026-10-21, updated 2026-11-18. Progress and open questions from folding the extraction
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
2. **Reader mapping.** `draft_from_artifacts` fills thesis, abstract, outline,
   sections and, since 2026-11-18, the context. Glossary, claims, caveats, reading path, and context stay
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
7. **Context research.** Rebuilt 2026-11-18 as a no-web plan node with a
   neighbour dossier from the citation graph; see `docs/context-node.md`.
8. **Dead code.** `AnalysisService::refresh_structure`, `refresh_context`, and
   the `local_cli.analyze` orchestration are unreachable but still compiled.
9. **Live validation.** The core path is unit-tested and stub-tested, not run
   against a real model since the rewrite. A live run needs `OPENROUTER_API_KEY`
   and a budget.
10. **Duplicate extraction.** `load_or_extract` extracts the PDF, then
    `paper_artifacts::resources::build` extracts it again.
11. **Provider and budget defaults.** Resolved 2026-11-18: the app reads an
    `analysis` section from `lysilogy.config.json` (model, optional
    `judgment_model` for the self-check, `providers` defaulting to
    `["deepinfra"]`, `paper_budget_usd` defaulting to 0.50).

## Robustness added 2026-11-18

- The page-1 title prior (`paper_artifacts::title`) ignores rotated margin
  stamps, edge furniture, drop caps and banners, joins multi-line titles, and
  is offered to the model as a hint, not as the answer.
- Records (version 6) carry `verification` (self-check, admission, repair:
  ran, disabled, not needed, skipped by budget, or failed). A skipped or
  failed stage makes the record `degraded()` rather than `succeeded()`.
- `paper_artifacts::plausibility` checks the merged record against the page
  text (section coverage and count, references and abstract against their
  headings, title and authors). A call whose sections or references look
  degenerate is retried once with a note saying what looked wrong; the better
  answer is kept. The warnings land on the record, on `AnalysisOutcome`, and
  on the stored `PaperAnalysis.quality_warnings`.
- `pipeline_version` (model, providers, and a digest of every prompt, schema
  and plan setting) is on the record and on the stored analysis, so a human
  review can be tied to what produced the output.
- None of this has been run against a live model yet.

## Two segmentations added 2026-11-18

The single model-made reading map is now two related structures, both in
the record and both on the stored `PaperAnalysis`.

- **Outline** (`artifacts.outline`, `PaperAnalysis.outline`): the paper's
  own headings as printed, with number, title, level, page, role
  (`front_matter`, `main`, `back_matter`, `references`, `appendix`) and the
  printed line as an anchor. `paper_artifacts::outline` finds heading
  candidates deterministically (numbering such as `2`, `2.1`, `IV.`, `A.`;
  familiar heading words; short all-caps lines; lines in larger type than
  the body; running heads removed) and offers them to the `outline` call,
  whose job is only to confirm, reject, correct and add, and to fill levels
  and roles. Every returned heading is admitted by matching its printed
  line on its page or a neighbour; unmatched headings are dropped, numbered
  levels follow the numbering. An empty outline is a valid result.
- **Logical units** (`artifacts.sections`, still `PaperAnalysis.sections`
  so the Overview keeps working): when the outline is usable (at least two
  main-text headings and about one per six pages of main text) the
  `sections` call is asked only for contiguous ranges of outline entries
  (`outline_first`, `outline_last`) plus a title, a synthesis and the
  question the unit answers (`paper-field-sections-grouped.schema.json`).
  `paper_artifacts::grouping` orders the ranges, clips overlaps, absorbs
  skipped main-text headings into a neighbour, keeps references and
  appendices to at most one unit each and out of main-text units, and
  derives pages and source spans from the outline. Each repair becomes a
  `sections_grouping_repaired` warning. Without a usable outline the call
  falls back to the old page-span units, with null outline indices. Each
  stored section records `outline_entries` and `question`.
- **Pipeline**: the default combined plan is now `core` (every other core
  field) and `outline` in parallel, then `sections` after `outline`. That is
  three paper-length prompts instead of one; prefix caching on the pinned
  provider should absorb most of the repeat, but the cost has not been
  measured.
- **Self-check**: patches to the outline, or to a grouped unit's range,
  pages or span, are recorded as flags (`structure::guard_corrections`);
  titles, summaries and questions stay editable. The draft is settled again
  after the self-check.
- **Plausibility**: `outline_missing` and `outline_sparse` (against the
  numbered heading lines in the text) trigger the one retry like the
  section and reference gates; `sections_grouping_invalid` and
  `sections_mirror_outline` (one unit per heading) are reported only.
- **Benchmark**: `score_sections_oracle` reads a unit's pages from its
  outline range when a record lacks them, and `score_outline` scores
  heading recall and precision against an optional `outline` list in an
  oracle. No oracle has an outline yet.
- **Prompts**: `prompts/outline.md`, a rewritten `prompts/sectioning.md`
  (the reader, what a good unit is, weak versus strong titles) with
  `sectioning-outline.md` and `sectioning-pages.md` for the two modes.
- Records are version 7. Plan-harness calls now write one schema file per
  schema, because concurrent calls shared one file.
- Tested with stub plan harnesses only. Nothing here has run against a live
  model, and the candidate heuristics have not been measured on real PDFs.

## Merge state

- `main` (`3164c0d`) is an ancestor of `integration/literature-viewer`
  (`642551c`). Integration is 108 commits ahead and fast-forwards into main.

## Gates

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --lib` (1005 tests)
- web `typecheck`, `lint`, `build`

## Context node added 2026-11-18

- A `context` plan node runs after the core call on the strong tier at high
  effort, reads the paper, a brief of the core draft and a neighbour dossier
  assembled from the citation graph, and writes a lead, before, contribution,
  after and a read-alongside list. Every claim is `paper` (admitted quote),
  `dossier` (work id checked against the dossier) or `background` (shown,
  labelled unverified). Failed support downgrades to background and is
  counted. Contract: `docs/context-node.md`.
- Context refresh is re-enabled on top of the node, over the last run's core.
- The thesis and abstract instructions were rewritten; the record and the
  stored analysis carry `abstract_agreement` (token F1 against the
  deterministic abstract).
- Records are version 8. Tested with stub plan harnesses only.

## Reader wiring and review merge, 2026-11-18

- **Merged** the `review-env` branch (rubric, evaluation set, fetcher, review
  storage and API, `lysilogy review`, the F2 review panel). The one conflict
  was the CLI dispatch in `src/main.rs`.
- **Abstract view** (`web/src/components/AbstractView.tsx`,
  `ReadAlongsideList.tsx`): shows the context node's lead as "Why it
  matters", then before / what it changed / after, each claim with a source
  class marker (From the paper, with its quote and a page link; Citation
  graph, naming the graph record; Background · unverified, with the reason
  when a claim was downgraded), the reception note, the read-alongside list
  (Open in reader for a library paper, Fetch PDF through the acquisition flow
  for a graph work, plain citation otherwise), and a collapsed line with the
  pipeline warnings, pipeline version and abstract F1. Older analyses fall
  back to the earlier URL-checked notes.
- **Overview** (`SectionAtlas.tsx`, `SourceMap.tsx`): a Logical units /
  Outline toggle (`O`, remembered per browser). Tiles and a unit index under
  the page map name the printed headings each unit groups (`§ 1–2.1`). The
  outline is a hierarchical list with each heading's page and owning unit;
  `j`/`k` move, Enter opens the unit that holds the heading (or its page).
- **Review panel**: reads snapshot fields with the reader model's types;
  shows the outline, the units with the headings they group and their
  questions, the context with source classes and counts, and per-component
  pipeline warnings. The snapshot now pins `quality_warnings`; the rubric's
  `reads` lists the real field names.
- **Shared fixtures**: `tests/fixtures/reader/analysis-current.json` (every
  reader field) and `analysis-legacy.json` (a schema-4 analysis without
  them). `src/domain/fixture_tests.rs` round-trips the current fixture and
  fails if `PaperAnalysis` drops a fixture field or has a field the fixture
  does not exercise; `web/scripts/reader-output.test.mjs` and
  `review.test.mjs` run the helpers on both; `web/scripts/reader-output-smoke.mjs`
  renders the built app on them and writes `docs/screenshots/`.

### Still unverified against a live model

Everything above renders stub or fixture data. Nothing in this branch has run
against a real model or a real paper: the title hint, the plausibility retry
and its cost, the outline candidates and the model's confirmation of them, the
quality of logical groupings, the context node's source classes and
specificity, the dossier assembled from a live Postgres graph, and the cost of
three or four whole-paper calls per paper within the 0.50 USD budget. The
evaluation-set identifiers in `eval/review/papers.json` are unverified and the
fetcher has never downloaded anything. The first live step is: fetch the set,
analyse two AI and two economics papers, read the outline candidates and
context in the run records, and grade them in the panel.

Known UI gap: on a narrow page-map tile the logical-unit title can still
overflow its tile (e.g. a long word at 6 columns); this predates the toggle.
