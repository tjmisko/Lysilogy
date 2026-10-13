# Ramp A finish: "Show me what this paper cites"

Status: planned 2026-10-12. Branch `feat/lit-ramp-a-finish`, based on
`integration/literature-viewer` at `9c886a1`. Parent plan:
[literature-viewer-plan.md](../literature-viewer-plan.md).

## What earlier work built

| Commit | What exists |
| --- | --- |
| `781d890`..`09389f4` | Workspace crates `lysilogy-core`, `-db`, `-ingest`; migration `0001_init` (all Ramp A tables), migrator. |
| `5642588` | Opaque IDs (`WorkId` serializes as `w` + 22 chars), state enums, `Work`/`Document` types. |
| `f71f343` | `WorkRepository`: `get`, `find_by_identifier`, `resolve_or_create`, `add_identifier`, `set_display_title/year`, `add_claim`. |
| `41faee1` | Move-tolerant scanner (`scan_library`, `scan_and_apply`) and `DocumentRepository::apply_scan` (full-library reconcile, tombstones missing paths). |
| `f65d434` | `identify::link_document_work`: extract DOI/arXiv from text, resolve/create the Work, link the document. |
| `4fd43a5` | `openalex`: `JsonTransport`, `parse_work`, `work_url`, `fetch_work_cached` (Postgres `source_record` cache, kind `openalex_work`), `ReqwestTransport`. |
| `deb6950` | `ingest_openalex_references`: referenced OpenAlex IDs become stub Works plus one `citation_assertion` each; idempotent. Migration `0002`. |
| `9da594e`, `9c886a1` | Optional `GraphState` on `AppState` (from `LYSILOGY_DATABASE_URL`); `GET /api/works/{id}`, `/api/works/{id}/citations`, `/api/graph/neighbors/{id}?direction=`. |

Baseline gate on this branch: `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo test --workspace` (with `LYSILOGY_TEST_DATABASE_URL`) are green.

Nothing is reachable by a user yet: no command runs the pieces in sequence, no
paper is ever linked to a Work outside tests, stub Works have no titles, and the
frontend has no graph surface.

## Scope

In:

1. `lysilogy lit <paper>`: scan the one file → link document to Work → fetch OpenAlex
   (cached) → ingest references → hydrate stub titles/years → print a summary.
2. The same flow as an API job, plus a paper → Work lookup endpoint.
3. Title fallback when the front matter has no usable identifier, accepted only on an
   exact normalized-title match, with provenance.
4. Consolidate the root `src/citation_graph/openalex.rs` parsing and contact-email
   handling onto `lysilogy_ingest::openalex`.
5. Reader `L` → literature graph overlay centered on the paper's Work, with local
   availability and a Work panel listing local citing papers.
6. Live smoke against the dev DB.

Out: AI edge labels, acquisition, relevance/marks/tags, multi-hop expansion, Work merging
(`work_redirect`), batch `lit --all`. No model-backed feature is introduced, so no sidecar
trait is needed in this ramp.

## Findings that shape the design

These were checked live on 2026-10-12 against the local library and OpenAlex.

1. **Full-text identifier extraction is wrong.** `link_document_work` runs
   `extract_identifiers` over whatever text it is given and prefers any DOI. In
   `Amodei et al. - 2016 - Concrete Problems in AI Safety.pdf` page 1 carries only
   `arXiv:1606.06565v2`, but the bibliography contains a DOI, so a full-text scan would
   link the paper to a cited work and attach every reference DOI to it. **Decision:**
   detect identifiers only in front matter (pages 1–2, cut at a References/Bibliography
   heading).
2. **OpenAlex mis-merges some arXiv records.** `doi:10.48550/arxiv.1606.06565` resolves to
   `W2462906003`, titled "Agnostic Learning with Unknown Utilities", with a LIPIcs DOI.
   `arxiv.2212.08073` (Bai 2022) resolves to an unrelated 2026 paper with 0 references.
   **Decision:** a provider record is accepted only when its title agrees with the local
   title (`title_similarity >= 0.6` when both titles are known). Otherwise the flow falls
   through to the title search and records a `provider_title_mismatch` warning.
3. **OpenAlex has duplicate records.** `title.search:the china syndrome` with year 2013
   returns two records with identical title and first author. **Decision:** if several
   candidates pass the exact-title, year and author checks, take the one with the most
   `referenced_works_count`, with the smallest OpenAlex ID breaking ties, and record the
   number of candidates considered. That is still a high-confidence match.
4. **pdftotext can glue tokens.** `Broome et al. - 2017` yields
   `10.1177/1354066117719320European`. The normalizer keeps the junk suffix, so the DOI
   fetch returns 404. A 404 is not an error: the flow falls through to the title search.
