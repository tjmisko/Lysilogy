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
- [ ] **#70 E8.3 arXiv LaTeX object truth** (after #68, #69, #24). Branch
  `feat/e8.3-latex-truth`. Owns the LaTeX parser and PDF aligner used only for evaluation.
  Brought forward from A3 so #25 can be measured against independent K1 before merging;
  this resolves a sequencing cycle under the user's detector-before-truth merge restriction.
- [ ] **#71 E8.4 Reference, acquisition, and read-next truth** (after #68, #63). Branch
  `feat/e8.4-reference-truth`. K7 needs parsed bibliographies of the scale tier; build the K2 and
  K5 parts first and finish K7 once #25 and the corpus mapping are available.
- [ ] **#72 E8.5 Person silver labels** (after #68, #63). Branch `feat/e8.5-person-labels`.

### Wave A3

- [ ] **#37 E2.5 Resolution gold set and evaluation** (after #25, #69). Branch `feat/e2.5-gold-set`.
  Owns `src/kb/gold/`; reports through the E8.1 `resolution` suite. Mine ~200 name and title pairs
  from parsed bibliographies in `local-articles` and the arXiv corpus, including hard negatives. Store only bibliographic
  metadata, never full text. Label with agents: two independent labeler subagents, then an
  adjudicator subagent for disagreements, each writing rationale to the plain-text gold file.
  Commit the labeling script or prompt so it is reproducible. The 0.99 precision gate becomes a
  `cargo test` that loads the gold set.

### Phase A notes

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
