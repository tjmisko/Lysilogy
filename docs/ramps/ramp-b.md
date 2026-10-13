# Ramp B: "Tell me why" — AI edge labels grounded in local evidence

Status: planned 2026-10-13. Branch `feat/lit-ramp-b-labels`, based on
`integration/literature-viewer` at `b2ad011` (Ramp A merged). Parent plan:
[literature-viewer-plan.md](../literature-viewer-plan.md). Previous ramp:
[ramp-a.md](ramp-a.md).

## What earlier work built

Ramp A gives us everything on the provider side:

- `lysilogy lit <paper>`, which goes paper → `reader.document` → Work (identifier or title match) →
  OpenAlex `referenced_works` → stub Works plus `citation_assertion` (source = provider record) →
  hydrated stub titles.
- Postgres schema `0001`–`0003`. `source.bibliography_entry`, `graph.citation_mention`,
  `graph.entry_resolution`, `graph.relationship_claim`, `graph.work_redirect` and
  `graph.resolution_decision` already exist, but nothing writes to them yet.
- The API serves `/api/works/{id}`, `/api/works/{id}/citations`,
  `/api/graph/neighbors/{id}`, `/api/papers/{id}/work`, and `POST|GET /api/papers/{id}/lit`.
- The web app has the `LiteratureGraph` overlay (`L`) with a Work panel.
- The model sidecar is `src/analysis/local_cli.rs` (`LocalCliAnalyzer`). Its
  `pub(crate) reader_tool::<T>(provider, directory, ReaderToolRequest { schema, prompt,
  live_web })` already runs one structured call through `claude --print --json-schema` or
  `codex exec --output-schema` and parses the result.

The old bibliography work lives on `feat/e1.2-bibliography`. Its worktree at
`.worktrees/feat/e1.2-bibliography` is **read only**. The relevant file is
`src/objects/bibliography.rs`: 1,160 lines of logic and 35 tests. It works on the root
`ReadingIndex` (UTF-16 offsets, tokens with rects, paragraphs with kinds, sentences) and
depends on the 900-line `kb::names::parse_name`. It uses `regex`, `unicode-normalization` and
`unicode-segmentation`.

## Scope

In:

1. Port PDF bibliography entry extraction and field parsing into `lysilogy-ingest`. It is a
   pure function over a crate-local `SourceText` that mirrors the reading index.
2. Port in-text mention detection: numeric `[12]`, `[3-5]`, `(4)`, superscript, and
   author-year `Smith et al., 2020` / `Smith (2020a)`. Each mention keeps its exact sentence
   context and its location (page, UTF-16 offsets, rects).
3. Persist the local evidence: `bibliography_entry` rows (raw text, parsed fields, ordinal,
   page, offsets, region) and `citation_mention` rows (marker, passage, location).
4. Resolve each entry to a Work by identifier, then by exact normalized title, then by
   creating a stub. Each result is written to `entry_resolution`. A local stub that later
   coincides with a provider Work is merged through `work_redirect` + `resolution_decision`.
   Local edges become `citation_assertion` rows keyed by `entry_id`.
5. Add a `RelationshipLabeler` trait with a claude/codex sidecar and a scripted fake. The
   grounding rule applies, labels are cached by (mention-set hash, model), and results are
   written as `relationship_claim` rows (`origin='ai'`, `review_state='unreviewed'`, quote,
   mention reference).
6. CLI: `lysilogy lit <paper> [--local] [--labels [--sidecar claude|codex] [--label-limit N]]`.
7. API: edges in `/citations` and `/neighbors` carry sources, mention counts and labels.
   New endpoints accept, reject or edit a label. `POST /lit` takes an optional body.
8. UI: the Work panel shows the reason and quote for the edge. Clicking it opens the citing
   paper at the exact sentence. Labels can be accepted, rejected or edited.
9. Live smoke with claude (and codex) on real local papers, with counts recorded here.

Out: acquisition (C), relevance/marks/tags (D), multi-hop (E), and porting the full name
parser. Out too: fuzzy or ML title resolution beyond exact `title_key`, batch labeling of the
whole library, labels for provider-only edges (they have no passage to ground in), and
SQLite/journal/truth-set/eval machinery from the old branch.

## Decisions taken

1. **Port location and input type.** The extractor lives in
   `crates/lysilogy-ingest/src/bibliography/` and reads a crate-local `SourceText`.
   `SourceText` is a plain-data mirror of the reading-index fields it needs. The root crate
   converts `ReadingIndex → SourceText` in one small adapter. The ingest crate does not
   depend on the root crate.
2. **No full name parser.** The old code calls `parse_name` in two places. We replace both
   with two small pure helpers in `lysilogy_core::names`:
   - `citation_family(author) -> Option<String>`. For `Family, I.` it returns the part
     before the comma. Otherwise it returns the last word with any preceding lowercase
     particles (`van der Waals`, `de la Cruz`). It returns `None` when the result has
     digits, has no letters, or the author has more than 6 words.
   - `looks_like_person_name(author) -> bool`, used by the dotted-sequence check.

   This is a deliberate behavior change: `citation_family("John Smith")` now returns `Smith`
   where the old parser returned `None`. ML bibliographies print full given names, and
   author-year resolution would otherwise fail on all of them. The old assertion is dropped
   and replaced by a test that expects `Smith`.
3. **Offsets are UTF-16 code units of the reading-index text**, exactly as in the old code
   and the frontend (JS string offsets). Passage text is stored whitespace-collapsed. The
   `local_bibliography` source record stores the reading-index `schema_version` and a sha256
   of `index.text`, so drift can be detected.
4. **Passage = sentence context.** The passage is the union of the sentences that the
   marker intersects, as in the old `Resolver`. If that union exceeds 1,500 UTF-16 units
   (bad segmentation), it is clipped to ±600 units around the marker, snapped to whitespace.
5. **Extraction is evidence and is written once.** One `source.source_record` of kind
   `local_bibliography` is keyed `<document sha256>|bib-v1`. Its payload holds
   `{extractor: "bib-v1", index_schema_version, index_text_sha256, entry_count,
   unresolved: [...]}`. Entries use `entry_key = "bib-v1:<ordinal 1-based>"` with
   `source_record_id` set. If the record already exists, the extraction write is skipped, so
   reruns are idempotent. Bumping the extractor version produces a new record and new entry
   keys; old evidence is never mutated.
6. **Local assertions are keyed by `entry_id`, never by `source_record_id`.** Two entries
   in one bibliography that resolve to the same Work would otherwise collide on
   `citation_assertion_source_key`. They are inserted with status `asserted`, while provider
   edges use `accepted`. The canonical edge stays the `(citing, cited)` pair.