5. **Stub Works are untitled.** `referenced_works` carries only IDs. A fanout of
   untitled nodes is useless, so stubs are hydrated with a batched
   `filter=openalex:W1|W2|…` lookup (50 IDs per request, ~3 requests per paper). The
   old root `citation_graph` already does this; the batching logic moves into
   `lysilogy-ingest`.
6. **`apply_scan` is a full-library reconcile.** Passing it one file would tombstone
   every other document. `lit` needs a partial variant that never tombstones.
7. **Local availability requires Work identity to converge.** For a cited stub (with an
   OpenAlex ID) to show as local, a lit-linked local paper must land on that same Work.
   **Decision:** resolve the paper's Work in this order: provider OpenAlex ID, then
   DOI, then arXiv, and create a Work only when none exists. The flow then attaches all
   known identifiers on a best-effort basis (`add_identifier` returns `None` on a
   conflict, which is reported as a warning and never fails the flow).
8. **Smoke candidates.** The suggested Bai/Amodei papers have only arXiv IDs on page 1,
   and OpenAlex's records for them are wrong (finding 2). They serve as negative
   smokes, which must not link to the wrong Work. The positive smoke uses
   `Autor, Dorn, and Hanson - 2013 - The China Syndrome - ….pdf`: DOI
   `10.1257/aer.103.6.2121` → `W2097916756`, 115 references, title agrees.
9. **`L` is taken only in PDF Cursor mode** (`PdfSourceTools`: line-number cycling).
   The global handler checks `event.defaultPrevented`, so Cursor mode keeps `L`.

## Design

### Schema: migration `0003_document_work_link.sql`

```sql
ALTER TABLE reader.document
    ADD COLUMN work_link_method text
        CHECK (work_link_method IN ('identifier', 'title_match', 'manual')),
    ADD COLUMN work_link_source_record_id uuid
        REFERENCES source.source_record(id) ON DELETE SET NULL,
    ADD COLUMN work_link_detail text,
    ADD COLUMN work_linked_at timestamptz;
```

`work_link_detail` is human-readable provenance: `doi:10.1257/aer.103.6.2121`,
`arxiv:1606.06565`, or `title_key=<key>;candidates=2`. The link columns are cleared
whenever `work_id` is cleared.

New `source_record` kinds (no schema change):

| kind | source_key | payload |
| --- | --- | --- |
| `openalex_work` (existing) | `doi:<doi>` or `W…`, lowercased | full work with `referenced_works` |
| `openalex_work_summary` | `w…` lowercased | one result from the batched summary lookup |
| `openalex_title_search` | `<title_key>\|<year or ->` | whole search response |

`openalex_work_summary` must stay separate from `openalex_work`. A cached summary
has no `referenced_works`, and `fetch_work_cached` would read it as a work with zero
references.

### Modules

```
lysilogy-core
  title.rs            title_key, title_similarity (ported from d18ed24:src/kb/titles.rs)
  identifier.rs       + front_matter(pages) -> String
  names.rs (new,tiny) first_author_family(raw) -> Option<String>
lysilogy-db
  document.rs         + apply_partial_scan, set_work_link, work_link, local_documents_for_works
  work.rs             + add_claim_once, fill_display_title_if_unknown, fill_display_year_if_unknown
lysilogy-ingest
  openalex.rs         + OpenAlexSummary, parse_summary, works_batch_url, title_search_url,
                        fetch_work_cached_record, fetch_summaries_cached, search_title_cached,
                        contact_email_from_environment; short_openalex_id/normalize_doi pub
  title_match.rs      TitleQuery, TitleMatch, select_title_match (pure)
  ingest.rs           ingest_openalex_references -> ReferenceIngestReport; hydrate_stub_works
  literature.rs       LiteratureContext, LiteratureInput, LiteratureSummary, run_literature_flow
root crate
  src/literature.rs   AppState::run_literature_flow(&PaperId), front-matter text sourcing,
                      LiteratureJobs (in-memory job map)
  src/api/literature.rs  routes: /api/papers/{id}/work, /api/papers/{id}/lit
  src/main.rs         Command::Lit
  src/citation_graph/openalex.rs  parsing delegated to lysilogy_ingest::openalex
web
  src/components/LiteratureGraph.tsx (+ .css), src/lib/literatureLayout.ts
```

The root crate gains a `lysilogy-ingest` dependency.

### Flow (`lysilogy_ingest::literature::run_literature_flow`)

