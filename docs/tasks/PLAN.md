# Backend implementation plan and execution order

Status: 2026-10-16. Execution order for the loop described in
[HANDOFF.md](HANDOFF.md). The five handoff decisions are folded in below.
Scope for the near term is **deterministic extraction only** (decision 3):
text, metadata, abstract, bibliography/references, tables, figures, equations.
Model-backed synthesis (summary, section map, glossary, context) is deferred
until the deterministic ground is correct.

## Decisions applied

| # | Decision | Effect on the order |
| --- | --- | --- |
| 1 | Commit the current working tree as the base. | Stage 0 below; every later stage branches from it. |
| 2 | Cheap-model bibliography verification is on by default; on disagreement insert a higher-confidence claim. | Stage 8, after the deterministic bibliography is measured. `bib-model-v2` claim inside the citing-bibliography tier. |
| 3 | Backfill the dev DB; dump all artifacts; only rebuild deterministic artifacts and check them against ground truth; defer model synthesis. | Stage 1 harness, then extraction stages. Backfill procedure below. `summary`, `structure`, `context`, `glossary` stay off. |
| 4 | `summary_only` trigger and model. | Deferred, decoupled in Stage 11/12. |
| 5 | 06 cheapest-tier routing and acceptable acceptance drop. | Deferred. |

## Sequencing rationale

1. **Measure before fixing.** There is no extraction report today, so loss is
   invisible. The audit harness (Stage 1) must exist before any fidelity work,
   or the loop cannot tell a fix from a regression.
2. **Substrate first.** Metadata, abstract, bibliography, figures and equations
   all read the reading index. Fix the index text, then each artifact.
3. **Deterministic before model.** Every artifact gets a measured deterministic
   baseline and an explicit status (`clean | degraded | unreadable`) before a
   model may propose a change. Model verification (Stage 8) and repair (Stage 9)
   are gated on that baseline.
4. **Grounding is the admission gate.** No stage changes source text without a
   validator; a model output that is not a verbatim span is flagged, not used.
5. **No silent drops.** Every stage emits counts for what it dropped or could
   not read, and those counts reach the user-visible extraction report.
6. **Deterministic external lookups are still deterministic.** OpenAlex,
   Crossref, arXiv, Unpaywall and front-matter fallbacks may run in the
   deterministic pass; they are not synthesis models.

## Stage map

| Stage | Track | Deliverable | Depends on |
| --- | --- | --- | --- |
| 0 | base | Current tree committed and gate-clean | — |
| 1 | audit | Ground-truth corpus + per-paper extraction report | 0 |
| 2 | index | Reading-index text fidelity + extraction status | 1 |
| 3 | meta | Printed title/subtitle/authors/year, fallback chain | 2 |
| 4 | abstract | Abstract locate/verify baseline | 2 |
| 5 | bib | Bibliography region, entries, fields (deterministic) | 2 |
| 6 | floats | Figures and tables | 2 |
| 7 | equations | Equation detection and numbering | 2 |
| 8 | bib-model | Cheap-model bibliography verification, default on | 5, 1 |
| 9 | repair | Bounded token-anchored repair agent | 2–7 |
| 10 | identity | Identifier status, hydration accounting, backfill | 3, 5 |
| 11 | registries | Pluggable registry contract | 0 (parallel) |
| 12 | deferred | Summary-only records; analysis agent harness | 10, 11 |

Stages 3–7 are independent of each other once Stage 2 lands and may run in
parallel; the order above is the recommended serial order.

---

## Stage 0 — Commit the base

**Objective.** Freeze the current working tree so every later change is a
reviewable diff.

**Work.**
- Commit the uncommitted tree as the loop base. Suggested grouping, in order:
  1. `docs/agent-harnesses.md`, `docs/tasks/` — reference material.
  2. `src/analysis/spend.rs` + the provider/CLI/API/UI defaults — OpenRouter
     provider and spend ledger.
  3. `web/` reader-tab + histogram changes.
  4. remaining `src/` and `scripts/` edits.
- Confirm the base is gate-clean before the first phase branch.

**Acceptance.** `git status` clean; all three gate commands pass at the base
commit.

**Gate.**
```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd web && npm run typecheck && npm run lint && npm run build)
```

---

## Stage 1 — Ground-truth corpus and audit harness (01.1)

**Objective.** Produce the real loss inventory from evidence, deterministically.
This is the first job listed in [01](01-text-extraction-fidelity.md) and the
prerequisite for every later stage.