7. **Resolution ladder** (per entry, first hit wins):
   1. An existing accepted `entry_resolution` whose method is `identifier` or `title_match`
      is reused.
   2. Identifier: the entry DOI, else the arXiv ID (normalized with
      `lysilogy_core::identifier::normalize`), looked up with `find_by_identifier`.
      Method `identifier`, confidence 1.0.
   3. Exact title: `title_key(parsed title)` with at least 3 key words. Candidates come first
      from the citing Work's current outgoing neighborhood (provider stubs), then from all
      Works with an equal `graph.work.title_key`. When both years are known they must be
      within ±1. If there are several candidates, prefer neighborhood ones, then the single
      one holding an OpenAlex identifier. If more than one remains, the entry is
      `ambiguous`: one `entry_resolution` row per candidate with status `ambiguous`, and no
      assertion. Method `title_match`, confidence 0.9. On a match, the entry's DOI or arXiv
      ID is attached best-effort with `add_identifier`; a conflict becomes a warning.
   4. Otherwise a stub Work is created, carrying the entry identifier if any (via
      `resolve_or_create`) or none. Its display title and year come from the parsed fields
      via `add_claim_once` + `fill_display_*_if_unknown` (source = the `local_bibliography`
      record). Method `local_stub`, confidence `NULL`.
   5. If step 1 finds an accepted `local_stub` resolution, steps 2–3 are rerun with the stub
      excluded. A different Work found this way means the stub coincides with it, so the
      stub is **merged** into it (decision 8).
   - Self-citation guard: a candidate equal to the citing Work is skipped and recorded as a
     `self_reference` warning.
8. **Merge** (`WorkRepository::merge_work(from, into, reason, actor)`, one transaction):
   - insert a `work_redirect(from → into)` and a `resolution_decision(kind='merge',
     subject_ids=[from, into], actor_id='lit:auto', reason)`;
   - repoint `citation_assertion.cited_work_id` and `citing_work_id`, and
     `reader.document.work_id`;
   - move `field_claim` rows, and move `work_identifier` rows that do not collide (delete
     the colliding duplicates);
   - set the stub's accepted `entry_resolution` rows to `withdrawn` and insert or upgrade
     `accepted` rows for `into` with method `merge`.

   The merge is refused (`DbError::Conflict`) when `from == into`, when `from` already
   redirects, or when a repointed assertion would become a self-loop. Only Works whose every
   accepted resolution is `local_stub` and that have no linked document are ever merged
   automatically. `WorkRepository::resolve_redirect(id)` follows at most 8 hops, and
   `/api/works/{id}` answers for the redirect target.
9. **`graph.work.title_key`** is a column maintained in Rust. `set_display_title` and
   `fill_display_title_if_unknown` write it. `backfill_title_keys()` fills missing keys in
   batches of 500 and runs at the start of each local-evidence pass, which covers Ramp A
   data.
10. **Mentions need an assertion.** `citation_mention.assertion_id` is `NOT NULL`, so only
    mentions of resolved entries are stored. Unresolved markers (missing target,
    ambiguous target, unsupported range, ambiguous marker) are counted in the summary and
    kept in the source record payload with their reason, never as rows.
11. **Labels are per canonical edge from local evidence.**
    - Input: the edge's distinct passages from this document version (at most 6, in text
      order, each ≤ 1,500 units), the citing title, and the cited Work's metadata: display
      title, else the parsed entry title; year; the first 3 parsed authors; the entry raw
      text clipped to 400 chars.
    - Output schema (strict, every key required, `additionalProperties: false`, which codex
      needs): `{relation, reason, quote, passage_index}`. `relation` is one of
      `background | uses_method | uses_data | extends | supports | contrasts | critiques |
      compares | other`.
12. **Grounding rule.** `quote` must be an exact (byte-for-byte) substring of
    `passages[passage_index]`, the whitespace-collapsed text the model was shown. Matching
    is exact after nothing more than trimming the quote's outer whitespace. If
    `passage_index` is out of range or wrong, the other passages are searched; a unique hit
    is accepted and re-indexed. Rejection codes:
    - `empty_quote`;
    - `quote_too_short`: fewer than 3 alphabetic words, so a bare marker like `[12]` is
      rejected;
    - `quote_not_in_passage`;
    - `invalid_relation`;
    - `empty_reason`;
    - `reason_too_long`: over 600 chars.

    A rejected label is never written as a claim, but its raw output stays cached, so a
    rerun does not pay for it again and reports it as rejected again.
13. **Cache.** The key is `sha256(json{prompt_version: LABEL_PROMPT_VERSION, citing_title,
    cited, passages})` hex. It is stored as `source_record(kind='relationship_label',
    source_key='<model_key>|<hash>', payload=<raw draft JSON>)`. A cache hit skips the model
    call. `relationship_claim` gains `source_record_id`, and a unique
    `(assertion_id, source_record_id)` makes the claim insert idempotent. The `model_key` is
    `claude:default` for claude (the CLI's configured model; the analyzer passes no
    `--model`) and `codex:<CODEX_PRIMARY_MODEL>` for codex.
14. **Claims are superseded, never destroyed.**
    - Accept and reject set `review_state` and `updated_at`.
    - Edit inserts a new claim (`origin='user'`, `review_state='accepted'`, same
      `assertion_id`/`mention_id`), and the old row gets `superseded_by = new.id`. An
      edited quote must pass the same grounding check against the mention's passage
      (HTTP 422 `quote_not_in_passage`).
    - The API returns only claims whose `superseded_by` is null. Rejected claims are
      returned so the UI can show them dimmed.
15. **Labeling runs only when asked.** `--labels` implies `--local`. Edges are ordered by
    mention count (descending), then by first-mention offset. Calls run 2 at a time and stop
    after `--label-limit` (default 25) *model calls*; cache hits do not count. Each sidecar
    call gets its own directory, `<data>/labels/<hash[..16]>/`, because `reader_tool`
    writes `output.schema.json`/`agent-output.json` into its directory, and concurrent calls
    would otherwise collide.
