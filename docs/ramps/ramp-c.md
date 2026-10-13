# Ramp C: "Get me the ones I don't have" — open-access acquisition

Status: planned 2026-10-13. Branch `feat/lit-ramp-c-acquire`, based on
`integration/literature-viewer` at `eae14f6` (Ramp B merged). Parent plan:
[literature-viewer-plan.md](../literature-viewer-plan.md). Previous ramps:
[ramp-a.md](ramp-a.md), [ramp-b.md](ramp-b.md).

## What earlier work built

- Ramp A: `lysilogy lit <paper>` links a library PDF to a Work (front-matter DOI/arXiv,
  else exact title match) and ingests OpenAlex `referenced_works` as stub Works with
  hydrated titles. Stub Works carry an `openalex` identifier and usually a `doi`
  (arXiv preprints appear as the DataCite DOI `10.48550/arxiv.<id>`).
- Ramp B: `lit --local` resolves the PDF bibliography; local stubs can carry an `arxiv`
  or `doi` identifier taken from the printed entry.
- Schema `0001`–`0005`. `reader.blob` exists (`storage_key` unique, `sha256`, `bytes`,
  `media_type`) but nothing writes it. `reader.document_version.blob_id` exists and is
  always NULL. `ops` is an empty reserved schema.
- `DocumentRepository::apply_partial_scan` registers files without tombstoning;
  `set_work_link(document, work, method, record, detail)` records link provenance
  (`work_link_method` CHECK: `identifier | title_match | manual`).
- `local_documents_for_works` + the catalog give `local_papers` on
  `/api/graph/neighbors/{id}` and `/api/works/{id}/citations`; the `LiteratureGraph`
  Work panel lists "Local copy" with buttons that open the paper in the reader.
- The root crate already downloads PDFs for `POST /api/import` (`src/api.rs`
  `download_remote_pdf`): public-address-only client (`src/remote.rs`
  `public_http_client`, SSRF guard), manual redirects (≤ 5), 100 MiB cap, `%PDF-` in
  the first 1 KiB, files land in the library root.
- `lysilogy_ingest::openalex`: `JsonTransport`, `ReqwestTransport` (no timeout; appends
  `mailto` and `api_key` to **every** URL), `RecordingTransport` fake,
  `contact_email_from_environment()` (`LYSILOGY_CONTACT_EMAIL`, legacy
  `LYSILOGY_CITATION_MAILTO`).
- Sidecar pattern: `src/labeling.rs` `SidecarLabeler` over
  `LocalCliAnalyzer::reader_tool::<T>(provider, dir, ReaderToolRequest{schema, prompt,
  live_web})`, one directory per call, `structured_model_key(provider)`; the ingest
  crate owns the trait and a scripted fake.

Live dev DB (2026-10-13): 3,059 non-local Works with an OpenAlex ID, 2,556 with a DOI,
56 with an arXiv ID, plus many `10.48550/arxiv.*` DOIs (for example
`Cooperative Inverse Reinforcement Learning`, `10.48550/arxiv.1606.03137`, cited by the
Ramp A/B papers). OpenAlex for that Work returns
`best_oa_location.pdf_url = https://arxiv.org/pdf/1606.03137` and
`open_access.oa_url` the same. For China Syndrome (`W2097916756`) `best_oa_location`
is the AEA PDF and `locations[]` holds four more `pdf_url`s (NBER, IZA, ZORA).

## Scope

In:

1. Migration `0006_acquisition.sql`: `ops.acquisition` state table and the
   `acquisition` link method.
2. Deterministic locator: arXiv PDF for arXiv IDs (including `10.48550/arxiv.*` DOIs),
   OpenAlex `best_oa_location` / `open_access.oa_url` / OA `locations[]`, then
   Unpaywall when a contact email is configured. Landing pages are resolved by
   `citation_pdf_url` meta tags before any model.
3. Download with politeness (per-host throttle, timeouts, user agent), content-type and
   size checks, `%PDF-` magic, streaming SHA-256.
4. Content-addressed blob store under the data dir (`reader.blob`), a copy in the
   library under `Fetched/`, `document`/`document_version` through
   `apply_partial_scan`, `document_version.blob_id`, and a Work link with method
   `acquisition`.
5. Resumable state machine: `unresolved → identifier_found → link_found → downloaded →
   mapped`, else `unavailable{reason, checked_at}`; attempts, timestamps, `last_error`;
   re-runs skip completed stages.
6. Model fallback (off by default, `--model-fallback claude|codex`): pick a PDF link
   from a landing page's anchors, through a `LandingLinkPicker` trait with a sidecar and
   a scripted fake; the pick is grounded to the offered links.
7. CLI `lysilogy fetch <work>` and `lysilogy fetch --cited-by <paper> [--limit N]`.
8. API `POST /api/works/{id}/fetch`, `GET /api/works/{id}/acquisition`, additive
   `acquisitions` map on neighbors/citations.
9. UI: fetch action (`f`) in the Work panel for non-local Works, live status, and the
   fetched paper opens in the reader once mapped.
10. Live smoke: fetch an arXiv-cited Work end to end and open it through the API.

Out: paywalled access (proxies, institutional login, Sci-Hub-like sources — never),
EPUB/HTML full text, Crossref `link` (TDM links mostly need tokens; listed as a
follow-up), Semantic Scholar `openAccessPdf`, scheduled/background bulk acquisition,
dedup of the same article published as preprint and version-of-record (the Work model
already treats them as one Work), auto-running `lit` on fetched papers.

## Decisions taken

1. **State lives in `ops.acquisition`, one row per Work** (`work_id` primary key). The
   schema `ops` was reserved for jobs; acquisition is operational state about a Work, not
   evidence. Provider responses that informed it are `source.source_record` rows
   (evidence), as before.
2. **Candidate order (deterministic, first success wins):**
   1. `arxiv` — `https://arxiv.org/pdf/<id>` for an `arxiv` identifier, or derived from a
      `10.48550/arxiv.<id>` DOI. Versionless (latest).
   2. `openalex_best` — `best_oa_location.pdf_url`.
   3. `openalex_oa_url` — `open_access.oa_url`.
   4. `openalex_location` — every `locations[]` entry with `is_oa = true` and a
      `pdf_url`, in provider order.
   5. `unpaywall_best` — `best_oa_location.url_for_pdf`, then `unpaywall_location` for
      each `oa_locations[].url_for_pdf` (only with a contact email).
   6. Landing pages (`*.landing_page_url`, `url` without `url_for_pdf`) come after every
      PDF candidate, with origin `<origin>_landing`.

   Candidates are deduplicated by a normalized URL (scheme lowercased, `http` → `https`
   for `arxiv.org`, host lowercased, fragment dropped, `arxiv.org/abs/<id>` →
   `arxiv.org/pdf/<id>`). At most `MAX_CANDIDATES_PER_RUN = 6` are tried per run.
3. **OpenAlex OA lookups are a separate cache kind.** Ramp A's `openalex_work` select
   has no OA fields, and widening it would change cached payloads. New kind
   `openalex_oa`, key `w…` lowercased (else `doi:<doi>`), URL
   `…/works/<id>?select=id,doi,ids,open_access,best_oa_location,locations`. New kind
   `unpaywall`, key `<doi>`. A 404 is cached as `{"not_found": true}` so reruns do not
   re-ask; `--retry-unavailable` bypasses (re-fetches and upserts) both caches.
4. **Unpaywall only with a contact email.** Without `LYSILOGY_CONTACT_EMAIL` the
   Unpaywall step is skipped and the summary carries `unpaywall_skipped_no_email`.
   Nothing hardcodes an email. `ReqwestTransport` is fixed to append `mailto` and
   `api_key` **only** for `api.openalex.org` (today it would leak the OpenAlex API key to
   any host) and gains `connect_timeout(10 s)` / `timeout(30 s)`.
5. **Downloads reuse the root's public-address client.** OA URLs are third-party data,
   so the SSRF guard in `src/remote.rs` applies. `download_remote_pdf` moves from
   `src/api.rs` into `src/remote.rs` as a shared `download_public_resource`, and
   `/api/import` keeps its behavior (its tests pass unchanged). The ingest crate defines
   the `PdfFetcher` trait; the root implements it.
6. **What counts as a PDF.** `Content-Type` main type (parameters stripped, lowercase):
   `application/pdf`, `application/x-pdf`, `application/octet-stream`,
   `binary/octet-stream`, `application/download`, `application/force-download`, or
   absent → stream and require `%PDF-` within the first 1,024 bytes (the PDF spec's
   allowance, as `/api/import` does). `text/html` / `application/xhtml+xml` → read up to
   `MAX_HTML_BYTES = 2 MiB` as a landing page. Anything else is rejected without reading
   the body (`content_type:<type>`). Cap `MAX_PDF_BYTES = 100 MiB` (header and stream).
   Timeouts: connect 15 s, whole PDF request 120 s, HTML 30 s. Redirects ≤ 5, each hop
   re-checked against the public-address guard.
