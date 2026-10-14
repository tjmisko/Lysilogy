# Literature provenance: tracked bibliographic fields, trust-tiered selection, overrides

Status: planned 2026-10-13. Branch `feat/lit-provenance`, based on
`integration/literature-viewer` at `14d8d19` (Ramps A-E merged). Built in parallel with the
"histogram" track (`feat/lit-histogram-view`, frontend only). Conventions follow
[ramps/ramp-e.md](ramps/ramp-e.md): plan, units, then per-unit implementation notes and a
final report appended to this file. Status guide: [literature-viewer-status.md](literature-viewer-status.md).

## What exists (verified at 14d8d19)

- `graph.field_claim` (`0001_init.sql`): `work_id, version_id, field (title|year|author|venue|
  type|abstract|identifier|other), value jsonb, state, source_record_id, locator, review_state
  (unreviewed|accepted|rejected|disputed), created_at`. No origin, extractor or confidence.
  Dev DB: 6,436 claims, all `title`/`year`: `openalex_work_summary` 5,821, `openalex_work` 202,
  `local_bibliography` 209, `source_record_id IS NULL` 204 (the paper's own catalog hints).
- `graph.work.display_title/title_state/display_year/year_state/title_key`. Selection is
  first-writer-wins: `WorkRepository::fill_display_title_if_unknown` /
  `fill_display_year_if_unknown` (`crates/lysilogy-db/src/work.rs:271-301`), called from
  `ingest.rs::hydrate_stub_works` (OpenAlex summaries, only untitled stubs),
  `literature.rs::fill_display_fields` (provider record, then `PaperHints`), and
  `local_evidence.rs::create_stub` (bib-v1, only when a stub is created). `set_display_title/
  set_display_year` bypass claims (tests and fixtures only).
- `add_claim` (tests only) and `add_claim_once` (dedupe on work, field, value, source record).
- bib-v1 (`crates/lysilogy-ingest/src/bibliography/fields.rs`) parses `authors, title, year,
  venue, doi, arxiv_id, printed_key` with `FieldConfidence {Missing, Heuristic, Explicit}`.
  **authors, title, year and venue are only ever `Heuristic` or `Missing`**; only doi, arxiv and
  printed_key are `Explicit`. Entries are stored in `source.bibliography_entry` (`raw_text`,
  `fields` jsonb, `entry_key = "bib-v1:<n>"`, `source_record_id` = the `local_bibliography` record).
- OpenAlex (`crates/lysilogy-ingest/src/openalex.rs`): `work_url` selects
  `id,doi,display_name,publication_year,referenced_works`; `works_batch_url` selects
  `id,doi,display_name,publication_year`. `parse_summary` already reads the first author when
  `authorships` is present (title search only). Cached kinds `openalex_work`,
  `openalex_work_summary`.
- Model sidecar: `LocalCliAnalyzer::reader_tool` (`src/analysis/local_cli.rs:175`) runs
  `PromptStage::Experiment` (codex primary model, effort medium). **Claude gets no `--model`**
  (`structured_model_key` returns `claude:default`). `SidecarLabeler` (`src/labeling.rs`) adapts
  it to `lysilogy_ingest::labels::RelationshipLabeler`; grounding + `source_record` caching
  pattern in `crates/lysilogy-ingest/src/labels.rs`.
- Crossref: a reference-list provider exists in `src/citation_graph/crossref.rs` (root crate,
  `GraphHttp` transport) whose `work()` parses `title`, ordered `author` (given + family) and
  year from `api.crossref.org/works/{doi}`. Not wired to claims. `OpenAlexClient::fetch_json`
  fetches any URL through the shared `JsonTransport`; `ReqwestTransport` adds credentials only
  for `api.openalex.org`.
- Paper's own metadata: `PaperMetadata {title, authors, year}` mixes the file name
  (`library.rs::metadata_from_filename`), the PDF info dict and layout front-matter authors
  (`frontmatter::authors`). `AppState::front_matter_text` gives pages 1-2 cut at the
  references heading.
- `Work` (core) is serialized directly by neighbors, citations, ramps. TS `Work` in
  `web/src/types.ts:406`. The Work panel is `WorkPanel` inside `LiteratureGraph.tsx:1280`.
- Web tests: `node --experimental-strip-types --test scripts/<name>.test.mjs` over pure
  `web/src/lib/*.ts` modules (e.g. `test:ramp`). Theme tokens in `web/src/styles.css`
  (`--evidence` green, `--danger` red, `--signal` accent, `--line`, `--ink*`, `--panel*`).
- ID prefixes in use: `w v i s e k d f l c m r t p`.

## Scope

In:

1. Migration `0009_field_provenance.sql`: `origin`, `extractor`, `confidence`, `entry_id`,
   `retracted_at` on `graph.field_claim` (legacy rows backfilled); `display_authors`,
   `authors_state` on `graph.work`.
2. One pure selection function in `lysilogy-core` and one DB reselect that recomputes
   `display_title`, `display_year`, `display_authors` after every claim write, replacing
   `fill_*_if_unknown`. Venue and DOI are selected on read (no column).
3. Curator overrides for title, year, authors, venue, doi (set / clear) and claim review
   (accept / reject / reset).
4. Harvest: bib-v1 claims (title, year, authors, venue, doi) for **every** resolved entry,
   including entries resolving to existing Works; OpenAlex authorships; Crossref publisher
   records; the paper's own front matter (deterministic). Grounded cheap-model pass (Haiku via
   the claude CLI) over bibliography entries and front matter, cached in `source_record`.
5. API: provenance read, override set/clear, claim review. CLI: `lysilogy provenance
   show|set|clear|reselect`. Neighbors and every `Work` payload gain `display_authors`.
6. UI: `WorkProvenance` component (authors line + Sources dialog with the field x source table,
   green/red agreement, selected cell, editable user column), mounted from the Work panel.
7. Live smoke on the dev DB and a report.