16. **`--local` builds the reading index if needed.** It uses `AppState::reading_index(id,
    false)`. This is a rebuildable cache artifact. Ramp A decision 7 ("lit never writes
    artifacts") still holds for plain `lit`.
17. **A paper with no Work skips local evidence.** If `lit` ends `Unmatched`, local evidence
    is skipped with the warning `local_evidence_requires_work`. No local-only citing Work is
    invented in this ramp.
18. Contact email and network behavior are unchanged from Ramp A. Unit tests never touch
    the network or the real `claude`/`codex` binaries.

## Design

### Schema

`0004_local_evidence.sql` (U3):

```sql
ALTER TABLE graph.work ADD COLUMN title_key text;
CREATE INDEX work_title_key ON graph.work (title_key) WHERE title_key IS NOT NULL;

ALTER TABLE source.bibliography_entry
    ADD COLUMN ordinal integer CHECK (ordinal >= 0),
    ADD COLUMN printed_key text,
    ADD COLUMN fields jsonb NOT NULL DEFAULT '{}'::jsonb,   -- BibliographicFields
    ADD COLUMN page integer,
    ADD COLUMN text_start integer,
    ADD COLUMN text_end integer,
    ADD COLUMN member_spans jsonb,                           -- [{start,end}]
    ADD COLUMN region jsonb;                                 -- {x_min,y_min,x_max,y_max}
CREATE INDEX bibliography_entry_record ON source.bibliography_entry (source_record_id);

ALTER TABLE graph.citation_mention
    ADD COLUMN entry_id uuid REFERENCES source.bibliography_entry(id) ON DELETE CASCADE,
    ADD COLUMN style text CHECK (style IN ('numeric', 'superscript', 'author_year')),
    ADD COLUMN marker_text text,
    ADD COLUMN page integer,
    ADD COLUMN marker_start integer,
    ADD COLUMN marker_end integer,
    ADD COLUMN passage_start integer,
    ADD COLUMN passage_end integer,
    ADD COLUMN rects jsonb;
-- quote_or_anchor holds the whitespace-collapsed passage; text_hash = sha256(passage);
-- page_or_locator = 'p.<page>'.
CREATE UNIQUE INDEX citation_mention_occurrence
    ON graph.citation_mention (entry_id, marker_start, page) WHERE entry_id IS NOT NULL;
CREATE INDEX citation_mention_entry ON graph.citation_mention (entry_id);
CREATE INDEX citation_assertion_entry ON graph.citation_assertion (entry_id)
    WHERE entry_id IS NOT NULL;
```

`0005_relationship_claim_labels.sql` (U5):

```sql
ALTER TABLE graph.relationship_claim
    ADD COLUMN quote text,
    ADD COLUMN model text,
    ADD COLUMN source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    ADD COLUMN superseded_by uuid REFERENCES graph.relationship_claim(id) ON DELETE SET NULL,
    ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();
CREATE UNIQUE INDEX relationship_claim_generation
    ON graph.relationship_claim (assertion_id, source_record_id)
    WHERE source_record_id IS NOT NULL;
CREATE INDEX relationship_claim_current
    ON graph.relationship_claim (assertion_id) WHERE superseded_by IS NULL;
```

`relationship_claim.label` holds the `relation` code and `explanation` holds the reason.

New `source_record` kinds: `local_bibliography` (decision 5) and `relationship_label`
(decision 13).

### Modules

```
lysilogy-core
  names.rs            + citation_family, looks_like_person_name
  id.rs               + RelationshipClaimId ('r')
  state.rs            + CitationStyle {numeric, superscript, author_year},
                        ResolutionMethod {identifier, title_match, local_stub, merge}
lysilogy-ingest
  bibliography/mod.rs      SourceText & friends, Bibliography, extract(), pub use
  bibliography/source.rs   UTF-16 <-> byte map, token lookup, anchors (port of `Source`)
  bibliography/entries.rs  heading detection, entry labels, split_entries, regions (port)
  bibliography/fields.rs   BibliographicFields, parse_fields, DOI/arXiv readers (port)
  bibliography/mentions.rs numeric/superscript/author-year detection, Resolver (port)
  local_evidence.rs        persist extraction, resolve entries, assertions, mentions
  labels.rs                RelationshipLabeler, LabelRequest/LabelDraft, ground_quote,
                           label_prompt, LABEL_SCHEMA, ScriptedLabeler, label_edges
lysilogy-db
  evidence.rs (new)        EvidenceRepository: entries, mentions, resolutions, edge mentions
  relationship.rs (new)    RelationshipRepository: claims insert/accept/reject/supersede/query
  work.rs                  + title_key upkeep, find_by_title_key, backfill_title_keys,
                             merge_work, resolve_redirect
  citation.rs              + add_entry_assertion, edge_sources
  document.rs              + version_for_sha
root
  src/literature/evidence.rs  source_text_from_index, AppState::run_local_evidence
  src/labeling.rs             SidecarLabeler (claude | codex) over LocalCliAnalyzer
  src/analysis/local_cli.rs   + pub(crate) structured_model_key(provider)
  src/main.rs                 Lit flags --local --labels --sidecar --label-limit
  src/api.rs, src/api/literature.rs, src/api/labels.rs (new)
web
  types.ts, lib/api.ts, components/LiteratureGraph.tsx(+css), App.tsx,
  components/PdfReader.tsx, components/PdfSourceTools.tsx (focus span)
```

Workspace deps to add: `regex = "1.12.3"` (`1.13.1` is in the local cargo cache) and
`unicode-segmentation = "1.12.0"`, both used by `lysilogy-ingest`. `unicode-normalization` and
`sha2` are already workspace deps.

### Pure extraction API (`lysilogy_ingest::bibliography`)

```rust
pub struct TextSpan { pub start: usize, pub end: usize }            // UTF-16
pub struct PageRect { pub x_min: f32, pub y_min: f32, pub x_max: f32, pub y_max: f32 }
pub struct SourceToken { pub start: usize, pub end: usize, pub page: u32,
                         pub rects: Vec<PageRect>, pub text: String }
pub struct SourcePage { pub number: u32, pub start: usize, pub end: usize }
pub struct SourceParagraph { pub start: usize, pub end: usize, pub kind: String,
                             pub spans: Vec<TextSpan> }
pub struct SourceText { pub text: String, pub pages: Vec<SourcePage>,
                        pub tokens: Vec<SourceToken>, pub paragraphs: Vec<SourceParagraph>,
                        pub sentences: Vec<TextSpan> }

pub enum FieldConfidence { Missing, Heuristic, Explicit }
pub struct ParsedField<T> { pub value: Option<T>, pub confidence: FieldConfidence }
pub struct BibliographicFields { printed_key, authors, title, year, venue, doi, arxiv_id }

pub struct ExtractedEntry {
    pub ordinal: usize,                 // 0-based order in the bibliography
    pub raw_text: String,               // members trimmed, joined by '\n' (as old entry_text)
    pub fields: BibliographicFields,
    pub page: u32,
    pub span: TextSpan,                 // first member start .. last member end
    pub member_spans: Vec<TextSpan>,
    pub region: Option<PageRect>,       // union of token rects on `page`
}
pub struct ExtractedMention {
    pub entry_ordinal: usize,
    pub style: CitationStyle,           // lysilogy_core::CitationStyle
    pub marker: TextSpan,
    pub marker_text: String,
    pub page: u32,
    pub rects: Vec<PageRect>,
    pub passage: TextSpan,
    pub passage_text: String,           // whitespace-collapsed
}
pub enum UnresolvedReason { MissingTarget, AmbiguousTarget, UnsupportedRange, AmbiguousMarker }
pub struct UnresolvedMention { pub text: String, pub key: String, pub style: CitationStyle,
    pub reason: UnresolvedReason, pub candidate_ordinals: Vec<usize>, pub marker: TextSpan,
    pub page: u32 }
pub struct Bibliography { pub entries: Vec<ExtractedEntry>,
    pub mentions: Vec<ExtractedMention>, pub unresolved: Vec<UnresolvedMention> }

pub const EXTRACTOR_VERSION: &str = "bib-v1";
pub fn extract(source: &SourceText) -> Bibliography;
pub fn parse_fields(raw: &str, printed_key: Option<String>) -> BibliographicFields;
```

Everything is `Serialize + Deserialize + Clone + Debug`, and `PartialEq` where it is
derivable. A mention that the old code split per page (a marker crossing a page break) stays
one `ExtractedMention` per page.

### Local evidence flow (`lysilogy_ingest::local_evidence`)

```rust
pub struct LocalEvidenceContext<'a> { pub works: &'a WorkRepository,
    pub citations: &'a CitationRepository, pub records: &'a SourceRecordRepository,
    pub evidence: &'a EvidenceRepository }
pub struct LocalEvidenceInput<'a> { pub citing_work: WorkId,
    pub document_version: DocumentVersionId, pub document_sha256: &'a str,
    pub index_schema_version: u16, pub index_text_sha256: &'a str,
    pub bibliography: &'a Bibliography }
pub struct ResolutionCounts { pub identifier: usize, pub title_match: usize,
    pub local_stub: usize, pub reused: usize }
pub struct LocalEvidenceSummary {
    pub entries: usize, pub mentions: usize, pub unresolved_mentions: usize,
    pub resolved_entries: usize, pub resolutions: ResolutionCounts,
    pub ambiguous_entries: usize, pub merged_stubs: usize, pub local_edges: usize,
    pub edges_also_from_provider: usize, pub warnings: Vec<String>,
    pub labels: Option<LabelSummary>,         // filled by the label pass
}
pub async fn run_local_evidence(ctx, input) -> Result<LocalEvidenceSummary>;
```

The steps:

1. `backfill_title_keys`.
2. Upsert the `local_bibliography` record, then insert the entries if new.
3. Resolve each entry (decision 7) and add its local assertion.
4. Insert mentions with `on conflict do nothing` on `citation_mention_occurrence`.

`edges_also_from_provider` counts local edges whose `(citing, cited)` pair also has a provider
assertion. It measures how far the local and provider graphs converge.

### Labeling (`lysilogy_ingest::labels`)

```rust
pub type LabelFuture<'a> = Pin<Box<dyn Future<Output = Result<LabelDraft, LabelerError>> + Send + 'a>>;
pub trait RelationshipLabeler: Send + Sync {
    fn model_key(&self) -> &str;
    fn label<'a>(&'a self, request: &'a LabelRequest) -> LabelFuture<'a>;
}
pub struct CitedWorkBrief { pub title: Option<String>, pub year: Option<i32>,
    pub authors: Vec<String>, pub entry_text: Option<String> }
pub struct LabelRequest { pub citing_title: Option<String>, pub cited: CitedWorkBrief,
    pub passages: Vec<String> }
#[derive(Deserialize, Serialize)]
pub struct LabelDraft { pub relation: String, pub reason: String, pub quote: String,
    pub passage_index: usize }
pub enum LabelerError { Unavailable(String), Failed(String), Malformed(String) }
pub struct GroundedQuote { pub passage_index: usize, pub start: usize, pub end: usize }
pub enum GroundingRejection { EmptyQuote, QuoteTooShort, QuoteNotInPassage,
    InvalidRelation, EmptyReason, ReasonTooLong }        // as_str() snake_case
pub const LABEL_PROMPT_VERSION: u32 = 1;
pub const RELATIONS: &[&str] = &["background", "uses_method", …, "other"];
pub const LABEL_SCHEMA: &str = r#"{ … strict JSON schema … }"#;
pub fn label_prompt(request: &LabelRequest) -> String;
pub fn label_cache_key(request: &LabelRequest) -> String;   // sha256 hex
pub fn ground_quote(passages: &[String], draft: &LabelDraft)
    -> Result<GroundedQuote, GroundingRejection>;
pub struct LabelOptions { pub limit: usize, pub concurrency: usize }
pub struct LabelSummary { pub edges_considered: usize, pub model_calls: usize,
    pub cache_hits: usize, pub labeled: usize, pub rejected_for_grounding: usize,
    pub rejections: BTreeMap<String, usize>, pub failed: usize, pub over_limit: usize,
    pub already_labeled: usize }
pub async fn label_edges(ctx: &LabelContext<'_>, citing_work: WorkId,
    document_version: DocumentVersionId, labeler: &dyn RelationshipLabeler,
    options: LabelOptions) -> Result<LabelSummary>;
pub struct ScriptedLabeler { … }   // fake: closure(&LabelRequest) -> Result<LabelDraft>, call counter
```

The prompt states the task in 1–2 sentences. It numbers the passages
`[0] …`, `[1] …`, gives the cited work's brief, and requires the quote to be copied
**verbatim** from one numbered passage. It stays fixed-format so the cache key is stable.

### API

| Route | Behavior |
| --- | --- |
| `GET /api/works/{id}/citations` | Additive `edges: GraphEdge[]` for both directions. `work` follows redirects. |
| `GET /api/graph/neighbors/{id}` | `GraphEdge` gains the fields below. |
| `POST /api/relationship-claims/{id}/accept` / `/reject` | 200 with the updated `EdgeLabel`. 400 malformed ID, 404 unknown, 409 `claim_superseded`, 503 no graph. |
| `PUT /api/relationship-claims/{id}` | Body `{reason, relation?, quote?}`. Creates the superseding user claim and returns it. 422 `quote_not_in_passage` / `invalid_relation` / `empty_reason`. |
| `POST /api/papers/{id}/lit` | Optional JSON body `{local?: bool, labels?: "claude"\|"codex", label_limit?: n}`. An empty body keeps the Ramp A behavior. `Succeeded` carries `{summary, local?: LocalEvidenceSummary}`. |

```ts
GraphEdge = { citing, cited, sources: ("provider" | "local")[], mention_count: number,
              labels: EdgeLabel[] }
EdgeLabel = { id, relation, reason, quote: string | null, origin: "ai" | "user" | "imported",
              review_state: "unreviewed" | "accepted" | "rejected" | "disputed",
              model: string | null, created_at,
              mention: { id, paper_id: string | null, page, passage_start, passage_end,
                         marker_start, marker_end, passage } | null }
```

`paper_id` comes from mention → `document_version` → `document.relative_path` →
`PaperId::from_relative_path`, and only when that paper is in the catalog. No path is
exposed.

### UI

- **Edges.** In the `LiteratureGraph` fanout, an edge with at least one non-rejected label
  is drawn solid with `--signal`; local-only edges are dashed; provider-only edges keep the
  current style. A node whose edge is labeled gets a small mark.
- **Work panel.** The panel for a neighbor shows a "Why it is cited" section for the
  center → node edge (and "Why it cites this" for incoming edges). Each label shows:
  - a relation chip and the reason;
  - the quote as a `<blockquote>`, with an "Unreviewed · claude" or "Edited" caption;
  - buttons **Show in paper** (`g`), **Accept** (`a`), **Reject** (`x`) and **Edit** (`e`).

  Edit opens an inline textarea for the reason and an optional quote field; `Enter` saves
  and `Esc` cancels (one layer, through the existing close chain). Rejected labels are dimmed
  under a "Rejected" disclosure.
- **Jump to the sentence.** `onOpenPassage({paperId, page, start, end, passage})` goes up
  to `App`. App closes the overlay, selects the paper if it differs, sets the page, and
  passes `sourceFocus = {page, start, end, passage, nonce}` to `PdfReader` →
  `usePdfSourceTools` as a new `focus` option. Once the index loads, the hook checks that
  the whitespace-collapsed `index.text.slice(start, end)` equals `passage`. If not, it falls
  back to the first `indexOf` of the passage's first 60 chars in the collapsed text, mapped
  back to an offset. It then does what search does: `setMatches([span])`, `setCurrent(0)`,
  `land(index, span)`. The sentence renders with the existing `current` mark, and the page
  scrolls to it.
- **Help overlay entries.** `a/x/e` (label review in the Work panel) and `g` (show the
  citing sentence).

## Gate (every unit)

```sh
export LYSILOGY_TEST_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy_test
cargo clippy --workspace --all-targets -- -D warnings   # workspace lints: all+pedantic+nursery = deny
cargo test --workspace                                  # DB tests must run, not print "skipping"
# frontend units also: (cd web && npm run typecheck && npm run lint && npm run build)
```

- `web/node_modules` is a symlink to `~/Projects/Lysilogy/web/node_modules`.
- Tests are named `should_<behavior>_when_<condition>`.
- DB tests use the existing `test_support::lock()` / `pool()` pattern with unique
  per-test markers.
- Bounded integer conversions (`u32` page ↔ `i32` column, `usize` ordinal ↔ `i32`) use
  plain `as` casts at the call site, with `#[allow(clippy::cast_possible_truncation,
  clippy::cast_possible_wrap, clippy::cast_sign_loss)]` on the narrowest item if clippy
  objects. No new fallible conversion paths.
- Dev DB check: `scripts/db status`.

## Implementation units

### U1. Port bibliography entry extraction and field parsing (pure)

Files:
- `Cargo.toml`: workspace deps `regex`, `unicode-segmentation`.
- `crates/lysilogy-ingest/Cargo.toml`: add `regex`, `unicode-segmentation` and
  `unicode-normalization` (all `.workspace = true`).
- `crates/lysilogy-ingest/src/lib.rs` (`pub mod bibliography`).
- `crates/lysilogy-ingest/src/bibliography/{mod.rs, source.rs, entries.rs, fields.rs}`.
- `crates/lysilogy-core/src/{names.rs, state.rs, lib.rs}`.

Port these pieces from `git show feat/e1.2-bibliography:src/objects/bibliography.rs`. Read
it from the old worktree and never edit there.

- The `pattern!` macro, `label_regex`, `year_regex`, `clean`, `trimmed`, `key` and
  `Source` go to `source.rs`. `Source` is rebuilt over `&SourceText` and keeps the UTF-16
  ↔ byte map. `anchor()` returns `(page, TextSpan)`.
- `extract`'s heading/appendix detection, `entry_labels`, `starts_new_line`,
  `dotted_sequence_starts`, `split_entries`, `entry_text`, `starts_mixed_unnumbered_entry`,
  `union` and the old `object` go to `entries.rs`. `object` becomes `fn
  extracted_entry(source, entry, ordinal) -> ExtractedEntry`.
- `FieldConfidence`, `ParsedField`, `BibliographicFields`, `author_year_prefix`,
  `sentence_boundary`, `parse_fields`, `split_authors`, `title_venue`, `clean_venue`,
  `extract_doi`, `read_doi`, `trim_identifier` and `extract_arxiv` go to `fields.rs`.
  `parse_fields` is `pub`.
- `mod.rs` holds the public types from Design (all of them, including the mention types,
  so U2 adds no type churn) and `pub fn extract(source: &SourceText) -> Bibliography`. In
  this unit it returns entries only, with `mentions` and `unresolved` empty.

Core:
- `CitationStyle { Numeric => "numeric", Superscript => "superscript", AuthorYear =>
  "author_year" }` and `ResolutionMethod { Identifier => "identifier", TitleMatch =>
  "title_match", LocalStub => "local_stub", Merge => "merge" }` via `string_enum!`.
  Re-export both.
- `names::citation_family` and `names::looks_like_person_name` per decision 2. Replace the
  two `parse_name` call sites with them.

Tests:
- Port the old fixture builder into `bibliography/mod.rs` tests as `fn fixture(blocks:
  &[(&str, &str, u32)]) -> SourceText`, with the same geometry.
- Port every old test that asserts on entries or fields only, adapting `fields(entry)` to
  `entry.fields` and `member_anchors` to `member_spans`:
  - `should_keep_printed_conventions_when_entries_use_dots_parentheses_or_keys`
  - `should_split_author_year_entries_when_no_printed_keys_exist`
  - `should_extract_a_doi_when_it_wraps_across_lines`
  - `should_preserve_identifiers_when_arxiv_versions_and_legacy_ids_are_printed`
  - `should_retain_raw_members_when_a_keyed_entry_wraps_across_pages`
  - `should_keep_field_confidence_separate_when_only_some_metadata_is_present`
  - `should_exclude_floating_captions_when_bibliography_members_are_disjoint`
  - `should_leave_the_year_missing_when_only_an_identifier_contains_four_digits`
  - `should_withhold_identifier_fields_when_an_entry_prints_conflicting_identifiers`
  - `should_keep_the_doi_boundary_when_the_next_line_is_following_prose`
  - `should_join_unnumbered_continuations_when_a_later_plain_year_starts_the_next_entry`
  - `should_preserve_the_next_entry_when_a_block_starts_with_a_continuation`
  - `should_retain_heading_classified_members_when_a_numbered_entry_wraps_before_its_year`
  - `should_split_merged_paragraph_entries_when_printed_keys_begin_distinct_physical_lines`
  - `should_keep_inline_keys_inside_the_entry_when_line_geometry_is_missing_or_raised`
  - `should_split_printed_entries_when_a_page_transition_has_no_word_rectangles`
  - `should_retain_wrapped_publication_years_when_bracketed_entries_continue_on_new_lines`
  - `should_preserve_four_digit_keys_when_explicit_brackets_or_an_adjacent_sequence_support_them`
    (entry part)
  - `should_preserve_terminal_title_punctuation_when_the_venue_follows_a_question_or_exclamation`
  - `should_retain_mixed_author_year_entries_when_an_explicit_author_follows_a_complete_numbered_entry`
  - `should_recognize_year_shaped_first_keys_when_lookahead_confirms_an_independent_dotted_sequence`
    (entry part)
  - `should_withhold_truncated_arxiv_dois_when_a_line_break_leaves_an_incomplete_structured_suffix`

  For tests that also assert mentions, port only the entry assertions here; U2 restores the
  mention assertions.
- New tests:
  - `should_return_no_entries_when_no_references_heading_exists`
  - `should_return_no_entries_when_source_text_is_empty`
  - `should_ignore_offsets_when_spans_fall_inside_a_surrogate_pair` (an astral char at a
    span boundary must not panic)
  - `should_skip_paragraph_spans_when_offsets_exceed_text_length`
  - `should_assign_ordinals_in_bibliography_order_when_entries_span_pages`
  - `should_report_region_on_first_page_when_entry_wraps`
  - names: `should_return_family_before_comma_when_author_is_inverted`,
    `should_keep_particles_when_family_has_van_der`,
    `should_return_last_word_when_author_is_given_family_order` (`John Smith` → `Smith`),
    `should_return_none_when_author_has_digits_or_no_letters`

Acceptance:
- The gate is green.
- `extract` is pure: no I/O, no env.
- The old worktree is unchanged (`git -C .worktrees/feat/e1.2-bibliography status` is clean
  of new changes).
- The plan's decision 2 behavior change is covered by a test.

### U2. Port in-text mention detection and the reading-index adapter

Files:
- `crates/lysilogy-ingest/src/bibliography/{mentions.rs, mod.rs}`.
- `src/literature.rs` → add `mod evidence;`.
- `src/literature/evidence.rs` (new; adapter only in this unit).

Mention detection in `mentions.rs`:
- Port `resolve_mentions`, `citation_cue`, `resolve_superscripts`, `citation_keys`,
  `sentences` (with the `unicode-segmentation` fallback) and `Resolver`. Use
  `lysilogy_core::names::citation_family` for the author-year keys.
- `Resolver::resolve_marker` emits an `ExtractedMention { entry_ordinal, … }` for a unique
  target, or an `UnresolvedMention` otherwise. The reasons and the `seen` dedupe are as in
  the old code.
- The passage is the old `sentence` context, clipped per decision 4 by a
  `fn clip_passage(source, marker, passage) -> TextSpan`. `passage_text` =
  `clean(source.text(passage))`.
- `extract` now calls the mention pass after entry splitting.

Root adapter: `pub(crate) fn source_text_from_index(index: &ReadingIndex) -> SourceText`
maps text, pages, tokens (the rects become `PageRect`), `objects.paragraph` and
`objects.sentence`. It copies only and never re-derives anything.

Tests:
- Restore the mention halves of the U1-split tests.
- Port:
  - `should_split_numbered_entries_when_keys_are_bracketed`
  - `should_leave_a_marker_unresolved_when_two_entries_match`
  - `should_preserve_each_occurrence_when_citations_share_a_sentence_with_astral_text`
  - `should_require_raised_geometry_when_a_bare_number_is_used_as_a_citation`
  - `should_resolve_appendix_mentions_when_the_appendix_follows_the_bibliography`
  - `should_withhold_links_when_no_bibliography_or_a_range_is_unsupported`
  - `should_preserve_family_particles_when_name_parsing_agrees_on_the_printed_family`
    (drop the `John Smith` → `None` assertion)
  - `should_keep_sentence_context_when_an_abbreviation_crosses_segmenter_boundaries`
  - `should_resolve_each_printed_marker_once_when_keys_resemble_author_year_citations`
  - `should_withhold_incompatible_number_markers_when_brackets_establish_the_citation_convention`
  - `should_resolve_a_pure_superscript_citation_when_no_competing_delimited_convention_exists`
- New tests:
  - `should_expand_numeric_range_when_marker_is_bracketed_range` (`[3-5]` → 3 mentions)
  - `should_resolve_author_year_when_et_al_and_comma_year` (`Smith et al., 2020`)
  - `should_resolve_given_family_entries_when_bibliography_prints_full_names`
    (`Ashish Vaswani, Noam Shazeer, … 2017` cited as `Vaswani et al. (2017)`)
  - `should_clip_passage_when_sentence_context_exceeds_limit`
  - `should_collapse_whitespace_when_building_passage_text`
  - `should_ignore_markers_inside_the_bibliography_when_entries_cite_each_other`
  - `should_not_treat_equation_or_figure_numbers_as_citations_when_prefixed_by_eq_or_fig`
  - `should_return_no_mentions_when_bibliography_is_empty`
- Root: `should_copy_offsets_and_rects_when_converting_reading_index`, using
  `source_index::test_support` fixtures.

Acceptance:
- The gate is green.
- Run a one-off probe locally and do not commit it: an `#[ignore]` test or a scratch
  binary under `$CLAUDE_JOB_DIR/tmp` that loads a cached `reading-index.json` from
  `~/Projects/Lysilogy/.lysilogy/papers/*/` and prints entry and mention counts. Run it on
  Ziegler 2019 (author-year) and Stiennon 2020 (numeric).
- Record the counts in the unit's commit body. Gross failures (0 entries on a paper with a
  References heading) are fixed in this unit.

