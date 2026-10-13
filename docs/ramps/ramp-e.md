# Ramp E: "Walk backward" — multi-hop expansion and reading ramps

Status: planned 2026-10-13. Branch `feat/lit-ramp-e-ramps`, based on
`integration/literature-viewer` at `a30611c` (Ramp D merged). Parent plan:
[literature-viewer-plan.md](../literature-viewer-plan.md), section "Ramp E". Previous ramps:
[ramp-a.md](ramp-a.md), [ramp-b.md](ramp-b.md), [ramp-c.md](ramp-c.md), [ramp-d.md](ramp-d.md).

## What earlier work built

- Ramp A: `run_literature_flow` (`crates/lysilogy-ingest/src/literature.rs`) links a library
  PDF to a Work, fetches its `OpenAlex` record (`fetch_work_cached_record`, cached as
  `source_record` kind `openalex_work`), asserts one `graph.citation_assertion` per
  `referenced_works` entry (`ingest_openalex_references`, stub Works created through
  `resolve_or_create(Openalex, …)`), then titles the stubs (`hydrate_stub_works`, batched
  `openalex_work_summary`). Only the *paper's own* Work is ever expanded; stub Works have
  no outgoing edges.
- Ramp B: local bibliography and in-text mentions for a held paper
  (`AppState::run_local_evidence`), optional model labels (`SidecarLabeler`).
- Ramp C: acquisition (`ops.acquisition`, `POST /api/works/{id}/fetch`, job registry
  `AcquisitionJobs` with a semaphore), `local_papers_for` (Work → catalog papers),
  `acquisitions` map on neighbors.
- Ramp D: `reader.tag|work_tag|work_mark|work_prominence`, `refresh_prominence_quietly`,
  ranked neighbors `GET /api/graph/neighbors/{id}?limit&expand&tags`, the `L` overlay
  (`web/src/components/LiteratureGraph.tsx`) showing a top-40 single-hop fanout of the
  paper's own Work with cluster pills, marks and tags. The overlay never re-centers and
  never shows a second hop.
- Workspace-owned state lives in `reader` keyed by `workspace_id`; operational state lives
  in `ops`. Merges go through `WorkRepository::merge_work` → `move_work_content` →
  `move_curation` (`crates/lysilogy-db/src/work.rs`).
- ID prefixes in use: `w v i s e k d f l c m r t`.

Live dev DB (2026-10-13): 3,240 Works, 48 with outgoing edges, 111 present documents linked to
Works. Seed for the live smoke: "Learning to summarize from human feedback" (215 outgoing
assertions); it cites "Fine-Tuning Language Models from Human Preferences" (held, 137
outgoing) and "Deep reinforcement learning from human preferences" (held, 115), whose
references are mostly stubs with no outgoing edges — the provider path is exercised one
hop further.

## Scope

In:

1. Migration `0008_ramps.sql`: `reader.ramp`, `reader.ramp_item` (ordered items, the edge
   path that led there, status, note) and `ops.work_expansion` (the cached-neighborhood
   ledger).
