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
| Resolution ground truth | A hand-labeled gold set mined from local bibliographies. Auto-merge precision must be at least 0.99; recall is reported, not gated. |
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

Blocked by: none

### E0.2 Content-hash paper identity

Record a SHA-256 content hash for each PDF alongside its path-derived `PaperId`. When a PDF moves or
is renamed, re-associate its existing artifacts instead of orphaning them. The KB links local
copies by content hash.

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

Blocked by: E1.1

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

Blocked by: E1.1

### E1.5 Algorithm and listing detection

Detect captioned algorithm blocks (`Algorithm 1`) and code listings with their regions and
mentions.

Acceptance: algorithm captions are not misclassified as figures; mentions attach correctly.

Tests: should detect an algorithm when its caption precedes numbered steps; should not classify an
algorithm as a figure.

Blocked by: E1.1

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

Blocked by: E1.1

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

Blocked by: E2.2

### E2.4 Title normalizer and fuzzy index

Produce exact title keys by folding Unicode, diacritics, LaTeX markup, punctuation, and case, and
populate the trigram index. Provide a similarity score used by resolution.

Acceptance: common variants (LaTeX math, curly quotes, trailing periods, subtitle separators)
normalize identically; distinct titles sharing a prefix do not.

Tests: should normalize "$\alpha$-Divergence" and "α-divergence" to the same key; should treat
"Title: Subtitle" and "Title - Subtitle" as equal; should not equate "Attention Is All You Need" with
a paper titled "Attention Is Not All You Need".

Blocked by: E2.2

### E2.5 Resolution gold set and evaluation

Mine roughly 200 candidate pairs (names and titles) from parsed local bibliographies, including
hard negatives such as same-surname different people and near-identical titles. Provide a small
labeling tool that records labels in plain text, and an evaluation command that reports precision
and recall by matcher version. Only pairs where matcher output and initial labels disagree need
manual review.

Acceptance: gold set checked into the repository without copyrighted full text; CI fails when
auto-merge precision falls below 0.99; recall is reported.

Tests: should fail evaluation when auto-merge precision drops below 0.99; should report recall
when the gold set is evaluated.

Blocked by: E1.2

### E2.6 Resolution engine

Implement candidate generation (identifier lookup, title key, trigram blocking, name blocking),
scoring, the auto-merge threshold, and a candidate queue for everything below it. Record every
automatic merge as a decision with its score and matcher version.

Acceptance: meets the E2.5 precision gate; rerunning with a new matcher version never overrides a
manual decision; resolution of a new paper's bibliography is incremental.

Tests: should merge works when they share a DOI; should queue a candidate when titles are similar
but years differ by three; should respect a `distinct` decision when scores exceed the threshold;
should keep a manual merge when the matcher version changes.

Blocked by: E2.1, E2.3, E2.4, E2.5

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

Blocked by: E3.1, E7.1

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
Rust against citation-js in the frontend and record the choice. For DOIs, doi.org content
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

Blocked by: E3.1, E3.2, E5.1

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

Blocked by: E6.1, E6.2

---

## E7 Integrations

Goal: use external scholarly databases efficiently and within their policies at library scale.

### E7.1 Provider cache and rate budgets

A shared on-disk provider response cache keyed by provider, endpoint, and identifier with explicit
expiry, and per-provider request budgets with cooldowns, usable by citation-graph fetches,
resolution, and acquisition batches.

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

## Suggested implementation order

| Phase | Issues |
| --- | --- |
| A | E0.1, E0.2, E1.1, E1.2, E2.1, E2.2, E2.3, E2.4, E2.5, E7.1 |
| B | E1.3–E1.8, E2.6–E2.10, E0.3–E0.5 |
| C | E3.1–E3.7, E4.1, E5.1, E5.2, E6.1, E2.12 |
| D | E5.3, E5.4, E6.2–E6.4, E4.2, E4.3, E2.11, E1.9, E7.2, E7.3 |