```
input: ScannedFile (hashed by caller), front_matter text, PaperHints{title, year, first_author}
1. workspace = ensure_default_workspace; document = apply_partial_scan(workspace, [file])
2. ids = extract_identifiers(front_matter); primary = DOI ?? arXiv
3. if primary: provider_key = doi:<doi> | doi:10.48550/arxiv.<id>
     fetch_work_cached_record(provider_key)
       404 / Malformed      -> provider = None, warn "provider_not_found"
       other error          -> return Err
     if provider && title_disagrees(hints.title, provider.title) -> warn
        "provider_title_mismatch", provider = None
     work = resolve_paper_work(openalex_id?, primary) ; attach identifiers
     set_work_link(document, work, Identifier, record?, "doi:…")
4. if provider is None and hints.title usable:
     search_title_cached(title, year) -> select_title_match
     Accepted(c): fetch_work_cached_record(c.openalex_id) -> provider
                  if no work yet: work = resolve_paper_work(c.openalex_id, None)
                     set_work_link(document, work, TitleMatch, search_record, "title_key=…;candidates=n")
                  else attach c.openalex_id to the existing work (warn on conflict)
     Rejected(reason): warn reason
5. if no work: return summary{status: Unmatched{reason}}
6. fill display title/year if unknown (from provider, else hints); claims via add_claim_once
7. if provider: report = ingest_openalex_references(...); hydrated = hydrate_stub_works(cited)
8. return LiteratureSummary
```

```rust
pub struct LiteratureSummary {
    pub status: LiteratureStatus,           // Linked | Unmatched { reason: String }
    pub document_id: DocumentId,
    pub work_id: Option<WorkId>,
    pub link_method: Option<WorkLinkMethod>,
    pub link_detail: Option<String>,
    pub openalex_id: Option<String>,
    pub references: usize,                  // edges asserted from the provider record
    pub new_stub_works: usize,              // Works created by this run's reference ingest
    pub hydrated_works: usize,
    pub warnings: Vec<String>,              // stable snake_case codes + short context
}
```

All of it is `Serialize`. The CLI prints it as text; the API returns it as JSON.

### Front-matter text sourcing (root, never writes artifacts)

The flow tries these sources in order and uses the first that has text on page 1:

1. A cached reading index: `source_index::load_cached(source, store.paper_dir(id))`,
   joining `tokens` with `page <= 2` by spaces. Token text avoids the index's UTF-16
   offsets.
2. A cached extraction: `store.load_extraction(id)`, using `pages[..2]`.
3. `pdftotext -f 1 -l 2 -enc UTF-8 <pdf> -` with a 30 s timeout, `kill_on_drop`, and an
   output bound of 4 MiB. The method is `PdfExtractor::extract_front_text`.

The text then passes through `lysilogy_core::identifier::front_matter`. Hints come from the
catalog's merged `PaperMetadata` (title, year, `authors[0]` →
`first_author_family`).

### API

| Route | Behavior |
| --- | --- |
| `GET /api/papers/{id}/work` | 200 `{paper_id, work: Work \| null, link: {method, detail, linked_at} \| null}`. 404 for an unknown paper, 503 `graph_unavailable`. Maps the paper to `reader.document` through the default workspace and the catalog `relative_path`. |
| `POST /api/papers/{id}/lit` | Starts the flow on a background task. 202 `{state: "running"}`. Returns the current state if a job is already running for that paper. 503 without a graph. |
| `GET /api/papers/{id}/lit` | `{state: "idle" \| "running" \| "succeeded" \| "failed", summary?, message?}` |
| `GET /api/works/{id}/citations`, `GET /api/graph/neighbors/{id}` | Additive field `local_papers: {[work_id]: [{paper_id, title}]}` covering every Work in the response. Papers missing from the catalog are skipped, and no paths are exposed. |

`LiteratureJobs` is an `Arc<Mutex<HashMap<PaperId, LiteratureJobState>>>` on `AppState`.
Its state is in memory only. Results persist in Postgres, so a lost job state only means
the UI re-reads `/work`.

### UI

- `L` (outside Cursor mode, when a paper is open) opens the `LiteratureGraph` overlay.
  `Esc`/`q` closes the Work panel first, then the overlay, and is wired into App's
  one-layer close chain.
- Data: `GET /api/papers/{id}/work`. With no Work, the overlay offers the lit flow:
  `r` or a button calls `POST /lit`, the overlay polls `GET /lit` every 1 s, and then
  reloads. With a Work, it fetches `GET /api/graph/neighbors/{work}?direction=references`.
- Layout: dependency-free SVG with a radial fanout in `literatureLayout.ts`. A pure
  function places the center node and the cited nodes on concentric rings (≤ 36 per
  ring), sorted by year then title. Labels are truncated titles plus the year. Untitled
  nodes show "Untitled (W…)". Local nodes get a filled `--signal` dot.
- Selection: `j`/`k` or the arrow keys cycle nodes, and `Enter`/`o` or a click opens the
  Work panel. The panel calls `GET /api/works/{id}/citations`. It lists "Local copy" when
  the node's `local_papers` is non-empty, and "Cited by local papers" (incoming Works with
  local papers) as buttons that open those papers through App's paper selection.
