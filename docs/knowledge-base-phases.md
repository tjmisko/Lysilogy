# Knowledge base implementation phases

This file is the execution plan for the [knowledge base plan](knowledge-base-plan.md). The design
doc explains *what* each issue builds and why; this file explains *in what order*, *on which
branch*, *touching which files*, and *how to know a phase is done*. Keep it current: tick an issue
when its PR merges and record anything a later agent must know under that phase's notes.

Tracking: [GitHub project 12](https://github.com/users/tjmisko/projects/12). Epics are #11–#18
and #67.
Issue numbers below link sub-issues to their design sections.

## Working rules for every issue

1. **Orient.** Read the issue, its design section in `knowledge-base-plan.md`, this file's notes for
   the current phase, and `docs/architecture.md`. Confirm every blocker issue is closed.
2. **Worktree.** `gh worktree create --branch <branch>` from an up-to-date `main`, using the branch
   names below. Never work in the main checkout: it carries unrelated uncommitted PDF-preview work.
3. **Tests alongside code.** Implement the issue's listed tests (named `should … when …`) plus any
   edge cases found while building. Prefer fixture-driven tests; no live network or model calls in
   the test suite.
4. **Quality gates** before opening a PR:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features -- -D warnings   # pedantic + nursery are denied
   cargo test --all-targets
   cd web && npm run typecheck && npm run lint && npm run build   # when web/ changed
   ```

   Also run the targeted `npm run test:*` / `smoke:*` scripts for any frontend area touched.
   `npm run smoke` needs a corpus fixture that is absent locally; use the fixture-backed smokes or a
   scratch Playwright harness instead.
5. **Metrics.** Each issue body lists the scorecard metrics it owns (see
   [Metrics and objectives](knowledge-base-plan.md#metrics-and-objectives)). Once E8.1 has merged,
   run `lysilogy eval <suite> --check` for every suite an issue touches, before and after the
   change, and put the scorecard delta in the PR body. A failing hard gate or an unjustified
   regression blocks the merge. Work toward an objective by measuring, changing, and re-measuring;
   when an issue merges below its objective, record the measured value and the next idea under the
   phase notes and open a follow-up issue linked to the epic. Issues that merge before their
   truth set exists record their metric the first time the suite becomes available.
6. **Commits and PRs.** Conventional, atomic commits. Open a **draft** PR titled
   `<type>: <summary> (E<x.y>)` whose body says `Closes #<n>`, lists tests added, and records any
   deviation from the design section. Update the design doc in the same PR when behavior deviates.
   **Merge policy:** within a phase, the implementing agent may mark a PR ready and merge it
   (merge commit, then delete the branch and remove the worktree) once the quality gates pass on
   the rebased branch and an independent review pass reports no unresolved correctness or
   security findings. After the last merge of a phase, run the phase exit checks, write a phase
   report to `docs/experiment-reports/<date>-kb-phase-<X>.md` (what shipped, exit-check results,
   deviations, known gaps), update this file, and continue to the next phase. There are no human
   gates during implementation; the user reviews the finished system.
7. **Scratch space.** `/tmp` is a RAM-backed tmpfs. Benchmark vaults, build caches, and large
   fixtures go under `~/.cache/lysilogy/` or the session scratchpad, never `/tmp`. The arXiv corpus
   lives at `~/Corpora/arxiv/` (expect roughly 25–40 GB for the scale tier plus eval sources; about
   139 GB was free on `/home` when this plan was written). Never copy corpus PDFs or sources into
   the repository, `local-articles`, or `.lysilogy`.
8. **Decide, record, continue.** When the design doc does not settle a decision, choose the option
   most consistent with the confirmed decisions and existing code, record it under the phase notes
   and in the PR body, and keep going. When a merged blocker turns out to be incomplete, open a
   follow-up issue linked to the epic, fix it first, then resume. Judgment tasks (gold-set
   labeling, library choices) are done by agents with their evidence recorded. Stop only for
   something no agent can resolve: missing credentials, a required `sudo` command, or a destructive
   operation on the user's vault or data root.

## Shared files and conflict hotspots

Parallel branches will collide here. Keep edits to these files small (a module declaration, a
route line, a subcommand variant) and rebase onto `main` before requesting review.

| File | Why it is shared |
| --- | --- |
| `Cargo.toml`, `Cargo.lock` | New crates (`rusqlite` with `bundled`, graph/metrics crates) |
| `src/lib.rs` | New top-level modules (`objects`, `kb`, `acquisition`, `citations`) |
| `src/main.rs` | New CLI subcommands (`kb rebuild`, `eval`, `corpus`, `bench`, `fetch`) in `enum Command` |
| `src/api.rs` | Router composition and `AppState` fields (KB store handle, global jobs) |
| `src/domain.rs` | `PaperOverview`/`PaperMetadata` changes (content hash, links to Works) |
| `web/src/App.tsx` | View modes, routing, command dispatch, key gating |
| `web/src/types.ts`, `web/src/lib/api.ts` | Response types and fetch helpers |
| `README.md`, `docs/architecture.md` | User-facing documentation of new behavior |

Proposed module layout (adjust if the code suggests better seams, and record the change here):

```text
src/objects/        mod.rs (types, objects.json), bibliography.rs, equations.rs, statements.rs,
                    algorithms.rs, enrichment.rs
src/kb/             mod.rs, types.rs, store/ (connection, migrations/*.sql, rebuild), names.rs,
                    titles.rs, resolve.rs, decisions.rs, ingest.rs, lists.rs, graph.rs, gold/
src/acquisition/    mod.rs (state machine), resolve.rs, locate.rs, agent.rs, verify.rs
src/citations/      csl.rs, bibtex.rs, ris.rs, style.rs
src/citation_graph/ cache.rs, budget.rs (E7.1), recommendations.rs (E7.2)
src/eval/           mod.rs (suites, results, scorecard, ratchet), corpus/ (arXiv harvest),
                    truth/ (latex.rs, crossref.rs, orcid.rs, lists.rs)
eval/               baselines.json, results/, truth/ (derived labels only, no PDFs or sources)
src/api/            objects.rs, kb.rs, acquisition.rs, lists.rs, graph.rs
web/src/components/ FiguresView.tsx, WorkPage.tsx, PersonPage.tsx, ReviewQueue.tsx,
                    ReadingListView.tsx, GraphView.tsx
web/src/lib/        route.ts, kbApi.ts, objects.ts
```

---

## Phase A: foundations

Goal: the types, stores, parsers, corpus, truth sets, and measurement harness that everything else
stands on.

Exit criteria: all Phase A issues merged; `kb rebuild` produces an empty but migrated database;
`objects.json` exposes figures, tables, and parsed bibliography entries for a mapped paper; the
arXiv corpus `eval` and `scale` tiers are downloaded and verified; K1, K2, K3, K4, K5, and K7 exist;
`lysilogy eval all` runs and `docs/kb-scorecard.md` records baselines for every metric measurable
so far (at least O1, O2, O8–O10, O25–O26, and the naive exact-key matcher on G1/O14); G4 and G5
pass. The Phase A report also records E0.1's measured serial/four-worker extractor capacity as
exploratory evidence. O27 is first measured against the production four-worker batch path in
E0.5 (Phase B); until that implementation exists, O27 remains unavailable. Its ≥0.70 target,
Phase B requirement for a measured result (or a measured miss with follow-up), and final system
acceptance are unchanged.

### Wave A1 (parallel, no blockers)

- [x] **#34 E2.2 KB domain types**. Branch `feat/e2.2-kb-types`.
  Owns `src/kb/mod.rs`, `src/kb/types.rs`, and the `pub mod kb;` line in `src/lib.rs`.
  Reuse or extend `citation_graph::Identifier` rather than duplicating identifier parsing. Merge
  this first: it is small and it creates the `src/kb/` module that A2 branches extend.
- [x] **#24 E1.1 PaperObject types and objects artifact**. Branch `feat/e1.1-paper-objects`.
  Owns `src/objects/mod.rs`, `src/api/objects.rs`, `web/src/lib/objects.ts`.
  `Figure` currently lives in `src/source_index.rs` inside the cached `reading-index.json`
  (schema version 6). Wrap it; do not move figure detection or bump the reading-index schema
  unless required. `objects.json` records the reading-index generation it was derived from.
- [x] **#19 E0.1 Synthetic 10k vault and benchmarks**. Branch `feat/e0.1-scale-bench`.
  Owns a generator and benchmark subcommand or script. The vault goes under
  `~/.cache/lysilogy/bench-vault/`. Commit the baseline report to
  `docs/experiment-reports/`.
- [x] **#20 E0.2 Content-hash paper identity**. Branch `feat/e0.2-content-hash`.
  Touches `src/library.rs`, `src/store.rs`, `src/domain.rs`. `sha2` is already a dependency.
  Renames currently orphan `papers/<id>/` because `PaperId` hashes the relative path; notes are
  keyed separately by relative path in `Notes/`, so decide and document how notes follow a move.
- [x] **#63 E7.1 Provider cache and rate budgets**. Branch `feat/e7.1-provider-cache`.
  Owns `src/citation_graph/cache.rs`, `budget.rs`; touches `http.rs`. Preserve the existing
  1.1-second spacing, `Retry-After` cooldown, and credential redaction behavior in
  `docs/citation-graph-sources.md`; the cache must never store credentials.
- [x] **#68 E8.1 Evaluation harness, scorecard, and ratchet**. Branch `feat/e8.1-eval-harness`.
  Owns `src/eval/` (or `eval/` tooling), `eval/baselines.json`, `docs/kb-scorecard.md`, and an
  `eval` subcommand. Merge early: every later PR reports metrics through it.
- [x] **#69 E8.2 arXiv research corpus**. Branch `feat/e8.2-arxiv-corpus`. Owns the corpus
  tooling (`corpus` subcommand or `scripts/corpus/`) and a checked-in selection config. Verified
  on 2026-09-12: the public bucket lists and serves over plain HTTPS
  (`https://storage.googleapis.com/storage/v1/b/arxiv-dataset/o?prefix=arxiv/arxiv/pdf/2608/`)
  and includes August 2026 papers; no `gsutil`/`gcloud`/`aws` CLI is installed or needed. The
  Kaggle metadata snapshot requires a login, so harvest metadata through OAI-PMH. Start the
  downloads as soon as the tool works; they take hours and should run in the background while
  other issues proceed.

### Wave A2 (after the listed blockers merge)

- [ ] **#25 E1.2 Backend bibliography extraction and parsing** (after #24).
  Branch `feat/e1.2-bibliography`. Owns `src/objects/bibliography.rs`; touches
  `web/src/lib/paperLinks.ts` and `PdfLinkHints.tsx`. Port the entry splitting and marker matching
  in `paperLinks.ts` (numeric, ranges, raised superscripts, author–year with suffixes) to Rust and
  keep `npm run test:paper-links` and `npm run smoke:link-hints` passing against the backend
  output. Ambiguous matches stay unresolved.
- [ ] **#103 Bibliography title line-break evidence** (follow-up to #25; after #25).
  Branch `fix/e1.2-bibliography-hyphens`. Owns conservative field parsing and focused fixtures.
  Preserve raw entry membership and literal compounds; measure unchanged K1/K2 fields. Five
  known K1 title misses remain after reasonable iteration; do not infer provider identifiers.
- [ ] **#33 E2.1 SQLite store, migrations, and rebuild** (no issue blocker; sequenced after #34 to
  avoid `src/kb/` conflicts). Branch `feat/e2.1-kb-store`. Owns `src/kb/store/`; adds `rusqlite`
  (`bundled`) to `Cargo.toml` and `kb rebuild` to `src/main.rs`. Database at
  `<data>/kb/kb.sqlite`, WAL mode, FTS5 trigram tokenizer (confirm the bundled SQLite enables
  FTS5). Include the 500k-Work synthetic neighborhood query benchmark from the design section.
- [x] **#35 E2.3 Person name parser** (after #34). Branch `feat/e2.3-names`. Owns
  `src/kb/names.rs`. Pure functions plus a large table-driven test.
- [x] **#36 E2.4 Title normalizer** (after #34). Branch `feat/e2.4-titles`. Owns
  `src/kb/titles.rs`. Pure functions; the FTS5 index population itself lands with #33 or #38,
  whichever merges later.
- [x] **#88 Corpus availability recovery** (follow-up to #69). Branch
  `fix/e8.2-corpus-availability`. Preserve exact quotas and immutable availability evidence;
  explicit recovery is limited to the original zero-artifact selection. PR #90 merged after
  independent verification of the actual recovery and canonical source download.
- [x] **#91 Canonical arXiv source endpoint** (follow-up to #69/#85). Branch
  `fix/e8.2-source-endpoint`. Use the approved host's `/src/` endpoint while preserving legitimate
  legacy source receipts; keep redirect refusal, version pinning and all transport safeguards.
- [x] **#70 E8.3 arXiv LaTeX object truth** (after #68, #69, #24). Branch
  `feat/e8.3-latex-truth`. Owns the LaTeX parser and PDF aligner used only for evaluation.
  Brought forward from A3 so #25 can be measured against independent K1 before merging;
  this resolves a sequencing cycle under the user's detector-before-truth merge restriction.
- [x] **#96 Figure/table metric collector** (follow-up to #24; after #70).
  Branch `feat/e1.1-object-metrics`. Owns `scripts/eval/object-metrics.py`, its tests, optional
  `examples/object_metrics.rs`, the matching contract and evidence/report. Run the actual objects
  path against frozen K1 inventories; measure O1 and independently annotated full-region O2.
  Keep caption geometry separate, count duplicates/misses, preserve PaperIds and exact generations.
  This missing measurement follow-up is required for the existing Phase A O1/O2 baselines.
- [x] **#101 Measured figure/table detection and region fixes** (follow-up to #24/#96; after #96).
  Branch `fix/e1.1-figure-detection`. Owns `src/source_index/figures.rs` and focused detector
  fixtures; minimal current-generation object/evaluation adapters as required. Recover Roman
  tables, distinguish prose references from captions, and tighten excessive body rectangles.
  Preserve frozen K1/index/registry artifacts, exact native anchor bindings, the #96 matching
  contract and all-truth O2 denominator. Measure actual current production behavior; cached old
  detector predictions cannot establish a new result. Coordinate object plumbing with #25.
- [ ] **#71 E8.4 Reference, acquisition, and read-next truth** (after #68, #63). Branch
  `feat/e8.4-reference-truth`. K7 needs parsed bibliographies of the scale tier; build the K2 and
  K5 parts first and finish K7 once #25 and the corpus mapping are available.
- [ ] **#72 E8.5 Person silver labels** (after #68, #63). Branch `feat/e8.5-person-labels`.
- [ ] **#97 Expand K1 to planned stratified coverage** (follow-up to #70; after #70).
  Branch `feat/e8.3-k1-coverage`. Owns evaluation-only source support, independent annotation
  tooling and expanded truth/evidence. Retain the approximately 500-paper target; compare the
  limited initial release with expanded strata without changing truth quality or score targets.
- [x] **#98 Preserve readable pages after native coordinate failure** (after #19, #20).
  Branch `fix/e0-reading-index-page-failure`. Owns `src/layout.rs`,
  `src/source_index/native.rs`, focused orchestration/tests and report. Keep the anchored layout
  parser strict; isolate malformed word geometry only within structurally valid reading-index
  pages, using existing bounded OCR or explicit unavailable provenance. Preserve valid frozen
  indexes and all identity records. Coordinate shared `src/source_index.rs` with #70.

### Wave A3

- [ ] **#37 E2.5 Resolution gold set and evaluation** (after #25, #69). Branch `feat/e2.5-gold-set`.
  Owns `src/kb/gold/`; reports through the E8.1 `resolution` suite. Mine ~200 name and title pairs
  from parsed bibliographies in `local-articles` and the arXiv corpus, including hard negatives. Store only bibliographic
  metadata, never full text. Label with agents: two independent labeler subagents, then an
  adjudicator subagent for disagreements, each writing rationale to the plain-text gold file.
  Commit the labeling script or prompt so it is reproducible. The 0.99 precision gate becomes a
  `cargo test` that loads the gold set.

### Phase A notes

- #96 merged in PR #102 (`d074dae`) after final independent clearance of `5d5c567`.
  First real limited-K1 baselines are **O1 0.7692307692** (10TP/1FP/5FN) and
  **O2 0.2448506858** over all 15 independently annotated bodies, with zero unknown regions.
  Both targets are missed and retained under #101. All ten figures match; five Roman tables
  are missing and one prose Figure 2 is an extra prediction. Every matched predicted region
  contains its entire true body but is 1.80–9.13 times too large; tighten excess coverage.
  G5 passes **320 Rust / 278 Python / 85 Node**, with 24 new Python and three bridge tests.
  Measurement took 4.570 s, zero network/models/$0. See
  [the report](experiment-reports/2026-09-13-object-metrics.md) and committed compact observations.
  Exact corpus/registry/index bytes and immutable truth are unchanged. Raw gates/predictions
  are external; worktree/branches removed after separate portability verification.
- #98 merged in PR #100 (`46f11e0`) after exact-head independent clearance of `8f27253`.
  Eleven new offline tests and final G5 **317 Rust / 254 Python / 85 Node** pass. The one
  failed paper now persists 45 native + 2 OCR pages, with original native failures on pages
  30/34 retained. All 1,749 prior index hashes and 10,928 prior identity rows are unchanged;
  the normal scan added 23 final corpus PDFs. Actual processing was 3.2725 s (10.9245 s with
  scan), 47,344 KiB RSS, no network/models/$0. See
  [the report](experiment-reports/2026-09-13-kb-reading-page-failure.md). No cache schema bump,
  coordinate repair or retroactive truth admission. Worktree and branches are removed; all
  62 retained evidence files and the standalone helper are already in permanent cache paths.
- #70 merged in PR #99 (`0dc68dc`) after independent clearance of exact head `4563702`.
  The immutable `k1-limited-v1` release contains two complete independently reviewed paper
  cohorts: 23 objects, 35 bibliography entries / 105 known fields, 40 citation groups / 50
  target pairs, ten O4 pairs and nine panel scoring opportunities. A complete negative visual
  inventory is retained. Both output files reproduced byte-for-byte; no detector objective is
  measured by truth publication. Current automatic coverage is 622 parsed / 1,000 inputs and
  zero eligible cohorts, preserving unsupported-source exclusions. Approximately 500-paper
  coverage remains unmet under #97. Final G5 passed **306 Rust / 254 Python / 85 Node**;
  all 144 LaTeX tooling tests and required Rust/eval gates passed, O30 remains 0/10k. See
  [the release report](experiment-reports/2026-09-13-kb-latex-truth.md), the pinned build config,
  and `eval/evidence/latex-truth-release.json`. #96 may now measure O1/O2 against the frozen
  complete inventories; #25 can measure its K1 components, while genuine K2 is still required.
- K0 downloads and full verification completed on 2026-09-13: **10,951 unique PDFs / 1,000
  sources**, including all **10,000 scale PDFs** and **1,000 eval PDF/source pairs**. The final
  production run exited 0 after SHA-256/MD5/size verification, with no problems. Root separately
  verified all 11,951 sidecars and exact selection/manifest agreement. Artifact bytes total
  46,892,682,976. No download process remains. See
  [the completion report](experiment-reports/2026-09-13-kb-corpus-complete.md) for frozen hashes,
  timing, RSS and limits; actual 10k app acceptance and index construction remain outstanding.
- Staged K1 publication decision on 2026-09-13: a named, versioned limited release may establish
  actual detector baselines once independent evidence covers every E1 object kind and per-paper
  quality holds. Its cohort size, strata, manual selection bias and exclusions must remain
  explicit. This does not achieve the approximately 500-paper target: #97 retains that coverage
  work after the measured historical automatic result of 0 admitted papers from 1,000 inputs.
  No metric target, hard gate or broad system acceptance claim changes. #70's PR must document
  this staged release and its actual labels; detector results retain their truth-version scope.
- #98 follows an actual eval indexing failure on arXiv 2308.05883v2: three reversed combining-
  accent boxes on pages 30/34 currently reject all 47 pages. Independent drawing evidence rules
  out swapping endpoints as a faithful repair. Retain coordinate validation and readable pages;
  label failed-page OCR/unavailability explicitly. This is an availability fix, not permission
  to invent native text or broaden truth eligibility through gaps.
- #88 merged in PR #90 (`e60acb9`) after independent clearance of exact head `5a2fc45`.
  Recovery preserved the original selection bytes, 136 public inventory proofs and all stratum
  quotas: 15 exclusions replaced 14 original IDs, retaining 1,000 eval / 10,000 scale / 10,951
  union papers. The 514,249-record metadata hash is unchanged. Review checked actual original,
  staged and published bytes, pinned inventory objects and the bounded source receipt. Final
  integrated G5 passed **305 Rust / 110 Python / 85 Node**; O30 remains 0/10k. The report and
  `eval/evidence/corpus-availability.json` retain all checkpoints; root independently verified
  and archived 63 raw receipts under `~/.cache/lysilogy/review-evidence/pr90/`. Worktree/branch
  removed after a separate runtime-dependency audit confirmed the loaded eval download needs
  only absolute corpus/cache paths. Main's source hash now matches the queued full-run guard.
  Corpus completion and K0 acceptance remain pending; never repeat selection recovery after
  admitting artifacts.
- #91 merged in PR #92 (`ca7d8d7`) after final independent clearance of `a540894`.
  New source requests use version-pinned `export.arxiv.org/src/`; exact legacy `/e-print/`
  receipts remain valid with their original bytes, URL and fetch time. Redirects remain refused.
  Seven new tests cover compatibility, interrupted manifest updates, identity/kind/hash rejection
  and byte preservation. Final G5 passed **305 Rust / 87 Python / 85 Node**, O30 remains 0/10k.
  A live run in the independently reviewed #88+#91 integration rehashed/reused `0812.5080v5.pdf`
  and admitted its source: 21,993 bytes, SHA-256
  `8b95087c0ab3a43d4f021459374bc52a66a4baae9211174f83984cb12f250c1d`, 3.19 s, 240,608 KiB RSS,
  $0 model cost. Root and reviewer checked both actual files and receipts. The report and
  `eval/evidence/corpus-source-endpoint.json` distinguish this integrated live run from the
  standalone branch gates. Root archived and verified 35 raw receipts under
  `~/.cache/lysilogy/review-evidence/pr92/`; branch/worktree removed. K0 remains incomplete.
- Sequencing adjustment on 2026-09-13: #70's independent K1 construction moves from A3 to A2.
  #25 ports bibliography detection and owns O8–O10, while the user's direct instruction requires
  detector truth and measurements before merge. Building K1 after every A2 merge would create a
  cycle. All #70 issue blockers (#68/#69/#24) are already closed; only its implementation wave
  changes. No truth quality requirement, metric target, hard gate or detector merge gate is
  relaxed. Start #70 when the corpus follow-ups free an implementation slot; keep #25 draft until
  its real collector runs. The collector uses independent K1 IDs and UTF-16 spans, penalizes
  missed segmentation in field accuracy, and reports unknown truth-field coverage separately.
- #36 merged in PR #89 (`0987241`) after independent clearance of final head `11027dd`.
  `kb::titles::title_key` folds common presentation variants while retaining negation, math
  operators/script binding, and unknown TeX argument structure. `title_similarity` is bounded
  multiset character-trigram Dice scoring for candidate generation; even a fuzzy score of one
  does not establish exact equality or identity. #33 must use the shared title key for its FTS5
  projection when resumed. Ten tests include 52 equivalent and 35 distinguishing title pairs;
  review reproduced and fixed fraction/group, minus, Unicode-script, and escaped-brace collisions.
  Final integrated G5 passed 305 Rust, 80 Python and 85 Node tests. O30 remains 0/10k; real
  resolver metrics remain unavailable without truth/collectors. No gate, target or baseline
  changed. Report `docs/experiment-reports/2026-09-13-title-normalizer.md` and evidence
  `eval/evidence/title-normalizer.json` retain exact before/after receipts and the Unicode audit.
  Root verified source/result/log hashes and archived 21 raw receipts at
  `~/.cache/lysilogy/review-evidence/pr89/`; branch/worktree removed after merge.
- #35 merged in PR #87 (`e066946`) after independent review of final head `4de6251` and
  all required gates. `kb::names` preserves raw input, unranked component alternatives, and
  coarse family/first-initial candidate keys. Particles, suffixes, compact capitals, and bare
  letters retain ambiguous interpretations; keys never establish identity. Review fixed literal
  Le/Van families and Al given names, compatibility-case folding, and bare-letter family
  alternatives. Final G5: 295 Rust, 80 Python, and 85 Node tests; 16 parser tests contain broad
  fixture tables. Persons/resolution remain unavailable without K3/K4; O30 remains 0/10k.
  No gate, objective target, or baseline changed. Report:
  `docs/experiment-reports/2026-09-13-person-name-parser.md`; committed before/after evidence:
  `eval/evidence/person-name-parser.json`. Root independently verified source/result/log hashes
  and archived 22 raw receipts at `~/.cache/lysilogy/review-evidence/pr87/` before removing the
  branch/worktree. The historical O25/O26 misses still belong to #21/#22.
