# Implementation status and architecture

[Back to guide](README.md)

This report describes the KB work through main commit
`4972d63f10efdc2a89d01901b1adc0624408e868`. The last implementation merge is
[#131 / PR133](https://github.com/tjmisko/Lysilogy/pull/133), merge
`6059dcfc0ec7d7c13df3a535c4d96f1f63028da7`. It is a documentation inventory;
the historical checks cited below were not rerun for this report.

**The complete knowledge base is not delivered.** Phase A, Wave A2 is unfinished.
The useful results so far are persistent paper identity, object artifacts and improved
figure/table extraction, KB domain/parser foundations, provider request controls,
evaluation infrastructure, a downloaded and indexed research corpus, and limited
independently reviewed truth sets. SQLite, bibliography parsing and provider truth
builders exist on separate branches. Most user-facing KB features remain planned.

The [execution plan](../../knowledge-base-phases.md) remains the progress authority.
The [design](../../knowledge-base-plan.md) defines required behavior and acceptance.
The [architecture](../../architecture.md) describes the existing reader. Existing PDF
reading, highlighting, model analysis and citation-provider code predate much of this
KB task; their presence must not be counted as completion of the new KB system.

## How the pieces relate

The reader starts with a PDF and retains extracted text, geometry and per-paper
artifacts. An object is a specific figure, table, reference or other item anchored
to that text. A Work is a cross-paper bibliographic identity; several downloaded
versions or references may ultimately point to one Work. A Person identifies an
author across differently printed names. Creating those cross-paper identities
requires the future resolution engine, rather than simply normalizing strings.

The intended flow is:

```text
PDF → extracted text/geometry → paper objects → bibliographic observations
                                                 ↓
                                    resolver + canonical decisions
                                                 ↓
                                      SQLite query projection
                                                 ↓
                              Work/Person pages, lists, graph, fetch
```

The first part exists on main. Bibliography is branch-only; resolution and the
product surfaces are outstanding. SQLite is designed to be rebuilt from canonical
artifacts and decisions. Evaluation truth is a separate input used to judge these
implementations; successfully publishing a truth set does not implement a detector.

## Merged and usable on main

“Usable” below means implemented with the cited retained validation. It does not
mean the complete KB scenario has passed or that a small evaluation cohort proves
general performance across all papers.

| Capability | What works | Limits and evidence |
| --- | --- | --- |
| KB types — #34 | Validated opaque Work/Person IDs, typed entities and observations, deterministic identifier serialization, cycle-safe alias projection, explicit anchor types. | No persistent KB or resolution engine on main. See [types](../../../src/kb/types.rs) and [module exports](../../../src/kb/mod.rs). |
| Paper object artifact/API — #24 | Generation-bound `objects.json`, figures/tables, caption anchors, separate body membership and mentions; `GET /api/papers/{id}/objects` reuses reading-index work. | The enum also names equations, statements, proofs, algorithms and bibliography; these variants do not imply implemented detectors. Enrichment remains separate and unimplemented. See [objects](../../../src/objects/mod.rs), [API](../../../src/api/objects.rs). |
| Stable paper identity — #20 | Content hashes preserve registered identity across unique moves, retain original note keys/tombstones, and avoid overwriting ambiguous duplicates. Source stamps reject changes during extraction. | Cannot infer a move that predates the first registry scan. Ambiguous content matches remain separate. The identity registry has its own durable publication and locking. See [library](../../../src/library.rs) and the [merged implementation record](../../knowledge-base-phases.md#phase-a-notes). |
| Name parsing — #35 | Preserves raw names and explicit alternative interpretations; derives family/initial candidate keys without choosing ethnicity, name order or identity. | Candidate overlap is not Person identity. Unsupported syntax/limits produce unresolved results. [Code](../../../src/kb/names.rs), [report and retained tests](../../experiment-reports/2026-09-13-person-name-parser.md). |
| Title normalization — #36 | Presentation normalization and bounded character-trigram candidate similarity, retaining distinctions such as negation, math operators and argument structure. | A similarity of one does not prove identity. FTS5 integration is on #33; resolver metrics are unavailable. [Code](../../../src/kb/titles.rs), [report](../../experiment-reports/2026-09-13-title-normalizer.md). |
| Provider cache/rate budgets — #63, #83 | Shared sanitized cache, persistent admission state, request spacing/window limits, server cooldown handling and explicit lease unlocking. | The zero-violation result is a 10,000-reference offline simulation, not live provider throughput. Provider access and genuine truth remain separate blockers. [Cache](../../../src/citation_graph/cache.rs), [budgets](../../../src/citation_graph/budget.rs), [measurement](../../experiment-reports/2026-09-12-provider-budgets.md), [lock fix](../../experiment-reports/2026-09-12-provider-lease-unlock.md). |
| Evaluation harness — #68, #79 | Owned metric collectors, content/source-bound evidence, hard gates, objective reporting, historical ratchets and isolated offline G5 execution. Cold linker setup was corrected. | A suite can pass checks for available measurements while other metrics remain unavailable. Final acceptance requires completeness. [Guide](../../../eval/README.md), [implementation](../../../src/eval/mod.rs). |
| Synthetic scale benchmark — #19 | Generated and exercised a real 10,000-PDF fixture; retained extraction, scan and browser measurements. | The scan/render/search targets were missed. Four-worker extraction was experimental benchmark code, not production batch ingest. [Instructions](../../../scripts/bench/README.md), [full report](../../experiment-reports/2026-09-12-e0.1-d2019ea2.md). |
| Corpus tooling and acquisition — #69, #85, #88, #91 | Fixed selections, quota-preserving availability evidence, explicit proxy transport, versioned canonical source downloads, hashes, receipts and resumable verification. All selected artifacts were downloaded. | Only external corpus storage contains PDFs/sources. This does not implement user-facing KB acquisition or pass production scale acceptance. [Tool guide](../../../scripts/corpus/README.md), [completion report](../../experiment-reports/2026-09-13-kb-corpus-complete.md). |
| Reading-index failure isolation — #98, #118 | Bad native geometry on a page no longer discards every readable page; fallback provenance stays explicit. Bounded diagnostic drainage retains usable native output when stderr is large. | No fabricated native geometry or automatic truth admission. Original failures and recoveries remain recorded. [Page failure report](../../experiment-reports/2026-09-13-kb-reading-page-failure.md), [diagnostics report](../../experiment-reports/2026-09-13-kb-native-diagnostics.md). |

### Object detection corrections

The original limited figure/table measurement was weak: missed Roman-numbered
tables, a prose reference misread as a caption, and excessively large body regions.
The following measured changes now ship:

- **#101:** Roman tables, caption/prose distinction and tighter full-body regions.
- **#106:** missing regions and duplicate captions on the expanded cohort.
- **#111:** bounded mask/opacity evidence; uncertain masks remain uncertain.
- **#121:** observed table grids constrain neighboring figure labels.
- **#125:** split table captions; an interleaved Figure 3 remains unresolved.
- **#123:** bounded painted vector components improve a previously weak plot region.

The implementation is in [figure detection](../../../src/source_index/figures.rs)
and [graphics evidence](../../../src/source_index/graphics.rs). The latest merged
[vector report](../../experiment-reports/2026-09-14-kb-vector-regions.md) records
49 true positives, zero false positives and one false negative across 50 visual
objects: O1 = 0.98989898989899 and median O2 = 0.8910021250829322. Every truth
object stays in the denominator, including two zero-overlap regions. Unsupported
graphics traces are still reported. These values meet their unchanged targets on
that limited cohort; they are not 10k-corpus quality estimates.

### Truth and replay work

Merged #70 introduced bounded LaTeX-derived inventories/alignment and independent
manual construction. Subsequent work preserved ambiguous duplicate source objects
(#107), corrected literal BibTeX field syntax (#109), and published reviewed
manual/visual cohorts (#105, #115, #117). Each release retains its original inputs,
selection limits, implementation and unavailable metric roles.

The #109 correction was not an unqualified coverage improvement: it recovered five
of eight original failures but retained 684 of 702 prior successes, a net loss of
13 source parses in that fixed 710-paper comparison. The
[BibTeX report](../../experiment-reports/2026-09-14-kb-bibtex-fields.md) preserves
those failures. Later full-population results use their own fixed input scope.

[#129](../../experiment-reports/2026-09-14-kb-bounded-k1-replay.md) added bounded,
serial per-paper publication/replay and collector workers, one metric owner and
explicit child-evidence closure. Its generated 500-row test measures infrastructure;
it is not 500 independently labeled real papers. Main's latest published release,
**V5**, adds #131's complete numbered-equation cohorts: 11 papers overall,
60 truth equations and 50 visual regions. It preserves previous outcomes. See the
[numbered-math report](../../experiment-reports/2026-09-14-kb-numbered-math-tranche.md)
and [truth tool guide](../../../scripts/truth/latex/README.md).

The main unresolved research problem is automatic truth admission: the latest
retained full run parsed 724 of 1,000 frozen papers, failed 276, and admitted zero
automatic papers or metric cohorts. Parsing source text is insufficient to prove
complete object inventory, semantics and original-PDF alignment. Manual releases
provide small, defensible denominators but do not satisfy #97's roughly 500-paper
stratified target.

The #110 reconstruction experiment did not solve this. One of ten source builds
completed; none reproduced every original page at both tested resolutions. Missing
packages explain nine build failures but do not establish reproducible layout or
truth correctness. No labels were admitted from that experiment. See the
[source-layout report](../../experiment-reports/2026-09-13-kb-source-layout-probe.md).

## Implemented only on branches

Branch state is not installed main behavior. The full worktree inventory is
maintained alongside this report; the exact commits below identify the code to
which these claims apply. Draft PR descriptions contain historical checkpoints and
may still name older main integrations or superseded approval status.

### #33 — SQLite store, migrations and rebuild

Branch `feat/e2.1-kb-store`, head
`905c3410cf22cb74c26aa652b2bb544cdea07c1a`; **no PR**.

The branch contains store/migration/rebuild implementation, canonical journal and
checkpoint handling, and regression source. Independent review corrections include
journal-prefix integrity, version/local-copy handling and rejection of versioned
arXiv identifiers at Work-level admission while allowing them on WorkVersion.

Formatting and direct Python SQLite/migration checks passed. Seven rebuild-adapter
tests passed with #71's loader supplied read-only. **Rust compilation, Clippy,
all-target tests, G5, actual G4 rebuild determinism and O28's 500k-Work/3M-edge
measurement are unrun.** Missing registry dependencies blocked the real build.
Those lighter checks cannot substitute for compiling the application.

There is also an unresolved reader-startup failure path described below. Resume
from committed source and the current branch report, not its historical unfinished
patch. The retained `target/kb-resume-notes.md` explicitly says that patch has been
superseded. Preserve the branch's unrelated/uncommitted `.gitignore` change.

### #25 — bibliography extraction and link hints

Branch `feat/e1.2-bibliography`, head
`201c53c7a7041f7740b68e7495e4b93358f8639b`;
[draft PR94](https://github.com/tjmisko/Lysilogy/pull/94).

Implemented: backend entry segmentation and fields; disjoint UTF-16 source
membership; numeric lists/ranges, author-year suffixes and geometry-confirmed
superscript mentions; unresolved ambiguity; frontend generation checks and fallbacks.
It integrates bibliography with the figure/graphics object factory and uses schema 2.

Retained validation includes 388 Rust tests, bibliography/object collectors,
frontend checks, paper-link tests and a production-artifact Playwright smoke.
The limited K1 cohort has 35 entries and 50 occurrence-target pairs: O8 and both
O10 components reached 1.0; title fields are 30/35 and author/year fields 35/35.
Five title misses remain tracked in #103.

**Genuine K2 data and full O9 are missing, so this detector must remain draft.**
Its branch scorecard and older 15-region visual measurements are not the current
main scorecard. It also needs integration with the later main changes before merge.
See the [committed branch report](https://github.com/tjmisko/Lysilogy/blob/201c53c7a7041f7740b68e7495e4b93358f8639b/docs/experiment-reports/2026-09-13-bibliography-parser.md).

### #71 — reference, acquisition and read-next truth

Branch `feat/e8.4-reference-truth`, head
`d5e12aa748b7c19a833989799c565c3309d01d65`;
[draft PR93](https://github.com/tjmisko/Lysilogy/pull/93).

Implemented tooling freezes Crossref/OpenAlex responses through the existing
budget/cache service, separates deposited inputs from expected answers, prepares
the 200-reference acquisition sample, and removes held-out outgoing edges from
every ranking view for read-next evaluation. Offline source/receipt, sampling and
leakage tests exist; retained G5 passed 309 Rust / 120 Python / 85 Node tests.

**Genuine K2/K5 provider snapshots and mapped K7 graph truth are absent.** Provider
host policy blocked the live attempt before a response. The user has since approved
the required hosts; effective access still needs verification after configuration
repair. K7 additionally needs #25 and the real mapped scale/local graph. Fixture
tests and metadata plans are not downloaded provider truth.

### #72 — person silver labels

Branch `feat/e8.5-person-labels`, head
`c867ec694d3c5c87eecb262012b2d3677a601037`;
[draft PR95](https://github.com/tjmisko/Lysilogy/pull/95), **based on #93's branch**.

Implemented tooling validates source-deposited per-authorship ORCIDs, preserves
positional mention identity, reports conflicts/missing coverage, and excludes label
and provider-identity leakage from resolver features. Profile-only ORCIDs do not
become independent truth. Retained G5 passed 309 Rust / 165 Python / 85 Node tests.

An offline metadata pass found 1,803 valid unique DOIs and prepared 73 bounded,
unexecuted requests. **No genuine K4 person labels exist.** This requires the shared
K2/K0 universe, real OpenAlex evidence and provider access. Merge #93's dependency
deliberately; PR95 is not an independent branch from main.

### #132 — formal statement/proof truth

Branch `feat/e8.3-formal-tranche`, head
`b4ff5b99ddbf29abec94947eb9cb54c3d21d32fb`; **no PR; paused before final verification**.

The branch constructs V6 with exactly seven statements and four proofs from
2310.01528v1, keeping Lemma 2's three associated parts distinct from counted
statements. Its total is 12 papers, 60 O3 equations, 50 visuals, 11 O5 statements
and five O6 proofs. Other roles remain explicitly unscored; prior V5 records remain
exact. This is truth construction, not statement/proof detection.

Source, construction, publication and corrected V1–V6 replay received separate
reviews. Integrated gates before the last loader correction passed 428 Rust /
645 Python / 85 Node tests. The correction has targeted independent validation;
do not report the older full gate receipt as a fresh check of the corrected head.

The first actual collector failed because its anonymous module loader did not
populate `sys.modules`; isolated replay tests had missed that entry point. The
branch now passes an explicit pinned-release context and tests the actual loader.
The corrected collector completed in 86.598899 seconds with 12 decisions, nine
visual prediction papers and all 50 visual outcomes unchanged.

**Still required:** independent audit of that collector, historical-outcome
comparison, the owning CLI after-check, latest-main integration, final gates/report,
draft PR, separate final review and merge. V6 must not be described as merged main
truth. #134 and #135 are prepared follow-on proposals with issue authorization, but
neither has a worktree or implementation yet.

## Remaining planned product work

All issues below remain open. The phase and wave assignments are the current plan,
not a newly adopted schedule. The existing reader may contain related behaviors,
but the KB-specific deliverables and acceptance for these issues are outstanding.

| Phase / wave | Outstanding work | What is missing for the user |
| --- | --- | --- |
| A2 | #25, #33, #71, #72 branches above; #97 broad K1; #103 bibliography title fixes; #132/#134/#135 truth extensions | Complete tested storage and independently measured input foundations. |
| A3 | #37 resolution gold set | Reviewed positive/negative name/title pairs and real resolution evaluation. |
| B1 | #26 equations, #27 statements/proofs, #28 algorithms, #30 enrichment, #31 Figures tab | Production detection for remaining object kinds, ranked/quoted object context and the new object reader UI. |
| B1 | #38 resolution, #39 decision log | Conservative cross-paper matching and persistent merge/split decisions. |
| B1 | #21 incremental scan, #22 virtualized home/search, #23 bounded batch ingest | Production fixes for measured 10k scan/UI misses and separate global extraction/model scheduling. |
| B2 | #29 object link hints, #40 local ingest, #41 provider ingest | New object navigation and population of the KB from actual observations. |
| B3 | #42 KB API | Query/edit endpoints supporting the new KB product surfaces. |
| C1 | #44 Work/Person routes, #45 acquisition jobs, #52 citation formats, #55 list model, #59 graph metrics, #73 list evaluation | Entity pages and the foundations for acquisition, exports, lists and neighborhoods. |
| C2–C4 | #46 identifier resolution, #47 OA locators, #48 agent fallback, #49 PDF verification, #50 download policy, #51 fetch UI, #56 list view | Verified reference-to-PDF acquisition and usable reading lists. |
| D1 | #32 clickable proofs, #43 review queue, #53 styled citations, #54 copy/export, #57 AI lists, #60 graph view, #64 recommendations, #65 author identity | Review/refinement workflows and advanced KB views/provider features. |
| D2 | #61 graph keys, #62 communities/read-next, #58 list graph | Complete graph exploration and evaluated next-reading suggestions. |

No Phase A exit report or completed system report is justified yet. Production
10k behavior, the complete Playwright scenario, all five hard gates and at least
24 of 30 objectives remain acceptance work, with explicit follow-ups for any
permitted objective misses.

## Architecture risks and proposed corrections

These are code-review findings, not observed data-loss incidents or proof that the
chosen Rust/React/SQLite design is unsuitable. They remain unresolved unless
explicitly identified as fixed below.

### A failed KB open can stop the reader on #33

The branch initializes SQLite while constructing `AppState`:

```rust
let kb = tokio::task::spawn_blocking(move || KbStore::open(kb_root))
    .await
    .map_err(|error| Error::Task(error.to_string()))??;
```

Both task failure and database-open/migration failure propagate out of startup.
The PDF reader then cannot start even if its per-paper artifacts are usable.
Introduce an explicit unavailable-KB state, return a clear failure for KB-dependent
operations, and keep independent reading functionality available. Add failure-path
tests. This affects the unmerged branch's
[startup code](https://github.com/tjmisko/Lysilogy/blob/905c3410cf22cb74c26aa652b2bb544cdea07c1a/src/api.rs#L142).

### Atomic rename alone is not crash durability

The generic [artifact writer](../../../src/store.rs) performs:

```rust
fs::write(&temporary, bytes).await?;
fs::rename(&temporary, path).await?;
```

Readers avoid a half-written destination, but this path does not explicitly sync
the new file and containing directory before reporting durable success. A crash
or power loss can therefore leave persistence weaker than the canonical-state
contract requires. Add durable writes and interrupted-update recovery tests where
the data is canonical, distinguishing it from disposable caches. The content
identity registry already has separate durable handling; this is not a claim that
every writer has the same defect. No data loss was observed.

### Per-paper reservations do not bound total model work

[API analysis scheduling](../../../src/api.rs) spawns work for each accepted paper;
[local analysis](../../../src/analysis/local_cli.rs) starts orientation, structure
and external-context stages concurrently within a paper. A per-paper duplicate-job
guard prevents two jobs for that paper, but does not impose an application-wide
model budget. Multiple papers can multiply subprocesses and memory demand.

Implement #23's separate bounded extraction and model queues before large batch
queueing. The one-heavy-job rule used during development is not an application
runtime limit. Benchmark-only four-worker extraction does not address this risk.

### Truth releases still require too much paper-specific routing

Bounded replay is implemented, but annotation profiles and dispatch still encode
specific cohorts, including [numbered-math profiles](../../../scripts/truth/latex/tranche_numbered_math.py).
Adding a paper can require code, a new retained producer bundle and repeated
construction/integration review. A stable per-paper evidence format and worker
contract should let routine additions become reviewed data, with adapters reserved
for new semantics. Preserve old bundles and complete denominators. This is an
engineering proposal, not permission to bypass source uncertainty or lower truth
requirements.

### Test the real integration entry point early

#132's collector-loader defect demonstrated a specific test gap: a replay helper
could pass while the actual collector loaded its module differently and failed
before measurement. The loader correction and actual-entry-point regression exist
on #132. Its final runtime audit is still pending. Future integration checks should
exercise artifact loading, dispatch and collection together before extensive
publication work.

## Planning decisions that have not been adopted

The architecture review recommended allowing dependency-ready product issues such
as #21/#22/#23/#31 to proceed while #97 expands truth on a parallel track, retaining
all consumer-specific truth prerequisites and final acceptance. **That phase-barrier
change has not been approved or written into the operative schedule.** Under the
current plan, Phase A is still the controlling phase.

The review also found tension between final acceptance saying “every issue is
closed” and permitting measured misses with open follow-ups. A proposed clarification
would distinguish required implementation/acceptance issues from permitted objective
follow-ups. It has not been adopted and does not defer #97 or lower any target.

Additional host grants for npm, Semantic Scholar and OpenCitations were proposed
but remain unapproved. The four Rust-registry/Crossref/OpenAlex hosts were explicitly
approved; applying and verifying those grants is an environment task, not another
permission decision. Unrelated PDF-preview changes in main remain protected and
will need a deliberate integration plan before overlapping frontend work.