- Styling: existing tokens (`--panel`, `--line`, `--ink-soft`, `--signal`) and the
  panel/overlay idioms of `QueuePanel`/`ExperimentPanel`. Help overlay entry:
  `L — Literature graph: what this paper cites`.

### Consolidation of the old citation graph path

The root `src/citation_graph` stays. It serves `/api/papers/{id}/citation-graph` and the
`citation-graph` command, with multi-provider snapshots, rate budgets and a no-redirect
transport. Only the duplicated OpenAlex knowledge moves:

- `openalex.rs::openalex_id`, the DOI handling in `work()`, and title/year extraction
  delegate to `lysilogy_ingest::openalex::{short_openalex_id, normalize_doi,
  parse_summary}`.
- `GraphHttp::from_environment` and `ReqwestTransport::from_environment` both read the
  contact email through `lysilogy_ingest::openalex::contact_email_from_environment()`:
  `LYSILOGY_CONTACT_EMAIL`, falling back to the legacy `LYSILOGY_CITATION_MAILTO`, with
  blank values treated as absent. Without an email, requests are simply sent without
  `mailto`.
- Existing `src/citation_graph/tests.rs` must pass unchanged.

## Decisions taken

1. Identifier detection is limited to front matter, never the full text (finding 1).
2. A provider record is title-guarded before use. The threshold is
   `PROVIDER_TITLE_MIN_SIMILARITY = 0.6`, applied only when both titles are known
   (finding 2).
3. A title match requires `title_key` equality, a year within ±1 when both are known,
   and a matching first-author family key when both are known. Titles with fewer than 3
   key words, or matching `untitled`, are rejected as `title_too_generic`. Duplicate
   records are resolved by `referenced_works_count`, then the smallest ID (finding 3).
4. arXiv IDs are looked up in OpenAlex through the DataCite DOI
   `10.48550/arxiv.<id>`.
5. Port the title normalizer (`d18ed24:src/kb/titles.rs`) verbatim with its tests.
   Do **not** port the 900-line name parser (`464baf9:src/kb/names.rs`) yet. Ramp A only
   needs a first-author family key, which a small pure `first_author_family` provides.
   The full parser belongs with Ramp B bibliography resolution.
6. Link provenance lives in columns on `reader.document` rather than in a new table:
   one link per document, plus a history-free audit trail that is sufficient for Ramp A.
7. `lit` never writes file artifacts and never tombstones other documents.
8. A `lit` run that finds no Work is not an error in the API (`status: unmatched`).
   The CLI prints the summary and exits non-zero.
9. The contact email is read from the environment only (decision in the Consolidation
   section). No email is hardcoded, and tests never read the environment.

## Gate (every unit)

```sh
export LYSILOGY_TEST_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy_test
cargo clippy --workspace --all-targets -- -D warnings   # workspace lints: all+pedantic+nursery = deny
cargo test --workspace                                  # DB tests must run, not print "skipping"
# frontend units also: (cd web && npm run typecheck && npm run lint && npm run build)
```

`web/node_modules` is a symlink to `~/Projects/Lysilogy/web/node_modules`. Tests are named
`should_<behavior>_when_<condition>`. New DB tests use unique per-test markers, following
the existing ingest tests.

## Implementation units

### U1. Pure helpers: title normalizer, front matter, first author

Files: `Cargo.toml` (workspace dep `unicode-normalization = "0.1.25"`; it is already in
`Cargo.lock`), `crates/lysilogy-core/Cargo.toml`, `crates/lysilogy-core/src/{lib.rs,
title.rs, names.rs, identifier.rs}`.

- `title.rs`: port `git show d18ed24:src/kb/titles.rs` verbatim, including its tests,
  as `lysilogy_core::title`. Re-export `title_key` and `title_similarity`. Fix only what
  the stricter workspace clippy demands.
- `identifier.rs`: `pub fn front_matter(pages: &[&str]) -> String` joins pages 1–2 with
  `\n`, then truncates at the first line whose trimmed, lowercased text is `references`,
  `bibliography`, `works cited`, or `literature cited`, optionally numbered or with a
  trailing colon (`7 References`, `REFERENCES:`). The output is capped at 20 000 chars on a
  char boundary.
- `names.rs`: `pub fn first_author_family(raw: &str) -> Option<String>`. It takes the
  segment before the first `,`, ` and `, `&`, ` et al`, or `;`, then the last
  whitespace word of that segment. Trailing `.` is stripped. It returns `None` when the
  result is empty or not alphabetic. Examples: `Autor, Dorn, and Hanson`→`Autor`,
  `Bai et al.`→`Bai`, `David H. Autor`→`Autor`, `""`→`None`, `et al.`→`None`.