### U3. Schema and repositories for local evidence and merges

Files:
- `crates/lysilogy-db/migrations/0004_local_evidence.sql`.
- `crates/lysilogy-db/src/{lib.rs, evidence.rs (new), work.rs, citation.rs, document.rs}`.
- `crates/lysilogy-core/src/{id.rs, lib.rs}` (`RelationshipClaimId`, prefix `'r'`, added
  now so that U5/U6 do not touch core).
- `crates/lysilogy-db/Cargo.toml` if `lysilogy-ingest` types are needed. They are not:
  the repositories take plain values.

`EvidenceRepository`:
- `insert_entries(version: DocumentVersionId, record: SourceRecordId, entries:
  &[NewEntry]) -> Result<Vec<EntryId>>`. `NewEntry { entry_key, ordinal: i32, raw_text,
  printed_key, fields: serde_json::Value, page: i32, text_start: i32, text_end: i32,
  member_spans: Value, region: Option<Value> }`. It inserts with `on conflict
  (citing_version_id, entry_key) do nothing`, then selects the IDs in ordinal order.
- `entries_for_record(record) -> Vec<StoredEntry>`.
- `accepted_resolution(entry) -> Option<(WorkId, ResolutionMethod)>`.
- `record_resolution(entry, work, status: ResolutionStatus, method: ResolutionMethod,
  confidence: Option<f64>)` upserts on `(entry_id, candidate_work_id)`.