2. Expansion: `expand` a Work = make its outgoing fanout known. Held Works (present document
   in the catalog) run the local flow (Ramp A `lit` flow + Ramp B local evidence, no labels);
   other Works run the provider flow (Ramp A reference ingest from the stub's identifier).
   Re-expansion is free (no network, no flow) unless `refresh`. A bounded walk expands a
   seed and then its top-ranked children, to `depth ≤ 3`, `fanout ≤ 10`, `≤ 40` expansions.
3. Ramps: create/list/read/rename/delete; add/update/remove items; manual reorder; a
   deterministic "foundational first" suggested order.
4. CLI: `lysilogy expand`, `lysilogy ramp …`. API: expansion job + status, ramp CRUD, item
   CRUD, order. Neighbors gains an additive `expansions` map.
5. UI: expand a node's fanout in place (multi-hop graph, cross-links drawn once), collapse,
   "add to ramp" (with missing ancestors), a ramp sidebar with the ordered path, status,
   notes, local/fetch state, open-in-reader, reorder, suggest order.
6. Live smoke: build a 2-hop ramp from one seed through the API on the dev DB.

Out: multiple workspaces or sharing ramps, ramps across seeds merged, citations-direction
walking (walk is references only, "backward"), model-written ramp summaries or ordering (no
model call in this ramp, so no new sidecar trait; a future model ordering would follow the
`SidecarLabeler` pattern with a fake), force-directed layout, persisting the in-overlay
expansion state across sessions, a standalone ramp page/route (the sidebar is the ramp UI),
drag-and-drop reorder.

## Decisions taken

1. **Expansion is a ledger plus the existing graph.** The neighborhood itself is the
   canonical `graph.citation_assertion` rows; `ops.work_expansion` only records *that* a
   Work's fanout was made known, how, and when. "Cached neighborhood" = the assertions +
   cached `openalex_work`/`openalex_work_summary` source records + the ledger row.
2. **Fanout known** = a ledger row with `state = 'expanded'`, *or* (no row and) at least one
   outgoing `asserted`/`accepted` assertion (papers linked by Ramps A–D before this ramp).
   Expanding a Work whose fanout is known without a row writes an `expanded` row with
   `method = 'existing'` and does nothing else.
3. **Expansion dispatch** (`AppState::expand_work`), first rule wins:
   - known fanout and not `refresh` → `cached` (no flow, no network, no prominence refresh).
   - ledger row `unavailable` and not `refresh` → return it (`cached: true`).
   - Work held locally (a present document linked to it whose paper is in the catalog; first
     by paper ID) → local flow: `run_literature_flow(paper)` then, unless `local = false`,
     `run_local_evidence(paper, &summary)` (failure → warning `local_evidence_failed:<e>`).
     If the flow ends `Unmatched` or links a *different* Work, add warning
     `local_flow_unmatched` / `local_flow_linked_other_work:<id>` and fall through to the
     provider flow for the requested Work.
   - else provider flow (`lysilogy_ingest::expand::expand_from_provider`).
   After a run (not a cache hit) that ends `expanded`, `refresh_prominence_quietly(work, true)`
   so the new children rank immediately.
4. **Provider flow key**: the Work's active identifiers, first available of `openalex` (its
   value), `doi` (`doi:<value>`), `arxiv` (`provider_key`, the `10.48550/arxiv.` DOI). None →
   `unavailable(no_provider_identifier)`. Provider 404 / soft error →
   `unavailable(provider_not_found)`. When the record's `openalex_id` already belongs to a
   *different* Work (`find_by_identifier`) → `unavailable(identifier_conflict)` with warning
   `openalex_id_on_other_work:<w…>`, nothing ingested (merging is resolution's job, not
   expansion's). Otherwise `ingest_openalex_references` + `hydrate_stub_works` over the
   outgoing Works. A record with zero `referenced_works` is still `expanded` with
   `references_count = 0`. Hard provider errors (transport/5xx) fail the job and write no row
   (only `attempts`/`last_error` are bumped on an existing row).
5. **Unavailable is cached**; `refresh: true` retries it. `refresh` on an expanded Work reruns
   the flow (the source-record cache still avoids network for the record itself; this is for
   picking up local evidence or a changed identifier).
6. **Walk bounds** (pure, `lysilogy_core::expansion::WalkBounds::parse`): `depth` 1..=3
   (default 1 = just this Work), `fanout` 1..=10 (default 5), total expansions capped at
   `WALK_EXPANSIONS_MAX = 40` (the walk stops and reports `truncated: true`). Out of range →
   400 `invalid_depth` / `invalid_fanout` (CLI: clap value parser error).
7. **Walk frontier** (pure, `src/walk.rs::plan_frontier`): after each level, for each parent
   in that level's order, its children ranked by prominence score desc, then `display_year`
   desc (nulls last), then Work ID; skip `hidden` and `downplayed` marks, already visited
   Works and the seed; take the first `fanout`; stop when the budget is spent. Breadth first,
   deterministic.
8. **Expansion jobs** mirror `AcquisitionJobs`: `ExpansionJobs` keyed by the requested Work
   (resolved through redirects), one running job per Work, a semaphore of **1** permit (polite
   to `OpenAlex`; a walk is a single job). Cache hits still go through the job so the client
   has one code path, but finish without waiting on the permit. Two processes (CLI and
   server) expanding the same Work concurrently is harmless: ingest is idempotent and the
   ledger is an upsert.
9. **Ramps live in `reader`, keyed by `workspace_id`**, like tags. One ramp has exactly one
   seed Work. Creating a ramp inserts the seed as item 0 with `path = [seed]`, depth 0. The
   seed item cannot be removed (409 `seed_item`); deleting the ramp is the way.
10. **Items** are keyed `(ramp_id, work_id)` (a Work appears once per ramp). Each stores
    `via_work_id` (the item it was reached from; NULL for the seed or a direct add) and
    `path_work_ids` (seed-or-root … via, work; last element = `work_id`), so the path is
    history and survives removing an intermediate item. Adding with `via` requires
    (a) `via` is an item of the ramp (409 `via_not_in_ramp`), (b) an `asserted`/`accepted`
    assertion `via → work` exists (409 `no_citation_edge`), (c) resulting path length ≤
    `RAMP_PATH_MAX = 9` (depth ≤ 8, 409 `ramp_too_deep`). Without `via`, `path = [work]`.
    Already present → 409 `item_exists`. A ramp holds ≤ `RAMP_ITEMS_MAX = 500` items (409
    `ramp_full`). The client adds missing ancestors itself (UI Decision 18).
11. **Status** `unread` (default) | `reading` | `read`; `status_changed_at` set when it changes.
    Notes reuse `parse_mark_note` (trim, empty → NULL, ≤ 2,000 chars, `invalid_note`).
    Titles: `RampTitle::parse` — trim, collapse internal whitespace, 1..=120 chars, no control
    characters (`invalid_ramp_title`); default title = the seed's `display_title` truncated to
    120 chars at a char boundary, else `"Reading ramp"`.
12. **Positions are sparse ordering keys.** Append = `max(position) + 1`; remove leaves a gap;
    a reorder rewrites `0..n` in one transaction (`UNIQUE (ramp_id, position) DEFERRABLE
    INITIALLY DEFERRED`). Responses list items by position and report a dense `ordinal`.
    A manual order must be an exact permutation of the current items (400 `invalid_order`:
    missing, extra, duplicate, or malformed IDs; redirects resolved first).
13. **Suggested order "foundational first"** (pure, `lysilogy_core::ramp::suggest_order`):
    - Constraint: if item X cites item Y (any asserted/accepted assertion between two items,
      not only path edges), Y comes before X. Kahn's algorithm.
    - Among available items, priority: greater depth first, then higher prominence score, then
      older `display_year` (nulls last), then `title_key`, then Work ID.
    - The seed is always last.
    - Cycles: when nothing is available, take the best-priority remaining item regardless of
      unmet prerequisites (deterministic).
    Applied with `PUT /api/ramps/{id}/order {"strategy":"foundational_first"}`; the response
    carries the new order. There is no separate preview endpoint (the user can reorder back).
14. **Merges.** `move_work_content` gains `move_ramps`: in every ramp, if `into` is not an item,
    repoint `from`'s item to `into`; else delete `from`'s item (the survivor's row wins);
    `path_work_ids = array_replace(path_work_ids, from, into)`, `via_work_id` and
    `reader.ramp.seed_work_id` repointed; `from`'s `ops.work_expansion` row is moved when
    `into` has none, else deleted. Consecutive duplicates in a path after a merge are kept
    (rare; documented).
15. **Every route resolves Work IDs through redirects** and responds with resolved IDs. Check
    order: path IDs → body → graph availability → existence (as Ramps C–D). Ramp IDs not in
    the default workspace are 404 `ramp_not_found`.
16. **New ID type** `RampId`, prefix `'p'`. Ramp items have no ID of their own.
17. **No model anywhere in this ramp**, so no new trait/sidecar. Ordering and walks are
    deterministic; unit tests use `RecordingTransport` fixtures and never touch the network.
18. **UI keys** (overlay): `w` expands / collapses the selected Work's fanout in place (panel
    open or closed; also an "Expand references" button in the Work panel and double-click on a
    node); `y` adds the selected Work to the current ramp, adding missing walk ancestors first
    and creating a ramp seeded at the center if none is current; `R` toggles the ramp sidebar.
    Single click keeps its Ramp D meaning (select). The help overlay documents the keys.
19. **In-place expansion limits (UI)**: an expanded node shows its top `EXPANSION_TOP_N = 8`
    children (`limit=8` on the existing neighbors endpoint); at most `MAX_UI_EXPANSIONS = 6`
    expanded nodes and hop depth ≤ `MAX_UI_DEPTH = 3` from the center (beyond → inline
    notice, nothing sent). A child already on the canvas is not duplicated; a cross-link edge
    is drawn to the existing node instead. Collapsing a node collapses its expanded
    descendants. Expansion state is per overlay session (not persisted).
20. **Current ramp (UI)**: `GET /api/ramps?seed=<center work>`; the current ramp is the one
    remembered in `localStorage["lysilogy.ramp.<centerWorkId>"]` (wrapped in try/catch) if it
    is still listed, else the most recently updated, else none.

## Design

### Schema: `0008_ramps.sql`

```sql
CREATE TABLE reader.ramp (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    seed_work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 120 AND title = btrim(title)),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id)
);
CREATE INDEX ramp_workspace_updated ON reader.ramp (workspace_id, updated_at DESC);
CREATE INDEX ramp_seed ON reader.ramp (seed_work_id);

CREATE TABLE reader.ramp_item (
    ramp_id uuid NOT NULL REFERENCES reader.ramp(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    position integer NOT NULL CHECK (position >= 0),
    via_work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    path_work_ids uuid[] NOT NULL CHECK (
        cardinality(path_work_ids) BETWEEN 1 AND 9
        AND path_work_ids[cardinality(path_work_ids)] = work_id),
    status text NOT NULL DEFAULT 'unread' CHECK (status IN ('unread', 'reading', 'read')),
    note text CHECK (note IS NULL OR char_length(note) BETWEEN 1 AND 2000),
    added_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    status_changed_at timestamptz,
    PRIMARY KEY (ramp_id, work_id),
    CONSTRAINT ramp_item_position UNIQUE (ramp_id, position) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX ramp_item_work ON reader.ramp_item (work_id);

-- Ledger of Works whose outgoing fanout has been made known. The neighborhood itself is
-- graph.citation_assertion; deleting a row only makes the next expand rerun the flow.
CREATE TABLE ops.work_expansion (
    work_id uuid PRIMARY KEY REFERENCES graph.work(id) ON DELETE CASCADE,
    state text NOT NULL CHECK (state IN ('expanded', 'unavailable')),
    method text CHECK (method IN ('provider', 'local', 'existing')),
    unavailable_reason text CHECK (unavailable_reason IN
        ('no_provider_identifier', 'provider_not_found', 'identifier_conflict')),
    provider_key text,
    source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    references_count integer NOT NULL DEFAULT 0 CHECK (references_count >= 0),
    new_works integer NOT NULL DEFAULT 0 CHECK (new_works >= 0),
    warnings jsonb NOT NULL DEFAULT '[]',
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error text,
    expanded_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((state = 'unavailable') = (unavailable_reason IS NOT NULL)),
    CHECK ((state = 'expanded') = (method IS NOT NULL))
);
```

### Modules

```
lysilogy-core
  id.rs          + RampId ('p')
  state.rs       + RampItemStatus {Unread, Reading, Read}, ExpansionState {Expanded, Unavailable},
                   ExpansionMethod {Provider, Local, Existing},
                   ExpansionUnavailableReason {NoProviderIdentifier, ProviderNotFound, IdentifierConflict}
  ramp.rs        (new) RampTitle, RampError, RAMP_PATH_MAX, RAMP_ITEMS_MAX, RAMP_TITLE_FALLBACK,
                   default_ramp_title, RampOrderItem, suggest_order
  expansion.rs   (new) WalkBounds, WALK_DEPTH_MAX, WALK_FANOUT_MAX, WALK_EXPANSIONS_MAX, WalkBoundsError
lysilogy-db
  ramp.rs        (new) RampRepository, Ramp, RampSummary, RampItem, NewRampItem, RampItemPatch
  expansion.rs   (new) ExpansionRepository, ExpansionRow, NewExpansion
  citation.rs    + edges_among(&[WorkId]) -> Vec<(WorkId, WorkId)>, outgoing_counts(&[WorkId])
  work.rs        move_work_content → move_ramps
lysilogy-ingest
  expand.rs      (new) ExpandContext, ProviderExpansion, expand_from_provider, provider_lookup_key
root
  src/graph.rs        + ramps: RampRepository, expansions: ExpansionRepository
  src/expansion.rs    (new) ExpandOptions, ExpansionReport, WalkReport, AppState::expand_work,
                      AppState::walk_from, AppState::expansion_views
  src/walk.rs         (new, pure) FrontierCandidate, plan_frontier
  src/ramps.rs        (new) RampService errors + AppState::{create_ramp, list_ramps, ramp_detail,
                      rename_ramp, delete_ramp, add_ramp_item, update_ramp_item,
                      remove_ramp_item, reorder_ramp, suggest_ramp_order}
  src/main.rs         Command::Expand, Command::Ramp { RampCommand }
  src/api/expansion.rs (new) ExpansionJobs, POST /expand, GET /expansion
  src/api/ramps.rs    (new) ramp routes
  src/api/neighbors.rs + expansions map
web
  src/types.ts, src/lib/api.ts, src/lib/walk.ts (+ scripts/walk.test.mjs),
  src/lib/ramp.ts (+ scripts/ramp.test.mjs), src/lib/literatureLayout.ts (+ layoutWalk),
  src/components/LiteratureGraph.tsx, LiteratureRamp.tsx (new), LiteratureGraph.css,
  HelpOverlay.tsx, web/package.json (test:walk, test:ramp)
```

### Core types

```rust
// lysilogy-core/src/ramp.rs
pub const RAMP_PATH_MAX: usize = 9;          // depth ≤ 8
pub const RAMP_ITEMS_MAX: usize = 500;
pub const RAMP_TITLE_MAX_CHARS: usize = 120;
pub const RAMP_TITLE_FALLBACK: &str = "Reading ramp";
pub struct RampTitle(String);   impl RampTitle { pub fn parse(&str) -> Result<Self, RampError>; pub fn as_str(&self) -> &str }
pub fn default_ramp_title(seed_title: Option<&str>) -> RampTitle;   // truncates at a char boundary, falls back
pub enum RampError { EmptyTitle, TitleTooLong, TitleControlCharacter, ItemExists, ViaNotInRamp,
                     NoCitationEdge, TooDeep, Full, SeedItem, InvalidOrder }
impl RampError { pub const fn code(&self) -> &'static str }   // invalid_ramp_title, item_exists, …
pub struct RampOrderItem { pub work_id: WorkId, pub depth: usize, pub score: f64,
                           pub year: Option<i32>, pub title_key: String, pub is_seed: bool }
pub fn suggest_order(items: &[RampOrderItem], cites: &[(WorkId, WorkId)]) -> Vec<WorkId>;

// lysilogy-core/src/expansion.rs
pub const WALK_DEPTH_MAX: u8 = 3; pub const WALK_FANOUT_MAX: u8 = 10; pub const WALK_EXPANSIONS_MAX: usize = 40;
pub struct WalkBounds { pub depth: u8, pub fanout: u8 }   // Default: depth 1, fanout 5
impl WalkBounds { pub fn parse(depth: Option<i64>, fanout: Option<i64>) -> Result<Self, WalkBoundsError> }
pub enum WalkBoundsError { Depth, Fanout }   // code(): invalid_depth / invalid_fanout
```

```rust
// lysilogy-ingest/src/expand.rs
pub struct ExpandContext<'a, T: JsonTransport> { works, citations, records, client: &'a OpenAlexClient<T> }
pub enum ProviderExpansion {
    Expanded { provider_key: String, source_record: SourceRecordId, references: usize,
               new_works: usize, hydrated: usize, warnings: Vec<String> },
    Unavailable { reason: ExpansionUnavailableReason, provider_key: Option<String>, warnings: Vec<String> },
}
pub fn provider_lookup_key(identifiers: &[WorkIdentifier]) -> Option<String>;   // Decision 4 order
pub async fn expand_from_provider<T: JsonTransport>(ctx: &ExpandContext<'_, T>, work: WorkId)
    -> Result<ProviderExpansion>;

// root src/expansion.rs
pub struct ExpandOptions { pub bounds: WalkBounds, pub refresh: bool, pub local: bool }  // local default true
pub struct ExpansionReport { pub work_id: WorkId, pub cached: bool, pub state: ExpansionState,
    pub method: Option<ExpansionMethod>, pub unavailable_reason: Option<ExpansionUnavailableReason>,
    pub references: usize, pub new_works: usize, pub paper_id: Option<PaperId>, pub warnings: Vec<String> }
pub struct WalkReport { pub seed: WorkId, pub expansions: Vec<ExpansionReport>, pub levels: usize,
                        pub truncated: bool }
impl AppState {
    pub async fn expand_work(&self, work: WorkId, refresh: bool, local: bool) -> Result<ExpansionReport>;
    pub async fn walk_from(&self, seed: WorkId, options: ExpandOptions) -> Result<WalkReport>;
}

// root src/walk.rs (pure)
pub struct FrontierCandidate { pub parent: WorkId, pub work_id: WorkId, pub score: f64,
                               pub year: Option<i32>, pub mark: Option<WorkMark> }
pub fn plan_frontier(candidates: &[FrontierCandidate], parents_in_order: &[WorkId], fanout: usize,
                     visited: &HashSet<WorkId>, budget: usize) -> Vec<(WorkId /*parent*/, WorkId)>;
```

### API

| Route | Behavior |
| --- | --- |
| `POST /api/works/{id}/expand` | Body optional `{depth?, fanout?, refresh?, local?}` (`deny_unknown_fields`; empty body = defaults). 202 `{state: "running"}` or the finished state if a cache hit completed synchronously. 400 `invalid_work_id` / `invalid_body` / `invalid_depth` / `invalid_fanout`; 503 `graph_unavailable`; 404 `work_not_found`. |
| `GET /api/works/{id}/expansion` | 200 `{work_id, job: {state: "idle"\|"running"\|"succeeded"\|"failed", report?: WalkReport, message?}, expansion: ExpansionView \| null}`. |
| `GET /api/graph/neighbors/{id}` | Additive `expansions: Record<WorkId, ExpansionView>` for the center + visible nodes; `ExpansionView = {state: "expanded"\|"unavailable"\|"known", method, references_count, unavailable_reason, expanded_at}` (`known` = Decision 2 without a row; method/time null). Works with unknown fanout have no entry. |
| `GET /api/ramps` | Optional `?seed=<w…>` (malformed → 400 `invalid_work_id`). 200 `{ramps: RampSummaryView[]}` by `updated_at` desc. `RampSummaryView = {id, title, seed_work_id, seed_title, item_count, status_counts: {unread, reading, read}, created_at, updated_at}`. |
| `POST /api/ramps` | `{seed_work_id, title?}`. 201 `RampDetail`. 400 `invalid_body` / `invalid_ramp_title`; 404 `work_not_found`. |
| `GET /api/ramps/{ramp}` | 200 `RampDetail`. 400 `invalid_ramp_id`, 404 `ramp_not_found`. |
| `PATCH /api/ramps/{ramp}` | `{title}`. 200 `RampSummaryView`. |
| `DELETE /api/ramps/{ramp}` | 204; items cascade. |
| `POST /api/ramps/{ramp}/items` | `{work_id, via_work_id?, note?}`. 201 `RampItemView`. 409 `item_exists` / `via_not_in_ramp` / `no_citation_edge` / `ramp_too_deep` / `ramp_full`; 404 `work_not_found`. |
| `PATCH /api/ramps/{ramp}/items/{work}` | `{status?, note?}` (at least one; `note: null` or `""` clears). 200 `RampItemView`. 400 `invalid_status` / `invalid_note` / `invalid_body`; 404 `item_not_found`. |
| `DELETE /api/ramps/{ramp}/items/{work}` | 204; 404 `item_not_found`; 409 `seed_item`. |
| `PUT /api/ramps/{ramp}/order` | Exactly one of `{work_ids: [...]}` or `{strategy: "foundational_first"}`. 200 `RampDetail`. 400 `invalid_order` / `invalid_strategy` / `invalid_body`. |

```ts
RampDetail = { ramp: RampSummaryView, items: RampItemView[],
               local_papers: LocalPapers, acquisitions: AcquisitionStates,
               expansions: Record<WorkId, ExpansionView> }
RampItemView = { work: Work, ordinal: number, depth: number, via_work_id: WorkId | null,
                 path: { work_id: WorkId, title: string | null }[],   // seed..work inclusive
                 status: "unread" | "reading" | "read", note: string | null,
                 added_at: string, updated_at: string, status_changed_at: string | null }
```

### CLI

```
lysilogy expand <work> [--paper] [--depth 1..3] [--fanout 1..10] [--refresh] [--no-local]
lysilogy ramp new <work> [--paper] [--title <t>]
lysilogy ramp list
lysilogy ramp show <ramp>
lysilogy ramp add <ramp> <work> [--via <work>] [--note <n>]
lysilogy ramp set <ramp> <work> [--status unread|reading|read] [--note <n> | --clear-note]
lysilogy ramp remove <ramp> <work>
lysilogy ramp order <ramp> (--suggest | <work>...)
lysilogy ramp delete <ramp>
```

`<work>` is a Work reference (`resolve_work_argument`: `w…`, `W…`, `doi:`, `arxiv:`); with
`--paper` it is a paper (path, ID, or title fragment, like `lit`) resolved to its linked Work
(error when unlinked: "run lysilogy lit first"). `<ramp>` is a `p…` ID or a unique
case-insensitive title prefix. `show` prints one line per item:
`<ordinal>. [<status>] <title> (<year>) · depth <d> · via <via title> · <held|fetch:<state>|—>`.
`expand` prints one line per expansion: `<title>: expanded (<method>, <n> references, <k>
new)` / `cached` / `unavailable (<reason>)`, then `Walked <levels> levels, <n> expansions
(<truncated?>)`.

### UI

- `web/src/lib/walk.ts` (pure): `EXPANSION_TOP_N`, `MAX_UI_EXPANSIONS`, `MAX_UI_DEPTH`,
  `type WalkExpansion = {parentId, childIds, depth}`, `addExpansion(list, parentId, childIds,
  parentDepth)`, `collapseExpansion(list, parentId)` (removes the subtree), `walkPathTo(list,
  centerId, workId) -> string[]` (center … work), `canExpand(list, workId, depthOf)` →
  `ok | too_deep | too_many`, `fanoutKnown(expansionView | undefined, prominence?)`.
- `layoutWalk(base: RankedPlacement[], expansions: WalkExpansion[], centerPoint)` in
  `literatureLayout.ts`: for each expansion in order, parent position P; outward direction =
  unit(P − position of P's walk parent) (center for depth-1); children on an arc of radius
  `HOP_RADIUS = 96` around P spanning ±60° about that direction, evenly spaced in rank order.
  A child already placed → no placement, a `crossLinks` entry instead. Returns `{placements
  (kind "work", hop: number, parentId), walkEdges: {from, to}[], crossLinks: {from, to}[],
  bounds}`; the SVG `viewBox` becomes the padded union of the canvas and `bounds`.
- `LiteratureGraph`: state `walk: WalkExpansion[]`, `walkData: Map<parentId, GraphNeighbors>`
  (child payload maps merged into lookups for the panel and node styling), `expanding:
  Map<workId, "running" | error>`. `w`: if `canExpand` fails → inline notice; if expanded →
  collapse; if fanout known → `graphNeighbors(id, "references", {limit: 8})`; else `POST
  /expand` then poll `GET /expansion` every 1 s (interval dies with the overlay) then fetch.
  Expanded nodes get `data-expanded="true"`, running ones `data-expansion="running"` (a
  pulsing ring), unavailable ones show the reason in the panel. Walk edges are drawn solid,
  cross-links dashed. `j`/`k` order: base placements, then walk children in expansion order.
  Label thinning (`planVisibleLabels`) runs over all placements; hop ≥ 2 labels have lower
  priority than hop 1.
- `LiteratureRamp.tsx` (sidebar, `R`): ramp picker (`<select>` of ramps for this seed + "New
  ramp"), title (click to rename), "Suggest order" button, ordered list: ordinal, status
  button cycling `unread → reading → read → unread` (`aria-label` names the next state), title
  · year, "via A › B" breadcrumb (path minus self, middle-truncated when > 3), depth indent,
  local status: "Open" (`onOpenPaper`) when held, else `acquisitionStatusText` + "Fetch" when
  `canFetch`, expansion badge; note (click to edit, Enter saves, Esc cancels); move up/down
  buttons (`PUT order` with the swapped list); remove ×(not on the seed). Clicking a row
  selects the Work on the canvas if present. Errors inline `role="alert"`. Width 320 px, the
  canvas shrinks; at < 720 px viewport the sidebar overlays the canvas.
- `y`: `missingAncestors(rampItems, walkPathTo(...))` (pure, `ramp.ts`) → add each missing
  ancestor in order with `via` = its predecessor, then the Work. A Work already in the ramp →
  notice "Already in ramp", no request. No current ramp → `POST /api/ramps {seed_work_id:
  center}` first.
- Pure `ramp.ts`: `nextStatus(status)`, `missingAncestors(items, path)`, `swapOrder(ids, index,
  delta)`, `pathBreadcrumb(path, limit)`, `pickCurrentRamp(ramps, rememberedId)`,
  `rampStorageKey(centerId)`.

## Gate (every unit)

```sh
export LYSILOGY_TEST_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy_test
cargo clippy --workspace --all-targets -- -D warnings   # workspace lints: all+pedantic+nursery = deny
cargo test --workspace                                  # DB tests must run, not print "skipping"
# frontend units also: (cd web && npm run typecheck && npm run lint && npm run build && npm run test:walk && npm run test:ramp && npm run test:relevance && npm run test:literature-layout)
```

- `web/node_modules` is a symlink to `~/Projects/Lysilogy/web/node_modules` (created).
- Tests are named `should_<behavior>_when_<condition>` (Rust) / `should … when …` (node).
  DB tests in `lysilogy-db` use `test_support::lock()` / `pool()` (`new_work`, `new_workspace`,
  `cite`); root tests use `test_graph()` and `marker()`; never assert global counts in the
  shared default workspace — assert on rows the test created.
- If root DB tests fail with `PoolTimedOut`, run `scripts/db start`. If the default-workspace
  test flakes, recreate the test DB (`scripts/db dropdb lysilogy_test && scripts/db createdb
  lysilogy_test`), see Ramp D U4 notes.
- Bounded integer conversions: plain `as` casts at the call site (narrowest
  `#[allow(clippy::cast_…)]` if clippy objects). No new fallible conversion paths.
- No network in tests (`RecordingTransport` / the root tests' stub transport). No build caches
  in `/tmp`; scratch under `$CLAUDE_JOB_DIR/tmp` or `~/.cache/`. No personal email anywhere;
  the contact email comes from `LYSILOGY_CONTACT_EMAIL` via existing helpers.

## Implementation units

### U1. Schema, core types, ramp and expansion repositories, merge handling

Files:
- `crates/lysilogy-db/migrations/0008_ramps.sql` (as in Design).
- `crates/lysilogy-core/src/id.rs` (`RampId`, `'p'`), `state.rs` (four `string_enum!`s),
  `ramp.rs` (new), `expansion.rs` (new), `lib.rs` re-exports.
- `crates/lysilogy-db/src/ramp.rs` (new), `expansion.rs` (new), `citation.rs`, `work.rs`,
  `lib.rs`.

Core (`ramp.rs`, `expansion.rs`): as in Core types. `suggest_order` per Decision 13; ignores
`cites` pairs that mention Works not in `items` and self-citations; duplicate items (same
Work twice) keep the first.

DB, `RampRepository::new(pool)` (writes bump `ramp.updated_at = now()`):
- `create(workspace, seed: WorkId, &RampTitle) -> Result<Ramp>` (inserts the seed item at
  position 0, `path = [seed]`, in one transaction).
- `list(workspace, seed: Option<WorkId>) -> Result<Vec<RampSummary>>` (counts by status).
- `get(workspace, RampId) -> Result<Option<Ramp>>`, `items(RampId) -> Result<Vec<RampItem>>`
  (by position), `item(RampId, WorkId) -> Result<Option<RampItem>>`.
- `rename(workspace, RampId, &RampTitle) -> Result<Option<Ramp>>`, `delete(workspace, RampId)
  -> Result<bool>`.
- `add_item(RampId, NewRampItem { work_id, via: Option<WorkId>, note: Option<String> }) ->
  Result<RampItem>`: in one transaction (`select … for update` on the ramp row) checks
  Decision 10 (a)–(c), item count, and existence, returning `DbError::Conflict(code)` with the
  `RampError::code()` strings; the edge check is an `exists` over
  `graph.citation_assertion` with `status in ('asserted','accepted')`.
- `update_item(RampId, WorkId, RampItemPatch { status: Option<RampItemStatus>, note:
  Option<Option<String>> }) -> Result<Option<RampItem>>` (`status_changed_at` only when the
  status differs).
- `remove_item(RampId, WorkId) -> Result<RemoveOutcome>` (`Removed | NotFound | Seed`).
- `reorder(RampId, &[WorkId]) -> Result<()>` (caller validated the permutation; rewrites
  positions 0..n in one transaction; relies on the deferrable unique constraint).
- `Ramp { id, workspace_id, seed_work_id, title, created_at, updated_at }`, `RampSummary {
  ramp, seed_title, item_count, unread, reading, read }`, `RampItem { ramp_id, work_id,
  position, via_work_id, path: Vec<WorkId>, status, note, added_at, updated_at,
  status_changed_at }`, all `Serialize`.

DB, `ExpansionRepository::new(pool)`:
- `get(WorkId) -> Result<Option<ExpansionRow>>`, `rows_for(&[WorkId]) -> Result<Vec<ExpansionRow>>`.
- `record(NewExpansion { work_id, state, method, unavailable_reason, provider_key,
  source_record_id, references_count, new_works, warnings }) -> Result<ExpansionRow>` (upsert;
  `attempts = attempts + 1`, `expanded_at = now()`, `last_error = NULL`).
- `record_failure(WorkId, &str) -> Result<()>` (bumps `attempts`, sets `last_error`; no-op
  when there is no row).
- `CitationRepository::edges_among(&[WorkId]) -> Result<Vec<(WorkId, WorkId)>>` (distinct,
  asserted/accepted, both ends in the set, no self pairs) and `outgoing_counts(&[WorkId]) ->
  Result<HashMap<WorkId, i64>>`.

`move_work_content` → new `move_ramps(transaction, from, into)` per Decision 14.

Tests:
- core (pure): `should_collapse_whitespace_when_parsing_ramp_title`,
  `should_reject_empty_long_and_control_character_ramp_titles` (121 `é` rejected, 120 ok),
  `should_truncate_at_char_boundary_when_defaulting_ramp_title`,
  `should_fall_back_when_seed_title_is_missing_or_blank`,
  `should_round_trip_ramp_id_when_formatting_and_parsing` (prefix `p`; `r…` rejected),
  `should_round_trip_status_and_expansion_enums_when_parsing_strings`,
  `should_reject_out_of_range_depth_and_fanout_when_parsing_walk_bounds` (0, 4, 11, -1,
  `i64::MAX`), `should_default_depth_one_fanout_five_when_bounds_absent`,
  `should_put_cited_item_before_citing_item_when_suggesting_order`,
  `should_order_deeper_then_more_prominent_then_older_when_unconstrained`,
  `should_place_seed_last_when_suggesting_order` (even when the seed is cited by an item),
  `should_break_cycles_deterministically_when_items_cite_each_other`,
  `should_ignore_unknown_and_self_citations_when_suggesting_order`,
  `should_be_deterministic_when_input_order_is_shuffled` (permutations of 6 items),
  `should_return_empty_when_no_items_given`.
- DB-gated: `should_insert_seed_item_when_creating_ramp`,
  `should_list_ramps_by_update_time_with_status_counts`,
  `should_filter_ramps_by_seed_when_listing`,
  `should_build_path_from_via_when_adding_item`,
  `should_conflict_when_item_already_in_ramp`,
  `should_conflict_when_via_is_not_an_item`,
  `should_conflict_when_via_does_not_cite_work` (and succeed for an `accepted` edge, fail for a
  `rejected` one), `should_conflict_when_path_exceeds_nine_works`,
  `should_conflict_when_ramp_is_full` (insert 499 rows by SQL, then one via the API method
  succeeds, the next conflicts), `should_set_status_changed_at_only_when_status_changes`,
  `should_clear_note_when_patch_sets_null`, `should_refuse_to_remove_seed_item`,
  `should_keep_descendant_paths_when_removing_intermediate_item`,
  `should_rewrite_positions_when_reordering` (and the deferrable constraint allows a swap),
  `should_return_none_when_ramp_belongs_to_other_workspace`,
  `should_cascade_items_when_deleting_ramp`,
  `should_violate_check_when_path_does_not_end_with_work` (raw SQL),
  `should_upsert_and_bump_attempts_when_recording_expansion`,
  `should_violate_check_when_unavailable_row_has_no_reason` (raw SQL),
  `should_record_failure_only_on_existing_row`,
  `should_return_edges_among_set_only_when_querying` (rejected and outside edges excluded),
  `should_repoint_items_paths_and_seed_when_merging_works`,
  `should_drop_merged_item_when_survivor_already_in_ramp`,
  `should_move_expansion_row_only_when_survivor_has_none`.

Acceptance: gate green; `scripts/db test-psql -c '\d reader.ramp_item'` shows the table; the
migration applies on the dev DB on next connect.

### U2. Expansion: provider flow, local dispatch, bounded walk, jobs, API, CLI, neighbors map

Files:
- `crates/lysilogy-ingest/src/expand.rs` (new), `lib.rs`.
- `src/expansion.rs` (new), `src/walk.rs` (new, pure), `src/lib.rs`, `src/graph.rs` (add
  `ramps`, `expansions`), `src/api/expansion.rs` (new), `src/api.rs` (merge routes; add
  `expansion_jobs: ExpansionJobs` to `AppState` next to `acquisition_jobs`),
  `src/api/neighbors.rs` (`expansions` map), `src/main.rs` (`Command::Expand`).

Ingest: `provider_lookup_key` and `expand_from_provider` per Decision 4. It loads active
identifiers, fetches with `fetch_work_cached_record` (provider `is_not_found()` or malformed →
`Unavailable(ProviderNotFound)`; other `ProviderError` → `Err`), checks the `openalex_id`
conflict, then `ingest_openalex_references` and `hydrate_stub_works(…, &outgoing)`.

Root:
- `AppState::expand_work(work, refresh, local)` per Decision 3; writes the ledger row via
  `ExpansionRepository::record` (method `local` / `provider` / `existing`), and on `Err` calls
  `record_failure`. `paper_id` in the report is the held paper when the local flow ran.
  Held-paper lookup reuses `api::literature::local_papers_for`'s logic — move the catalog part
  into `AppState::held_papers_for(&[WorkId]) -> Result<BTreeMap<WorkId, Vec<PaperId>>>` in
  `src/expansion.rs` and have `local_papers_for` call it (no behavior change).
- `AppState::walk_from(seed, ExpandOptions)`: level 0 = `[seed]`; for each level expand every
  Work (sequentially), then if `level + 1 < depth`, load outgoing of the expanded Works
  (`citations.outgoing`), their prominence (`prominence_for`, current year) and marks
  (`marks_for_works`), and call `plan_frontier` with the remaining budget
  (`WALK_EXPANSIONS_MAX - done`). An expansion that fails with `Err` aborts the walk with
  that error (the reports so far are logged, not returned).
- `AppState::expansion_views(&[WorkId]) -> Result<BTreeMap<String, ExpansionView>>` (rows +
  `outgoing_counts` for Decision 2's `known`); used by neighbors (U2) and ramps (U3).
- `src/api/expansion.rs`: `ExpandBody { depth: Option<i64>, fanout: Option<i64>, refresh:
  Option<bool>, local: Option<bool> }` (`deny_unknown_fields`, parsed like `fetch`: empty or
  whitespace body = defaults, else must be a JSON object), `ExpansionJobs` per Decision 8 with
  `start(state, work, options)` and `get(work)`; routes per the API table.
- `Command::Expand { work: String, #[arg(long)] paper: bool, #[arg(long, default_value_t = 1,
  value_parser = clap::value_parser!(u8).range(1..=3))] depth: u8, #[arg(long, default_value_t
  = 5, value_parser = …range(1..=10))] fanout: u8, #[arg(long)] refresh: bool, #[arg(long)]
  no_local: bool }`, output per the CLI section.

Tests:
- ingest (pure + DB-gated with `RecordingTransport`):
  `should_prefer_openalex_then_doi_then_arxiv_when_choosing_lookup_key` (inactive identifiers
  ignored; none → `None`), `should_ingest_references_and_title_stubs_when_expanding_stub`
  (fixture record with 3 `referenced_works` + a summary batch: 3 edges, stubs titled),
  `should_report_no_provider_identifier_when_work_has_no_identifiers` (zero requests),
  `should_report_provider_not_found_when_provider_returns_404`,
  `should_report_identifier_conflict_when_openalex_id_belongs_to_other_work` (no assertions
  added), `should_expand_with_zero_references_when_record_cites_nothing`,
  `should_make_no_record_request_when_record_cached` (second expansion: 0 `works/` requests),
  `should_return_error_when_transport_fails_hard`.
- root pure (`src/walk.rs`): `should_take_top_fanout_children_per_parent_by_score_when_planning`,
  `should_skip_hidden_downplayed_visited_and_seed_when_planning`,
  `should_stop_at_budget_when_planning_frontier`,
  `should_break_ties_by_year_then_id_when_scores_equal`,
  `should_be_deterministic_when_candidates_are_shuffled`.
- root DB-gated: `should_return_cached_without_flow_when_fanout_known` (a Work with an
  outgoing assertion and no row → row `existing`, zero transport requests),
  `should_return_cached_unavailable_until_refresh`,
  `should_run_provider_flow_when_work_not_held`,
  `should_run_local_flow_when_work_is_held` (linkable fixture paper as in
  `api::literature::tests`; report `method = local`, `paper_id` set),
  `should_fall_back_to_provider_when_local_flow_links_other_work`,
  `should_walk_two_levels_with_fanout_two_when_bounds_given` (seed → 3 children → each 2
  grandchildren from fixtures: 1 + 2 expansions; third child skipped by score),
  `should_truncate_walk_when_budget_exhausted` (unit-test seam: `walk_from_with_budget`
  `pub(crate)`), `should_record_failure_and_fail_job_when_provider_errors`.
- API: `should_return_400_when_expand_bounds_out_of_range` (`depth: 0`, `4`, `"2"`, `fanout:
  11`, unknown field, `[1]`; no DB needed), `should_return_503_when_expanding_without_graph`,
  `should_return_404_when_expanding_unknown_work`,
  `should_report_succeeded_with_walk_report_when_expand_job_finishes`,
  `should_follow_redirect_when_expanding_merged_work`,
  `should_include_expansions_for_center_and_visible_nodes_when_reading_neighbors` (an
  expanded, an unavailable, a `known`, and an unknown Work).
- CLI parse: `should_reject_depth_and_fanout_out_of_range_when_parsing_expand_command`.

Live check (record in "U2 implementation notes"): with `LYSILOGY_CONTACT_EMAIL` unset or set
from the environment (never hardcoded), `lysilogy expand <a stub cited by "Fine-Tuning
Language Models from Human Preferences">` → `expanded (provider, n references)`; same
command again → `cached`, no network (time it); `lysilogy expand <seed> --paper --depth 2
--fanout 2` on "Learning to summarize from human feedback" → levels, expansions, run time.

Acceptance: gate green; live check recorded; no network in tests.

### U3. Ramps: service, API, CLI, suggested order

Files: `src/ramps.rs` (new), `src/lib.rs`, `src/api/ramps.rs` (new), `src/api.rs` (merge
routes), `src/main.rs` (`Command::Ramp { #[command(subcommand)] command: RampCommand }`).

- `src/ramps.rs`: the `AppState` methods listed in Modules, each taking the default workspace,
  resolving Work IDs through redirects, and returning `RampServiceError { NotFound(&'static
  str /*ramp_not_found|item_not_found|work_not_found*/), Invalid(&'static str, String),
  Conflict(&'static str), Graph(Error) }` so API and CLI share codes.
  `ramp_detail(ramp) -> RampDetail` assembles items, path titles (one `works` lookup for the
  union of path IDs), `local_papers_for`, acquisitions (`states_for`), `expansion_views`.
  `suggest_ramp_order(ramp)`: items → `RampOrderItem` (depth = path length − 1, score from
  `prominence_for`, `title_key` of the display title), `edges_among(item ids)`, then
  `suggest_order` and `reorder`.
  `ramp_reference(text)` for the CLI: `p…` ID, else unique case-insensitive title prefix
  (ambiguous → error listing up to 5 matches).
- `src/api/ramps.rs`: bodies parsed to `serde_json::Value`, must be an object, then into
  `deny_unknown_fields` structs (`CreateRampBody { seed_work_id: String, title:
  Option<String> }`, `RenameRampBody { title: String }`, `AddItemBody { work_id: String,
  via_work_id: Option<String>, note: Option<String> }`, `PatchItemBody { status: Option<Value>,
  note: Option<Value> }` (distinguish absent / null / string by hand), `OrderBody { work_ids:
  Option<Vec<String>>, strategy: Option<String> }`). Routes and codes per the API table.
- CLI per the CLI section; `ramp show` prints items with held/fetch state from the same
  `RampDetail`.

Tests:
- API (DB-gated unless noted): `should_return_503_when_ramps_requested_without_graph`,
  `should_return_400_when_ramp_or_work_ids_are_malformed` (a `w…` in the ramp slot, a `p…` as
  `seed_work_id`; no DB needed), `should_create_ramp_with_seed_item_and_default_title`,
  `should_return_400_when_ramp_title_invalid` (`""`, 121 chars, control char, wrong type),
  `should_list_only_ramps_for_seed_when_seed_given`,
  `should_add_item_with_path_and_titles_when_via_cites_work`,
  `should_return_409_codes_when_item_add_conflicts` (exists, via not in ramp, no edge, too deep),
  `should_return_404_when_adding_unknown_work_or_to_unknown_ramp`,
  `should_update_status_and_note_and_clear_note_with_null`,
  `should_return_400_when_status_unknown_or_patch_empty`,
  `should_return_409_when_removing_seed_item`,
  `should_reorder_when_work_ids_are_a_permutation`,
  `should_return_400_when_order_is_not_a_permutation` (missing, extra, duplicate, malformed,
  both `work_ids` and `strategy`, unknown strategy),
  `should_apply_foundational_first_when_strategy_given` (seed → A → B with A citing B and
  also seed citing B: order B, A, seed),
  `should_include_local_papers_acquisitions_and_expansions_when_reading_ramp`,
  `should_follow_redirects_when_adding_merged_work_and_reading_ramp`,
  `should_return_404_when_ramp_deleted`.
- CLI: `should_parse_ramp_subcommands_when_arguments_valid`,
  `should_reject_order_without_suggest_or_works_when_parsing`,
  `should_reject_note_with_clear_note_when_parsing_set`,
  `should_resolve_ramp_by_unique_title_prefix_and_reject_ambiguous` (DB-gated).

Acceptance: gate green; a curl session against `lysilogy serve` on the dev DB (create ramp
on the seed, add a 1-hop item with `via`, set status/note, suggest order, delete) recorded in
"U3 implementation notes", rows removed afterwards.

### U4. UI: in-place multi-hop expansion

Files: `web/src/types.ts` (`ExpansionView`, `ExpansionJob`, `WalkReport`, `GraphNeighbors.
expansions?`), `web/src/lib/api.ts` (`expandWork(workId, body?)`, `expansionStatus(workId,
signal)`), `web/src/lib/walk.ts` (new), `web/scripts/walk.test.mjs` (new), `web/package.json`
(`test:walk`), `web/src/lib/literatureLayout.ts` (`layoutWalk`), `web/scripts/literature-
layout.test.mjs`, `web/src/components/LiteratureGraph.tsx`, `LiteratureGraph.css`,
`HelpOverlay.tsx`.

Behavior per UI Decisions 18–19 and the UI design (`w`, panel "Expand references" button,
double-click). Walk state resets when the center or top-N/filters change only if a parent
leaves the canvas (keep expansions whose parent is still placed).

Tests (`walk.test.mjs`): `should add expansion with depth one more than parent`, `should
collapse expansion and its descendants`, `should refuse expansion beyond max depth or count`,
`should return path from center through expansion parents`, `should return center only path
when work is a base node`, `should treat known and expanded views as fanout known`.
Layout (`literature-layout.test.mjs`): `should place walk children outward from their parent`
(dot product of child−parent and parent−center > 0), `should spread children within sixty
degrees of the outward direction`, `should draw cross link instead of duplicating a placed
work`, `should grow bounds to include walk children`, `should ignore expansion whose parent
is not placed`, `should be deterministic for the same input`.

Acceptance: web gate green; mock Playwright pass under `~/.cache/lysilogy-smoke/walk-smoke.mjs`
(serves `web/dist`, mocks `/api/*`; imports
`~/Projects/Lysilogy/web/node_modules/playwright/index.mjs`): `w` on an unexpanded node sends
`POST /expand`, polls, then `limit=8`; 8 child nodes appear outside the parent with walk edges;
a child that is also a base node gets a dashed cross-link and no duplicate; `w` on a child
(hop 2) expands again; `w` on the hop-1 node collapses both; a 4th-hop attempt shows the
notice and sends nothing; a known-fanout node fetches neighbors without `POST /expand`; no
console errors; screenshot `walk-expanded.png`. Noted in "U4 implementation notes".

### U5. UI: ramp sidebar and add-to-ramp; live 2-hop smoke; report

Files: `web/src/types.ts` (`RampSummaryView`, `RampDetail`, `RampItemView`, `RampItemStatus`),
`web/src/lib/api.ts` (`listRamps(seed?)`, `createRamp`, `ramp`, `renameRamp`, `deleteRamp`,
`addRampItem`, `updateRampItem`, `removeRampItem`, `orderRamp`), `web/src/lib/ramp.ts` (new),
`web/scripts/ramp.test.mjs` (new), `web/package.json` (`test:ramp`),
`web/src/components/LiteratureRamp.tsx` (new), `LiteratureGraph.tsx` (`R`, `y`, sidebar
wiring, selection sync), `LiteratureGraph.css`, `HelpOverlay.tsx`, `docs/ramps/ramp-e.md`
(notes, result, report).

Tests (`ramp.test.mjs`): `should cycle unread reading read when advancing status`, `should
list ancestors not yet in ramp in path order`, `should return nothing missing when whole path
is in ramp`, `should swap neighbors and clamp at ends when moving item`, `should middle
truncate long breadcrumbs`, `should prefer remembered ramp then most recent when picking`,
`should ignore remembered ramp that no longer exists`.

Scratch Playwright `~/.cache/lysilogy-smoke/ramp-smoke.mjs` (mocked ramp routes implementing
Decisions 10–13 in miniature): `R` opens the sidebar empty-state; `y` on a hop-2 node with no
ramp sends `POST /api/ramps` then two `POST …/items` (ancestor with `via_work_id = center`,
then the node with `via_work_id = ancestor`); the list shows 3 rows with depth indent and the
"via" breadcrumb; the status button cycles and sends `PATCH {"status":"reading"}`; a note
saves on Enter; "Suggest order" sends `PUT …/order {"strategy":"foundational_first"}` and the
list re-renders in the returned order; a held item shows "Open" and clicking it calls the
reader (overlay closes, `/api/papers/<id>` requested); a non-held item shows "Fetch" and sends
`POST /api/works/<id>/fetch`; move-down sends the swapped permutation; remove sends `DELETE`;
no remove button on the seed; at 390 px width the sidebar overlays without horizontal scroll;
no console errors; screenshots `ramp-sidebar.png`, `ramp-narrow.png`.

Live 2-hop smoke (record in "U5 implementation notes"; release build, `lysilogy serve --bind
127.0.0.1:7395 --web web/dist` on the dev DB, scratch data/notes dirs under `~/.cache`, contact
email only from the environment):
1. `GET /api/papers/<seed paper>/work` for "Learning to summarize from human feedback" → seed
   Work `S`.
2. `POST /api/ramps {"seed_work_id": S}` → ramp `R` with the seed item.
3. `POST /api/works/S/expand` → poll → `cached` (`existing`).
4. Pick hop-1 `A` = "Fine-Tuning Language Models from Human Preferences" from `GET
   /api/graph/neighbors/S?limit=40`; `POST /api/ramps/R/items {"work_id": A, "via_work_id": S}`.
5. `POST /api/works/A/expand` (held → `local`, or `existing` if already linked); pick hop-2
   `B` = the top-ranked non-held child of `A` that had no outgoing edges; `POST
   /api/works/B/expand` → `provider`, record references/new works; `POST …/items {"work_id": B,
   "via_work_id": A}`.
6. Add one child `C` of `B` (`via_work_id: B`) to show a 3-deep path; `PUT /api/ramps/R/order
   {"strategy":"foundational_first"}`; `GET /api/ramps/R` → record the order, paths, held/fetch
   states. Expect `C, B, A, S`-compatible order (C before B before A; seed last).
7. Repeat step 5's `expand` for `B` → `cached`, no provider request (time it).
8. UI pass against the same server: open the seed paper, `L`, `w` on `A`, `w` on `B`, `R`
   shows the ramp from step 6; screenshots `live-e-walk.png`, `live-e-ramp.png`.
9. `DELETE /api/ramps/R`; confirm `reader.ramp`/`ramp_item` rows from the check are gone
   (expansion ledger rows and the new stub Works/edges stay: they are canonical graph data).

Report: append "U5 implementation notes", "Result", and "Report" (what shipped, how to
user-test, smoke results, known gaps) to this file.

Acceptance: web gate green incl. `test:ramp`, `test:walk`; the Playwright scripts pass in
mock mode; the live 2-hop smoke recorded; ramp rows created by the check removed.

## Open questions for the user (defaults chosen above)

1. Should a plain click on a node expand it (spec wording) instead of selecting it? Chosen:
   click selects (Ramp D), `w` / double-click / panel button expand.
2. Default walk `depth 1, fanout 5`, cap 40 expansions — enough, or allow deeper walks?
3. Should expanding a held paper run local evidence by default (chosen: yes, no labels) or
   only the provider flow?
4. "Foundational first" ignores reading status — should `read` items sink to the end?
5. Should the seed be removable from its ramp (chosen: no)?

## U1 implementation notes

Shipped: migration `0008_ramps.sql` (Design SQL verbatim), `RampId` (`p`), the four state
enums, `lysilogy_core::{ramp, expansion}`, `RampRepository`, `ExpansionRepository`,
`CitationRepository::{edges_among, outgoing_counts}`, and `move_ramps` called from
`move_work_content`. Core has 77 tests, `lysilogy-db` 97 (all DB-gated tests ran against
`lysilogy_test`); every test named in U1 exists.

Deviations and choices:

- `should_conflict_when_ramp_is_full` bulk-inserts 498 rows (not 499): the seed item already
  counts, so 499 items exist, the API add makes the 500th, the next conflicts.
- `RampTitle::parse` rejects any control character, including tab and newline, like `TagName`.
  `default_ramp_title` is more forgiving, since provider titles may contain them: it drops
  non-whitespace control characters, collapses whitespace, truncates, trims, and falls back.
- `add_item` check order: ramp exists (else `DbError::not_found`), `item_exists`, via checks
  (`via_not_in_ramp`, `no_citation_edge`, `ramp_too_deep`), then `ramp_full`. An empty `note`
  is stored as NULL; `update_item` treats `Some(Some(""))` as a clear too.
- `reorder` takes a `for update` lock on the ramp row and refuses a list that is not exactly
  the ramp's items (`Conflict("invalid_order")`, rolled back) as a backstop; the service layer
  still validates first.
- `remove_item` on a missing ramp returns `NotFound`; the seed is detected by comparing with
  `ramp.seed_work_id`, which `move_ramps` keeps in step across merges.
- `move_ramps` drops `from`'s item when `into` is in the ramp, so `from`'s status and note are
  lost (survivor wins, Decision 14). If the dropped item was the seed item, the survivor's
  item becomes the seed in name but keeps its own path (documented duplicate-path case).
- `NewExpansion`/`ExpansionRow` use `i32` counts and `Vec<String>` warnings to avoid
  conversions at the repository boundary.
- `suggest_order` ignores citation pairs that involve the seed (it is always last), keeps the
  first of duplicate items, and uses `f64::total_cmp` for scores.

Extra tests: `should_expose_stable_error_codes_when_ramp_changes_conflict`,
`should_keep_first_when_item_listed_twice` (core); CHECK coverage for note/position/status
bounds and path lengths 0/9/10, the ledger CHECKs for state/method/reason mismatches, and
`edges_among` with duplicate assertions and repeated input IDs (db).

## U2 implementation notes

Shipped: `lysilogy_ingest::expand` (`ExpandContext`, `ProviderExpansion`, `provider_lookup_key`,
`expand_from_provider`), `src/walk.rs` (`plan_frontier`), `src/expansion.rs` (`ExpandOptions`,
`ExpansionReport`, `WalkReport`, `ExpansionView`, `AppState::{expand_work, walk_from,
expansion_views, held_papers_for}`), `src/api/expansion.rs` (`ExpansionJobs`, `POST
/api/works/{id}/expand`, `GET /api/works/{id}/expansion`), the additive `expansions` map on
`GET /api/graph/neighbors/{id}`, `GraphState.{ramps, expansions}`, and `lysilogy expand`. 39 new
tests (ingest 9, root lib 28, root bin 2); every U2 test named in the plan exists. Gate:
clippy `-D warnings` clean; `cargo test --workspace` with `LYSILOGY_TEST_DATABASE_URL` set
(core 77, db 97, ingest 241, root lib 323, bin 16), no network.

Deviations and choices:

- `held_papers_for` is a thin wrapper over a `pub(crate)` `held_papers_with_titles` that keeps
  the catalog title and document order, so `local_papers_for` (which needs titles) keeps its
  exact behavior and error mapping (`graph_query_failure`). `held_papers_for` sorts by paper
  ID; the dispatch takes the first.
- `ExpansionView.references_count` is `i64` (a `known` view carries the live outgoing count).
  `expansion_views_raw` returns `DbError` for the neighbors route; `expansion_views` wraps it.
- Cache hits: `ExpansionJobs::start` finishes inline (no permit) only for `depth = 1` without
  `refresh` when `AppState::expansion_is_cached` (a ledger row or an outgoing edge). A deeper
  walk always takes the permit because its children may need the network.
- Cached report: `new_works = 0`, no warnings for an expanded row, the stored warnings for an
  unavailable row.
- `WalkReport.truncated` is true only when the budget cut the plan: `plan_frontier` is run
  with the remaining budget and unbounded, and the lengths are compared. A fanout cut is not a
  truncation. The seed always expands even with a zero budget.
- A body of `[1]` or `null` is refused with `invalid_body` before bounds: serde would otherwise
  accept an array for a struct, so the body is parsed to a `Value` and required to be an object.
- `plan_frontier` skips a child an earlier parent already claimed in the same plan, so a Work
  cited by two parents is expanded once.
- CLI lines: `<title>: expanded (<method>, <n> references, <k> new)`, `cached (<method>, <n>
  references)`, `unavailable (<reason>[, cached])`, with `[paper <id>]` for a local run and
  indented `warning:` lines, then `Walked <levels> levels, <n> expansions[, truncated at 40
  expansions] in <s> s`. `--paper` uses `AppState::linked_work` (the existing "run
  `lysilogy lit` first" error for an unlinked paper).

Live check (dev DB, debug build, library read from the main checkout's `local-articles`, data
and notes under `~/.cache/lysilogy-u2`, no `LYSILOGY_CONTACT_EMAIL` set, so unauthenticated):

1. `lysilogy expand W2612675303` ("A Deep Reinforced Model for Abstractive Summarization", a
   no-outgoing-edge stub cited by "Fine-Tuning Language Models from Human Preferences") ->
   `expanded (provider, 39 references, 29 new)`, 1.2 s wall including the network record fetch,
   stub titling, and the prominence refresh.
2. Same command again -> `cached (provider, 39 references)`, 0.08 s wall (process start and
   library scan; the walk itself reports 0.0 s), no network.
3. `lysilogy expand "Learning to summarize from human feedback" --paper --depth 2 --fanout 2`
   -> `cached (existing, 93 references)` for the seed, `cached (existing, 66 references)` for
   "Deep reinforcement learning from human preferences", and `expanded (local, 75 references,
   53 new) [paper a76cf077fbb9974f]` for "Neural Text Summarization: A Critical Evaluation"
   (held, local flow plus local evidence, no warnings). `Walked 2 levels, 3 expansions in 3.5 s`.

The migration `0008_ramps.sql` was applied to the dev DB by the first connect. The new stub
Works, edges, and ledger rows from the check stay: they are canonical graph data.