- Tests: the ported title tests; `should_stop_front_matter_when_references_heading_appears`,
  `should_keep_only_first_two_pages_when_given_many`,
  `should_ignore_references_word_inside_prose_when_not_a_heading`,
  `should_return_empty_front_matter_when_no_pages`,
  `should_extract_family_when_author_list_uses_commas_and_et_al`,
  `should_return_none_when_author_text_is_blank_or_only_et_al`, plus non-ASCII names
  (`Ţigănaşu, Bănică, and Wong`).

Acceptance: the gate is green and the new functions are pure, with no I/O.

### U2. DB: link provenance, partial scan, local lookup, idempotent claims

Files: `crates/lysilogy-db/migrations/0003_document_work_link.sql`,
`crates/lysilogy-db/src/{document.rs, work.rs}`, `crates/lysilogy-core/src/{state.rs,
document.rs}`.

- Core: `WorkLinkMethod { Identifier, TitleMatch, Manual }` with `as_str`/`FromStr`,
  following `state.rs` conventions. `Document` gains `work_link_method:
  Option<WorkLinkMethod>` and `work_link_detail: Option<String>`. Update
  `DOCUMENT_COLUMNS` and `document_from_row`.
- `DocumentRepository::apply_partial_scan(workspace, files) -> ScanReport`: identical to
  `apply_scan` without `state.tombstone`. Refactor `apply_scan` to share a private
  `reconcile_files(…, tombstone: bool)`.
- `set_work_link(document_id, work_id, method, source_record_id: Option<SourceRecordId>,
  detail: &str)` sets all four link columns plus `work_id` and `work_linked_at = now()`.
  The existing `set_work(id, None)` also nulls the link columns.
- `work_link(document_id) -> Option<WorkLink { work_id, method, detail, linked_at }>`.
- `find_document_id_by_path(workspace, relative_path)` is covered by `find_by_path`; use it.
- `local_documents_for_works(workspace, &[WorkId]) -> Vec<LocalDocument { work_id,
  document_id, relative_path: String }>` returns only rows with non-null `relative_path`.
  It uses `work_id = any($2)` and returns an empty vec immediately for an empty input.
- `WorkRepository::add_claim_once(...)` matches `add_claim` but inserts only when no row
  with the same `(work_id, field, value, source_record_id is not distinct from)` exists.
- `fill_display_title_if_unknown(work_id, title) -> bool` and
  `fill_display_year_if_unknown(work_id, year) -> bool` each run `update … where id=$1 and
  title_state <> 'known'` and return whether a row changed.
- Tests (DB-gated):
  `should_keep_other_documents_when_partial_scan_registers_one_file`,
  `should_record_link_provenance_when_document_is_linked`,
  `should_clear_link_provenance_when_work_is_unset`,
  `should_list_only_present_documents_when_looking_up_local_works` (tombstoned excluded),
  `should_return_empty_when_no_work_ids_given`,
  `should_not_duplicate_claim_when_added_twice`,
  `should_not_overwrite_known_title_when_filling_display_title`. Update the migration
  test's table expectations if needed.

Acceptance: the gate is green, migration 0003 applies on top of an existing test DB, and
the existing scan tests are unchanged and pass.

### U3. Ingest: OpenAlex summaries, title search, hydration, consolidation

Files: `crates/lysilogy-ingest/src/{openalex.rs, title_match.rs, ingest.rs, lib.rs}`,
root `Cargo.toml` (add `lysilogy-ingest` dep), `src/citation_graph/{openalex.rs, http.rs}`.

- `openalex.rs`:
  - Make `short_openalex_id` and `normalize_doi` `pub`.
  - `pub struct OpenAlexSummary { openalex_id, doi, title, year, first_author:
    Option<String>, referenced_works_count: Option<u32> }` and
    `pub fn parse_summary(&Value) -> Result<OpenAlexSummary, ProviderError>`.
    `first_author` comes from `authorships[0].author.display_name`. A missing or garbage
    field becomes `None`; only a missing `id` is `Malformed`.
  - `works_batch_url(ids: &[&str]) -> String` builds `filter=openalex:W1|W2`,
    `per_page=<len>`, and `select=id,doi,display_name,publication_year`. `debug_assert!`
    checks that the length is at most 50 (`SUMMARY_BATCH = 50`).
  - `title_search_url(title, year: Option<i32>) -> Option<String>` builds the query from
    the alphanumeric words of `title_key(title)` (dropping `,`, `|`, `:`) as
    `filter=title.search:<words>[,publication_year:<y-1>-<y+1>]`, `per_page=10`, and
    `select=id,doi,display_name,publication_year,referenced_works_count,authorships`.
    It returns `None` when no words remain.
  - `fetch_work_cached_record(records, client, id) -> Result<(SourceRecordId,
    OpenAlexWork), ProviderError>` is the existing cached fetch that also returns the
    record ID. `fetch_work_cached` delegates to it.
  - `fetch_summaries_cached(records, client, ids) -> Result<Vec<(SourceRecordId,
    OpenAlexSummary)>, ProviderError>` reads `openalex_work_summary` records first,
    fetches only missing IDs in chunks of 50, and upserts each result individually.
    Requested IDs that OpenAlex omits are skipped silently.
  - `search_title_cached(records, client, title, year) -> Result<Option<(SourceRecordId,
    Vec<OpenAlexSummary>)>, ProviderError>` caches under `openalex_title_search`.
  - `ProviderError::is_not_found()` is true for `Status(404)`.
  - `pub fn contact_email_from_environment() -> Option<String>` reads
    `LYSILOGY_CONTACT_EMAIL`, then `LYSILOGY_CITATION_MAILTO`, ignoring blank values.
    Test it through a pure `contact_email_from(lookup: impl Fn(&str) -> Option<String>)`,
    never by mutating env. `ReqwestTransport` uses it.