7. **Politeness.** `HostThrottle`: minimum interval between requests to one host —
   3 s for `arxiv.org` / `export.arxiv.org` (arXiv's guidance for automated access),
   1 s default; API hosts (`api.openalex.org`, `api.unpaywall.org`) 100 ms. All runs in
   one process share it. `fetch --cited-by` runs works sequentially; the API runs at most
   2 acquisition jobs at once (`tokio::sync::Semaphore`). User agent:
   `Lysilogy/<version>` plus ` (mailto:<email>)` when a contact email is configured.
8. **Errors are transient or permanent.** Transient: connect/timeout, HTTP 408/425/429,
   5xx, truncated body. Permanent: 401/403/404/410, not a PDF, too large, non-public
   address, bad redirect. A run where every tried candidate failed permanently ends
   `unavailable{all_links_failed}`. A run with any transient failure and no success keeps
   state `link_found`, increments `attempts`, sets `last_error`, and returns outcome
   `pending` so a rerun tries again.
9. **Unavailable reasons:** `no_identifier`, `no_oa_location`, `all_links_failed`.
   `unavailable` is sticky for `UNAVAILABLE_RECHECK_DAYS = 30` from `checked_at`; inside
   that window a rerun returns `skipped_unavailable` unless `--retry-unavailable`
   (API `retry_unavailable: true`), which restarts from `unresolved` and bypasses caches.
10. **Resumability.** Every stage writes its output to the row before the next stage
    starts, and each stage is skipped when the row is past it:
    - `identifier_found` stores `identifiers` (`{doi, arxiv, openalex}`).
    - `link_found` stores `candidates` (ordered `[{url, origin}]`).
    - `downloaded` stores `pdf_url`, `final_url`, `source` (origin), `sha256`, `bytes`,
      `blob_id`. On resume, the blob file must exist with the stored size; if not, the row
      falls back to `link_found` (warning `blob_missing`).
    - `mapped` stores `document_id`, `document_version_id`. On resume, if the document is
      tombstoned (`relative_path` NULL) it falls back to `downloaded` and is re-mapped.
    - `trials` (jsonb, newest last, capped at 20) logs `{url, origin, outcome, at}`;
      candidates whose last trial failed permanently are skipped on later runs unless
      retrying.
11. **Single runner per Work.** A lease (`lease_until`, 10 minutes) is claimed with one
    conditional `UPDATE … RETURNING`; a second concurrent run gets outcome `busy` and does
    nothing. The lease is cleared at the end of a run (also on error, best-effort).
12. **Already local short-circuits.** If the Work (after `resolve_redirect`) already has a
    present document, the row goes straight to `mapped` with that document and outcome
    `already_local`; nothing is fetched.
13. **Storage.** Blob path `<data>/blobs/sha256/<sha[0..2]>/<sha>.pdf`; `storage_key`
    `sha256/<sha[0..2]>/<sha>.pdf`; `media_type` `application/pdf`. Downloads stream into
    `<data>/blobs/tmp/<random>.part` (same filesystem) and are renamed into place; an
    existing blob with the same hash wins and the temp file is deleted. `reader.blob` is
    upserted by `storage_key`.
14. **The library gets a copy under `Fetched/`.** The reader and every paper ID are
    library-relative, so a fetched paper must appear in the catalog. The copy is
    `<library>/Fetched/<name>.pdf`, name `"<year> - <title>"` (parses through
    `metadata_from_filename` as year + title), else `"<title>"`, else `"<work id>"`;
    sanitized (no `/\:*?"<>|` or control characters, whitespace collapsed, ≤ 150 chars).
    If the path exists with the same SHA-256 it is reused; with different bytes, ` (2)` …
    suffixes. **Copy, not hard link**: a hard link would let in-place edits of the library
    file (annotation tools) silently change a content-addressed blob. `/api/import`
    already writes into the library, so this is not a new kind of side effect. A library
    document that already has the same SHA-256 is linked instead of writing a copy.
15. **Mapping uses the existing scanner identity.** `apply_partial_scan(workspace,
    [ScannedFile{relative_path: "Fetched/…", sha256, bytes, Pdf}])`, then
    `set_version_blob(version, blob)`, then `set_work_link(document, work,
    WorkLinkMethod::Acquisition, record, "acquired:<origin>:<host>")` where `record` is the
    `openalex_oa`/`unpaywall` record the winning candidate came from (NULL for arXiv
    derived from an identifier). A document already linked to a **different** Work is not
    relinked: the row is mapped to it with warning `document_linked_elsewhere:<work>`.
16. **Sanity check is a warning.** After mapping, the root reads pages 1–2
    (`extract_front_text`) and checks that ≥ 80 % of the Work's `title_key` words appear in
    the front matter's `title_key` (titles with < 3 key words are not checked). A miss adds
    warning `title_not_in_front_matter`; it never undoes the mapping (publisher cover
    pages and scanned PDFs make a hard gate too brittle; the warning is visible in the
    CLI/API summary).
17. **Model fallback only for landing pages, off by default.** When a candidate returns
    HTML: (a) deterministic `citation_pdf_url` meta, then `<link rel="alternate"
    type="application/pdf">`, then a single anchor whose href ends in `.pdf` and whose
    text or href contains `pdf`/`download`; (b) only if those find nothing and a picker
    is configured, the model sees the Work title/year, the page URL and at most 150
    absolute http(s) anchors (`[i] text — href`, text clipped to 160 chars) and returns
    `{pdf_link_index: integer | null, reason}`. The index must be in range (grounding,
    like Ramp B quotes); out of range counts as `pick_rejected`. One extra hop per landing
    page; the hop's result is never sent to the model again. Picks are cached as
    `source_record(kind='acquisition_link_pick', key='<model_key>|<sha256 of prompt>')`.
18. **`--cited-by` is bounded and deterministic.** Targets are the paper's Work's
    outgoing Works without a local copy, ordered: has an arXiv ID (direct or DataCite
    DOI), then has a DOI, then `display_year` desc (nulls last), then title. Works already
    `mapped`, or `unavailable` inside the recheck window, are skipped and counted, not
    charged to `--limit` (default 10, max 100). The paper must already have a Work (run
    `lit` first); otherwise the command fails with that instruction.
19. **The catalog refreshes after a mapping** (`AppState::refresh`) so the paper is
    immediately openable and appears in `local_papers`.
20. Unit tests never touch the network, the real library, or real model CLIs; fetchers,
    transports and pickers are fakes; filesystem tests use `tempfile`.

## Design

### Schema: `0006_acquisition.sql`

```sql
ALTER TABLE reader.document DROP CONSTRAINT document_work_link_method_check;
ALTER TABLE reader.document ADD CONSTRAINT document_work_link_method_check
    CHECK (work_link_method IN ('identifier', 'title_match', 'manual', 'acquisition'));

CREATE TABLE ops.acquisition (
    work_id uuid PRIMARY KEY REFERENCES graph.work(id) ON DELETE CASCADE,
    state text NOT NULL DEFAULT 'unresolved'
        CHECK (state IN ('unresolved', 'identifier_found', 'link_found',
                         'downloaded', 'mapped', 'unavailable')),
    unavailable_reason text
        CHECK (unavailable_reason IN ('no_identifier', 'no_oa_location', 'all_links_failed')),
    identifiers jsonb,                       -- {doi, arxiv, openalex}
    candidates jsonb NOT NULL DEFAULT '[]',  -- [{url, origin, source_record_id?}]
    trials jsonb NOT NULL DEFAULT '[]',      -- [{url, origin, outcome, at}], newest last, <= 20
    source text,                             -- origin of the winning candidate
    pdf_url text,
    final_url text,
    sha256 text,
    bytes bigint CHECK (bytes >= 0),
    blob_id uuid REFERENCES reader.blob(id) ON DELETE SET NULL,
    document_id uuid REFERENCES reader.document(id) ON DELETE SET NULL,
    document_version_id uuid REFERENCES reader.document_version(id) ON DELETE SET NULL,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error text,
    checked_at timestamptz,                  -- last time a run reached a verdict
    lease_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((state = 'unavailable') = (unavailable_reason IS NOT NULL))
);
CREATE INDEX acquisition_state ON ops.acquisition (state);
```

Verify the generated constraint name with `\d reader.document` before writing the
`DROP CONSTRAINT` (Postgres names it `document_work_link_method_check`).

New `source_record` kinds: `openalex_oa`, `unpaywall`, `acquisition_link_pick`.

### Modules

```
lysilogy-core
  state.rs        + AcquisitionState, UnavailableReason, CandidateOrigin (string_enum!),
                    WorkLinkMethod::Acquisition
lysilogy-db
  acquisition.rs  (new) AcquisitionRepository, AcquisitionRow, CandidateRecord, TrialRecord
  document.rs     + upsert_blob, set_version_blob, present_document_for_sha,
                    document_presence
  work.rs         + active_identifiers
lysilogy-ingest
  openalex.rs     ReqwestTransport: host-scoped mailto/api_key, timeouts
  acquire/mod.rs        pub use; AcquisitionContext, AcquisitionOptions, AcquisitionSummary,
                        AcquisitionOutcome, run_acquisition
  acquire/locate.rs     AcquisitionIdentifiers, LinkCandidate, identifiers_from,
                        arxiv_pdf_url, parse_openalex_oa, parse_unpaywall,
                        openalex_oa_url, unpaywall_url, normalize_candidate_url,
                        rank_candidates, fetch_openalex_oa_cached, fetch_unpaywall_cached
  acquire/landing.rs    PageLink, pdf_link_from_meta, page_links
  acquire/fetch.rs      PdfFetcher, FetchRequest, FetchedResource, FetchError,
                        ContentClass, classify_content_type, has_pdf_magic,
                        HostThrottle, ScriptedFetcher (cfg(test) pub(crate) or test_support)
  acquire/store.rs      blob_storage_key, blob_path, store_blob, fetched_filename,
                        place_library_copy
  acquire/picker.rs     LandingLinkPicker, LinkPickRequest, LinkPick, PickerError,
                        LINK_PICK_SCHEMA, link_pick_prompt, link_pick_cache_key,
                        ground_pick, ScriptedLinkPicker
  acquire/verify.rs     title_appears_in(title, text) -> Option<bool>
root
  src/remote.rs              + download_public_resource (moved from api.rs)
  src/acquisition.rs         PublicPdfFetcher (PdfFetcher), AppState::acquire_work,
                             AppState::acquisition_targets_cited_by
  src/acquisition/picker.rs  SidecarLinkPicker
  src/graph.rs               + acquisitions: AcquisitionRepository
  src/main.rs                Command::Fetch
  src/api/acquisition.rs     routes, AcquisitionJobs
  src/api.rs                 build_router merge; additive `acquisitions` on responses
web
  src/types.ts, src/lib/api.ts, src/lib/acquisition.ts (+ scripts/acquisition.test.mjs),
  src/components/LiteratureGraph.tsx (+ .css), src/components/HelpOverlay.tsx
```

### Core types (ingest `acquire`)

```rust
pub struct AcquisitionIdentifiers { pub doi: Option<String>, pub arxiv: Option<String>,
                                    pub openalex: Option<String> }
pub struct LinkCandidate { pub url: String, pub origin: CandidateOrigin,
                           pub source_record_id: Option<SourceRecordId> }

pub struct FetchRequest<'a> { pub url: &'a str, pub temp_dir: &'a Path }
pub enum FetchedResource {
    Pdf { final_url: String, temp_path: PathBuf, sha256: String, bytes: i64 },
    Html { final_url: String, body: String },          // body capped at MAX_HTML_BYTES
}
pub enum FetchError { Transient(String), Permanent(String) }
pub trait PdfFetcher: Send + Sync {
    fn fetch<'a>(&'a self, request: FetchRequest<'a>) -> FetchFuture<'a>;
}

pub trait LandingLinkPicker: Send + Sync {
    fn model_key(&self) -> &str;
    fn pick<'a>(&'a self, request: &'a LinkPickRequest) -> PickFuture<'a>;
}
pub struct LinkPickRequest { pub work_title: Option<String>, pub work_year: Option<i32>,
                             pub page_url: String, pub links: Vec<PageLink> }
#[derive(Deserialize, Serialize)]
pub struct LinkPick { pub pdf_link_index: Option<usize>, pub reason: String }

pub struct AcquisitionContext<'a, T: JsonTransport> {
    pub works: &'a WorkRepository, pub documents: &'a DocumentRepository,
    pub records: &'a SourceRecordRepository, pub acquisitions: &'a AcquisitionRepository,
    pub openalex: &'a OpenAlexClient<T>, pub fetcher: &'a dyn PdfFetcher,
    pub throttle: &'a HostThrottle, pub picker: Option<&'a dyn LandingLinkPicker>,
    pub contact_email: Option<&'a str>, pub blob_root: &'a Path, pub library_root: &'a Path,
}
pub struct AcquisitionOptions { pub retry_unavailable: bool }

pub enum AcquisitionOutcome {          // serde tag = "kind", snake_case
    Mapped, AlreadyMapped, AlreadyLocal,
    Unavailable { reason: UnavailableReason },
    SkippedUnavailable { reason: UnavailableReason, checked_at: DateTime<Utc> },
    Pending { error: String },          // transient failure; rerun resumes
    Busy,
}
pub struct AcquisitionSummary {
    pub work_id: WorkId, pub outcome: AcquisitionOutcome, pub state: AcquisitionState,
    pub stages: Vec<AcquisitionState>,  // stages reached by this run, in order
    pub candidates: usize, pub tried: usize, pub source: Option<CandidateOrigin>,
    pub pdf_url: Option<String>, pub sha256: Option<String>, pub bytes: Option<i64>,
    pub document_id: Option<DocumentId>, pub relative_path: Option<String>,
    pub warnings: Vec<String>,          // stable snake_case codes + short context
}
pub async fn run_acquisition<T: JsonTransport>(ctx: &AcquisitionContext<'_, T>,
    work: WorkId, options: AcquisitionOptions) -> Result<AcquisitionSummary>;
```

### Flow (`run_acquisition`)

```
work = resolve_redirect(work); Work must exist (else IngestError::Db(NotFound))
claim lease (else outcome Busy); ensure row
row = load
if row.state == mapped and document present          -> AlreadyMapped
if row.state == mapped and document tombstoned       -> state := downloaded (re-map)
if row.state == unavailable:
   if !retry and checked_at + 30d > now              -> SkippedUnavailable
   reset row to unresolved (keep attempts, trials)
if work has a present local document                 -> mark_mapped(doc), AlreadyLocal
if state == unresolved:
   ids = identifiers_from(active_identifiers(work))
   none -> mark_unavailable(no_identifier) -> Unavailable
   record_identifiers -> identifier_found
if state == identifier_found:
   candidates = arxiv(ids) ++ openalex_oa(ids) ++ unpaywall(ids, email) ; rank/dedupe
   provider transient error and no candidates        -> record_failure -> Pending
   empty                                             -> mark_unavailable(no_oa_location)
   record_candidates -> link_found
if state == link_found:
   for c in candidates (skip permanent-failed unless retry; <= 6 tries):
      throttle.wait(host(c)); fetcher.fetch(c)
      Pdf  -> store_blob; record_download -> downloaded; break
      Html -> link = pdf_link_from_meta(body, final_url)
                     ?? (picker? pick(page_links(body)) grounded)
              link -> one hop fetch (origin landing_meta | model); Pdf -> as above
      error -> push trial
   no success: any transient -> record_failure -> Pending
               else          -> mark_unavailable(all_links_failed)
if state == downloaded:
   verify blob file size (else state := link_found, warning blob_missing, return Pending)
   existing = present_document_for_sha(workspace, sha)
   doc = existing ?? place_library_copy + apply_partial_scan
   set_version_blob; set_work_link unless linked to another Work (warning)
   mark_mapped -> Mapped
release lease
```

`record_failure` and `mark_*` set `checked_at = now()` for verdicts, `attempts += 1` once
per run that performed network work, and `updated_at = now()`.

### CLI

```
lysilogy fetch <work>                      # work: w… id, OpenAlex W…, doi:…, arxiv:…
lysilogy fetch --cited-by <paper> [--limit 10]
   [--retry-unavailable] [--model-fallback claude|codex]
```

Single-work output:

```
Work       wXXXX  Cooperative Inverse Reinforcement Learning (2016)
Outcome    mapped (source arxiv)
Stages     identifier_found → link_found → downloaded → mapped
PDF        https://arxiv.org/pdf/1606.03137  (1.2 MiB, sha256 3f2a…)
Paper      7c1d…  Fetched/2016 - Cooperative Inverse Reinforcement Learning.pdf
Warnings   unpaywall_skipped_no_email
```

`--cited-by` prints one line per target (`<outcome> <title> <detail>`) and a total line:

```
Fetched 3 · already local 5 · unavailable 4 (no_oa_location 3, all_links_failed 1) · pending 1 · skipped 2
```

Exit status: 0 unless a run returned an error (DB/IO); `unavailable` and `pending` are
results, not errors. With no database the command fails like `lit`.

### API

| Route | Behavior |
| --- | --- |
| `POST /api/works/{id}/fetch` | Optional body `{retry_unavailable?: bool, model_fallback?: "claude" \| "codex"}` (`deny_unknown_fields`). 202 `{state: "running"}`, or the current job state if one is running for that Work. 400 `invalid_work_id` / `invalid_body`, 404 unknown Work, 503 `graph_unavailable`. |
| `GET /api/works/{id}/acquisition` | 200 `{work_id, job: {state: "idle" \| "running" \| "succeeded" \| "failed", summary?, message?}, acquisition: AcquisitionView \| null, paper_id: string \| null}`. `paper_id` only when the mapped document's `relative_path` is in the catalog. No paths or blob locations exposed. |
| `GET /api/graph/neighbors/{id}`, `GET /api/works/{id}/citations` | Additive `acquisitions: {[work_id]: {state, unavailable_reason}}` for every Work in the response that has a row. |

```ts
AcquisitionView = { state: AcquisitionState, unavailable_reason: string | null,
  checked_at: string | null, attempts: number, last_error: string | null,
  source: string | null, pdf_url: string | null, bytes: number | null, updated_at: string }
```

Jobs: `AcquisitionJobs(Arc<Mutex<HashMap<WorkId, AcquisitionJobState>>>)` plus a shared
`Arc<Semaphore>` with 2 permits, mirroring `LiteratureJobs`. Job state is in memory; the
row is the durable truth.

### UI

- Work panel, "Local copy" section, when the Work has no local paper: a status line from
  `acquisitions[work.id]` (or "Not fetched yet") and a **Fetch open-access copy (f)**
  button. Unavailable shows the reason in words and the check date, and the button
  becomes **Retry (f)** which sends `retry_unavailable: true`.
- `f` (Work panel open, no label editor) or the button → `POST /fetch`, then poll
  `GET /acquisition` every 1 s while `job.state === "running"`. Status text by state:
  Resolving identifiers… / Looking for an open-access copy… / Downloading… / Saving to
  the library… / Fetched. Polling stops when the panel closes or the overlay unmounts.
- On `mapped` with a `paper_id`: reload neighbors and citations (so `local_papers` and the
  node's local dot update) and show **Open in reader** (button, focused), which calls the
  existing `onOpenPaper`.
- Nodes: `data-acquisition="<state>"`; unavailable nodes get a muted outline, running
  fetches a pulsing ring (CSS only, respects `prefers-reduced-motion`).
- Help overlay: `f — Fetch an open-access copy of the selected work`.
- Pure helper `web/src/lib/acquisition.ts`: `acquisitionStatusText(view, jobState)`,
  `canFetch(workId, localPapers, acquisitions)`, `unavailableReasonText(reason)`.

## Gate (every unit)

```sh
export LYSILOGY_TEST_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy_test
cargo clippy --workspace --all-targets -- -D warnings   # workspace lints: all+pedantic+nursery = deny
cargo test --workspace                                  # DB tests must run, not print "skipping"
# frontend units also: (cd web && npm run typecheck && npm run lint && npm run build)
```

- `web/node_modules` is a symlink to `~/Projects/Lysilogy/web/node_modules` (created).
- Tests are named `should_<behavior>_when_<condition>`; DB tests use
  `test_support::lock()` / `pool()` with unique per-test markers.
- Bounded integer conversions use plain `as` casts at the call site (narrowest
  `#[allow(clippy::cast_…)]` if clippy objects). No new fallible conversion paths.
- No build caches in `/tmp`; scratch under `$CLAUDE_JOB_DIR/tmp` or `~/.cache/`.

## Implementation units

### U1. Schema, core enums, and repositories

Files:
- `crates/lysilogy-db/migrations/0006_acquisition.sql` (as in Design).
- `crates/lysilogy-core/src/state.rs`, `crates/lysilogy-core/src/lib.rs` (re-exports).
- `crates/lysilogy-db/src/acquisition.rs` (new), `crates/lysilogy-db/src/lib.rs`,
  `crates/lysilogy-db/src/document.rs`, `crates/lysilogy-db/src/work.rs`.

Core (`string_enum!`):
- `AcquisitionState { Unresolved, IdentifierFound, LinkFound, Downloaded, Mapped,
  Unavailable }` with a `const fn rank(self) -> u8` (0..=4; `Unavailable` = 5) used for
  "is past stage" checks.
- `UnavailableReason { NoIdentifier, NoOaLocation, AllLinksFailed }`.
- `CandidateOrigin { Arxiv, OpenalexBest, OpenalexOaUrl, OpenalexLocation,
  UnpaywallBest, UnpaywallLocation, OpenalexLanding, UnpaywallLanding, LandingMeta,
  Model }`.
- `WorkLinkMethod::Acquisition => "acquisition"`.

DB, `AcquisitionRepository` (all `updated_at = now()`):
- `claim(work) -> Result<bool>`: insert the row if missing, then
  `update … set lease_until = now() + interval '10 minutes' where work_id = $1 and
  (lease_until is null or lease_until < now()) returning work_id`.
- `release(work)`; `get(work) -> Option<AcquisitionRow>`;
  `states_for(&[WorkId]) -> Vec<(WorkId, AcquisitionState, Option<UnavailableReason>)>`.
- `reset_to_unresolved(work)` (clears identifiers, candidates, download and mapping
  fields, reason; keeps `attempts`, `trials`).
- `record_identifiers(work, &serde_json::Value)`; `record_candidates(work,
  &[CandidateRecord])`; `push_trial(work, TrialRecord)` (append, keep the last 20 in SQL:
  `jsonb_path_query_array` or rebuild in Rust after a `select … for update` — either is
  fine, test the cap).
- `record_download(work, DownloadRecord{source, pdf_url, final_url, sha256, bytes,
  blob})`, `mark_mapped(work, document, version: Option<DocumentVersionId>)`,
  `mark_unavailable(work, reason)` (sets `checked_at`), `record_failure(work, error)`
  (`attempts += 1`, `last_error`), `set_state(work, AcquisitionState)` for the
  fall-backs, `bump_attempts(work)`.
- `AcquisitionRow` mirrors the columns with typed fields (`state`,
  `unavailable_reason`, `candidates: Vec<CandidateRecord>`, `trials: Vec<TrialRecord>`,
  `blob_id: Option<BlobId>`, …), all `Serialize`.

DB, `DocumentRepository`:
- `upsert_blob(storage_key, sha256, bytes, media_type) -> BlobId` (`on conflict
  (storage_key) do update set storage_key = excluded.storage_key returning id`).
- `set_version_blob(version, blob)`.
- `present_document_for_sha(workspace, sha256) -> Option<(DocumentId,
  DocumentVersionId, String /*relative_path*/, Option<WorkId>)>` (present = path not
  NULL; deterministic order by path).
- `document_presence(document) -> Option<Option<String>>` (row exists → its path).

DB, `WorkRepository::active_identifiers(work) -> Vec<WorkIdentifier>` (status `active`,
ordered by scheme, created_at), reusing `identifier_from_row`.

Tests (DB-gated unless noted):
- core: `should_round_trip_acquisition_state_when_parsing_strings`,
  `should_order_stage_ranks_when_comparing_states`,
  `should_reject_unknown_unavailable_reason_when_parsing` (pure).
- `should_insert_row_and_grant_lease_when_claiming_first_time`
- `should_refuse_second_claim_when_lease_is_live`
- `should_grant_claim_when_lease_expired` (set `lease_until` in the past via SQL)
- `should_store_reason_and_checked_at_when_marking_unavailable`
- `should_violate_check_when_unavailable_without_reason` (raw SQL insert must fail)
- `should_keep_last_twenty_trials_when_pushing_more`
- `should_increment_attempts_and_keep_state_when_recording_failure`
- `should_clear_stage_outputs_and_keep_attempts_when_resetting`
- `should_return_same_blob_id_when_upserting_same_storage_key_twice`
- `should_find_present_document_when_sha_matches_and_ignore_tombstoned`
- `should_accept_acquisition_link_method_when_setting_work_link`
- `should_list_only_active_identifiers_when_work_has_conflicting_ones`
- `should_return_states_only_for_works_with_rows`

Acceptance: gate green; `scripts/db test-psql -c '\d ops.acquisition'` shows the table;
migration applies cleanly on the dev DB on next connect.

### U2. Deterministic locator: identifiers, OpenAlex/Unpaywall/arXiv candidates, landing pages

Files:
- `crates/lysilogy-ingest/src/acquire/{mod.rs, locate.rs, landing.rs}` (new; `mod.rs`
  only declares modules and re-exports in this unit), `crates/lysilogy-ingest/src/lib.rs`.
- `crates/lysilogy-ingest/src/openalex.rs` (transport fix).
- `crates/lysilogy-ingest/tests/fixtures/acquire/` (new): trimmed real responses —
  `openalex_oa_arxiv.json` (W2410842990), `openalex_oa_china.json` (W2097916756),
  `unpaywall_ok.json`, `unpaywall_no_oa.json`, `landing_highwire.html`,
  `landing_anchor_only.html`, `landing_none.html`. Fetch the OpenAlex ones live once
  (`curl` with the select above) and trim; write the Unpaywall/HTML ones by hand from
  the documented shapes. No email in any fixture.

`locate.rs`:
- `identifiers_from(&[WorkIdentifier]) -> AcquisitionIdentifiers`: first active `doi`,
  `arxiv`, `openalex`; an arXiv ID is derived from a `10.48550/arxiv.<id>` DOI when no
  `arxiv` identifier exists (normalize through `lysilogy_core::identifier::normalize`).
  `is_empty()`.
- `arxiv_pdf_url(id) -> Option<String>`: new-style `NNNN.NNNNN(vN)?` and old-style
  `archive(.sub)?/NNNNNNN` → `https://arxiv.org/pdf/<id>`; anything else `None`.
- `openalex_oa_url(&AcquisitionIdentifiers) -> Option<String>` (prefers the `W…` ID,
  else `https://doi.org/<doi>`, percent-encoded like `work_url`).
- `unpaywall_url(doi, email) -> String` (`https://api.unpaywall.org/v2/<doi>?email=<email>`,
  DOI percent-encoded except `/`).
- `parse_openalex_oa(&Value, Option<SourceRecordId>) -> Vec<LinkCandidate>` and
  `parse_unpaywall(&Value, …)` per decision 2; ignore non-http(s) URLs and non-strings;
  `{"not_found": true}` → empty.
- `normalize_candidate_url(&str) -> Option<String>` and
  `rank_candidates(Vec<LinkCandidate>) -> Vec<LinkCandidate>` (stable order by origin
  rank, dedupe by normalized URL keeping the higher-ranked origin).
- `fetch_openalex_oa_cached(records, client, ids, refresh) -> Result<Option<(SourceRecordId,
  Value)>, ProviderError>` (kind `openalex_oa`; 404 → upsert `{"not_found": true}`) and
  `fetch_unpaywall_cached(records, client, doi, email, refresh)` (kind `unpaywall`; same
  404 rule). `refresh = true` skips the cache read.
- Constants `OPENALEX_OA_KIND`, `UNPAYWALL_KIND`.

`landing.rs`:
- `PageLink { href: String, text: String }` (`Serialize/Deserialize/Clone/Debug/PartialEq`).
- `pdf_link_from_meta(html, base_url) -> Option<String>`: `<meta name="citation_pdf_url"
  content=…>` (any attribute order, single or double quotes, case-insensitive), then
  `<link rel="alternate" type="application/pdf" href=…>`, then the unique-anchor rule of
  decision 17(a). Resolves relative URLs against `base_url` (`reqwest::Url::join`), keeps
  only http(s), decodes `&amp;`.
- `page_links(html, base_url, limit) -> Vec<PageLink>`: absolute http(s) anchors,
  deduped by href, text whitespace-collapsed, tags stripped, clipped to 160 chars,
  `javascript:`/`mailto:`/`#` dropped. Regex-based (the crate already has `regex`); the
  input is capped by the caller.

`openalex.rs` `ReqwestTransport`:
- Append `mailto`/`api_key` only when the URL host is `api.openalex.org`.
- `connect_timeout(Duration::from_secs(10))`, `timeout(Duration::from_secs(30))`.
- Extract the query decoration into a pure `decorate_url(url, mailto, api_key) ->
  Result<Url, ProviderError>` so it can be tested.

Tests (pure unless noted):
- `should_derive_arxiv_id_when_doi_is_datacite_arxiv`
- `should_prefer_explicit_arxiv_identifier_when_both_present`
- `should_return_empty_identifiers_when_work_has_none`
- `should_build_arxiv_pdf_url_when_id_is_new_or_old_style`
- `should_reject_arxiv_pdf_url_when_id_is_malformed` (`"../etc"`, `""`, `"1234"`)
- `should_rank_arxiv_before_openalex_and_landing_last_when_ranking`
- `should_dedupe_abs_and_pdf_arxiv_urls_when_normalizing`
- `should_read_best_location_oa_url_and_oa_locations_when_parsing_openalex_fixture`
- `should_skip_non_oa_and_non_http_locations_when_parsing_openalex`
- `should_return_no_candidates_when_payload_is_not_found_marker`
- `should_tolerate_missing_and_wrongly_typed_fields_when_parsing_unpaywall`
- `should_find_citation_pdf_url_when_meta_attributes_are_reordered`
- `should_resolve_relative_pdf_link_when_base_url_given`
- `should_return_none_when_landing_page_has_no_pdf_signal`
- `should_return_none_when_several_pdf_anchors_are_ambiguous`
- `should_drop_javascript_and_mailto_links_when_listing_page_links`
- `should_clip_link_text_when_anchor_text_is_long`
- `should_append_mailto_only_when_host_is_openalex`
- `should_not_leak_api_key_when_url_is_unpaywall`
- DB-gated with `RecordingTransport`: `should_cache_not_found_marker_when_openalex_returns_404`
  (needs a 404-capable fake: extend `RecordingTransport` with an optional status
  responder, or add a small `StatusTransport` in the test module),
  `should_refetch_when_refresh_requested`, `should_build_unpaywall_url_with_email_when_configured`.

Acceptance: gate green; no network in tests; one manual
`curl` comparison recorded in "U2 implementation notes" (fixture provenance).

### U3. Acquisition runner: fetch trait, throttle, blob store, library copy, state machine, model-fallback trait

Files:
- `crates/lysilogy-ingest/src/acquire/{fetch.rs, store.rs, picker.rs, verify.rs}` (new),
  `crates/lysilogy-ingest/src/acquire/mod.rs` (runner, context, summary).
- `crates/lysilogy-ingest/Cargo.toml` (`chrono` to `[dependencies]` for `checked_at`
  in the outcome; `sha2` is already there for the cache keys).

`fetch.rs`: `PdfFetcher`, `FetchRequest`, `FetchedResource`, `FetchError` (with
`is_transient()`), `FetchFuture`; `ContentClass { PdfLike, Html, Other(String) }`,
`classify_content_type(Option<&str>) -> ContentClass`, `has_pdf_magic(&[u8]) -> bool`
(`%PDF-` within the first 1,024 bytes); `classify_status(u16) -> Option<FetchError>`
(decision 8); `MAX_PDF_BYTES`, `MAX_HTML_BYTES`; `HostThrottle` (`new(default,
&[(host, interval)])`, `disabled()`, `async wait(&self, url)`, pure
`throttle_delay(last: Option<Instant>, now: Instant, interval) -> Duration`);
`ScriptedFetcher` (map url → scripted result; writes the scripted bytes into
`temp_dir` and computes SHA-256 so the store path is exercised; records calls). Make it a
plain public type documented as a fake, as Ramp B did with `ScriptedLabeler`
(`labels.rs`), so root tests in U4/U5 can inject it. (`openalex::fake::RecordingTransport`
is `cfg(test)`-only; root tests keep their own transport fakes.)

`store.rs`: `blob_storage_key(sha) -> String`, `blob_path(root, sha) -> PathBuf`,
`store_blob(blob_root, temp_path, sha) -> io::Result<PathBuf>` (rename, or delete temp
when the target exists), `fetched_filename(title, year, work_id) -> String`,
`place_library_copy(library_root, blob_path, filename, sha) -> io::Result<String /*relative
path*/>` (decision 14; collision suffixes; same-SHA reuse via `scan::hash_file`; copy
through a `.part` file then rename; always `/` separators).

`picker.rs`: `LandingLinkPicker`, `LinkPickRequest`, `LinkPick`, `PickerError
{Unavailable, Failed, Malformed}`, `LINK_PICK_PROMPT_VERSION = 1`, `LINK_PICK_SCHEMA`
(strict: both keys required, `additionalProperties: false`, `pdf_link_index`
`{"type": ["integer", "null"], "minimum": 0}`), `link_pick_prompt(&LinkPickRequest)`,
`link_pick_cache_key(&LinkPickRequest) -> String` (sha256 hex), `ground_pick(&request,
&pick) -> Result<Option<PageLink>, &'static str>` (`index_out_of_range`), and
`ScriptedLinkPicker` (closure + call counter). Cache via `source_record` kind
`acquisition_link_pick` (store the raw `LinkPick`; a cache hit skips the call; picker
errors are not cached).

`verify.rs`: `title_appears_in(title, text) -> Option<bool>` (decision 16; `None` when the
title has < 3 key words).

Runner (`mod.rs`): types and flow exactly as in Design. Helper fns, each early-return
style: `stage_identifiers`, `stage_links`, `stage_download`, `try_candidate`,
`follow_landing`, `stage_map`. `run_acquisition` always calls `release` (wrap the body in
an inner async fn and release after it, ignoring a release error with a `tracing::warn!`).
`AcquisitionSummary`/`AcquisitionOutcome` are `Serialize`.

Tests (DB-gated with `ScriptedFetcher`, `RecordingTransport`, `tempfile` dirs for
`blob_root`/`library_root`, `HostThrottle::disabled()`):
- `should_map_work_when_arxiv_pdf_downloads` (row `mapped`, blob row + file, library file
  under `Fetched/`, document linked with method `acquisition`, version `blob_id` set).
- `should_not_contact_network_when_rerun_after_mapping` (fetcher and transport call
  counts unchanged; outcome `already_mapped`).
- `should_resume_from_link_found_when_previous_run_failed_transiently` (first run 503 →
  `pending`, attempts 1; second run succeeds without re-querying OpenAlex).
- `should_mark_unavailable_no_identifier_when_work_has_no_identifiers`
- `should_mark_unavailable_no_oa_location_when_providers_offer_nothing`
- `should_mark_unavailable_all_links_failed_when_every_candidate_is_permanent`
- `should_skip_unavailable_within_recheck_window_when_not_retrying`
- `should_retry_and_bypass_cache_when_retry_unavailable_set`
- `should_reject_html_disguised_as_pdf_when_magic_missing` (scripted fetcher returns
  bytes without `%PDF-` → permanent; implement the check in the runner too, not only the
  live fetcher, so the fake path covers it)
- `should_follow_citation_pdf_url_when_candidate_is_landing_page`
- `should_not_call_picker_when_model_fallback_disabled`
- `should_call_picker_and_fetch_grounded_link_when_meta_missing`
- `should_reject_pick_when_index_out_of_range`
- `should_reuse_cached_pick_when_rerun`
- `should_skip_unpaywall_and_warn_when_no_contact_email`
- `should_link_existing_library_document_when_sha_already_present` (no copy written)
- `should_not_relink_document_when_linked_to_another_work` (warning)
- `should_remap_when_mapped_document_was_tombstoned`
- `should_fall_back_to_link_found_when_blob_file_missing`
- `should_return_busy_when_lease_held`
- `should_short_circuit_already_local_when_work_has_present_document`
- `should_follow_redirect_when_work_was_merged`
- Pure: `should_classify_content_types_when_parameters_and_case_vary`,
  `should_detect_pdf_magic_within_first_kilobyte_only`,
  `should_compute_zero_delay_when_interval_elapsed`,
  `should_classify_429_and_5xx_as_transient_and_404_as_permanent`,
  `should_sanitize_fetched_filename_when_title_has_path_separators`,
  `should_truncate_fetched_filename_when_title_is_long`,
  `should_fall_back_to_work_id_when_title_unknown`,
  `should_add_suffix_when_library_name_taken_by_different_bytes`,
  `should_reuse_library_copy_when_same_bytes_exist`,
  `should_report_title_present_when_most_key_words_appear`,
  `should_skip_title_check_when_title_has_fewer_than_three_words`,
  `should_number_links_when_building_pick_prompt`,
  `should_parse_link_pick_schema_when_embedded` (valid JSON, strict).

Acceptance: gate green; no network; every unavailable reason, the transient path, and
both landing paths are covered.

### U4. Root wiring: live fetcher, sidecar picker, `lysilogy fetch`, live CLI smoke

Files:
- `src/remote.rs` (+ `download_public_resource`), `src/api.rs` (import calls it;
  delete the moved code; `contains_pdf_header` replaced by
  `lysilogy_ingest::acquire::has_pdf_magic`).
- `src/acquisition.rs` (new), `src/acquisition/picker.rs` (new), `src/lib.rs`.
- `src/graph.rs` (`acquisitions: AcquisitionRepository`).
- `src/main.rs` (`Command::Fetch`), `src/error.rs` (`Error::Acquisition(String)` → 502
  `acquisition_failed`, `From<IngestError>` mapping as `Literature` does).

`download_public_resource(url, temp_dir, limits) -> Result<PublicResource>`:
`PublicResource::{Pdf{final_url, temp_path, sha256, bytes}, Html{final_url, body}}`;
redirect loop and public-address client as today; `Content-Type` classified with
`classify_content_type` before reading the body; SHA-256 streamed with `sha2`; temp file
`create_new` in `temp_dir` with a random name; temp file removed on any error. Import
passes `accept_html = false` semantics (HTML → the existing "not a PDF" error).

`PublicPdfFetcher` implements `PdfFetcher` by calling it and mapping errors to
`Transient`/`Permanent` (decision 8; reqwest timeouts/connect → transient). User agent
per decision 7 (extend `public_http_client` with an optional UA suffix argument or build
the suffix from `contact_email_from_environment()` inside it — do not change the
import's behavior otherwise).

`SidecarLinkPicker` mirrors `SidecarLabeler`: rejects `Heuristic`, directory
`<data>/acquisition/picks/<cache key[..16]>/`, `reader_tool::<LinkPick>` with
`LINK_PICK_SCHEMA`, `live_web: false`, same error mapping.

`AppState` (in `src/acquisition.rs`):
- `acquisition_throttle: Arc<HostThrottle>` (constructed in `with_services`; add the
  field to `AppState`).
- `acquire_work(work, options: AcquireOptions{retry_unavailable, model_fallback:
  Option<AnalysisProvider>}) -> Result<AcquireReport>` builds the context (blob root
  `store.root().join("blobs")`, library root, `LiteratureClient` transport,
  `contact_email_from_environment()`), runs `run_acquisition`, on `Mapped`/`AlreadyLocal`
  calls `refresh()`, runs the front-matter title check (warning only), and returns
  `AcquireReport{summary, paper_id: Option<PaperId>, title, year}`.
- `acquisition_targets_cited_by(paper) -> Result<Vec<WorkId>>` (decision 18 ordering; a
  small SQL in `AcquisitionRepository::cited_targets(citing_work, workspace)` is fine —
  add it here with a DB test if you go that way).
- `resolve_work_argument(text) -> Result<WorkId>`: `w…` parse, `W\d+` → openalex lookup,
  `doi:` / `arxiv:` prefixes → `find_by_identifier`; 404-style `InvalidRequest` otherwise.

CLI `Command::Fetch { work: Option<String>, #[arg(long)] cited_by: Option<String>,
#[arg(long, default_value_t = 10)] limit: usize, #[arg(long)] retry_unavailable: bool,
#[arg(long, value_enum)] model_fallback: Option<LabelSidecar> }` (exactly one of
`work`/`--cited-by`, enforce with clap `ArgGroup` or a manual check; clamp `limit` to
100). `--cited-by` reuses `literature_command`'s path/ID/title resolution (extract it into
`resolve_lit_target(state, library, text) -> Result<PaperId>`). Output per Design/CLI;
renderers are pure fns (`render_acquisition_summary`, `render_cited_by_totals`) with
tests.

Tests:
- `should_classify_timeout_as_transient_when_mapping_fetch_errors`
- `should_reject_private_address_when_candidate_points_to_loopback` (no network: the
  guard rejects before connecting)
- `should_keep_import_behavior_when_download_moves_to_remote` (existing import tests
  stay green; add one asserting HTML is still rejected for import)
- `should_render_mapped_summary_when_outcome_is_mapped`,
  `should_render_totals_by_reason_when_cited_by_finishes`
- `should_parse_work_argument_when_given_w_id_openalex_doi_or_arxiv` (DB-gated)
- `should_parse_link_pick_when_claude_sidecar_returns_structured_output` and
  `should_report_unavailable_when_sidecar_program_missing` (fake-program style from
  `src/labeling.rs`)
- `should_refresh_catalog_and_return_paper_id_when_work_is_mapped` (DB-gated, `AppState`
  over a temp library and data dir, `ScriptedFetcher` via a test-only constructor such as
  `AppState::with_acquisition_fetcher(Arc<dyn PdfFetcher>)`)

Live smoke (record under "U4 implementation notes"), dev DB, real library
`~/Projects/Lysilogy/local-articles`, data dir as used by earlier ramps:
- `lysilogy fetch <id of "Cooperative Inverse Reinforcement Learning">` → `mapped`,
  source `arxiv`, file `Fetched/2016 - Cooperative Inverse Reinforcement Learning.pdf`.
- Rerun → `already_mapped`, no network (check with `RUST_LOG=debug` or by timing).
- `lysilogy fetch --cited-by "Fine-Tuning Language Models from Human Preferences"
  --limit 5` → totals line; then `lysilogy lit "Cooperative Inverse"` on the fetched paper
  links to the same Work (identity converges).
- One non-arXiv DOI with OA (for example a cited AER/NBER work) to exercise the OpenAlex
  locations path; one Work with no OA copy to see `no_oa_location`.
- Run once with and once without `LYSILOGY_CONTACT_EMAIL` (any address you control set
  only in the shell; never committed) to show the Unpaywall skip warning.
- List every file written under `local-articles/Fetched/` so the user can remove them.

Acceptance: gate green; live smoke recorded; `/api/import` behavior unchanged.

### U5. API, UI fetch action, live API/UI smoke, report

Files:
- `src/api/acquisition.rs` (new; routes merged in `build_router`), `src/api.rs`
  (`acquisitions` on `NeighborsResponse`/`CitationsResponse`, `AppState` field
  `acquisition_jobs`).
- `web/src/types.ts`, `web/src/lib/api.ts` (`fetchWork(workId, options?)`,
  `workAcquisition(workId, signal?)`), `web/src/lib/acquisition.ts` (new),
  `web/scripts/acquisition.test.mjs` (new), `web/package.json` (`test:acquisition`),
  `web/src/components/LiteratureGraph.tsx`, `web/src/components/LiteratureGraph.css`,
  `web/src/components/HelpOverlay.tsx`.
- `docs/ramps/ramp-c.md` (Result + Report sections).

API per Design/API. `AcquisitionJobs::start(state, work, options)` mirrors
`LiteratureJobs::start` (double `tokio::spawn` so a panic becomes `failed`), acquiring a
semaphore permit inside the job. Parse the Work ID with the existing `parse_id`-style
helper (400 `invalid_work_id`), check the Work exists through `fetch_work` (404),
`graph_or_unavailable` (503).

Rust tests (`oneshot` style; DB-gated where needed; `ScriptedFetcher` injected):
- `should_return_202_running_when_posting_fetch`
- `should_return_current_state_when_fetch_already_running`
- `should_return_400_when_work_id_malformed` / `should_return_400_when_body_has_unknown_field`
- `should_return_404_when_work_unknown`
- `should_return_503_when_graph_unavailable`
- `should_report_mapped_acquisition_and_paper_id_when_job_finished`
- `should_include_acquisitions_in_neighbors_when_rows_exist`
- `should_omit_paper_id_when_document_not_in_catalog`

UI per Design/UI. `acquisition.test.mjs` cases: every state's text; unavailable reasons
in words; `canFetch` false when a local paper exists, false while running, true for
unavailable (as Retry); unknown state string → generic text (malformed input).

Verification:
- `npm run typecheck`, `npm run lint`, `npm run build`, `npm run test:acquisition`,
  `npm run test:literature-layout`, `npm run test:passage-focus`,
  `npm run smoke:paragraph-selection` (Cursor-mode `L` unaffected).
- Scratch Playwright `~/.cache/lysilogy-smoke/acquire-smoke.mjs` (not committed), importing
  `~/Projects/Lysilogy/web/node_modules/playwright/index.mjs`, serving `web/dist`, mocking
  `/api/library`, `/api/papers/*`, `/work`, neighbors, citations, `/fetch`,
  `/acquisition` (scripted progression running → link_found → downloaded → mapped). Assert:
  the button shows for a non-local node and not for a local one; `f` POSTs once; status
  text progresses; on mapped the neighbors reload, the node gets the local dot, and
  **Open in reader** sets `#paper=<id>`; an unavailable node shows the reason and
  **Retry** sends `retry_unavailable: true`; `f` does not open link hints; Esc closes the
  panel then the overlay; polling stops when the panel closes.
- Live: `lysilogy serve --bind 127.0.0.1:7394 --web web/dist` on the dev DB.
  `POST /api/works/<arXiv-cited Work>/fetch`, poll `GET /acquisition` to `mapped`, then
  `GET /api/papers/<paper_id>/work` returns that Work with link method `acquisition`, and
  `GET /api/papers/<paper_id>` serves the paper. Run the Playwright script in live mode:
  open the citing paper, `L`, select the node, `f`, wait for **Open in reader**, open it,
  screenshot the first page.
- Append "Result" and "Report" sections (what shipped, how to user-test, smoke results,
  known gaps, files written to the library) in the Ramp A/B format.

Acceptance: Rust gate and frontend checks green; Playwright passes against mocks and
live; live smoke recorded.

## Open questions for the user (defaults chosen above)

1. Library side effect: fetched PDFs are copied into `<library>/Fetched/` so they behave
   like any library paper. Prefer a different folder name, or keeping fetched papers out
   of the library (would need a second catalog root)?
2. Unavailable recheck window: 30 days. Shorter?
3. Crossref `link[]` (publisher TDM URLs) is left out because most need tokens; worth a
   follow-up?

## U1 implementation notes

- `record_failure` also sets `checked_at`, per the closing note under Flow.
- `set_state` clears `unavailable_reason` for any non-terminal target so the table CHECK holds; setting `unavailable` through it is rejected by the CHECK (use `mark_unavailable`).
- `push_trial` caps in a single SQL statement (no `select ... for update`).
- `present_document_for_sha` test inserts the second same-hash document with raw SQL, because the scanner treats a second path with the same hash as a duplicate rather than a new document.
- Extra tests beyond the plan list: `should_record_mapping_and_bump_attempts_when_mapped`; the core round-trip test also covers `CandidateOrigin` and `WorkLinkMethod::Acquisition`.

## U2 implementation notes

- Fixture provenance (fetched 2026-10-13, no `mailto`, no API key):
  `curl -s "https://api.openalex.org/works/W2410842990?select=id,doi,ids,open_access,best_oa_location,locations" | jq .` and the same for `W2097916756`. Trimmed with jq: `source`, `license`, `license_id`, `raw_source_name`, `raw_type` removed from each location, and `locations` cut to the first 8 entries. `unpaywall_*.json` and `landing_*.html` are hand-written from the documented shapes with `example.*` hosts.
- The arXiv fixture's DataCite DOI landing location (`https://doi.org/10.48550/arxiv...`) and a second `arxiv.org/pdf` landing entry come back from OpenAlex as landing candidates; the latter dedupes into the `arxiv` PDF candidate through `normalize_candidate_url`.
- `rank_candidates` emits the normalized URL (not the raw one), so `arxiv.org/abs/<id>` becomes `/pdf/<id>`, and `/pdf/<id>.pdf` collapses to `/pdf/<id>`; `www.arxiv.org` is treated as `arxiv.org`.
- OpenAlex landing candidates come from `best_oa_location` and OA `locations[]` only (non-OA locations are ignored entirely). Unpaywall landing candidates are locations whose `url_for_pdf` is not usable.
- `IdentifierStatus` has `Conflicting`/`Retracted` (not `Withdrawn`); only `Active` identifiers are used. `W` IDs are upper-cased for the URL; the cache key is lower-cased.
- `decorate_url` also skips `query_pairs_mut` when there is nothing to add, so URLs without credentials are not given a trailing `?`.
- Extra tests beyond the plan list: Unpaywall 404 marker, 5xx not cached, no-identifier returns `None`, source-record attachment, alternate-link and unsafe-scheme landing cases, link dedupe/whitespace.
- Fetchers take the existing `OpenAlexClient<T>` as the generic JSON client for both providers; `ReqwestTransport` decorates only OpenAlex URLs, so Unpaywall requests carry only their own `email` parameter.

## U3 implementation notes

- `DbError::not_found()` (new, `lysilogy-db`) is the error for an unknown Work, so the root can map it to 404 through the existing `is_not_found()`. The ingest crate has no `sqlx` dependency to build `RowNotFound` itself.
- `MAX_PDF_BYTES` is a `u64`, `MAX_HTML_BYTES` a `usize`. `ScriptedFetcher` is public (`with`, `with_sequence` for transient-then-success scripts, `calls`, `urls`); an unscripted URL answers `Permanent("http_404")`. `ScriptedResponse::Pdf` bytes are written as given, so tests can script a body without `%PDF-` to exercise the runner's own magic check.
- Trial outcomes are `pdf`, `pdf_via_landing`, `transient:<reason>` and `permanent:<reason>`; only a last trial starting with `permanent:` makes a later run skip that URL (unless `retry_unavailable`). A landing candidate gets one trial for the page plus one for the hop URL, and `permanent:landing_no_pdf_link`, `permanent:pick_rejected`, `permanent:landing_hop_failed` when it led nowhere.
- Plan adjustment: the 6-candidate cap. If a run hits the cap with untried candidates left and nothing transient happened, it returns `pending` (`candidate_limit_reached:<n>_untried`) instead of `unavailable{all_links_failed}`; the next run skips the permanent ones and tries the rest. Otherwise a Work with more than 6 dead links would be marked unavailable without looking at the remainder.
- A picker that fails (`Unavailable`/`Failed`/`Malformed`) is treated as transient for that candidate (warning `link_picker_failed`, `pending`), is never cached, and is not logged as a permanent trial. An out-of-range pick adds warning `pick_rejected:index_out_of_range`.
- Resetting an expired `unavailable` row (after 30 days, no flag) also bypasses the provider caches, since a cached `{"not_found": true}` would otherwise never be re-asked.
- A provider lookup that fails transiently while other candidates exist (for example OpenAlex down but an arXiv ID present) does not block the run: the candidates found are stored with warning `provider_transient:<error>`, and the skipped provider is not re-queried on resume (state is already `link_found`). With no candidates at all it is `pending`. Non-transient provider errors (4xx, malformed) are ignored with warnings `openalex_lookup_failed` / `unpaywall_lookup_failed`.
- `attempts` rises once per run that reached a provider or the fetcher (a recorded failure counts itself; an unavailable verdict after network work is bumped in the summary step). A run that only resumes mapping, or returns `already_mapped`, `already_local`, `busy`, `skipped_unavailable` or `no_identifier`, does not count.
- A document already linked to the same Work keeps its existing link method (identifier/manual provenance is not overwritten by `acquisition`). A document linked to a redirected predecessor of the Work counts as the same Work.
- On resume from `downloaded`, the source record for the link is looked up by `pdf_url` among stored candidates; a landing hop (`landing_meta`/`model`) loses its record link in that case. In a single run the winning candidate's record is used.
- `already_local` marks the row `mapped` with the present document (no version recorded); a later tombstone sends it back through `downloaded`, which reuses a stored blob if there is one.
- Tests: 32 runner tests in `acquire/tests.rs`, plus pure tests in `fetch.rs`, `store.rs`, `picker.rs`, `verify.rs` (extra: throttle slot spacing, scripted fetcher sequence, unfinished-copy cleanup, picker error not cached, 5xx/429 provider classification, outcome JSON shape).

## U4 implementation notes

- `download_public_resource(url, temp_dir, accept_html)` returns `Result<PublicResource, DownloadError>`; `DownloadError { error, reason, transient }` keeps the import's exact `Error` values (`into_error()`), while the fetcher uses the short `reason` (`http_503`, `request_failed`, `body_failed`, `non_public_address`, `not_pdf`, `content_type:<type>`, `too_large`, `bad_redirect`, `io`). The temp file is `.lysilogy-download-<uuid>.part` in `temp_dir`; for `/api/import` that is the library root, as before (dot-prefixed, removed on any error). Content-Type is now classified before the body for import too, so a non-PDF/non-HTML type is rejected without streaming; the message is unchanged. The import's `User-Agent` is now `Lysilogy/<version>[ (mailto:…)]`; connect timeout is 15 s (was 120 s), whole PDF request 120 s, HTML body read 30 s. Content-type rejection is a pure `reject_content_class`, so the "HTML still rejected for import" test needs no network.
- `polite_user_agent` drops a contact email that is blank or has characters unsafe in a header.
- Plan adjustment: `CitedTarget { work_id, skipped }` and `AcquisitionRepository::cited_targets(citing, workspace, retry_unavailable)` return every candidate target with a `skipped` flag (mapped, or unavailable inside the window unless retrying), so the CLI can count skipped Works without charging them to `--limit`. `acquisition_targets_cited_by(paper, retry_unavailable)` wraps it.
- Plan adjustment: `arxiv:<id>` in `resolve_work_argument` falls back to the DataCite DOI `10.48550/arxiv.<id>`, because most arXiv-only Works in the graph carry only that DOI.
- `HostThrottle` gained a `Debug` impl; `AppState` gained `with_acquisition_throttle` (tests) next to `with_acquisition_fetcher`. `LiteratureClient::client()` exposes the shared OpenAlex client to the acquisition module.
- `--cited-by` stops at the first run error (DB/IO); unavailable and pending results never fail the command. `--limit` is clamped to 1..=100.
- Tests added: 6 pure + 1 DB in `src/acquisition.rs`/`src/api.rs`, 4 in `src/remote.rs`, 4 in `src/acquisition/picker.rs`, 7 in `src/main.rs`, 1 DB in `lysilogy-db` (`cited_targets` ordering and skip flags). Gate: `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green with `LYSILOGY_TEST_DATABASE_URL` set (no "skipping" lines).

### Live smoke (2026-10-13)

Dev DB `lysilogy`, library `~/Projects/Lysilogy/local-articles`, data `~/Projects/Lysilogy/.lysilogy`, `LYSILOGY_CONTACT_EMAIL` unset unless noted.

- `fetch arxiv:1606.03137` (the Work only has the DataCite DOI): `mapped (source arxiv)`, stages `identifier_found → link_found → downloaded → mapped`, 581.8 KiB, `Fetched/2016 - Cooperative Inverse Reinforcement Learning.pdf`, warning `unpaywall_skipped_no_email`. 1.9 s.
- Rerun (`fetch doi:10.48550/arxiv.1606.03137`): `already_mapped`, 0.6 s; `RUST_LOG=debug` shows only SQL, no HTTP.
- `lit "Cooperative Inverse"`: `Work wpFXM6gy4QeqHe5tuX0WpPA (identifier: arxiv:1606.03137)`, the same Work as the fetch (identity converges). It also ingested 25 references (21 new stub Works).
- `fetch --cited-by "Fine-Tuning Language Models from Human Preferences" --limit 5`: five arXiv-sourced papers mapped (13.6 s, the 3 s arXiv spacing is visible); totals `Fetched 5 · already local 0 · unavailable 0 · pending 0 · skipped 0`.
- OpenAlex-locations DOI case: `fetch doi:10.3386/w16425` (NBER working paper): `mapped (source openalex_best)` via `https://doi.org/10.3386/w16425` redirecting to the NBER PDF, 337.7 KiB.
- `no_oa_location`: `fetch doi:10.1257/aer.104.6.1597` (AER, closed): `unavailable (no_oa_location)`. `all_links_failed`: `fetch doi:10.1257/jep.21.3.131`: the AEA PDF and landing page both answer 403, `unavailable (all_links_failed)`; a rerun inside the window prints `skipped_unavailable (all_links_failed, checked 2026-10-13)`.
- Contact email: without it every run that reached Unpaywall's step prints `Warnings   unpaywall_skipped_no_email`. With `LYSILOGY_CONTACT_EMAIL=smoke-test@example.org` set only in the shell, the warning is gone and the closed Works stay `no_oa_location` / `all_links_failed` (Unpaywall found nothing more). No address is committed.

Files written under `local-articles/Fetched/` (remove with the matching `reader.document` tombstoning on the next scan; blobs are in `~/Projects/Lysilogy/.lysilogy/blobs/sha256/`):

- `2010 - Recent Findings on Trade and Inequality.pdf`
- `2016 - Cooperative Inverse Reinforcement Learning.pdf`
- `2019 - Deep Batch Active Learning by Diverse, Uncertain Gradient Lower Bounds.pdf`
- `2019 - Discriminative Active Learning.pdf`
- `2019 - Finding Generalizable Evidence by Learning to Convince Q&A Models.pdf`
- `2019 - Learning from Dialogue after Deployment Feed Yourself, Chatbot!.pdf`
- `2019 - Neural Text Summarization A Critical Evaluation.pdf`

## U5 implementation notes

- `src/api/acquisition.rs` holds the routes, `AcquisitionJobs` (job map plus an `Arc<Semaphore>` of 2, permit acquired inside the job) and the additive `acquisitions` helper (`states_for`) used by neighbors and citations. The `fetch` body is parsed by hand like `lit`, so an empty body is the default and anything else must match `FetchOptions` (`deny_unknown_fields`); wrong types or non-objects are 400 `invalid_body`. Order of checks: 400 `invalid_work_id`, 400 `invalid_body`, 503 `graph_unavailable`, 404 `work_not_found`. Jobs are keyed by the Work after `resolve_redirect`.
- `AcquisitionView` serializes the row's state, reason, `checked_at`, attempts, `last_error`, source, `pdf_url`, bytes and `updated_at`; `final_url`, `sha256`, blob and document IDs stay server side. `paper_id` is set only for a `mapped` row whose document has a path that is in the catalog.
- Observed live: the first poll after `POST` can show `acquisition.state = mapped` with `job.state = running` and `paper_id = null` (the row is written before the catalog refresh). The UI keeps polling until the job leaves `running`, then reloads.
- Plan adjustment (UI): `canFetch(workId, localPapers, acquisitions, jobState)` is false only for a local paper or a running job; unavailable and half-finished rows stay fetchable (the former as Retry). The status text prefixes "No copy of this work is in the library." only when no fetch is running. `Open in reader` replaces the fetch control after a fetch the panel watched finish; the Work's other local papers still list below it. `f` only acts once the Work's citations have loaded (the same moment the button appears).
- Reading `GET /acquisition` once when the panel opens (not only while running) lets a job started earlier, or from the CLI, show its status. A read that says `idle` never overwrites an optimistic `running`.
- Tests: 10 API tests in `src/api/acquisition.rs` (9 need the database, all ran), 14 node tests in `web/scripts/acquisition.test.mjs`. Gate: `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` green with `LYSILOGY_TEST_DATABASE_URL` set (root 244 passed, no "skipping" lines); web `typecheck`, `lint`, `build`, `test:acquisition` (14), `test:literature-layout` (11), `test:passage-focus` (8), `test:api` (6) and `smoke:paragraph-selection` pass.

## Result

- `POST /api/works/{id}/fetch` starts a bounded background acquisition (202 `{state: "running"}`; a second post while one runs returns the current state and starts nothing). `GET /api/works/{id}/acquisition` returns job state, the durable row view, and `paper_id`. Neighbors and citations responses carry `acquisitions` for every Work with a row.
- The `L` overlay's Work panel fetches an open-access copy with `f` (or the button), shows live status, reloads the graph so the node gains its local dot, and offers a focused **Open in reader**. Unavailable Works show the reason and check date and offer **Retry** (`retry_unavailable: true`). Nodes carry `data-acquisition`; unavailable ones get a dashed muted outline and running ones a pulsing ring (static under `prefers-reduced-motion`). The help overlay lists `f`.
- Verification (2026-10-13): scratch Playwright `~/.cache/lysilogy-smoke/acquire-smoke.mjs` passes in mock mode and live mode (not committed).
  - Mock: the button shows for a non-local node and not for a local one (and `f` there does not POST); `f` POSTs exactly once and a second `f` while running does not; the status reads Resolving identifiers, Looking for an open-access copy, Downloading, Saving to the library, Fetched; on mapped the neighbors reload and the node's `data-local` becomes true; **Open in reader** is focused and sets `#paper=<id>`; an unavailable node shows the reason and date and **Retry** POSTs `{"retry_unavailable":true}`; `f` opens no link hints; Esc closes the panel, then the overlay; polling stops within the next 2.6 s after the panel closes.
  - Live (`lysilogy serve --bind 127.0.0.1:7394 --web web/dist`, dev DB, real library): `POST /api/works/wNqTb4OrZSb64jRLZbBMzcQ/fetch` (Generative Adversarial Imitation Learning, `10.48550/arxiv.1606.03476`, cited by the Christiano 2017 paper) polled to `mapped` in about 1 s with `paper_id be6869e76887b857`; `GET /api/papers/be6869e76887b857/work` returned that Work with link method `acquisition` (`acquired:arxiv:arxiv.org`); `/api/papers/be6869e76887b857` and `/source` returned 200 (`application/pdf`). Error paths: malformed id 400, unknown field 400.
  - Live UI: the Playwright script opened the Christiano paper, pressed `L`, selected "Guided Cost Learning: Deep Inverse Optimal Control" (`w8fyuJlPLQfWiBhIKZNcXIA`), pressed `f`, saw Resolving identifiers, Downloading, Fetched, pressed Open in reader, and the hash became `#paper=79e33f62917c4df0` with the PDF's first page rendered. Screenshots: `~/.cache/lysilogy-smoke/live-graph-panel.png`, `live-opened-paper.png`.

## Report

### What shipped

Ramp C turns cited-but-missing Works into library papers. A deterministic locator (arXiv, OpenAlex locations, Unpaywall with a contact email, landing-page `citation_pdf_url`) finds an open-access PDF; a polite, SSRF-guarded downloader stores it content-addressed, copies it under `<library>/Fetched/`, maps it as a document and links it to the Work with method `acquisition`. A resumable state machine records progress in `ops.acquisition`; an optional model fallback (`claude`/`codex`, off by default) picks a PDF link from a landing page and is grounded to the offered links. `lysilogy fetch` runs it from the CLI, and the API plus the `L` overlay run it from the browser.

### How to user-test

- CLI: `lysilogy fetch arxiv:1606.03137`, then again (no network, `already_mapped`); `lysilogy fetch --cited-by "Fine-Tuning Language Models from Human Preferences" --limit 5`.
- Server: `lysilogy serve --bind 127.0.0.1:7394 --web web/dist` with `LYSILOGY_DATABASE_URL`, `LYSILOGY_LIBRARY`, `LYSILOGY_DATA` set. Open a paper that has been through `lit`, press `L`, move to a dot without a filled centre (`j`/`k`), press `↵`, then `f`. When it says Fetched, press `↵` on **Open in reader**. A closed Work (for example `doi:10.1257/aer.104.6.1597`) ends as Unavailable with the reason; `f` again retries.
- API: `curl -X POST /api/works/<id>/fetch`, then `curl /api/works/<id>/acquisition` until `job.state` is `succeeded`.

### Smoke results

See U4's live CLI smoke and this unit's Result section: arXiv, NBER (OpenAlex `best_oa_location`), `no_oa_location` and `all_links_failed` outcomes through the CLI; arXiv through the API and the browser; mock Playwright covering progress, retry, key handling and polling.

### Known gaps

- Only open-access sources. Publisher pages that answer 403 to automated clients (for example the AEA) end as `all_links_failed` even though a human could download the PDF.
- Crossref `link[]`, Semantic Scholar `openAccessPdf`, EPUB/HTML full text and bulk scheduled acquisition are not implemented.
- The title sanity check is a warning only (`title_not_in_front_matter`), visible in the CLI summary and the job `summary.warnings`, not in the panel.
- Job state is in memory: after a server restart `GET /acquisition` reports `idle` plus the durable row; a lease from a killed run blocks that Work for up to 10 minutes.
- The panel shows the failure message of a failed job but not the row's `last_error` for a pending run beyond "The fetch did not finish. Try again."
- A fetched PDF is a copy in the library; deleting it from `Fetched/` tombstones the document on the next scan and the next fetch remaps it from the stored blob.
- `npm run smoke` remains broken locally (missing fixture).

### Files written to the library

`~/Projects/Lysilogy/local-articles/Fetched/` (blobs are under `~/Projects/Lysilogy/.lysilogy/blobs/sha256/`):

- `2010 - Recent Findings on Trade and Inequality.pdf`
- `2016 - Cooperative Inverse Reinforcement Learning.pdf`
- `2016 - Generative Adversarial Imitation Learning.pdf` (U5 API smoke)
- `2016 - Guided Cost Learning Deep Inverse Optimal Control via Policy Optimization.pdf` (U5 UI smoke)
- `2019 - Deep Batch Active Learning by Diverse, Uncertain Gradient Lower Bounds.pdf`
- `2019 - Discriminative Active Learning.pdf`
- `2019 - Finding Generalizable Evidence by Learning to Convince Q&A Models.pdf`
- `2019 - Learning from Dialogue after Deployment Feed Yourself, Chatbot!.pdf`
- `2019 - Neural Text Summarization A Critical Evaluation.pdf`