Out: person/author identity resolution (authors stay printed strings), fuzzy title matching,
re-resolving entries when selection changes their Work's title, OpenAlex venue
(`primary_location`), Semantic Scholar, publisher landing-page scraping, a provenance page/route
outside the overlay, changing `graph.work_identifier` from a DOI override (identity is
resolution's job; a DOI override only masks the displayed DOI), multi-user curator attribution.

## Decisions

User decisions (verbatim in substance):

1. **Claims are append-only evidence.** Superseding happens by selection, never by
   overwriting a claim. The only mutations on an existing claim row are curation metadata:
   `review_state` and, for user overrides only, `retracted_at`.
2. **Trust order** (highest first): user override > publisher (publisher site, or
   publisher-deposited registry metadata such as Crossref; registry counts as publisher tier) >
   the paper itself (its own front matter, when we hold the PDF) > OpenAlex > other papers citing
   it (bibliography explicit > bibliography heuristic > bibliography model-extracted).
3. **Origin, extractor, confidence are claim columns** (migration 0009). The tier ranking lives
   in Rust (`lysilogy_core::provenance`), never in the DB.
4. **One pure selection function** recomputes display_title, display_year and the new
   display_authors after every claim insert, replacing `fill_*_if_unknown`. Disagreement across
   tiers is not a conflict (higher tier wins). Disagreement within the winning tier marks the
   field `conflicting`. Normalize before comparing: `title_key` for titles, exact year, ordered
   author family names for authors. `review_state = 'rejected'` claims are excluded.
5. **User overrides can mask any field** (title, year, authors, venue, doi): an override is a
   curator claim that wins selection; clearing it restores the tiered choice.
6. **Harvest during the build.** Authors and year come from the citing paper's bibliography.
   bib-v1 first; only where it leaves authors or year `Missing`/`Heuristic`, a cheap model pass
   (Claude Haiku via the claude CLI sidecar, `--model haiku`). Model fields are grounded: every
   returned value must be a verbatim substring of the entry text shown, else dropped. Raw model
   output cached in `source_record`. Claims are written when an entry resolves to an existing
   Work, not only for new stubs.
7. **Self front matter**: when we hold the paper, its own title/authors/year from its front
   matter are `paper` tier claims (deterministic first; model fallback grounded the same way).
8. **OpenAlex**: authorships (display names, ordered) added to the fetch select; `openalex`
   tier author claims.
9. **Publisher tier**: Crossref by DOI is a cheap, reliable source already present in the
   codebase (`src/citation_graph/crossref.rs` parses it), so it is implemented (Decision 21).
10. **API + UI**: one endpoint returns, for one Work, every field x every source column (claim
    value, origin, extractor, source record, locator, review state) plus the selected value and
    state; endpoints set/clear a user override per field. The provenance view (new
    `WorkProvenance` component, reachable from the Work panel) is a table: rows = fields; columns
    = sources by tier, the editable user column furthest left, then publisher, paper, OpenAlex,
    other citing papers. Cells agreeing with the selected value are green, disagreeing cells red,
    the selected cell visibly indicated, empty sources blank. Keyboard accessible. Existing theme
    tokens.
11. **`display_authors` flows into the neighbors API** per the cross-track contract.

Design decisions (this plan):

12. **Origins** (`ClaimOrigin`, stored text): `user`, `publisher`, `paper`, `openalex`,
    `library_metadata`, `citing_bibliography`. `library_metadata` is the paper's catalog metadata
    (file name, PDF info dict) that today's `PaperHints` claims come from; it is not front-matter
    evidence, so it ranks **below OpenAlex and above citing papers** (see Assumption A3).
    **Confidence** (`ClaimConfidence`): `explicit`, `heuristic`, `model`.
    **Rank** = `(origin trust index, confidence index)`, lexicographic, lower wins:
    origin order `user 0, publisher 1, paper 2, openalex 3, library_metadata 4,
    citing_bibliography 5`; confidence order `explicit 0, heuristic 1, model 2`. Every distinct
    rank is its own tier for conflict purposes, so "bibliography explicit > heuristic > model"
    and "paper deterministic > paper model" fall out of one rule.
13. **Extractor codes** (text, 1..=64 chars): `curator` (user), `crossref-v1`, `frontmatter-v1`,
    `frontmatter-model-v1`, `openalex`, `catalog-metadata`, `bib-v1`, `bib-model-v1`, `legacy`
    (backfill of unknown kinds). The model identity (`claude:haiku`) lives in the cache record
    payload and source key, not in `extractor`.
14. **Fields** (`ProvenanceField`, API spelling): `title` (claim field `title`, JSON string),
    `year` (`year`, JSON integer), `authors` (`author`, JSON array of printed strings in order;
    one claim per source = the whole list), `venue` (`venue`, JSON string), `doi`
    (`identifier`, JSON object `{"scheme":"doi","value":"<normalized>"}`). Rows with other
    `field` values are ignored by provenance. Unparseable values are `excluded: unreadable`.
15. **Normalization and agreement** (pure, core):
    - title, venue: `title_key(value)` equal (empty key never agrees).
    - year: integer equal.
    - doi: `identifier::normalize(Doi, value)` equal.
    - authors: `author_list_key(list) -> AuthorListKey {families: Vec<String>, truncated: bool}`.
      Each printed author: strip a trailing `et al.` / `et al` / `and others` / `others`
      (case-insensitive) and set `truncated`; drop items that are then empty; family =
      `title_key(citation_family(author).unwrap_or(author))`. Two lists agree iff their family
      vectors are equal, or one is `truncated` and its families are a non-empty prefix of the
      other's (Assumption A4).
16. **Selection** (`select_field(field, &[ClaimCandidate]) -> FieldSelection`, pure):
    1. Eligible = not `rejected`, not retracted, value parses for the field.
    2. No eligible claims -> `value None`, `state unknown`, `winner None`.
    3. Winning tier = eligible claims with the minimum rank.
    4. `conflicting` iff some pair in the winning tier does not agree; else `known`.
    5. Winner inside the tier: sort by (support desc, completeness desc, created_at asc, id asc),
       where support = number of tier claims agreeing with it, completeness = author count for a
       non-truncated author list, -1 for a truncated one, 0 for other fields. The winner's value
       is the selected value (also when conflicting, so the graph still has a label).
    6. Agreement for **every** candidate: `excluded(rejected|retracted|unreadable)`, `selected`
       (the winner), `agrees` (agrees with the winner's value), `disagrees`.
17. **Writing the selection** (`reselect_in(conn, work)`): inside a transaction that first locks
    the Work row (`select … for update`), load claims for `title`, `year`, `author`, run
    `select_field`, then per field: a selected value -> write value and state (`title_key` with
    the title); no eligible claim but at least one claim row for the field -> NULL and `unknown`;
    no claim rows at all -> leave the column untouched (keeps fixture values set by
    `set_display_*`). `updated_at = now()` only when something changed. Returns `Reselection
    {title_changed, year_changed, authors_changed, title_became_known}`.
18. **Every claim write reselects.** `ClaimRepository::record(&[NewClaim])` inserts each claim
    once (dedupe on `work_id, field, value, origin, extractor, source_record_id, entry_id`, all
    `is not distinct from`) and reselects each Work that received at least one new row.
    `set_override`, `clear_override`, `set_review_state` and `merge_work` reselect too.
    `reselect_all` exists for the migration backfill (Decision 26).
19. **Overrides.** `set_override(work, field, value)`: one transaction: retract the active user
    claim for that field (`retracted_at = now()`), insert a new claim (`origin user, extractor
    curator, confidence explicit, review_state accepted`, no source record), reselect. A partial
    unique index allows at most one active override per `(work_id, field)`. `clear_override`
    retracts it and reselects; clearing with no active override is a no-op returning `false`.
    Override validation (core `parse_override(field, &Value)`): title/venue trimmed,
    whitespace-collapsed, 1..=500 chars, no control characters; year integer 1400..=2200; authors
    array of 1..=100 strings, each trimmed 1..=200 chars, no control characters; doi string that
    `identifier::normalize(Doi, …)` accepts (resolver prefixes stripped). Codes:
    `invalid_value`, `invalid_field`.
20. **Review.** `set_review_state(claim, accepted|rejected|unreviewed)` on non-user claims (a user
    claim -> 409 `override_not_reviewable`; use clear). Rejected claims stay in the provenance
    view as `excluded`.
21. **Publisher = Crossref.** New `crates/lysilogy-ingest/src/crossref.rs` (not the root
    `citation_graph` module: ingest cannot depend on the root crate). `crossref_work_url(doi)` =
    `https://api.crossref.org/works/<percent-encoded doi>`; `parse_crossref_work(&Value) ->
    CrossrefWork {doi, title, authors, year, venue}` (title = `message.title[0]`; authors =
    ordered `message.author[]` as `given family`, else `name`; year = first of `published`,
    `published-print`, `published-online`, `issued` `date-parts[0][0]`; venue =
    `container-title[0]`). Fetched through `OpenAlexClient::fetch_json` (same transport; no
    credentials are added for this host). Cached as `source_record` kind `crossref_work`, key =
    normalized DOI; a 404 is cached as payload `{"not_found": true}` and never refetched.
    DataCite arXiv DOIs (`10.48550/…`) are skipped (Crossref does not hold them). Fetched for the
    paper's own Work on every `lit` run (one request, cached), and for cited Works with a
    non-arXiv DOI only when `--publisher` / `publisher: true` is given (bounded by
    `PUBLISHER_FETCH_MAX = 200` per run, sequential). Soft errors (404, malformed) -> warning
    `publisher_not_found:<doi>`; transport/5xx -> warning `publisher_fetch_failed:<doi>`, never
    fails the flow.
22. **OpenAlex authorships.** `work_url` and `works_batch_url` append `,authorships` to
    `select`. `OpenAlexSummary` and `OpenAlexWork` gain `authors: Vec<String>` (ordered
    `authorships[].author.display_name`, trimmed, empties skipped). `hydrate_stub_works` changes
    from "untitled stubs" to "every given Work with an OpenAlex ID": a cached summary whose
    payload has no `authorships` key is stale and is refetched once (the upsert then makes the
    next run free); for each summary it records title, year, authors (when non-empty) and doi
    claims (`openalex`, `openalex`, `explicit`, source = summary record). The paper's own Work is
    passed in too. Return value stays "Works whose title became known".
23. **bib-v1 claims for every resolved entry.** After any `EntryOutcome::Resolved` (identifier,
    title match, stub, reused, merged), `record_entry_claims` writes title, year (leading four
    digits of the printed year), authors, venue, doi for the fields that are not `Missing`;
    confidence from the field's `FieldConfidence`; `origin citing_bibliography`, `extractor
    bib-v1`, `source_record_id` = the `local_bibliography` record, `entry_id` = the entry,
    `locator` = `entry_key`. `create_stub` no longer writes claims itself. Ambiguous and
    self-reference outcomes write nothing.
24. **Front matter, deterministic** (`paper` tier, `frontmatter-v1`, `heuristic`):
    `front_matter_fields(front_matter, hints) -> PaperFields {title, authors}` (pure, ingest):
    - title = `hints.title` when `title_key(title)` has >= 3 words and is a substring of
      `title_key(front_matter)`.
    - authors = `hints.authors` when non-empty, every name has >= 2 words (rejects file-name
      surnames), and every name occurs in the whitespace-collapsed front matter.
    - no deterministic year (arXiv stamps and dates on page 1 are version dates; Assumption A6).
    Written by `run_literature_flow` for the linked Work with source record kind
    `paper_front_matter`, key `<document sha256>|frontmatter-v1`, payload `{extractor,
    front_matter_sha256, title, authors}`, locator `front_matter`. `PaperHints` gains
    `authors: &'a [String]`.
25. **Model pass** (`--fields` / API `fields`; never implicit, like `--labels`):
    - Sidecar trait `FieldExtractor` in `crates/lysilogy-ingest/src/field_model.rs`
      (`model_key()`, `extract(&FieldRequest) -> FieldFuture`), root adapter
      `SidecarFieldExtractor` in `src/field_extraction.rs` (pattern of `SidecarLabeler`, scratch
      `<data>/fields/<hash16>/`).
    - Fast model: `ReaderToolRequest` gains `fast: bool`. `fast` -> Claude gets `--model haiku`
      (`CLAUDE_FAST_MODEL = "haiku"`), codex gets `CODEX_FAST_MODEL` with effort `low`; model key
      `claude:haiku` / `codex:<CODEX_FAST_MODEL>` via new `structured_fast_model_key`. All
      existing callers pass `fast: false` (no behavior change).
    - **Bibliography targets**: resolved entries whose bib-v1 `authors` or `year` is `Missing` or
      `Heuristic` (today: every entry, Assumption A5), minus entries whose resolved Work already
      has an eligible claim ranked above `citing_bibliography` for both `author` and `year`
      (the model claim could never be selected or contradict the winner's tier; Assumption A7).
    - Shown text per entry = whitespace-collapsed `raw_text`, cut at 600 chars on a char
      boundary. Batches of `FIELD_BATCH = 20` entries in ordinal order; one call per batch;
      `FieldOptions {limit: 10 calls, concurrency: 2}`; cache hits do not count.
    - **Cache per entry**: kind `bibliography_fields_model`, key
      `<model_key>|v<FIELD_PROMPT_VERSION>|<sha256(shown text)>`, payload = the raw item the model
      returned for that entry (`{index,title,authors,year}`), or `{"omitted": true}` when the
      batch answer had no item for it. Written right after each call (a crash keeps paid answers).
      A rerun finds every entry cached -> 0 calls.
    - **Grounding** (`ground_field_draft(shown, item) -> GroundedFields`, pure): trim each value;
      title kept iff non-empty, <= 500 chars, substring of shown; year kept iff it matches
      `^\d{4}[a-z]?$`, is a substring of shown, leading four digits in 1400..=2200; authors kept iff
      1..=100 non-empty names each a substring of shown (any miss drops the **whole list**,
      since a partial ordered list is wrong). Drop codes counted in the summary:
      `title_not_in_entry`, `year_not_in_entry`, `year_malformed`, `author_not_in_entry`,
      `too_many_authors`, `unknown_index`.
    - Claims: `citing_bibliography`, `bib-model-v1`, `model`, source = the per-entry cache
      record, `entry_id`, locator = `entry_key`.
    - **Front-matter fallback**: when `--fields` is given and the paper's Work has no
      non-model `paper` claim for any of title, authors, year (year: always, see 24): one call,
      kind `paper_fields_model`, shown text = front matter cut at 4,000 chars, same grounding,
      claims `paper`, `frontmatter-model-v1`, `model`, locator `front_matter`.
    - Errors: `FieldExtractorError {Unavailable, Failed, Malformed}`; failures count as `failed`
      and never stop the pass (as labels).
26. **Migration backfill.** SQL classifies legacy claims (Design, Schema). Selection over the
    legacy rows is applied by `lysilogy provenance reselect --all` (Rust, batched by Work ID,
    500 per page), run in each unit's live check and documented in the status guide. GET
    provenance computes selection on read, so it is correct even before the backfill.
27. **Merges.** Before `move_work_content` moves claims, retract `from`'s active override for
    any field where `into` has one (survivor wins, like marks). After moving, `reselect_in(into)`
    inside the merge transaction.
28. **API conventions** as Ramps C-E: path IDs parsed first, then body, then graph availability,
    then existence; Work IDs resolved through redirects; resolved IDs in responses. New ID type
    `FieldClaimId`, prefix `'a'`.
29. **UI keys**: `s` (graph, Work panel open) opens the Sources dialog (`WorkProvenance` owns the
    listener; no edit to `LiteratureGraph`'s key handler). In the dialog: Tab / Shift+Tab move
    through cells' controls (focus trapped), Enter on a user cell edits, Enter saves (Ctrl+Enter
    in the authors textarea), Esc cancels an edit, else closes the dialog. Esc and `q` reach the
    App close chain first: the dialog registers a capture-phase listener for
    `LITERATURE_QUIT_EVENT` on `document` that, while open, closes the dialog and calls
    `preventDefault()` + `stopImmediatePropagation()` so neither the Work panel nor the overlay
    closes on the same press.

## Cross-track contract (histogram track)

- `lysilogy_core::Work` gains `display_authors: Option<Vec<String>>` and `authors_state:
  FieldState`, so every Work in `GraphNeighbors.nodes` carries `display_authors` (JSON
  `string[] | null`).
- `web/src/types.ts` `Work`: add exactly one line, `  display_authors?: string[] | null;`,
  directly after `  year_state: FieldState;`. Nothing else is added to `Work` (`authors_state`
  is read from the provenance view, not from `Work`), so the identical hunk from both branches
  merges cleanly.
- This track owns migration 0009 and all backend/schema changes. The histogram track adds no
  migration.
- Shared files are touched minimally: `LiteratureGraph.tsx` gets one import line and one JSX
  line inside `WorkPanel` (after the `literature-work-meta` paragraph); `LiteratureGraph.css`
  is not touched (styles live in `WorkProvenance.css`); `HelpOverlay.tsx` gets one line after the
  `f (graph, Work open)` line; `package.json` gets `test:provenance` directly after
  `test:relevance`; new TS types and `api.ts` methods are appended at the end of their files.
- The Work panel's authors line is rendered by `WorkProvenance` (this track). The histogram
  track renders authors only inside its own components (Assumption A10).

## Design

### Schema: `0009_field_provenance.sql`

```sql
-- 0009: claim provenance (origin, extractor, confidence), curator overrides, and the
-- selected author list. Tier ranking lives in Rust (lysilogy_core::provenance).

ALTER TABLE graph.field_claim
    ADD COLUMN origin text NOT NULL DEFAULT 'library_metadata'
        CHECK (origin IN ('user', 'publisher', 'paper', 'openalex',
                          'library_metadata', 'citing_bibliography')),
    ADD COLUMN extractor text NOT NULL DEFAULT 'legacy'
        CHECK (char_length(extractor) BETWEEN 1 AND 64),
    ADD COLUMN confidence text NOT NULL DEFAULT 'heuristic'
        CHECK (confidence IN ('explicit', 'heuristic', 'model')),
    ADD COLUMN entry_id uuid REFERENCES source.bibliography_entry(id) ON DELETE SET NULL,
    ADD COLUMN retracted_at timestamptz,
    ADD CONSTRAINT field_claim_retraction_user_only
        CHECK (retracted_at IS NULL OR origin = 'user');

-- Legacy rows: provider records, local bibliographies, and the paper's catalog hints.
UPDATE graph.field_claim c SET origin = 'openalex', extractor = 'openalex', confidence = 'explicit'
  FROM source.source_record s
 WHERE s.id = c.source_record_id AND s.kind IN ('openalex_work', 'openalex_work_summary');
UPDATE graph.field_claim c SET origin = 'citing_bibliography', extractor = 'bib-v1',
       confidence = 'heuristic',
       entry_id = (SELECT e.id FROM source.bibliography_entry e
                     JOIN graph.entry_resolution r ON r.entry_id = e.id AND r.status = 'accepted'
                    WHERE e.source_record_id = c.source_record_id
                      AND r.candidate_work_id = c.work_id
                    ORDER BY e.ordinal NULLS LAST, e.id LIMIT 1)
  FROM source.source_record s
 WHERE s.id = c.source_record_id AND s.kind = 'local_bibliography';
UPDATE graph.field_claim SET extractor = 'catalog-metadata' WHERE source_record_id IS NULL;

-- New writers must classify every claim.
ALTER TABLE graph.field_claim
    ALTER COLUMN origin DROP DEFAULT,
    ALTER COLUMN extractor DROP DEFAULT,
    ALTER COLUMN confidence DROP DEFAULT;

CREATE INDEX field_claim_entry ON graph.field_claim (entry_id) WHERE entry_id IS NOT NULL;
-- At most one active curator override per Work field.
CREATE UNIQUE INDEX field_claim_active_override ON graph.field_claim (work_id, field)
    WHERE origin = 'user' AND retracted_at IS NULL;

ALTER TABLE graph.work
    ADD COLUMN display_authors text[] CHECK (display_authors IS NULL OR cardinality(display_authors) >= 1),
    ADD COLUMN authors_state text NOT NULL DEFAULT 'unknown'
        CHECK (authors_state IN ('known', 'unknown', 'not_checked', 'conflicting', 'not_applicable'));
```

(The final `UPDATE … source_record_id IS NULL` leaves `origin` at its default
`library_metadata`; rows from other kinds keep `library_metadata/legacy/heuristic`.)

### Modules

```
lysilogy-core
  id.rs          + FieldClaimId ('a')
  state.rs       + ClaimOrigin, ClaimConfidence, ProvenanceField (string_enum!)
  provenance.rs  (new, pure) SelectionRank, selection_rank, FieldValue, AuthorListKey,
                 author_list_key, values_agree, ClaimCandidate, ClaimAgreement, ExclusionReason,
                 FieldSelection, select_field, parse_override, OverrideError
  work.rs        Work + display_authors: Option<Vec<String>>, authors_state: FieldState
  lib.rs         re-exports
lysilogy-db
  claim.rs       (new) ClaimRepository, NewClaim, StoredClaim, Reselection, RecordOutcome,
                 pub(crate) reselect_in
  work.rs        WORK_COLUMNS + display_authors, authors_state; remove add_claim, add_claim_once,
                 fill_display_title_if_unknown, fill_display_year_if_unknown; merge_work
                 retracts colliding overrides and reselects
  lib.rs         pub mod claim; truncation list in test_support unchanged (field_claim already)
lysilogy-ingest
  ingest.rs        hydrate_stub_works -> claims via ClaimRepository, authorships, stale rule
  openalex.rs      authorships in select + parse
  literature.rs    fill_display_fields -> claims; paper front-matter claims; Crossref for own Work
  local_evidence.rs record_entry_claims for every resolved entry
  crossref.rs      (new) Crossref publisher fetch + parse + cache
  front_matter.rs  (new, pure) front_matter_fields
  field_model.rs   (new) FieldExtractor, FieldRequest, FieldDraft, ground_field_draft,
                   harvest_bibliography_fields, harvest_front_matter_fields, FieldOptions,
                   FieldHarvestSummary, FIELD_SCHEMA, FIELD_PROMPT_VERSION
  identify.rs      PaperHints + authors
root
  src/graph.rs             + claims: ClaimRepository
  src/analysis/local_cli.rs ReaderToolRequest.fast, CLAUDE_FAST_MODEL, structured_fast_model_key
  src/field_extraction.rs  (new) SidecarFieldExtractor
  src/provenance.rs        (new) ProvenanceView assembly (AppState::work_provenance), CLI rendering
  src/api/provenance.rs    (new) routes
  src/literature.rs        hints.authors; fields model pass entry points
  src/literature/evidence.rs AppState::harvest_entry_fields_with_model
  src/api/literature.rs    LiteratureOptions + fields, fields_limit, publisher
  src/main.rs              Command::Provenance { ProvenanceCommand }, lit --fields --fields-limit --publisher
web
  src/types.ts (Work.display_authors; provenance types appended), src/lib/api.ts (appended),
  src/lib/provenance.ts (new, pure) + scripts/provenance.test.mjs (new),
  src/components/WorkProvenance.tsx + WorkProvenance.css (new),
  LiteratureGraph.tsx (2 lines), HelpOverlay.tsx (1 line), package.json (test:provenance)
```

### Core types

```rust
// lysilogy-core/src/state.rs
string_enum!(ClaimOrigin { User => "user", Publisher => "publisher", Paper => "paper",
    Openalex => "openalex", LibraryMetadata => "library_metadata",
    CitingBibliography => "citing_bibliography" });
string_enum!(ClaimConfidence { Explicit => "explicit", Heuristic => "heuristic", Model => "model" });
string_enum!(ProvenanceField { Title => "title", Year => "year", Authors => "authors",
    Venue => "venue", Doi => "doi" });

// lysilogy-core/src/provenance.rs
pub const ORIGIN_TRUST_ORDER: [ClaimOrigin; 6] = [User, Publisher, Paper, Openalex,
    LibraryMetadata, CitingBibliography];
pub const PROVENANCE_FIELDS: [ProvenanceField; 5] = [Title, Year, Authors, Venue, Doi];
impl ProvenanceField {
    pub const fn field_kind(self) -> FieldKind;          // Authors -> Author, Doi -> Identifier
    pub const fn selected_on_work(self) -> bool;         // title, year, authors
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SelectionRank { pub origin: u8, pub confidence: u8 }
pub const fn selection_rank(origin: ClaimOrigin, confidence: ClaimConfidence) -> SelectionRank;
pub enum FieldValue { Title(String), Year(i32), Authors(Vec<String>), Venue(String), Doi(String) }
impl FieldValue {
    pub fn parse(field: ProvenanceField, value: &serde_json::Value) -> Option<Self>;
    pub fn to_json(&self) -> serde_json::Value;
    pub fn display(&self) -> String;                     // authors joined with "; "
}
pub struct AuthorListKey { pub families: Vec<String>, pub truncated: bool }
pub fn author_list_key(authors: &[String]) -> AuthorListKey;
pub fn values_agree(left: &FieldValue, right: &FieldValue) -> bool;
pub struct ClaimCandidate { pub id: FieldClaimId, pub origin: ClaimOrigin,
    pub confidence: ClaimConfidence, pub value: Option<FieldValue>, pub review_state: ReviewState,
    pub retracted: bool, pub created_at: DateTime<Utc> }
#[serde(tag = "agreement", content = "reason", rename_all = "snake_case")]
pub enum ClaimAgreement { Selected, Agrees, Disagrees, Excluded(ExclusionReason) }
pub enum ExclusionReason { Rejected, Retracted, Unreadable }
pub struct FieldSelection { pub field: ProvenanceField, pub state: FieldState,
    pub value: Option<FieldValue>, pub winner: Option<FieldClaimId>,
    pub rank: Option<SelectionRank>, pub agreement: BTreeMap<FieldClaimId, ClaimAgreement> }
pub fn select_field(field: ProvenanceField, candidates: &[ClaimCandidate]) -> FieldSelection;
pub enum OverrideError { InvalidValue(&'static str /*detail*/) }   // code(): "invalid_value"
pub fn parse_override(field: ProvenanceField, value: &serde_json::Value)
    -> Result<FieldValue, OverrideError>;
```

```rust
// lysilogy-db/src/claim.rs
pub struct NewClaim<'a> { pub work_id: WorkId, pub field: ProvenanceField, pub value: &'a FieldValue,
    pub origin: ClaimOrigin, pub extractor: &'a str, pub confidence: ClaimConfidence,
    pub source_record_id: Option<SourceRecordId>, pub entry_id: Option<EntryId>,
    pub locator: Option<&'a str> }
pub struct StoredClaim { pub id: FieldClaimId, pub work_id: WorkId, pub field: ProvenanceField,
    pub value: Value, pub origin: ClaimOrigin, pub extractor: String,
    pub confidence: ClaimConfidence, pub source_record_id: Option<SourceRecordId>,
    pub entry_id: Option<EntryId>, pub locator: Option<String>, pub review_state: ReviewState,
    pub retracted_at: Option<DateTime<Utc>>, pub created_at: DateTime<Utc> }
pub struct Reselection { pub title_changed: bool, pub year_changed: bool,
    pub authors_changed: bool, pub title_became_known: bool }
pub struct RecordOutcome { pub inserted: usize, pub reselected: BTreeMap<WorkId, Reselection> }
impl ClaimRepository {
    pub const fn new(pool: PgPool) -> Self;
    pub async fn record(&self, claims: &[NewClaim<'_>]) -> Result<RecordOutcome>;
    pub async fn claims_for(&self, work: WorkId) -> Result<Vec<StoredClaim>>;   // provenance fields only, by created_at, id
    pub async fn get(&self, claim: FieldClaimId) -> Result<Option<StoredClaim>>;
    pub async fn set_override(&self, work: WorkId, field: ProvenanceField, value: &FieldValue)
        -> Result<StoredClaim>;
    pub async fn clear_override(&self, work: WorkId, field: ProvenanceField) -> Result<bool>;
    pub async fn set_review_state(&self, claim: FieldClaimId, state: ReviewState)
        -> Result<Option<StoredClaim>>;      // Conflict("override_not_reviewable") for user claims
    pub async fn reselect(&self, work: WorkId) -> Result<Reselection>;
    pub async fn reselect_all(&self) -> Result<usize>;  // Works with >= 1 claim row
    pub async fn source_records(&self, ids: &[SourceRecordId]) -> Result<Vec<SourceRecordBrief>>;  // id, kind, source_key, retrieved_at
    pub async fn entries(&self, ids: &[EntryId]) -> Result<Vec<EntryBrief>>;  // id, entry_key, raw_text, citing_work_id (via document_version -> document.work_id)
}
pub(crate) async fn reselect_in(conn: &mut PgConnection, work: WorkId) -> Result<Reselection>;
```

### API

| Route | Behavior |
| --- | --- |
| `GET /api/works/{id}/provenance` | 200 `ProvenanceView`. 400 `invalid_work_id`; 404 `work_not_found`; 503 `graph_unavailable`. Read-only (selection computed on read). |
| `PUT /api/works/{id}/provenance/{field}` | Body `{"value": <json>}` (`deny_unknown_fields`; must be an object). Sets the override, 200 `ProvenanceView`. 400 `invalid_field` / `invalid_body` / `invalid_value`. |
| `DELETE /api/works/{id}/provenance/{field}` | Clears the override (no-op when none), 200 `ProvenanceView`. 400 `invalid_field`. |
| `PUT /api/field-claims/{claim}/review` | Body `{"review_state": "accepted"\|"rejected"\|"unreviewed"}`. 200 `ProvenanceView` of the claim's (redirect-resolved) Work. 400 `invalid_claim_id` / `invalid_review_state` / `invalid_body`; 404 `claim_not_found`; 409 `override_not_reviewable`. |
| `GET /api/graph/neighbors/{id}` and every Work payload | Additive `display_authors: string[] \| null`, `authors_state`. |

```ts
type ProvenanceColumnKey = "user" | "publisher" | "paper" | "openalex" | "library_metadata" | "citing_bibliography";
type ProvenanceView = {
  work_id: string;
  columns: { key: ProvenanceColumnKey; label: string; editable: boolean }[];  // trust order
  fields: FieldProvenance[];                                                  // title, year, authors, venue, doi
};
type FieldProvenance = {
  field: "title" | "year" | "authors" | "venue" | "doi";
  selected: { value: unknown; display: string | null; state: FieldState;
              claim_id: string | null; origin: ProvenanceColumnKey | null; confidence: ClaimConfidence | null };
  claims: ClaimView[];                                                         // created_at order
};
type ClaimView = {
  id: string; origin: ProvenanceColumnKey; extractor: string; confidence: "explicit" | "heuristic" | "model";
  value: unknown; display: string | null;          // null when unreadable
  review_state: "unreviewed" | "accepted" | "rejected" | "disputed";
  retracted_at: string | null;
  agreement: "selected" | "agrees" | "disagrees" | "excluded";
  excluded_reason: "rejected" | "retracted" | "unreadable" | null;
  source_record: { id: string; kind: string; source_key: string; retrieved_at: string } | null;
  locator: string | null;
  entry: { id: string; entry_key: string; raw_text: string; citing_work_id: string | null;
           citing_title: string | null } | null;
  created_at: string;
};
```

Column labels: `You`, `Publisher`, `The paper`, `OpenAlex`, `Library file`, `Citing papers`.

### CLI

```
lysilogy provenance show <work> [--paper]
lysilogy provenance set <work> [--paper] <field> <value>     # authors: "A. One; B. Two"
lysilogy provenance clear <work> [--paper] <field>
lysilogy provenance reselect (<work> [--paper] | --all)
lysilogy lit <paper> [--local] [--labels] [--fields] [--sidecar claude|codex]
                     [--label-limit N] [--fields-limit N] [--publisher]
```

`--fields` implies `--local` and uses `--sidecar`. `show` prints, per field:
`<field>  <selected display> [<state>] via <origin>/<confidence>` then one line per claim:
`  <selected|agrees|disagrees|excluded:reason>  <origin>/<confidence> <extractor>  <display>  (<source kind>[, <locator>][, cited by <title>])`.
`lit` output gains `Fields     considered N · model calls N · cached N · claims N · dropped N
(<code> n, …) · failed N` when `--fields` ran, and `Publisher  <doi> fetched|cached|not found`.

### UI

- `web/src/lib/provenance.ts` (pure): `PROVENANCE_COLUMN_ORDER`, `type CellMark = "selected" |
  "agrees" | "disagrees" | "excluded"`, `type CellValue = {display, mark, count, claims}`,
  `cellValues(field, column): CellValue[]` (claims of that origin, retracted user claims
  dropped, grouped by `display`; a group's mark = `selected` if it holds the winner, else
  `excluded` if every claim is excluded, else `agrees` if any non-excluded claim agrees, else
  `disagrees`; groups ordered selected, agrees, disagrees, excluded, then by count desc),
  `activeOverride(field)`, `parseOverrideInput(field, text): {ok: true, value} | {ok: false,
  error}` (authors: split on newlines or `;`, trimmed, empties dropped, 1..=100; year: integer
  1400..=2200; title/venue: trimmed non-empty <= 500; doi: strips `https://doi.org/` / `doi:`,
  must start with `10.`), `formatAuthors(authors, limit = 6)` ("A, B, C +4"),
  `fieldLabel(field)`.
- `WorkProvenance.tsx` (`{ work: Work }`): renders (1) an authors line from
  `work.display_authors` (`formatAuthors`), nothing when absent; (2) a "Sources" button with
  `<kbd>s</kbd>`; (3) the dialog when open. The dialog loads `api.workProvenance(work.id)` (abort
  on close / Work change), shows `Loading…` / `role="alert"` errors, and a `<table>` with
  `<caption>`, `<th scope="col">` per column (Field, You, Publisher, The paper, OpenAlex, Library
  file, Citing papers) and `<th scope="row">` per field holding the field name, the selected
  value and a `conflicting` badge when so. Each cell lists its `CellValue`s as
  `<li data-mark=…>` with glyph (selected: ◆ + outline in `--signal`, agrees: ✓ on `--evidence`
  tint, disagrees: ✗ on `--danger` tint, excluded: struck through `--ink-faint`) and visually
  hidden text ("selected value", "agrees with selected value", "disagrees with selected value",
  "excluded: rejected"). Values with sources open a `<details>`: per claim the extractor,
  confidence, source record kind, locator, citing paper title and entry raw text, and a
  Reject / Restore button (non-user claims). Empty cells are blank (`<td>` with a visually hidden
  "no claim"). The You column shows the active override or an "Override" button; editing shows an
  input (textarea for authors) with Save / Cancel / Clear; validation errors from
  `parseOverrideInput` inline; server errors `role="alert"`. Saving/clearing/reviewing replaces
  the view with the response. Focus trap as `PassageQuestion`; `onKeyDown` stops propagation so
  graph keys do not fire. Width `min(1100px, 94vw)`, horizontal scroll for the table on small
  screens. Styles in `WorkProvenance.css` using `--panel`, `--panel-bright`, `--line`,
  `--line-bright`, `--ink*`, `--signal`, `--evidence`, `--danger` (tints via
  `color-mix(in srgb, var(--evidence) 18%, transparent)`).
- Mount: `LiteratureGraph.tsx` `import { WorkProvenance } from "./WorkProvenance";` and
  `<WorkProvenance work={work} />` right after the `literature-work-meta` paragraph in
  `WorkPanel`. `HelpOverlay.tsx`: `["s (graph, Work open)", "Sources: every claimed title, year,
  author list, venue and DOI by source; override any field"]`.

## Gate (every unit)

```sh
eval "$(~/Projects/Lysilogy/scripts/db env)"        # scripts/db start if status fails
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                              # DB tests must run, not print "skipping"
(cd web && npm run typecheck && npm run lint && npm run build \
  && npm run test:walk && npm run test:ramp && npm run test:relevance \
  && npm run test:literature-layout && npm run test:api)
# from U4 also: npm run test:provenance
```

- Run from the worktree root; default `target/` inside the worktree; no caches in `/tmp`.
- Tests named `should_<behavior>_when_<condition>` (Rust) / `should … when …` (node). DB tests
  in `lysilogy-db` use `test_support::lock()` / `pool()`; ingest tests use their module harness;
  assert only on rows the test created (unique titles/DOIs per test).
- No network in tests (`RecordingTransport`, root stub transports); no real model calls in
  tests (fake `FieldExtractor`, fake `claude` scripts as in `labeling.rs` tests).
- Bounded integers: plain `as` at the call site (narrowest `#[allow(clippy::cast_…)]`).
- Commits: atomic conventional commits ending with the `Co-Authored-By` trailer. No push.

## Implementation units

### U1. Schema, pure selection, claim repository, writers rewired, display_authors

Files: `crates/lysilogy-db/migrations/0009_field_provenance.sql` (new);
`crates/lysilogy-core/src/{id.rs, state.rs, provenance.rs (new), work.rs, lib.rs}`;
`crates/lysilogy-db/src/{claim.rs (new), work.rs, lib.rs}`;
`crates/lysilogy-ingest/src/{ingest.rs, literature.rs, local_evidence.rs}` (rewire only);
`src/graph.rs` (`claims`), `src/main.rs` (`Command::Provenance` with only `Reselect`),
`src/relevance.rs`, `src/main.rs` test literal `Work` constructions (add the two fields);
`web/src/types.ts` (the one contract line).

Steps:
1. Migration exactly as in Design.
2. Core: enums, `FieldClaimId 'a'`, `provenance.rs` per Core types and Decisions 12-16, 19.
   `Work` gains `display_authors`, `authors_state` (serde names identical).
3. DB: `WORK_COLUMNS` and every `returning` list add `w.display_authors, w.authors_state`
   (`create`, `resolve_or_create` use the bare column names); `work_from_row` reads them.
   `claim.rs` per Core types and Decisions 17-20; `record` groups by Work, one transaction per
   Work (lock, inserts, `reselect_in`). Delete `add_claim`, `add_claim_once`,
   `fill_display_title_if_unknown`, `fill_display_year_if_unknown` and port their tests.
   `merge_work`: Decision 27.
4. Rewire with unchanged harvest scope (U2 widens it):
   - `hydrate_stub_works`: title/year claims via `ClaimRepository::record` (`openalex`,
     `openalex`, `explicit`); count `titled` from `Reselection::title_became_known`. Signature
     gains `claims: &ClaimRepository` (update `literature.rs`, `expand.rs` callers and the
     `LiteratureContext` / `ExpandContext` structs: add `claims: &'a ClaimRepository`; root
     builds them from `graph.claims`).
   - `fill_display_fields`: provider title/year (`openalex`), hint title/year
     (`library_metadata`, `catalog-metadata`, `heuristic`). Drop the "provider over hints"
     ordering comment: ranking now decides.
   - `create_stub`: claims (`citing_bibliography`, `bib-v1`, confidence from `FieldConfidence`,
     `entry_id`, locator `entry_key`); pass the `StoredEntry` in.
   - `LocalEvidenceContext` gains `claims`.
5. `lysilogy provenance reselect (<work> [--paper] | --all)` prints `Reselected N works`.
6. TS contract line in `Work`.

Tests:
- core pure (`provenance.rs`):
  `should_select_highest_trust_claim_when_tiers_disagree` (one claim per origin, each a
  different title; winner is user; drop user -> publisher; … down to citing heuristic > model),
  `should_select_highest_trust_claim_when_input_order_is_shuffled` (all permutations of 5),
  `should_mark_conflicting_when_winning_tier_claims_disagree`,
  `should_not_mark_conflicting_when_only_lower_tiers_disagree`,
  `should_not_mark_conflicting_when_titles_differ_only_in_case_and_punctuation` (title_key),
  `should_treat_explicit_heuristic_and_model_as_separate_tiers_when_selecting`,
  `should_mask_every_field_when_user_override_present` (title, year, authors, venue, doi),
  `should_restore_tiered_value_when_override_retracted`,
  `should_exclude_rejected_claim_when_selecting` (a rejected publisher claim loses to openalex;
  agreement `excluded(rejected)`),
  `should_return_unknown_when_every_claim_is_excluded`,
  `should_exclude_unreadable_value_when_json_does_not_match_field`,
  `should_compare_authors_by_ordered_family_names_when_normalizing`
  ("Christiano, P. F." == "Paul F. Christiano"; order matters),
  `should_agree_when_truncated_author_list_is_prefix_of_full_list`,
  `should_prefer_complete_author_list_when_tier_agrees`,
  `should_mark_agreement_for_every_claim_when_selecting`,
  `should_break_ties_by_support_then_age_then_id_when_tier_agrees`,
  `should_reject_invalid_override_values_when_parsing` (empty title, 501 chars, control char,
  year 1399/2201/"2020", authors `[]`, 101 names, `[""]`, bad DOI),
  `should_round_trip_claim_enums_and_field_claim_id_when_parsing`.
- DB-gated (`claim.rs`, `work.rs`):
  `should_apply_migration_backfill_when_legacy_claims_exist` (insert rows mimicking the three
  legacy shapes with raw SQL before reading; assert origins via a fresh `record` round trip is
  not needed: query columns),
  `should_select_highest_trust_claim_when_recording_claims` (openalex then citing then publisher:
  display_title follows the highest tier after each write),
  `should_mark_title_conflicting_only_when_winning_tier_disagrees`,
  `should_write_display_authors_when_author_claims_recorded`,
  `should_not_duplicate_claim_when_recorded_twice`,
  `should_mask_any_field_when_override_set_and_restore_when_cleared` (all five fields; title,
  year, authors via `graph.work`, venue/doi via `select_field` over `claims_for`),
  `should_keep_one_active_override_when_set_twice` (first retracted, unique index holds),
  `should_return_false_when_clearing_without_override`,
  `should_exclude_rejected_claim_when_review_state_set` (and restore on `unreviewed`),
  `should_refuse_review_of_user_claim`,
  `should_leave_fixture_title_when_work_has_no_claims` (`set_display_title` then `reselect`),
  `should_null_display_title_when_only_claim_is_rejected`,
  `should_reselect_survivor_and_keep_its_override_when_merging_works`,
  `should_reselect_every_claimed_work_when_reselecting_all`.
- ingest (existing harnesses, adjusted): `should_title_stub_from_summary_when_hydrating` (count
  via `title_became_known`), local-evidence stub title/year tests keep passing with claims now
  carrying `bib-v1` and `entry_id`.

Acceptance: gate green; `scripts/db test-psql -c '\d graph.field_claim'` shows the columns; on
the dev DB (migrates on connect) `lysilogy provenance reselect --all` runs once; record counts
of `origin` x `field` and how many `display_title`s changed (spot-check 5) in "U1
implementation notes".

### U2. Deterministic harvest: every resolved entry, OpenAlex authorships, Crossref, front matter

Files: `crates/lysilogy-ingest/src/{local_evidence.rs, ingest.rs, openalex.rs, literature.rs,
identify.rs, crossref.rs (new), front_matter.rs (new), expand.rs, lib.rs}`;
`src/literature.rs` (`hints.authors`, `publisher` option plumbing), `src/api/literature.rs`
(`publisher: Option<bool>`), `src/main.rs` (`lit --publisher`, output lines), `src/expansion.rs`
(context fields).

Steps:
1. `record_entry_claims` per Decision 23; call it for every `Resolved` outcome in
   `run_local_evidence` (after `add_entry_assertion`). Remove claim writes from `create_stub`.
   Add `LocalEvidenceSummary.field_claims: usize` (inserted rows).
2. OpenAlex per Decision 22 (`work_url`, `works_batch_url`, `parse_summary`, `parse_work`
   authors; `hydrate_stub_works` over all given Works with the stale rule; doi claims).
   `run_literature_flow` passes `cited + [own Work]`. Update request-counting fixtures in
   `ingest.rs`, `literature.rs`, `expand.rs`, `src/expansion.rs` tests to include
   `"authorships": []` (or real lists) so cached summaries are not stale.
3. Crossref per Decision 21: `fetch_crossref_cached(records, client, doi) ->
   Result<Option<(SourceRecordId, CrossrefWork)>, ProviderError>` (`None` = cached or fresh
   not-found), `record_publisher_claims(claims, work, record, &CrossrefWork)` (title, year,
   authors, venue, doi; `publisher`, `crossref-v1`, `explicit`). In `run_literature_flow`, after
   identifiers are attached: the own Work's DOI = provider DOI, else the detected DOI; skip
   `10.48550/`. `LiteratureInput` gains `publisher_cited: bool`; when true, after hydration,
   fetch for outgoing Works' active DOIs (`active_identifiers`), at most
   `PUBLISHER_FETCH_MAX`. `LiteratureSummary` gains `publisher: Vec<PublisherFetch {doi,
   outcome: fetched|cached|not_found|failed}>` (own Work first; cited ones summarized as counts
   in CLI output).
4. Front matter per Decision 24: `front_matter_fields` pure; claims written in
   `fill_display_fields` for the linked Work.

Tests:
- ingest DB-gated (`local_evidence.rs` harness):
  `should_write_claims_when_entry_resolves_to_existing_work_by_identifier`,
  `should_write_claims_when_entry_resolves_to_existing_work_by_title`,
  `should_write_claims_when_entry_reuses_accepted_resolution` (second run on a new bibliography
  record version is not needed: delete the claims rows by SQL, rerun, claims come back),
  `should_write_author_venue_and_doi_claims_with_entry_and_locator_when_fields_present`,
  `should_use_explicit_confidence_for_doi_and_heuristic_for_authors_when_recording_entry_claims`,
  `should_write_no_claims_when_entry_is_ambiguous_or_self_reference`,
  `should_not_duplicate_claims_when_local_evidence_reruns` (claim count per Work identical after
  run 2; `field_claims == 0` on run 2),
  `should_keep_openalex_title_when_bibliography_disagrees` (existing Work with an openalex title
  claim; entry with a different title resolves by DOI; display_title unchanged, state known).
- ingest (`openalex.rs`, `ingest.rs`, `RecordingTransport`):
  `should_select_authorships_when_building_work_and_batch_urls`,
  `should_parse_ordered_authors_when_authorships_present` (empty names skipped, garbage ->
  empty),
  `should_record_openalex_author_claims_when_hydrating`,
  `should_refetch_summary_once_when_cached_payload_lacks_authorships` (2nd hydrate: 0 requests),
  `should_hydrate_titled_works_too_when_authors_missing`.
- ingest (`crossref.rs`): `should_parse_title_authors_year_venue_when_crossref_message_given`
  (fixture modeled on a real `api.crossref.org/works` message), `should_percent_encode_doi_when_building_crossref_url`,
  `should_cache_not_found_and_skip_refetch_when_crossref_returns_404` (transport returning
  `ProviderError::Status(404)`; 2nd call 0 requests), `should_skip_arxiv_datacite_doi_when_fetching_publisher`,
  `should_record_publisher_claims_that_win_selection_when_crossref_fetched` (DB-gated: publisher
  title beats an openalex title).
- ingest (`front_matter.rs` pure): `should_accept_title_when_present_in_front_matter`,
  `should_reject_title_when_absent_or_too_short`, `should_accept_authors_when_every_name_occurs`,
  `should_reject_authors_when_a_name_is_missing_or_single_word`.
- ingest (`literature.rs` harness): `should_write_paper_claims_that_outrank_openalex_when_front_matter_matches`,
  `should_fetch_publisher_record_for_own_work_when_doi_known`,
  `should_fetch_publisher_for_cited_works_only_when_requested`,
  `should_warn_and_continue_when_publisher_fetch_fails_hard`.
- root: `should_reject_non_boolean_publisher_when_parsing_lit_options` (API body),
  CLI parse `should_accept_publisher_flag_when_parsing_lit`.

Live check ("U2 implementation notes"): `lysilogy lit "Deep Reinforcement Learning from Human
Preferences" --local` then again: report `field_claims` (run 2 = 0), OpenAlex requests on run 2
(expect 0 after the one-time upgrade on run 1; time both), author claim counts by origin for the
paper's neighborhood, the paper's own provenance (`psql` until U3's `show` exists), Crossref
outcome for its DOI.

Acceptance: gate green; live check recorded.

### U3. Grounded model pass, provenance API, CLI show/set/clear

Files: `crates/lysilogy-ingest/src/{field_model.rs (new), lib.rs, local_evidence.rs (target
selection helper)}`; `src/analysis/{local_cli.rs, mod.rs}` (`fast`), `src/field_extraction.rs`
(new), `src/literature/evidence.rs`, `src/literature.rs`, `src/api/literature.rs` (`fields`,
`fields_limit`), `src/main.rs` (`--fields`, `--fields-limit`, `Command::Provenance`
`Show|Set|Clear`), `src/provenance.rs` (new), `src/api/provenance.rs` (new), `src/api.rs`
(merge routes), `src/lib.rs`.

Steps:
1. Fast model plumbing per Decision 25. Before coding the Claude branch, run one manual call
   (`echo '{}' | claude --print --model haiku --output-format json --json-schema '<schema>' …`)
   and record whether `--effort` is accepted with Haiku; omit it for `fast` unless accepted.
2. `field_model.rs`: `FIELD_PROMPT_VERSION = 1`, `FIELD_SCHEMA` (object `items: [{index:
   integer, title: string, authors: [string], year: string}]`, all required, no extra keys,
   empty string / empty array when absent), `field_prompt(&FieldRequest)` (bibliography: numbered
   entries, "copy every value character for character from the entry; leave empty when not
   printed"; front matter: the text, "this paper's own title, authors in printed order, year of
   publication if printed"), `field_cache_key(model_key, shown)`, `ground_field_draft`,
   `harvest_bibliography_fields(ctx, citing_work, document_version, extractor, options) ->
   FieldHarvestSummary`, `harvest_front_matter_fields(ctx, work, front_matter, extractor) ->
   FieldHarvestSummary`. Reuse `labels.rs`'s `join_all` and semaphore pattern (move `join_all`
   to a small `crate::concurrency` module rather than copy it).
3. Root: `SidecarFieldExtractor`; `AppState::harvest_entry_fields_with_model(id, summary,
   extractor, options)` and `AppState::harvest_paper_fields_with_model(id, summary, extractor)`;
   `lit --fields` runs both after local evidence (and labels); API `fields` likewise.
   `LocalEvidenceSummary.fields: Option<FieldHarvestSummary>`; `LiteratureSummary` unchanged
   (paper fields summary attached to the local summary as `paper_fields`).
4. `src/provenance.rs`: `AppState::work_provenance(work) -> ProvenanceView` (claims, source
   record briefs, entry briefs with citing Work titles, `select_field` per field); errors
   `ProvenanceError {NotFound(&'static str), Invalid(&'static str, String), Conflict(&'static
   str), Graph(Error)}` shared by API and CLI (as `RampServiceError`).
5. `src/api/provenance.rs` routes per the API table.
6. CLI `provenance show|set|clear` per the CLI section (`set` value parsing: title/venue/doi as
   given, year integer, authors split on `;`).

Tests:
- ingest pure (`field_model.rs`): `should_drop_model_value_when_not_verbatim_in_entry_text`
  (title paraphrased, year not printed, one author misspelled -> whole author list dropped; the
  verbatim fields kept), `should_drop_year_when_malformed_or_out_of_range`,
  `should_ground_against_whitespace_collapsed_text_when_entry_has_line_breaks`,
  `should_drop_unknown_index_when_model_answers_for_missing_entry`,
  `should_drop_author_list_when_more_than_one_hundred_names`,
  `should_change_cache_key_when_prompt_version_or_model_changes`,
  `should_reject_extra_properties_when_schema_is_strict` (schema JSON has
  `additionalProperties: false` at both levels and all properties required).
- ingest DB-gated (fake `FieldExtractor` counting calls):
  `should_make_no_model_calls_when_rerun_hits_cache` (run 1: k calls; run 2: 0 calls, same
  claim count), `should_cache_omitted_entries_when_batch_answer_skips_them`,
  `should_batch_twenty_entries_per_call_and_stop_at_limit`,
  `should_skip_entries_whose_work_has_higher_tier_authors_and_year`,
  `should_write_model_claims_below_bib_v1_when_both_exist` (heuristic wins, model `agrees` or
  `disagrees`, not `conflicting`), `should_count_failures_without_stopping_when_extractor_errors`,
  `should_write_paper_model_claims_when_front_matter_lacks_deterministic_fields`.
- root (`field_extraction.rs`, fake `claude` scripts as in `labeling.rs`):
  `should_pass_haiku_model_when_fast_claude_call_runs` (script echoes its args into the output),
  `should_key_fast_sidecar_as_claude_haiku`, `should_report_malformed_when_field_output_is_not_items`.
- root (`local_cli.rs`): `should_not_pass_model_when_reader_tool_is_not_fast` (existing callers).
- API (DB-gated unless noted): `should_return_400_when_provenance_ids_or_field_malformed` (no
  DB), `should_return_503_when_provenance_requested_without_graph`,
  `should_return_every_field_and_claim_with_agreement_when_reading_provenance`,
  `should_include_entry_and_citing_title_when_claim_came_from_bibliography`,
  `should_mask_field_and_return_view_when_override_put` (each of the five fields),
  `should_restore_tiered_value_when_override_deleted`,
  `should_return_400_when_override_value_invalid` (per field, plus unknown body key, non-object),
  `should_exclude_claim_when_review_rejected_and_return_409_for_override`,
  `should_follow_redirect_when_reading_provenance_of_merged_work`,
  `should_include_display_authors_when_reading_neighbors`.
- CLI parse: `should_parse_provenance_subcommands_when_arguments_valid`,
  `should_reject_reselect_without_work_or_all`, `should_accept_fields_flags_when_parsing_lit`.

Live check ("U3 implementation notes"): `lysilogy lit "Deep Reinforcement Learning from Human
Preferences" --fields --sidecar claude --fields-limit 3` (record calls, cached, claims, dropped
codes, wall time, a sample of 5 grounded and 3 dropped answers from the cache records); rerun:
0 model calls. `lysilogy provenance show` on the paper and on two cited stubs; `set`/`clear` a
title on a scratch stub and confirm restore. Remove nothing except scratch overrides.

Acceptance: gate green; live check recorded.

### U4. UI: WorkProvenance, agreement marking, live smoke, report

Files: `web/src/types.ts` (append provenance types), `web/src/lib/api.ts` (append
`workProvenance(workId, signal)`, `setFieldOverride(workId, field, value)`,
`clearFieldOverride(workId, field)`, `reviewFieldClaim(claimId, reviewState)`),
`web/src/lib/provenance.ts` (new), `web/scripts/provenance.test.mjs` (new), `web/package.json`
(`test:provenance` after `test:relevance`), `web/src/components/WorkProvenance.tsx` +
`WorkProvenance.css` (new), `LiteratureGraph.tsx` (import + one JSX line),
`HelpOverlay.tsx` (one line), `docs/lit-provenance-plan.md` (notes, report),
`docs/literature-viewer-status.md` (provenance section: setup step `lysilogy provenance
reselect --all`, test script, known gaps).

Behavior per the UI design and Decision 29.

Tests (`provenance.test.mjs`): `should mark cell green when its value agrees with the selected
value`, `should mark cell red when its value disagrees with the selected value`, `should mark
the winning cell selected`, `should return no values when a source has no claim`, `should mark
excluded when every claim in a cell is rejected`, `should hide retracted overrides from the
user column`, `should group citing claims by value with counts`, `should order columns user
publisher paper openalex library citing`, `should parse authors one per line or semicolon
separated when overriding`, `should reject year outside 1400 to 2200 when overriding`, `should
strip doi resolver prefix when overriding`, `should truncate author list with remaining count
when formatting`.

Scratch Playwright `~/.cache/lysilogy-smoke/provenance-smoke.mjs` (serves `web/dist`, mocks
`/api/*`, imports `~/Projects/Lysilogy/web/node_modules/playwright/index.mjs`): Work panel shows
the authors line; `s` opens the dialog with focus inside; columns in order with You first; a
red cell, a green cell and the selected outline are present (`data-mark`); Tab reaches the You
cell's Override button, Enter edits, typing a title + Enter sends `PUT …/provenance/title`, the
mocked response marks the user cell selected; Clear sends `DELETE`; Reject on a citing value
sends `PUT /api/field-claims/…/review`; Esc closes only the dialog (panel stays), a second Esc
closes the panel; graph keys (`j`, `a`) do nothing while the dialog is open; no console errors;
screenshot `provenance.png`.

Live smoke against `lysilogy serve` on the dev DB (record in "U4 implementation notes"): open
"Deep Reinforcement Learning from Human Preferences", `L`, select a cited stub with
bibliography + OpenAlex claims, `s`: verify table against `lysilogy provenance show`; override
its year, confirm the graph label after reopening, clear it. Then write the **Report**: what was
built, gate numbers, live numbers (claims by origin, conflicting counts by field across the dev
DB, model cost proxy: calls and cache hits), known gaps.

Acceptance: web gate green including `test:provenance`; Rust gate green; Playwright pass and live
smoke recorded; report written.

## Assumptions

- A1. Columns in the UI are tiers (one column per origin), not one column per individual source;
  the citing column groups values across citing papers with counts and per-source details.
- A2. Venue and DOI are selected on read only (no `graph.work` column); a DOI override never
  changes `graph.work_identifier`.
- A3. Legacy claims with no source record (catalog hints: file name, PDF info) are
  `library_metadata`, ranked below OpenAlex and above citing papers, and shown in a "Library
  file" column between OpenAlex and Citing papers. They are not front-matter evidence, so they
  do not get the `paper` tier.
- A4. Author agreement extends "ordered family names" with one rule: a list truncated by
  "et al." agrees with a list it is a prefix of. Without it, bibliographies that print
  "Smith et al." would mark nearly every multi-author Work `conflicting`.
- A5. bib-v1 never emits `Explicit` for authors/year/title/venue, so the model pass targets every
  resolved entry (bounded by `--fields-limit` and batching), and "bibliography explicit" is a
  defined but currently empty tier.
- A6. No deterministic front-matter year (page 1 dates are often arXiv version or conference
  dates); the year for the `paper` tier comes only from the grounded model fallback.
- A7. The model pass skips entries whose Work already has eligible author **and** year claims
  ranked above `citing_bibliography` (cost; such model claims could never be selected).
- A8. The model pass is opt-in (`--fields`), like `--labels`; deterministic harvest always runs.
- A9. Overrides use `retracted_at` (only on user claims) instead of a tombstone row; history of
  overrides is kept but hidden in the UI.
- A10. The Work panel authors line belongs to `WorkProvenance` (this track); the histogram track
  shows authors only in its own view.
- A11. Crossref is fetched for the paper's own Work on every `lit` (cached), for cited Works
  only with `--publisher`; no mailto is sent to Crossref (the transport only decorates OpenAlex).
- A12. The one-time OpenAlex summary refetch for cached payloads without `authorships` is
  acceptable (about one batch request per 50 cited Works on the first rerun of each paper).
- A13. Haiku is addressed as `--model haiku` (CLI alias); whether `--effort` is accepted with it is
  verified live in U3 and omitted otherwise.

## Open questions for the user

1. Should model-extracted bibliography values really rank below bib-v1 heuristic values? Given
   A5, the model then only fills `Missing` fields and otherwise corroborates or contradicts. An
   alternative is grounded model > heuristic for authors/year (bib-v1's known et-al. boundary
   bug would then be masked by the model).
2. Is the "Library file" column (A3) wanted, or should catalog-hint claims be hidden/dropped?
3. Should `--fields` become the default for `lit --local` once the cost is measured in U3?

## U1 implementation notes

Built as planned: migration 0009, core `provenance` module, `ClaimRepository`, writers rewired,
`display_authors` on `Work`, `lysilogy provenance reselect`, and the one TS contract line.
Commits: `802e40e` (core) and `e259189` (migration, db, ingest, root, web). The core commit does
not build the whole workspace on its own (the `Work` literals elsewhere need the new fields);
the second commit does.

Deviations (smallest sound changes):

- Migration backfill ordered the entry lookup by `e.ordinal`, which is not a column of
  `source.bibliography_entry`; it orders by `e.created_at, e.id`. Each backfill `UPDATE` also
  filters `c.extractor = 'legacy'`, so the block is replay-safe. The backfill test relies on
  that: it inserts legacy-shaped rows, replays the three statements (sliced out of the
  migration file) in a transaction and rolls back.
- `Reselection` has an extra `changed()` helper; `StoredClaim`/`claims_for`/`get` only expose
  tracked claims (title, year, author, venue, and `identifier` rows whose value scheme is
  `doi`). `set_override` rejects a value whose field differs from the requested field
  (`Conflict("override_field_mismatch")`).
- `title_became_known` is true when the Work had no title and its title state was neither
  `known` nor `conflicting` (same meaning as the old `fill_display_title_if_unknown` count).
- `lysilogy provenance reselect` is backed by `AppState::reselect_claims` in the new
  `src/provenance.rs`; `--paper` requires the positional `<work>`.
- `fill_display_fields` now takes `&ClaimRepository` instead of `&WorkRepository`.
- `should_title_stub_works_when_hydrating` is renamed `should_title_stub_from_summary_when_hydrating`
  and curates the title with `set_override` (selection ignores `set_display_title` once claims
  exist). `claims_and_identifiers_are_recorded`, the title-key test and the merge test record
  claims through `ClaimRepository` instead of `add_claim`/`fill_*`. `should_create_titled_stub_when_nothing_matches`
  additionally asserts origin `citing_bibliography`, extractor `bib-v1`, an `entry_id` and a
  `bib-v1:` locator.

Gate (worktree root, test DB from `scripts/db env`):

- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: every binary passed, 0 failed (root 344 + 21 + 1 + 1, core 95, db 110,
  ingest 241); DB tests ran (no "skipping").
- web: `typecheck`, `lint`, `build` pass; `test:walk` 11, `test:ramp` 10, `test:relevance` 13,
  `test:literature-layout` 31 pass. `test:api` fails identically on the unmodified base
  (`ERR_MODULE_NOT_FOUND` for the extensionless `./relevance` import in `web/src/lib/api.ts`
  under `--experimental-strip-types`); unrelated to this track.

Dev DB (migrated on connect, then `lysilogy provenance reselect --all`, 3,109 Works, 2.8 s):
`\d graph.field_claim` shows `origin, extractor, confidence, entry_id, retracted_at`, the
`field_claim_entry` and `field_claim_active_override` indexes and the retraction constraint.
Claims by origin/field/extractor/confidence: openalex title 3,011 and year 3,012
(`openalex/explicit`), citing_bibliography title 105 and year 104 (`bib-v1/heuristic`),
library_metadata title 106 and year 98 (`catalog-metadata/heuristic`); total 6,436 as before.
A before/after dump of all 3,461 Works (`display_title`, `display_year`, `title_state`) differs
in 0 rows: the legacy first-writer-wins picks already equal the tiered selection, and no Work is
`conflicting` yet (`known` 3,099, `unknown` 362).

## U2 implementation notes

Built as planned: bib-v1 claims for every resolved entry, OpenAlex authorships, Crossref publisher
claims, deterministic front-matter claims, and the `--publisher` / `publisher` plumbing.
Commits: `d44bd00` (entry claims), `54d8050` (OpenAlex authorships and hydration over every
Work), `d501261` (Crossref, front matter, root plumbing). Intermediate commits build; the root
tests only fully pass from `d501261`.

Deviations (smallest sound changes):

- `fetch_crossref_cached` returns a `CrossrefOutcome` (`Fetched | Cached | NotFound{cached} |
  Skipped`) instead of `Option<(SourceRecordId, CrossrefWork)>`. The flow needs
  fetched/cached/not-found to fill `PublisherFetch`, and a cached positive record is re-recorded
  as claims (deduplicated), which keeps a new Work with the same DOI correct. A `Failed` outcome
  is produced by the flow, not the fetcher.
- `PublisherFetch` has a third field `cited: bool` so the CLI can print the paper's own lookup
  and summarize the cited ones. Skipped (arXiv) DOIs produce no entry.
- `PUBLISHER_FETCH_MAX` bounds requests, not DOIs seen: cached answers cost nothing, so a rerun
  keeps making progress through a long reference list. The cap adds warning
  `publisher_fetch_limit_reached`.
- The own DOI for the lookup is the first of provider DOI, then detected DOIs, that Crossref can
  hold (a provider DOI that is a `10.48550/` DataCite DOI no longer hides a real detected DOI).
- `CrossrefWork.doi` is `Option<String>` (the record's normalized `DOI`).
- Crossref URLs keep `/ - . _ ~` readable and percent-encode everything else.
- `run_literature_flow` (root `AppState`) takes `publisher_cited: bool`; expansion passes `false`.
- `Local` line in the CLI gains ` · field claims N` (no separate line).
- Test fixtures: summaries in `openalex.rs`, `ingest.rs`, `expand.rs`, `src/expansion.rs`,
  `src/api/literature.rs` carry `"authorships": []`. The expansion `ScriptedTransport::serving`
  answers Crossref with an empty message, and the API `SlowTransport` now answers summary batches
  (including for the paper's own Work, titled consistently) because the own Work is always
  hydrated now.

Gate (worktree root, test DB from `scripts/db env`):

- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace --no-fail-fast`: ingest 268 passed (stable over 4 reruns; DB tests ran,
  no "skipping"), core 95, db 110, root bin 23 + 1 + 1 passed. Root lib: 344 passed, 1 failed:
  `api::acquisition::tests::should_report_mapped_acquisition_and_paper_id_when_job_finished`
  (the Work's cached prominence row has `local_available: false`). It fails identically on the
  unmodified U1 head `1628049` (checked with `git stash -u`), so U2 did not cause it. Likely
  cause: the shared test database accumulates workspaces from harnesses that never clean up
  (`labels-test` 25 rows, `local-evidence-test`, ...), and `ensure_default_workspace` returns the
  oldest one; I did not clear the shared test DB to confirm.
- web (no web change in U2): `typecheck`, `lint`, `build` pass; `test:walk` 11, `test:ramp` 10,
  `test:relevance` 13, `test:literature-layout` 31 pass; `test:api` fails as in U1 (unrelated,
  `ERR_MODULE_NOT_FOUND` on the base).

Live check on the dev DB (`lysilogy lit "Deep Reinforcement Learning from Human Preferences"
--local`, debug build, scratch `--data`/`--notes`, library read from the main checkout):

- Run 1 (the first `lit` after U1, so the one-time upgrade): `field claims 140`, 2.7 s. Requests
  to OpenAlex: 1 summary batch (32 stale summaries, 31 cited + the paper, refetched and upserted;
  the work record was cached) plus the prominence refresh's cited-by batch (21 counts). Crossref:
  none, see below.
- Run 2: `field claims 0`, 1.7 s, 0 OpenAlex requests (no `source_record` row was retrieved
  after run 1).
- Author claims for the paper's neighborhood (the paper and its provider-cited Works):
  `citing_bibliography/bib-v1/heuristic` 53, `openalex/openalex/explicit` 31 (30 cited Works
  plus the paper).
- The paper's own claims: openalex title/year and author list (`Paul F. Christiano, Jan Leike,
  Brown, Tom B., Miljan Martic, Shane Legg, Dario Amodei`), a `paper/frontmatter-v1/heuristic`
  title claim (locator `front_matter`), and `library_metadata` title/year. No `paper` author claim:
  the catalog author hints did not all occur in the first pages. The second run linked via
  `title_match` instead of `identifier` because the cached reading index's front pages lack the
  arXiv stamp (existing behavior, not changed here).
- Crossref outcome: the paper's only DOI is `10.48550/arxiv.1706.03741` (DataCite), so it is
  skipped and no `Publisher` line is printed. As a real-DOI check, `lit "The China Syndrome"`
  (DOI `10.1257/aer.103.6.2121`) printed `Publisher  10.1257/aer.103.6.2121 fetched` (2.4 s),
  then `cached` on the rerun (1.0 s); it recorded 5 `publisher/crossref-v1/explicit` claims
  (title, year, authors `David H Autor, David Dorn, Gordon H Hanson`, venue `American Economic
  Review`, doi). `--publisher` for cited Works was exercised only against fixtures.

## U3 implementation notes

Built as planned: fast-model plumbing, `field_model.rs` (grounding, per-entry cache, batching),
`SidecarFieldExtractor`, `lit --fields`, `AppState::work_provenance` and the override/review
wrappers, the provenance API, and `lysilogy provenance show|set|clear`. Commits: `b8859e1` (fast
model), `7fba890` (`field_model`, `concurrency`, `EvidenceRepository::resolved_entries`),
`9870ef2` (provenance service and API), `0bd7782` (field extractor, `--fields`, CLI).

Haiku check (manual `claude --print` calls): `--effort low` is accepted with `--model haiku` and
the call ran on `claude-haiku-5-5`, so `fast` keeps `--effort low`. A second finding changed the
design: with `--permission-mode plan`, the `haiku` alias is silently replaced by the plan model
(`modelUsage` showed `claude-sonnet-5-5`), while the full id `claude-haiku-5-5` and plan-free calls
run on Haiku. Fast calls are tool-free, so `StageProfile.claude_model = Some("haiku")` also
drops `--permission-mode plan`. Every other call is unchanged (a test asserts no `--model`).

Deviations (smallest sound changes):

- `EvidenceRepository::resolved_entries(version)` (db) lists a version's accepted entries with
  their Work in ordinal order; the plan put target selection in `local_evidence.rs`, but the
  selection needs no resolver internals, so it lives in `field_model.rs` and reads that query.
- `FieldDraftItem.index` is `i64`, so a negative or oversized index is `unknown_index`, not a
  parse failure. Entries are numbered from 0 in the prompt.
- Paper fallback runs when any of title, authors, year lacks a non-model `paper` claim (always,
  since no deterministic year exists) and writes claims only for the fields that lack one.
- `entries_considered` counts resolved entries other than the paper itself; the weak-field test is
  `Missing`/`Heuristic` (everything today, A5).
- `lit --fields` shares `--sidecar` with `--labels`; the CLI prints a `Fields` line and, for the
  front-matter call, a `Paper` line in the same format. API: `fields`, `fields_limit` (strict
  parsing; unknown values give `invalid_body`).
- `ScriptedFieldExtractor` is public in `field_model.rs` like `ScriptedLabeler`.

Gate (worktree root, test DB from `scripts/db env`):

- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace --no-fail-fast`: root lib 367, root bin 26 + 1 + 1, core 95, db 110,
  ingest 286 passed, 0 failed (DB tests ran; the acquisition test that failed in U2 passed).
- web (no web change in U3): `typecheck`, `lint`, `build` pass; `test:walk` 11, `test:ramp` 10,
  `test:relevance` 13, `test:literature-layout` 31 pass; `test:api` still fails on the base
  (`ERR_MODULE_NOT_FOUND` for `./relevance`).

Live check on the dev DB (release build, scratch `--data`/`--notes`, library read from the main
checkout), `lit "Deep Reinforcement Learning from Human Preferences" --fields --sidecar claude
--fields-limit 3`:

- Run 1, 37.4 s: `Fields considered 53 · model calls 2 · cached 0 · claims 69 · dropped 0 · failed 0
  · skipped (higher tier) 21`; `Paper considered 1 · model calls 1 · claims 1`. 32 entries went to
  the model (batches of 20 and 12, under the limit of 3). Claims: title 22, year 25, author 22
  (`citing_bibliography/bib-model-v1/model`) plus 1 `paper/frontmatter-model-v1` author list. No
  omitted entries.
- Run 2, 1.0 s: `model calls 0 · cached 32 · claims 0`; paper `cached 1`.
- Grounded samples (cache records): "April: Active preference learning-based reinforcement
  learning" / 3 authors / 2012; "Mastering the game of Go ..." / 20 authors / 2016; "Concrete
  problems in AI safety" / 6 authors / 2016; "Asynchronous methods for deep reinforcement
  learning" / 8 authors / 2016; the paper's own title and 6 authors. A tail fragment ("In
  International Conference on Intelligent Robots and Systems, ... 2012.") came back with an empty
  title and authors and year 2012 (empty means absent, not a drop).
- Dropped samples: none. Haiku copied every value verbatim in this run, so there are no real
  dropped answers to show; the drop codes are covered by the pure tests.
- `provenance show` on the paper: title `paper/heuristic` selected with four agreeing OpenAlex /
  library claims; year 2017 from OpenAlex; DOI from OpenAlex; authors selected from
  `paper/model` (front matter) and shown as disagreeing with OpenAlex only in order (front-matter
  text extraction reads the author grid row by row: Christiano, Martic, Leike, ... versus the
  OpenAlex order Christiano, Leike, Brown, ...). Because `paper` outranks `openalex` whatever the
  confidence (Decision 12), a model-read author order from a gridded header can beat an explicit
  OpenAlex list. Candidate follow-up: rank `model` below `explicit`/`heuristic` across origins for
  authors, or require agreement with a lower tier. Not changed here.
- Stub `Superintelligence` (OpenAlex + bib-v1 + model): model claims agree on title and authors;
  year 2014 (bib-v1 and model) disagrees with OpenAlex 2015, which wins.
- Stub `arXiv preprint arXiv:1603.04467, 2016` (bib-v1 mis-parse): bib-v1 title is the venue
  string and authors are `Martin Abadi et al. Tensorflow: ...`; the model claims are correct
  (`Tensorflow: Large-scale machine learning on heterogeneous distributed systems`, `Martin
  Abadi`) but rank below bib-v1, so the wrong value stays selected. This is Open question 1
  with data: for entries bib-v1 mis-splits, the model is the better source.
- `set`/`clear` on that stub: `set title "Scratch Override Title"` selected `user/explicit` and
  set `display_title`; `clear title` restored `arXiv preprint arXiv:1603.04467, 2016`
  (`display_title`, `known` checked in `graph.work`); `set year 19999` is refused
  (`year must be between 1400 and 2200`); `set authors "A. One; B. Two"` then `clear authors`
  restored the bib-v1 list. Only retracted scratch overrides remain (history is kept, A9).

## U4 implementation notes

Built as planned: appended provenance types and `api` methods, pure `web/src/lib/provenance.ts`
with `web/scripts/provenance.test.mjs` (13 tests, `test:provenance` after `test:relevance`),
`WorkProvenance.tsx` / `.css`, the one import and one JSX line in `WorkPanel`, the help-overlay
entry, and the scratch Playwright smoke. Commits: `5e4625c` (types, api, pure logic, tests),
`7a11b68` (component), `4db602b` (manual Tab handling), then the focus-ring fix and these docs.

Deviations (smallest sound changes):

- **Tab is moved by hand.** `useTabPhase` cancels every Tab at the document capture phase (it cycles
  reading phases), so the `PassageQuestion` style trap alone left focus stuck on the dialog. The
  dialog's `onKeyDown` now moves focus through its visible controls (buttons, inputs, textareas,
  summaries) itself and wraps. Found by the Playwright check, not by typecheck.
- **Esc/q arrive only through `LITERATURE_QUIT_EVENT`.** App's window-capture handler consumes the
  key before React sees it, so the capture listener on `document` also cancels an open edit first
  (Esc during an edit cancels the edit, a second press closes the dialog). The dialog's own Escape
  handler is a fallback outside the App.
- The dialog renders in a portal on `document.body` (the Work panel is `aside` inside an
  overflow-hidden overlay) with `z-index: 200`.
- The override editor starts pre-filled with the current value (the override, else the selected
  display); authors are one per line.
- Grouping key is `display`; an unreadable claim (`display: null`) groups under "(unreadable)".
- Extra pure helpers: `overrideDraft`; `parseOverrideInput` also rejects `https://` URLs that are
  not doi.org (they fail the `10.` check).
- A hidden "no claim" cell and "selected value / agrees with ... / excluded: reason" text use a
  local `.work-provenance-hidden` class (the app has no shared visually-hidden class).

Gate (worktree root, test DB from `scripts/db env`):

- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace --no-fail-fast`: root lib 367, root bin 26 + 1 + 1, core 95, db 110,
  ingest 286 passed, 0 failed (DB tests ran).
- web: `typecheck`, `lint`, `build` pass; `test:walk` 11, `test:ramp` 10, `test:relevance` 13,
  `test:literature-layout` 31, `test:provenance` 13 pass. `test:api` fails as on the base
  (`ERR_MODULE_NOT_FOUND` for `./relevance`), unrelated.

Playwright (`~/.cache/lysilogy-smoke/provenance-smoke.mjs . `, from `web/` after `npm run build`,
mocked `/api/*`): 29 checks pass, 0 console errors, 0 unexpected requests. It covers the authors line
(first six + `+2`), `s` opening a modal dialog with focus inside, columns Field, You, Publisher, The
paper, OpenAlex, Library file, Citing papers, selected/green/red cells (computed outline and tints),
grouped citing values with counts, Tab to Override, Esc cancelling an edit, Enter sending
`PUT .../provenance/title`, an inline year error sending nothing, Clear sending `DELETE`, Reject
sending `PUT /api/field-claims/<id>/review`, `j a x p w y R` doing nothing while open, Esc closing
only the dialog then the panel, and `q` closing only the dialog. Screenshot
`~/.cache/lysilogy-smoke/provenance.png`.

Live smoke (debug build of this branch, `serve` on 127.0.0.1:7397, dev DB, scratch data/notes dirs,
`~/.cache/lysilogy-smoke/live-provenance.mjs`): opened "Deep Reinforcement Learning from Human
Preferences", `L`, selected the stub `Superintelligence: paths, dangers, strategies`, `s`. The table
matched `lysilogy provenance show` row for row: title selected from OpenAlex with two agreeing
citing values (`Paths, Dangers, Strategies` x2); year 2015 selected from OpenAlex, citing 2014 x2 red;
authors `Nick Bostrom` selected from the citing column (x2); venue `Oxford University Press`;
DOI `10.5860/choice.187536` from OpenAlex. Overriding the year to 2019 made the You cell selected, the
row header `2019`, OpenAlex 2015 and citing 2014 red, and the graph label `... 2019` after reloading
the overlay. Clear restored 2015 in the table and the label. No console errors. The override left a
retracted `user` year claim on that stub (history, A9).

## Report

**Built.** Claims became tracked evidence: migration 0009 adds origin, extractor, confidence,
entry and retraction to `graph.field_claim` and `display_authors`/`authors_state` to `graph.work`.
One pure `select_field` (core) ranks claims by (origin, confidence) with the trust order user >
publisher > paper > OpenAlex > library file > citing papers, and one `reselect_in` recomputes
title, year and authors after every write. Harvest: bib-v1 claims for every resolved entry, OpenAlex
authorships, Crossref publisher records (own Work always, cited Works with `--publisher`),
deterministic front-matter claims, and an opt-in grounded Haiku pass (`--fields`) cached per entry.
API and CLI: provenance read, override set/clear, claim review, `provenance show|set|clear|reselect`.
UI: `display_authors` in every Work payload, the authors line and the Sources dialog.

**Gate.** Rust: clippy clean; tests root 367 + 26 + 1 + 1, core 95, db 110, ingest 286, 0 failed.
Web: typecheck, lint, build pass; walk 11, ramp 10, relevance 13, literature-layout 31, provenance 13
pass; `test:api` fails on the unmodified base too. Playwright 29/29.

**Dev DB after all units** (active claims, `retracted_at IS NULL`; 6,921 rows, 3 retracted scratch
overrides): openalex title 3,014, year 3,015, author 138, identifier 121 (`openalex/explicit`);
citing_bibliography bib-v1 title 127, year 124, author 53, venue 44 (heuristic), identifier 1
(explicit), bib-model-v1 title 22, year 25, author 22 (model); library_metadata title 106, year 98;
paper frontmatter-v1 title 2, frontmatter-model-v1 author 1; publisher crossref-v1 one each of title,
year, author, venue, identifier. Selected states across 3,461 Works: title known 3,099 / unknown 362;
year known 3,095 / unknown 366; authors known 170 / unknown 3,291. **Conflicting: 0 for title, year
and authors** (no Work has two disagreeing claims in its winning tier).

**Model cost proxy.** One `lit --fields` on the Christiano paper: 2 bibliography calls (32 entries,
batches of 20 and 12) plus 1 front-matter call, 37 s, 0 dropped, 0 failed, 21 entries skipped as
higher tier. Rerun: 0 calls, 32 + 1 cache hits, 1.0 s. Cache records: 32 `bibliography_fields_model`,
1 `paper_fields_model`. Only that one paper has been run through the model pass.

**Known gaps.**

1. Open question 1 stands with data: model claims rank below bib-v1, so bib-v1 mis-parses stay
   selected; and `paper` beats `openalex` regardless of confidence, so a model-read front-matter
   author order displaced OpenAlex's order for the Christiano paper.
2. Authors are known for 170 of 3,461 Works only: OpenAlex authorships fill in as summaries are
   refetched, and bib-v1 author claims exist for few entries until `lit --fields` runs per paper.
3. The Sources UI marks agreement per claim as the server computed it; the 6-column table is wide
   (scrolls horizontally under ~860px). No test renders the React component (the Playwright smoke is
   scratch, not in the repo), and screen-reader behavior is unverified.
4. Venue and DOI are selected on read only; a DOI override masks the displayed DOI but never changes
   identity. Crossref lookups for cited Works were exercised only against fixtures; real-DOI Crossref
   was checked for one paper ("The China Syndrome").
5. Out of scope and not done: author identity resolution, fuzzy title matching, re-resolving entries
   when a Work's selected title changes, OpenAlex venue, Semantic Scholar, a provenance page.
6. App-level keys such as `:` and `Q` are not blocked while the dialog is open (only graph keys are).
7. Open questions 2 (keep the Library file column) and 3 (make `--fields` the default) remain open.

## Review fixes

- `front_matter::confirmed_title` no longer confirms a catalog title hint that is only a prefix of
  the printed title. A hint is confirmed only when its `title_key` equals a whole block of
  consecutive non-blank front-matter lines that ends at a blank line or the end of the text. A
  file-name hint such as "Deep Reinforcement Learning" therefore no longer outranks OpenAlex's full
  title as a `paper`-tier claim. Regression test: `should_reject_title_when_hint_is_a_truncated_prefix_of_the_printed_title`.
- Known tradeoff: a title printed directly above the authors with no blank line between them is not
  confirmed (the fallback is the provider title). The literature test fixture now separates title
  and authors with a blank line.