- `title_match.rs` (pure): `TitleQuery { title, year, first_author }`,
  `TitleMatch::{Accepted { candidate, considered: usize }, Rejected(TitleRejection)}`,
  `TitleRejection::{TitleTooGeneric, NoCandidates, NoExactTitle, YearMismatch,
  AuthorMismatch}` with `as_str()` snake_case, and `select_title_match(&TitleQuery,
  &[OpenAlexSummary]) -> TitleMatch` per decision 3. Also `pub fn
  titles_disagree(local: Option<&str>, provider: Option<&str>) -> bool`, which is true
  only when both titles are non-empty and `title_similarity < 0.6`.
- `ingest.rs`: `ingest_openalex_references` returns `ReferenceIngestReport { edges,
  new_works }`, counting `resolve_or_create`'s `created`. Update the existing test.
  `hydrate_stub_works(works, records, client, cited: &[Work]) -> Result<usize,
  …>` considers only Works with `title_state != Known`. For each, it looks up the Work's
  OpenAlex ID through a new `WorkRepository::identifiers(work_id)` (or a single query
  `openalex_ids_for(&[WorkId]) -> Vec<(WorkId, String)>`; add to `work.rs`). It then calls
  `fetch_summaries_cached`, sets the title/year with `add_claim_once` and `fill_*_if_unknown`
  (source = summary record), adds the DOI identifier on a best-effort basis, and returns
  the count of Works given a title. `IngestError` gains `Provider(#[from] ProviderError)`.
- Consolidation: `src/citation_graph/openalex.rs::openalex_id` and `work()` delegate ID,
  DOI, title and year parsing to `lysilogy_ingest::openalex`. Old `Identifier`/`Work` types
  are mapped from the shared parse, keeping the `authorships` list. `GraphHttp` reads
  mailto via `contact_email_from_environment()`. Remove now-dead helpers.
  `src/citation_graph/tests.rs` is unchanged and passing.
- Tests (fake transport, plus fixture JSON inline): `should_parse_summary_when_authorships_missing`,
  `should_reject_summary_when_id_missing`, `should_build_batch_url_when_given_ids`,
  `should_return_none_title_search_url_when_title_has_no_words`,
  `should_escape_filter_delimiters_when_title_contains_commas_and_pipes`,
  `should_accept_exact_title_when_year_within_one`,
  `should_pick_most_referenced_when_duplicate_records_match`,
  `should_reject_when_titles_differ_only_by_subtitle` (`The China Syndrome` vs `The China
  Syndrome Ails America Too`), `should_reject_when_title_too_generic`,
  `should_reject_when_first_author_differs`, `should_flag_disagreement_when_provider_title_unrelated`
  (Amodei vs "Agnostic Learning with Unknown Utilities"),
  `should_not_flag_disagreement_when_local_title_unknown`,
  `should_prefer_contact_email_when_both_variables_set`,
  `should_ignore_blank_contact_email_when_set_to_whitespace`. DB-gated tests:
  `should_fetch_only_uncached_summaries_when_some_are_cached` (counting transport, URL
  assertions), `should_title_stub_works_when_hydrating`,
  `should_count_new_stub_works_when_ingesting_references`.

Acceptance: the gate is green, `/api/papers/{id}/citation-graph` tests are unchanged, and
no new code reads env in tests.

### U4. The lit flow and the `lysilogy lit` command

Files: `crates/lysilogy-ingest/src/{literature.rs, identify.rs, lib.rs}`, `src/literature.rs`,
`src/lib.rs`, `src/extract.rs`, `src/main.rs`.

- `lysilogy_ingest::literature`:
  - `LiteratureContext<'a, T: JsonTransport> { works, documents, citations, records, client:
    &'a OpenAlexClient<T> }`
  - `LiteratureInput<'a> { file: ScannedFile, front_matter: &'a str, hints: PaperHints<'a> }`
  - `PaperHints` gains `first_author: Option<&'a str>`.
  - `LiteratureSummary`/`LiteratureStatus` as in Design, with `Serialize`.
  - `run_literature_flow` implements the Flow steps exactly.
  - `resolve_paper_work(works, openalex_id: Option<&str>, primary: Option<&DetectedIdentifier>)
    -> Result<Option<Work>>` follows the lookup order of finding 7.
  - `provider_key(&DetectedIdentifier) -> String` (pure; arXiv → `doi:10.48550/arxiv.<id>`).