**Work.**
- **Corpus.** Three ground-truth sources, all model-free:
  1. Synthetic pages with known text (split letters, hyphen joins, ligatures,
     soft hyphens, two-column order, rotated text).
  2. LaTeX-compiled papers whose `.tex` source is the truth. `scripts/truth/latex`
     currently holds only stale `__pycache__` for `parser`, `tex`, `align`,
     `annotations`, `archive`, `native_exports`; restore or rewrite that tooling
     so `.tex` aligns to extracted text, entries, floats and equations.
  3. A frozen sample of the local library (133 PDFs) with manual adjudication
     for the hostile cases already known: the 53-entry body-as-bibliography
     record, the mid-title split, the "et al." boundary.
- **Runner.** A dev command (for example `lysilogy audit --paper <id>`) plus a
  corpus manifest. It runs the deterministic pipeline only and emits per paper:
  token recall/precision, dropped/merged/split paragraph counts, missed
  headings, abstract boundary, entry count and field correctness, figure/table
  count and labels, equation count, front-matter fields.
- **Aggregate.** A Markdown/JSON report keyed by loss class, with counts and
  locations. This is the artifact every later stage updates.

**Files.** `scripts/truth/` (restore), a new `src/audit.rs` or `audit`
subcommand, corpus manifest and fixtures under `tests/fixtures/` or
`experiments/`.

**Acceptance.** The report runs end to end on the full corpus and lists every
loss class with counts. No class is missing an owner.

**Gate.** Deterministic only; no model calls. Workspace gates.

---

## Stage 2 — Reading-index text fidelity and extraction status (01.2)

**Objective.** The reading index faithfully represents the page, and a bad
index is visible instead of silently trusted.

**Work.**
- Repair token text: split letters (`W ord`), line-end hyphen misjoins,
  ligatures, soft hyphens, wrong reading order in columns/rotated text.
  Touch points: `src/layout.rs`, `src/source_index/native.rs`,
  `src/source_index/paragraphs.rs`.
- Improve page reconstruction: body/float interleaving, tables/captions spliced
  into prose, footnotes, dropped pages. Touch points: `src/extract.rs`,
  `src/source_index/paragraphs.rs`, `src/source_index/ocr.rs`.
- Add deterministic mangle signals: empty/sparse pages, low OCR confidence,
  abnormal token geometry, paragraph/page ratios, unmatched bibliography,
  reference heading not found, missing printed metadata on page 1.
- Persist a per-paper/per-version extraction status `clean | degraded |
  unreadable` with reason codes. Add a migration beside
  `crates/lysilogy-db/migrations/` and write it from `source_index/cache.rs`.
- Surface the status in the reader and graph. A degraded paper must not render
  identically to a clean one.

**Acceptance.** Every loss class from Stage 1 is fixed, detected, or declared
unrecoverable. Rerunning on an unchanged PDF is a no-op; a changed PDF
invalidates derived artifacts. Degraded/unreadable are visible in the UI.

**Gate.** Audit report before/after with measured loss per class; workspace and
web gates.

---

## Stage 3 — Metadata completeness (01.4)

**Objective.** A printed title, subtitle, year and every listed author is
extracted; an untitled Work is never shown when the title is printed.

**Work.**
- Fix `crates/lysilogy-ingest/src/front_matter.rs`: multi-column author grid
  order, dropped subtitles, printed year.
- Fallback chain: deterministic front matter → bounded deterministic read of the
  first pages → external registry by DOI/arXiv/OpenAlex → explicit
  `title_unresolvable`. No untitled-with-no-reason state.
- Keep each field as a fallible claim; the selected value comes from the
  existing trust tiers.

**Acceptance.** On the audit corpus, every Work has a title or an explicit
reason. The 8 bibliography-derived untitled stubs
([02](02-bibliography-integrity.md)) are titled or parked. The audit report
reports metadata completeness per paper.

**Gate.** Audit report; workspace gates.

---

## Stage 4 — Abstract baseline (01 abstract row)

**Objective.** The authored abstract is located and bounded deterministically;
the model boundary review stays off unless the index is degraded.

**Work.**
- Strengthen `src/abstracts/locate.rs` for wrong boundaries, body leakage and
  unlabeled abstracts.
- Keep `verify.rs` admission rules. The `admit_boundary_review` model path
  remains an opt-in for degraded indexes only.
- Add abstract correctness to the audit report.

**Acceptance.** Audit corpus abstracts match ground truth or carry an explicit
`start_boundary_unconfirmed` / `end_boundary_unconfirmed` / not-found state.

**Gate.** Audit report; workspace gates.

---

## Stage 5 — Bibliography, deterministic half (02.1–02.3)

**Objective.** The reference list is found, split and parsed correctly on
normal and hostile layouts, with the raw entry kept as immutable evidence.

**Work.**
- **Region detection** (`crates/lysilogy-ingest/src/bibliography/entries.rs`,
  `source.rs`): recognize `References`, `Bibliography`, `Works Cited`,
  `Literature`, non-English headings and unlabeled lists by layout (last pages,
  hanging indents, year density). Never fall back to "the rest of the document";
  record `bibliography_not_found` and route to repair.