- `withdraw_accepted(entry)`.
- `insert_mention(NewMention { assertion, version, entry, style, marker_text, page,
  marker_start, marker_end, passage_start, passage_end, passage, rects }) ->
  Option<MentionId>` uses `on conflict do nothing` and sets
  `text_hash = sha256(passage)` and `page_or_locator = 'p.<page>'`.
- `mentions_for_edge(citing, cited, version) -> Vec<StoredMention>`, ordered by
  `passage_start`.

`WorkRepository`:
- `set_display_title`/`fill_display_title_if_unknown` also write
  `title_key = lysilogy_core::title_key(title)` (NULL when empty).
- `find_by_title_key(key, exclude: Option<WorkId>) -> Vec<Work>` excludes redirected Works.
- `backfill_title_keys() -> usize`.
- `merge_work(from, into, reason, actor) -> Result<()>` per decision 8.
- `resolve_redirect(id) -> WorkId`.
- `is_auto_mergeable_stub(id) -> bool`: every accepted resolution is `local_stub`, and no
  document points at it.

Other repositories:
- `CitationRepository::add_entry_assertion(citing, cited, entry) -> AssertionId` inserts with
  status `asserted` and is idempotent on `citation_assertion_entry_key`.
- `CitationRepository::edge_sources(pairs: &[(WorkId, WorkId)]) -> HashMap<(WorkId, WorkId),
  EdgeSources { provider: bool, local: bool, mention_count: usize }>` runs one query
  grouping assertions and mentions.
