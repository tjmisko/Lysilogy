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
- [ ] **#33 E2.1 SQLite store, migrations, and rebuild** (no issue blocker; sequenced after #34 to
  avoid `src/kb/` conflicts). Branch `feat/e2.1-kb-store`. Owns `src/kb/store/`; adds `rusqlite`
  (`bundled`) to `Cargo.toml` and `kb rebuild` to `src/main.rs`. Database at
  `<data>/kb/kb.sqlite`, WAL mode, FTS5 trigram tokenizer (confirm the bundled SQLite enables
  FTS5). Include the 500k-Work synthetic neighborhood query benchmark from the design section.
- [ ] **#35 E2.3 Person name parser** (after #34). Branch `feat/e2.3-names`. Owns
  `src/kb/names.rs`. Pure functions plus a large table-driven test.
- [ ] **#36 E2.4 Title normalizer** (after #34). Branch `feat/e2.4-titles`. Owns
  `src/kb/titles.rs`. Pure functions; the FTS5 index population itself lands with #33 or #38,
  whichever merges later.
- [ ] **#71 E8.4 Reference, acquisition, and read-next truth** (after #68, #63). Branch
  `feat/e8.4-reference-truth`. K7 needs parsed bibliographies of the scale tier; build the K2 and
  K5 parts first and finish K7 once #25 and the corpus mapping are available.
- [ ] **#72 E8.5 Person silver labels** (after #68, #63). Branch `feat/e8.5-person-labels`.

### Wave A3

- [ ] **#70 E8.3 arXiv LaTeX object truth** (after #68, #69, #24). Branch
  `feat/e8.3-latex-truth`. Owns the LaTeX parser and PDF aligner used only for evaluation.
- [ ] **#37 E2.5 Resolution gold set and evaluation** (after #25, #69). Branch `feat/e2.5-gold-set`.
  Owns `src/kb/gold/`; reports through the E8.1 `resolution` suite. Mine ~200 name and title pairs
  from parsed bibliographies in `local-articles` and the arXiv corpus, including hard negatives. Store only bibliographic
  metadata, never full text. Label with agents: two independent labeler subagents, then an
  adjudicator subagent for disagreements, each writing rationale to the plain-text gold file.
  Commit the labeling script or prompt so it is reproducible. The 0.99 precision gate becomes a
  `cargo test` that loads the gold set.

### Phase A notes

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