- **Entry splitting:** continuation lines, column boundaries, wrapped titles,
  page breaks, hyphenation; author-year and numbered markers as the primary
  signal.
- **Field parsing** (`fields.rs`): fix the `sentence_boundary` "et al." bug,
  initials, multi-word venues; title-vs-venue by position and italic/bold
  evidence, never a pure venue as a title.
- **No silent drops:** `no_bibliography_entries`, `entry_not_stored`,
  `entry_fields_unreadable` reach the extraction report.
- Add every hostile paper to the audit corpus.

**Acceptance.** Every entry has a title or an explicit reason; no entry contains
body text beyond a bounded length. The 8 mis-split stubs are retitled or parked.
Reruns are idempotent; a changed index text invalidates the extraction.

**Gate.** Audit report before/after with entry-field correctness; workspace
gates.

---

## Stage 6 — Figures and tables (01 floats)

**Objective.** Figure and table captions, labels, membership and references are
deterministic and correct.

**Work.**
- Harden `src/source_index/figures.rs` (`label_at`, `identifier`,
  `figure_region`, `attach_references`) and the caption detection in
  `src/source_index/native.rs`.
- Keep source membership in `spans` even when prose heuristics call a cell
  prose.
- Add figure/table counts, labels and reference matches to the audit report.

**Acceptance.** Corpus figures/tables match ground truth labels and memberships;
unlabeled floats carry an explicit candidate/uncertain state.

**Gate.** Audit report; workspace gates.

---

## Stage 7 — Equations

**Objective.** Displayed equations and their numbers are a deterministic
artifact, measured like the other floats.

**Work.**
- Decide the representation first: a new `objects.equation` list on
  `ReadingIndex`, or extend the float model with an `equation` kind. This is a
  schema decision (`SCHEMA_VERSION` bump).
- Detect displayed equations (isolated lines, math glyph density, numbering
  like `(3)`) without OCR/HTML guesswork, and record references like
  "Equation 3".
- Add equations to the audit corpus and report.

**Acceptance.** Corpus equation count and numbering match ground truth;
uncertain detections are an explicit state.

**Gate.** Audit report; workspace gates.

---

## Stage 8 — Bibliography verification, default on (02.4–02.6)

**Objective.** A cheap model checks each entry against its printed text, and a
grounded disagreement becomes a higher-confidence claim.

**Work.**
- For every entry, a cheap model (`deepseek/deepseek-v4.1-flash` by default)
  sees the printed entry and answers whether parsed title, authors, year and
  venue are correct, quoting verbatim on disagreement.
- **On disagreement, insert a new `bib-model-v2` claim** inside the
  citing-bibliography tier at higher confidence than `bib-v1` (decision 2). The
  corrected value is admitted only when it is a verbatim span of the entry;
  otherwise it is flagged, never used.
- Add `venue` handling to `graph.field_claim` if the current CHECK constraint
  blocks it; store model provenance.
- Cache per entry with a prompt version; reruns cost nothing.
- Record `funding` and cost per call in the spend ledger
  (`src/analysis/spend.rs`).
- A failure to verify is `unverified`, never a silent discard.

**Acceptance.** The verification pass resolves the known mis-parses. An
`unverified`/`dropped` reason is recorded for the rest. A model cannot
overwrite source text (poisoned-proposal test).

**Gate.** Routing score: grounded acceptance × cost on a fixed entry sample,
not prose. Workspace gates.

---

## Stage 9 — Bounded repair agent (01.3)

**Objective.** A model can repair mangled text without ever inventing or
rewriting the authors' words.

**Work.**
- The model proposes **token-ID-level** edits only: split, join, reorder within
  a line, repair a hyphen/ligature, relabel a heading boundary. No free-text
  rewrite, no new tokens.
- A deterministic validator rejects proposals that add, drop, reorder or
  overlap tokens, or that cannot be tied to the source revision. Rejected or
  uncertain proposals keep the deterministic result and are recorded.
- Applies to paragraphs, headings, reference boundaries and front matter.
- Two variants (raw vs repaired) if the reader must know which artifact an
  anchor points into; this is an open question carried from
  [01](01-text-extraction-fidelity.md).

**Acceptance.** A poisoned proposal cannot change source text. The audit report
shows repaired vs rejected counts. A rerun on unchanged input is a no-op.

**Gate.** Open question 2 from [01](01-text-extraction-fidelity.md) (validation
without ground truth) must be answered before this stage starts.

---

## Stage 10 — Work identity, hydration and backfill (03)