- `DocumentRepository::version_for_sha(document, sha256) -> Option<DocumentVersionId>`.

Tests (DB-gated):
- `should_not_duplicate_entries_when_inserted_twice`
- `should_keep_one_accepted_resolution_when_entry_resolved_twice`
- `should_store_mention_once_when_same_occurrence_inserted_twice`
- `should_write_title_key_when_display_title_is_set`
- `should_backfill_title_keys_when_titles_predate_the_column`
- `should_find_work_by_title_key_when_not_redirected`
- `should_repoint_assertions_and_record_decision_when_merging_stub`
- `should_withdraw_stub_resolution_when_merged`
- `should_refuse_merge_when_it_would_create_a_self_citation`
- `should_refuse_merge_when_source_already_redirects`
- `should_follow_redirect_chain_when_resolving_work`
- `should_report_provider_and_local_sources_when_edge_has_both`
- `should_count_mentions_per_edge_when_grouping_sources`
- Update the migration test's expected tables and columns if it lists them.

Acceptance:
- The gate is green.
- `0004` applies on top of the dev DB holding Ramp A data: start `lysilogy serve`
  briefly, or run the migrator against `lysilogy`, and see no error.
- Existing Ramp A tests are unchanged.

### U4. Local evidence flow, `lit --local`, and resolution

Files:
- `crates/lysilogy-ingest/src/{local_evidence.rs (new), lib.rs}`.
- `src/literature.rs`, `src/literature/evidence.rs`, `src/main.rs`.