- `identify.rs`: retire `link_document_work` in favor of the flow. Move its two DB tests
  to `literature.rs` as flow tests, or keep them as tests of `resolve_paper_work`.
  `primary_identifier` stays.
- Root `src/literature.rs`: `impl AppState { pub async fn run_literature_flow(&self, id:
  &PaperId) -> Result<LiteratureSummary> }`. The root error type maps a missing graph to
  `Error::InvalidRequest("set LYSILOGY_DATABASE_URL …")`. The method hashes the file with
  `lysilogy_ingest::scan::hash_file` and builds the `ScannedFile` from the catalog
  `relative_path`. Front-matter text follows the sourcing order above, with a
  `front_matter_text(&self, id) -> Result<String>` helper. The `OpenAlexClient` lives in
  `AppState` (constructed from `ReqwestTransport::from_environment()`), with
  `with_literature_client` for tests. Map `IngestError` into `crate::Error`.
- `PdfExtractor::extract_front_text(path, last_page: u32) -> Result<String>` runs
  `pdftotext -f 1 -l <n> -enc UTF-8`, bounded and with a timeout. Test it with the
  existing fake-program style used in `extract.rs` tests, if one exists. Otherwise a
  tempdir shell-script fake is acceptable.
- `main.rs`: `Command::Lit { paper: String }` resolves a path or a paper ID. If `paper`
  is an existing file, it is canonicalized and must lie under the canonical library root,
  and the flow uses the `PaperId::from_relative_path` of its relative path. Otherwise
  `resolve_paper` applies (paper ID or title fragment). The command prints:
  ```
  Work      w…  (identifier: doi:10.1257/aer.103.6.2121)
  OpenAlex  W2097916756
  References 115 (new stub works: 112, titled: 115)
  Warnings  provider_title_mismatch: …
  ```
  An unmatched result prints the reason and returns `Err` (exit 1).
- Tests (DB-gated, fake transport routing by URL substring):
  `should_link_and_ingest_references_when_front_matter_has_doi`,
  `should_be_idempotent_when_run_twice` (second run: `new_stub_works == 0`, same work,
  same edge count), `should_fall_back_to_title_search_when_provider_title_disagrees`,
  `should_fall_back_to_title_search_when_doi_returns_not_found`,
  `should_return_unmatched_when_no_identifier_and_title_generic`,
  `should_reuse_existing_stub_work_when_paper_was_already_cited` (pre-create a stub
  with the OpenAlex ID, then run lit on a DOI paper whose provider ID equals it;
  assert the same `WorkId`), `should_not_tombstone_other_documents_when_running_lit`,
  `should_ignore_reference_dois_when_they_appear_after_references_heading`. Root:
  `should_reject_path_outside_library_when_resolving_lit_target` (pure helper),
  `should_prefer_reading_index_text_when_cached` (tempdir artifact fixture).
- Live check (not a test): `export LYSILOGY_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy`.
  Then `cargo run -- --library ~/Projects/Lysilogy/local-articles --data
  ~/Projects/Lysilogy/.lysilogy lit "China Syndrome - Local"` prints the OpenAlex ID
  W2097916756 and 100+ references. Running it on `Concrete Problems in AI Safety` must
  not link to "Agnostic Learning…". `--data` is read-only for this command.

Acceptance: the gate is green and the live check behaves as stated, with output
recorded in the unit's commit message body or the ramp report.

### U5. API: lit job, paper→Work lookup, local availability

Files: `src/api/literature.rs` (new; routes merged in `build_router`), `src/api.rs`
(`AppState` field `literature_jobs`, `CitationsResponse`/`NeighborsResponse` gain
`local_papers`), `src/literature.rs`.

- `LiteratureJobs` holds `Arc<Mutex<HashMap<PaperId, LiteratureJobState>>>`, where
  `LiteratureJobState::{Running, Succeeded(LiteratureSummary), Failed(String)}`, with
  serialization as in Design. `start(&self, state: AppState, id)` is a no-op if a job is
  running; otherwise it spawns `run_literature_flow` with `tokio::spawn`.
- Handlers `get_paper_work`, `start_literature`, `literature_status` reuse `parse_id`,
  `graph_or_unavailable`, and `graph_error` codes (`paper_not_found` 404,
  `graph_unavailable` 503).
- `local_papers_for(state, &[WorkId]) -> BTreeMap<String, Vec<LocalPaper>>` combines
  `local_documents_for_works` (default workspace) with the catalog: `PaperId::from_relative_path`,
  then the catalog title. Entries not in the catalog are skipped.