**Objective.** Every Work has a title or an explicit terminal state; stale
identifiers are reconciled or retired; nothing is silently skipped.

**Work.**
- **Identifier state** on `graph.work_identifier`: `active | dead | merged |
  unknown`, `checked_at`, and the merge target when OpenAlex reports one. Add a
  migration.
- **Hydration accounting** in `crates/lysilogy-ingest/src/openalex.rs`
  (`fetch_summaries_cached`) and `ingest.rs` (`hydrate_stub_works`):
  `fetched | provider_omitted | not_attempted` per identifier. Retry
  `not_attempted`; treat `provider_omitted` as a state. Record the 352 dead
  OpenAlex IDs as `dead`, not as unknown.
- **Fallback title resolution:** dead identifier → attached DOI/arXiv → the
  citing paper's bibliography entry text (deterministic) → `title_unresolvable`
  with a reason.
- **Backfill command:** one idempotent, resumable command that walks untitled
  Works and reports counts by outcome.
- **Terminal state UI:** stable short label plus a state badge ("identifier
  retired", "no printed title", "unavailable"). Manual title entry stays in the
  `user` tier.

**Acceptance.** After backfill, zero Works with indefinite `unknown`; the 352
dead IDs are `dead`; the 8 bib-derived stubs are titled or parked. Hydration
summary reports `fetched`, `provider_omitted`, `not_attempted`, `retitled`.
Rerunning `lit` on a paper with dead references is stable and cheap.

**Gate.** Workspace and web gates. Backfill proof is the count of indefinite
`unknown` going to zero.

---

## Stage 11 — Search registries (04, parallel)

**Objective.** Identity resolution is a pluggable contract, and the
deterministic providers are migrated behind it with no regression.

**Work.**
- Registry trait with capabilities, auth, rate limits, timeout and
  deterministic/model-backed flag.
- Wrap OpenAlex, Crossref, arXiv, Unpaywall. Add one institutional catalogue
  adapter (SRU or JSON) with a recorded fixture.
- Orchestration: deterministic registries in parallel, per-registry failures
  isolated, best-candidate merge by identifier then score.
- Model web-search fallback only when all deterministic registries return
  nothing usable; returns sourced candidates or explicit `not_found`.
- User registry manifest under the config directory plus CLI/API.

**Acceptance.** Documented contract with fixture tests; existing calls migrated
with no behavior regression; one catalogue adapter works against a fixture.

**Gate.** Workspace gates; existing provider tests unchanged.

---

## Stage 12 — Deferred (05, 06)

Blocked on decisions 4 and 5.

- **05 summary-only:** needs the availability state, the grounded summary
  artifact, and a trigger answer.
- **06 analysis agent harness:** rebuild stage harnesses one at a time, starting
  with bibliography verification (already exercised in Stage 8). The cheapest-
  tier routing and acceptable grounded-acceptance drop are decision 5. The
  `summary`, `structure`, `context` and `glossary` stages wait until the
  deterministic extraction above is correct (decision 3).

---

## Backfill procedure (decision 3)

The dev database is disposable; treat it as such.

1. **Dump and reset.** Record a dump first, then drop the `graph`, `source` and
   `reader` schemas and re-run migrations. Model-derived rows (summary, section
   map, glossary, context) are not migrated.
2. **Re-ingest deterministically only.** For the local library, run the
   deterministic path: reading index, front matter, abstract (locate/verify),
   bibliography, floats, equations, identifiers. Do **not** run `analyze`,
   `refresh-structure`, `refresh-context` or any synthesis stage. Add an
   explicit flag (for example `--extract-only`) if the current `lit`/`ingest`
   path cannot skip the model stages cleanly.
3. **Check against ground truth.** Run the Stage 1 audit report over the rebuilt
   database. The acceptance target is the extraction report, not the graph.
4. **Enable Stage 8 verification** only after the deterministic bibliography
   passes its baseline; it is default-on from then.

Full pipeline rebuilds (model synthesis) are explicitly out of scope until the
deterministic artifacts are correct.

## Remaining open questions

Carried forward, to answer when the stage starts:

1. **Stage 7:** equation representation and `SCHEMA_VERSION` bump.
2. **Stage 9:** how to validate a repair with no ground truth (cross-model
   agreement, raster round-trip, or unchanged-token checksum).
3. **Stage 9:** one extraction variant or raw + repaired.
4. **Stage 10:** store and auto-rewrite the OpenAlex merge target, or only
   record it; and whether `title_unresolvable` Works are excluded from
   prominence.
5. **Stage 11:** which catalogue protocol first; model-fallback trigger and
   budget; whether a library catalogue is a new origin or `library_metadata`.
6. **Decisions 4 and 5** from the handoff (summary trigger/model; cheapest-tier
   routing and acceptable drop).