- #19 merged in PR #82 (`2683dce`), completing A1. The full default run generated and verified
  10,000 PDFs, persisted all 10,000 extraction artifacts, performed six full extraction trials,
  and drove the release app with Playwright for three navigations and 60 searches. O25's populated
  no-change median is **15.503406 s** (target 2 s; follow-up #21); O26's first-render median is
  **1,757.1 ms** and search p95 **163.4 ms** (targets 500/150 ms; follow-up #22). Both existing
  issues now record the measured gap and next approach. These initial baselines are ratcheted;
  targets remain unchanged. All extractor outputs match; serial/four-worker medians are
  155.140652/45.328635 s, giving exploratory efficiency 0.855644. O27 remains unavailable until
  production concurrency ships in #23. The original clean-source run took 1,260.601 s and $0;
  raw observations and screenshot remain in
  `~/.cache/lysilogy/bench-vault/runs/d2019ea2409e42ceb373a98b03fb08ce`.
  Reports and derived evidence are committed under `docs/experiment-reports/2026-09-12-e0.1-*`
  and `eval/evidence/synthetic-benchmark-10k*.json`. Independent review verified physical counts,
  exact raw observations, all 150 source fingerprints, metric formulas, and retained gates.
  After integrating #85, final gates passed 279 Rust, 80 Python and 85 Node tests; G5 passes,
  O30 remains 0/10k. Both completed branches/worktrees are removed. Corpus harvesting continues
  from already loaded code; independent audit confirmed no remaining worktree dependency.
- Follow-up #85 (epic #67) merged in PR #86 (`e12adc3`) after independent review and all
  quality gates. Corpus builds can explicitly select `--proxy-env HTTPS_PROXY` (or
  `https_proxy`); this supports HTTP CONNECT transport with normal origin TLS verification.
  Unsupported HTTPS-proxy URLs fail rather than silently downgrading. Credential redaction
  covers connection failures, response reads/cleanup, and interrupted cooldown persistence.
  Fixed destinations, redirect refusal, request spacing and durable Retry-After remain enforced.
  Final gates: 277 Rust, 61 Python and 84 Node tests under G5; O30 remains 0/10k. The report and
  `eval/evidence/corpus-proxy-transport.json` explicitly distinguish retained historical-before
  evidence from the fresh after checks. After the user's daemon restart, approved proxy access
  succeeded and the live harvest saved 53,300 metadata records by 2026-09-13 06:32:45 UTC.
  K0 is still building; this is transport verification, not full corpus acceptance.
- Follow-up #83 (epic #18) merged in PR #84 (`481e911`). A duplicated or inherited descriptor
  could retain a provider's flock after the request lease ended. A private guard now explicitly
  unlocks before releasing the local mutex and covers state read/parse errors too. The exact
  duplicated-descriptor regression failed before the fix and passes after it; the original
  cooldown assertion and all admission rules remain unchanged. Independent review cleared final
  head `164fd9f`; 277 Rust, 50 Python, and 84 Node tests pass under G5. O30 remains 0/10k.
  Before/after evidence is in `eval/evidence/provider-lease-unlock.json`, with a measured report
  under `docs/experiment-reports/`. The original intermittent test did not record its returned
  value; the deterministic reproduction establishes the defect without claiming certainty about
  that particular failure. Issue closed and branch/worktree removed.
- O27 sequencing clarification: Phase A previously required an O27 baseline even though E0.5's
  production worker path is in B1. The current `ingest` loop in `src/main.rs` awaits each paper
  sequentially; E0.1's four-worker `JoinSet` exists only in its benchmark example. The Phase A
  exit text now requires the available O25/O26 baselines and separately records exploratory
  extractor capacity. O27's first production baseline remains required with E0.5, with its
  unchanged ≥0.70 target and unchanged Phase B/final acceptance. Benchmark-only scheduling must
  not establish a passing production objective.
- #20 merged in PR #80 (`94ddacc`) after both independent review findings were fixed and
  independently reproduced again. A strict source stamp now spans fresh extraction, rejecting
  changed-and-restored bytes before artifact publication. The canonical
  `<data>/paper-identities.json` registry retains initial path-derived IDs across unique
  content-hash moves, tombstones, original notes keys, and unresolved prior identity IDs.
  Duplicate-move warnings persist across scans/restarts; replacements and ambiguous matches
  receive separate identities without overwriting artifacts or notes. Registry transactions use
  cross-process locking and durable atomic publication, bind to one canonical library root,
  and fail closed on corrupt state. Hash caching uses size/mtime plus inode/ctime safeguards.
  Later KB rebuilds must preserve these canonical identity records. Moves before the initial
  registry scan cannot be inferred from unrecorded hashes. Final gates: 275 Rust tests,
  frontend typecheck/lint/build and 17 targeted tests, G5 pass, unchanged O30 = 0/10k. Evidence
  is retained under `eval/evidence/*content-identity*.json`. Branch/worktree removed.
- Follow-up #79 (epic #67) merged in PR #81 (`1e9bc5e`). A cold benchmark build exposed G5's
  missing `mold`/`ld.mold` aliases; warm targets had hidden the gap. The production restricted
  PATH now includes installed linker aliases and a regression links a fresh executable through
  that same construction. Independent review cleared the fix. After removing only the isolated
  worktree's package outputs, G5 rebuilt Rust test binaries and passed 255 Rust, 50 Python, and
  84 Node tests in 17.816 seconds. Before/after evidence, including the failed run's dirty-state
  flag, is retained in `eval/evidence/g5-cold-linker.json`. No gate or target was changed; G5 and
  O30 still pass. The follow-up issue is closed and its branch/worktree removed.
- #63 merged in PR #78 (`2c21e77`) after independent review of the transport, credential
  redaction, durable shared budgets, independent O30 observer, and atomic collector publication.
  O30 meets target: 0 violations across 10,000 admissions, with 216 deferrals, 72 cooldowns, and
  32 serialized state reloads. Final G5 passed 255 Rust, 49 Python, and 84 Node tests; formatting
  and Clippy passed. Scorecard: 1/5 gates and 1/30 objectives. Report and retained evidence are in
  `docs/experiment-reports/2026-09-12-provider-budgets.md` and `eval/evidence/`. Shared storage is
  `~/.cache/lysilogy/providers`, independent of data roots, created only for explicit requests.
  Defaults are seven-day cache expiry, 1.1-second spacing, and 50 requests per 60-second window;
  fixed provider endpoints and credential redaction remain enforced. No live provider/model
  calls were made. On fresh worktrees, run `python3 scripts/eval/provider-budgets.py` before
  `eval scale --check` to regenerate its ignored input instead of reporting O30 unavailable.
  Branch/worktree removed after merge; no objective miss or follow-up issue was needed.
- #68 merged in PR #76 (`a04c0c0`) after independent re-review of committed-baseline history,
  real `should_*` Python discovery, frontend test coverage, and collector composition. Final
  quality gates and isolated G5 passed: 239 Rust, 45 Python, and 84 Node tests. G5 is the first
  passing hard gate; every other gate and all objectives remain unavailable on main. Later
  collectors own disjoint metrics in `eval/inputs/<suite>/<collector-id>.json`; see `eval/README.md`.
  Stale evidence affects only its collector, and duplicate ownership fails even for stale inputs.
  Run relevant suites before/after subsequent implementations. Final numerical acceptance adds
  `--require-complete` to `eval all --check`; buildout checks never count unavailable metrics as
  passed. Every first-parent baseline transition is audited, so committing weaker values cannot
  bypass the ratchet. Branch/worktree removed after merge.
- #69 tooling merged in PR #75 (`b8baa7b`), independently cleared after three fixes and tested
  against current main: 41 offline Python tests plus all 200 Rust tests, formatting, and Clippy.
  See `scripts/corpus/README.md` for selection, background launch, resume, and verification.
  K0 does **not** exist yet: designated storage and arXiv HTTP access remain blocked. The issue's
  tooling is complete; the Phase A requirement for both verified tiers is not. Corpus-dependent
  detector/resolver measurements must wait for actual truth. Selection memory scales with the
  metadata harvest; measure peak RSS on the first live harvest and consider bounded per-stratum
  heaps with streaming metadata hashing if needed. Branch/worktree removed after merge.
- #24 merged in PR #77 (`86e7c75`). `objects::ObjectsArtifact` wraps the existing schema-6
  figures/tables and uses the reading-index document ETag as `reading_index_generation`.
  `ReadingIndexAnchor` holds half-open UTF-16 ranges; caption anchors and membership spans remain
  separate, and mentions retain source rectangles. `GET /api/papers/{id}/objects` shares existing
  index jobs; `objects-enrichment.json` is reserved separately. The existing `test:api` frontend
  entry point includes object client tests. No detector or reading-index schema changed; O1/O2
  remain unavailable until K1. Independent review cleared final head `742426b`; 200 Rust tests,
  frontend checks/build, and targeted tests passed. Branch/worktree removed after merge.
- #34 merged first in PR #74 (`4ff924b`). `WorkId`/`PersonId` are validated opaque W/P tokens;
  the store must allocate once and preserve allocations in canonical records. `AliasMap<I>` is a
  cycle-safe projection with `merge(absorbed, surviving)`, `resolve`, and `links`; persistence and
  replay remain #33/#39. Identifiers and alias maps serialize in sorted order. Authorship positions
  are zero-based. Person identifiers are distinct from Work identifiers. Citation anchors explicitly
  distinguish layout-token anchors from reading-index generation plus half-open UTF-16 ranges.
- Workflow deviation: automatic approval review rejected rebasing twice despite this task's
  instruction to rebase. Use a normal merge from current `origin/main` before rerunning gates;
  this preserves history and was approved. Keep merge commits for PR integration. PR #74 passed
  independent review at `b779118` and all quality gates (186 unit + 2 integration tests).
- Main's working tree has uncommitted changes to `web/src/App.tsx`, `HomePage.tsx`,
  `PaperPreview.tsx`, `pdfPreview.ts`, and new `pdfPreviewCache.ts`/`pdfPreviewStorage.ts`. Those
  are unrelated and not on `main`; E0.4 (Phase B) will likely conflict with them once committed.
- Live provider hosts were blocked in an earlier development sandbox. Adapter tests are fixture
  based; keep new provider code the same way.
- #101 merged in PR #104 (`a54de426d6691c013e5ea4b3333e7027d7103335`), reviewed head
  `84a04d7`, measured source0722e9a. On unchanged two-paper `k1-limited-v1`, **O1=1.0
  (15TP/0FP/0FN), O2=0.9107793204006069** across all15 original regions, zero unknowns.
  One absent region remains zero; this passes the median target, not every-region perfection
  or the expanded500-paper target retained by#97. G5 **350Rust/282Python/85Node** and all
  required integrated gates pass. See `docs/experiment-reports/2026-09-13-figure-detection.md`.
  API/metric callers now share async `objects::from_source`, versioned native/PDF/tool/graphics
  identity, bounded image traces and native fallback. #25 must preserve this factory and compact
  native commitment when integrating its bibliography artifact. All old measurements remain;
  portable raw evidence is under `~/.cache/lysilogy/figure-detection-development/release-281785b/`.
  Worktree and both branches removed; main's ten preview files are unchanged.

---

## Phase B: resolution and paper objects

Goal: local papers populate a resolved KB with citation evidence, and the reader shows a Figures
tab with equations, statements, and proofs.

Exit criteria: ingesting the local library and the arXiv scale tier produces Works, Persons, and
citation edges; the KB API serves Works, Persons, and neighborhoods; the Figures tab replaces
Glossary; new link hints work. Scorecard: G1, G3, G4, G5 pass; O1–O15 and O25–O28 are at target, or
each miss has a measured gap, a recorded next idea, and a follow-up issue.

### Wave B1

- [ ] **#26 E1.3 Numbered equations** (after #24, #70). Branch `feat/e1.3-equations`.
- [ ] **#27 E1.4 Statements and proofs** (after #24, #70). Branch `feat/e1.4-statements-proofs`.
- [ ] **#28 E1.5 Algorithms and listings** (after #24, #70). Branch `feat/e1.5-algorithms`.
- [ ] **#30 E1.7 Object enrichment** (after #24, #70). Branch `feat/e1.7-object-enrichment`. Follow the
  stage-cache keying and exact-quote verification used by key quotes in `src/analysis/`.
- [ ] **#31 E1.8 Figures tab** (after #24). Branch `feat/e1.8-figures-tab`. The Glossary wiring to
  replace is in `App.tsx` (`ViewMode`, `VIEW_ORDER`, `openGlossary`, command allowlist, `onGloss`
  props, tab button, render branch) and `useGlobalKeys.ts`.
- [ ] **#38 E2.6 Resolution engine** (after #33, #35, #36, #37, #72). Branch `feat/e2.6-resolution`.
- [ ] **#39 E2.7 Decision log** (after #33, #34). Branch `feat/e2.7-decisions`.
- [ ] **#21 E0.3 Incremental catalog scan** (after #19). Branch `feat/e0.3-incremental-scan`.
- [ ] **#22 E0.4 Home virtualization** (after #19). Branch `feat/e0.4-home-virtualization`.
- [ ] **#23 E0.5 Concurrent batch ingest** (after #19). Branch `feat/e0.5-batch-ingest`.

E1.3, E1.4, and E1.5 each own one detector file and share only registration in
`src/objects/mod.rs`.

### Wave B2

- [ ] **#29 E1.6 Link hints for new objects** (after #26, #27). Branch `feat/e1.6-object-hints`.
- [ ] **#40 E2.8 Ingest local papers** (after #20, #25, #38, #39). Branch `feat/e2.8-ingest-local`.
- [ ] **#41 E2.9 Ingest provider snapshots** (after #38, #39). Branch `feat/e2.9-ingest-providers`.

### Wave B3

- [ ] **#42 E2.10 KB API** (after #40). Branch `feat/e2.10-kb-api`.

### Phase B notes

_Record here as work lands._

---

## Phase C: acquisition, lists, and routes

Goal: references can be fetched incrementally to mapped local copies, Works and Persons have
pages, and reading lists exist with a keyboard view.

Exit criteria: "fetch all references" on a mapped paper advances each reference through recorded
stages with verified downloads; `#work=`, `#person=`, `#list=` routes work; lists can be built and
reordered by keyboard; citations export as CSL-JSON, BibTeX, and RIS. Scorecard: G2 passes; O16–O19
and O21 are at target or have recorded gaps and follow-up issues.

### Wave C1

- [ ] **#44 E2.12 Routes plus Work and Person pages** (after #42). Branch `feat/e2.12-kb-routes`.
  Replace the `selectedId === null` home test with a discriminated route; keep `#home` and
  `#paper=` working.
- [ ] **#45 E3.1 Acquisition state and global jobs** (after #33, #34). Branch
  `feat/e3.1-acquisition-jobs`. `src/jobs.rs` is paper-scoped; model the global runner on the
  reader-tools job pattern rather than forcing `JobTracker` to hold non-paper keys.
- [ ] **#52 E4.1 CSL-JSON, BibTeX, RIS** (after #34). Branch `feat/e4.1-citation-export`.
- [ ] **#55 E5.1 Reading list model** (after #33, #34). Branch `feat/e5.1-reading-lists`.
- [ ] **#59 E6.1 Neighborhood and metrics** (after #42). Branch `feat/e6.1-graph-metrics`.
- [ ] **#73 E8.6 Reading-list evaluation** (after #68). Branch `feat/e8.6-list-eval`.

### Wave C2

- [ ] **#46 E3.2 Deterministic identifier resolution** (after #45, #63, #71). Branch
  `feat/e3.2-identifier-resolution`.
- [ ] **#48 E3.4 Agent fallback** (after #45). Branch `feat/e3.4-agent-fallback`. Reuse
  `AnalysisService::reader_tool` with `live_web`; treat reference text as untrusted input.
- [ ] **#49 E3.5 Download verification** (after #45). Branch `feat/e3.5-download-verification`.
  Downloads go through `AppState::import_remote_pdf`.
- [ ] **#50 E3.6 Post-download policy** (after #45, #23). Branch `feat/e3.6-download-policy`.
- [ ] **#56 E5.2 Reading-list view** (after #44, #55). Branch `feat/e5.2-list-view`.

### Wave C3

- [ ] **#47 E3.3 Open-access locators** (after #46). Branch `feat/e3.3-oa-locators`. Unpaywall
  requires a contact email; reuse `LYSILOGY_CITATION_MAILTO`.

### Wave C4

- [ ] **#51 E3.7 Fetch UI** (after #42, #46, #47, #48, #49). Branch `feat/e3.7-fetch-ui`. Migrate
  existing `SavedReference` records in `reader-tools.json`.

### Phase C notes

_Record here as work lands._

---

## Phase D: graph, AI lists, and refinements

Goal: the literature becomes explorable: graph view, AI-generated reading lists with visible
relationships, review queue, styled citations, and read-next suggestions.

Exit criteria: the graph view navigates a neighborhood entirely by keyboard; an AI-generated list
resolves proposals, marks unresolved ones, and shows its relationship graph; the review queue
writes decisions; styled citations render for the target styles. Scorecard: all hard gates pass;
O20, O22–O24, and O29 are at target or have recorded gaps and follow-up issues.

### Wave D1

- [ ] **#32 E1.9 Clickable proofs** (after #27, #31). Branch `feat/e1.9-clickable-proofs`.
- [ ] **#43 E2.11 Review queue** (after #42, #44). Branch `feat/e2.11-review-queue`.
- [ ] **#53 E4.2 Styled citations** (after #52). Branch `feat/e4.2-styled-citations`. Spike
  hayagriva vs citation-js first; choose on rendered correctness for APA, Chicago author-date, and
  IEEE across article, conference, preprint, and book fixtures, and record the evidence.
- [ ] **#54 E4.3 Copy and export commands** (after #52, #55). Branch `feat/e4.3-citation-commands`.
- [ ] **#57 E5.3 AI-generated reading lists** (after #45, #46, #55, #73). Branch `feat/e5.3-ai-lists`.
- [ ] **#60 E6.2 Graph view** (after #44, #59). Branch `feat/e6.2-graph-view`.
- [ ] **#64 E7.2 Semantic Scholar recommendations** (after #42, #63). Branch
  `feat/e7.2-s2-recommendations`.
- [ ] **#65 E7.3 Author identity** (after #38, #63). Branch `feat/e7.3-author-identity`.

### Wave D2

- [ ] **#61 E6.3 Keyboard graph navigation** (after #60). Branch `feat/e6.3-graph-keys`.
- [ ] **#62 E6.4 Communities and read-next** (after #59, #60, #71). Branch `feat/e6.4-read-next`.
- [ ] **#58 E5.4 List relationship graph** (after #56, #59, #60). Branch `feat/e5.4-list-graph`.

### Phase D notes

_Record here as work lands._

---

## System acceptance

The project is done when every issue is closed and this scenario works end to end on a real mapped
paper from `local-articles`, verified by an agent driving the running app (Playwright against the
built frontend and a live backend) and recorded in a final report
`docs/experiment-reports/<date>-kb-system.md` with screenshots:

1. Open a mapped paper; `g` opens Figures with ranked figures/tables, quoted context, equations,
   and a theorem whose proof expands and jumps.
2. `f` follows an equation mention and a citation; `Ctrl-o` returns.
3. Fetch all references: each reference shows its stage; at least one open-access reference
   reaches `mapped` through a verified download, and unavailable ones state why.
4. Open a cited Work page and its first author's Person page; name variants from different papers
   are resolved to one Person.
5. Generate an AI reading list from a prompt; unresolved proposals are badged; reorder, mark read,
   and export the list as BibTeX by keyboard.
6. Open the graph focused on the list or paper; navigate nodes by keyboard; read-next suggestions
   cite the local papers that justify them.
7. `kb rebuild` from scratch reproduces the same entity IDs and aliases.
8. `lysilogy eval all --check` passes: every hard gate (G1–G5) holds, at least 80% of objectives
   (O1–O30) meet their targets, and every missed objective has its measured value, the best
   attempted approach, and an open follow-up issue.
9. The same scenario (steps 1, 2, 4, and 6) works inside a data root that maps the 10k arXiv scale
   tier, with scale objectives O25–O29 met.

Live model and provider calls are allowed for this verification but kept small: at most three
papers for enrichment, one AI list, and never a `full_model` batch.

## Session log

Long runs span sessions and context compactions. This file is the source of truth for progress.
At the end of every session, or before a context compaction is likely, append one entry:
date, last merged issue, in-flight branches and their state, next action, and open problems.

### 2026-09-12 — initial autonomous build orientation

- Last merged issue: none from implementation; plan PR #66 was already merged as `d2dd6eb`.
  Main and origin/main matched at orientation. No open PRs or prior session entries existed.
- Current phase/wave: A/A1. Implementers are running in
  `.worktrees/feat/e2.2-kb-types` (#34), `.worktrees/feat/e8.1-eval-harness` (#68), and
  `.worktrees/feat/e8.2-arxiv-corpus` (#69). No implementation PR has opened yet. Merge #34 first.
- Build resource settings: one Cargo build job per worktree, debug information disabled for dev
  and test profiles, incremental compilation disabled. Targets stay in their worktrees on disk.
- Background downloads: none. Creating `~/.cache/lysilogy/` and `~/Corpora/arxiv/` failed with
  `Read-only file system` even after an approved sandbox escalation. Do not relocate the corpus
  into the repository or `/tmp`. Writable access to the designated roots is required for corpus
  and synthetic-scale measurements. The first OAI-PMH HTTP probe also encountered a sandbox
  domain-allowlist block; the corpus implementer is testing an authorized escalation.
- Next action: finish the three implementations, run independent PR reviews and quality gates,
  merge #34 first, then continue A1 (#24, #19, #20, #63). E8.1 can use an unprivileged network
  namespace for G5; `unshare --user --map-root-user --net true` succeeded.
- Main's unrelated changes remain untouched, including `.gitignore`, `web/package.json`, the
  PDF-preview files listed in Phase A notes, and the new PDF-preview test/smoke scripts. Existing
  unrelated `/tmp/lysilogy-*` worktrees remain untouched.

### 2026-09-12 — first foundation merged

- Last merged issue: #34, PR #74, merge commit `4ff924b`. Independent reviewer found no unresolved
  findings; all 188 tests, formatting, and Clippy passed after updating from main. Implementation
  branch and worktree were removed, reclaiming its target directory.
- Current phase/wave: A/A1. #68 (`feat/e8.1-eval-harness`) and #69
  (`feat/e8.2-arxiv-corpus`) are still implementing/testing in their `.worktrees/feat/` paths;
  neither has a PR yet. Next implementation is #24 (`feat/e1.1-paper-objects`), then #20/#63/#19
  as capacity permits. Do not advance to A2 before A1 finishes.
- Scorecard: not yet available; #68 is building the harness. No measured objective misses or
  follow-up issues yet. No hard gate has been weakened.
- Background downloads: none. Corpus/cache storage remains read-only after approved escalation;
  OAI-PMH remains blocked by the domain allowlist after escalation. A pending user question requests
  writable `/home/tjmisko/Corpora` and `/home/tjmisko/.cache/lysilogy` plus access to
  `oaipmh.arxiv.org`, `export.arxiv.org`, and `storage.googleapis.com`. Offline implementations
  continue. No credentials or sudo are needed for the implemented foundation.
- Next action: independently review #68/#69 when ready, and implement the remaining A1 issues.
  Use the documented normal-merge alternative to the rejected rebase. Preserve all unrelated
  main-checkout changes; their SHA-256 fingerprints are in `/tmp/lysilogy-preview-before.json`.

### 2026-09-12 — A1 implementation and review checkpoint

- Last merged issue remains #34 / PR #74 (`4ff924b`); current phase/wave A/A1.
- In flight: #24 (`feat/e1.1-paper-objects`) is implementing in its corresponding
  `.worktrees/feat/` directory; #68 (`feat/e8.1-eval-harness`) is completing final quality and G5
  gates, with no PR yet. #69 is draft PR #75 (`feat/e8.2-arxiv-corpus`, initial head `4af7da6`).
- PR #75 independent review found three required fixes, now assigned to its implementer:
  retain the earliest harvest response day across midnight; verify selection/manifest/pinned
  upstream identity and hashes together; persist Retry-After cooldown before any sleep or final
  error. All are reproducible with offline fixtures. Initial 33 Python tests and Rust gates passed.
  Re-review the fixes before merge. The unmeasured whole-harvest selection memory cost is an
  optional improvement; streaming selection is the next idea.
- #68 has demonstrated G5 in an actual isolated network namespace with no model CLIs on PATH.
  It is adding corpus Python tests and an npm PATH guard to that run, and recording dirty source
  state and content-verified measurement evidence. Provisional scorecard: G5 passes, remaining
  gates and every objective unavailable. This is an implementation checkpoint, not acceptance.
- Background downloads: none. Designated corpus/cache roots and OAI network access remain
  blocked as recorded above; the sandbox-update question is pending. Prepared launch and resume
  commands are in PR #75's `scripts/corpus/README.md`. Corpus data has never been copied into the
  repository, library, data root, or `/tmp`.
- Next action: finish/fix/review #68/#69/#24, merge when their gates and reviews pass, then
  implement #20, #63, and #19 in A1. No follow-up issues or measured objective misses yet.

### 2026-09-12 — paper-object foundation merged

- Last merged issue: #24 / PR #77 (`86e7c75`); #34 is also merged. Current phase/wave A/A1.
- In flight: #69 draft PR #75, head `780313e`, has fixes for all three review findings and
  41 passing Python tests; independent re-review is running. Its worktree remains
  `.worktrees/feat/e8.2-arxiv-corpus` until merge/cleanup. #68 draft PR #76 is fixing the ratchet
  guard so committing a weakened baseline cannot bypass checks; root fixed G5 discovery in
  `07f80fd` with three subprocess regressions, now being tested by the implementer.
- #63 is implementing in `.worktrees/feat/e7.1-provider-cache`, branch of the same name;
  it must measure O30 on a simulated 10k-reference batch and provide an eval collector. Next
  implementation is #20 content-hash identity, followed by #19 scale benchmark tooling.
- Scorecard remains provisional G5 pass, all other gates/objectives unavailable. No measured
  objective misses or follow-up issues. Background downloads: none; designated writable roots
  and arXiv network access remain blocked, with the sandbox-update question pending.
- Main's ten unrelated changed files remain untouched. Continue normal merges from main in
  place of the approval-review-rejected rebase; do not advance beyond A1 yet.

### 2026-09-12 — corpus tooling merged, live corpus still blocked

- Last merged issue: #69 / PR #75 (`b8baa7b`); #34 and #24 also merged. Current phase/wave A/A1.
  All corpus review findings were independently cleared, and the final current-main integration
  passed 41 Python and 200 Rust tests. Corpus branch/worktree removed.
- In flight: #68 draft PR #76 in `.worktrees/feat/e8.1-eval-harness`; ratchet history fix
  `58f4464` adds six CLI regression tests. Integrate current main, rerun G5 including the actual
  corpus and object tests, obtain re-review, then merge. #63 is implementing in
  `.worktrees/feat/e7.1-provider-cache`. Next ready issues are #20 and #19.
- No background downloads. The required corpus/cache directories remain read-only and OAI-PMH
  remains allowlist-blocked after approved escalation. The user sandbox-update question remains
  pending; no alternate corpus location has been used. Launch commands are now on main under
  `scripts/corpus/README.md` and should run as soon as the environment permits.
- Provisional scorecard: G5 passes; other gates and all objectives unavailable. No objective
  follow-ups yet. The last fingerprint check confirmed all ten unrelated main-checkout changed
  files remain byte-for-byte unchanged.

### 2026-09-12 — evaluation harness merged

- Last merged issue: #68 / PR #76 (`a04c0c0`); #34, #24, and #69 also merged. Current phase/wave
  A/A1. Final independent review cleared all harness findings and composition changes; merged
  branch/worktree removed. Scorecard: G5 passes (1/5 gates), 0/30 objectives available at target.
- In flight: #63 draft PR #78 (`feat/e7.1-provider-cache`, initial head `1713315`) is undergoing
  independent review, then integrating the harness and running fresh scale/tests evaluations.
  Its offline production-state simulation provisionally measures O30 at 0 violations across
  10,000 admissions; this must pass the actual collector/harness review before merge. #20
  (`feat/e0.2-content-hash`) is finalizing identity/notes/provenance regressions and integrating
  the harness before opening its draft PR.
- #19 started in `.worktrees/feat/e0.1-scale-bench`, branch `feat/e0.1-scale-bench`, commit
  `4563749`: deterministic PDF generator plus seven offline tests, including real Poppler
  parsing, pass. The timing/Playwright/extraction benchmark runner and baseline report remain
  to implement. No large fixture or Rust target has been created in this worktree yet.
- No background downloads; required corpus/cache storage and arXiv network access remain
  blocked as recorded above, with the sandbox-update question pending. No corpus or 10k vault
  has been relocated. Next action: finish/review/measure #63/#20/#19, merge eligible A1 work,
  then begin A2 only once the wave is complete. No measured objective misses or follow-up
  issues yet. Main's unrelated PDF-preview changes remain untouched.

### 2026-09-12 — provider budgets merged and cold-build correction verified

- Last merged issue: follow-up #79 / PR #81 (`1e9bc5e`), correcting G5's omitted mold aliases;
  last planned issue #63 / PR #78 (`2c21e77`). Planned issues #34, #24, #68, #69, and #63 are
  merged. Current phase/wave A/A1; #19 and #20 remain. Scorecard: G5 passes, O30 = 0 violations
  across 10,000 simulated references; 1/5 gates, 1/30 objectives at target. No objective misses.
  Follow-up #79 was opened under epic #67 and is now closed with independent review and cold
  G5 evidence. All merged branches/worktrees have been removed.
- In flight: #20 draft PR #80, branch `feat/e0.2-content-hash`, head `7e7b14f` before review
  fixes. Independent review reproduced two issues: a source changed and restored during
  extraction can publish wrong text; duplicate-move conflicts disappear on an unchanged rescan.
  Implementer is fixing both with strict before/after source stamps and persisted unresolved
  identity associations. Reviewer scratch reproductions are under that worktree's
  `target/identity-independent-review/`. Re-review and final gates are required before merge.
  Its earlier integrated gates passed 273 Rust tests, frontend gates, G5, and unchanged O30.
- #19 branch `feat/e0.1-scale-bench` is implementing the real catalog/extraction benchmark and
  Playwright home/search runner; generator commit `4563749` has seven passing tests. The branch
  contains the initial linker correction `8f626ac`; merge current main to include its final
  helper/test refinement. No PR yet. Benchmark-only four-worker extractor capacity will be
  reported separately; O27 remains unavailable until E0.5 measures the production worker path.
  O25/O26 require an actual verified 10k run, never tiny-fixture extrapolation.
- Background downloads: none. Corpus/cache roots remain read-only after approved escalation,
  and OAI-PMH remains blocked by the runtime domain allowlist. The sandbox-update question is
  pending. No corpus or full synthetic vault exists, and none has been relocated. A1 cannot
  complete its benchmark acceptance until the designated cache location is writable.
- Next action: finish and independently re-review #20, finish/review #19 tooling and run its
  real 10k baseline when storage permits, then advance to A2. Preserve main's ten unrelated
  PDF-preview files; the last hash check found no changes. An attempted npm cache directory in
  main was safely removed by its creator; use existing dependencies via worktree symlinks.

### 2026-09-12 — content identity merged, benchmark is the remaining A1 issue

- Last merged issue: #20 / PR #80 (`94ddacc`). Both review reproductions were independently
  rerun against the fixed code and cleared. Final gates passed 275 Rust tests, frontend checks
  and targeted tests, G5, and O30. Branch/worktree removed; retained evidence remains in Git.
  Current phase/wave A/A1. Planned issues #34, #24, #20, #63, #68, and #69 are merged; only #19
  remains in A1. Follow-up #79 is also merged and closed. Scorecard unchanged: 1/5 hard gates,
  1/30 objectives at target; other metrics unavailable, no measured objective misses.
- In flight: #19, branch `feat/e0.1-scale-bench`, worktree at the corresponding `.worktrees/feat/`
  path; no PR yet. It includes current content identity and the final G5 linker correction.
  Runner code measures discovered and populated catalog rescans separately, real extractor
  capacity, and actual Playwright home/search against an isolated running backend. O25 publishes
  only the populated-artifact 10k case, O26 requires real 10k browser measurements, and O27 waits
  for the production ingest worker implementation. Tiny verification must never publish scale
  metrics. Source and run provenance are fingerprinted independently of O30's collector.
- Early independent generator review found two concrete storage defects: an existing matching
  file behind a symlinked parent bypasses validation, and swapping a checked parent during
  temporary publication can redirect writes. Implementer is replacing path-based operations
  with no-follow directory descriptors and adding both regressions; reuse the safe operations
  for runner output and manifest reads. Final generator/runner review and gates are still needed.
- Background downloads: none. The default 10k generation command was attempted again and
  failed with EROFS at `~/.cache/lysilogy` before creating a PDF. Corpus/cache storage and arXiv
  metadata network access remain blocked; the sandbox-update question is pending. No full
  benchmark baseline or K0 exists. Next action: finish/review #19, run actual measurements when
  the environment permits, and advance to A2 only after A1's benchmark acceptance is complete.
  The latest fingerprint check again confirmed all ten unrelated main-checkout files unchanged.

### 2026-09-12 — session checkpoint: reviewed A1 benchmark awaits external storage

- Current phase/wave: **A/A1**. Last merged issue: follow-up **#83**, PR **#84**, merge
  `481e911`; last planned issue **#20**, PR **#80**, merge `94ddacc`. Planned A1 issues
  **#34, #24, #20, #63, #68, and #69** are merged. Follow-ups **#79 and #83** were opened,
  independently reviewed, fixed, and closed. Only **#19** remains in A1. No Phase A exit or
  final-system acceptance has passed; Phases B–D have not begun.
- In flight: **draft PR #82** (`https://github.com/tjmisko/Lysilogy/pull/82`), issue #19,
  branch **`feat/e0.1-scale-bench`**, final pushed head
  **`65d2c418bd63f5f07138169b09b5f2d2833b6a91`**. Retained clean worktree:
  **`/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e0.1-scale-bench`**. It includes both
  foundation corrections and main's O27 sequencing clarification. All other task branches and
  worktrees were removed after merge. Leave unrelated `/tmp/lysilogy-*` worktrees untouched.
- PR #82 has **no unresolved code-review findings**. Independent review cleared deterministic
  generation, descriptor-relative storage, both controlled symlink races, exact truth/inventory
  checks, actual populated catalog timing, browser timing boundaries, and collector provenance.
  The independent eight-paper live Playwright smoke passed with eight real extraction artifacts,
  three fresh browser contexts, and sixty checked searches. This is functional evidence only.
  Final integrated checks passed formatting, strict Clippy, `eval scale --check`, and G5:
  **279 Rust, 69 Python, and 85 Node tests**. Frontend checks/build passed; no frontend source
  changed in the subsequent lock correction. Reports/evidence are committed in PR #82, including
  `docs/experiment-reports/2026-09-12-e0.1-synthetic-benchmark.md` and
  `eval/evidence/synthetic-benchmark-fixture.json`; the original browser observations retain
  their original measured source, and integration results have a separate receipt.
- Scorecard: **1/5 hard gates passing (G5), 1/30 objectives at target (O30 = 0 violations over
  10,000 simulated references)**. Other metrics are unavailable, not failed measurements or
  passes. No measured objective misses or objective follow-up issues yet. No hard gate or target
  was lowered. O25/O26 require the actual populated 10k run. O27 first measures the production
  four-worker path in E0.5/B1; its ≥0.70 target and final-system requirement are unchanged.
- **Blocking environment conditions:** creating `/home/tjmisko/.cache/lysilogy` and
  `/home/tjmisko/Corpora/arxiv` returned `Read-only file system` even after approved escalation.
  The default 10k benchmark command was attempted and failed before creating any PDF.
  HTTP access to `oaipmh.arxiv.org` was blocked by the runtime domain allowlist after escalation.
  The pending user question requests writable `/home/tjmisko/.cache/lysilogy` and
  `/home/tjmisko/Corpora`, plus corpus host access to `oaipmh.arxiv.org`, `export.arxiv.org`,
  and `storage.googleapis.com`. No credential is missing for the completed foundations.
  Do not relocate corpus/scale data into the repository, library, data root, or `/tmp`.
- **Background downloads: none. K0 and the full synthetic 10k vault do not exist.** All temporary
  tiny app processes have exited. No implementation or reviewer agent remains running at this
  checkpoint; completed agents remain available for follow-up. Do not mark #19 complete or
  merge PR #82 merely because its code and small fixtures pass.
- **Next action after the environment changes:** inspect PR #82/worktree state and synchronize
  main with a normal merge (automatic approval review rejected rebase earlier). From the retained
  benchmark worktree run `python3 scripts/bench/run.py` with its default 10,000 count, then refresh
  the provider collector and run `cargo run --offline -- eval scale --check` and
  `cargo run --offline -- eval tests --check`. Retain the full-size report, review measured
  O25/O26 baselines and any misses, rerun applicable gates, then merge with a merge commit,
  remove its branch/worktree, and tick #19. Launch the arXiv harvest/downloads in the background
  immediately when storage/host access permit, following `scripts/corpus/README.md`; preserve its
  rate policy and free-space floor. Once A1 is complete, select ready issues in A2 from the plan.
- Main's unrelated PDF-preview work remains byte-for-byte unchanged. Its changed paths are
  `.gitignore`, `web/package.json`, `web/src/App.tsx`, `web/src/components/HomePage.tsx`,
  `web/src/components/PaperPreview.tsx`, `web/src/lib/pdfPreview.ts`, plus untracked
  `web/scripts/pdf-preview-cache.test.mjs`, `web/scripts/pdf-preview-smoke.mjs`,
  `web/src/lib/pdfPreviewCache.ts`, and `web/src/lib/pdfPreviewStorage.ts`. Original fingerprints
  are in `/tmp/lysilogy-preview-before.json`; continue to preserve them and make implementation
  changes only in isolated worktrees. Use one Cargo job, debug information off, incremental
  compilation off, and worktree-local targets; reuse cached frontend dependencies without npm
  network installs or main-checkout cache writes.

### 2026-09-12 — goal continuation: storage blocker revalidated

- Previous goal turn classified as progress: foundation/fix merges and reviewed benchmark work
  changed authoritative state. This continuation revalidated the remaining external blocker;
  it did not complete another implementation or measurement.
- Current state remains A/A1, only #19 unfinished. GitHub still shows draft PR #82 at
  `65d2c418bd63f5f07138169b09b5f2d2833b6a91`; its retained worktree is clean. The preceding
  checkpoint contains the complete resume instructions, merged issues, and validation evidence.
- Fresh filesystem inspection confirms both required roots are absent. The existing ancestors
  `/home/tjmisko/.cache` and `/home/tjmisko` report `ST_RDONLY` and no write access. No benchmark
  or corpus process is running to wait on, and the required 10k acceptance remains impossible
  at the designated location. No data was relocated and no completed gates were rerun needlessly.
- Scorecard unchanged: G5 passes, O30 = 0/10k, 1/5 gates and 1/30 objectives at target; remaining
  metrics unavailable. No new merges, objective misses, or follow-up issues this continuation.
  This is the second consecutive goal-turn observation of the same environment blocker. The
  full project goal remains active; it is neither complete nor narrowed to the completed work.

### 2026-09-12 — third blocker audit: goal blocked pending environment change

- Previous goal turn classified as no progress toward feature completion: it revalidated the
  same external blocker and saved continuity, without changing the next executable action.
  This third consecutive goal turn again confirms both required storage roots are absent and
  their existing parents report `ST_RDONLY` and no write access. The blocked audit threshold is
  satisfied; mark the full project goal blocked, not complete. No objective or scope is reduced.
- Current phase/wave A/A1; last merged issue #83 / PR #84 (`481e911`), last planned issue #20 /
  PR #80 (`94ddacc`). No new merges or follow-ups in this continuation. Scorecard remains G5
  passing and O30 = 0/10k: 1/5 gates and 1/30 objectives at target; other metrics unavailable.
- Draft PR #82 remains open and reviewed at `65d2c418bd63f5f07138169b09b5f2d2833b6a91`;
  branch `feat/e0.1-scale-bench` has a clean retained worktree at
  `/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e0.1-scale-bench`. It has no unresolved code
  findings but cannot complete #19 without its real 10k baseline. No background downloads,
  benchmark processes, or corpus files exist. Main's unrelated preview changes remain intact.
- Resume requires writable `/home/tjmisko/.cache/lysilogy` and `/home/tjmisko/Corpora`, plus
  corpus host access (OAI-PMH was previously allowlist-blocked). After that external change,
  synchronize the retained branch with main using a normal merge, run
  `python3 scripts/bench/run.py` at its default 10,000 count, refresh the provider collector,
  and run scale/tests eval checks. Review the real baseline and any misses before merging
  PR #82 and completing A1. Start the corpus background harvest/download using
  `scripts/corpus/README.md` as soon as its storage/network requirements permit. Never relocate
  large data into the repository or `/tmp`. The preceding full session checkpoint records
  validation artifacts, all prior merges, and the exact protected main-checkout file list.

### 2026-09-12 — approved environment repair prepared for external execution

- The user explicitly approved writable `/home/tjmisko/Corpora` and
  `/home/tjmisko/.cache/lysilogy`, and access to `oaipmh.arxiv.org`, `export.arxiv.org`, and
  `storage.googleapis.com`; they authorized automatic setup or a script. An approved escalated
  mkdir still returned `Read-only file system`. Approval alone did not change the active mounts.
- Inspected the relevant settings in `/home/tjmisko/.codex/config.toml`: the selected
  `claude-like` profile lacks those five grants. Prepared
  `/home/tjmisko/.config/lysilogy/apply-codex-corpus-permissions.py` for execution in a normal
  terminal. It validates the exact semantic delta, preserves other config values and comments,
  checks storage writes, backs up the config, and atomically adds the approved entries. Dry-run,
  idempotence, and conflicting-entry refusal were verified without changing the active config.
- Next action: user runs
  `python3 ~/.config/lysilogy/apply-codex-corpus-permissions.py`, restarts Codex, and resumes this
  conversation. Verify the fresh session's actual writes and corpus host access before using the
  preceding benchmark/corpus resume instructions. No storage or host repair is claimed yet.
- No new merges or follow-up issues. Last merged issue remains #83 / PR #84; draft PR #82 at
  `65d2c418bd63f5f07138169b09b5f2d2833b6a91` and its retained benchmark worktree remain in flight.
  Phase A / wave A1; G5 passes and O30 = 0/10k (1/5 gates, 1/30 objectives); other measurements
  remain unavailable. No background downloads or full 10k data exist. The external environment
  repair remains the next prerequisite; the full system goal is not complete.

### 2026-09-12 — storage repaired; full benchmark running; corpus transport follow-up

- User ran the approved setup script. Fresh root-agent commands now create and fsync files in
  both designated storage roots; about 139 GiB is free. Retained implementer agents still have
  old read-only mounts, so root launches storage-dependent commands while they handle code and
  review in worktrees. The main checkout's ten unrelated preview files remain byte-identical.
- #19 / draft PR #82 is synchronized with main at `7e84828` in the retained
  `feat/e0.1-scale-bench` worktree. The real default 10,000-PDF vault now exists at
  `/home/tjmisko/.cache/lysilogy/bench-vault` (about 41 MiB of PDFs). Release/backend and frontend
  builds completed; six full extraction trials are running before the 10,000-paper persistence
  pass and browser timings. Root exec session `63200`; log
  `/home/tjmisko/.cache/lysilogy/bench-e0.1.log`; run directory
  `runs/d2019ea2409e42ceb373a98b03fb08ce`. No new objective result is published yet. Avoid source
  changes and heavy competing test/build jobs during timing. Detached subprocesses did not survive
  their tool namespace; obsolete PID records were removed. Poll the retained exec session, not a
  PID from another tool's namespace.
- Corpus attempt ended with DNS failure after 93.08 s, peak RSS 30,096 KiB, before the first
  metadata page. No background corpus process, selected K0 paper, or download exists. The log is
  `/home/tjmisko/.cache/lysilogy/arxiv-corpus.log`. Its explicit disabled proxy prevented use of
  the managed transport. A rate-coordinated root probe through the configured proxy then hit
  the runtime host allowlist, both normally and after approved escalation, even though all three
  approved hosts are present in the saved Codex config. The active proxy policy still needs a
  reload; the installed CLI supports `codex app-server daemon restart`. Do not restart the daemon
  while the full benchmark is active. Do not broaden network access or bypass the proxy policy.
- Follow-up #85 (epic #67) is implementing an explicit proxy option in
  `.worktrees/fix/e8.2-corpus-proxy-transport`. 49 offline Python tests initially passed. Independent
  reviewer found that urllib does not actually implement TLS to an HTTPS proxy; reject unsupported
  HTTPS proxy URLs rather than silently downgrading them. Review/fix remains in flight; Cargo/G5
  gates wait for benchmark timing to finish. No PR or merge yet at this checkpoint.
- Current phase/wave A/A1; last merged issue #83 / PR #84. Scorecard remains the last validated
  G5 pass and O30 = 0/10k (1/5 gates, 1/30 objectives), with other metrics unavailable. Next:
  complete/review #19's measured results and #85's fix, run gates, merge and record each; use
  existing open #21/#22 as measured optimization follow-ups if O25/O26 miss. Then start A2.
  Read-only #33 preparation identified the need for canonical mint/bind events independent of
  SQLite, retained admitted observations where mutable inputs cannot reproduce them, and deferred
  provider-specific admission to #40/#41. No A2 implementation has begun.

### 2026-09-12 — A1 complete, A2 implementations and live corpus harvest

- This resumed goal turn made progress: the full default synthetic 10k run completed, independent
  reviews and integrated gates passed, #85/PR #86 merged as `e12adc3`, and #19/PR #82 merged as
  `2683dce`. Progress notes are committed through `705b94b`. Both branches/worktrees were removed.
  The full project goal is active and unchanged; Phase A has not exited.
- **Current phase/wave: A/A2.** Three implementations have started from current main:
  #33 `feat/e2.1-kb-store` in `.worktrees/feat/e2.1-kb-store` (agent `finish_corpus_proxy`),
  #35 `feat/e2.3-names` in `.worktrees/feat/e2.3-names` (`finish_benchmark`), and
  #36 `feat/e2.4-titles` in `.worktrees/feat/e2.4-titles` (`review_ready_prs`, now an implementer).
  No PRs are open at this checkpoint. Assign a different agent to review each implementation.
  Names/titles coordinate the same cached `unicode-normalization = "0.1.25"` dependency; #33 owns
  FTS5 population and integrates #36's exact-key helper when available. SQLite's `rusqlite` and
  iterator dependencies may require fetching. One Cargo job per worktree, dev/test debug off,
  incremental off, worktree-local targets, at most three concurrent implementations.
- **Live corpus build:** after the user restarted the daemon, approved proxy access succeeded.
  Root exec session **9593** is running the reviewed corpus `run --proxy-env HTTPS_PROXY` under
  `/usr/bin/time -v`; log `/home/tjmisko/.cache/lysilogy/arxiv-corpus-proxy.log`. Poll the handle
  before concluding it stopped; do not start a duplicate build. At 2026-09-13 06:44:09 UTC it
  had **208,880 distinct metadata records**, `cs:cs:CV` complete, and `cs:cs:LG` in progress.
  No frozen selection/PDF/source existed yet; about 138 GiB remained free. Its code/config were
  loaded before worktree removal; independent audit found no subsequent worktree dependency,
  and a post-cleanup handle poll confirmed it remained live. No daemon restart or corpus process
  restart is needed now. If it actually exits, inspect its final log and resume from main using
  `python3 -u scripts/corpus/corpus.py --root /home/tjmisko/Corpora/arxiv --proxy-env HTTPS_PROXY run`.
  Keep its shared three-second arXiv budget and 20 GiB floor. Watch whole-metadata selection RSS;
  optimize only if measurement warrants it. Corpus files never enter the repo/library/`/tmp`.
- **Scale evidence:** original clean source `7e84828`; full 10k run
  `~/.cache/lysilogy/bench-vault/runs/d2019ea2409e42ceb373a98b03fb08ce` retains raw observations and
  the screenshot. Total 1,260.601 s, $0. After #85 integration, independent review reconfirmed all
  150 source hashes, all physical PDF/artifact counts, metrics, and gate logs at final head
  `8083602`. Do not repeat expensive timings without a source/measurement reason. Committed
  reports and evidence are listed in Phase A notes.
- Scorecard: G5 passes (final integrated 279 Rust / 80 Python / 85 Node tests), **1/5 gates**;
  O30 = 0 violations/10k, **1/30 objectives at target**. Measured misses are O25 = 15.503406 s and
  O26 = 1,757.1 ms first render / 163.4 ms search p95, recorded with next ideas in existing open
  follow-ups #21/#22. O27 remains unavailable despite exploratory extractor efficiency 0.855644;
  production worker measurement belongs to #23. Other metrics await implementations/truth.
  Follow-up #85 was opened and resolved during this work; no objective target or hard gate changed.
- **Next actions:** finish/review/merge #33/#35/#36 in dependency-safe order; select ready #25,
  #71 and #72 within A2 as implementation slots free. Preserve canonical minted-once identities
  through SQLite rebuilds and avoid claiming real G4 before K2 exists. Continue monitoring the
  corpus through metadata, selection, inventory and downloads while offline implementations run.
  Do not advance to A3 before A2 finishes. The user's unrelated ten preview-file changes remain
  byte-for-byte intact; original fingerprints are `/tmp/lysilogy-preview-before.json`, and the
  earlier full checkpoint lists every protected path. Unrelated `/tmp` worktrees remain untouched.

### 2026-09-13 — person parser merged; corpus selection frozen; registry permission pending

- Last merged issue: **#35 / PR #87**, merge `e066946`; final independently cleared head
  `4de6251`. All gates passed (295 Rust, 80 Python, 85 Node), and the names branch/worktree
  was removed. The source fixes and retained evidence are summarized in Phase A notes.
  Current phase/wave remains **A/A2**. This continuation made progress; the full goal remains
  active and incomplete, with no phase exit or final system acceptance claimed.
- In flight: #33 `feat/e2.1-kb-store` in `.worktrees/feat/e2.1-kb-store`, agent
  `finish_corpus_proxy`; #36 `feat/e2.4-titles` in `.worktrees/feat/e2.4-titles`, agent
  `review_ready_prs`. Neither has a PR yet. Both have uncommitted implementation work. The
  #36 independent reviewer (`finish_benchmark`) found substantive math-group, Unicode script,
  and escaped-brace key collisions; fixes and re-review remain required. Review probes are at
  `~/.cache/lysilogy/review-titles/`. Next implementation slot goes to #25 bibliography on
  `feat/e1.2-bibliography`; ready #71/#72 remain in A2. Do not begin A3 yet.
- #33 includes canonical allocation/admission records, durable retained revisions, migrations,
  transactional rebuild/query snapshots, and offline tests/benchmark tooling. Local system
  SQLite migration smoke and four Python collector tests passed; Rust has **not** compiled or
  passed because required crates are missing. The real O28 500k-Work/3M-edge benchmark has not
  run. G4 must use genuine K2 deposited records through the store's allocate/admit APIs in an
  isolated root, avoiding a dependency on Phase B's #40 ingest. Do not force WorkId/PaperId
  bindings into the K2 truth schema or fabricate PDF copies. Real G4 remains unavailable.
- **Registry permission pending:** normal and escalated Cargo fetching reached a runtime denial
  for `index.crates.io`; offline resolution lacks `fallible-iterator`. A narrow validated script
  is prepared at `~/.config/lysilogy/apply-codex-rust-registry-permissions.py` to add only
  `index.crates.io` and `static.crates.io` to the selected Codex profile. Automatic approval
  review rejected executing it because these persistent network grants need exact user
  authorization. Root explained that rejection and asked asynchronously; no answer has arrived,
  and the config has not changed. Do not bypass the rejected grants with mirrors or indirect
  execution. Continue independent work while awaiting that approval. If granted, retry the
  narrow script automatically; if the config mount remains read-only, provide its normal-terminal
  command. Save a fresh checkpoint before any daemon restart, which will interrupt tool jobs.
- **Corpus:** root exec session **9593** remains live. Full metadata harvest completed with
  **514,249** distinct records. Frozen selection has **1,000 eval**, **10,000 scale**, and
  **10,951 unique papers**; selection SHA-256
  `196b6f49520e10f0e33ef306313ec0908f9cb23ad9cca680184c059484ad71e8`.
  Disk preflight passed: 146,379,161,600 free bytes, 54,320,431,104 projected additional bytes,
  21,474,836,480-byte floor. At 07:14:38 UTC there were no PDFs/sources yet; GCS inventory was
  building. Metadata selection completed without an observed process failure; actual peak RSS
  will be available from the retained `/usr/bin/time -v` receipt on exit. Log remains
  `~/.cache/lysilogy/arxiv-corpus-proxy.log`. Poll session 9593 before concluding it stopped;
  never duplicate a live build. Resume from main with the documented `--proxy-env HTTPS_PROXY`
  run command only after a confirmed exit/restart. K0 is not yet downloaded/verified.
- Scorecard remains **1/5 gates (G5)** and **1/30 objectives (O30 = 0/10k)**. The earlier
  O25/O26 measurements remain historical baselines and measured misses with open follow-ups
  #21/#22; no old source fingerprints were re-attested after adding the parser. No new follow-up
  issues were opened in this checkpoint. All ten unrelated main-checkout preview changes were
  rechecked byte-for-byte against `/tmp/lysilogy-preview-before.json` and remain untouched.

### 2026-09-13 — missing public PDF stops corpus; availability follow-up started

- Last merged issue remains **#35 / PR #87**, merge `e066946`, progress through `26b4adc`.
  Phase/wave **A/A2**; full goal remains active. #25 now has its own worktree
  `.worktrees/feat/e1.2-bibliography`, branch `feat/e1.2-bibliography`, agent `finish_benchmark`.
  It started from `26b4adc`; baseline evaluation/implementation is underway, no PR yet.
- #36 `feat/e2.4-titles` remains in its matching worktree, agent `review_ready_prs`.
  Independent reviewer `finish_benchmark` cleared source `1558119` after reproducing and fixing
  math-argument minus, letter/digit script, and escaped-brace collisions. Final integrated gates,
  evidence review, and draft PR are still required. Review probes are retained at
  `~/.cache/lysilogy/review-titles/`.
- #33 is **paused at a clean committed checkpoint `7fed742`**, branch `feat/e2.1-kb-store`
  in `.worktrees/feat/e2.1-kb-store`; it has integrated main/#35. No PR, Rust gates or O28 run
  yet. Eight Python collector tests and the local SQLite/FTS smoke passed. Missing registry
  dependencies remain blocked on the pending exact two-host permission question described in
  the preceding entry; no user approval or config change has arrived. Do not bypass that denial.
- **Corpus exec session 9593 is terminal, exit 1** (handle confirmed). Metadata and selection
  succeeded, but inventory preparation failed with `Selected PDF unavailable in public bucket:
  1801.00600`, before any manifest/PDF/source was admitted. A new rate-coordinated OAI GetRecord
  confirms header and metadata ID `1801.00600`, title *Static Free Space Detection with Laser
  Scanner using Occupancy Grid Maps*, and `created=2020-06-30`; an exact-prefix GCS listing is
  empty. The record therefore is not evidence of a parser ID mismatch. 733 selected IDs predate
  2020 despite their OAI created years; do not rewrite upstream metadata dates without evidence.
  The original 514,249 records, 10,951-paper frozen selection and its hash in the previous entry,
  and fetched inventories remain intact. Log: `~/.cache/lysilogy/arxiv-corpus-proxy.log`.
  Final receipt: **45:13.98 wall time, 3,732,208 KiB peak RSS, $0**, no PDF/source downloads.
  There is now **no live corpus process**. Do not restart the unchanged failing command.
- Opened **follow-up #88** on epic #67 and added it to project 12. Agent `finish_corpus_proxy`
  switched from paused #33 to `fix/e8.2-corpus-availability`, creating its own matching worktree.
  The fix must qualify deterministic candidates by actual public-object availability, retain exact
  tier/category/year quotas and exclusion evidence, and provide explicit safe recovery of this
  zero-artifact frozen run. Preserve the original selection; refuse reselection after admitted
  artifacts; never silently shrink tiers or change pinned versions. Implement, test, independently
  review, and perform bounded live verification before resuming the full download. No destructive
  repair or manual selection edit has been performed.
- Current active implementations are #25/#36/#88 (three maximum); #33's target is idle. Keep one
  heavy gate window at a time. Next: merge reviewed #36, review/fix #88 and resume downloads;
  continue #25 and start remaining ready #71/#72 as slots free. Scorecard unchanged: G5 passes,
  O30=0/10k (**1/5 gates, 1/30 objectives**); historical O25/O26 misses have open #21/#22.
  New follow-up #88 is open. All unrelated main-preview edits and unrelated worktrees remain
  untouched; protected-file fingerprints are `/tmp/lysilogy-preview-before.json`.

### 2026-09-13 — title normalizer merged; remaining A2 builders

- Last merged issue: **#36 / PR #89**, merge `0987241`, independently cleared final head
  `11027dd`. Its branch/worktree was removed after root verified and archived the raw receipts.
  This continuation merged #35 and #36 and opened #88; the full goal remains active and
  incomplete. Current phase/wave **A/A2**, no phase exit report yet.
- Active #25 `feat/e1.2-bibliography` in `.worktrees/feat/e1.2-bibliography`, agent
  `finish_benchmark`: baseline completed; bibliography/backend artifact edits are underway and
  uncommitted, no PR. Active #88 `fix/e8.2-corpus-availability` in its matching worktree, agent
  `finish_corpus_proxy`: 52-test corpus/O30/G5 baseline passed, availability/recovery implementation
  is underway and uncommitted, no PR. Both began at `26b4adc` and need current-main integration
  before final gates. Next slot is #71 `feat/e8.4-reference-truth`, agent `review_ready_prs`;
  verify blockers #68/#63, create the worktree with `gh worktree create --branch`, then build
  K2/K5 first and K7 after bibliography plus scale mapping. #72 remains ready in A2.
- #33 remains paused at clean `7fed742` in `.worktrees/feat/e2.1-kb-store`. Preserve its
  `target/kb-post-checkpoint-unfinished.patch` and `target/kb-resume-notes.md`: the patch contains
  additional tests and an unfinished journal-stamp hardening attempt, with known misplaced/missing
  edits; do not apply blindly. On resume, integrate #36's title key into the FTS5 projection,
  finish/review the store, obtain actual Rust gates and the full O28 measurement, and keep G4
  unavailable until genuine K2 exists. The K2/G4 adapter contract remains provisional and must
  avoid a dependency on Phase B's #40 or fabricated local copies.
- **No corpus process is running.** Session 9593 ended with the confirmed missing GCS object
  `1801.00600`; metadata, frozen selection and inventories remain unchanged. See the preceding
  entry for exact hashes, 514,249 metadata / 1,000 eval / 10,000 scale counts and the
  45:13.98 / 3,732,208-KiB receipt. A cached-inventory audit covered 98 selected IDs across
  40 complete months, with only that ID missing among those checked. #88 will retain the seed
  and per-stratum ranking, select enough valid available objects, and freeze their versions and
  inventory evidence. Proposed explicit `recover-selection --reason …` archives the old
  zero-artifact selection before atomic replacement and refuses any admitted files/receipts/
  partials or manifest entries. Root approved this implementation design; actual corpus repair
  still awaits independent source review. Do not rerun the unchanged failing download command.
- The exact registry-host approval is still pending: root asked to add `index.crates.io` and
  `static.crates.io`; automatic approval review rejected persistent grants without explicit
  authorization. Script `~/.config/lysilogy/apply-codex-rust-registry-permissions.py` is validated
  but unapplied. No answer/config change has arrived. Continue independent work; do not bypass
  this rejection. All three implementation slots are reserved for #25/#88/#71, with #33 paused.
- Scorecard remains **1/5 gates (G5)** and **1/30 objectives (O30=0/10k)**; latest integrated
  G5 count is **305 Rust / 80 Python / 85 Node**. Historical O25/O26 misses remain assigned to
  open #21/#22. Follow-up #88 is open on epic #67/project12. The ten unrelated main-preview
  files were checked again against `/tmp/lysilogy-preview-before.json` after the merge and are
  byte-identical. Unrelated `/tmp` worktrees remain untouched.

### 2026-09-13 — reviewed corpus recovery running; truth builders underway

- Last merged issue remains **#36 / PR #89**, merge `0987241`; main progress checkpoint
  `2c3a557`. This continuation merged #35 and #36. Phase/wave **A/A2**; the full goal remains
  active and incomplete. No phase exit has run.
- **Corpus recovery is live in unified exec session `80353`**, launched from reviewed source
  `013911a` in `.worktrees/fix/e8.2-corpus-availability`. Poll that session before starting any
  corpus mutation; process listings in other tool namespaces are not authoritative. Command:
  `python3 -B -u scripts/corpus/corpus.py --root /home/tjmisko/Corpora/arxiv --proxy-env HTTPS_PROXY recover-selection --reason 'Selected PDF 1801.00600 is absent from the public GCS bucket (verified 2026-09-13).'`,
  wrapped in `/usr/bin/time -v`, appending to
  `~/.cache/lysilogy/arxiv-corpus-recovery.log`. At 07:55:19 UTC, 70 consulted inventory months
  were checkpointed; schema-1 selection remains published and no manifest exists. The exact
  original selection is archived under
  `selection-recovery/c0a206cab1d997c793bd5853d1e25c24cb5f281cb30992c3e1b0f8b9445c28bb/`.
  Do not duplicate or restart this process until its handle confirms exit. Checkpoints permit
  the identical recovery command after an actual interruption.
- #88 has **draft PR #90**, head `2802e44`, branch `fix/e8.2-corpus-availability` in its matching
  worktree, agent `finish_corpus_proxy`. Independent reviewer `review_ready_prs` cleared source
  `96b702d`; later source bytes are unchanged. Final integrated gates passed **305 Rust / 103
  Python / 85 Node**, formatting, strict Clippy, all targets, G5 and scale evaluation; O30 is
  0/10k. Evidence/report accurately mark live verification pending. Next: record recovery's
  new selection hash, exact quotas, exclusions, wall time/RSS; run a bounded
  `download --tier eval --limit 1` with the same proxy to verify both PDF and version-matched
  source. Fold those receipts into PR90, independently review the final evidence, then merge
  and resume the full corpus in a retained exec session. Do not detach a process from a short
  tool namespace. K0 remains incomplete.
- #25 `feat/e1.2-bibliography`, agent `finish_benchmark`, remains active in its matching
  worktree, with backend bibliography/schema edits underway and no PR. #71 now owns
  `.worktrees/feat/e8.4-reference-truth`, branch `feat/e8.4-reference-truth`, agent
  `review_ready_prs`, based on `2c3a557`. Commit `f83c8b4` adds cache/GraphHttp original-fetch
  provenance and a narrow provider freeze helper; 39 citation-graph tests, two helper tests
  and strict Clippy passed. Offline K2/K5/K7 builders are next. Raw provider snapshots remain
  external; only derived labels are committed. K2 must avoid system WorkId/PaperId dependencies;
  K5 uses referenced works' actual field/year/OA metadata; K7 needs #25 and real scale mapping.
- #33 remains paused at `7fed742` with its unfinished patch/resume notes preserved as above.
  Its Rust dependencies remain unavailable. Three implementation slots are #25/#88/#71;
  #72 is ready but has no active worktree. Use one heavy Rust gate window at a time.
- **Exact network permission question now covers four hosts**, superseding the earlier
  two-host question: `index.crates.io`, `static.crates.io`, `api.crossref.org` and
  `api.openalex.org`. No approval or configuration change has arrived. The provider hosts were
  each explicitly rejected by the effective allowlist; a direct GraphHttp probe also failed
  without obtaining truth. Do not bypass denied hosts or assert a missing credential without
  evidence. Automatic approval review previously rejected the two-registry-host persistent
  grant because exact user authorization was absent. The new validated, **unapplied** script
  `~/.config/lysilogy/apply-codex-kb-network-permissions.py` changes only those four entries;
  `--check` passed. An approval for only two hosts would not authorize all four. After explicit
  approval, try the exact allowed script automatically; if the Codex config mount remains
  read-only, provide the normal-terminal command. A daemon restart is needed for effective
  policy; save a fresh session checkpoint first. Approved OAI/GCS access and corpus storage
  are working; export source access awaits the bounded download.
- Scorecard remains **1/5 gates (G5)** and **1/30 objectives (O30=0/10k)**. Historical O25/O26
  misses retain follow-ups #21/#22; #88 remains open. No targets or hard gates changed. Main
  checkout still contains only the ten protected unrelated preview changes plus this docs
  checkpoint; preserve those changes and all unrelated worktrees.

### 2026-09-13 — recovery succeeded; PDF downloads live; source endpoint follow-up

- Last merged issue remains **#36 / PR #89**, merge `0987241`; prior main progress checkpoint
  `0030399`. This continuation merged #35/#36 and opened #88/#91. Phase/wave **A/A2**, full goal
  incomplete. #70 has moved into A2 for the detector/truth sequencing reason recorded above.
- **Recovery session `80353` is terminal, exit 0**: 20:25.86 wall time, 3,330,592 KiB peak RSS,
  $0 model cost. The replacement contains **1,000 eval / 10,000 scale / 10,951 unique papers**,
  136 immutable consulted inventories and 15 recorded exclusions; 14 original selected IDs were
  replaced. Metadata is unchanged, original archive bytes match their hash, and the staged
  replacement exactly matches publication. New selection hash:
  `172d18c2eeb8a640ead81f55261619800e2728553c7b25f87a191393d25b3e5f`; file hash:
  `5c2a5f7c556fadf947ac129040e00c9c4d68399ec067fddff5649179a0d1a686`.
  Recovery log `~/.cache/lysilogy/arxiv-corpus-recovery.log`, SHA-256
  `c7211489ab3a31ee10f206d25906a15bdd08476abc35f1dff8c478e27837c5ce`.
  **Artifacts are now admitted; never run selection recovery again.**
- Bounded verification session `97620` is terminal, exit 1 after downloading and verifying
  `0812.5080v5.pdf`; source `/e-print/0812.5080v5` returned HTTP 301. Receipt: 4.00 s,
  240,944 KiB peak RSS, $0; log `~/.cache/lysilogy/arxiv-corpus-bounded-verification.log`, SHA-256
  `5dfe8b3f3cefa8c5598690bf5ecd74edf50e6021bfb030aaf32e201cd8578518`. A shared-rate-coordinated
  no-follow probe showed the redirect stays on **export.arxiv.org**, to `/src/0812.5080v5` with
  no query/credentials. A direct canonical request returned HTTP 200, application/gzip,
  21,993 bytes, SHA-256 `8b95087c0ab3a43d4f021459374bc52a66a4baae9211174f83984cb12f250c1d`
  in 0.450 s. Those probe bytes were not persisted as corpus source. Effective access to all
  three approved corpus hosts is now verified; no additional host grant is needed for this fix.
- **PDF-only full-union download is live in retained TTY exec session `94057`**, from reviewed
  #88 source in `.worktrees/fix/e8.2-corpus-availability`:
  `/usr/bin/time -v python3 -B -u scripts/corpus/corpus.py --root /home/tjmisko/Corpora/arxiv --proxy-env HTTPS_PROXY download --no-sources`,
  appending to `~/.cache/lysilogy/arxiv-corpus-pdfs.log`. At 08:16:51 UTC it was progressing
  through older selected IDs. Root owns this mutation. Poll its handle; do not start another
  corpus mutator. When #91 is reviewed, root can send Ctrl-C through this TTY session to stop
  gracefully, confirm exit/checkpoints, run the bounded source verification on schema-2-aware
  integrated code, then prioritize the eval tier's sources and resume the remaining PDFs.
- #88 PR #90 remains draft at `1c401e1` before its pending live-receipt docs update. Its source
  remains reviewed; final gates at integrated `9ba9fe5` passed 75 corpus tests and G5
  305 Rust / 103 Python / 85 Node, O30=0/10k. The implementation is paused pending **new #91**
  on epic #67/project12. Agent `finish_corpus_proxy` is assigned `fix/e8.2-source-endpoint`, to
  create its own worktree from current main and implement only canonical source URLs plus
  compatibility with valid legacy receipts. It has the next heavy gate window. Preserve #88's
  worktree/target; do not change its source during the running PDF download. Independent
  reviewer `review_ready_prs` will review #91 before root live verification. Merge #91, integrate
  it into #88 with an ordinary merge, finish #90's live/evidence review and merge it, then start
  #70. No rebase is authorized under the standing restriction.
- #25 remains in `feat/e1.2-bibliography`, agent `finish_benchmark`. Source checkpoint `084a677`
  includes the production backend, schema2 and frontend links, with a successful pilot Playwright
  smoke. Independent review found DOI/newline overjoining, wrapped unnumbered-entry splitting,
  duplicate bracket-key/author-year occurrences, and a frontend figure-link fallback regression
  inside the bibliography. Fixes and regression fixtures are in the working tree, awaiting
  lightweight verification and commit; neither review group has final clearance yet. An offline
  O8/O9/O10 collector and shared K1 truth contract are being implemented. No PR/real metrics yet.
- #71 `feat/e8.4-reference-truth`, agent `review_ready_prs`, is committed through `40e1973`.
  K2/K5 builders and freeze tooling have 22 passing Python tests; K7 compact fold construction is
  underway. Root review fixed exact request/result identity validation, separate 128-MiB aggregate
  versus 8-MiB response limits, seeded stratum order and request-manifest descriptor substitution.
  Root independently reproduced the last defect before the fix and verified rejection afterward.
  K7 will withhold the held-out citing paper's edges from every normalized view and recompute
  derived graph features; only independently known mapped candidates enter the universe. Real
  K2/K5 labels still require provider access; K7 requires real mapped scale/local graph inputs.
- #33 stays paused at `7fed742` with its target notes/unfinished patch preserved. #72 is ready
  but unstarted. The exact **four-host** permission question is still unanswered; the validated
  config script remains unapplied. Do not bypass the previous automatic approval rejection.
  Active implementations are #25/#71/#91; #88 awaits verification. After recovery, memory
  recovered to about 2.7 GiB available; continue one heavy gate window at a time.
- Scorecard remains **1/5 gates (G5)** and **1/30 objectives (O30=0/10k)**. O25/O26 misses retain
  #21/#22; new follow-ups are #88/#91. No targets changed. All ten protected main-preview files
  were independently fingerprinted again and remain unchanged; unrelated worktrees are intact.

### 2026-09-13 — canonical source endpoint merged; priority eval download running

- Last merged issue: **#91 / PR #92**, merge `ca7d8d75ebc8957cd800c1567e83cbfcdc81d727`;
  final reviewed head `a540894`. Its worktree/branch are removed. This continuation has merged
  #35/#36/#91 and opened #88/#91 (91 now closed). Current phase/wave **A/A2**; no phase exit.
- **Only live corpus mutation is retained TTY exec `97363`.** It first downloads the complete
  eval tier's PDFs/sources using loaded reviewed integration code from
  `.worktrees/fix/e8.2-corpus-availability/scripts/corpus/corpus.py`, logging to
  `~/.cache/lysilogy/arxiv-corpus-eval-priority.log`. On success, the same shell checks that
  main's corpus.py SHA-256 equals
  `82991d0ad9e4c1beb277402836426a03dff6a4ed2a886bdc4524b80ef8812930`, then runs the full corpus
  `run` command from main, logging to `~/.cache/lysilogy/arxiv-corpus-full-resume.log`. Both
  stages use `/usr/bin/time -v`, Python `-B -u`, root `/home/tjmisko/Corpora/arxiv` and
  `--proxy-env HTTPS_PROXY`. A differing main source or failed eval stage stops the shell with
  progress retained. Poll this handle before any new mutation. The first command reads its
  config/code paths only at startup; root audited remaining `__file__` callsites, and reviewer
  should confirm before removing that worktree. The shell's cwd is the permanent main checkout;
  it writes no files there. Later stages use only permanent main paths.
- PDF-only TTY session `94057` is terminal after requested Ctrl-C (tool exit 1); the root lock
  was independently free and both manifest and sidecars agreed on **1,125 PDFs / 2,495,826,730
  bytes**. Its time wrapper did not retain a completion footer, so no completed wall/RSS receipt
  is claimed. Recovery `80353`, failed bounded `97620`, and successful bounded `65998` are also
  terminal. The new bounded run took **3.19 s / 240,608 KiB**, reused the PDF and admitted source
  `0812.5080v5`; actual bytes and source identity are verified. Receipt/log:
  `~/.cache/lysilogy/arxiv-corpus-source-endpoint-verification.{json,log}`. The log SHA-256 is
  `8a4d295852a8daa24801549030691e94c0c1a2b7168bdb10e91a3779c40c244a`. The standalone receipt
  records a message-only amendment from first-fetch `a975703` to identical-tree `06dc693`,
  independently verified by the reviewer. No reselection is permitted after artifact admission.
- #88 / draft PR #90 now has local reviewed integration `06dc693` in its existing worktree.
  Root resolved only documentation conflicts, retaining both fixes; all **82 corpus tests**
  and combined fmt/strict Clippy/all-targets/O30/scale/G5 checks passed. New gate logs and commands
  are under `target/kb-availability-integrated-06dc693/`; the generated scorecard is the only
  temporary dirty file. Agent `finish_corpus_proxy` is resuming #88 report/evidence work, then
  integrating this main progress checkpoint with an ordinary merge. Next: final integrated
  receipt/review, merge90, remove its branch/worktree after runtime-dependency audit, start #70.
  Running source bytes must remain unchanged. PR90's prior pushed receipt head was `78d91ce`.
- #25 branch `feat/e1.2-bibliography` is clean through `9e34011`. Parser reviewer independently
  cleared all 23 tests after corrections through `e51a865`; root independently cleared the
  frontend bibliography mask fix at `5c2d63a` and the later frontend diff is unchanged. At
  `1fe91cf`, typecheck/lint/build and production-backend Playwright smoke passed. The new metric
  collector's 13 tests pass, but review found a possible stale executable when Cargo uses a
  custom target directory; it must use/fingerprint the actual compiler artifact. That correction
  and final current-main integration/gates remain required. K1/K2 measurements are still pending.
- #71 agent `review_ready_prs` has normally integrated `ca7d8d7` into its existing branch and
  is waiting for this docs checkpoint before its serial full gate window. The prior clean
  `a3aa30b` checkpoint had 33 passing truth tests, including compact K7 mechanics, exact frozen
  provenance and K2 field-label contracts. No actual K2/K5/K7 truth is claimed. It will retain
  a clearly partial draft/report while live provider and mapped graph inputs remain unavailable.
  #25 released the build window; #71 goes next, then the short #88 integration refresh.
- #33 remains paused at `7fed742`; preserve its patch/notes, and audit canonical Person
  identifiers on resume as well as the earlier title-key/index/metadata aggregation concerns.
  #72 is unstarted. The exact four-host grant question is still unanswered and its validated
  script unapplied. No provider/registry bypass, config change or new credential requirement.
- Scorecard: **1/5 gates (G5), 1/30 objectives (O30=0/10k)**. Historical O25/O26 misses retain
  #21/#22; #88 stays open, #91 is resolved. All protected main-preview changes and unrelated
  worktrees remain untouched. Full eval/scale corpus acceptance and all later phases remain.


### 2026-09-13 — availability recovery merged; LaTeX truth starts

- Last merged issue: **#88 / PR #90**, merge `e60acb9865b045c1414e535864f3483c9a489f1b`,
  final independently cleared head `5a2fc45`. This continuation has merged #35/#36/#91/#88;
  current phase/wave **A/A2**, no phase exit. PR90's clean worktree and local/remote branch
  are removed. Main corpus.py is exactly
  `82991d0ad9e4c1beb277402836426a03dff6a4ed2a886bdc4524b80ef8812930`.
- **Only live corpus mutation remains retained TTY exec `97363`.** It downloads eval PDF/source
  pairs, currently progressing through 2021 IDs, to the external corpus root. Its loaded code
  remains valid after worktree cleanup, independently audited. The subsequent full `run` uses
  permanent main paths and its exact source hash guard now passes. Poll this handle; do not
  launch another mutator. Logs remain `~/.cache/lysilogy/arxiv-corpus-eval-priority.log` and,
  after transition, `arxiv-corpus-full-resume.log`. Prior corpus sessions are terminal as logged
  above. All three approved corpus hosts and both storage roots have effective access.
- Agent `finish_corpus_proxy` next starts **#70**, branch `feat/e8.3-latex-truth`, from this
  main checkpoint. Build independent K1 from the eval corpus; parse without executing LaTeX,
  preserve truth provenance/coverage, and use independently extracted PDF text for alignment.
  Coordinate K1's exact UTF-16 bibliography contract with #25. Three-agent O11 panel occurs
  only after actual candidates/evidence exist; no model calls in ordinary tests.
- **#25** remains clean through `97067fc` in `feat/e1.2-bibliography`, agent `finish_benchmark`.
  Parser, collector and frontend reviews are clear; collector now resolves Cargo's actual
  compiler artifact and fingerprints its executable. Next normally merge this main checkpoint,
  run full gates, record report/evidence and open a draft. **Hold merge until real K1/K2
  measurements**; no detector metric is inferred from synthetic fixtures. It owns the next
  heavy gate window. Runtime/source review notes remain in external cache.
- **#71 / draft PR #93** is safely committed/pushed at `d5e12aa` in
  `feat/e8.4-reference-truth`, agent `review_ready_prs`. Final integrated gates at `83b2244`
  pass: **309 Rust / 120 Python / 85 Node**, O30=0/10k, 33 truth tests. Actual K2/K5/K7 remain
  pending, so this partial draft must not merge. GitHub temporarily returned 502 on draft
  creation; the later retry succeeded, with no credential issue. No running build remains.
  The agent can start **#72** offline person-label tooling next; reuse the frozen provenance
  contract, keep actual labels pending until approved provider access exists.
- **#33** stays paused at `7fed742`; preserve target notes/unfinished patch and earlier review
  concerns. Implementation slots are #25/#70/#72 with #71/#33 paused. At most one heavy Rust
  gate window at a time. Use ordinary main merges; no rebase is authorized.
- Exact four-host permission question remains unanswered: `index.crates.io`, `static.crates.io`,
  `api.crossref.org`, `api.openalex.org`. The validated four-host config script is unapplied.
  Existing provider response caches are empty. Do not bypass denied hosts or infer missing
  credentials. The earlier automatic review rejected the persistent two-host registry grant
  because it lacked exact user authorization; this is separate from the approved corpus hosts.
- Scorecard remains **1/5 gates (G5), 1/30 objectives (O30=0/10k)**. Historical O25/O26 misses
  retain #21/#22; follow-ups #88/#91 are now resolved. All ten protected main-preview files
  remain byte-identical and unrelated worktrees are untouched. Full phase/system acceptance
  and later phases remain outstanding.


### 2026-09-13 — bibliography draft and independent truth checkpoints

- Last merged issue remains **#88 / PR90** (`e60acb9`); main progress checkpoint `cc329dd`.
  Current phase/wave **A/A2**. Only corpus mutator remains retained TTY exec **97363**, logging
  eval pairs to `~/.cache/lysilogy/arxiv-corpus-eval-priority.log` before its guarded permanent-main
  full run. It continues successfully; no new corpus process or reselection was started.
- **#25 / draft PR #94**, branch `feat/e1.2-bibliography`, is clean/pushed at `3ff7055`, tested
  source `b779eb1`. Both root and independent reviewer cleared that partial checkpoint: root
  verified 28 source fingerprints, 16 final gate logs, 14 G5 logs and two screenshot hashes,
  then inspected both screenshots. G5 **329 Rust / 124 Python / 90 Node**, 7.650 seconds;
  all final gates 64.534 seconds, zero model calls/cost. Parser/collector/frontend source remains
  identical to reviewed `97067fc`. Actual K1/K2 O8–O10 measurements are still required before merge.
  A newly identified test-helper issue is being fixed by `finish_benchmark`: its inherited custom
  Cargo target can differ from the hardcoded fixture executable path. Earlier default-target
  measurements remain valid; add a focused regression, refresh affected gates/evidence and get
  independent narrow review before pausing #25.
- **#70**, `feat/e8.3-latex-truth`, agent `finish_corpus_proxy`, starts from `cc329dd`; all blockers
  are closed. Evaluation-only archive/parser/aligner code and native reading-index helper are
  planned under `scripts/truth/latex/` and `examples/`. The helper will map the read-only corpus PDF
  root into dedicated `~/.cache/lysilogy/arxiv-kb-data` using canonical registry PaperIds. Preserve
  that registry across reruns. Alignment consumes raw PDF index text/tokens, never production
  object predictions. The fresh worktree's initial G5 found absent TypeScript dependencies;
  the agent is reusing installed frontend dependencies and rerunning baseline, then releases
  the heavy window for #25's narrow correction.
- Root's read-only structural survey selected 25 actual source archives across all seven eval
  strata from 400 available pairs. Results: 19 tar, five standalone, one above the survey's
  32-MiB expansion cap; 20 have bibitem syntax, 14 include .bib, 17 include .bbl, four have explicit
  bibfield/bibinfo syntax, 12 have newtheorem syntax, nine have multiple document candidates.
  Counts include comments/definitions and are **not truth labels or K1 exclusions**. No files
  extracted, no corpus mutation/network/model calls; 0.402 seconds, $0. Script/report are
  `~/.cache/lysilogy/k1-source-survey.{py,json}`. Use these cases to test source-main ambiguity,
  standalone gzip handling and explicit field-label coverage.
- **#71 / draft PR #93** remains safely paused at `d5e12aa`, tested source `83b2244`, with no
  actual K2/K5/K7 publication. Root independently reran all 33 truth tests, inspected K7
  withholding/alias/provenance behavior and the Rust timestamp path, and verified 12 source hashes,
  11 gate logs and 15 G5 logs. No additional code finding. Review receipt:
  `~/.cache/lysilogy/review-reference-truth-root.json`. Actual input acceptance is still held.
- **#72**, `feat/e8.5-person-labels`, agent `review_ready_prs`, is active. It reuses #71's frozen
  provenance contract as an explicit stacked dependency. K4 mention identity is work DOI plus
  authorship array index. Raw deposited ORCIDs back labels; profile-propagated ORCIDs alone do
  not. Preserve raw/profile values, validate checksums, exclude conflicting slots and repeated
  same-work ORCIDs, and report profile clustering conflicts separately. Official OpenAlex
  documentation supports this distinction; the design/report will cite it. Clustering features
  exclude ORCID labels and resolved OpenAlex author IDs. No actual provider access or K4 yet.
- **#33** remains at `7fed742`, with an existing uncommitted `.gitignore` `.worktrees/` addition
  preserved. Agent `finish_benchmark` has read its code/notes but will resume edits only after
  #25's helper correction. Planned bounded work: aggregate all current bound observations across
  refresh/merge/split; validate canonical Person identifiers; use shared title_key in FTS; add
  nonunique indexed identifier lookup; resolve retained journal-prefix validation patch carefully.
  Rust validation/O28/G4 remain unrun until registry access and actual K2 are available. Preserve
  `target/kb-resume-notes.md` and the unfinished patch; no blind application.
- The exact four-host grant question is still pending and the validated script remains unapplied.
  Registry/provider restrictions are unchanged. Main preview fingerprints are unchanged. Scorecard
  remains **1/5 gates (G5), 1/30 objectives (O30=0/10k)**; historical O25/O26 misses retain #21/#22.
  No new missed-objective issue or target change. At most three implementations and one heavy
  gate window; #25 transitions to #33 after its narrow correction, while #70/#72 continue.


### 2026-09-13 — fixture correction reviewed; real-source pilot diagnostics

- Last merged issue remains **#88 / PR90** (`e60acb9`); current phase/wave **A/A2**.
  Main prior progress checkpoint `9b6b3d8`. All ten protected preview files remain byte-identical.
  No additional host approval has arrived; the exact four-host grant and validated script remain
  pending/unapplied. Offline implementations continue; no provider/registry bypass.
- **#25 / draft PR94** is safely paused, clean/pushed at **`46ff41b`**. Final helper source
  **`6291b7e`** selects Cargo's actual compiler-artifact executable, including custom target
  directory and native target-triple layouts; four configuration regressions pass. Independent
  reviewer cleared source and all 79 current/historical source/log hashes. Root separately
  verified 29 current source hashes, 11 final and 11 earlier follow-up gate logs, both sets of
  14 G5 logs, and two retained screenshots. Final G5 **329 Rust / 124 Python / 91 Node**.
  Supplemental evidence `eval/evidence/bibliography-fixture-target.json` is the final checkpoint;
  `bibliography-parser.json` intentionally preserves the earlier `b779eb1` receipts. Root review
  is at `~/.cache/lysilogy/review-bibliography-root.json`. No remaining code finding; real K1/K2
  O8–O10 measurements still block merge. Keep its worktree/target for the collector.
- **#72** source **`b33e924`** in `feat/e8.5-person-labels` has independent clearance and normal
  integration of `9b6b3d8`. Agent `review_ready_prs` owns final heavy gates, retained exec **65963**.
  Initial source `9584a88`; root reproduced and reviewer checked fixes for OpenAlex profile IDs
  embedded in raw names, K4 content-version drift at the feature CLI boundary, and output above
  the shared 128-MiB derived-file bound. All **55 truth tests** pass (22 new), independently
  rerun before the final small output-bound guard. The pure feature projection remains independent
  of answer labels. No real K4/person metric or provider request; draft PR still to be opened.
  It stacks paused #71's reviewed provenance foundation, so #71 must merge first.
- Agent `finish_benchmark` has finished #25 and #72 review and now resumes **#33** in its existing
  worktree. Preserve its `.gitignore` change and target notes/unfinished patch. Await its aggregation
  design, then independently review current-bound-observation refresh/merge/split behavior,
  canonical Person identifiers, shared title keys and identifier indexing. Rust registry access
  and real K2 remain unavailable; do not claim Rust/O28/G4 success before their actual checks.
- **#70** remains on `feat/e8.3-latex-truth` from `cc329dd`, with uncommitted evaluation code under
  `scripts/truth/latex/` and `examples/k1_index.rs`. Baseline dependency issue is resolved; the
  native-index helper and its focused Rust test/strict example Clippy pass. No actual mapped
  indexing has started. Root survey and parser pilot concern the same 25 real source archives;
  the first pilot parsed seven, with every exclusion retained in
  `~/.cache/lysilogy/k1-parser-pilot.json`. This is exploratory source parsing only.
- Root independently diagnosed exclusions, retaining scripts/reports at
  `~/.cache/lysilogy/k1-exclusion-review.{py,json}` and `k1-main-evidence-review.{py,json}`.
  Three argument failures are ordinary unbraced formatting tokens; one citation uses angle-bracket
  options. Four ambiguous source-root cases include genuine main/supplement/revision documents.
  First-page PDF title evidence uniquely favors one root only for 1902.08294; two other papers
  retain title ties. 2109.10028 has byte-identical Bigdata roots, requiring equivalent resource
  closures before deduplication. All four PDFs/sources were rehashed, no files extracted, no
  network/model calls, PDF diagnostic wall 0.268 seconds. These are diagnostics, never K1 labels.
- #70 is correcting nearest-object ownership for nested labels and adding numbered-row truth for
  align/gather/eqnarray, with notag/nonumber/tag support; nested aligned/split within equation must
  remain one equation. Unknown citation-family macros make bibliography inventories non-exhaustive
  until supported. Title ties stay excluded absent unique PDF evidence. Figure/table `region`
  remains null with independent-visual-annotation-required status; caption/text boxes cannot
  establish O2. Add source/alignment regressions before admitting mapped pilot labels.
- **Only live corpus mutation is still retained TTY exec97363.** Latest read-only status:
  **1,629 PDFs / 600 complete eval PDF-source pairs / 5,686,790,413 bytes**, 1,060 scale PDFs,
  no status path problems. Full artifact verification/completion is still pending. Log proceeds
  through 2023 IDs. The queued permanent-main full run and hash guard remain as previously logged.
- Prepared offline metadata plans, with zero network/model calls: K0 Crossref seed plan
  `~/.cache/lysilogy/reference-truth-plans/k0-172d18c2-seeds-v1.json` has 30 unique citing DOIs,
  SHA-256 `6cf7c4cd0afb156aaf0c09bd4d92622d628af6fed0dbefdf2836a93492370602`, K0 origins only.
  K4 K0-only universe/request files are
  `~/.cache/lysilogy/person-truth/k0-172d18c2-{universe-v1,requests-v1}.json`: 10,951 rows,
  9,146 missing and two invalid DOI values, 1,803 unique valid DOIs, 73 requests at 25/batch.
  Actual live sequence after approval: Crossref freeze/build K2; build combined K2+K0 K4 universe;
  freeze OpenAlex once and reuse those exact snapshots for both K4 and K5. Do not execute the
  preparatory K0-only plan as final combined coverage or duplicate overlapping lookup batches.
- Scorecard unchanged: **1/5 gates (G5), 1/30 objectives (O30=0/10k)**. Historical O25/O26 misses
  retain #21/#22; no new measured miss or follow-up. Current implementations #33/#70/#72;
  #25/#71 are paused drafts. Full phase/system acceptance remains outstanding.


### 2026-09-13 — person-truth draft reviewed; store and LaTeX remain active

- Last merged issue remains **#88 / PR90**, current phase/wave **A/A2**. Main prior checkpoint
  `c269dd1`. No phase exit or new objective measurement. All grant/blocker conditions below
  remain unchanged; no user approval of the additional four hosts has arrived.
- **#72 / draft PR #95** is safely paused at clean/pushed **`c867ec6`**, explicitly based on
  `feat/e8.4-reference-truth` / PR93. Tested source **`b33e924`** is independently cleared.
  Final checks pass: **309 Rust / 165 Python / 85 Node**, 55 truth tests (22 new), O30=0/10k;
  G1.persons/O13 remain unavailable. Exec65963 is terminal. Root verified 13 current source hashes,
  eight gate logs, 15 G5 logs and both actual metadata-plan files; review receipt
  `~/.cache/lysilogy/review-person-truth-root.json`. Report/evidence are
  `docs/experiment-reports/2026-09-13-person-silver-labels.md` and
  `eval/evidence/person-silver-labels.json`. No actual K4, complete K2/K0 input set or provider call.
  Do not merge until actual label acceptance exists and PR93 has merged.
- Agent `review_ready_prs` now independently reviews **#70** committed checkpoint **`3500522`**
  while implementer `finish_corpus_proxy` continues builder/coverage receipts and a small native
  index/alignment pilot. Checkpoint3500522 has 28 Python tests and the native helper's focused
  Rust/Clippy checks passing; it does not yet contain final K1 publication/CLI/O11 panel.
  Root reproduced a math-alignment collision: case-folding matched distinct uppercase/lowercase
  equation variables with quality1.0. Owner is fixing case-sensitive equation matching and auditing
  omitted unsupported operators; independent reviewer is checking the same semantic boundary.
  Keep every rejected/partial alignment and unknown geometry explicit. No fabricated O2 labels.
- **#33** normally integrated main at **`f2da82e`**; only kb/mod.rs conflicted and both store/titles
  modules were retained. Agent `finish_benchmark` is implementing aggregation from every current
  canonical-bound observation revision, using stable observation-ID ordering instead of refresh
  time. Union identifiers/versions/copies/name variants, select known scalars and coherent parsed
  name tuples deterministically, and retain every full asserted entity/revision in derived
  provenance tables. Refresh replaces only that observation's contribution; merge/split use the
  same aggregation path. Exact same-key version/copy contradictions reject before journal append.
  Planned migration3 adds assertion/identifier projection tables plus journal validation stamp;
  rebuild existing projections on migration. Root agreed to this storage default and requested
  explicit documentation that later resolver policy may select among all retained claims.
  Existing `.gitignore` change and target notes/patch remain preserved. No Rust compile/registry
  probe or G4/O28 claim; dependency hosts remain blocked.
- **#25 / draft PR94** stays paused at `46ff41b`, final helper source `6291b7e`, held for real
  K1/K2 measurement. Root's final receipt verification uses the supplemental
  `bibliography-fixture-target.json`; the original bibliography-parser.json remains intentionally
  historical. **#71 / draft PR93** stays paused at `d5e12aa`; actual K2/K5/K7 remain unavailable.
- Only live corpus mutator remains retained **exec97363**, with eval and subsequent guarded full
  run logs/paths as recorded above. Poll that handle before any new corpus mutation. Native
  indexing is separate, writes only the dedicated cache data root, and must preserve its canonical
  PaperId registry. Current active implementations are #33/#70; #72's agent is reviewing #70.
  Scorecard remains **1/5 gates (G5), 1/30 objectives (O30)**; O25/O26 misses retain #21/#22.


### 2026-09-13 — eval corpus downloaded; store source review cleared

- Last merged issue remains **#88 / PR90**; phase/wave **A/A2**, main prior checkpoint
  `f751485`. This is a continuity checkpoint, not a phase exit. No further host approval has
  arrived: the prepared four-host grant script remains unapplied. Preserve all earlier permission
  and no-rebase instructions; no blocked host was accessed through another transport.
- The eval stage of sole corpus mutator **exec97363** finished **exit 0**: all **1,000 eval PDFs
  and 1,000 source archives** downloaded. Retained log
  `~/.cache/lysilogy/arxiv-corpus-eval-priority.log` records **1:15:30 wall / 241,008 KiB RSS / $0**.
  The queued main source hash guard passed and the same shell is now downloading the full scale
  tier into the same corpus. Full-run log remains `arxiv-corpus-full-resume.log`. Read-only status
  at this checkpoint: **2,292 PDFs / 1,000 sources / 10,000,961,400 bytes**, 1,341 scale PDFs,
  no path problems; 117 GiB free. These counts precede final artifact rehash verification.
  Do not start another corpus mutator or repeat selection recovery.
- **#33** is pushed and paused at **`905c3410cf22cb74c26aa652b2bb544cdea07c1a`** in
  `.worktrees/feat/e2.1-kb-store`. Storage checkpoint `6576c48`, ordinary main integration
  `c3f5032`, G4 adapter `045d93f`. Root independently reviewed the aggregation, identifier index,
  title-key refresh, journal-prefix checks, upgrade retry and G4 adapter. Final correction rejects
  version-pinned arXiv IDs at Work level while retaining them on WorkVersion; candidate lookup
  covers explicit Work identifiers and infers no versionless aliases. Nine new Rust scenarios
  are written but **uncompiled/unrun**. Formatting and direct Python SQLite checks pass. Root ran
  all seven rebuild-adapter tests with the reviewed #71 loader injected read-only: pass, no skips.
  Receipt `~/.cache/lysilogy/review-kb-store-root.json` records source hashes and the source review
  clearance, explicitly not Rust gate clearance. Rust/Clippy/G5/actual G4/O28 remain unavailable.
  No PR opened. Preserve unrelated `.gitignore` and the retained target resume notes/old patch;
  the old unfinished patch is superseded and must not be applied.
- **#70** remains active in `.worktrees/feat/e8.3-latex-truth`. Independent reviewer
  `review_ready_prs` cleared core source **`60c0c355558ddc7cd43f20c44512ef6316e6f2ba`** after
  48 core tests, 12 original probes and nine late probes. Corrections preserve math case/operators
  and script binding, reject hidden structural macros/conditionals/definitions and literal-body
  false objects, share a paper expansion budget, forbid reused object spans and unknown-rendering
  shortened truth, and admit bibliography field labels only from explicit deposited field roles.
  First-author ambiguity never promotes a later coauthor. Unrelated/BibTeX substring matches
  remain comparison evidence, not printed field labels. Core review receipts are under
  `~/.cache/lysilogy/review-latex-60c0c35/`. The separate builder/archive-closure scope and new
  per-kind contract are still under review; no final source/PR clearance or K1 publication.
- Actual native indexing of the original 25-paper pilot completed **186.32 s / zero network or
  model calls**, preserving the mapped PaperId registry and exact index/PDF hashes. Historical
  alignment pilot `~/.cache/lysilogy/k1-alignment-pilot.json` records **0/25 accepted**, eight
  parsed candidates at quality 0.16–0.69, wall 7.984 s; this predates later parser fixes and is not
  a metric result. Owner now runs full frozen **1,000-paper eval indexing in exec84834**, using
  the previously hashed Cargo-selected helper and absolute external cache paths. One heavy window
  belongs to this run; parser/source work can continue. Preserve its data-root registry and poll
  the owning agent/handle before any duplicate index job.
- Root approved explicit **per-kind exhaustive eligibility** for K1 while retaining overall
  paper completeness. A metric cohort requires that kind's entire independent source inventory
  and every metric-relevant link, with omitted kinds/counts/reasons visible. Unsupported semantics
  capable of hiding the kind exclude that paper/kind. Freeze cohort selection before detector
  evaluation; no shortened denominator, lower target or caption-derived O2 region truth.
  Owner is documenting the schema/design rationale and adding regression coverage before review.
- Root's read-only 1,000-source diagnostic `~/.cache/lysilogy/k1-field-markup-survey.json` found
  **79 papers with bibliography field-role syntax**, with 46 archive-bound/unsupported exclusions,
  wall 18.623 s, zero calls/$0. It rehashed actual sources and records the archive module hash.
  Syntax may occur in inactive files/definitions; these are fixture leads, never truth labels.
- Paused drafts remain **#25/PR94 `46ff41b`**, **#71/PR93 `d5e12aa`**, and **#72/PR95 `c867ec6`**
  (stacked on PR93). Their final source/evidence reviews are retained above. Real K1/K2 detector
  metrics, K2/K5/K7 and K4 inputs remain outstanding. Next: finish/review #70 builder and cohorts,
  retain full-index evidence, build actual truth/panel, then measure #25. Continue scale downloads.
- Scorecard remains **1/5 gates (G5), 1/30 objectives (O30=0/10k)**; measured O25/O26 misses retain
  #21/#22. No new measured objective miss or follow-up. Main's ten protected PDF-preview files
  independently match their original hashes. Full phase/system acceptance remains incomplete.


### 2026-09-13 — independent visual truth and three-agent panel retained

- Last merged issue **#88 / PR90**; phase/wave **A/A2**; main prior checkpoint `b93fa30`.
  No phase exit or objective-target change. Additional host approval remains pending and the
  grant script is still unapplied. #33/#25/#71/#72 remain paused at their prior reviewed heads.
- **#70** source core, builder closure and per-kind eligibility are independently cleared at
  **`d75ddbac4a7e0a478c94866f76af521b7323809b`**, pushed. The earlier `60c0c35` clearance was
  superseded after new probes found inline-math operator/script loss in prose, unsupported range
  reference omissions, local-style closure gaps, unsafe substring main-title selection and lost
  excluded inventories. All are fixed; 68 pinned tests and six independent late probes pass.
  Full original parsed object/link inventory now persists with source spans/targets and a canonical
  hash. Receipts: `~/.cache/lysilogy/review-latex-d75ddba/`. New O11 scoring commit **`b79563c`**
  has 73 offline tests passing and is being independently reviewed; final manual annotation and
  publication integration remain unimplemented/unreviewed. No PR or final K1 publication yet.
- Warm full gates at historical `60c0c35` passed **306 Rust / 166 Python / 85 Node**, retained in
  #70's `target/k1-checkpoint-60c0c35/receipt.json`. Later changes are Python-only and still require
  final full gates. Full eval native indexing **exec84834** remains live in the owner agent,
  using the unchanged hashed helper; last owner count 639 index caches, no stderr. Do not duplicate
  the run or reset its persistent mapped identity registry. Its one heavy window remains reserved.
- Root inspected all eight original pages of **2104.01511v1** and manually enclosed all **10
  figure / 5 table visual bodies**, including axes, legends and subpanels, excluding separate
  captions/prose. This is independent of detector predictions. External annotation directory:
  `~/.cache/lysilogy/k1-region-annotation/2104.01511v1/`. `regions-root-v1.json` SHA
  `85acfbfab6e3959434294d6e10d42cac62d78f34081766fd8ae1d5998196b8ef` is independently cleared;
  `review-independent-v1.json` SHA `0831bbb6bbb113acf9a10a3c44e13093c897729898b9a256a6da5dacec358bf2`.
  Reviewer viewed all pages, checked original PDF/all PNG hashes and coordinates, and independently
  rerendered byte-identical 96-dpi pages. PDF-point coordinates use top-left origin, 612×792 pages.
- Root then checked every source-caption/ID/printed-label/page association against the visuals.
  `source-associations-root-v1.json` SHA
  `6b92bc02cc2b130d7d9b58ef9a55256d73e66435aa81efb26767e1e70d1b84c0` is separately cleared in
  `source-associations-independent-review-v1.json` SHA
  `a5fef0a167bca26b5c53cb01f860640d18c62db6f4ca0a4eae758f75f58a235e`. Fresh archive parsing
  reproduces the exact frozen inventory. In particular, deposited `main.tex[31529:31841]`
  `object:boxPlot` and page 7 Figure 10 agree on the mean/comp subscripts and four violin panels.
  Automatic `unverified_script_binding` remains unchanged. The planned manual overlay must bind
  all source/index/PDF/annotation/review hashes and preserve this separate admission provenance.
  O2 is not yet measured, and these external candidates are not yet published K1 labels.
- Three fresh independent panel agents (`k1_panel_1`, `_2`, `_3`, forked without conversation
  history) read all eight original images and page text, with no detector outputs, region boxes,
  other votes, network or extra model calls. Frozen packet directory:
  `~/.cache/lysilogy/k1-panel-pilot/2104.01511v1-d75ddba/`. Packet SHA
  `9641d29f4a391de9ed9557bbe14160ac22c309d8d2a3df67a213020dc7346536`; prompt SHA
  `2606e25cd7865a12aeefb2b6d46e95e43ec6356899ef99f60e02bd49b57e2f04`. All three independently
  ranked `object:tab:lateFusionResults`, `object:timeToEvent`, `object:shapPlot` in that order.
  Raw votes `vote-evaluator-{1,2,3}.json` retain distinct identities, rationale, page hashes,
  uncertainties and timing. Root verified all receipts in `panel-root-review-v1.json`, SHA
  `59f6bff3ce21d386f86ccaec62cd14057a9b407e6e8d9d360bce7196efe0dcec`. Total agent wall time
  313.431 s; harness dollar costs unavailable, not reported as zero. No production enrichment
  was run: **O11 remains unmeasured**. This is an exploratory one-paper sample, not broad coverage.
- O11 scoring was frozen before root/owner inspected vote contents: mean top-3 set overlap divided
  by three across each independent panelist and paper, fixed nine opportunities per paper.
  Only the first three model positions count; missing/duplicate/invalid IDs are not backfilled
  from ranks 4–5. Missing model papers score zero; malformed ranks/panels fail validation.
  Target remains 0.70. External `scoring-policy-v1.json` SHA
  `18f285c9a7083946545d7094a21218289f7be5612d5f202459aadf30663c99ed`; #70 records the same design.
- **Follow-up #96** is open, linked as an E1/#12 sub-issue and added to project 12. Main has no
  O1/O2 collector beyond harness examples; #24 shipped before K1. #96 adds the missing actual
  detector/region measurement after #70, in Wave A2. It is not a measured objective miss.
- Prepared **10 local-library citing DOI seeds** under `~/.cache/lysilogy/reference-truth-plans/`.
  Agent inspected 44/124 regular PDF first pages; 32 lacked DOI evidence, one empty PDF and one
  concatenated DOI ambiguity were excluded. No vault/notes/derivative writes. Root independently
  rehashed all ten selected originals and reproduced exact first-page text hashes and title/DOI
  occurrences. `local-library-k2-root-review.json` retains this check. Local seed SHA
  `11bc709de4f5cf5882e1204d249c6fe9c8117d0fe7bd5fea29e489c617a9df3b`; audit SHA
  `a48b22e1c8b4b5de6d9c8a6f0b36a2f8747c4ab607c6e93765553b51bbfdd2d0`. The Bagley file's
  filename year conflicts with its cover; printed 2019 was retained. Provider identity and deposited
  references remain pending. Combined `combined-local-k0-k2-plan-v1.json` SHA
  `50f582ea11a0b746bcbd760a4697c7f42c91e62b33f12756cc3e62739ecdd06d` has 30 K0 plus 10 local
  origins and 40 unique Crossref requests, unexecuted. Use this combined plan after approval,
  validate returned DOI/title evidence, build K2, then freeze combined K2+K0 OpenAlex once for K4/K5.
- Sole live corpus mutator remains **exec97363**, now the permanent-main full run. Latest read-only
  status: **4,387 PDFs / 1,000 sources / 16,897,921,355 bytes**, 3,436 scale PDFs, no path problems;
  final full verification still pending. Next: finish/review manual annotation and publication,
  retain full 1,000-index evidence, measure deterministic source/cohort coverage, then actual K1
  and #25/#96 collectors. Keep all omitted kinds and source limitations visible.
- Scorecard unchanged: **1/5 gates (G5), 1/30 objectives (O30)**. O25/O26 misses retain #21/#22;
  new follow-up #96 covers unavailable measurement only. Full phase/system acceptance incomplete.

### 2026-09-13 — full eval index verified; broad truth alignment running

- Last merged issue remains **#88 / PR90**, phase/wave **A/A2**, previous main checkpoint
  `7803160`. No phase exit. Protected ten PDF-preview files still match their original hashes.
  No additional host approval has arrived. The four-host script
  `~/.config/lysilogy/apply-codex-kb-network-permissions.py` remains unapplied; it requests only
  `index.crates.io`, `static.crates.io`, `api.crossref.org`, and `api.openalex.org`. Earlier
  automatic review rejected persistent registry grants as not specifically authorized. Continue
  offline work; do not bypass that rejection. The originally approved corpus hosts/storage work.
- Native eval indexing **exec84834 completed** in **1,374.785 s**, no network/model calls/$0.
  All 1,000 PDFs have unique mapped PaperIds; **999 indexes succeeded**. Dedicated data root
  `~/.cache/lysilogy/arxiv-kb-data` and its `paper-identities.json` must be preserved. Frozen
  `k1-full-index-receipt.json` binds helper source, executable, inputs/request and output hashes;
  output SHA `40b0545ac4f34c4c1d389f3f7bbdb8b613a533cef6e566285b570ccd7ea0f785`.
  Root independently rehashed all 1,000 PDFs and 999 index files: **6,171,809,461 bytes**, all
  matched in 9.302 s. Receipt `~/.cache/lysilogy/k1-full-index-root-review.json`. Native peak RSS
  was not instrumented. The heavy build window is now free; do not repeat the full index run.
- The retained failure is **2308.05883v2**, PaperId `e723f251047f05c0`, PDF SHA
  `6470900fce80751fc31cc97401933fba56e5f2ad8848a353a7e34d6f16cc0647`. Independent investigation
  found three negative-width Poppler combining-circumflex word boxes on pages 30/34; MuPDF traces
  and a rendered crop show that swapping endpoints would invent incorrect geometry. Preserve the
  finite/ordered gate and failed-paper record. A separately tested extraction correction is
  needed before re-indexing only this paper. No PDF or source edits. Diagnostic
  `~/.cache/lysilogy/layout-diagnostics/2308.05883v2/diagnostic.json`, SHA
  `c110edb04dc9807d123679c3aebe8ad03e58b9650bda79a8bc09b87dea08d56a`, retains source/tool/glyph
  evidence; 47 pages, 19,570 words, no nonfinite boxes, qpdf check passed. This is an open coverage
  problem, not a measured scorecard miss; no follow-up issue yet.
- **#70** in `.worktrees/feat/e8.3-latex-truth` is committed through **`0910c35`**. Independent
  review cleared `b79563c` O11 arithmetic, `91a20ec` supported empty-kind negative cohorts and
  `3b40dfb` clipped citation contexts. Later `016eb31` retains uncertain labeled equation rows
  and supports single-token macro arguments; `0910c35` localizes unsupported rendering while
  preserving inventories and classifies deposited PDF sources. Reviewer `review_ready_prs` is
  examining those two latest commits. Owner `finish_corpus_proxy` is adding a separately bound
  manual region/source overlay and distinct panel identity validator; those edits are uncommitted
  and not yet cleared. No final K1 publication or PR. Historical full gates remain at `60c0c35`;
  final integrated gates are still required.
- A pinned source-only census of all 1,000 archives at historical `d75ddba` parsed 195 papers in
  82.061 s / 357,600 KiB RSS. It was diagnostic, not truth coverage. Clipped TeX contexts caused
  many whole-paper failures; four actual cases were fixed and independently reproduced. Other
  actual failures included unbraced macro arguments and bibliography href boilerplate. Some
  canonical arXiv sources decompress directly to `%PDF`; they are legitimately unsupported
  source format, not corruption to redownload. Receipts under
  `~/.cache/lysilogy/k1-source-census-d75ddba/` retain exact source hashes and exclusions.
- Owner now runs broad alignment in retained **TTY exec65518**, pinned at `0910c35`, using
  frozen 1,000 inputs and 999 successful indexes. External directory
  `~/.cache/lysilogy/k1-full-alignment-0910c35/` retains all candidates, full inventories and
  exclusions. One process, 1.5 GiB address-space bound and 30-second per-paper diagnostic timeout;
  actual PDF/source/index hashes rechecked, no network/model calls or corpus mutation. At the
  first 150 papers, 82 sources parsed and no metric cohort was accepted; this partial count is
  not final coverage. Next use the complete exclusion census to improve supported semantics and
  alignment without silently shortening any kind's independent inventory.
- After all manual truth and panel judgments were frozen and independently checked, root inspected
  the existing native detector output for 2104.01511v1: 11 figure candidates and zero tables,
  versus independent 10 figures/five Roman-numbered tables. One extra Figure 2 is a prose mention;
  detector regions also extend beyond visual bodies. Diagnostic
  `~/.cache/lysilogy/k1-detector-inventory-diagnostic.json` is not an O1/O2 measurement. Preserve
  the frozen truth unchanged; #96 must run its actual collector after #70. O11 remains unmeasured
  because no production enrichment has run.
- Paused branches/drafts remain **#25/PR94 `46ff41b`**, **#33 `905c341` (no PR)**,
  **#71/PR93 `d5e12aa`**, and **#72/PR95 `c867ec6` (stacked on PR93)**. Source reviews and
  relevant offline gates are retained in prior entries. #33 Rust dependencies and live provider
  truth remain blocked by the pending hosts. The combined 40-DOI local/K0 request plan is ready,
  unexecuted; no fake provider evidence or fixture-derived metrics may substitute for it.
- Sole corpus mutator **exec97363** is still running the permanent-main scale download. Latest
  read-only status: **6,429 PDFs / 1,000 sources / 25,580,298,883 bytes**, 5,478 scale PDFs, no
  path problems; 103 GiB free, enforced floor 20 GiB. Log
  `~/.cache/lysilogy/arxiv-corpus-full-resume.log`. Poll the retained handle; do not start a
  second mutator or repeat selection recovery. Final whole-corpus verification remains pending.
- Next: finish broad alignment and independently reviewed manual/publication integration for #70,
  publish actual K1 only with visible coverage, then measure #25/#96. Continue scale download.
  Scorecard unchanged: **1/5 gates (G5), 1/30 objectives (O30)**. Misses retain #21/#22; #96 is
  the only newly opened follow-up this continuation. Full system acceptance remains incomplete.

### 2026-09-13 — full truth coverage failure retained; independent math labels reconciled

- Last merged issue remains **#88 / PR90**, phase/wave **A/A2**, previous main checkpoint
  `d5087d9`. No phase exit or new objective target/baseline. The four additional hosts remain
  unapproved; `~/.config/lysilogy/apply-codex-kb-network-permissions.py` is still unapplied.
  It covers only `index.crates.io`, `static.crates.io`, `api.crossref.org`, `api.openalex.org`.
  Original corpus grants work. Do not bypass the earlier automatic rejection of persistent
  registry grants. Read-only Cargo cache inventory confirms no cached rusqlite version or its
  two fallible-iterator dependencies; there is no established compatible offline substitute.
- **#70** is clean through independently reviewed **`bd52610ac9d4f280c72d17b6a0c4eb40e8229add`**
  in `.worktrees/feat/e8.3-latex-truth`. Main `d5087d9` was integrated normally at `4afb9b8`;
  no rebase. Later source fixes retain starred/uncertain equations, numbering-macro exclusions,
  literal penalty parameters and the micro-sign/Greek-mu glyph alias, and withhold unverified
  semantic math alphabets. The latter fixed independently reproduced wrong-span matches for
  mathbb/mathcal/mathbf/mathsf. Standard inventory-only primitives do not imply faithful rendered
  math. Source/annotation/panel review at `bd52610` passed **107 tests and 22 independent checks**.
  Rejected panel reviews and packet image paths are now validated against actual original files.
  Actual 2104.01511 candidate reassembly preserves all automatic fields and binds 15 objects,
  three distinct panelists and eight images. Review receipts under
  `~/.cache/lysilogy/review-latex-bd52610/`; final full gates and K1 publication remain pending.
- Corrected an overly narrow O6 truth interpretation: the confirmed E1.4 design and issue #27
  explicitly link unnamed proofs to the nearest preceding statement. Commit `ef924cc` implements
  that source-structural relationship and keeps unresolved explicit headings unlinked. Provenance
  distinguishes structural attribution from an explicitly named proof. No metric target changed.
- Full historical alignment **exec65518 completed exit 0**: **626/1,000 sources parsed, zero
  accepted metric cohorts**, 1,017.604 s / 512,176 KiB RSS / zero network/models/$0. All 626 have
  unsupported inventory commands; 374 source/index failures remain explicit, including deposited
  PDF sources and two diagnostic timeouts. Complete positive text alignments before semantic
  guards occur for 126 table / 75 figure / 42 algorithm / one statement papers, zero equation or
  proof papers. Those counts are diagnostic, never accepted truth. Frozen directory
  `~/.cache/lysilogy/k1-full-alignment-0910c35/` retains all candidates/exclusions/runner/logs.
  Root independently rehashed **115,525,965 candidate bytes**, every inventory hash and all
  receipt files; all five pinned modules exactly match committed `0910c35`. Counts reproduce in
  `root-review.json`. Later fixes are absent from this historical run. The **500-paper K1 target
  remains unmet**; do not present the manual pilot as meeting it.
- A bounded independent capability review of 20 candidates across seven categories retained
  exact source/member evidence in `~/.cache/lysilogy/k1-source-capability-review/`. Standard
  math/layout vocabulary may have known inventory behavior while its text remains unsupported.
  Local class programs, dynamic aliases, invisible phantom content and hidden references remain
  excluded. In particular, 2107.12657 has an actual `captionof{table}` inside a figure: simply
  whitelisting it would silently omit a table. No source capability was inferred from predictions.
- Root and a separate annotator independently viewed all five pages of **2503.05828v1**,
  source member bodies and high-resolution details. Both confirm **four statements, two
  algorithms, one numbered equation, one proof**, plus zero figures/tables (decorative copyright
  badge excluded). Immutable evidence directory:
  `~/.cache/lysilogy/k1-manual-annotation/2503.05828/`. Packet SHA
  `977b1520af70450328eaa3660a4004465bbf0d1d9bd853953170a0a486ec2c9f`, PDF SHA
  `b63feab2a4437d86312ef59daf0b1cebb23abdfa8cb172c777d4b22e5c258384`, source SHA
  `3e7d947f340d70c356d8f1a98163e0a170abace3bf1c3adee574a59303ffa783`, PaperId
  `3a516027f3a6af6d`. Frozen historical inventory SHA
  `d9790986c1f25dcfe4a2b0c54cc71a255900a87b1916ac16870c90289b9a5741` is not replaced by new parses.
- `root-v1.json` SHA `a7c8c9b6b826447e1993cf855a9c04af9fbc6368c981a416cd9e4f42013e5a42` and
  `independent-v1.json` SHA `94692bc32ff0b8671e2cb3ffac963399a50def61b4a515185302178b3a1c38c5`
  agree on every source/body membership and all **13 object-reference number spans/targets**;
  ten statement refs are relevant to O4. The remaining source ref is explicitly section 1.1.
  Final independent comparison `reconciliation-independent-v1.json` SHA
  `1436e09691530d0ab675fac060feda48a858546911774cfe9808f683d1317f4a` has verdict
  `clear_complete_object_overlay`, no findings, and binds the original annotation/receipt hashes.
  The unnamed proof directly follows and constructs Theorem 3.1 in source and PDF. Both preserve
  actual author slips and lossy native math. This is visual/source identity evidence, not faithful
  flat-text mathematical quotation. Algorithm floats are separate source children; Definition 2.2
  uses disjoint column spans. Prefer independent direct spans plus the separate attached footnote
  and child edges. Original annotations remain immutable; no detector output informed labels.
- Root separately annotated the same paper's **35 bibliography entries, 105 first-author/title/
  year labels, 40 citation groups and 50 target pairs**, using printed fields and deposited roles.
  `bibliography-root-v1.json` SHA
  `13cc19beba4f55fb297b57289e4c135a44d11114c2d6c1e877479345292b453c` is awaiting independent
  comparison. Neutral `bibliography-packet.json` SHA
  `e6adac5c6fe250fae488c73d80a6e49fca6098e62f78d374a904fac3d5128a91` was supplied to
  `finish_benchmark`, now running blind labels. Do not expose root bibliography labels/code to
  that agent until its own artifact freezes. Source citation [13] and [27] move into footnotes;
  the explicit mapping does not assume source/printed order. Bibliographic strings remain
  transcriptions, not validated provider identifiers or person identities. No K1 publication or
  O8–O10 measurement yet. Harness judgment costs are unknown, not reported as zero.
- #70 owner `finish_corpus_proxy` is now generalizing the manual object validator and CLI. Only
  untracked `object_annotations.py` and `test_object_annotations.py` extend the reviewed head;
  **112 provisional Python tests pass**, but these edits are incomplete/unreviewed. Finish actual
  file-boundary/CLI assembly, use the accepted comparison receipt, then independent source review.
  Bibliography integration follows after independent label comparison. Keep actual labels compact
  in published artifacts and full native/source review excerpts external. No draft PR yet.
- First bounded native scale batch **exec40550 completed exit 0**: **250/250 indexes**,
  1,000.356 s wall / 194,896 KiB peak child RSS / zero network/models/$0. The initial catalog scan
  of the growing corpus was slow; summed per-paper native time was 308.249 s. All 2,388 prior
  identity records were preserved; originals and outputs were rehashed. Receipt/output in
  `~/.cache/lysilogy/scale-index-batches/batch-0001/`; output SHA
  `77336682c2412dbaa9ffc05cbb3e37c9c78afa1d1b66d2cd7e3c8344a28def0f`. Combined frozen outputs
  now have 1,249 successful native indexes, including **299 scale papers**; only the original
  failed eval PDF is excluded. This cache preparation is not an O25/O27 production benchmark.
- **Second bounded scale batch is live in TTY exec22368**, 250 further frozen PDFs, using
  `~/.cache/lysilogy/run-scale-index-batch.py --batch 2 --limit 250`. Its directory is
  `~/.cache/lysilogy/scale-index-batches/batch-0002/`. Same unchanged helper executable/source
  hashes and canonical `arxiv-kb-data` registry as the full eval index. It checks the 20 GiB
  storage floor and preserves prior identities. This batch owns the heavy window; owner expects
  more than ten minutes before final full gates. Poll before any other native/build run.
- Sole corpus mutator is still **exec97363**, permanent-main full run; latest status **9,654 PDFs /
  1,000 sources / 40,049,125,883 bytes**, 8,703 scale PDFs, no path problems, 89 GiB free. Log
  `~/.cache/lysilogy/arxiv-corpus-full-resume.log`. Do not start a second mutator or repeat
  selection recovery. Its `run` includes final artifact verification after downloads complete.
- Paused reviewed drafts remain **#25/PR94 `46ff41b`**, **#71/PR93 `d5e12aa`**,
  **#72/PR95 `c867ec6` (stacked on PR93)**; **#33 `905c341`** has no PR and Rust gates remain
  unrun. Exact source/evidence clearances and combined 40-DOI provider plan remain in prior logs.
  Next: finish/review actual manual K1 publication and broader coverage; measure #25/#96 only
  against admitted truth, continue full corpus verification/scale mapping, then remaining phases.
  Scorecard unchanged: **1/5 gates (G5), 1/30 objectives (O30)**; misses retain #21/#22, new
  follow-up #96 covers missing measurement. Full phase/system acceptance remains incomplete.

### 2026-09-13 — complete corpus verification, reconciled bibliography and page-failure follow-up

- Still **Phase A / Wave A2**. Last merged issue remains **#88 / PR90**, merge `e60acb9`;
  no additional implementation merged since the previous session entry. Main was `29578f2`
  before this documentation checkpoint. No phase exit or system acceptance is claimed.
- **Corpus construction is complete. TTY exec97363 terminated with exit 0.** The final production
  `corpus.py run` rehashed every admitted artifact and reported **10,951 unique PDFs / 1,000
  sources / 46,892,682,976 bytes**, all 1,000 eval pairs and 10,000 scale PDFs, no problems.
  Final resume plus verification took **1:45:40 / 303,808 KiB peak RSS**; preceding eval stage
  took **1:15:30 / 241,008 KiB**. No model calls or model cost. No download is running; do not
  restart completed sessions, repeat selection recovery, or relocate corpus artifacts.
- Completion report: `docs/experiment-reports/2026-09-13-kb-corpus-complete.md`.
  Full log `~/.cache/lysilogy/arxiv-corpus-full-resume.log`, SHA
  `965846a9a2197c37ae6d1d3d4259c2d1985def78ce1aaf8fb51f2dd565e33308`.
  Complete manifest SHA `b89786b72a4f12720b24c54650d36a1d2e7ac212deb6b405ee80b60c9ad4c1e0`;
  selection identity remains `172d18c2eeb8a640ead81f55261619800e2728553c7b25f87a191393d25b3e5f`.
  Root independently checked all 11,951 sidecars, IDs/strata, paths, versions, hosts and sizes in
  0.665 s; receipt `~/.cache/lysilogy/arxiv-complete-corpus-metadata-review.json`, SHA
  `2525f2047d5397950e98a83e4169bf0e3cb0ff52757d96462bc809e0fc5d4ff7`.
  This is metadata verification in addition to the production run's complete content rehash.
- **Scale batch2 exec22368 completed**: 250/250 successful, **808.854586 s / 220,816 KiB RSS**,
  all 7,198 preexisting identity rows preserved. Directory
  `~/.cache/lysilogy/scale-index-batches/batch-0002/`; output SHA
  `1db5327dd4d6f83d4e28d8ce04872e7ae9b431d135c4955eb2eb4021aef384e6`.
  Together with batch1 and the full eval run, 1,499 successful native indexes exist, including
  549 scale papers; the original failed eval paper remains an explicit failure.
- **Scale batch3 is live in TTY exec40146**, 250 further papers, directory
  `~/.cache/lysilogy/scale-index-batches/batch-0003/`, launched with
  `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 3 --limit 250`.
  It owns the single heavy window. Same helper executable SHA
  `9583e52411a0fc80466bfa4ad9f50d671d79ad8b218b9f4b5fccec506f7350c9` and helper source SHA
  `4cdce79da883df3fd656d5ba1f3dee3cbdc780532dab2f77587e20c664a4bf51` as prior runs; same
  `~/.cache/lysilogy/arxiv-kb-data` identity registry. No corpus writes, network or model calls.
  Poll completion before any other heavy native/build run. These batches are preparation,
  not actual O25/O27 measurements.
- **#70**, `.worktrees/feat/e8.3-latex-truth`, owner `finish_corpus_proxy`, is committed through
  **`9d3bd42`**. Math overlay `b3d8ff5` assembled the actual independently accepted eight-object
  bundle correctly (14 total reference roles, 13 object references, 10 O4 references), with
  original automatic exclusions unchanged. Independent review found missing ancillary/child
  ownership and invalid/contradictory reference-role inputs could pass, including a theorem
  reference relabeled as a section that reduced O4's denominator. `9d3bd42` fixes these cases;
  reviewer `review_ready_prs` has been explicitly resumed to verify the fixes. Prior probes and
  actual 23-fingerprint bundle audit are in `~/.cache/lysilogy/review-latex-b3d8ff5/`.
- **Independent bibliography reconciliation is clear** for 2503.05828v1: all 35 source keys and
  complete entry memberships, 105 field values and their 105 exact native anchors, all 40
  source-linked citation groups and 50 target pairs agree exactly. Root and independent labels
  froze separately before comparison; source/PDF/index/images and source-role payloads were
  revalidated. Directory `~/.cache/lysilogy/k1-manual-annotation/2503.05828/`:
  root `bibliography-root-v1.json` SHA
  `13cc19beba4f55fb297b57289e4c135a44d11114c2d6c1e877479345292b453c`;
  independent `bibliography-independent-v1.json` SHA
  `2d549e223a375aa02d435616382ef79823899cfd560f9d08e0fccc227088998a`;
  accepted `bibliography-reconciliation-independent-v1.json` SHA
  `c49e407ef128f7581a1c8b2ceb4dd13f418fb5a32035a0bcf3c737d5f4b9b00b`.
  Receipt retains literal names/punctuation, footnote reordering and one-paper transcription
  scope. No provider identity or detector metric claim. Owner has the accepted artifact;
  bibliography overlay and final release writer remain to implement/review. No #70 PR yet.
- **#97** was opened, linked to E8/#67 and project12, to retain the approximately 500-paper
  stratified K1 target after measured historical automatic admission **0/1,000**. A named,
  explicitly limited first release may support real baselines once every E1 kind is independently
  covered, but its size/strata/manual bias/quality remain explicit. Do not lower the coverage
  target or imply broad success from the limited cohort. See the issue and phase note for next
  source capability/independent annotation steps. Branch planned `feat/e8.3-k1-coverage`, none yet.
- **#98** was opened, linked to E0/#11 and project12, and assigned to `finish_benchmark` on
  `.worktrees/fix/e0-reading-index-page-failure` / branch `fix/e0-reading-index-page-failure`,
  based on `29578f2`. Source work is beginning; no implementation commit/PR/gates yet. It isolates
  malformed word geometry to structurally valid reading-index pages, retains original errors
  through bounded local OCR/unavailable provenance, keeps anchored parsing strict, and preserves
  valid frozen indexes. Diagnostic PDF is 2308.05883v2, PaperId `e723f251047f05c0`; retained
  evidence is `~/.cache/lysilogy/layout-diagnostics/2308.05883v2/`. Never swap endpoints or invent
  accent geometry. Source-only compatibility is expected to keep reading-index schema6; verify
  with tests and review. Ask root for the heavy window before compiling or reindexing.
- Paused reviewed drafts: **#25/PR94 `46ff41b`**, **#71/PR93 `d5e12aa`**, **#72/PR95 `c867ec6`
  (stacked on93)**. #33 remains **`905c341`**, no PR, no successful Rust compilation because the
  required rusqlite/fallible crates are absent. The approved corpus permissions are effective;
  additional grants for **index.crates.io, static.crates.io, api.crossref.org,
  api.openalex.org** are still awaiting an answer. Do not apply the prepared four-host script
  `~/.config/lysilogy/apply-codex-kb-network-permissions.py` or bypass prior auto-review rejection
  without explicit approval. No provider response cache exists; the frozen combined 40-DOI plan
  remains unexecuted. Standing rebase restriction also remains in force; use ordinary main merges.
- Next: poll batch3, complete independent #70 fixes and bibliography/release integration, then
  run exact-head/current-main offline gates and PR review. Build/test/review #98 in the next
  coordinated heavy window; retry only the failed paper while retaining its failed receipt.
  Measure #25/#96 against admitted K1, continue #97/scale indexing and the rest of Phase A.
  Scorecard unchanged: **1/5 gates (G5), 1/30 objectives (O30)**; O25/O26 misses retain #21/#22.
  Follow-ups opened this continuation: #97 coverage and #98 extraction availability; #96 remains
  measurement work. Main's ten unrelated PDF-preview files remain protected and unchanged.

### 2026-09-13 — K1 release merged; object measurement and failed-page recovery active

- Still **Phase A / Wave A2**. Last merged issue is now **#70 / PR99**, merge
  `0dc68dc5ccf3ab699126e2ca6136b0413b79f1cb`; independently cleared head
  `4563702608f7c47581c39c077058381382c2499a`. Main was fast-forwarded with all ten protected
  PDF-preview files byte-identical to `/tmp/lysilogy-preview-before.json`. No phase exit or
  system acceptance is claimed. Earlier merges this continuation: #35/PR87, #36/PR89,
  #91/PR92 and #88/PR90. Conventional commits retain the Codex co-author trailer.
- **K1 limited release is published** under `eval/truth/k1-limited-v1/`.
  Objects SHA `0afccc35dc5eedb48b4df4e30ee33e06dac02a07f0b7cab6d6f5c53743202976`;
  bibliography SHA `a821d3070768175f2de7022928a6a19f6ef8cddce9edef726448447e24fc7c82`.
  Two papers in cs.LG/2021 and econ.TH/2025 cover every E1 kind, with 15 visual objects,
  eight math/statement/proof/algorithm objects, 35 entries / 105 fields, 40 citation groups /
  50 target pairs, ten O4 pairs, nine panel scoring opportunities and one complete visual
  negative. Selection bias and all automatic exclusions remain explicit; approximately
  500-paper coverage is unmet under #97. No detector objective is inferred from publication.
- Independent final review: `~/.cache/lysilogy/review-latex-4563702/final-review.json`.
  Reviewer reproduced both output hashes in memory; rehashed all 2×1,000 input ledger
  identities and 626+622 full candidates (231,576,011 bytes), source inventories and exact
  artifact/PaperId/index bindings; verified 33 config documents, 71 raw receipts/logs and
  25 implementation hashes. Required Rust/eval gates and all 144 LaTeX tests passed.
  G5 actually ran **306 Rust / 254 Python / 85 Node** tests; O30 remains 0/10,000.
  Report: `docs/experiment-reports/2026-09-13-kb-latex-truth.md`; final evidence:
  `eval/evidence/latex-truth-release.json`. Historical checkpoint evidence remains separate.
- Current full automatic run **exec64263 completed exit 0** at reviewed automatic source
  `9d3bd42`: 622 parsed / 1,000 inputs, zero admitted cohorts, 378 source/index failures,
  two timeouts, 992.856 s wrapper time / 470,192 KiB peak RSS, zero external calls/$0.
  Directory `~/.cache/lysilogy/k1-full-alignment-9d3bd42/`; report SHA
  `24cf2c681e131575ed190ebaa9014c8ca055a7b00b0c044d4219cbe57140e1a8`.
  Root rehashed all current candidate/inventory bytes and reproduced aggregates; use
  `root-review-v2.json`, SHA
  `314436ab2f0261d660ea5b2e860b900c796e5586c077bbe96cd84e5d4bb55708`.
  v2 corrects one historical diagnostic label, retaining the first receipt. Four new
  duplicate-label rejections explain the historical 626-to-622 parsed change; no lost
  automatic admissions. #97 now records both runs and finite source-support next ideas.
- **No corpus download or native scale batch is running.** Corpus construction is complete:
  10,951 unique PDFs / 1,000 sources, all 1,000 eval pairs and 10,000 scale PDFs, final
  content verification clean. Corpus stays exclusively `~/Corpora/arxiv/`; dedicated data
  root is `~/.cache/lysilogy/arxiv-kb-data`. Do not repeat selection recovery or reset its
  canonical identity registry. Full corpus report/receipts are in the preceding entry.
- **Scale batch3 exec40146 completed exit 0**: 250/250 successful, 576.425545 s /
  235,680 KiB RSS, all 9,789 preexisting identity rows preserved. Output SHA
  `1796dd422c4971b4b229ccc6b8cc197bb4146a77092e087b392df7306777d654`.
  Batches1–3 plus full eval contain 1,750 unique requests / **1,749 successful indexes**,
  including 999 eval and 799 scale (49 overlap), retaining only the failed eval paper
  2308.05883v2. Root aggregate receipt `native-index-after-batch3-review.json` under the
  cache, SHA `e999a0e86f50ebfc58689f017361dc2be77a6f0a73da30bbae34933a714d04af`.
  This is preparation, not an actual production O25/O27 or end-to-end 10k result.
- Before #70 cleanup, root preserved the exact 8 MB standalone helper and source under
  `~/.cache/lysilogy/native-tools/9583e52411a0fc80466bfa4ad9f50d671d79ad8b218b9f4b5fccec506f7350c9/`,
  with a portability receipt. Runtime dependency audit and empty-request probe passed;
  no whole Rust target was copied. External `run-scale-index-batch.py` now uses this
  portable binary/source and an external batch cwd, with no #70 worktree dependency.
  New runner SHA `6d6bc57462b3c11b77edce2e5f8ccca09207f43ea2ff7baec35001557d1dcc34`;
  original runner is archived by hash under `native-tools/runner-history/`. Before cleanup,
  owner `finish_corpus_proxy` is archiving the remaining small gate receipts into
  `~/.cache/lysilogy/review-evidence/pr99/`; verify that archive, then remove only the owned
  clean `feat/e8.3-latex-truth` worktree/branches. No remaining #70 process exists.
- **#98** owner `finish_benchmark`, branch/worktree `fix/e0-reading-index-page-failure`.
  Source independently clear through **`5cc01999074c3ff06959479da473b50ae8f615c7`**; ordinary
  main integration is **`0843453`**. Twelve new tests and all standalone Rust/eval gates
  passed (G5 316 Rust / 110 Python / 85 Node). A compiled read-only comparison proves all
  45 valid pages of the actual 47-page PDF match strict parser words, geometry, ordering
  and sentence IDs exactly; only pages30/34 are withheld, 0.466 s. Evidence:
  `~/.cache/lysilogy/page-failure-development/after-5cc0199-1789301371330023893/compatibility-v2/`.
  No coordinate swapping, native-text invention or schema6 cache invalidation.
- **#98 owns the heavy window now** for current-main warm gates and the one failed-paper
  retry (2308.05883v2, PaperId `e723f251047f05c0`). No PR yet. All 1,749 existing indexes
  were independently hashed before retry (5.15 GB / 11.73 s), and all 10,928 registry rows
  retained in `~/.cache/lysilogy/page-failure-development/actual-retry/`. Verify these remain
  unchanged after retry; preserve the original failed map/receipt. Freeze the new standalone
  helper and exact Cargo-selected build fingerprints for later batches. `finish_corpus_proxy`
  will independently review the actual retry, integrated gates and eventual draft PR.
- **#96** owner `review_ready_prs`, fresh branch/worktree `feat/e1.1-object-metrics` from
  `0dc68dc`, implementing the frozen collector/contract and offline tests. No PR or metric
  yet. O1 matches kind and independently printed caption identity/membership one-to-one,
  never region overlap; duplicates/unmatched predictions are FP and missed truth FN.
  O2 includes all independently region-annotated truth objects, with missed/invalid regions
  scoring zero. Preserve exact index/PDF/source/objects generation and registry bytes.
  Source/light Python work may proceed; coordinate with root before compilation/measurement.
  #97 branch `feat/e8.3-k1-coverage` does not yet exist; starts after #98 review frees a slot.
- Paused drafts remain **#25/PR94 `46ff41b`**, **#71/PR93 `d5e12aa`**, **#72/PR95 `c867ec6`
  (stacked on93)**. #25 can now run K1 O8/O10 measurements but still needs genuine K2 for
  complete O9. #33 remains **`905c341`**, no PR, source reviewed but Rust uncompiled because
  rusqlite/fallible dependencies are absent. No genuine provider cache exists. The combined
  frozen 40-DOI Crossref plan remains unexecuted under `~/.cache/lysilogy/reference-truth-plans/`.
- Original corpus storage/three-host grants are effective. Additional grants for
  **index.crates.io, static.crates.io, api.crossref.org, api.openalex.org** remain pending,
  with no user answer. Do not apply `~/.config/lysilogy/apply-codex-kb-network-permissions.py`
  or bypass the earlier auto-review rejection without explicit approval. Standing rebase
  restriction remains in force; integrate main using ordinary merge commits. Never read
  denied private paths or environment/secrets files. Main is docs-only; never stage its
  unrelated PDF-preview changes. No destructive vault/data operations are authorized.
- Next: finish #70 evidence archive/cleanup; complete and independently review/merge #98;
  let #96 build and measure, then resume bounded scale batch4 with the new helper and start
  #97 finite source-support experiments. Run #25's real K1 collector when its short window
  is available. Continue the remaining phases after A2 blockers clear. Scorecard remains
  **1/5 gates (G5), 1/30 objectives (O30)**; historical O25/O26 misses retain #21/#22.
  Follow-ups opened this continuation are #97 and #98; #96 was already open. System
  acceptance and final `docs/experiment-reports/<date>-kb-system.md` remain outstanding.

### 2026-09-13 — page recovery merged; first real bibliography baseline retained

- Still **Phase A / Wave A2**. Last merged issue is **#98 / PR100**, merge
  `46f11e0357a4282e4a94c8dce17765272e5ae834`, exact reviewed head
  `8f2725367fd7478ae4da80aa4eae7ff3561718d0`. #70/PR99 merged earlier this checkpoint at
  `0dc68dc`; #35/PR87, #36/PR89, #91/PR92 and #88/PR90 also merged this continuation.
  No phase exit or system acceptance. Main remains docs-only with ten protected preview
  files unchanged; compare `/tmp/lysilogy-preview-before.json` before any main staging.
- #98's report/evidence are committed: `docs/experiment-reports/2026-09-13-kb-reading-page-failure.md`
  and `eval/evidence/reading-page-isolation.json`. Eleven new test functions were counted
  independently (6 layout + 5 page-failure); earlier references to twelve were corrected.
  Final required gates pass; G5 actually ran **317 Rust / 254 Python / 85 Node**.
  Final independent review is `~/.cache/lysilogy/review-page-failure-0843453/final-review.json`,
  SHA `57816668c1d17a5d943729ece924395ea9a6cf8f84389f8f877c082a086eab8d`.
- Actual retry ran once for 2308.05883v2 / PaperId `e723f251047f05c0`: 47 pages / 18,962 tokens,
  45 native + 2 OCR, original native error gaps on pages30/34 retained. New index SHA
  `8b3e01363ad7383cd8019f71d2e9818e8bc447bef799b0ece82b2c6bd71e85b8`.
  Processing 3.272503362 s, helper including catalog scan 10.924505562 s, peak 47,344 KiB,
  zero network/model calls/$0. All 1,749 old index hashes (5,151,173,020 bytes) and all
  10,928 prior identity rows remain unchanged. The normal catalog scan added 23 final PDFs,
  so the registry now has 10,951 rows. Preserve original failed/frozen index receipts;
  never rewrite the old 999-index eval map or infer new K1 eligibility from this retry.
- Root reverified all 62 retained external files (19,730,095 bytes), including the portable
  helper, registry snapshots and exact build/gate receipts. No additional artifact copying
  was necessary; receipt `~/.cache/lysilogy/review-evidence/pr100/root-portability-review.json`.
  #98 worktree/local/remote branches are removed. #70 was also fully cleaned up after root
  rehashed all 156 original/archive pairs (8,371,227 bytes); archive manifest
  `~/.cache/lysilogy/review-evidence/pr99/manifest.json`, SHA
  `0acab4040a870062571962b429327fc1224e4110e704b701759be361983bef46`.
- **No corpus download, automatic alignment run or scale indexing batch is running.** The
  corpus is complete: 10,951 unique PDFs / 1,000 sources, all 10,000 scale PDFs and 1,000
  eval pairs, verified. Corpus root `~/Corpora/arxiv/`; data/registry root
  `~/.cache/lysilogy/arxiv-kb-data`. Do not repeat recovery or reset/copy these roots.
  Batches1–3 are complete. With the successful retry there are 1,750 successful indexes,
  covering all 1,000 eval papers and 799 scale papers (49 overlap). These are preparation,
  not production O25/O27 or end-to-end 10k acceptance. Preserve the 20 GiB free-space floor.
- Next scale batch is **4**, `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 4 --limit 250`.
  Wait for the coordinated heavy window before launching. Runner SHA is now
  `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`; previous versions are
  immutable under `native-tools/runner-history/`. It uses the reviewed standalone helper
  `~/.cache/lysilogy/k1-native-page-isolation-0843453/k1_index`, SHA
  `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`, with source copies,
  Cargo compiler-artifact selection and build receipt retained beside it. No removed
  worktree dependency remains. Existing identity rows and frozen index hashes are protected.
- **#96** owner `review_ready_prs`, worktree/branch `feat/e1.1-object-metrics`, source through
  `47f3f2b` (initial checkpoint `06e75ca`). Twenty-one Python tests pass; Rust/Clippy checks
  are active in its heavy window. Reviewer `finish_corpus_proxy` is reviewing source before
  first actual measurement. No PR or O1/O2 measurement yet. Roman printed labels stay
  distinct from Arabic labels; bridge validates canonical IDs, paths and byte bounds; every
  run invokes Cargo's selected exact adapter. Contract matches kind/label/page and mutual
  caption membership, never region overlap; duplicates count FP, misses FN, and O2 includes
  all independently annotated truth objects with missed/invalid predicted regions zero.
  Integrate current main normally before final gates/measurement; no rebase.
- **#25 / draft PR94** owner `finish_benchmark`, branch/worktree `feat/e1.2-bibliography`,
  integrated K1/main at `779f0c2` (source previously clear through `46ff41b`). Root ran the
  actual one-paper K1 collector before any parser changes: **O8 = 0.289855** (TP10/FP24/FN25),
  **O10 precision = 0.261905 / recall = 0.22** (TP11/FP31/FN39). K1 field diagnostics are
  title7/35, first-author10/35, year10/35; **O9 remains unavailable without genuine K2**.
  Before/collector/after eval checks passed; collector took 0.498 s, zero network/models/$0.
  Frozen original logs, receipt, observations, collector input, scorecard and baselines are
  under `~/.cache/lysilogy/bibliography-k1-first-measurement/`. Observation SHA
  `7c9e1f4a9921353de54aac165a56da7adc731b7eaa5eb016c64e2a43aa7302db`.
- #25 is iterating against the unchanged truth/contract. Root found three general causes:
  paragraph heading heuristics drop legitimate bibliography continuation lines; collapsed
  paragraphs hide physical numbered-entry starts; unnumbered author/year logic splits
  numbered-entry continuations. Owner is adding independent synthetic/adversarial tests
  and preserving the first measured baseline. Wait for #96 to release compilation capacity.
  Re-run identical K1 metrics, required gates and separate source review after changes.
  Keep PR94 draft until genuine K2 and the full owned measurements exist; surviving misses
  need measured follow-ups after reasonable effort. No truth or target relaxation.
- Published immutable K1 remains `eval/truth/k1-limited-v1/`: two independently reviewed
  papers covering every E1 kind, all denominators/negative inventories and selection bias
  explicit. Full automatic run was 622 parsed / 1,000 and zero admitted cohorts; see the
  #70 report and `~/.cache/lysilogy/k1-full-alignment-9d3bd42/root-review-v2.json`.
  **#97** retains approximately 500-paper coverage, next finite source-support/independent
  annotation experiments, branch `feat/e8.3-k1-coverage` not yet created. It starts after
  #96's initial review frees `finish_corpus_proxy`; do not alter the limited release bytes.
- Paused: **#33 `905c341`**, worktree `feat/e2.1-kb-store`, source reviewed but Rust not
  compiled (rusqlite/fallible dependencies unavailable), no PR. **#71/PR93 `d5e12aa`** on
  `feat/e8.4-reference-truth`, **#72/PR95 `c867ec6`** on `feat/e8.5-person-labels` stacked
  on93, source/gates reviewed but no actual provider truths. The combined frozen 40-DOI
  Crossref plan under `~/.cache/lysilogy/reference-truth-plans/` remains unexecuted.
- The original storage/three-host corpus grants work. Still no answer to the additional
  **index.crates.io, static.crates.io, api.crossref.org, api.openalex.org** grant request.
  Do not apply `~/.config/lysilogy/apply-codex-kb-network-permissions.py` or bypass the
  earlier auto-review rejection without explicit approval. Do not access private denied
  paths or env/secrets files. No rebase/sudo/rm-rf, persistent systemctl or vault destruction.
- Next: finish independent #96 source review and actual measurement, then review its PR;
  continue #25 measured parser fixes, start #97 and scale batch4 when capacity permits.
  Required remaining phases and final Playwright/10k system acceptance are outstanding.
  Main scorecard still **1/5 gates (G5), 1/30 objectives (O30)**; newly measured #25 O8/O10
  misses are on its draft branch, while historical O25/O26 misses remain under #21/#22.
  Follow-ups opened this continuation: #97 and #98; #96 already existed. Final system
  report is not yet due because the system definition of done has not passed.

### 2026-09-13 — figure/table baselines merged; measured detector work next

- Still **Phase A / Wave A2**. Last merged issue **#96 / PR102**, merge
  `d074dae96d9158c75add4ba9cc6fb27be1adf5bb`, reviewed head
  `5d5c567d0f39ecec3f50fc0e99ff7f7a1f814e39`. Earlier this continuation merged
  #35/PR87, #36/PR89, #91/PR92, #88/PR90, #70/PR99 and #98/PR100. No phase exit or system
  acceptance. Main permits docs commits only; all ten unrelated preview file hashes still
  match `/tmp/lysilogy-preview-before.json`. No merged issue worktree remains.
- **#96 actual baselines are now on main:** O1 = 0.7692307692 (10TP/1FP/5FN),
  O2 = 0.2448506858 over all 15 independently region-annotated objects, zero unknowns.
  Ten figures match; all five Roman tables I–V are missing; one prose Figure 2 is extra.
  Matched-only IoU median 0.2906506211 is diagnostic only. Root independently showed every
  matched box contains the entire true body but has 1.80–9.13 times its area; the problem is
  excessive surrounding coverage. Retained diagnostic:
  `~/.cache/lysilogy/object-metrics/root-region-diagnostic-v1.json`. **#101** on E1/#12 and
  project12 records both misses, unchanged targets and next fixes; it is ready in A2.
- #96 report: `docs/experiment-reports/2026-09-13-object-metrics.md`; evidence:
  `eval/evidence/object-metrics.json`, `object-metrics-observations.json` and
  `object-metrics-integration.json`. Measured source `8558886`, final warm gates `32b7341`
  include main `fd024fc`, with all 77 measurement implementation hashes unchanged.
  G5 actually ran **320 Rust / 278 Python / 85 Node**; all required checks pass. Original
  4.570 s measurement was not repeated during docs-only integration. Zero network/models/$0.
  Exact corpus/registry/index/truth hashes remain unchanged. No detector changed in #96.
- Separate final source review tested 24 Python scenarios, 200 independent integer-grid
  IoU/union cases and byte-identical K1 reconstruction before predictions. Final PR and
  integrated evidence review rehashed all source/log/result dependencies and reproduced
  every actual match/IoU. Latest receipt:
  `~/.cache/lysilogy/review-object-metrics-5d5c567/final-integration-review.json`.
  Root additionally verified 156 final references (4,415,876 bytes) and all target receipt
  archive aliases: `~/.cache/lysilogy/review-evidence/pr102/root-reference-review.json` SHA
  `96cf4a6bd675b0e888351173eada075d644288f3d8fe24c8e4855a6251ed63ef`, portability SHA
  `0dda1841980d8816a90ae979305e31adb322d3c17799a61a9761cfae2eb73042`.
  #96's clean worktree/local/remote branch are removed; no runtime dependency remains.
- **Scale batch4 / exec44342 completed exit 0:** 250/250 indexes, **430.352604 s /
  244,160 KiB peak child RSS**, zero network/models/$0. All 10,951 canonical identity rows
  and the entire registry bytes are unchanged. Output SHA
  `51cc5c8091a19edd92de480411b8f6fa1aae77fa1fa751b904478a0e59140b98` in
  `~/.cache/lysilogy/scale-index-batches/batch-0004/`. Root independently rehashed all 250
  new indexes (758,290,872 bytes) and audited the aggregate identities/tiers in 0.817 s.
  Receipt `native-index-after-batch4-review.json`, SHA
  `fcec08a0025fbbf507c293fae955ffb2928cc15758172ebf40db969fb218fd91`.
- **2,000 unique successful indexes** now exist: all 1,000 eval papers and 1,049 scale
  papers (49 overlap). This includes the separately retained successful #98 retry; the old
  failed map/receipt stays unchanged. There is no corpus download, alignment run or native
  scale batch running. The complete corpus remains 10,951 PDFs / 1,000 sources at
  `~/Corpora/arxiv/`, all scale/eval quotas verified. Canonical data root is
  `~/.cache/lysilogy/arxiv-kb-data`; never reset it or repeat selection recovery.
- Next native batch is **5**, using `run-scale-index-batch.py --batch 5 --limit 250` under
  the cache. Runner SHA `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`;
  portable helper `k1-native-page-isolation-0843453/k1_index` SHA
  `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`.
  Source/build fingerprints and older runner versions remain external. Wait for root's
  coordinated heavy window; these preparation batches are not production O25/O27 acceptance.
- **#25 / draft PR94** owner `finish_benchmark`, worktree `feat/e1.2-bibliography`, has
  completed one measured iteration at **`aae7369`** after main integration `ed6dfc3`.
  First baseline `779f0c2` remains immutable under
  `~/.cache/lysilogy/bibliography-k1-first-measurement/`: O8 .289855, O10 precision .261905 /
  recall .22. General physical-line/keyed-continuation fixes plus four adversarial tests
  improved O8 to **.916667** (33TP/4FP/2FN) and O10 to **.738462 precision / .96 recall**
  (48TP/17FP/2FN). K1 fields: title27/35, author33/35, year33/35; O9 still unavailable
  without genuine K2. The truth/matching contract was unchanged.
- #25 owns the current short build/remeasurement window. Its next uncommitted iteration
  distinguishes publication-year continuation lines from actual four-digit keys, and
  withholds ambiguous parenthesized/superscript numbers when established bracket citations
  do not provide an explicit mixed-citation cue. Remaining extras were six footnotes,
  four math indices, four parenthesized list numbers and an equation label. Preserve
  candidate IDs/raw occurrences without links, pure superscript documents and evidenced
  four-digit printed keys. Add adversarial tests, remeasure identical K1, and independently
  review source after iteration. Integrate new main/#96, retain the union of measured
  baselines and run its actual objects collector once under the current wrapper. Keep PR94
  draft pending real K2/full O9; surviving misses need measured follow-ups after reasonable effort.
- **#97** owner `finish_corpus_proxy`, fresh worktree `feat/e8.3-k1-coverage` based on
  `fd024fc`. Before checks objects/bibliography passed with explicit --root using a proven
  source-equivalent reviewed CLI: 72 Rust/Cargo/config inputs identical, receipt
  `~/.cache/lysilogy/k1-coverage/before-fd024fc/receipt.json`, SHA
  `59ac2c493dc7cc0e9e6e3a961c0eb84ddba07cc351e9376565723e9802060cb5`.
  It is now writing a version-selected old verifier bundle under
  `eval/implementations/k1-limited-v1/` and `scripts/truth/latex/versioned.py`; no committed
  source checkpoint or PR yet. Exact reviewed module bytes and a pinned manifest/isolated
  worker must reproduce the immutable old release independently of later parser changes.
  Fixed bundle depth preserves old ROOT semantics. Integrate #96's new collector into
  this worktree for dispatch; never edit its removed worktree or immutable truth/config.
- #97 next adds finite standard symbol/Greek and provably equivalent source capabilities
  with adversarial tests, retaining redefinition/local-style/unknown-control-flow guards.
  Keep the same frozen 1,000 inputs and original 999 valid indexes for comparable full
  runs; no full run or cold Rust compile until its heavy window. Prior full automatic
  result: 622 parsed / 1,000, zero admitted cohorts. The two-paper limited release covers
  every E1 kind but the approximately500-paper stratified target remains unmet. No truth
  target, matching denominator or hard gate may be relaxed.
- **#101** will be assigned to `review_ready_prs` after this docs checkpoint, branch
  `fix/e1.1-figure-detection` (not yet created). Its owned files and required current-
  production generation/anchor binding are in Wave A2 and the issue. Coordinate shared
  object plumbing with #25 and verifier dispatch with #97. Begin source/light tests while
  #25 owns compilation; measure before and after with frozen truth and separate review.
- Paused: **#33 `905c341`**, `feat/e2.1-kb-store`, no PR; reviewed Rust remains uncompiled
  because rusqlite/fallible dependencies are absent. **#71/PR93 `d5e12aa`**, reference
  truth; **#72/PR95 `c867ec6`**, person labels stacked on93. Their actual provider truths
  remain unavailable. Frozen combined 40-DOI plan under `reference-truth-plans/` is unexecuted.
  No answer yet to extra grants for **index.crates.io, static.crates.io, api.crossref.org,
  api.openalex.org**. Original storage/three corpus-host grants work. Do not apply the
  four-host permission script or bypass prior auto-review rejection without explicit approval.
- Next: continue measured #25 fixes, #97 verifier/capability work and #101 detector fixes;
  launch scale batch5 when the short gates release capacity. Every later phase and the
  final running-app Playwright/10k scenario remain outstanding. Main scorecard:
  **1/5 gates (G5), 1/30 objectives (O30)**, now with O1/O2 measured misses under #101.
  Historical O25/O26 misses remain under #21/#22. Follow-ups opened this continuation:
  #97, #98 and #101; #96 already existed. No phase/system report is claimed complete.

### 2026-09-13 — bibliography K1 targets reached; detector source under review

- Still **Phase A / Wave A2**. Last merged issue remains **#96 / PR102** (`d074dae`);
  main before this docs checkpoint is `9dc3f27`. No new merge, phase exit or system acceptance.
  Earlier this continuation merged #35, #36, #91, #88, #70, #98 and #96. All ten unrelated
  preview file hashes match `/tmp/lysilogy-preview-before.json`; main remains docs-only.
- **#25 / draft PR94**, owner `finish_benchmark`, worktree `feat/e1.2-bibliography`,
  pushed partial report/evidence at **`1cbfb87`**, tested source **`2608bc8`**, ordinary main
  integration **`e8601d0`**. Actual frozen K1 now matches **all35 entries and all50 citation
  target pairs: O8 = 1.0; O10 precision/recall = 1.0**. Known K1 fields are title30/35,
  first-author35/35 and year35/35. **O9 remains unavailable without genuine K2**, so PR94
  stays draft and cannot merge yet. Original first baseline stays immutable under
  `~/.cache/lysilogy/bibliography-k1-first-measurement/`: O8 .289855, O10 .261905/.22.
- #25 general fixes preserve physical entry boundaries and keyed author continuations,
  support genuine consecutive four-digit keys using neighboring author/title evidence,
  and retain ambiguous numeric occurrences without assigning links. Explicit mixed-style
  citation cues remain supported. Exact truth, UTF-16 matching and denominators are unchanged.
  **#103** on E1/#12 records the five remaining line-break hyphen title ambiguities, measured
  K1 title30/35 versus target.90, and conservative physical-line/alternative-spelling next ideas.
- At2608bc8 all formatting, strict Clippy, all Rust, frontend checks/build,20 paper-link
  tests, Playwright smoke and bibliography/objects/scale/G5 pass. G5 **355 Rust /292 Python
  /92 Node**,15 command logs retained; full sequence69.149s, actual bibliography.817s and
  object collector5.288s, no external/model calls/$0. O1/O2 remain unchanged. Separate
  reviewer `finish_corpus_proxy` ran34 compiled bibliography tests and8 independent bridge
  probes: `review-bibliography-2608bc8/probe-receipt.json` under the cache. Final partial
  report review rehashed298 references/all189 tested source hashes and independently
  recomputed35/50 exact matches in.236s; `review-bibliography-1cbfb87/` retains both receipts.
- #25 has a **new uncommitted narrow provenance fix** withholding a truncated structured
  arXiv DOI, exposed by wrapped `10.48550/arXiv.2102. 04906`. Raw entry remains intact;
  do not manufacture an explicit prefix identifier. Owner currently has the **short heavy
  gate window** for regression/full warm checks, then reviewer must cover this delta and
  updated report. Preserve historical receipts; do not claim their gates cover new source.
- **#101**, owner `review_ready_prs`, worktree `fix/e1.1-figure-detection`, source checkpoint
  **`e7d0e79`**, no PR yet. Clean-main before collector confirmed O1 .7692307692/O2 .2448506858
  after94.38s cold bridge and5.07s measurement. Source now recovers Roman captions, rejects
  prose-reference continuations and tightens excess regions using native geometry. Production
  `ObjectsArtifact` factory recomputes current figures; separate detector version/generation
  invalidates stale objects while preserving exact native reading-index generation. The
  evaluation bridge verifies unchanged native basis and current derived generation.
  Six independent coordinate fixtures, cache-generation regression, targeted Rust/objects/API,
  strict Clippy and25 collector tests pass. **No changed-detector measurement yet**; #25 owner
  will independently review after DOI checks. Root inspects integration/provenance. Prepare
  actual measurement while review proceeds, then use unchanged frozen K1 contract.
- **#97**, owner `finish_corpus_proxy`, worktree `feat/e8.3-k1-coverage`, checkpoint
  **`dcea8a0`**, no PR. Compatibility **`cad5797`** independently clear: exact12 old modules
  (257,691 bytes) match Git, nine boundary tests and27 collector tests pass; read-only replay
  reproduces both immutable release hashes in4.21s. Receipt
  `~/.cache/lysilogy/review-k1-versioned-cad5797/review.json`. Source capability delta adds
  finite inventory-only standard TeX atoms and direct zero-argument preamble equation aliases
  with deposited definition/invocation spans. Rendering, unknown semantics, redefinitions and
  local-style exclusions stay separate;160 LaTeX tests pass independently.
- Root found a **#97 review blocker**: star-suffixed alias invocations disappear from
  environment events but semantic guards consider them supported, wrongly certifying empty
  equation inventory. Fix token/span semantics or withhold unsupported invocation; add an
  adversarial regression before any full run. Receipt `review-k1-capabilities-dcea8a0/review.json`,
  SHA `ba09073cdcecf24130a461ddc1d1604e1dd9975c5fc34106fa10967ebaabdbd2`. Full automatic
  comparison still uses the same frozen1000 inputs/original999 valid index map. Prior result
  remains622 parsed/1000,0 admitted; approximately500-paper target is unchanged. The separate
  successful #98 retry does not retroactively change that comparison population.
- **No corpus downloader, native batch or full alignment job is running.** Corpus is complete:
  10,951 unique PDFs/1,000 sources, verified exact eval1000/scale10000 quotas at
  `~/Corpora/arxiv/`. Data root `~/.cache/lysilogy/arxiv-kb-data` has2,000 successful unique
  indexes: all1000 eval and1049 scale (49 overlap). Batches1–4 and separate #98 retry complete.
  Next batch **5**: `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 5 --limit 250`
  after short gates release capacity. Runner SHA `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`,
  portable helper SHA `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`
  in `k1-native-page-isolation-0843453/`; all provenance external. Preserve20GiB free floor,
  existing identity/cache bytes, and one heavy process.82GiB free at latest check. These
  batches are preparation, not production scale metric or running-app acceptance.
- Paused clean checkpoints remain **#33 `905c341`**, `feat/e2.1-kb-store`, Rust uncompiled
  because registry dependencies are unavailable, no PR; **#71/PR93 `d5e12aa`**, reference
  truth; **#72/PR95 `c867ec6`**, person labels stacked on93. Their actual provider truths
  remain unavailable. Frozen combined40-DOI plan under `reference-truth-plans/` is unexecuted.
  No answer to additional grants for **index.crates.io, static.crates.io, api.crossref.org,
  api.openalex.org**. Original storage/three corpus-host grants work. Do not apply the pending
  four-host permission script or bypass the prior automatic-review rejection. No rebase, sudo,
  rm-rf, persistent systemctl or private-path access; ordinary main integration is documented.
- Next: finish DOI and alias corrections/reviews, measure independently reviewed #101, launch
  scale batch5 and full1000 source comparison in coordinated windows. Main scorecard remains
  **1/5 gates (G5),1/30 objectives (O30)**; draft #25 additionally reaches O8/O10 but is not
  published. Main O1/O2 misses are under#101; historical O25/O26 misses under#21/#22.
  Follow-ups opened this continuation: **#97,#98,#101,#103**. All later phases and final
  Playwright/10k system acceptance remain outstanding; no completed phase/system report claimed.

### 2026-09-13 — bibliography draft complete; graphics evidence and source semantics next

- Still **Phase A / Wave A2**. Last merged issue remains **#96 / PR102** (`d074dae`);
  preceding main docs checkpoint is `0ed9e0a`. No new issue merge or phase/system exit.
  Earlier this continuation merged #35, #36, #91, #88, #70, #98 and #96. The ten unrelated
  preview file hashes still match `/tmp/lysilogy-preview-before.json`; main is docs-only.
- **#25 / draft PR94** is clean and pushed at **`df5426029586a80dedff9556d2f6cf85db4d8915`**,
  source **`e3539d204dfb937cc797b2a01e2d8ad782309300`**. Owner `finish_benchmark` has finished
  the scoped parser/report and is now reviewing other branches. Incomplete structured arXiv
  DOIs are withheld; nine separate production probes verified the fix without changing raw
  entry membership. Final source, full gates and report/evidence are independently clear.
  Final review `~/.cache/lysilogy/review-bibliography-df54260/final-review.json`, SHA
  `db4fea17bc31875a00b3429ce725d09270e7e686ea972c60ab6b73fd6203416b`, rehashes 320 retained
  references and all 189 tested source fingerprints. All 16 final gate statuses pass,
  G5 **356 Rust / 292 Python / 92 Node**. Exact final gate receipt:
  `bibliography-development/gates-e3539d2-1789305538794094542/receipt.json` under the cache.
- Draft #25 measures **O8 = 1.0 and O10 precision/recall = 1.0** on unchanged limited K1
  (35 entries / 50 citation-target pairs). Known fields remain title 30/35, author/year 35/35;
  #103 retains five title line-break hyphen misses. **Genuine K2/full O9 still missing: do not
  merge PR94.** All original measurements remain under `bibliography-k1-first-measurement/`
  and every iteration/failure under `bibliography-development/`. No truth or target changes.
- **Scale batch5 / exec9735 completed exit 0:** 250/250, **429.440087 s / 232,256 KiB RSS**,
  zero network/models/$0. Output SHA
  `f5031cef1c75ad5cb9a9f5fa7756c0d7772f366be84bda86052c4f1d09130f80`; all 10,951 identity
  rows and the entire registry bytes unchanged. Root independently rehashed all 250 new
  indexes (758,021,677 bytes), audited all aggregate identities/tiers and original inputs.
  Receipt `~/.cache/lysilogy/native-index-after-batch5-review.json`, SHA
  `e25c58aca0c9a5a52390d3856066f7094b539f9bb9e82a9053f1bb8978565dc7`.
  **2,250 unique successful indexes: all 1,000 eval and 1,299 scale papers** (49 overlap).
- **Native scale batch6 is RUNNING, exec10286**, launched with
  `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 6 --limit 250`.
  Inspect `scale-index-batches/batch-0006/receipt.json` and terminal status before acting;
  do not rerun an existing batch directory. Batch6 currently owns the heavy window.
  After completion run the independent read-only audit helper
  `python3 -B ~/.cache/lysilogy/audit-native-index-batch.py --batch 6`; it writes an exclusive
  new review receipt, so inspect an existing receipt instead of overwriting it. Helper SHA
  `c37667c8ef614c3df03996939a980c7a43e964925893b1f02fe3bdd48a7228e8`. Its first development
  attempt incorrectly assumed historical batches1–3 had byte-identical registries; those
  legitimate early registry additions are already independently reviewed. Final audit pins
  all original outputs and requires unchanged current-batch registry bytes.
- Corpus downloads are complete and no downloader runs: **10,951 PDFs / 1,000 sources**,
  exact eval1000/scale10000 quotas verified, root `~/Corpora/arxiv/`. Canonical registry/data
  is `~/.cache/lysilogy/arxiv-kb-data`; never reset or copy these roots. Batches1–5 and the
  separate #98 retry are complete. Preserve the immutable original 999-success/one-failure
  eval map. Scale runner SHA `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`,
  portable helper SHA `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`
  in `k1-native-page-isolation-0843453/`. No removed worktree dependencies. Maintain the
  20 GiB free-space floor; preparation indexing is not production scale acceptance.
- **#101**, owner `review_ready_prs`, worktree `fix/e1.1-figure-detection`, no PR. Current
  tested/reviewed source **`6180f0af65be092afb951ebd4419303a7fc5e937`**, includes ordinary
  main integration `144b8f0` of `0ed9e0a`. Separate reviewer `finish_benchmark` found and
  cleared disjoint-paragraph membership, later-page geometry and neighboring-column defects.
  Exact pieces/page filtering is `205788e`, complete test-page metadata `50084ad`, fused
  arithmetic lint fix `6180f0a`. Independent compiled review ran all ten figure regressions
  and bound 69 committed Rust sources, Cargo artifact/executable/log. Receipt
  `review-figures-b93b83b/compiled-clearance.json` under the cache, SHA
  `494e2b327fd4bc429e73042095a74d47215bdb9599d5e1f7f7eb91d121a84c1a`.
- Full #101 Rust tests exposed an existing wide-diagram fixture regression: a diagram label
  outside its narrower caption is excluded. Keep the failed gate at
  `figure-detection-development/6180f0a-1789306470300347863/receipt.json`. Formatting/Clippy
  passed, but **full gates are not accepted**. A separately labeled exploratory same-K1
  collector took 6.197567 s, zero network/models/$0: **O1 = 1.0 (15TP/0FP/0FN)**; **O2
  median = 0.0**, regressing because eight figure regions are absent. Seven regions have
  nonzero IoU, including Figure1 .916972 and TablesI .942344 / II .924885 / IV .925985.
  TableV incorrectly shares Figure8's entire rectangle; separate those bodies. All truth
  captions/regions, denominators and canonical native inputs remain unchanged.
- First changed-detector evidence: `figure-detection-development/6180f0a-exploratory/`
  contains receipt and observations. Frozen predictions:
  `object-metrics/3549d3606aa628d85526edec2273f661ae9f5879f9751ed3b8cbf0d6f622a683/predictions.json`.
  Original baseline predictions `object-metrics/d662cc9ef128ce01983b5bf33fd3b693db5a11f85d1dce0e8fd9fc1e42de4bb6/`
  remain untouched. **Do not merge this regression or claim O2 achieved.**
- #101 next: missing plot bodies have no native text (embedded raster plots), so inspect
  installed `mutool trace` image placements rather than inventing label-only regions. Root
  authorized one bounded read-only trace of the retained positive PDF, with cache output,
  timeout/byte bounds and PDF/tool/page/timing/RSS provenance. That small diagnostic may run
  alongside batch6 (prior native peak only 232 MiB); no concurrent Rust build or rasterization.
  Any graphics dependency must be used by the actual async production objects path and
  measurement bridge, with a separately versioned graphics/detector generation. Preserve
  native index bytes, existing good table regions and the wide-diagram fixture.
- Root also found #101's full `native_basis_json` transport exceeds the aggregate 16 MiB
  response bound after a few papers, incompatible with planned 500-paper truth. Owner will
  provide a compact independently reproducible native-basis commitment before final PR,
  preserving the whole-index hash, exact typed native fields, explicit framing/order/numeric
  semantics and unchanged matching. Review concrete algorithm and cross-language vectors
  before final collector use; no population truncation. Heavy window is released by #101
  while it implements this and the graphics correction.
- **#97**, owner `finish_corpus_proxy`, worktree `feat/e8.3-k1-coverage`, no PR. Compatibility
  `cad5797` is independently clear and reproduces immutable `k1-limited-v1` exactly. Current
  capability source **`9680f860917d64a8c6569d4bf36eddc44511e959`** adds finite standard atoms
  and literal zero-argument equation aliases. Root found/fixed alias-star loss (`0674838`)
  and leading bracket math loss (`9680f86`); 164 tests and 11 separate probes pass. Root
  receipt `review-k1-capabilities-9680f86/review.json`, SHA
  `d6b6a08a15129f04c9e6c6925fdf0b7e8add904edb438b13eb020e701501ec69`.
- The same-cohort run at **`~/.cache/lysilogy/k1-full-alignment-9680f86/execute.py` remains
  PREPARED BUT UNLAUNCHED**, because owner then found an inherited macro-argument defect: a
  custom macro can discard or duplicate a structural argument while the scanner certifies
  its literal occurrence. Separate reviewer reproduced false O5 for a discarded theorem and
  false O3 for discarded equation aliases; no other bounded source/runner finding. Receipt
  `review-k1-capabilities-independent-9680f86/review.json`, SHA
  `2d2f90f27fc68369878d9e14b0575016c30404635f1ae48394a4ba3fa07e0bf4`. **Do not launch this
  prepared version.** Retain it untouched as an unlaunched historical setup.
- #97 is implementing conservative unsupported evidence for structural tokens in consumed
  custom-macro arguments and unproven forwarding through parameterized custom macros. Cover
  discard, duplicate/reorder, nested forwarding and accepted equation aliases with original
  source spans; no broad TeX expansion. Reviewer `finish_benchmark` needs `followup_task`
  if idle once the correction is committed. Prepare a new versioned run only after independent
  clearance, same frozen1000 inputs/original999 indexes, same 30 s/paper and 1.5 GiB limits.
  The old prepared modules/runner/cohort were independently rehashed, no native extraction.
  Last actual automatic result remains 622 parsed/1000 and zero admitted cohorts. Approximately
  500 defensible stratified papers remain required; no target or source-quality relaxation.
- Paused blockers remain **#33 `905c341`** (`feat/e2.1-kb-store`, no PR, Rust not compiled:
  registry dependencies missing), **#71/PR93 `d5e12aa`** (reference truth), **#72/PR95
  `c867ec6`** (person labels stacked on93). Their frozen combined 40-DOI plan is unexecuted;
  actual K2/K4/K5/K7 unavailable. No answer to the pending extra grants for **index.crates.io,
  static.crates.io, api.crossref.org, api.openalex.org**. Original corpus storage/three-host
  grants work. Do not apply the four-host script or bypass the earlier automatic-review
  rejection without explicit approval. No rebase, sudo, rm-rf, persistent systemctl or
  private/env/secrets access; ordinary main integration remains the documented alternative.
- Next: finish batch6/audit; review #97 argument correction and run the corrected frozen
  cohort; continue #101 graphics/transport fixes, remeasure and rerun required gates before
  its draft PR. Main scorecard remains **1/5 gates (G5), 1/30 objectives (O30)**. Draft #25
  additionally reaches O8/O10; exploratory #101 reaches O1 but regresses O2 and is not
  accepted. Main O1/O2 misses remain under #101; historical O25/O26 under #21/#22; title
  component misses under #103. Follow-ups opened this continuation: **#97, #98, #101, #103**.
  All later phases and final live Playwright/10k acceptance remain outstanding.

### 2026-09-13 — corrected source comparison running; native graphics integration

- **Phase A / Wave A2**, last merged issue still **#96 / PR102** (`d074dae`); main before
  this docs checkpoint is `720126b`. No new issue merge or phase/system exit. Earlier this
  continuation merged #35, #36, #91, #88, #70, #98 and #96. All ten protected preview files
  match `/tmp/lysilogy-preview-before.json`; main edits are docs-only.
- **#97 corrected same-cohort comparison is RUNNING, retained TTY exec13944**:
  `python3 -I -B ~/.cache/lysilogy/k1-full-alignment-4ada537/execute.py`. Source
  **`4ada537dd7aaaab8e3fa0f00b60a5369d4d2bfd3`**, owner `finish_corpus_proxy`, worktree
  `feat/e8.3-k1-coverage`, no PR. Do not mutate source or the frozen run while it executes.
  This is exploratory coverage measurement, not truth publication. Last interim count was
  94/1000 processed, 50 candidate papers, zero eligible metric cohorts; inspect completed
  `receipt.json` before reporting final counts. Never tail the potentially huge final report
  line in `run.log`; read selected JSON fields or the small per-paper summaries.
- Launch SHA `d688b3f64c8263a38b111b9e7b9ec3bd33d6cbb1975c103e3b6b64cdb047a238`, runner
  `709f8aae9629cd4396fb1bdc580c292087e32bc5a1d275874c832338e49a5fc6`, wrapper
  `4c7c9025da530f91c553d2cecfc524945f2d44a03e7427612b6522e743652ee5`. Nine exact source
  modules plus frozen corpus.py match the reviewed commit. Original frozen 1000 input SHA
  `2d4504df1208d27dc56ddae5549d1a49465d5db8fff06765215d0351a63cf8e0` and 999-success/one-
  failure map SHA `40b0545ac4f34c4c1d389f3f7bbdb8b613a533cef6e566285b570ccd7ea0f785`
  are unchanged. Same order, 30 s/paper and 1.5 GiB address-space bounds. No native extraction.
- Independent reviewer `finish_benchmark` cleared final source after 178 tests under
  warnings-as-errors and separate argument/origin probes. Review
  `review-k1-capabilities-independent-4ada537/review.json`, SHA
  `d351bd3b17c3c9d84186d618f757b9b259d8d8637aef16152e63be7477311ff7`; snapshot review
  `snapshot-review.json` beside it, SHA
  `4514cc9220a27901ac24ba7923d6e1fe55642365cc7692ce7a3bdd65dacbc547`. Root also rehashed
  launch/modules/corpus/runner while running. Custom discarded/duplicated arguments were
  fixed at `9fff2f6`, standard stored/literal roles at `6f1cd6c`, tail consumers at `22d27ca`,
  optional-only consumer membership at `4ada537`. Exact original invocation/argument spans
  are retained; no general TeX expansion. Prior `9680f86` setup stays unlaunched and immutable.
- Compatibility `cad5797` still independently reproduces immutable `k1-limited-v1`. The last
  completed automatic run remains 622 parsed/1000, zero admitted. The approximately 500-paper
  stratified target remains required. Read-only planning found another inherited math-fidelity
  gap: nested `array` can flatten its column specification into ordinary text and wrongly
  certify O3. Probe `k1-coverage/unverified-math-array-layout-probe.json` under the cache.
  Keep current comparison unchanged; before publishing any newly eligible math cohort, add
  explicit array/aligned-layout fidelity exclusion or independently prove rendering/geometry.
  Preserve separately justified other-kind inventories. No source/target/truth relaxation.
- **Scale batch6 / exec10286 completed exit 0:** 250/250 in **396.789135 s**, peak
  **240,752 KiB**, zero network/models/$0; output SHA
  `0d9a16ac83c94ab43545e438e4e8946b57448654fabfdeb48a2ec3144befdbf8`. All 10,951 identity
  rows and the entire registry bytes are unchanged. Root rehashed all 250 new indexes
  (722,364,246 bytes) and all aggregate IDs/tiers, receipt
  `~/.cache/lysilogy/native-index-after-batch6-review.json`, SHA
  `923c3972553524d83d5b7947fd837f06cf800e49a6a2cc611890d3faba37f156`.
  **2,500 unique successful indexes: all 1,000 eval and 1,549 scale papers** (49 overlap).
- No native batch or corpus downloader runs now. Next native batch is **7**, after the
  current full source comparison releases the heavy window:
  `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 7 --limit 250`.
  Runner SHA `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`; portable
  helper in `k1-native-page-isolation-0843453/`, SHA
  `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`. After completion,
  exclusive new audit `audit-native-index-batch.py --batch 7`, helper SHA
  `c37667c8ef614c3df03996939a980c7a43e964925893b1f02fe3bdd48a7228e8`. Inspect existing
  receipts instead of rerunning/overwriting a batch or review directory.
- Corpus complete: **10,951 unique PDFs / 1,000 sources**, verified eval1000/scale10000
  quotas at `~/Corpora/arxiv/`. Canonical data root `~/.cache/lysilogy/arxiv-kb-data`; never
  reset/copy it or repeat selection recovery. Batches1–6 and separate #98 retry are complete.
  Keep the original failed map immutable. Latest free space 82 GiB, available RAM about
  2.9 GiB; preserve 20 GiB disk floor and one heavy job. Small bounded graphics traces are
  allowed alongside other work based on measured 20 MiB footprint; no concurrent Rust/G5
  build during the full comparison. Preparation indexes are not production 10k acceptance.
- **#101**, owner `review_ready_prs`, worktree `fix/e1.1-figure-detection`, no PR. Current
  committed checkpoint **`7fa66d7`** adds compact native commitments over wide-diagram fix
  `06c4909`; source before those was independently reviewed `6180f0a`. The compact protocol
  replaces full native-basis transport, retaining exact whole-index SHA/schema6, type/length
  framing, ordered UTF-8 keys, finite f32 bit patterns and signed zero. Nine cross-language
  vectors and all-field mutations are included; 26 Python tests and four compiled bridge
  tests pass. Reviewer `finish_benchmark` is now independently reviewing this narrow delta
  without compiling during #97. Contract/vector files are in the branch.
- Root independently sampled 25 deterministic hash-selected real frozen eval indexes
  (67,542,405 bytes); all strict native-f32 commitments accepted, hashes unchanged, 8.898 s.
  Receipt `review-native-commitment-7fa66d7/sampled-native-inputs.json`, SHA
  `8bfa4a51cbef3d79f7bb4e5e48e73198553380db8be6931498a664b59c86f44f`. Full #101 gates
  are **not accepted**: the original wide-diagram label now survives, but an analogous
  third table column beyond a narrow caption is excluded. Owner will use repeated row
  alignment evidence to recover real grid columns while excluding neighboring page columns.
- First #101 exploratory result remains **O1 = 1.0 (15TP/0FP/0FN)**, **O2 median = 0.0**,
  from reviewed `6180f0a`: eight null figure regions dominate the median. Seven nonzero
  regions remain, four exceeding .91 IoU. TableV wrongly shares Figure8's entire rectangle.
  Preserve failed full gate `figure-detection-development/6180f0a-1789306470300347863/` and
  exploratory receipt/observations `6180f0a-exploratory/`. Frozen predictions
  `object-metrics/3549d3606aa628d85526edec2273f661ae9f5879f9751ed3b8cbf0d6f622a683/` and
  original `d662cc9ef128ce01983b5bf33fd3b693db5a11f85d1dce0e8fd9fc1e42de4bb6/` are unchanged.
- #101 graphics diagnostic found the missing plots are raster images with no native labels.
  One read-only installed mutool trace of page3 produced 248,910 XML bytes in .0169 s,
  20,752 KiB RSS, no network/models/rasterization/$0. It identifies the two image tiles per
  figure with page-local placement matrices; no clips/transparency groups on this page.
  PDF/tool hash/version/XML/receipt are under
  `figure-detection-development/graphics-trace-diagnostic/`. Owner is implementing a bounded
  trace-placement module, native page-dimension/finite-transform validation, explicit
  unavailable clipped/masked cases, and shared async production objects/cache/API/eval
  integration with versioned PDF/native/tool/trace evidence. No native index rewrites.
  Placed images also stop tables before neighboring figures. Review new source before
  remeasurement; preserve good native table regions and all existing PDF regressions.
- **#25 / draft PR94** remains clean, pushed and independently fully reviewed at
  **`df54260`**, source `e3539d2`; worktree `feat/e1.2-bibliography`. Owner `finish_benchmark`
  is serving independent reviews. Final G5 **356 Rust /292 Python /92 Node** and all gates
  pass; O8/O10 = 1.0 on 35 entries/50 pairs; K1 fields title30/35, author/year35/35.
  **Do not merge until genuine K2/full O9 exists.** #103 retains five title hyphen misses.
  Final receipt `bibliography-development/gates-e3539d2-1789305538794094542/receipt.json`;
  final review `review-bibliography-df54260/final-review.json`, SHA
  `db4fea17bc31875a00b3429ce725d09270e7e686ea972c60ab6b73fd6203416b`. First actual baseline
  and all iterations/failed gates remain external and immutable.
- Paused: **#33 `905c341`**, `feat/e2.1-kb-store`, no PR, Rust uncompiled because registry
  dependencies missing; **#71/PR93 `d5e12aa`**, reference truth; **#72/PR95 `c867ec6`**,
  person labels stacked on93. Frozen combined 40-DOI plan remains unexecuted; K2/K4/K5/K7
  are unavailable. Still no answer to grants for **index.crates.io, static.crates.io,
  api.crossref.org, api.openalex.org**. Original corpus storage/three-host grants work.
  Do not apply the four-host script or bypass the prior automatic-review rejection without
  explicit approval. No rebase, sudo, rm-rf, persistent systemctl or private/env/secrets
  access. Ordinary main merges remain the documented integration alternative.
- Next: finish/audit #97 full comparison and measure the next source capability from its
  actual exclusions; continue reviewed #101 graphics/grid/compact transport, then test and
  remeasure before PR; launch batch7 when capacity permits. Main remains **1/5 gates (G5),
  1/30 objectives (O30)**. Draft #25 reaches O8/O10; exploratory #101 reaches O1 but regresses
  O2. Main misses remain #101 (O1/O2), #21/#22 (historical O25/O26), #103 (title component).
  Follow-ups opened this continuation: **#97, #98, #101, #103**. All later phases and final
  running-app Playwright/10k acceptance remain outstanding; no phase/system report complete.

### 2026-09-13 — full source comparison audited; graphics compiling

- **Phase A / Wave A2**, last merged issue **#96 / PR102** (`d074dae`), main before this
  docs checkpoint `7ec28d6`. No new issue merge or phase exit. Earlier continuation merges:
  #35, #36, #91, #88, #70, #98, #96. Main remains docs-only; protect the ten preview files
  recorded in `/tmp/lysilogy-preview-before.json`. No rebase approval exists; ordinary main
  integration remains the documented alternative.
- **#97 full comparison completed, exec13944 exit0**, source
  `4ada537dd7aaaab8e3fa0f00b60a5369d4d2bfd3`, immutable directory
  `~/.cache/lysilogy/k1-full-alignment-4ada537/`. Same frozen1000 inputs/original999-success
  map, 30 s/paper and 1.5 GiB limits. **617 parsed, 383 failed, one raw automatic admission,
  zero defensible new truth papers**. The sole admission `2007.05954` has manually formatted
  References and author/year citations despite an empty parsed bibliography. Its procedural
  list also invalidates an unreviewed algorithm-negative claim: nine literal item commands,
  eight active after removing a commented item. No truth publication occurred.
- Completed comparison wall **997.138481 s**, peak **499,824 KiB**, zero network/models/$0.
  Receipt SHA `2827baf051989435f77dd765ef543aa0ec403eb971bdacf70142eb72339b9c36`, report
  `211525f280560b1abfdece63c0878870ce92a80f6cf889f49ebc8849cc10cec8`, paper ledger
  `3c99db2152488b07d4378eac0a40e73d89da861d46f951d1563015e4f6d51a0c`. Owner audit
  `owner-audit.json` SHA `37b15a3b4aab6f476d0112af67eec334b273423beec2d57c83e62bedac54e548`;
  independent root audit `root-review.json` SHA
  `53e41b07626f0f57916a433d62db0a270262c117455f56e0873c14e0b03e3ae4`. Root rehashed all617
  candidate payloads (**119,831,173 bytes**), original inventory hashes, source/PDF/index/
  PaperId bindings, exact1000 order/42strata, seven parsing-status changes against the old
  run, actual contradictory source excerpts, and the unchanged two-paper truth release.
  `root-audit.py` is retained alongside the receipt. Earlier runs and unlaunched9680f86 stay
  untouched. The required approximately500-paper stratified target remains unmet.
- #97 owner **`finish_corpus_proxy`**, worktree `feat/e8.3-k1-coverage`, no PR. Post-run math
  guards at8fb41d0 initially missed clipped reference/citation contexts inside array-like
  layouts. Separate reviewer reproduced that false admission; corrected source
  **`d36801faed4fd6837cd79274c4359bb75b7c2eae`** passes182 tests and8 independent probes,
  preserving original included-file spans. Receipt
  `review-math-layout-independent-d36801f/review.json`, SHA
  `4234a2977d9ca9ffd2c0bd901457991f2713e174e50f9139bc0d06b787793277`. Owner now adds
  conservative source-role guards for unparsed references/procedures, then finite source
  capabilities selected from actual exclusions. Review before another versioned full run;
  never relabel the completed failed attempt or weaken truth/score targets.
- **#101 owns the short heavy compile window**, owner **`review_ready_prs`**, worktree
  `fix/e1.1-figure-detection`, no PR. Committed graphics integration567e488/958f839 adds
  bounded installed-mutool image placements, exact caption-page coverage, shared async
  production cache/API/metric factory and independent trace generation. Eight compiled
  graphics tests passed (12.82 s, peak1,236,112 KiB); detector tests found three association
  gaps in remote numeric table columns, connected labels and image tiles. Correction
  **`eff226e`** is committed; focused tests/Clippy are being rerun. No new real measurement
  exists yet. Reviewer **`finish_benchmark`** independently checks combined source/provenance
  and unchanged fixture expectations without launching another build.
- Compact native commitment7fa66d7 independently passed9 protocol vectors,15 type probes,
  eight invalid-number cases, depth64/65 and three actual native indexes; root separately
  tested25 real indexes/67,542,405 bytes. Review
  `review-native-commitment-independent-7fa66d7/review.json`, SHA
  `b924e936e2e1e86d98b9af9e24670c6bab98823fe9505304121cb9ed98132977`.
  No actual500-paper transport/performance result yet. Independent graphics packet remains
  `review-graphics-fixtures/packet.json`, SHA
  `a44584fe4e7c94ef3363876c8827e70c4a173d45b391483559f66f2bcb03c55a`.
- #101's previous exploratory6180f0a measurement remains **O1=1.0, O2 median=0.0**, eight
  missing raster figure regions; it is an unaccepted regression. Preserve all original and
  exploratory predictions under `object-metrics/` and `figure-detection-development/`.
  Next: finish focused compile, independent combined review, unchanged-K1 remeasurement,
  then complete gates and draft PR. Do not mistake targeted tests for full G5 clearance.
- **No corpus downloader/native batch runs now.** Completed batches1–6 plus separate#98
  retry give **2,500 unique indexes: all1000eval and1549scale**,49overlap. Batch6 root receipt
  `native-index-after-batch6-review.json` SHA
  `923c3972553524d83d5b7947fd837f06cf800e49a6a2cc611890d3faba37f156`.
  Next batch **7**, after#101 releases its short window:
  `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 7 --limit 250`, followed
  by `python3 -B ~/.cache/lysilogy/audit-native-index-batch.py --batch 7`. Do not overwrite
  existing receipts or repeat a batch. Runner SHA
  `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`; audit helper SHA
  `c37667c8ef614c3df03996939a980c7a43e964925893b1f02fe3bdd48a7228e8`. Portable native
  helper remains `k1-native-page-isolation-0843453/k1_index`, SHA
  `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`.
- Corpus complete **10,951 PDFs/1000 sources**, exact eval1000/scale10000, at
  `~/Corpora/arxiv`; canonical data root `~/.cache/lysilogy/arxiv-kb-data`. Never copy/reset
  corpus or identities. Registry10951rows SHA
  `934d7fa7860d8a84639f52fbb7d77c0a721f3b7e4bc8f836b70b8063c4fa835d`. Latest disk free
  81.56 GiB, available RAM about2.9 GiB; preserve20 GiB floor and one heavy window. Native
  preparation is not actual production10k acceptance.
- **#25 / draft PR94** remains clean/pushed/independently reviewed **`df54260`**, source
  e3539d2, worktree `feat/e1.2-bibliography`. G5 **356Rust/292Python/92Node** and all gates
  pass; O8/O10=1.0 on35entries/50pairs; title30/35,author/year35/35. **Do not merge without
  genuine K2/full O9**. #103 retains five title misses. Final review
  `review-bibliography-df54260/final-review.json` SHA
  `db4fea17bc31875a00b3429ce725d09270e7e686ea972c60ab6b73fd6203416b`.
- Paused **#33 `905c341`**, `feat/e2.1-kb-store`, no PR, Rust uncompiled/missing registry
  dependencies; **#71/PR93 `d5e12aa`**, reference truth; **#72/PR95 `c867ec6`**, person labels
  stacked on93. Combined40-DOI plan under `reference-truth-plans/` remains unexecuted;
  actual K2/K4/K5/K7 unavailable. No answer to pending **index.crates.io, static.crates.io,
  api.crossref.org, api.openalex.org** grants. Original corpus grants work. Do not apply the
  prepared four-host script or bypass the earlier automatic-review rejection without
  explicit approval. No rebase, sudo, rm-rf, persistent systemctl or private/env/secrets access.
- Main scorecard **1/5 gates(G5),1/30 objectives(O30)**; draft#25 reaches O8/O10; exploratory
  #101 O1 target/O2 regression remains unaccepted. Misses retained under#101(O1/O2),
  #21/#22(historical O25/O26),#103(title). Follow-ups opened this continuation:
  **#97,#98,#101,#103**. Continue A2; all later phases, final running-app Playwright,
  all-gate/80%-objective check, actual10k scenario and final system report remain outstanding.

### 2026-09-13 — figure/table targets merged; bibliography integration resumes

- **Phase A / Wave A2**, last merged issue now **#101 / PR104**, merge
  **`a54de426d6691c013e5ea4b3333e7027d7103335`**, reviewed head
  `84a04d7e6d63b5ff451982afa154d3e4198e2473`. Branch/worktree `fix/e1.1-figure-detection`
  removed after all evidence was retained externally. Main's ten unrelated preview files
  still match `/tmp/lysilogy-preview-before.json`; main mutations remain docs-only except
  fast-forwarding merged commits. Earlier continuation merges #35,#36,#91,#88,#70,#98,#96.
- Actual unchanged K1 **O1=1.0 (15/0/0), O2=0.9107793204006069**, all15 truth regions,
  zero unknowns. One region is absent; four traced pages are supported/two unsupported.
  Preserve original0.76923/0.24485 and exploratory1.0/0.0 results. **G5 350Rust/282Python/
  85Node** and all required gates pass. Main now **1/5 gates,3/30 objectives(O1/O2/O30)**.
  This is a limited manually selected two-paper result; #97 retains approximately500papers.
  Report `docs/experiment-reports/2026-09-13-figure-detection.md`; exact committed evidence
  `eval/evidence/figure-detection.json` and `figure-detection-observations.json`.
- Final separate PR review `review-graphics-final-84a04d7/final-review.json` under cache,
  SHA `fea25c02d4fa0413ec3931d399bf33889c69c6fd1620fd34cda63865087167df`, verified157hashes,
  source/review/gate bindings and every IoU. Root independently verified all15 native caption
  memberships and regions. Actual receipt directory
  `figure-detection-development/0722e9a-first-graphics-measurement/`, root region review SHA
  `5843f40fabede3a057f7aa78688923338c3179758b7cdba86d372760bf3ce011`, caption review SHA
  `e02aaffeebc36a085cf6b4e4d097c87c093c6166c4fbfddc6fc6cff6df1cf163`. Portable archive
  `figure-detection-development/release-281785b/` retains required logs and only the measured
  executable, not the whole target tree. Root portability review
  `review-evidence/pr104/root-portability-review.json` SHA
  `eb8e4ad844316698317978ce5b9a91646e9fb9a07fd680918e67d180f2f13e45`:71artifacts/6,575,353B.
- **#25 / draft PR94**, owner **`finish_benchmark`**, worktree `feat/e1.2-bibliography`.
  Last clean/pushed head **`df54260`** (sourcee3539d2) was independently cleared with
  G5 356/292/92 and O8/O10=1.0, title30/35, author/year35/35. Owner is now assigned ordinary
  current-main integration, preserving new async graphics factory/cache/API/compact native
  commitments while retaining bibliography behavior. Inspect worktree for current progress;
  previous gates do not certify the new integration. **No rebase; do not merge PR94 without
  actual K2/full O9.** #103 retains five title-field misses. All old evidence stays immutable.
- **#97**, owner **`finish_corpus_proxy`**, worktree `feat/e8.3-k1-coverage`, no PR. Latest
  source checkpoint **`b14e32b`** adds document-body-only heading scanning over declaration
  capability **`f389393`**. Declarations are **not yet independently clear**: root reproduced
  occupied environment namespace collisions (`claim/endclaim` from newtheorem, built-in
  equation/endequation) wrongly certified as fresh operators. Both false declaration proofs
  can hide a direct theorem invocation and falsely admit O5. Reproducer
  `review-ams-operator-f389393/paired-environment-collision.json`. Owner notified to reject
  environment entry/exit names and forbidden end-prefixed targets; review corrections before
  any full run/admission. Root currently owns this source review; `review_ready_prs` is idle
  after completing#101 and can take a bounded independent task via `followup_task` if needed.
- Earlier source-role guards are independently clear at **`2409e489801f5275a1bdffaf026960f5c4c73339`**:
  unparsed bibliography/procedure roles, correct section hierarchy through child headings,
  explicit Step1/Algorithm1 forms and source provenance. Root113targeted tests/14final probes,
  plus12included-file hierarchy controls; receipt
  `review-source-roles-2409e48/review.json` SHA
  `8956f7bbdc00d4727b67a6f3f729146af23d8df9fdb8792359183facbb490c51`. Math-layout/link guards
  d36801f were separately clear. Do not overwrite old2eb4826/bd493a4 findings or failed probes.
- Latest full automatic attempt remains immutable **`k1-full-alignment-4ada537/`**:
  617parsed/383failed, rawadmission1 contradicted by manual references/procedure, **zero
  defensible new truth papers**,997.138481s/499824KiB/zero external calls/$0. Root final audit
  `root-review.json` SHA `53e41b07626f0f57916a433d62db0a270262c117455f56e0873c14e0b03e3ae4`.
  Original1000 inputs and999-success map unchanged; separate#98 retry never inserted into
  this comparison. Prior9680f86 setup remains unlaunched. No new truth release/full run.
- Source capability bounds from all617frozen candidates are retained at
  `k1-coverage/capability-bounds-4ada537.json` SHA
  `d3ff860f8fcf3b24558dbaadc03245578993208c0cd219c050a7f09fabfe3492`: atoms alone recover0;
  even proving all unknown commands caps inventory candidates at34, or59 with all math-layout
  inventory support; all nonstyle semantics/environments still cap at301. Reaching500 also
  needs local-style proof or failed-paper recovery. No target/quality relaxation. Source-only
  f389393 pilot14papers/13parsed verified16declarations in7papers and unchanged original
  object inventories/spans,2.024s/158960KiB/no external calls. Receipt
  `k1-coverage/operator-source-pilot-f389393.json` SHA
  `4e7387eea4b399c962ea3683b6b52556cfc7785bc7c9390652a18e29905375aa`; exploratory, not admission.
- **Scale batch9 RUNNING, retained TTY exec20377**, owns the heavy window:
  `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 9 --limit 250`.
  Complete/audit it before another Rust/G5/native job. Batch7 finished250/250,383.659913s,
  221424KiB; batch8 finished250/250,392.288791s,204784KiB,zero external calls/$0.
  **3,000 unique indexes: all1000eval and2049scale**,49overlap; canonical registry bytes
  unchanged. Root rehashed706,751,207B(batch7)/733,776,562B(batch8). Receipts
  `native-index-after-batch7-review.json` SHA
  `53686e1194c9c50d29055ed8ded3b1518900c11198a5fdfdc13b1368e5daca4b`, and batch8 SHA
  `d1e781dd8abe1cb663e7e719c5466350c665716d02c8054b17936e21a19fe3e1`.
  After9 finishes, `python3 -B ~/.cache/lysilogy/audit-native-index-batch.py --batch 9`;
  next batch10. Runner SHA `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`,
  audit helper SHA `c37667c8ef614c3df03996939a980c7a43e964925893b1f02fe3bdd48a7228e8`.
- No corpus downloads run: **10951PDFs/1000sources**, exacteval1000/scale10000, all verified
  at `~/Corpora/arxiv/`; canonical data `~/.cache/lysilogy/arxiv-kb-data`. Never reset/copy
  corpus or identities;20GiB floor. Portable native helper
  `k1-native-page-isolation-0843453/k1_index` SHA
  `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`. Index preparation is
  not the production10k scenario or O25/O27 acceptance.
- Paused **#33 `905c341`**, `feat/e2.1-kb-store`, no PR, Rust uncompiled/missing registry
  dependencies; **#71/PR93 `d5e12aa`**, reference truth; **#72/PR95 `c867ec6`**, person labels
  stacked on93. Combined40-DOI plan under `reference-truth-plans/` remains unexecuted;
  actualK2/K4/K5/K7 unavailable. Still no approval for **index.crates.io, static.crates.io,
  api.crossref.org, api.openalex.org**. Original corpus grants work. Do not apply prepared
  four-host permission script or bypass previous automatic-review rejection without explicit
  approval. No rebase/sudo/rm-rf/persistent systemctl/private/env/secrets access.
- Next: finish/audit batch9, review#97 declaration correction, integrate/review#25 against
  newmain, continue source capability work and native batches. Historical O25/O26 misses
  remain#21/#22; title component#103; coverage#97. Follow-ups opened this continuation:
  #97,#98,#101,#103. All later phases, final running-app Playwright, all-gate/80%-objective
  evaluation, actual10k scenario and final system report remain outstanding.

### 2026-09-13 — bibliography integration reviewed; broader truth work selected

- **Phase A / Wave A2**, main `d51ff9533184bbdb1f570dc960540e503407f3d9`; last merged
  issue **#101 / PR104** (`a54de426d6691c013e5ea4b3333e7027d7103335`). Its worktree and
  branches are removed. Earlier continuation merges: #35, #36, #91, #88, #70, #98, #96.
  Main's ten unrelated PDF-preview files still exactly match
  `/tmp/lysilogy-preview-before.json`; mutate only docs in this checkout.
- Main scorecard remains **1/5 hard gates (G5), 3/30 objectives (O1, O2, O30)**.
  Actual limited-K1 O1=1.0 (15TP/0FP/0FN), O2=0.9107793204006069 over all15 bodies.
  This two-paper release is not broad coverage. Preserve all original and failed exploratory
  detector results. Historical O25/O26 misses remain #21/#22; title-field misses remain #103.
- **#25 / draft PR94** is clean/pushed and independently clear at
  **`201c53c7a7041f7740b68e7495e4b93358f8639b`**, tested source
  `347de697a87c72ea1d6d3ff41ee0eff66f2e78cb`, worktree `feat/e1.2-bibliography`.
  Ordinary main merges retain the shared async graphics factory, current cache generation,
  compact native commitment and schema2 bibliography artifact. Final gates pass:
  **388 Rust / 296 Python / 92 Node**, Clippy/fmt, frontend gates, production-artifact
  Playwright smoke and affected suites. K1 O8=1.0 (35 entries), O10 precision/recall=1.0
  (50 pairs); known title30/35, first-author/year35/35. All15 figure outcomes and O1/O2
  match the merged implementation exactly. **Do not merge without genuine K2/full O9.**
  The draft has 5/30 objectives at target; main has not acquired its O8/O10 measurements.
- PR94 final independent review is
  `~/.cache/lysilogy/review-bibliography-final-201c53c/final-review.json`, SHA
  `07fc3ce54d79c33ac4ea1abd1377ba9ab45639e6e0e91ab8fb66743ff20f7d42`:193 source
  fingerprints at tested/final heads,330 referenced files including71 portable artifacts,
  raw gates, unchanged real outcomes and two viewed screenshots. Root source/compiled/
  actual-measurement audits are in `review-bibliography-integration-347de69/`;
  `measurement-review.json` SHA
  `3a3a4f28a5735389fc693cff25290f47d5eca88ffe25e9107b8d9720d53e637d`.
  Actual bibliography0.823258s and objects7.763569s, zero external calls/$0. All historical
  measurements remain immutable. Owner `finish_benchmark` is now auditing whether paused
  #33 has remaining useful offline work; no build/dependency change authorized by that task.
- **#97**, owner `finish_corpus_proxy`, worktree `feat/e8.3-k1-coverage`, no PR. Safe source
  **`8a8f58cd769a89fe5a44c9bf2b4d3dd376ff59e8`** independently passes198 tests and11
  adversarial probes. Stored/deferred package tokens cannot prove an executed initial import
  prefix; every unproved imported operator namespace stays excluded. Literal-definition and
  original-span evidence remain available, with no admission flag. Review
  `review-k1-operators-8a8f58c/final-review.json` SHA
  `8a870c0134b3381786e252180a5b6380b620cca4e81d3f2771157595c1306b1f`. Prior38f27a0
  pilot's18 flags in8 papers are an invalid partial namespace proof, not truth. Last known
  clean docs checkpoint `d98a0b3998a92ebd907cd8d614af1e5b638f8a4c`; owner is binding this
  final review into the report, now clean at `80376e2c5cab60d1b6756fa051f0e7b19760dd22`.
  Full own-worktree gates remain due.
- #97 last complete comparison stays **617 parsed / 383 failed / raw accepted1 / zero
  defensible new papers**,997.138481s/499824KiB/no external calls/$0. Preserve
  `k1-full-alignment-4ada537/` and its independently audited617 payloads/119,831,173 bytes.
  Root audit SHA `53e41b07626f0f57916a433d62db0a270262c117455f56e0873c14e0b03e3ae4`.
  Raw candidate2007.05954 has unparsed manual references/procedure; never publish its
  negative-cohort claims. Original1000 inputs and999-success index map remain fixed;
  separate#98 retry never retroactively enters them. Prepared9680f86 remains unlaunched.
  Original limited-v1 objects/bibliography bytes remain unchanged.
- The measured capability ceilings show that small command adapters cannot deliver500
  papers. The101-paper DOI survey found94 exact fallback templates,71 with no known doi
  use,41 additionally without other dynamic/low-level definitions,14 additionally without
  local styles; these are diagnostics, not admissions. Receipt
  `k1-coverage/doi-conditional-survey-4ada537.json` SHA
  `05c393f1e3596ca25e93a0af9158bc085bcd7b4540e159dccb00170a49a42e74`.
  Generated trusted namespace probing is design-only; a current installed namespace is not
  evidence of historical arXiv compilation. No TeX probe or new full alignment run launched.
- Next #97 work is a concrete independently annotated tranche: freeze21 candidates, three
  per existing category in frozen corpus order, using only source/PDF/index availability
  and an explicit page cap, never detector outcomes. Begin with7, one per category, to
  measure annotation throughput and quality. Reuse existing blind source/PDF inventory and
  reconciliation tooling, exact input bindings and versioned releases. This is a milestone,
  not a replacement for approximately500; #97 remains open. Owner prepares selection and
  assignments offline; `review_ready_prs` independently assesses the bounded next experiment.
- **Scale batch11 RUNNING, retained TTY exec96102**, owns the sole heavy window:
  `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 11 --limit 250`.
  Completed batches1–10 plus separate#98 retry now give **3,500 unique indexes: all1000
  eval and2549 scale**,49overlap. Batch9 completed250/250,414.260813s/229168KiB;
  batch10 completed250/250,402.820288s/213184KiB; zero network/models/$0.
  Root rehashed706,810,945B(batch9) and709,997,258B(batch10). Latest audit
  `native-index-after-batch10-review.json` SHA
  `91fb8dbab6e899fbbba6b537b6a1d09838eb425425c76de3d94edd96cde3d6db`.
  Poll session and inspect terminal receipt, then run
  `python3 -B ~/.cache/lysilogy/audit-native-index-batch.py --batch 11`; next batch12.
  No Rust/G5/native job while batch11 runs. Runner SHA
  `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`, audit helper SHA
  `c37667c8ef614c3df03996939a980c7a43e964925893b1f02fe3bdd48a7228e8`.
- No downloads run. Complete corpus **10,951 PDFs / 1,000 sources**, exacteval1000 /
  scale10000, lives at `~/Corpora/arxiv`; canonical data root
  `~/.cache/lysilogy/arxiv-kb-data`. All10951 registry identities/bytes remain unchanged,
  SHA `934d7fa7860d8a84639f52fbb7d77c0a721f3b7e4bc8f836b70b8063c4fa835d`.
  Portable native helper `k1-native-page-isolation-0843453/k1_index`, SHA
  `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`.
  Preserve20GiB free-space floor; do not copy/reset corpus or data. Native index preparation
  is not production10k app acceptance or O25/O27 performance.
- Paused **#33 `905c341`**, `feat/e2.1-kb-store`, no PR; Rust uncompiled/missing rusqlite
  dependencies. Preserve its unrelated dirty.gitignore and target resume notes. Paused
  **#71/PR93 `d5e12aa`**, `feat/e8.4-reference-truth`; **#72/PR95 `c867ec6`**,
  `feat/e8.5-person-labels`, stacked on93. ActualK2/K4/K5/K7 unavailable; combined40-DOI
  plan under `reference-truth-plans/` remains unexecuted. Still no answer approving
  **index.crates.io, static.crates.io, api.crossref.org, api.openalex.org**. Original corpus
  grants work. Do not apply prepared four-host permission script or bypass the earlier
  two-host automatic-review rejection without explicit approval. No rebase, sudo, rm-rf,
  persistent systemctl or private/env/secrets access.
- Follow-ups opened this continuation: **#97, #98, #101, #103** (#98/#101 now closed).
  The #33 readiness audit found no useful remaining offline implementation gap:26 written
  store tests await compilation, followed by Clippy/G5, fresh-root smoke, actual O28 and
  genuine K2-backed G4. Receipt `kb-store-readiness-905c341/readiness.json`, SHA
  `4c75e3419470353d422988377d9499642cacff6d6734624fe1d396cf41dcdd06`.
  Next: audit batch11, finish safe K1 checkpoint and deterministic annotation selection,
  continue scale preparation and unblocked A2 work. All later phases, final running-app
  Playwright scenario, 5/5 gates and24/30 objectives, actual10k scenario and final system
  report remain outstanding. The system is not complete.

### 2026-09-13 — seven-paper blind annotation pilot begins

- **Phase A / Wave A2**, main before this docs checkpoint
  `21ef86b3972520140f286cbd5e44ff9a1377fe18`. Last merged issue remains **#101 / PR104**,
  merge `a54de426d6691c013e5ea4b3333e7027d7103335`; worktree/branches removed. Earlier
  continuation merges #35/#36/#91/#88/#70/#98/#96. Main's ten unrelated PDF-preview files
  still match `/tmp/lysilogy-preview-before.json`; main working-tree edits stay docs-only.
  **1/5 gates (G5), 3/30 objectives (O1/O2/O30)** on main. Limited two-paper O1=1.0,
  O2=0.9107793204006069 over all15 regions. No phase exit/system acceptance yet.
- **#25 / draft PR94** remains independently clear, clean/pushed at `201c53c`, tested
  source347de69; worktree `feat/e1.2-bibliography`. G5 **388Rust/296Python/92Node**, all
  integrated gates and frontend smoke pass; O8/O10=1.0 on35entries/50pairs, known title30/35,
  author/year35/35. Its actual figure outcomes equal merged#101. **Do not merge without
  genuine K2/full O9.** Final review `review-bibliography-final-201c53c/final-review.json`
  SHA `07fc3ce54d79c33ac4ea1abd1377ba9ab45639e6e0e91ab8fb66743ff20f7d42` under
  `~/.cache/lysilogy/`. Draft has5/30 objectives; main has3/30. #103 retains title misses.
- **#97**, worktree `feat/e8.3-k1-coverage`, no PR. Clean owner checkpoint
  **`7c6296e363ebe47b5523fb043b46f93756442ddb`**, ordinary-main integration
  **`8f8105a3ba8b29ec3fece1e08c6f81df4b7b22c6`**. Conflicts only in object-metrics.py and
  its tests: preserve isolated v1 replay plus current#101 graphics/native derivation and
  scoring. **9 versioned +31 collector tests pass**; all74 pinned parser/support/truth/
  packet files unchanged. Full own-worktree Cargo/G5 and final measurements remain due.
  Parser source remains independently clear8a8f58c,198 tests +11 independent probes;
  unproved imported operator namespaces stay excluded. No TeX probe or new full run.
- Root independently checked integration diff/AST, owner raw test logs, all74 protected
  hashes and actual original-v1 replay through the current collector: both original truth
  payloads reproduce byte-for-byte in4.597661s,zero external calls/$0. Review directory
  `k1-coverage/manual-tranche-v1/root-merge-review-8f8105a/`; `review.json` SHA
  `57b2ef82e08321d98c4e1e4569e3e8cecc74ca6de283f597818cb8edbb206d41`,
  `protected-inputs-review.json` SHA
  `adc7b47fd6d079a597f799155add5bd99a9add362fab4362adb8c85590c1e664`.
  Actual replay receipt SHA `5d57ded4ed5eddb6e6f3f1b018c4d5f454278634daa72bd9b7640357c50f8586`.
- #97's latest full comparison is still **617parsed /383failed /rawaccepted1 /zero
  defensible new truth papers**,997.138481s/499824KiB/no external calls/$0, retained in
  `k1-full-alignment-4ada537/`. Raw2007.05954 falsely claimed negative cohorts despite
  manual references/procedure; never publish it. Root audit SHA
  `53e41b07626f0f57916a433d62db0a270262c117455f56e0873c14e0b03e3ae4`. Historical runs,
  unlaunched9680f86 setup, original1000 inputs/999-success index map and separate#98 retry
  remain separate and immutable. Approximately500-paper coverage remains unmet.
- **New frozen annotation pilot** lives at
  `~/.cache/lysilogy/k1-coverage/manual-tranche-v1/`. Policy SHA
  `d69801ad32c78bec8ef7cd2d8107a196980e0653ce98f715e7e40754103ad4d4`: original input
  order, page cap8, three new papers/category planned, first one/category active; exclude
  only existing two releases or unavailable/unsafe artifacts, abort on hash contradictions.
  No parser/detector/alignment success selection or replacement after annotation failure.
  All21 planned slots filled. Full1000 ledger:214 page-cap exclusions, one original-index
  failure, two already released,21selected,762later quota-filled. Selection took9.436s,
  187936KiB,zero calls/$0. `selection.json` SHA
  `0ba9a356987f4f93787b4df41a0c32cd1ff061ea4b69270c63e883187ef6e6ca`.
- Root independently reproduced every ledger row and all21 selected artifact/member
  bindings. `root-selection-review.json` SHA
  `689c325f45c37073bcb3d3fa7cf6bb8f4757649d439467d9c473ec66a884a28c`.
  Active seven, in frozen order: **2210.11141v1 cs.CV/2022 5p;2310.04162v1 cs.RO/2023 8p;
  2001.05217v1 hep-th/2020 7p;2409.03655v1 cs.LG/2024 6p;2002.03492v1 econ.TH/2020 5p;
  2207.03024v1 stat.ML/2022 5p;2303.07834v2 math.PR/2023 7p**. Explicit short-paper
  selection bias; seven/21 are milestones, not replacement targets or published truth.
- All43 complete original pages rendered96dpi; root separately verified source exports
  against every safe raw member and native exports against exact text/page/token allowlists.
  Embedded historical figures and all parsed/aligned/detector outcomes are omitted.
  `root-render-review.json` SHA
  `1ec4db9eb44fca205e457ddb7d3282fdc189ec201616207f9bf5f80c4f61bfd2`,2.851385s/72464KiB,
  zero calls/$0. Each `packets/<arxiv_id>/packet-rendered.json` binds actual PDFs/sources/
  indexes,exports and all full images. Pre-render packets remain unchanged. The frozen
  selector is named select.py and shadows Python's stdlib select: render launch used
  `python3 -P -B`, preimported subprocess, then inserted the tranche path and ran root-render.py
  through runpy. First import-only startup failed before reading/rendering anything; do not
  mistake it for a PDF failure or rerun the completed exclusive output directories.
- **First paper2210.11141 annotations RUNNING**. Primary **`finish_benchmark`** owns
  `primary/2210.11141/`; independent **`review_ready_prs`** owns `independent/2210.11141/`.
  Both read the frozen `blind-annotation-prompt.md`, inspect all original images before
  source/native association, never read each other's labels or raw detector-bearing indexes,
  and write immutable inventory.json/receipt.json. Both have independently reported6figures,
  2tables,4numbered equations,25bibliography entries,49citation groups and20source refs;
  manual procedure/list roles remain explicit. Entry14 crosses columns; preserve disjoint
  memberships. **Neither final inventory/receipt was received at this checkpoint.** Wait
  for both frozen artifacts, then root reconciles completeness/ambiguities/source spans.
  Root also viewed all5pages independently; preparatory visual receipt is
  `root-visual-preparation-2210.11141.json` SHA
  `0f976eedcf2e21a6ddd0a60a4bc21317563759aff17f64f1a697992a44b5495a`; not truth admission.
- #97 owner **`finish_corpus_proxy` is idle** at7c6296e; use `followup_task` after dual
  annotation freeze/root reconciliation. Do not give one annotator the other's labels before
  freeze. Demonstrated assembly issues: region/object bundle exclusivity, missing typed
  figure/table reference roles outside O4, raw-index path requirement despite proper blinded
  exports, all-math-kind eligibility coupling and cohort denominator coupling. Source-linked
  assessment `assembly-gap-assessment.md` SHA
  `91bb175ebc1acccb606fda2b386daa3c86f923529cf21416210c84a70b7c44eb`, receipt SHA
  `bc5889bc06ef3a2c34df6a864cd7489a9bd3d63815ace764d42c42c4fed4bb83`. No adapter edits
  yet. Start with demonstrated smallest change: independently validate each overlay against
  original bytes, combine overlay fields, retain typed reference roles and actual blinded
  export paths; keep per-kind omissions/ambiguities and v1 replay immutable. Broader manual
  occurrence/per-kind support follows reconciled evidence, without new trust flags.
- **Scale batch13 RUNNING, retained TTY exec64315**, sole heavy window:
  `python3 -B -u ~/.cache/lysilogy/run-scale-index-batch.py --batch 13 --limit 250`.
  Completed batches1–12 plus separate#98 retry give **4,000 unique indexes:1000eval and
  3049scale**,49overlap. Batch11:250/250,403.129014s/334960KiB; batch12:250/250,
  391.310517s/216320KiB,zero external calls/$0. Latest root audit
  `native-index-after-batch12-review.json` SHA
  `2970ec0df6e84de0bb28de4718c27ba8566ca6afb0594929570c798b40fef64d`, rehashed723,648,477B.
  Poll terminal/receipt, then `python3 -B ~/.cache/lysilogy/audit-native-index-batch.py
  --batch 13`; next14. One Rust/G5/native job at a time; lightweight source tests/review may
  continue. Runner SHA `9d8ae083a30808110d09cb94aadad1aeb7bdcc2c76203a8a6c7e07ca0347325b`,
  audit SHA `c37667c8ef614c3df03996939a980c7a43e964925893b1f02fe3bdd48a7228e8`.
- No corpus downloader. Complete **10,951 PDFs/1,000sources**, exact1000eval/10000scale,
  at `~/Corpora/arxiv`; canonical data `~/.cache/lysilogy/arxiv-kb-data`. Registry10951
  rows/bytes unchanged, SHA
  `934d7fa7860d8a84639f52fbb7d77c0a721f3b7e4bc8f836b70b8063c4fa835d`. Portable native
  helper `k1-native-page-isolation-0843453/k1_index`, SHA
  `3f25b41305452eac7bdfde2b02f77f2dc664b89aa3f5530e26125ed3cda2e4f7`. Preserve20GiB
  floor; no corpus/data reset or copies. Index prep is not actual production10k acceptance.
- Paused #33 `feat/e2.1-kb-store` **905c341**, no PR, Rust uncompiled/missing rusqlite
  dependencies;26written tests, no further useful offline implementation gap. Preserve
  dirty.gitignore/target resume notes. Paused #71/draftPR93 `feat/e8.4-reference-truth`
  **d5e12aa**; #72/draftPR95 `feat/e8.5-person-labels` **c867ec6**, stacked on93.
  Genuine K2/K4/K5/K7 missing; combined40-DOI plan remains unexecuted. No approval for
  **index.crates.io, static.crates.io, api.crossref.org, api.openalex.org**. Do not apply
  prepared four-host permission script or bypass prior two-host automatic-review rejection.
  Original corpus grants work. No rebase/sudo/rm-rf/persistent systemctl/private/env access.
- Next: reconcile the two immutable first-paper inventories, assign next frozen paper,
  resume owner on demonstrated adapter gaps, audit batch13/continue14. Follow-ups opened
  this continuation #97/#98/#101/#103; #98/#101 now closed. Coverage#97/title#103 and
  historical O25/O26#21/#22 remain. All later phases, final Playwright app scenario,
  5/5 gates and24/30 objectives, actual10k scenario and final system report remain pending.


### 2026-09-13 — three blind annotations frozen; mixed-bundle adapter review

- Main before this checkpoint is2ad503d. PhaseA, WaveA2; no phase exit. Last merge
  remains #101/PR104, mergea54de426d6691c013e5ea4b3333e7027d7103335; owned branch and
  worktree removed. Current scorecard **1/5 gates,3/30 objectives(O1,O2,O30)**. O1=1.0,
  O2=.9107793204006069 on all15 objects in the immutable two-paper release. All later
  phases, five hard gates,24/30 objectives, production10k scenario and final app/report
  remain outstanding. Protected ten unrelated PDF-preview file hashes unchanged.
- #97 `feat/e8.3-k1-coverage`, `.worktrees/feat/e8.3-k1-coverage`, owner
  `finish_corpus_proxy`, now clean source **e64b9d85bf0c4aefe9627bf175a01aeed4992cc8**.
  It contains ordinary main merge8f8105a and independently cleared parser8a8f58c;
  no rebase. New narrow adapter composes separately verified visual/math/bibliography
  overlays against original candidate bytes, preserves typed figure/table references
  outside O4 and verifies the exact native export actually shown to blind annotators.
  Owner reports210 offline Python tests-Werror passing,12new; root independent review
  is in progress. No expanded truth assembled/admitted. All51 pinned verifier/truth/
  blind-packet files unchanged. Original v1 truth and retained historical implementations
  remain immutable; previous actual replay reproduced both payloads byte-for-byte.
- Owner's before-change own-CLI build/check receipt is
  `~/.cache/lysilogy/k1-coverage/manual-adapter-before-7c6296e/receipt.json`, SHA
  `91df0d363255b7cbc14611b09b9e312552be414d1f8288979e93348a0404293d`.
  Actual objects and bibliography `--check` passed; no fresh collector or truth build.
  After batch14 completed, owner was given the next sole heavy build/gate window for
  e64b9d8; await terminal receipts before starting batch15. Use one native/Rust/G5 job,
  CARGO_BUILD_JOBS1, dev/test debug0, incremental0. Lightweight annotation/review continues.
- Frozen seven-paper blind pilot is under
  `~/.cache/lysilogy/k1-coverage/manual-tranche-v1/`. Policy/selection/render/export hashes
  are recorded in the prior entry and remain unchanged. All43 original pages are rendered;
  exports include only exact native text/pages/tokens and safe raw deposited source. No
  raw detector-bearing reading indexes or other annotator labels are shown to annotators.
  First three dual annotations are now frozen; fourth2409.03655v1 is RUNNING. Primary
  `finish_benchmark` and independent `review_ready_prs` own their separate immutable
  `primary/<id>/` and `independent/<id>/` inventories and receipts. Use followup_task to
  assign later papers after freeze, in order2002.03492v1,2207.03024v1,2303.07834v2.
  Seven/21 planned milestones do not replace the approximately500-paper coverage target.
- First2210.11141v1: both found6figures,2tables,4equations,25bib/75fields,49citation
  groups/75pairs,20source refs. Root checked232primary and207independent source spans,
  native memberships, all full original pages and independent evidence. Initial comparison
  SHAae8df874fe30bf0b47b7cab33959fcd15455702a25b4f53fb5b2cd69b6ba2204 found17
  independent fields lost26 accents/hyphens during normalized character lookup. Original
  labels remain immutable. Independent inventory-v2 SHA
  `29aa1a0b1010e252323d8c13f2e3566c935c674129b1cf7314144fe071c659a6` corrects exact
  raw membership only; all75 now independently match primary. Root correction review
  `reconciliation/2210.11141/root-field-correction-review-v2.json` SHA
  `31f2903c9055153d4e9ef7070b7e20931066faf00a509b6b2fdea1f13e0cd106`. Five genuine
  source/native wrap or missing-hyphen gaps remain explicit; no invented glyphs.
- Root re-rendered first-paper pages2–4 at192dpi and visually reconciled complete eight
  graphic/table regions, retaining all labels/subpanels and separate captions. Root region
  reconciliation SHA001c12e6b58f0b7add19cfb53df78294d1356f082c69471f21dc9d7d1b5745a5.
  Role reconciliation SHA11dea94e9c9a8c4031e60806f455a3f5a2453dd48cdcbe3040099ba251882866:
  task-definition paragraph, two prose lists and four narrative passages remain explicit
  candidates, outside original E1.4 formal statement/proof and E1.5 captioned Algorithm/
  code-listing scope. No automatic guard or objective target changed. These are supplements,
  not truth admission. Remaining assembly prerequisites: post-freeze source-inventory/
  packet reconciliation and independently drawn four math-body visual boxes. Owner is
  preparing exact requirements; do not fabricate geometry or accepted/completeness flags.
- Second2310.04162v1 frozen: primary inventory
  `ee3e94419a17895f51f1934ea37656628277f0a423cdff8fa4431f4ec1c9583e`, receipt
  `45917c3e97bcc07f1c01e3c2ec32046cd92e1578b1cf3a660141cf2dcd82fbf7`,490.575s;
  independent inventory`2a5786952a7dd26b2888fb8c78e70e2069607ee6b80dac5787c86c86f52ce847`,
  receipt`9b1c95ae3bd656e118194ad8a3198529394ff309fe86425d34017ae405b6ea71`,634.33738s.
  Both found8figures,5tables,22eq,20bib/60fields,32citationgroups/40pairs,26object refs
  and5section refs. Three author-numbering inconsistencies remain unresolved. Independent
  raw field audit59normalizationmatches/one genuine missing-native-hyphen gap. Prose
  candidate segmentation differs and needs root reconciliation. Second schema uses
  occurrence_id/caption_native/visual_body/native/target_pairs; adapt comparator explicitly,
  retaining first-paper immutable outputs. Root still needs full eight original-page views.
- Third2001.05217v1 frozen: primary inventory
  `5a12b5bbcff11675604e34ef56d4bb99d1a068543a8e4d2eaa213a486c8274c7`, receipt
  `be9b2abe68741f5f168ba077c20d5964442dbf6fd97bac7b284ce77be5c1924b`,495.526s;
  independent inventory`3b76d4023b89b8b93317f41716925e752d1136ef5806cf084d6c6ea1cbb20cfe`,
  receipt`f8a4beb4f0881a952eda90548baebcb911d5680de1205145a9e295a6808f3541`,449.179735s.
  Both4figures,14eq,15bib,10citationgroups/17pairs,13objectrefs; independent additionally
  records one next-section ref. Two collective authors have no named individual first author;
  field eligibility needs reconciliation. Math fidelity/definition/procedure roles explicit.
  All annotation receipts record zero external calls; agent reasoning cost is unknown.
- #97's latest full automated run remains4ada537:617parsed/383failed/rawaccepted1/
  **zero defensible new truth**,997.138481s/499824KiB/zero calls/$0. Historical ledger
  stays unchanged. No namespace probe/new full run. Approximately500 coverage remains unmet.
- **Scale batches1–14 complete**, plus separate#98 retry: **4,500 unique indexes,
  all1000eval and3549scale**,49overlap. Batch14:250/250,436.895303s/305392KiB,zero
  network/model calls/$0; output0267f750538fbbcbd5cb1bf1db5cb3c3342a05e52f83807030d7126102d81108.
  Root rehashed821,016,778 new index bytes; audit
  `~/.cache/lysilogy/native-index-after-batch14-review.json` SHA
  `996543d3f9bfc390785e7d7af0e01570b594b615285993cad227659ec92c89e4`.
  Terminal exec81555 completed exit0; do not poll again. Next batch15 using retained
  run-scale-index-batch.py --batch15 --limit250 after owner's heavy window releases.
  Corpus complete10951PDFs/1000sources at~/Corpora/arxiv; no downloader. Canonical data
  ~/.cache/lysilogy/arxiv-kb-data; registry10951 SHA934d7fa7860d8a84639f52fbb7d77c0a721f3b7e4bc8f836b70b8063c4fa835d
  unchanged. Preserve20GiB free-space floor. This is indexing preparation, not production
  O25/O27 or10k acceptance. Portable helper/runner/auditor remain those in the prior entry.
- Draft#25/PR94 `feat/e1.2-bibliography`201c53c remains independently clear, paused for
  genuine K2/fullO9. Actual limited O8/O10=1.0, title30/35, author/year35/35; #103 retains
  five title misses. G5388Rust/296Python/92Node; full source/compiled/runtime evidence
  remains in prior entry. Do not merge based on the small K1 cohort alone.
- Paused#33 `feat/e2.1-kb-store`905c341, noPR, uncompiled/missing rusqlite dependencies;
  paused#71/draftPR93 `feat/e8.4-reference-truth`d5e12aa and#72/draftPR95
  `feat/e8.5-person-labels`c867ec6. No response to pending permission for index.crates.io,
  static.crates.io,api.crossref.org,api.openalex.org. Four-host script is prepared but NOT
  applied; prior automatic review rejected a persistent two-host expansion without exact
  authorization. Do not bypass. Genuine K2/K4/K5/K7/production G4/O28 remain unmeasured.
  Original corpus storage/OAI/export/GCS permissions work. No rebase/sudo/rm-rf/private/env
  access. Preserve unrelated worktrees and store dirty.gitignore/resume notes.
- Next: review e64b9d8 adapter independently; reconcile second then third frozen annotations;
  obtain first-paper exact supplemental math regions and source-inventory reconciliation;
  finish remaining four blind papers; await owner after gates, audit/continue scale15. No
  new issue merges or follow-ups at this checkpoint. This continuation opened#97/#98/#101/
  #103; #98/#101 now closed. Coverage#97,title#103 and historical#21/#22 remain open.