- Tests (existing `api.rs` test style with `tower::ServiceExt::oneshot`; DB-gated where needed):
  `should_return_503_when_paper_work_requested_without_graph`,
  `should_return_null_work_when_paper_has_no_document_row`,
  `should_return_linked_work_when_document_is_linked`,
  `should_return_404_when_lit_requested_for_unknown_paper`,
  `should_report_running_then_succeeded_when_lit_job_completes` (fake transport injected via
  `with_literature_client`; poll the GET until done, with a bounded loop and
  `tokio::time::timeout`), `should_include_local_papers_when_neighbor_is_held_locally`,
  `should_omit_local_papers_when_document_tombstoned`. Existing neighbor and citation tests
  keep passing because the field is additive.

Acceptance: the gate is green. A manual `curl` against `lysilogy serve` with the dev DB
shows `/api/papers/<china-id>/work` returning the Work, and `/api/works/<w>/citations`
returning 100+ outgoing Works, most with titles.

### U6. Frontend graph surface, live smoke, ramp report

Files: `web/src/types.ts`, `web/src/lib/api.ts`, `web/src/lib/literatureLayout.ts`,
`web/src/components/LiteratureGraph.tsx`, `web/src/components/LiteratureGraph.css`,
`web/src/hooks/useGlobalKeys.ts` (`onLiterature` on `L`), `web/src/App.tsx` (state
`literatureOpen`, close chain, the `overlayHandlesTab`/guard lists, open paper callback),
`web/src/components/HelpOverlay.tsx`, and `docs/ramps/ramp-a.md` (append a "Result" section).

- Types: `Work`, `LocalPaper`, `WorkCitations`, `GraphNeighbors`, `PaperWork`,
  `LiteratureJob`, `LiteratureSummary`, mirroring the Rust JSON exactly.
- `api.ts`: `paperWork`, `startLiterature`, `literatureJob`, `workCitations`,
  `graphNeighbors(id, direction = "references")`.
- `literatureLayout.ts` is a pure function, `layoutFanout(centerId, nodes, {width, height})
  → {id, x, y, ring}[]`, deterministic and with at most 36 nodes per ring. Add a node test
  under `web/scripts/literature-layout.test.mjs` and the npm script
  `test:literature-layout`, following the existing `--experimental-strip-types` tests.
  Cases: 0, 1, 37, and 120 nodes; untitled nodes; duplicate IDs collapse.
- Overlay behavior is as in Design/UI: loading, error (`graph_unavailable` gives a clear
  message), no-Work offer (`r`), job polling with cleanup on unmount, graph, and Work panel.
  The open-paper action closes the overlay and selects the paper.
- Verify that `L` in PDF Cursor mode still cycles line numbers and does not open the
  overlay.
- Verification: `npm run typecheck`, `npm run lint`, `npm run build`, and
  `npm run test:literature-layout`. Write a scratch Playwright script under
  `$CLAUDE_JOB_DIR/tmp` (or `~/.cache/lysilogy-smoke/`). It imports
  `~/Projects/Lysilogy/web/node_modules/playwright/index.mjs`, serves `web/dist`, and mocks
  `/api/library`, `/api/papers/*`, `/api/papers/*/work`, `/api/papers/*/lit`, and
  `/api/graph/neighbors/*`/`/api/works/*/citations`. It asserts that `L` opens the
  overlay, that nodes render with titles, that the local indicator shows on a mocked
  local node, that clicking a node lists local citing papers, that the no-Work state
  offers and runs lit (mock flips after the POST), and that `Esc` closes. It saves
  screenshots to the scratch dir for review.
- Live smoke: `lysilogy serve` with the dev DB on a non-default port, `bind
  127.0.0.1:7391`. Run `curl` checks for the China Syndrome paper: `/work` gives a Work,
  `/api/graph/neighbors/<w>` gives 100+ nodes, and `POST /lit` on a second DOI paper
  (e.g. `Harrison - 2018 - …Macartney Embassy…`, DOI `10.1093/ehr/cey066`) reaches
  `succeeded`. Run lit on two library papers where one cites the other, if any exist, to
  observe `local_papers`. Otherwise document that local availability was only
  exercised in tests. Append a "Result" section to this doc with the commands, counts,
  and the Amodei/Bai negative outcome.

Acceptance: all frontend checks and the Rust gate are green, the Playwright script
passes, and the live smoke is recorded in this doc.

### U2 implementation notes

- `fill_display_year_if_unknown` guards on `year_state <> 'known'` (the plan text said
  `title_state`, which was a typo).
- `add_claim_once` returns `bool` (inserted or not); `openalex_ids_for` also returns early
  for empty input.
- `work_link` returns `None` unless `work_id`, method, and `work_linked_at` are all set.
