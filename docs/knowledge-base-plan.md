# Paper objects, a cross-paper knowledge base, and reading into a literature

Status: planned 2026-09-12. Tracked in the GitHub project **Lysilogy: Knowledge Base**. Each
`### E<epic>.<n>` heading below is the design section for exactly one issue; issue bodies are
generated from these sections. Execution order, branches, file ownership, and phase exit criteria
are in [knowledge-base-phases.md](knowledge-base-phases.md).

Lysilogy's first goal is making one paper easy to read. This plan adds the second: making it easy
to be *read into a literature*. That requires two layers of modeling.

1. **Paper-local objects.** Figures, tables, equations, theorem-like statements, proofs,
   algorithms, and bibliography entries are first-class items of a paper, anchored to its source
   geometry and linked to their in-text mentions.
2. **A cross-paper knowledge base.** Works, people, authorships, and citations are resolved across
   papers despite variations in names and titles, so that citation trails can be followed, papers
   fetched, citations generated, reading lists built, and neighborhoods explored as a graph.

## Confirmed decisions

| Question | Decision |
| --- | --- |
| Scale target | 10,000 local PDFs. The KB is sized for roughly 500k Works, 500k Persons, and 1–3M citation edges. |
| KB storage | SQLite (bundled `rusqlite`) as a rebuildable query store. User-authored state is canonical plain text: `kb/decisions.jsonl` and `kb/lists/<id>.json`. Deleting the database costs only a rebuild. |
| Graph database | Not used. Entity resolution is an indexed relational problem; graph views need 1–3 hop neighborhoods, which indexed edge tables and recursive CTEs serve at this scale; graph algorithms run in memory over extracted subgraphs. An embedded graph DB adds a fragile dependency and a JVM server breaks the local single-binary design. |
| Glossary | The Figures tab takes the Glossary's ladder slot and the `g` key. Glossary remains available through `:glossary` and digest links. |
| After a reference PDF downloads | Configurable: `download_only`, `extract_heuristic` (default), or `full_model`. Batches over five papers with `full_model` require an explicit confirmation that states the credit/API/usage cost. |
| Citation manager integration | Export only: CSL-JSON, BibTeX, RIS, and styled text. No Zotero sync. |
| Resolution ground truth | An agent-labeled gold set mined from local bibliographies: two independent model labelers plus an adjudicator for disagreements, with no human labeling gate. Auto-merge precision against it must be at least 0.99; recall is reported, not gated. Humans may audit labels later. |
| Human gates | None during implementation. Agents build, review, and merge; human review happens on the finished system. |
| Evaluation | Metric-driven buildout: hard gates, objectives with ratcheted baselines, and agent-built truth sets. See [Metrics and objectives](#metrics-and-objectives). |
| Research corpus | A separately stored arXiv corpus (PDFs from the public GCS bucket, LaTeX sources from `export.arxiv.org` within its rate policy) feeds truth sets and realistic scale runs. Never committed; never mixed with the user's library. |
| Unresolvable AI reading-list items | Kept, visibly marked unresolved, and excluded from graph edges until resolved. |

Defaults assumed unless revised: the Figures tab enriches the top 3–5 figures/tables; the graph
view uses graphology and sigma.js; agents reuse the existing Codex/Claude provider selection with
Claude as the default for acquisition and reading-list generation; only legitimate open-access
sources are used for downloads.

## Baseline

- [`source_index/figures.rs`](../src/source_index/figures.rs) detects figure and table captions and
  estimates their regions deterministically. `Figure` records live in the cached
  `reading-index.json` with label, caption, page, conservative rectangle, and in-text references.
- No equations, theorem-like statements, proofs, or algorithms are represented.
  [`paperLinks.ts`](../web/src/lib/paperLinks.ts) only uses "Eq." and "Theorem" as negative context
  when matching citations.
- Bibliography entries are split and matched to citation markers only in the frontend. Entries are
  raw strings with no structured authors, title, or year, and nothing is persisted.
- All artifacts are per paper under `.lysilogy/papers/<id>/`. There is no cross-paper store and no
  database. `PaperId` hashes the vault-relative path, so a moved PDF orphans its artifacts.
- [`citation_graph/`](../src/citation_graph/) has OpenAlex, Semantic Scholar, OpenCitations, and
  Crossref adapters producing a per-paper `citation-graph.json`. `Work.authors` are raw strings and
  nothing maps a `Work` to a local paper.
- Reader tools already offer a single-step "Find & fetch" through `AnalysisService::reader_tool`
  (schema-constrained local CLI with web tools) and `import_remote_pdf` (address-pinned, bounded
  download). Jobs in [`jobs.rs`](../src/jobs.rs) are strictly paper-scoped.
- The library catalog is rebuilt in memory on every refresh, the home grid renders every card, and
  batch ingest runs papers sequentially.

## Architecture

```text
per-paper artifacts                           kb/ (under the data root)
─────────────────────                         ─────────────────────────────────────────────
reading-index.json ─► objects.json ──┐        observations (rebuildable) ─┐
  (deterministic)     (+ enrichment) │                                    ├─► resolution ─► entities
extraction.json ─────────────────────┼──────► provider cache ────────────┤        ▲        Work
citation-graph.json ─────────────────┘                                    │        │        Person
                                              decisions.jsonl (canonical)─┘        │        Authorship
                                              lists/<id>.json (canonical) ─────────┘        Citation
                                                                                            ReadingList
                                                         kb.sqlite: all of the above, indexed + FTS5
```

Provenance rules carry over from the reading pipeline. A quoted context passage is admitted only
after an exact source match. A database edge says that A cites B, never why. AI output is labeled
as such. Automatic merges require strong evidence; everything else is a reviewable candidate.

## Knowledge base model

### Layers

| Layer | Storage | Contents | Lifetime |
| --- | --- | --- | --- |
| Observation | SQLite (rebuildable) | A bibliography entry in a local paper, a provider record, PDF metadata, an AI proposal. Each has a source, a retrieval time, and the raw text or payload. | Regenerated from per-paper artifacts and the provider cache |
| Decision | `kb/decisions.jsonl` (canonical) + SQLite mirror | `merge`, `split`, `distinct` (never merge), and automatic merges with their score and matcher version | Append-only, survives rebuilds |
| Entity | SQLite (projection) | Work, Person, Authorship, Citation | Recomputed from observations + decisions |
| Reading list | `kb/lists/<id>.json` (canonical) + SQLite mirror | Ordered items, status, notes, roles, generation provenance | User-owned |

Entity IDs are minted once (`W…`, `P…`) and never recomputed. A merge keeps the surviving ID and
records the absorbed ID as an alias, so every existing link, list item, and URL continues to
resolve. A split mints a new ID and records which observations moved.

### Entities

- **Work**: identifier set (DOI, arXiv ID without version, OpenAlex, Semantic Scholar, PMID), title,
  normalized title key, year, venue, type. **Versions** record preprint and published
  manifestations separately, preserving version-specific identifiers and dates. **Local copies**
  link zero or more PDFs by content hash and current `PaperId`.
- **Person**: display name, parsed family and given names, name variants with counts, external IDs
  (ORCID, OpenAlex author, Semantic Scholar author).
- **Authorship**: Work × Person × position, plus the raw name string as printed.
- **Citation**: citing Work → cited Work with one evidence record per source. Local evidence holds
  the paper, bibliography entry ID, and each in-text citation sentence with its anchor. Provider
  evidence holds the provider, retrieval time, and provider-supplied passages, intents, or
  influence labels.
- **Acquisition state** (per Work): `unresolved → identifier_found → link_found → downloaded →
  mapped`, or `unavailable { reason, checked_at }` at any stage. Each transition records its source
  so progress is incremental and retries skip completed stages.

### Matching

- **Titles** fold Unicode compatibility forms, diacritics, LaTeX commands and braces, punctuation,
  and case into an exact key. Fuzzy candidates come from an FTS5 trigram index and are scored with
  trigram similarity gated by year (±1, wider for preprint/published pairs) and first-author family
  name.
- **Names** parse `Last, First`, `F. M. Last`, `First Middle Last`, particles (`van`, `de la`,
  `von`), suffixes (`Jr.`, `III`), hyphenated given names, initials with and without periods, and
  family-name-first orders. Candidates are blocked on family name + first initial. Automatic person
  merges require a shared external ID, or a shared co-author on an already-merged Work.
- **Works** merge automatically on a shared exact identifier, or on an exact title key plus
  compatible year and first author. Everything below the threshold becomes a review candidate.
- **Bibliography entries** resolve through DOI/arXiv extraction, then Crossref
  `query.bibliographic`, Semantic Scholar `/paper/search/match`, and OpenAlex search, with a model
  parse only as a last resort.

## Metrics and objectives

The buildout is metric-driven. Every measurable behavior has a named metric, a ground-truth
source that agents can construct without human labeling, and a target. `lysilogy eval <suite>`
(E8.1) computes metrics, writes a results record per commit, and regenerates
[`kb-scorecard.md`](kb-scorecard.md).

- **Hard gates (`G`)** protect correctness and provenance. They must pass for a PR to merge and are
  never lowered.
- **Objectives (`O`)** are targets to climb toward, set at roughly the "80% of the way there"
  level. Initial values below are provisional. The first measurement of each metric records a
  baseline; after that a PR touching the metric's area must not regress it by more than one
  percentage point (or 10% relative for latencies) without a recorded justification. An agent may
  revise an objective's target only with evidence, such as a measured truth-set ceiling, recorded
  in this section.
- **Reported (`R`)** metrics are tracked without a target.

Full evaluation suites run from local caches. Network access is used only by explicit corpus-fetch
steps, never by `cargo test`; unit tests use small committed fixtures derived from the suites.

### Ground-truth sources

| ID | Truth set | Construction | Built by |
| --- | --- | --- | --- |
| K0 | arXiv research corpus | A separately stored corpus of arXiv PDFs and LaTeX sources (E8.2): an `eval` tier with sources and a `scale` tier of 10,000 PDFs concentrated in a few dense subfields. It is the raw material for K1, K7, and realistic scale runs. | E8.2 |
| K1 | arXiv object truth | ~500 papers from the K0 `eval` tier, stratified across math, CS, physics, statistics, and economics. Parse `figure`/`table` environments and captions, numbered equation environments and `\label`/`\eqref`, `\newtheorem` environments, `proof` environments and their targets, `algorithm` environments, `\bibitem`/`.bbl` entries, and `\cite` keys. Align to the compiled PDF by caption and text matching. Commit only derived labels and arXiv IDs. | E8.3 |
| K2 | Reference resolution truth | Papers with DOIs whose Crossref records include deposited references with DOIs; each deposited reference is a labeled (entry → identifier) pair. Includes local library papers and K0 papers with published versions. | E8.4 |
| K3 | Entity-pair gold set | ~200 agent-labeled name and title pairs with hard negatives, mined from local and K0 bibliographies. | E2.5 |
| K4 | Person silver labels | OpenAlex authorships carrying ORCIDs for works in K2 and K0; clusters by ORCID. | E8.5 |
| K5 | Acquisition set | 200 references sampled from local and K0 bibliographies with known identifiers from K2, stratified by field and age, with Unpaywall/OpenAlex open-access status recorded at build time. | E8.4 |
| K6 | Reading-list prompts | 10 topic prompts, each paired with a recent survey whose bibliography serves as a soft reference set, plus a three-agent relevance panel rubric. | E8.6 |
| K7 | Read-next holdout | Leave-one-out over the citation graph of the K0 `scale` tier and the local library: hide one paper's references and predict them from the rest. | E8.4 |

### Scorecard

| ID | Metric | Truth | Target | Issues |
| --- | --- | --- | --- | --- |
| G1 | Auto-merge precision for Works and Persons | K3, K4 | ≥ 0.99 | E2.6, E7.3 |
| G2 | Wrong paper linked after download | K5 | 0 | E3.5 |
| G3 | Published enrichment quotes that fail exact source match | All E1.7 outputs | 0 | E1.7 |
| G4 | Rebuild determinism: identical entities, IDs, and aliases across two rebuilds | K2 library | 100% | E2.1, E2.7 |
| G5 | Test suite passes with network access disabled and no model CLIs on `PATH` | `cargo test`, web tests | Pass | E8.1 |
| O1 | Figure and table detection F1 | K1 | ≥ 0.90 | E1.1 |
| O2 | Figure and table region IoU, median | K1 | ≥ 0.75 | E1.1 |
| O3 | Numbered equation detection F1 | K1 | ≥ 0.85 | E1.3 |
| O4 | Equation and statement mention → object link accuracy | K1 | ≥ 0.90 | E1.3, E1.4, E1.6 |
| O5 | Theorem-like statement detection F1 | K1 | ≥ 0.85 | E1.4 |
| O6 | Proof → statement link accuracy | K1 | ≥ 0.85 | E1.4 |
| O7 | Algorithm detection F1 | K1 | ≥ 0.80 | E1.5 |
| O8 | Bibliography entry segmentation F1 | K1 | ≥ 0.95 | E1.2 |
| O9 | Bibliography field accuracy: title / first author / year | K1, K2 | ≥ 0.90 / 0.90 / 0.95 | E1.2 |
| O10 | Citation marker → entry precision / recall | K1 | ≥ 0.97 / 0.85 | E1.2 |
| O11 | Key-figure top-3 agreement with a three-agent panel | K1 | ≥ 0.70 | E1.7 |
| O12 | Reference → identifier precision / recall | K2 | ≥ 0.98 / 0.80 | E2.6, E2.8, E3.2 |
| O13 | Person clustering B-cubed F1 | K4 | ≥ 0.90 | E2.6, E7.3 |
| O14 | Auto-merge recall | K3 | ≥ 0.80 | E2.6 |
| O15 | Duplicate Works remaining after ingest (agent-audited sample) | K2 | ≤ 2% | E2.8, E2.9 |
| O16 | References reaching `identifier_found` | K5 | ≥ 0.85 | E3.2, E3.4 |
| O17 | Open-access references reaching `downloaded` | K5 (OA subset) | ≥ 0.80 | E3.3, E3.5 |
| O18 | Downloaded references reaching `mapped` under `extract_heuristic` | K5 | ≥ 0.95 | E3.6 |
| O19 | Model calls per reference in an acquisition batch | K5 | ≤ 0.30 | E3.4 |
| O20 | Styled citation match with doi.org CSL output after normalization | K2 | ≥ 0.90 | E4.2 |
| O21 | BibTeX and RIS exports that parse without errors | K2 | 100% | E4.1 |
| O22 | AI list proposals resolved / fabricated works | K6 | ≥ 0.90 / ≤ 0.05 | E5.3 |
| O23 | AI list relevance, panel mean on a 5-point rubric | K6 | ≥ 4.0 | E5.3 |
| O24 | Read-next recall@10 | K7 | ≥ 0.30 | E6.4 |
| O25 | No-change rescan of the 10k vault | Synthetic, K0 scale | ≤ 2 s | E0.3 |
| O26 | Home first render / search p95 at 10k papers | Synthetic, K0 scale | ≤ 500 ms / ≤ 150 ms | E0.4 |
| O27 | Batch extraction parallel efficiency at 4 workers | Synthetic, K0 scale | ≥ 0.70 | E0.5 |
| O28 | Two-hop neighborhood query p95 at 500k Works and 3M edges | Synthetic | ≤ 150 ms | E2.1, E6.1 |
| O29 | Graph view frame rate while panning at the 500-node cap | Synthetic | ≥ 30 fps | E6.2 |
| O30 | Provider budget violations in a simulated 10k-reference batch | Recorded fixtures | 0 | E7.1 |
| R1 | Agent-fallback lift over deterministic acquisition | K5 | Reported | E3.4 |
| R2 | Overlap of AI lists with survey bibliographies | K6 | Reported | E5.3 |
| R3 | Inter-labeler agreement in the gold set | K3 | Reported | E2.5 |
| R4 | Cost and wall time per paper for enrichment, per reference for acquisition, per AI list | All | Reported | E1.7, E3.4, E5.3 |

---

## E0 Library at scale

Goal: the library, catalog, and ingest paths remain responsive and correct with 10,000 local PDFs,
and paper identity survives moves and renames.

### E0.1 Synthetic 10k vault and scale benchmarks

Build a generator for a synthetic vault of 10,000 small PDFs with varied filenames, nesting, and
metadata, plus a benchmark command that measures catalog scan, home page load, search, and batch
extraction throughput. Record results in `docs/experiment-reports/`.

Acceptance: benchmarks run locally without the real corpus; generated PDFs stay out of the
repository and out of `/tmp`; results establish the baseline used by E0.3–E0.5.

Tests: should generate a deterministic vault when given the same seed; should report per-phase
timings when the benchmark completes.

Implementation contract: `python3 scripts/bench/run.py` uses release builds and the designated
external synthetic vault. It separates discovered-only scans, experimental one/four-worker
extractor capacity, full-vault extraction/persistence setup, and populated no-change scans. O25
uses the populated 10k case; O26 uses real Playwright navigation/input-to-paint observations on
the app serving those artifacts. The independent `synthetic-vault` scale collector publishes only
verified full-size runs. O27 remains unavailable until E0.5 measures the production worker path;
benchmark-only parallel scheduling is reported as capacity without changing production ingest.
See [benchmark instructions](../scripts/bench/README.md). Tiny fixture runs prove orchestration,
not scale performance. Required external storage being read-only leaves the full baseline pending.

Blocked by: none

### E0.2 Content-hash paper identity

Record a SHA-256 content hash for each PDF alongside its path-derived `PaperId`. When a PDF moves or
is renamed, re-associate its existing artifacts instead of orphaning them. The KB links local
copies by content hash.

Implementation decision: `paper-identities.json` is canonical state under the data root, bound to
the canonical library root. A `PaperId` is initially path-derived and is retained on a unique
hash-proven move; artifact directories never move or merge. Missing identities remain as
tombstones. Replaced content and ambiguous duplicate/move groups receive separate identities;
the scan API and CLI report duplicate paths and unresolved identity groups. Unresolved prior IDs
are persisted by content hash, so provisional new IDs do not hide conflicts on subsequent scans
or restarts. Reports follow current paths; absent sources do not resolve a conflict. Reusing an unrelated
library with the same data root is rejected. Legacy artifacts at an unchanged path are adopted
on the first identity scan; moves made before any content hash was recorded cannot be inferred.

Notes keep the original relative PDF path as a canonical notes key. A rename changes only the
PDF's current location, so notes continue to open and save through the original key. Replaced
content or reuse of an already-reserved notes name gets a distinct ` [paper-<id>-<n>]` suffix.
No note is read, moved, or rewritten during discovery; existing authored source links inside notes
are preserved verbatim. Registry transactions use a cross-process lock and a synced atomic
replacement, and refresh publication is serialized. Hashes are streamed and cached by size/mtime
with inode/ctime safeguards so atomic replacement with preserved size/mtime is detected. Source
stamps are checked around hashing and extraction. Extraction must finish with exactly its freshly
verified initial stamp: even restored original bytes cannot validate output read during a temporary
change. An unresolved content change requires a rescan.

Acceptance: moving or renaming a mapped PDF preserves analysis, highlights, notes linkage, reader
tools, and objects; duplicate PDFs at two paths are reported rather than silently merged; hashing
is cached by size and modification time so rescans do not rehash unchanged files.

Tests: should reattach artifacts when a mapped PDF is renamed; should reattach artifacts when a PDF
moves between subdirectories; should report both paths when identical PDFs exist twice; should not
rehash when size and mtime are unchanged.

Blocked by: none

### E0.3 Incremental catalog scan

Replace full catalog rebuilds with an incremental scan keyed by path, size, and modification time.
Load saved metadata lazily or from a compact catalog cache rather than opening every
`extraction.json` on refresh.

Acceptance: a no-change rescan of the 10k synthetic vault completes within the target set by E0.1;
additions, deletions, and moves are reflected after a rescan.

Tests: should detect an added PDF when rescanning; should drop a deleted PDF when rescanning;
should reuse cached metadata when files are unchanged.

Blocked by: E0.1

### E0.4 Home grid virtualization and server-side search

Virtualize the home grid and move filtering, sorting, and search to paginated API endpoints.
Keyboard selection, `/` search, restoration of the selected card, and preview rendering limits must
keep working.

Acceptance: the home page renders a 10k library without rendering 10k cards; keyboard navigation
across page boundaries works; restoring home returns to the previously selected paper.

Tests: should render only visible cards when the library holds 10k papers; should move selection
across a virtualized boundary when pressing `j`; should restore the selected card when returning
home.

Blocked by: E0.1

### E0.5 Bounded concurrent batch ingest

Run extraction and heuristic mapping across papers with a bounded worker pool, keeping the
per-paper job reservation. Model-backed stages keep their own, lower concurrency limit.

Acceptance: batch extraction throughput scales with the worker count up to the configured limit;
one failure does not stop the batch; concurrent first-use extraction of the same paper is
impossible.

Tests: should process papers concurrently when the worker limit exceeds one; should continue the
batch when one paper fails extraction; should refuse a second job for a paper already running.

Blocked by: E0.1

---

## E1 Paper objects

Goal: every paper exposes its figures, tables, equations, theorem-like statements, proofs,
algorithms, and bibliography entries as typed, anchored, linkable objects, and the reader presents
the important ones in a Figures tab.

### E1.1 PaperObject types and objects artifact

Define `PaperObject` with kinds `figure`, `table`, `equation`, `statement` (theorem, lemma,
proposition, corollary, definition, remark), `proof`, `algorithm`, and `bib_entry`. Each object has
a stable ID (`fig-3`, `tab-2`, `eq-12`, `thm-2.1`, `proof-thm-2.1`, `alg-1`, `ref-17`), label,
page, anchors in reading-index coordinates, region rectangle, confidence, and in-text mentions.

Deterministic objects derive from the reading index and are stored in `objects.json` with the
reading-index generation they came from. Enrichment output (E1.7) is stored separately so the
deterministic layer stays model-free. Add `GET /api/papers/{id}/objects`.

Acceptance: existing figure/table detection is exposed through the new type without regressions;
objects are invalidated when the reading index generation changes; IDs are stable across rebuilds
of an unchanged PDF.

Tests: should keep object IDs stable when the index is rebuilt for an unchanged PDF; should
invalidate objects when the reading-index generation changes; should serialize every kind with its
anchors.

Blocked by: none

### E1.2 Backend bibliography extraction and parsing

Move bibliography detection and entry splitting from `paperLinks.ts` to the backend, and persist
entries as `bib_entry` objects. Parse each entry into raw text, printed key, authors, title, year,
venue, DOI, and arXiv ID, with per-field confidence. Resolve citation markers (numeric, ranges,
superscript, author–year) to entries in the backend and store each mention with its sentence
anchor. The frontend consumes the backend result for link hints.

Implementation contract: objects schema 2 stores parsed fields under `bib_entry.bibliography`;
each field has a nullable `value` and categorical `explicit`, `heuristic`, or `missing` confidence.
These labels are evidence categories, not calibrated probabilities. Authors retain printed name
strings, publication years retain citation suffixes, and arXiv IDs retain printed versions.
Conflicting identifiers leave the corresponding structured field missing; the raw entry remains.
Wrapped entries retain disjoint member anchors, excluding classified floating captions. Object
IDs use deterministic bibliography order within the paper and remain stable for an unchanged index.

Resolved mentions keep the exact UTF-16 occurrence anchor and its source token rectangles separately
from `sentence_anchor`. If a citation crosses the sentence segmenter's abbreviation boundary, its
context covers all intersected sentence segments. `unresolved_citations` retains missing, ambiguous,
and unsupported-range keys with candidate IDs; ranges expand at most 30 steps. Person-name parsing
supplies a family key only when every retained name interpretation agrees. No entity merge occurs.
The reader requires both paper ID and exact reading-index ETag agreement before using reference
links, refreshes a mismatched pair once, and preserves figure/table/native hints when references
are unavailable. Browser fixtures invoke the production Rust object builder through the offline
`objects_fixture` example, so no entry splitter or citation matcher remains duplicated in the
frontend. A narrow bibliography-section mask remains there solely to exclude figure/table mentions
inside references when backend objects are unavailable; appendix mentions after references remain active.

The offline [bibliography collector](../eval/bibliography-contract.md) measures O8–O10 against
independently aligned K1 entries/occurrences and deposited K2 reference field labels. Segmentation
matches one-to-one by exact non-whitespace UTF-16 membership; an unmatched truth entry still
contributes every known field to O9's denominator. Unknown field labels are excluded and counted.
Occurrence/target pairs retain misses, wrong destinations, and duplicate predictions. O9 components
stay unavailable until both genuine truth populations have known labels. Synthetic fixtures only
verify arithmetic and production integration; they never become truth evidence.

Acceptance: link-hint behavior matches or improves on the current frontend matcher across the
existing smoke fixtures; DOIs and arXiv IDs are extracted exactly; ambiguous marker matches remain
unresolved rather than guessed.

Tests: should split numbered entries when keys are bracketed; should split author–year entries
when no printed keys exist; should extract a DOI when it wraps across lines; should leave a marker
unresolved when two entries match; should resolve "Smith et al. 2020a" to the correct suffixed
entry.

Blocked by: E1.1

### E1.3 Numbered equation detection

Detect display equations with printed numbers (for example a right-aligned `(12)` or `(2.3)`),
estimate their regions, and attach in-text mentions (`Eq. (12)`, `Equation 12`, `eqs. (3)–(5)`).
Unnumbered display math is detected only when its region is unambiguous.

Acceptance: equation numbers are not confused with numeric citations or list markers; mentions of
equation ranges attach to every equation in the range.

Tests: should detect an equation when its number is right-aligned on the same line; should not
detect a numeric citation as an equation; should attach a mention to each equation when a range is
cited.

Blocked by: E1.1, E8.3

### E1.4 Theorem-like statements and proofs

Detect statement headers (`Theorem 2.1.`, `Lemma 3 (Name).`, `Definition 4`, `Corollary A.2`) and
their extents. Detect proofs from `Proof.` / `Proof of Theorem 3.` to an end marker (□, ∎, `Q.E.D.`)
or the next structural boundary. Link each proof to its statement: the explicitly named statement
when given, otherwise the nearest preceding one. Record deferrals such as "the proof is in
Appendix B" as links.

Acceptance: proofs in appendices link to statements in the body; a statement without a proof is
valid; unlinked proofs remain visible with an explicit unlinked state.

Tests: should link a proof to the preceding theorem when the proof is unnamed; should link an
appendix proof to the named lemma; should end a proof at the next heading when no end marker
exists; should record a deferral when the text says the proof appears in an appendix.

Blocked by: E1.1, E8.3

### E1.5 Algorithm and listing detection

Detect captioned algorithm blocks (`Algorithm 1`) and code listings with their regions and
mentions.

Acceptance: algorithm captions are not misclassified as figures; mentions attach correctly.

Tests: should detect an algorithm when its caption precedes numbered steps; should not classify an
algorithm as a figure.

Blocked by: E1.1, E8.3

### E1.6 Link hints for new object kinds

Extend `f` link hints and `Ctrl-o` return to equations, statements, proofs, and algorithms, and
remove the negative "Eq."/"Theorem" citation filtering where a positive object match now exists.
`Tab` previews the destination's label and first line.

Acceptance: `Eq. (3)` and `Theorem 2.1` receive hints that follow to the object; hints respect
section crops as existing hints do.

Tests: should show a hint when an equation mention is visible; should follow a theorem mention to
its statement; should return to the prior position when pressing `Ctrl-o`.

Blocked by: E1.3, E1.4

### E1.7 Object enrichment: ranking and quoted context

Add a model stage that ranks the paper's most important figures and tables (the top 3–5) and
writes context for each: exact quoted passages that discuss the object, verified against the
source like other key quotes, plus a labeled AI explanation of what the object shows and why it
matters. Deterministic signals (mention count, section family, region size) are provided to the
model and used alone by the heuristic provider. Output is cached with the same keying as other
stages and refreshable independently.

Acceptance: unverified quotes are withheld; the heuristic provider produces a ranking without
generated prose; refresh does not touch analysis, highlights, or deterministic objects.

Tests: should withhold a quote when it does not match the source exactly; should rank by mention
count when using the heuristic provider; should reuse the cached stage when inputs are unchanged.

Blocked by: E1.1, E8.3

### E1.8 Figures tab replaces the Glossary slot

Replace Glossary in the reading ladder (Abstract → Overview → Figures → Text) and bind `g` to
Figures. Show enriched figures and tables first, then remaining figures/tables, equations,
statements, and algorithms. Each card shows the PDF region crop, caption or label, quoted mentions,
and AI context when present. Glossary moves to `:glossary` and digest links.

Acceptance: keyboard navigation matches other spatial lists; `Enter` opens the object in the
reader; papers without enrichment still show deterministic objects; `:glossary` opens the existing
view.

Tests: should open Figures when pressing `g` in the reader; should list enriched objects first when
enrichment exists; should open the object's page when pressing Enter; should open Glossary when
running `:glossary`.

Blocked by: E1.1

### E1.9 Clickable proofs

In the Figures tab and the reader, a statement with a linked proof offers expand-in-place (cropped
proof region) and jump-to-proof. Appendix proofs open in the full reader with `Ctrl-o` return.

Acceptance: expanding a proof does not lose list position; jumping to a deferred proof lands on the
appendix location.

Tests: should expand the proof region when activating a statement's proof; should jump to the
appendix when the proof is deferred.

Blocked by: E1.4, E1.8

---

## E2 Knowledge base core

Goal: a resolved, provenance-preserving graph of works, people, authorships, and citations across
the library, with reviewable matching and stable identities.

### E2.1 SQLite store, migrations, and rebuild

Add a bundled `rusqlite` store at `<data>/kb/kb.sqlite` with versioned migrations, WAL mode, indexed
citation edges in both directions, and an FTS5 trigram index for titles and names. Add
`lysilogy kb rebuild`, which recreates the database from per-paper artifacts, the provider cache,
and canonical plain-text state.

Acceptance: deleting the database and rebuilding yields identical entities, IDs, and aliases;
migrations run on startup; two-hop neighborhood queries meet a target measured on a 500k-Work
synthetic graph.

Tests: should produce identical entities when rebuilding from scratch; should apply pending
migrations when opening an older database; should return a two-hop neighborhood when edges exist in
both directions.

Blocked by: none

### E2.2 KB domain types

Define `WorkId`, `PersonId`, `Identifier` (reusing and extending `citation_graph::Identifier`),
`Work`, `WorkVersion`, `LocalCopy`, `Person`, `NameVariant`, `Authorship`, `Citation`,
`CitationEvidence`, `Observation`, `Decision`, and `AcquisitionState` as described in
[Knowledge base model](#knowledge-base-model).

Acceptance: all types serialize to stable JSON; enums keep states explicit; aliases resolve to
surviving IDs.

Tests: should round-trip every type through JSON; should resolve an alias to its surviving ID
after a merge.

Blocked by: none

### E2.3 Person name parser and normalizer

Parse printed names into given names, particles, family name, and suffix, and produce blocking keys.
Handle `Last, First`, initials with or without periods, particles, suffixes, hyphenated and
compound names, diacritics, all-caps names, and family-name-first orders. Keep the raw string.

Acceptance: every documented form parses; ambiguous forms return alternatives rather than a single
guess.

Tests: should parse "van der Waals, J. D." with particle and family name; should parse "J.-P. Serre"
with a hyphenated initial; should match "Müller" and "Mueller" blocking keys; should return
alternatives when order is ambiguous.

Implementation contract: `kb::names::parse_name` retains the exact raw string and returns an
unranked set of component interpretations. Commas delimit family-first forms; unmarked full
names retain both orders and compound-family boundaries. Period-marked initial groups constrain
candidate splits; bare letters retain the literal family-name interpretation too. Neither surname
dictionaries nor inferred ethnicity choose an order. Compact capital groups and
unmarked Roman suffixes keep their alternative word/initial interpretations. A single undivided
name receives no given-name initial or wildcard blocking key.

Components preserve case and accents with NFC/whitespace normalization; blocking keys separately
fold accents, punctuation, case, and common ligatures. Umlaut spellings also supply ae/oe/ue
variants so Müller/Mueller retrieve one another. Particle prefixes retain a literal compound-family interpretation too; the particle vocabulary
never excludes a whole family or given name such as Le, Van, or Al. Particle-bearing and
particle-free keys are candidate retrieval aids. Key collisions (including different given names or suffixes) never
establish person equality; raw evidence and all alternatives survive for the later resolver.
Inputs over 1024 bytes, 16 words, or 256 alternatives, and unsupported syntax, remain explicitly
unresolved without truncation. K3/K4 resolver metrics stay unavailable until their truth and
resolution collectors exist; these parser fixtures do not claim precision or clustering scores.

Blocked by: E2.2

### E2.4 Title normalizer and fuzzy index

Produce exact title keys by folding Unicode, diacritics, LaTeX markup, punctuation, and case, and
populate the trigram index. Provide a similarity score used by resolution.

Acceptance: common variants (LaTeX math, curly quotes, trailing periods, subtitle separators)
normalize identically; distinct titles sharing a prefix do not.

Tests: should normalize "$\alpha$-Divergence" and "α-divergence" to the same key; should treat
"Title: Subtitle" and "Title - Subtitle" as equal; should not equate "Attention Is All You Need" with
a paper titled "Attention Is Not All You Need".

Implementation contract: `kb::titles::title_key` folds compatibility Unicode, letter diacritics,
case, ordinary punctuation and known LaTeX presentation commands. Words, negation, repetition,
mathematical operators and script grouping remain significant. Unknown LaTeX command names retain
their spelling and case, and their argument braces remain explicit; unsupported macros are not
silently interpreted or removed. This is a syntactic decoder, not a TeX execution engine.

`title_similarity` uses a multiset character-trigram Sørensen–Dice score, with two boundary
sentinels on each side of the normalized title and repeated grams counted. Empty inputs score
zero. Unequal keys above 1,024 Unicode characters score zero to bound fuzzy work and memory;
exact nonempty equality remains comparable at any length. A fuzzy score is not a probability:
different strings can share a gram multiset, so exact title equality must use the key itself.
Neither helper makes entity identity decisions or claims resolver metrics before K3 exists.
E2.1 owns the SQLite FTS5 table and populates it with `title_key` when the two branches integrate;
there is one database index, rather than a second in-memory candidate store.

Blocked by: E2.2

### E2.5 Resolution gold set and evaluation

Mine roughly 200 candidate pairs (names and titles) from parsed local bibliographies, including
hard negatives such as same-surname different people and near-identical titles. Label them with
agents, not humans: two independent model labelers see each pair with its bibliographic context
(and may use web lookup for identifiers), and a third adjudicator resolves disagreements, recording
its rationale. Labels, labeler agreement, and adjudication notes are stored in plain text so a
human can audit them later. Add an evaluation command that reports precision and recall by matcher
version.

Acceptance: gold set checked into the repository without copyrighted full text; labeling is
reproducible from a checked-in script; `cargo test` fails when auto-merge precision falls below
0.99; recall and inter-labeler agreement are reported.

Tests: should fail evaluation when auto-merge precision drops below 0.99; should report recall
when the gold set is evaluated; should route a pair to adjudication when the two labelers
disagree.

Blocked by: E1.2, E8.2

### E2.6 Resolution engine

Implement candidate generation (identifier lookup, title key, trigram blocking, name blocking),
scoring, the auto-merge threshold, and a candidate queue for everything below it. Record every
automatic merge as a decision with its score and matcher version.

Acceptance: meets the E2.5 precision gate; rerunning with a new matcher version never overrides a
manual decision; resolution of a new paper's bibliography is incremental.

Tests: should merge works when they share a DOI; should queue a candidate when titles are similar
but years differ by three; should respect a `distinct` decision when scores exceed the threshold;
should keep a manual merge when the matcher version changes.

Blocked by: E2.1, E2.3, E2.4, E2.5, E8.5

### E2.7 Decision log and stable identities

Implement `kb/decisions.jsonl` (merge, split, distinct, automatic merge), its SQLite mirror, alias
maintenance, and split semantics.

Acceptance: decisions replay deterministically during rebuild; splitting a merged entity restores
the moved observations under a new ID and leaves the original ID with the rest.

Tests: should replay decisions in order when rebuilding; should keep the original ID for the larger
side when splitting; should reject a merge that contradicts a `distinct` decision.

Blocked by: E2.1, E2.2

### E2.8 Ingest local papers

Convert each mapped paper's metadata, bibliography entries, and citation mentions into
observations, resolve them, and link the paper to its Work by content hash. Ingest is incremental
and runs after extraction.

Acceptance: a local paper appears as a Work with a local copy; its bibliography entries become cited
Works with local citation evidence including sentence anchors; re-ingesting an unchanged paper is a
no-op.

Tests: should create citation evidence with the in-text sentence when a marker resolves; should
link two local papers when one cites the other; should not duplicate observations when re-ingesting.

Blocked by: E0.2, E1.2, E2.6, E2.7

### E2.9 Ingest provider snapshots

Convert existing `citation-graph.json` snapshots into provider observations and citation evidence,
preserving each provider's identity and retrieval time. Agreement between providers is not
treated as independent corroboration.

Acceptance: provider edges attach to resolved Works; unresolved provider records remain as
observations.

Tests: should keep separate evidence records when two providers report the same edge; should retain
an unresolved provider work as an observation.

Blocked by: E2.6, E2.7

### E2.10 KB API

Add endpoints for Works, Persons, aliases, a Work's citations and references (local and provider,
with evidence), a Person's works and co-authors, bounded neighborhoods, and search across Works and
Persons.

Acceptance: alias IDs redirect to surviving entities; neighborhood responses are bounded by node
count; responses distinguish local from provider evidence.

Tests: should return the surviving Work when requested by alias; should cap a neighborhood when it
exceeds the node limit; should label evidence by source.

Blocked by: E2.8

### E2.11 Review queue

A keyboard-driven view of match candidates showing both sides with their observations and
evidence. Keys accept a merge, record `distinct`, skip, or split an existing entity.

Acceptance: each action writes a decision immediately; the queue orders by expected impact (for
example, candidates touching local papers first).

Tests: should write a merge decision when accepting a candidate; should remove both sides from the
queue when marking them distinct.

Blocked by: E2.10, E2.12

### E2.12 Routes plus Work and Person pages

Replace the `selectedId === null` home test with a discriminated route (`home`, `paper`, `work`,
`person`, `list`, `graph`) parsed from the hash, with Back/Forward, Escape, and command-menu support.
Add basic Work pages (metadata, versions, local copies, acquisition state, references, citations)
and Person pages (variants, works, co-authors).

Acceptance: every route is directly linkable; existing `#home` and `#paper=` links keep working;
unsaved-notes guards apply to every navigation.

Tests: should open a Work page when loading `#work=<id>`; should keep `#paper=<id>` working when
routing changes; should prompt about unsaved notes when navigating away from a paper.

Blocked by: E2.10

---

## E3 Acquisition

Goal: from any reference, make incremental, observable progress toward a mapped local copy —
identifier, link, download, map — and record precisely why a paper is unavailable.

### E3.1 Acquisition state and global jobs

Implement per-Work acquisition state with stage history, and a job runner for jobs not owned by one
paper (acquisition batches, reading-list generation). Jobs persist under `kb/jobs/`, survive restart
as retryable interrupted failures, and expose progress by stage.

Acceptance: a retry resumes from the last completed stage; job state is backend-owned; concurrency
is bounded per job type.

Tests: should resume at the download stage when the identifier and link already exist; should mark
a running job interrupted when the server restarts.

Blocked by: E2.1, E2.2

### E3.2 Deterministic identifier resolution

Resolve references to identifiers without a model: DOI/arXiv from the entry text, Crossref
`query.bibliographic`, Semantic Scholar `/paper/search/match`, OpenAlex title search, and the arXiv
API. Every returned candidate is scored with E2.4/E2.3 before acceptance.

Acceptance: an identifier is accepted only when its metadata matches the reference above the
threshold; rejected candidates are recorded with scores.

Tests: should accept a Crossref match when title and first author agree; should reject a
high-ranked search result when its year and authors disagree.

Blocked by: E3.1, E7.1, E8.4

### E3.3 Open-access locators

Given an identifier, find legitimate open-access PDFs through Unpaywall (with a configured contact
email), OpenAlex `best_oa_location`, arXiv, and Semantic Scholar `openAccessPdf`, recording each
candidate's host type and license where available.

Acceptance: locators run in a documented priority order; a missing open-access copy yields
`unavailable` with the providers checked.

Tests: should prefer an arXiv PDF when the reference has an arXiv ID; should record the providers
checked when no copy is found.

Blocked by: E3.2

### E3.4 Agent fallback

When deterministic resolution or location fails, run a schema-constrained `claude -p` (or Codex)
agent with web search and fetch tools. The reference text is treated as untrusted input. The agent
returns identifiers and URL candidates with explanations; the backend verifies all of them.

Acceptance: the agent never writes files or downloads; its proposals pass the same checks as
deterministic candidates; each call is explicit and bounded by timeout.

Tests: should reject an agent-proposed DOI when its metadata does not match; should record the
agent explanation when a candidate is accepted.

Blocked by: E3.1

### E3.5 Downloaded PDF verification

After download through `import_remote_pdf`, extract the first page and verify title and authors
against the expected Work. A mismatch quarantines the file and records the reason instead of
linking it.

Acceptance: a wrong paper is never linked to a Work; a verified download links by content hash.

Tests: should quarantine a PDF when its title does not match the expected Work; should link a PDF
when title and first author match.

Blocked by: E3.1

### E3.6 Post-download policy

Add a configuration setting (`download_only`, `extract_heuristic` default, `full_model`) applied
after verified downloads, overridable per batch. A batch of more than five papers with `full_model`
requires explicit confirmation stating that it may consume significant credits, API budget, or
usage.

Acceptance: the setting is visible in the fetch UI and CLI; confirmation is required in both.

Tests: should require confirmation when a `full_model` batch exceeds five papers; should stop after
download when the policy is `download_only`.

Blocked by: E3.1, E0.5

### E3.7 Fetch UI

From a reference hint, a bibliography entry, a Work page, or a reading list, fetch one paper; from a
paper, fetch all its references. Show per-reference stage progress, unavailable reasons, and retry.
Supersedes the single-step reader-tools Find & fetch while preserving saved references.

Acceptance: batch progress is visible and resumable; existing saved references migrate.

Tests: should show each reference's stage when fetching all references; should retry only failed
stages when pressing retry.

Blocked by: E2.10, E3.2, E3.3, E3.4, E3.5

---

## E4 Citations

Goal: generate reliable citations for any Work or reading list.

### E4.1 CSL-JSON, BibTeX, and RIS

Generate CSL-JSON as the internal export form, plus BibTeX (stable citation keys, escaped LaTeX) and
RIS from resolved Works.

Acceptance: exports import cleanly into Zotero and BibLaTeX; citation keys are deterministic and
collision-free within an export.

Tests: should escape special characters when writing BibTeX; should disambiguate keys when two
works share author and year.

Blocked by: E2.2

### E4.2 Styled citations

Render formatted citations (APA, Chicago, IEEE, and others) with a CSL processor. Spike hayagriva in
Rust against citation-js in the frontend, choose on output correctness for the target styles,
and record the choice and evidence in this section. For DOIs, doi.org content
negotiation can supply publisher-formatted text as a cross-check.

Acceptance: at least APA, Chicago author-date, and IEEE render correctly for journal articles,
conference papers, preprints, and books.

Tests: should render an APA citation when given a journal article; should render an arXiv preprint
with its identifier.

Blocked by: E4.1

### E4.3 Copy and export commands

Commands to copy a Work's citation in a chosen style, export a reading list as `.bib`/RIS/CSL-JSON,
and export the citations of the current paper.

Acceptance: keyboard accessible from reader, Work page, and list view; the default style is
configurable.

Tests: should copy the citation when running the copy command on a Work page; should export every
list item when exporting a reading list.

Blocked by: E4.1, E5.1

---

## E5 Reading lists

Goal: build, maintain, and explore reading lists by keyboard, including AI-generated lists with
visible relationships.

### E5.1 Reading list model and API

Canonical `kb/lists/<id>.json` with ordered items (Work ID or unresolved proposal), status
(to-read, reading, read), note, role (foundational, survey, prerequisite, frontier, other),
rationale, and generation provenance; mirrored into SQLite; CRUD and reorder API.

Acceptance: lists survive rebuilds; items referring to merged Works follow aliases.

Tests: should follow an alias when a listed Work is merged; should preserve order when reordering.

Blocked by: E2.1, E2.2

### E5.2 Keyboard reading-list view

`#list=<id>` view: `j/k` move, `J/K` reorder, `dd` remove, `a` fuzzy-add any Work (owned or not),
`x` cycle status, `e` edit note, `f` fetch, `o` open, `u` undo. A lists index shows all lists with
progress.

Acceptance: every operation works without a pointer and persists immediately; undo restores the
previous list state.

Tests: should move the item down when pressing `J`; should add a Work when choosing it from the
fuzzy finder; should restore a removed item when pressing `u`.

Blocked by: E2.12, E5.1

### E5.3 AI-generated reading lists

A prompt such as "the ten most important papers to understand VLMs and VLAs in robotics" runs an
agent with web tools that proposes Works with roles, rationale, and a suggested order. Each proposal
is resolved through E3.2; unresolved proposals stay marked unverified and have no graph edges.
Relationships among resolved items are fetched from providers.

Acceptance: generation runs as a global job with progress; the prompt, model, and time are recorded
in list provenance; nothing is downloaded unless requested.

Tests: should mark a proposal unresolved when no identifier matches; should record the prompt in
provenance when generation completes.

Blocked by: E3.1, E3.2, E5.1, E8.6

### E5.4 List relationship graph

Show the citation and shared-author relationships among a list's items, laid out by year, beside or
within the list view, using the E6 renderer.

Acceptance: selecting a node selects the list item and vice versa; unresolved items appear outside
the graph with their badge.

Tests: should select the list item when a node is focused; should omit unresolved items from graph
edges.

Blocked by: E5.2, E6.1, E6.2

---

## E6 Graph

Goal: a focused, legible, keyboard-driven view of a literature's structure.

### E6.1 Neighborhood and metrics backend

Compute bounded k-hop neighborhoods around a Work, Person, or list, and metrics over the extracted
subgraph: in-library citation counts, PageRank, co-citation strength, and bibliographic coupling.

Acceptance: responses are bounded and ranked; metrics are computed in memory over the subgraph, not
the whole KB.

Tests: should compute co-citation strength when two works share citing papers; should rank
neighbors by in-library citation count when the limit truncates.

Blocked by: E2.10

### E6.2 Graph view

`#graph` view with graphology and sigma.js: focus node plus k-hop neighborhood, x axis by year, y by
influence, node styling for owned/unowned, list membership, and acquisition state, and filters for
edge type (citation, co-citation, coupling, co-authorship). No whole-KB rendering by default.

Acceptance: stays responsive at the neighborhood cap; refocusing animates; theme and ink inversion
follow reader settings.

Tests: should place newer works to the right when laying out by year; should hide co-citation edges
when that filter is off.

Blocked by: E2.12, E6.1

### E6.3 Keyboard graph navigation

`h/j/k/l` move between nodes spatially (reusing `spatialNavigation.ts`), `Enter` refocuses, `o`
opens the paper or Work page, `f` fetches, `a` adds to a list, `Ctrl-o` returns to the prior focus.

Acceptance: every graph action is keyboard reachable.

Tests: should focus the nearest node to the right when pressing `l`; should return to the previous
focus when pressing `Ctrl-o`.

Blocked by: E6.2

### E6.4 Communities and read-next suggestions

Detect communities (Louvain) in the neighborhood and suggest what to read next: unowned or unread
Works most cited by papers already read, with the evidence for each suggestion.

Acceptance: suggestions cite the specific local papers that justify them.

Tests: should suggest the most co-cited unread work when several read papers cite it; should list
the citing local papers as evidence.

Blocked by: E6.1, E6.2, E8.4

---

## E7 Integrations

Goal: use external scholarly databases efficiently and within their policies at library scale.

### E7.1 Provider cache and rate budgets

A shared on-disk provider response cache keyed by provider, endpoint, and identifier with explicit
expiry, and per-provider request budgets with cooldowns, usable by citation-graph fetches,
resolution, and acquisition batches.

Implementation defaults: seven-day successful JSON cache and durable provider budgets under
`~/.cache/lysilogy/providers/`, shared across data roots and processes. Budgets retain 1.1-second
spacing and one active connection, with an explicit local limit of 50 admissions per 60-second
window. The public Rust constructor supports other shared storage roots and configurable per-provider
policies. Credentials and response echoes are removed before serialization. See
[citation provider operations](citation-graph-sources.md#shared-response-cache-and-batch-budgets)
for expiry/provenance semantics, policy evidence and the offline O30 collector.

Acceptance: repeated lookups hit the cache; a 10k-paper batch stays within configured budgets;
credentials never enter cache keys or files.

Tests: should serve a cached response when the entry has not expired; should defer requests when a
provider budget is exhausted; should omit credentials when writing cache entries.

Blocked by: none

### E7.2 Semantic Scholar recommendations

Use Semantic Scholar recommendations as a labeled "related" source for Work pages, reading lists,
and read-next suggestions. Recommendations are similarity, never citation evidence.

Acceptance: recommendations are visually distinct from citations and never become citation edges.

Tests: should label recommendations as similarity when displayed; should not create citation
evidence from a recommendation.

Blocked by: E2.10, E7.1

### E7.3 Author identity from OpenAlex and ORCID

Attach OpenAlex author IDs and ORCIDs to Persons and use them as strong evidence in person
resolution.

Acceptance: a shared ORCID merges persons automatically; conflicting ORCIDs block a merge.

Tests: should merge persons when they share an ORCID; should block a merge when ORCIDs differ.

Blocked by: E2.6, E7.1

---

## E8 Evaluation

Goal: every scorecard metric is computed automatically from agent-built truth sets, results are
tracked per commit, and regressions block merges.

### E8.1 Evaluation harness, scorecard, and ratchet

Add `lysilogy eval <suite>` (suites: `objects`, `bibliography`, `resolution`, `persons`,
`acquisition`, `citations`, `lists`, `read-next`, `scale`, `all`). Each run writes
`eval/results/<suite>/<timestamp>-<commit>.json` with metric values, truth-set versions, and
cost/time, and regenerates `docs/kb-scorecard.md` showing current value, baseline, target, and
status per metric. `eval/baselines.json` holds ratcheted baselines; `lysilogy eval --check`
exits nonzero on a failed hard gate or a regression beyond tolerance. Add a G5 check that runs the
test suites with network disabled and no model CLIs on `PATH`.

Acceptance: suites without truth sets report "unavailable" rather than failing; baselines ratchet
only upward (downward for latency and cost) unless a justification file accompanies the change;
the scorecard is deterministic for identical results.

Tests: should exit nonzero when a hard gate fails; should exit nonzero when an objective regresses
beyond tolerance; should report a suite as unavailable when its truth set is missing; should
ratchet the baseline when a metric improves.

Implementation contract (E8.1): suites consume versioned, content-addressed local observations
from `eval/inputs/<suite>/<collector-id>.json` (alongside legacy `eval/inputs/<suite>.json`);
each later detector/resolver/benchmark issue adds its collector. Collectors own disjoint metrics;
duplicate ownership fails. Each collector retains independent dependency hashes and cost/time,
so stale evidence cannot invalidate or be re-attested by another collector. Discovery is bounded
to 32 files per suite and 8 MiB per input.
The collector must rerun against the current implementation before evaluation. The harness
calculates ratios, F1, mean, median, and nearest-rank p95 from raw samples and also accepts
collector-computed statistics (such as B-cubed F1) with case counts and evidence. Changed
implementation, truth, or observation hashes make the metric unavailable. See
[`eval/README.md`](../eval/README.md) for the interchange contract. No missing collector or truth
set implies a passing measurement.

Compound scorecard metrics retain independent components; an objective counts at target only
when every component does. The additional `tests` suite owns G5, and `all` includes it. G5 runs
`cargo test --offline --all-targets`, discovered `scripts/**/test*.py` unittest suites, and every
frontend `test`/`test:*` script in an unprivileged
network namespace with an allowlisted executable PATH; logs, namespace identity, a blocked
connection probe, and model-CLI absence are retained. Browser smoke scenarios remain explicit
verification, separate from these fixture unit tests. Failure to establish isolation fails G5.

`--check` permits unavailable suites during incremental implementation; final system acceptance
also runs `lysilogy eval all --check --require-complete`, requiring all five gates and at least
24 of the 30 objectives. Latency tolerance is 10% relative, proportions permit 0.01 absolute,
and other objective units permit no regression without documented evidence. Baselines improve
automatically only on a run without failures; within-tolerance declines never lower them.
`--justification` accepts explicit objective resets with a recorded reason and hashed evidence;
hard gates cannot be waived. R metrics are reported without a baseline ratchet.

O12 precision/recall live in `resolution`; identifier-acquisition changes run both `resolution`
and `acquisition`. O30 provider-budget simulation lives in `scale`. Synthetic scale truth may
establish early baselines, while the final corpus scale report must identify the real K0 tier.

Blocked by: none

### E8.2 arXiv research corpus

Build a separately stored arXiv corpus (K0) outside the library, the data root, the repository,
and `/tmp`, at a configurable root (default `~/Corpora/arxiv/`, `LYSILOGY_CORPUS`).

- **Metadata:** harvest arXiv OAI-PMH (`https://oaipmh.arxiv.org/oai`, `metadataPrefix=arXiv`) for
  the selected category sets, incrementally, into a local metadata store.
- **Selection:** an `eval` tier of about 1,000 papers stratified by category and year for LaTeX
  truth, and a `scale` tier of 10,000 papers concentrated in a few dense subfields so that
  intra-corpus citations are common (for example robotics, computer vision, and machine learning
  papers on VLMs/VLAs; probability and statistical learning theory for theorem-heavy papers).
  Selection is deterministic from a seed and a checked-in selection config.
- **PDFs:** download from the public Google Cloud Storage bucket over HTTPS
  (`https://storage.googleapis.com/arxiv-dataset/arxiv/arxiv/pdf/<YYMM>/<id>v<n>.pdf`; listing via
  the JSON API). No `gsutil` is required.
- **Sources:** fetch LaTeX sources for the `eval` tier per paper from
  `https://export.arxiv.org/src/<id>v<n>`, with a shared single-connection budget of at most one request every
  three seconds, including retries. This follows the stricter
  [API terms](https://info.arxiv.org/help/api/tou.html), checked 2026-09-12; the bulk
  harvesting page separately permits four-request bursts. The requester-pays S3 bucket
  `s3://arxiv/src/` is not used by default because its monthly tar chunks mix all categories.
- **Manifest:** `manifest.jsonl` records ID, version, categories, tier, file hashes, and fetch
  times. Downloads are resumable and verified; partial files never enter the corpus.
- **Mapping:** the corpus is ingested with its own data root (`--library <corpus>/pdf --data
  <corpus>/.lysilogy`) so it never mixes with the user's library.

Terms: most arXiv papers carry arXiv's default license, which does not grant redistribution. Only
IDs, derived labels, and metrics are committed; tools link back to arXiv for downloads.

Implementation: standard-library Python tooling at `scripts/corpus/corpus.py` and checked-in
`scripts/corpus/selection.json`. See [corpus operations](../scripts/corpus/README.md) for commands,
selection/version freezing, format and hash verification limits, isolated scale mapping, and
background resume. OAI modification dates are only incremental-harvest bounds; submitted-year
strata use `created`. Sources explicitly request the pinned PDF version. Missing artifacts or
sparse strata fail without silently reducing the tier count.

Follow-up #88 qualifies each deterministically ranked candidate against a completed public PDF
inventory before filling its original category/year quota. Schema-2 selections pin version,
generation, size and MD5; immutable consulted inventory snapshots and excluded-ID reasons retain
the availability boundary. Invalid or oversized latest objects are excluded, never silently
replaced with an older version. The original OAI `created` value remains unchanged when an ID
suggests an earlier year. Identical metadata, config and inventory snapshots reproduce selection;
interrupted preparation checkpoints its consulted inputs.

The first full harvest exposed missing PDF `1801.00600` after a metadata-only selection froze.
An explicit `recover-selection --reason …` may repair only a legacy schema-1 selection with zero
manifest rows and no artifacts, receipts or partials. It archives exact original bytes and the
failure reason, verifies the original metadata snapshot, retains unavailable original members as
exclusion evidence, and durably stages a full-quota replacement before atomic publication.
Interrupted publication resumes the staged replacement; archives are immutable, and any admitted
artifact prevents recovery. Ordinary resume never reselects a frozen corpus. No metric target,
host, proxy/TLS policy, request pacing, source-version rule or disk floor changes.

Follow-up #91 uses the canonical `/src/` endpoint after live verification showed `/e-print/`
returns HTTP 301. The exact same pinned ID/version's previously verified `/e-print/` source
receipt remains reusable and keeps its original URL, time and bytes, including after an
interrupted manifest update. No new request follows the legacy redirect; other host/path/version
aliases remain invalid. PDF generation URLs remain exact. Automatic redirect refusal, TLS,
shared arXiv pacing and the free-space floor are unchanged.

Direct transport is the default. Managed environments may explicitly select their approved
HTTP CONNECT proxy for HTTPS destinations with `--proxy-env HTTPS_PROXY` (or `https_proxy`).
Unsupported HTTPS-scheme proxy URLs are rejected. This reads only the selected process
variable, redacts proxy credentials from errors, and leaves destination hosts, TLS,
redirect refusal and the shared rate budget unchanged. Environment host permissions remain
independent of this transport choice.

Acceptance: a fresh run reproduces the same selection; an interrupted run resumes without
re-downloading verified files; the harvester stays within the documented rate; disk usage is
reported before download starts and the run refuses to proceed when free space would drop below a
configured floor.

Tests: should select identical papers when given the same seed and config; should resume without
re-downloading when a verified file exists; should discard a partial download when its hash
mismatches; should pace e-print requests within the published limit; should refuse to start when
projected usage exceeds the free-space floor.

Blocked by: none

### E8.3 arXiv LaTeX object truth

Build K1 from the K0 `eval` tier: parse LaTeX sources (resolving `\input`/`\include`, custom
`\newtheorem` names, `\label`/`\ref`/`\eqref`, `\cite`, `.bbl`/`\bibitem`) into truth objects,
align them to the PDF by caption and text matching, drop papers whose alignment falls below a
quality threshold, and record alignment confidence. Include a three-agent panel judging the top
figures and tables for O11.

K1 records overall per-paper alignment completeness separately from explicit exhaustive metric
cohorts. A paper can supply complete figure/table truth while its separate bibliography or math
inventory remains unsupported. Each metric requires its entire independently parsed inventory
and all relevant links; missing members exclude the paper from that metric, never shorten its
denominator. Source semantics that might hide an inventory exclude affected cohorts. Cohorts and
all exclusions are frozen before detector evaluation. This clarifies the truth schema after an
initial 25-paper pilot found useful complete figure/table inventories alongside unsupported math
and bibliography forms; it does not lower any gate, objective or the approximately 500-paper
target. See [the K1 cohort contract](../eval/latex-contract.md). Figure/caption alignment supplies
no full-region O2 labels; O2 region evidence and the three-agent O11 panel remain independent.
Before any panel votes or detector ranks are inspected, O11 arithmetic is fixed as mean top-three
set overlap divided by three against each of three panelists over every frozen paper (denominator
nine per paper). Only the first three model ranking positions count; duplicates, invalid IDs,
missing choices and missing paper results keep their denominator and are not backfilled from
positions four/five. Malformed rankings fail validation. Panel votes require three distinct valid
source IDs; display-only consensus ordering cannot change this score or its 0.70 target.

Staged publication decision (2026-09-13): after the complete frozen 1,000-paper automatic
run produced no accepted metric cohorts, independently reviewed source/PDF annotations may
establish an explicitly versioned limited K1 release covering every E1 object kind. The original
approximately 500-paper target stays unchanged and unmet; [follow-up #97](https://github.com/tjmisko/Lysilogy/issues/97)
tracks finite source capability and independently stratified coverage expansion. Both historical
and current automatic runs, every omitted cohort, per-paper/per-kind denominators, strata and
manual selection bias remain visible. This is a staged publication deviation, not broad K1 or
system acceptance. Detector collectors must report the limited release version and cohort sizes.
Complete reviewed zero-kind papers remain negative examples. Original annotations and automatic
exclusions remain immutable; only compact independently derived labels are committed.

Acceptance: truth covers every E1 object kind; alignment quality is reported per paper; the suite
runs offline from the corpus cache.

Tests: should expand `\input` files when parsing a multi-file source; should map a custom
`\newtheorem{thm}{Theorem}` environment to a theorem truth object; should link a `proof` to the
statement named in its optional argument; should exclude a paper when alignment confidence is
below the threshold.

Blocked by: E8.1, E8.2, E1.1

### E8.4 Reference, acquisition, and read-next truth

Build K2 from Crossref deposited references, K5 as a stratified acquisition sample with open-access
status recorded at build time, and K7 as a leave-one-out holdout over the K0 `scale` tier and local
citation graphs. Provider responses come through the E7.1 cache so rebuilds are offline.

Acceptance: each truth set is versioned with its build date; K5 open-access status is frozen so
metric drift reflects code, not upstream changes.

Tests: should skip deposited references without DOIs when building K2; should stratify K5 by field
and decade; should exclude the held-out paper's own edges when building a K7 fold.

Blocked by: E8.1, E7.1

### E8.5 Person silver labels

Build K4 from OpenAlex authorships with ORCIDs for works in K2 and K0, clustering name mentions by
ORCID, and flag ORCID conflicts as excluded rather than truth.

Acceptance: only mentions with ORCID-backed identity become labels; conflicts are reported.

Tests: should cluster mentions when they share an ORCID; should exclude a mention when its work
lists conflicting ORCIDs for one position.

Blocked by: E8.1, E7.1

### E8.6 Reading-list evaluation

Build K6: ten topic prompts across fields, each paired with a recent survey and its bibliography,
and a three-agent relevance panel with a fixed rubric. Compute O22, O23, and R2 for generated lists.

Acceptance: panel judgments are stored with rationale; survey overlap uses resolved identifiers,
not titles.

Tests: should count a proposal as fabricated when no provider or web evidence finds the work;
should compute survey overlap from resolved identifiers.

Blocked by: E8.1

---

## Suggested implementation order

| Phase | Issues |
| --- | --- |
| A | E0.1, E0.2, E1.1, E1.2, E2.1, E2.2, E2.3, E2.4, E2.5, E7.1, E8.1–E8.5 |
| B | E1.3–E1.8, E2.6–E2.10, E0.3–E0.5 |
| C | E3.1–E3.7, E4.1, E5.1, E5.2, E6.1, E2.12, E8.6 |
| D | E5.3, E5.4, E6.2–E6.4, E4.2, E4.3, E2.11, E1.9, E7.2, E7.3 |