Ingest:
- `run_local_evidence` and `LocalEvidenceSummary` per Design, with resolution per
  decision 7, merges per decision 8, assertions per decision 6 and mentions per decision 10.
- Helpers:
  - `fn resolve_entry(ctx, citing, neighborhood: &[Work], entry: &StoredEntry,
    fields: &BibliographicFields) -> Result<EntryOutcome>`
  - `enum EntryOutcome { Resolved { work, method, created: bool }, Ambiguous(Vec<WorkId>),
    SelfReference }`
  - `fn title_candidates(neighborhood, global, key, year) -> Vec<Work>` (pure, unit-tested)
- Warnings are stable codes: `self_reference:<ordinal>`, `identifier_conflict:<ordinal>`,
  `merge_refused:<ordinal>:<reason>`, `local_evidence_requires_work`.

Root (`src/literature/evidence.rs`):
- `impl AppState { pub async fn run_local_evidence(&self, id: &PaperId, summary:
  &LiteratureSummary) -> Result<Option<LocalEvidenceSummary>> }`. It returns `None` plus the
  warning when the summary has no Work.
- It finds the document version through `version_for_sha` (re-hash the file with
  `hash_file`, as `run_literature_flow` does), gets `self.reading_index(id, false)`, and
  converts it with `source_text_from_index`. It computes `index_text_sha256` with `sha2`,
  calls `bibliography::extract`, then `run_local_evidence`.
- `GraphState` (`src/graph.rs`) gains an `evidence: EvidenceRepository` field; construct it
  where the other repositories are built.

CLI:
- `Command::Lit` gains `#[arg(long)] local: bool`.
- `literature_command` runs the Ramp A flow, then (if `local`) `run_local_evidence`, and
  prints with `render_local_evidence(&LocalEvidenceSummary)`:
  ```
  Local      entries 48 · mentions 112 (unresolved 9)
  Resolved   46 (identifier 5, title 30, stub 11, reused 0) · ambiguous 2 · merged 0
  Edges      local 44 (also provider 31)
  ```

Tests (DB-gated, fixture `SourceText` built with the U1 test fixture helper; make it
`pub(crate)` under `#[cfg(test)]` in `bibliography`):
- `should_create_entries_mentions_and_local_edges_when_paper_has_numbered_bibliography`
- `should_be_idempotent_when_local_evidence_runs_twice`: same entry/mention/assertion
  counts, `reused == resolved`, no new Works.
- `should_resolve_by_doi_when_entry_prints_identifier_of_existing_work`
- `should_reuse_provider_stub_when_entry_title_matches_neighborhood_work`
- `should_mark_ambiguous_when_two_unrelated_works_share_the_title_key`
- `should_reject_title_match_when_years_differ_by_more_than_one`
- `should_create_titled_stub_when_nothing_matches`
- `should_merge_local_stub_when_provider_work_with_same_title_appears_later`: run once,
  then add a provider-style Work with the same title to the citing work's outgoing edges,
  then run again. Assert `merged_stubs == 1`, a redirect row, and the assertion repointed.
- `should_skip_self_reference_when_entry_resolves_to_citing_work`
- `should_store_no_mentions_when_markers_are_unresolved`
- Root (`tempdir` + cached index fixture, DB-gated):
  `should_run_local_evidence_from_cached_reading_index_when_paper_is_linked`, and
  `should_skip_local_evidence_when_paper_has_no_work`.

Acceptance:
- The gate is green.
- Live check on the dev DB: `LYSILOGY_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy
  cargo run -- --library ~/Projects/Lysilogy/local-articles --data
  ~/Projects/Lysilogy/.lysilogy lit "Fine-Tuning Language Models from Human Preferences"
  --local`, and the same for "Learning to Summarize from Human Feedback". Both report
  non-zero entries, mentions and local edges, plus a non-zero "also provider" count.
- A rerun reports 0 new Works.
- Record both outputs in this doc under "U4 implementation notes".

### U5. Relationship labeler, grounding, cache, sidecars, and `lit --labels`

Files:
- `crates/lysilogy-db/migrations/0005_relationship_claim_labels.sql`.
- `crates/lysilogy-db/src/{relationship.rs (new), lib.rs}`.
- `crates/lysilogy-ingest/src/{labels.rs (new), lib.rs}`.
- `src/labeling.rs` (new), `src/lib.rs`, `src/analysis/local_cli.rs`, and `src/graph.rs`
  (`GraphState.relationships: RelationshipRepository`).
- `src/literature/evidence.rs`, `src/main.rs`.

DB, `RelationshipRepository`:
- `insert_ai_claim(assertion, mention: Option<MentionId>, relation, reason, quote, model,
  record: SourceRecordId) -> Option<RelationshipClaimId>` uses `on conflict do nothing` on
  `relationship_claim_generation`.
- `has_current_claim(assertion) -> bool`.
- `set_review_state(claim, ReviewState) -> Result<StoredClaim>` returns `Conflict` when the
  claim is superseded.
- `supersede_with_user_claim(claim, relation, reason, quote) -> Result<StoredClaim>` runs in
  one transaction.
- `current_claims_for_edges(pairs) -> Vec<StoredClaimWithMention>` joins the mention, its
  passage and offsets, and the document `relative_path`.
- `get(claim) -> Option<StoredClaimWithMention>`.

Ingest (`labels.rs`):
- Everything in the Labeling design.
- `label_edges` works as follows:
  1. Load the edges with local mentions for `(citing_work, version)` through
     `mentions_for_edge` (add `EvidenceRepository::local_edges(citing, version) ->
     Vec<(WorkId, Vec<StoredMention>)>`).
  2. Skip edges whose assertion already has a current claim (`already_labeled`).
  3. Build the `LabelRequest` with deduped passages.
  4. Look up the cache record.
  5. On a miss, call the labeler (bounded with `futures`-free manual chunking or a
     `tokio::sync::Semaphore`; ingest already depends on `tokio`), then upsert the
     `relationship_label` record with the raw draft.
  6. Ground the draft. On success, insert the claim with `mention_id` = the first mention
     whose passage is `passages[grounded.passage_index]`. On failure, count it under
     `rejections[code]`.
- Labeler errors increment `failed` and never abort the pass. `over_limit` counts edges
  left once `model_calls == limit`.

Sidecar (`src/labeling.rs`):
- `pub struct SidecarLabeler { analyzer: LocalCliAnalyzer, provider: AnalysisProvider,
  scratch: PathBuf, model_key: String }`.
- `SidecarLabeler::new(provider, analyzer, scratch) -> Result<Self>` rejects
  `AnalysisProvider::Heuristic`.
- The `RelationshipLabeler` impl creates `scratch/<label_cache_key(request)[..16]>/` and
  calls `analyzer.reader_tool::<LabelDraft>(provider, &dir, ReaderToolRequest { schema:
  LABEL_SCHEMA, prompt: &label_prompt(request), live_web: false })`. It maps
  `Error::ProgramUnavailable` → `LabelerError::Unavailable` and other errors → `Failed`.
- Add `pub(crate) fn structured_model_key(provider) -> String` in `local_cli.rs`
  (`claude:default`, `codex:<CODEX_PRIMARY_MODEL>`).

CLI:
- `--labels` (implies `--local`), `--sidecar claude|codex` (a clap `ValueEnum`
  `LabelSidecar`, default `claude`), `--label-limit <usize>` (default 25).
- The labeling scratch directory is `<data>/labels`.
- Output:
  ```
  Labels     edges 44 · calls 20 · cached 0 · labeled 17 · rejected (grounding) 2 · failed 1 · over limit 24
  ```
- The root wiring `AppState::label_local_edges(id, &dyn RelationshipLabeler, options)` sets
  `LocalEvidenceSummary.labels`.

Tests:
- Pure tests:
  - `should_accept_quote_when_exact_substring_of_named_passage`
  - `should_reindex_quote_when_found_uniquely_in_another_passage`
  - `should_reject_quote_when_whitespace_or_case_differs`
  - `should_reject_quote_when_only_a_citation_marker`
  - `should_reject_label_when_relation_not_in_vocabulary`
  - `should_reject_label_when_reason_empty_or_too_long`
  - `should_reject_quote_when_passage_index_out_of_range_and_not_found`
  - `should_keep_cache_key_stable_when_request_is_identical`
  - `should_change_cache_key_when_passages_change`
  - `should_number_passages_when_building_prompt`
  - `should_parse_schema_when_embedded` (`LABEL_SCHEMA` is valid JSON, every property is
    required, `additionalProperties: false`)
- DB-gated tests with `ScriptedLabeler`:
  - `should_write_unreviewed_ai_claim_with_quote_and_mention_when_label_grounded`
  - `should_not_call_labeler_when_cached_output_exists` (call counter 0 on the second run;
    claims not duplicated)
  - `should_count_rejection_and_write_no_claim_when_quote_not_in_passage`
  - `should_continue_when_labeler_fails_for_one_edge`
  - `should_stop_calling_when_label_limit_reached`
  - `should_supersede_claim_when_user_edits_label`
  - `should_refuse_review_when_claim_superseded`
- Sidecar test following the existing `local_cli.rs` fake-program style: a `tempdir` shell
  script named `claude` that echoes `{"structured_output": {...}}`, via
  `LocalCliAnalyzer::with_commands`. Name it
  `should_parse_label_draft_when_claude_sidecar_returns_structured_output`. Also add
  `should_report_unavailable_when_sidecar_program_missing`.

Acceptance and live smoke:
- The gate is green.
- Run on the dev DB: `lit "Fine-Tuning Language Models from Human Preferences" --labels
  --sidecar claude --label-limit 15`, and the same for "Learning to Summarize from Human
  Feedback". If `codex` works, also run `--sidecar codex --label-limit 8` on one of them.
- Hand-check at least 5 claims in `scripts/db psql` for plausibility, and check that the
  quote appears in the PDF sentence.
- A rerun of the same command makes 0 model calls.
- Record in this doc under "Live smoke (labels)": entries extracted, mentions, unresolved,
  resolved by method, local edges, labeled, rejected for grounding (by code), failed, and
  wall time.

### U6. API and UI: labeled edges, review endpoints, jump to sentence, report

Files:
- `src/api.rs` (`GraphEdge`, `CitationsResponse.edges`, `fetch_work` follows redirects),
  `src/api/labels.rs` (new; routes merged in `build_router`), `src/api/literature.rs`
  (optional POST body; job runs local evidence and labels; `Succeeded` carries `local`).
- `web/src/types.ts`, `web/src/lib/api.ts` (`acceptLabel`, `rejectLabel`, `editLabel`;
  `startLiterature(id, options?)`), `web/src/components/LiteratureGraph.tsx`,
  `web/src/components/LiteratureGraph.css`.
- `web/src/App.tsx` (`sourceFocus` state, `onOpenPassage`, close chain for label edit),
  `web/src/components/PdfReader.tsx` (`sourceFocus` prop),
  `web/src/components/PdfSourceTools.tsx` (`focus` option),
  `web/src/components/HelpOverlay.tsx`.
- `docs/ramps/ramp-b.md` (Result + Report sections).

API:
- `edge_payloads(state, pairs) -> Vec<GraphEdge>` combines `edge_sources` and
  `current_claims_for_edges`. It maps `relative_path` to `paper_id` only for catalog papers.
- The labels handlers reuse `parse_id`-style parsing for `RelationshipClaimId` (400
  `invalid_claim_id`), `graph_or_unavailable`, and `graph_error`.
- Edit validation reuses `ground_quote` against the claim's mention passage (single-passage
  slice), plus the relation and reason checks.

Rust tests (existing `oneshot` style, DB-gated where needed):
- `should_include_labels_and_sources_in_neighbor_edges_when_local_evidence_exists`
- `should_include_edges_in_citations_response_when_work_has_labels`
- `should_accept_label_when_posting_accept`
- `should_return_404_when_accepting_unknown_claim`
- `should_return_400_when_claim_id_malformed`
- `should_return_409_when_reviewing_superseded_claim`
- `should_return_422_when_edited_quote_not_in_passage`
- `should_supersede_and_return_user_claim_when_editing`
- `should_follow_redirect_when_work_was_merged`
- `should_run_local_evidence_when_lit_body_requests_local` (fake transport plus a cached
  index fixture)
- `should_keep_ramp_a_behavior_when_lit_body_empty`

UI:
- As in Design/UI.
- Add a pure helper `web/src/lib/passageFocus.ts` with `locatePassage(text, start, end,
  passage) → {start, end} | null` (whitespace-collapsed comparison and fallback search).
  Give it a node test `web/scripts/passage-focus.test.mjs` and the npm script
  `test:passage-focus`. Cases: exact match, drifted offsets, astral characters before the
  span, passage absent → null, empty passage → null.

Verification:
- `npm run typecheck`, `npm run lint`, `npm run build`, `npm run test:passage-focus`, and
  `npm run test:literature-layout`. Also `npm run smoke:paragraph-selection`, to confirm
  that Cursor-mode `L` still cycles line numbers.
- Scratch Playwright under `~/.cache/lysilogy-smoke/` (not committed). It imports
  `~/Projects/Lysilogy/web/node_modules/playwright/index.mjs`, serves `web/dist`, and mocks
  `/api/library`, `/api/papers/*`, `/work`, `/lit`, `/reading-index`, neighbors, citations
  and the label endpoints. It asserts that:
  - labeled edges render solid;
  - clicking a node shows the reason and quote;
  - `a` sends accept and the caption updates;
  - `x` moves the label under "Rejected";
  - `e` → edit → `Enter` sends the PUT and shows "Edited";
  - **Show in paper** closes the overlay, selects the citing paper, and renders a
    `.pdf-source-mark.is-current` on the mocked page;
  - `Esc` closes the edit, then the panel, then the overlay.

  Screenshots go to the scratch dir.
- Live smoke: `lysilogy serve` on 127.0.0.1:7393 with the dev DB. `curl` the neighbors of
  the Ziegler Work and show that labeled edges come with `labels[].mention.paper_id`. POST
  accept on one claim and reject on another, then re-GET. In a real browser via the
  Playwright script against the live server, click one label's **Show in paper** and
  screenshot the highlighted sentence.
- Append "Result" and "Report" sections to this doc in the Ramp A format (what shipped, how
  to user-test, smoke results, known gaps).

Acceptance: the Rust gate and frontend checks are green, the Playwright script passes
against mocks and against the live server, and the live smoke is recorded in this doc.
