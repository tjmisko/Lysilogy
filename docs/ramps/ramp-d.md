# Ramp D: "Show me only what matters" — relevance, marks, and tags

Status: planned 2026-10-13. Branch `feat/lit-ramp-d-relevance`, based on
`integration/literature-viewer` at `528297e` (Ramp C merged). Parent plan:
[literature-viewer-plan.md](../literature-viewer-plan.md), section "Relevance, marks, and
tags". Previous ramps: [ramp-a.md](ramp-a.md), [ramp-b.md](ramp-b.md), [ramp-c.md](ramp-c.md).

## What earlier work built

- Ramp A: `lysilogy lit <paper>` links a library PDF to a Work and ingests OpenAlex
  `referenced_works` as stub Works (titles/years hydrated through the batched
  `openalex_work_summary` cache, `SUMMARY_BATCH` IDs per request). `GET
  /api/graph/neighbors/{id}?direction=references|citations|both` returns **every**
  neighbor; the `L` overlay (`web/src/components/LiteratureGraph.tsx`) places them on
  concentric rings of 36 (`layoutFanout`, ordered by year then title) and navigates them
  with `j`/`k`/`↵`.
- Ramp B: local bibliography + in-text mentions (`graph.citation_mention`, counted per edge
  as `mention_count`), grounded relation labels in `graph.relationship_claim.label`
  (`background | uses_method | uses_data | extends | supports | contrasts | critiques |
  compares | other`; current = `superseded_by IS NULL`, `review_state <> 'rejected'`).
- Ramp C: acquisition (`ops.acquisition`), additive `acquisitions` and `local_papers` maps on
  neighbors/citations, a Work panel with a fetch control (`f`).
- The only workspace is the implicit local one (`DocumentRepository::ensure_default_workspace`,
  table `reader.workspace`). There is no `workspace` Postgres schema; workspace-owned state
  lives in `reader`.
- No cited-by counts are cached anywhere today: none of the OpenAlex `select` lists
  (`work_url`, `works_batch_url`, `title_search_url`) include `cited_by_count`.
- `graph.work_identifier.value_normalized` for `openalex` is upper-case (`W2097916756`);
  `source_record.source_key` is lower-case (`source_key()`).

Live dev DB (2026-10-13): 3,240 Works. Largest outgoing fanouts: 332 ("On the
computational complexity of ethics", 2024), 265 ("Vision-Language-Action Models for
Robotics", 2025), 161, 136, 114 — the hairballs this ramp exists for. 118 Works are cited
by more than one Work in the DB; 223 mentions; 68 current labels (30 `background`, 17
`uses_method`, 7 `uses_data`). A live OpenAlex batch request
`/works?filter=openalex:W2410842990|W2097916756&select=id,cited_by_count,publication_year`
returns `cited_by_count` per Work.

## Scope

In:

1. Migration `0007_relevance.sql`: `reader.tag`, `reader.work_tag`, `reader.work_mark`
   (workspace-owned, private), and the derived cache `reader.work_prominence`.
2. A deterministic prominence score from six signals: neighborhood degree (distinct citing
   Works in the DB), global cited-by count (OpenAlex, cached), recency, local availability,
   local mention count, and citation role (the Ramp B relation label: method vs background)
   when known.
3. A new OpenAlex cache kind `openalex_metrics` (batched `cited_by_count`), filled during
   `lit` and by the recompute command; the neighbors endpoint never touches the network.
4. Recompute command `lysilogy prominence` and best-effort incremental refresh after `lit`
   (API and CLI), after local evidence/labels, and after an acquisition maps a Work.
5. API: CRUD for tags, work tags and marks; `GET /api/graph/neighbors/{id}` accepts `limit`,
   `expand`, `tags` and returns ranked visible nodes plus collapsed-remainder clusters, with
   additive `prominence`, `marks`, `work_tags`, `ranking`, `clusters` fields.
6. UI: default top-N view with the remainder as cluster nodes; pin/downplay/hide controls in
   the Work panel; tag create (with color) / assign / unassign; tag legend that doubles as
   a filter; node size from prominence; mark and tag styling.
7. Scratch Playwright check with a mocked 180-node fanout proving the view is navigable.

Out: public/curated tags (plan open decision 4: private only), multiple workspaces or auth,
per-user marks, model-written cluster names or summaries (clusters are deterministic; no
model call anywhere in this ramp, so no new sidecar), force-directed layout, multi-hop
expansion (Ramp E), tag rename merges, bulk mark operations, CLI for tags/marks.

## Decisions taken

1. **Tables live in `reader`, keyed by `workspace_id`.** The plan's `workspace.tag` etc.
   means "workspace-owned"; the repo's convention puts workspace-owned state in `reader`
   (`reader.workspace`, `reader.document`). Every row carries `workspace_id`; the API always
   uses the default workspace. A composite FK `(workspace_id, tag_id) → reader.tag
   (workspace_id, id)` makes a cross-workspace tag assignment impossible.
2. **Tags are private, named, colored, and kinded.** `name` is trimmed with internal
   whitespace collapsed, 1–48 characters, no control characters; uniqueness is
   case-insensitive per workspace (`unique (workspace_id, lower(name))`). `color` is
   `#rrggbb`, stored lower-case; `#RGB` and upper-case input are normalized; anything else
   is 400 `invalid_color`. When omitted, the color is `TAG_PALETTE[count_of_tags % 8]`.
   `kind` is `topic` (default; e.g. `methods`, `foundational`) or `status` (e.g. `read`,
   `ignore`); it only affects legend grouping.
3. **One mark per Work per workspace**: `pinned | downplayed | hidden` plus an optional note
   (trimmed, ≤ 2,000 characters, empty → NULL). Setting a mark replaces the previous one;
   deleting clears it. Marks and tags are independent (an `ignore` tag never hides
   anything by itself).
4. **Work IDs resolve through redirects** on every curation route, and `merge_work` moves
   tags (`on conflict do nothing`) and the mark (the surviving Work's own mark wins) to the
   surviving Work and deletes the merged Work's prominence row.
5. **Prominence is a cache, per workspace** (local availability depends on the workspace),
   with a `signals_version` column (`PROMINENCE_SIGNALS_VERSION = 1`). A row with another
   version, or a missing row, is computed on read. Deleting the whole table loses nothing.
6. **Signals** (all computed in one SQL query for a set of Works, see Design):
   - `citing_works_count`: distinct Works with an `asserted`/`accepted` assertion citing it.
   - `cited_works_count`: distinct Works it cites (stored, shown, not scored).
   - `cited_by_global_count`: `cited_by_count` from the `openalex_metrics` record of its
     active `openalex` identifier; NULL when not cached.
   - `display_year`.
   - `local_available`: a present (`relative_path IS NOT NULL`) document in the workspace.
   - `mention_count`: in-text mentions on assertions citing it.
   - `method_citation_count` (`uses_method`, `uses_data`, `extends`) and
     `background_citation_count` (`background`) over current, non-rejected labels on
     assertions citing it.
7. **Score** (pure `lysilogy_core::prominence::prominence_score(&signals, current_year)`,
   range 0..=1, rounded to 4 decimals). Weighted mean of components over the components
   that are *known*, so a missing count or year neither helps nor punishes:

   | Component | Weight | Value | Known when |
   | --- | --- | --- | --- |
   | degree | 0.30 | `min(1, ln(1+citing) / ln(1+8))` | always |
   | global | 0.25 | `min(1, log10(1+cited_by) / 5)` | count cached |
   | local | 0.15 | 1 if `local_available` else 0 | always |
   | mentions | 0.10 | `min(1, ln(1+mentions) / ln(1+10))` | always |
   | role | 0.10 | 1.0 if any method citation, else 0.3 if any background | any label |
   | recency | 0.10 | `exp(-max(0, current_year - year) / 15)` | year known |

   Negative or absurd inputs (negative counts, year > current_year + 1) are clamped
   (counts at 0, future years count as age 0). NaN is impossible by construction; the
   function is tested for monotonicity in every input.
8. **Marks modulate ranking at query time, never the cache.** Order of the visible set:
   pinned (by score) then unmarked (by score); ties by `display_year` desc (nulls last),
   then title key, then Work ID, so the order is total and stable.
   - `pinned`: always visible, even beyond `limit` (they count toward `limit`; if pinned
     alone exceed it, all pinned are still shown and no unmarked node is).
   - `downplayed`: never in the top-N; always collapsed into the `downplayed` cluster.
   - `hidden`: never visible and never counted in `ranking.total`; listed only in the
     `hidden` cluster so the UI can offer to unhide.
   - The center Work is always node 0 and is never ranked, marked-out, or clustered.
9. **Clusters are deterministic.** Every non-visible neighbor goes to exactly one cluster,
   first rule wins: `hidden` → `downplayed` → `tag:<tag id>` (the work's tag with the
   smallest `lower(name)`) → `decade:<YYYY0>` → `decade:unknown`. Cluster order: tag
   clusters by name, decades descending, `decade:unknown`, `downplayed`, `hidden`. Each
   cluster carries `{key, kind, label, count, work_ids (ranked), sample_titles (top 3)}`.
   Labels: tag name; `2010s`; `Year unknown`; `Downplayed`; `Hidden`.
10. **`expand` makes a cluster's members visible** (after the top-N, in rank order; an
    expanded `hidden` cluster shows hidden nodes with their mark so they can be unhidden).
    Unknown or stale cluster keys are ignored (clusters move as marks change). At most 32
    keys; each ≤ 64 characters, else 400 `invalid_expand`.
11. **`tags` filters before ranking.** Comma-separated tag IDs (≤ 16); a neighbor passes when
    it has any of them. Pinned Works that fail the filter are not shown (the filter is
    explicit intent). Malformed ID → 400 `invalid_tag_id`; a well-formed unknown ID matches
    nothing (no error, empty result plus clusters).
12. **`limit` is optional for compatibility.** Absent → every non-hidden, non-downplayed
    neighbor is visible (Ramp A–C behavior and tests unchanged apart from additive fields;
    hidden ones are still filtered out). Present → integer 1..=500, else 400
    `invalid_limit`. The UI always sends one (default `DEFAULT_TOP_N = 40`).
13. **`edges`, `local_papers`, `acquisitions`, `prominence`, `marks`, `work_tags` cover
    visible nodes only** (plus the center). Clusters carry IDs and titles, not payloads; the
    332-node fanout response stays small.
14. **Cited-by counts come from a new cache kind** `openalex_metrics`, key = lower-case
    `w…`, payload = the batch item (`{id, cited_by_count}`), batched like summaries
    (`works?filter=openalex:A|B…&per_page=n&select=id,cited_by_count`). Widening the
    existing `openalex_work_summary` select would change cached payload shape; a separate
    kind does not. IDs the provider omits are skipped (no negative cache; a later
    recompute retries).
15. **When metrics are fetched.** (a) After every successful `lit` (API job and CLI), for the
    linked Work and its neighbors, best effort: a provider error becomes warning
    `prominence_metrics_failed:<error>` in the log and never fails the job. (b)
    `lysilogy prominence --fetch-metrics`. Never from the neighbors endpoint or tests.
16. **Incremental refresh.** `AppState::refresh_prominence_around(work)` recomputes the
    Work, its outgoing and incoming neighbors (one SQL signal query + one upsert), called
    after `lit` (API job + CLI, after local evidence/labels if they ran) and after an
    acquisition run that ends `mapped`/`already_local` (that Work only: only its
    `local_available` signal changed). Failures are logged with
    `tracing::warn!` and swallowed. Mark/tag changes do not touch the cache.
17. **Recompute command.** `lysilogy prominence [<paper>] [--all] [--fetch-metrics]`:
    with `<paper>` (path, paper ID, or title fragment, like `lit`) → that paper's Work and
    its neighborhood; with `--all` → every Work that participates in any assertion or has a
    document, in chunks of 500. Exactly one of `<paper>`/`--all`. Prints
    `Recomputed <n> works (<m> with global counts) in <t> s` and with `--fetch-metrics`
    also `Fetched cited-by counts for <k> works (<r> requests)`.
18. **No model.** Nothing in this ramp is model-backed, so there is no new trait or sidecar.
    If cluster naming by model is ever wanted it follows the `SidecarLabeler` pattern.
19. **UI keys** (panel closed): `j`/`k`/`h`/`l` move across nodes *and* clusters; `↵` on a
    node opens the panel, on a cluster expands it; `c` collapses every expanded cluster;
    `+`/`=` and `-` change top-N by 20 within 10..=200; `1`–`9` toggle the nth legend tag as
    a filter, `0` clears tag filters. Panel open: `p` pin, `d` downplay, `h` hide (pressing
    the active mark again clears it), `t` focuses the tag input, `f`/`a`/`x`/`e`/`g` as
    before. Top-N, filters and expansions are per overlay session (not persisted).

## Design

### Schema: `0007_relevance.sql`

```sql
CREATE TABLE reader.tag (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 48 AND name = btrim(name)),
    color text NOT NULL CHECK (color ~ '^#[0-9a-f]{6}$'),
    kind text NOT NULL DEFAULT 'topic' CHECK (kind IN ('topic', 'status')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id)
);
CREATE UNIQUE INDEX tag_name ON reader.tag (workspace_id, lower(name));

CREATE TABLE reader.work_tag (
    workspace_id uuid NOT NULL,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    tag_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id, tag_id),
    FOREIGN KEY (workspace_id, tag_id) REFERENCES reader.tag (workspace_id, id) ON DELETE CASCADE
);
CREATE INDEX work_tag_tag ON reader.work_tag (tag_id);

CREATE TABLE reader.work_mark (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    mark text NOT NULL CHECK (mark IN ('pinned', 'downplayed', 'hidden')),
    note text CHECK (note IS NULL OR (char_length(note) BETWEEN 1 AND 2000)),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id)
);

-- Derived cache: safe to truncate; recomputed on read when missing or stale.
CREATE TABLE reader.work_prominence (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    signals_version smallint NOT NULL,
    citing_works_count integer NOT NULL CHECK (citing_works_count >= 0),
    cited_works_count integer NOT NULL CHECK (cited_works_count >= 0),
    cited_by_global_count bigint CHECK (cited_by_global_count >= 0),
    display_year integer,
    local_available boolean NOT NULL,
    mention_count integer NOT NULL CHECK (mention_count >= 0),
    method_citation_count integer NOT NULL CHECK (method_citation_count >= 0),
    background_citation_count integer NOT NULL CHECK (background_citation_count >= 0),
    score double precision NOT NULL CHECK (score >= 0 AND score <= 1),
    computed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id)
);
CREATE INDEX work_prominence_score ON reader.work_prominence (workspace_id, score DESC);
```

### Signal query (`ProminenceRepository::compute_signals`)

One statement for up to 500 Work IDs (`$1` workspace, `$2 uuid[]`):

```sql
with target as (select distinct unnest($2::uuid[]) as id)
select t.id, w.display_year,
  (select count(distinct a.citing_work_id) from graph.citation_assertion a
     where a.cited_work_id = t.id and a.status in ('asserted','accepted')) as citing,
  (select count(distinct a.cited_work_id) from graph.citation_assertion a
     where a.citing_work_id = t.id and a.status in ('asserted','accepted')) as cited,
  (select max((r.payload->>'cited_by_count')::bigint)
     from graph.work_identifier i
     join source.source_record r on r.kind = 'openalex_metrics'
                                 and r.source_key = lower(i.value_normalized)
    where i.work_id = t.id and i.scheme = 'openalex' and i.status = 'active'
      and jsonb_typeof(r.payload->'cited_by_count') = 'number'
      and (r.payload->>'cited_by_count')::numeric >= 0) as cited_by_global,
  exists (select 1 from reader.document d where d.workspace_id = $1
            and d.work_id = t.id and d.relative_path is not null) as local_available,
  (select count(*) from graph.citation_mention m
     join graph.citation_assertion a on a.id = m.assertion_id
    where a.cited_work_id = t.id) as mentions,
  (select count(*) from graph.relationship_claim c join graph.citation_assertion a on a.id = c.assertion_id
    where a.cited_work_id = t.id and c.superseded_by is null and c.review_state <> 'rejected'
      and c.label in ('uses_method','uses_data','extends')) as method_citations,
  (select count(*) … and c.label = 'background') as background_citations
from target t join graph.work w on w.id = t.id
```

Fractional `cited_by_count` values (should not happen) are truncated by the `::bigint`
cast only after the `numeric` guard; the test inserts a string, a negative number, and a
float to pin the behavior (string/negative → NULL; float → truncated).

### Modules

```
lysilogy-core
  id.rs           + TagId ('t')
  state.rs        + WorkMark { Pinned, Downplayed, Hidden }, TagKind { Topic, Status }
  prominence.rs   (new) ProminenceSignals, ProminenceComponents, PROMINENCE_SIGNALS_VERSION,
                  prominence_score, prominence_components
  curation.rs     (new) TagName, TagColor, MarkNote (validated newtypes), TAG_PALETTE,
                  CurationError
lysilogy-db
  curation.rs     (new) CurationRepository, Tag, TagWithCount, StoredMark
  prominence.rs   (new) ProminenceRepository, StoredProminence
  work.rs         move_work_content: move tags/marks, drop prominence
lysilogy-ingest
  openalex.rs     + OPENALEX_METRICS_KIND, works_metrics_url, fetch_cited_by_counts_cached
  prominence.rs   (new) refresh_prominence(ctx, workspace, works) -> RefreshSummary,
                  neighborhood_of(citations, work), fetch_metrics_for(...)
root
  src/graph.rs            + curation, prominence repositories
  src/relevance.rs        (new, pure) NeighborCandidate, RankingRequest, RankedNeighborhood,
                          Cluster, ClusterKind, rank_neighborhood
  src/prominence.rs       (new) AppState::refresh_prominence_around, AppState::recompute_prominence
  src/main.rs             Command::Prominence; refresh after lit
  src/api/curation.rs     (new) tag/work-tag/mark routes
  src/api/neighbors.rs    (new; get_graph_neighbors moves here from api.rs)
  src/api/literature.rs   refresh after run_job; src/api/acquisition.rs refresh after mapped
web
  src/types.ts, src/lib/api.ts, src/lib/relevance.ts (+ scripts/relevance.test.mjs),
  src/lib/literatureLayout.ts (+ layoutRankedFanout, tests),
  src/components/LiteratureGraph.tsx, LiteratureCuration.tsx (new), LiteratureLegend.tsx
  (new), LiteratureGraph.css, HelpOverlay.tsx
```

### Core types

```rust
// lysilogy-core/src/prominence.rs
pub const PROMINENCE_SIGNALS_VERSION: i16 = 1;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProminenceSignals {
    pub citing_works_count: i64, pub cited_works_count: i64,
    pub cited_by_global_count: Option<i64>, pub display_year: Option<i32>,
    pub local_available: bool, pub mention_count: i64,
    pub method_citation_count: i64, pub background_citation_count: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ProminenceComponents {  // each Some(0..=1) when known
    pub degree: f64, pub global: Option<f64>, pub local: f64, pub mentions: f64,
    pub role: Option<f64>, pub recency: Option<f64>,
}
pub fn prominence_components(signals: &ProminenceSignals, current_year: i32) -> ProminenceComponents;
pub fn prominence_score(signals: &ProminenceSignals, current_year: i32) -> f64;

// lysilogy-core/src/curation.rs
pub const TAG_PALETTE: [&str; 8] = ["#4e79a7", "#f28e2b", "#59a14f", "#e15759",
                                    "#76b7b2", "#edc948", "#b07aa1", "#9c755f"];
pub struct TagName(String);   impl TagName { pub fn parse(&str) -> Result<Self, CurationError> }
pub struct TagColor(String);  impl TagColor { pub fn parse(&str) -> Result<Self, CurationError>;
                                              pub fn from_palette(index: usize) -> Self }
pub struct MarkNote;          pub fn parse_mark_note(Option<&str>) -> Result<Option<String>, CurationError>
pub enum CurationError { EmptyName, NameTooLong, NameControlCharacter, InvalidColor, NoteTooLong }
```

```rust
// root src/relevance.rs (pure)
pub struct NeighborCandidate<'a> {
    pub work: &'a Work, pub score: f64, pub mark: Option<WorkMark>,
    pub tags: &'a [TagRef],            // (TagId, lower-cased name) for cluster choice
}
pub struct RankingRequest<'a> {
    pub limit: Option<usize>, pub expand: &'a HashSet<String>, pub tag_filter: Option<&'a HashSet<TagId>>,
}
pub enum ClusterKind { Tag, Decade, Downplayed, Hidden }   // serde snake_case
pub struct Cluster { pub key: String, pub kind: ClusterKind, pub label: String,
                     pub count: usize, pub work_ids: Vec<WorkId>, pub sample_titles: Vec<String> }
pub struct RankedNeighborhood { pub visible: Vec<WorkId>, pub clusters: Vec<Cluster>,
                                pub total: usize, pub hidden: usize, pub pinned: usize,
                                pub filtered_out: usize }
pub fn rank_neighborhood(candidates: &[NeighborCandidate<'_>], request: &RankingRequest<'_>)
    -> RankedNeighborhood;
pub fn cluster_key_for(...);  pub fn decade_label(year: Option<i32>) -> (String, String);
```

### API

| Route | Behavior |
| --- | --- |
| `GET /api/tags` | 200 `{tags: TagView[]}` ordered by `lower(name)`; `TagView = {id, name, color, kind, work_count, created_at, updated_at}`. |
| `POST /api/tags` | Body `{name, color?, kind?}` (`deny_unknown_fields`). 201 `TagView`. 400 `invalid_body` / `invalid_tag_name` / `invalid_color`, 409 `tag_exists`. |
| `PATCH /api/tags/{tag}` | Body `{name?, color?, kind?}`, at least one. 200 `TagView`. 400 `invalid_tag_id` / `invalid_body` / …, 404 `tag_not_found`, 409 `tag_exists`. |
| `DELETE /api/tags/{tag}` | 204; cascades `work_tag`. 404 `tag_not_found`. |
| `GET /api/works/{id}/tags` | 200 `{work_id, tags: TagView[]}`. |
| `PUT /api/works/{id}/tags/{tag}` | 204, idempotent. 404 `work_not_found` / `tag_not_found`. |
| `DELETE /api/works/{id}/tags/{tag}` | 204, idempotent (absent assignment is fine). |
| `GET /api/works/{id}/mark` | 200 `{work_id, mark: MarkView \| null}`; `MarkView = {mark, note, updated_at}`. |
| `PUT /api/works/{id}/mark` | Body `{mark, note?}` (`deny_unknown_fields`). 200 `{work_id, mark: MarkView}`. 400 `invalid_body` / `invalid_mark` / `invalid_note`. |
| `DELETE /api/works/{id}/mark` | 204, idempotent. |
| `GET /api/graph/neighbors/{id}` | New query params `limit`, `expand`, `tags` (Decisions 10–12). Additive fields below. |

All curation routes: 503 `graph_unavailable` without a DB, 400 `invalid_work_id` for a
malformed Work ID, Work IDs resolved through redirects (responses carry the resolved ID).
Check order: path IDs → body → graph availability → existence (as Ramp C).

Neighbors response, additive:

```ts
{
  center, direction, nodes, edges, local_papers, acquisitions,          // existing; visible only
  ranking: { limit: number | null, total: number, visible: number,      // visible excludes center
             pinned: number, hidden: number, filtered_out: number },
  prominence: Record<WorkId, { score: number, signals: ProminenceSignals }>,
  marks: Record<WorkId, { mark: "pinned" | "downplayed" | "hidden", note: string | null }>,
  work_tags: Record<WorkId, TagId[]>,
  clusters: { key: string, kind: "tag" | "decade" | "downplayed" | "hidden", label: string,
              count: number, work_ids: WorkId[], sample_titles: string[] }[],
}
```

`nodes` order is the rank order (center first). `GET /api/works/{id}/citations` gains the
same `marks` and `work_tags` maps for the Works it returns (cheap; the panel uses them).

### UI

- `LiteratureGraph` requests neighbors with `{limit: topN, expand, tags}`; state
  `topN = DEFAULT_TOP_N (40)`, `expanded: Set<string>`, `tagFilter: Set<string>`. Changing
  any of them refetches (abort the previous request); the selected Work stays selected if it
  is still visible (by ID, not index), else the selection clamps.
- Layout: `layoutRankedFanout(centerId, rankedIds, clusterKeys, size)` keeps the server's
  rank order (no year sort): rank fills rings inner → outer, clusters take the slots after the
  last node on the outermost ring (a new ring if that ring is full). Returns placements with
  `kind: "center" | "work" | "cluster"`.
- Nodes: dot radius `nodeRadius(score) = 3.5 + 3.5 * score` (local nodes keep a filled
  center); `data-mark`, `data-score`, `data-tags`; pinned → small ring + pin glyph;
  downplayed (only visible when expanded) → opacity 0.45; hidden (only when the hidden cluster
  is expanded) → dashed outline, label struck through. Up to 3 tag ticks (small colored
  circles) next to the dot; more → `+n`.
- Cluster nodes: rounded pill, `role="listitem"`, `data-cluster-key`, label
  `"<label> · <count>"`, `aria-label="<label>: <count> more works, press Enter to expand"`,
  tooltip/`<title>` with the sample titles. `↵`/click expands. Expanded clusters show as
  removable chips in the legend.
- Legend (`LiteratureLegend`): existing keys, plus "Showing V of T · P pinned · H hidden ·
  `+`/`-` more/fewer"; tag chips (color swatch, name, count in this graph, `1`–`9` index,
  `aria-pressed`) toggling filters; "Clear filters (0)" when any; expanded-cluster chips.
- Work panel (`LiteratureCuration`): "Relevance" section with three toggle buttons Pin `p`,
  Downplay `d`, Hide `h` (`aria-pressed`), an optional note field saved with the mark
  (Enter saves), the score with a one-line signal summary ("cited by 3 here · 1,204 overall
  · 2 mentions · method citation · held locally · 2016"); "Tags" section with assigned chips
  (× removes) and an input (`t` focuses) with a datalist of existing tags; Enter assigns an
  existing tag (case-insensitive match) or creates one with the chosen color (`<input
  type="color">`, default `nextTagColor(tags)`) then assigns. Errors show inline
  (`role="alert"`). After any mark/tag change: optimistic update, then refetch neighbors.
- Pure helpers `web/src/lib/relevance.ts`: `DEFAULT_TOP_N`, `TOP_N_STEP`, `TOP_N_MIN`,
  `TOP_N_MAX`, `nextTopN(current, delta)`, `markAfterToggle(current, requested)`,
  `nodeRadius(score)`, `normalizeTagColor(text)`, `nextTagColor(tags)`,
  `findTagByName(tags, name)`, `neighborQuery({limit, expand, tags})`,
  `signalSummary(prominence)`, `clusterAriaLabel(cluster)`, `toggleInSet(set, value)`.
- Help overlay: the new keys of Decision 19.

## Gate (every unit)

```sh
export LYSILOGY_TEST_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy_test
cargo clippy --workspace --all-targets -- -D warnings   # workspace lints: all+pedantic+nursery = deny
cargo test --workspace                                  # DB tests must run, not print "skipping"
# frontend units also: (cd web && npm run typecheck && npm run lint && npm run build)
```

- `web/node_modules` is a symlink to `~/Projects/Lysilogy/web/node_modules` (created).
- Tests are named `should_<behavior>_when_<condition>`; DB tests in `lysilogy-db` use
  `test_support::lock()` / `pool()`; root tests use `test_graph()` and `marker()` so tag
  names and Works are unique per run (the default workspace is shared by all tests, so
  never assert global counts of tags; assert on the rows the test created).
- Bounded integer conversions use plain `as` casts at the call site (narrowest
  `#[allow(clippy::cast_…)]` if clippy objects). No new fallible conversion paths.
- No network in tests; OpenAlex metrics use the existing fake transports.
- No build caches in `/tmp`; scratch under `$CLAUDE_JOB_DIR/tmp` or `~/.cache/`.
- `npm run smoke` is broken locally; UI checks use the scratch Playwright script (U6).

## Implementation units

### U1. Schema, core types, and curation/prominence repositories

Files:
- `crates/lysilogy-db/migrations/0007_relevance.sql` (as in Design).
- `crates/lysilogy-core/src/id.rs` (`TagId`, prefix `'t'`), `state.rs` (`WorkMark`,
  `TagKind` via `string_enum!`), `curation.rs` (new), `prominence.rs` (new), `lib.rs`
  (re-exports).
- `crates/lysilogy-db/src/curation.rs` (new), `crates/lysilogy-db/src/prominence.rs` (new),
  `crates/lysilogy-db/src/lib.rs`, `crates/lysilogy-db/src/work.rs` (`move_work_content`).

Core:
- `curation.rs`: `TagName::parse` (trim, collapse internal whitespace runs to one space,
  reject empty, > 48 chars counted in `char`s, any `char::is_control`), `as_str`;
  `TagColor::parse` (`#rgb` → `#rrggbb`, lower-case, else `InvalidColor`), `from_palette(i)`
  (`TAG_PALETTE[i % 8]`); `parse_mark_note(Option<&str>)` (trim, empty → `None`, > 2,000
  chars → `NoteTooLong`). `CurationError` with `code()` returning the API error code.
- `prominence.rs`: as in Design, Decision 7. `prominence_score` rounds to 4 decimals.

DB, `CurationRepository::new(pool)` (`updated_at = now()` on writes):
- `create_tag(workspace, &TagName, &TagColor, TagKind) -> Result<Tag>`; unique violation
  (`23505` on `tag_name`) → `DbError::Conflict("tag_exists")`.
- `update_tag(workspace, TagId, Option<&TagName>, Option<&TagColor>, Option<TagKind>) ->
  Result<Option<Tag>>` (None when not in the workspace; conflict as above).
- `delete_tag(workspace, TagId) -> Result<bool>`.
- `list_tags(workspace) -> Result<Vec<TagWithCount>>` (`work_count` from `work_tag`),
  `count_tags(workspace) -> Result<i64>`, `get_tag(workspace, TagId) -> Result<Option<Tag>>`.
- `assign_tag(workspace, WorkId, TagId) -> Result<()>` (`on conflict do nothing`),
  `unassign_tag(workspace, WorkId, TagId) -> Result<()>`,
  `tags_for_works(workspace, &[WorkId]) -> Result<Vec<(WorkId, Tag)>>` (ordered by work, then
  `lower(name)`).
- `set_mark(workspace, WorkId, WorkMark, Option<&str>) -> Result<StoredMark>` (upsert),
  `clear_mark(workspace, WorkId) -> Result<bool>`,
  `marks_for_works(workspace, &[WorkId]) -> Result<Vec<(WorkId, StoredMark)>>`.
- `Tag { id, workspace_id, name, color, kind, created_at, updated_at }`, `StoredMark { mark,
  note, created_at, updated_at }`, all `Serialize`.

DB, `ProminenceRepository::new(pool)`:
- `compute_signals(workspace, &[WorkId]) -> Result<Vec<(WorkId, ProminenceSignals)>>`
  (the Design query; empty input → empty without a query; > 500 IDs → chunks of 500).
- `store(workspace, &[(WorkId, ProminenceSignals, f64)]) -> Result<()>` (one upsert with
  `unnest` arrays, sets `signals_version = PROMINENCE_SIGNALS_VERSION`, `computed_at = now()`).
- `cached(workspace, &[WorkId]) -> Result<Vec<StoredProminence>>` (current version only).
- `prominence_for(workspace, &[WorkId], current_year) -> Result<HashMap<WorkId,
  StoredProminence>>`: cached rows, then compute + store the missing/stale ones.
- `participating_works(after: Option<WorkId>, limit) -> Result<Vec<WorkId>>` (keyset paging
  over Works in any assertion or with a document; for `--all`).
- `StoredProminence { work_id, signals: ProminenceSignals, score: f64, computed_at }`.

`move_work_content` (`work.rs`): insert `from`'s `work_tag` rows for `into` (`on conflict do
nothing`), delete `from`'s; move `from`'s mark only when `into` has none; delete `from`'s
`work_prominence` rows.

Tests:
- core (pure): `should_collapse_whitespace_when_parsing_tag_name`,
  `should_reject_empty_long_and_control_character_tag_names`,
  `should_count_characters_not_bytes_when_limiting_tag_name` (48 `é` ok, 49 rejected),
  `should_expand_short_hex_and_lowercase_when_parsing_color`,
  `should_reject_named_colors_and_bad_hex_when_parsing_color` (`red`, `#12345`, `#ggg`,
  `rgb(…)`, `""`), `should_wrap_palette_index_when_choosing_default_color`,
  `should_drop_empty_note_and_reject_long_note_when_parsing`,
  `should_round_trip_work_mark_and_tag_kind_when_parsing_strings`,
  `should_round_trip_tag_id_when_formatting_and_parsing` (prefix `t`, other prefixes rejected),
  `should_score_zero_when_all_signals_are_empty`,
  `should_ignore_unknown_components_when_averaging` (no year, no count, no labels: score =
  weighted mean of degree/local/mentions only),
  `should_increase_score_when_any_signal_increases` (each signal, holding others fixed),
  `should_cap_components_at_one_when_counts_are_huge` (`i64::MAX` citations),
  `should_clamp_negative_counts_and_future_years_when_scoring`,
  `should_prefer_method_over_background_when_role_known`,
  `should_stay_within_unit_interval_when_signals_vary` (grid over inputs).
- DB-gated: `should_create_and_list_tag_with_zero_count_when_new`,
  `should_conflict_when_tag_name_differs_only_by_case`,
  `should_update_color_and_kind_when_patching_tag`,
  `should_return_none_when_updating_tag_of_other_workspace`,
  `should_cascade_assignments_when_deleting_tag`,
  `should_ignore_duplicate_assignment_when_assigning_twice`,
  `should_reject_cross_workspace_assignment_when_tag_belongs_elsewhere` (raw SQL insert
  fails the composite FK),
  `should_replace_mark_and_note_when_setting_again`,
  `should_report_false_when_clearing_absent_mark`,
  `should_violate_check_when_mark_value_is_unknown` (raw SQL),
  `should_count_distinct_citing_works_and_ignore_rejected_assertions_when_computing_signals`,
  `should_read_cited_by_count_from_metrics_record_when_cached` (and NULL when the payload
  value is a string or negative; truncated when fractional),
  `should_count_mentions_and_method_and_background_labels_when_computing_signals`
  (superseded and rejected labels ignored),
  `should_mark_local_available_only_for_present_documents_in_workspace` (tombstoned and
  other-workspace documents do not count),
  `should_recompute_stale_rows_when_signals_version_differs`,
  `should_return_empty_without_query_when_no_ids_given`,
  `should_move_tags_and_keep_target_mark_when_merging_works`,
  `should_move_mark_when_target_has_none_and_drop_prominence_when_merging`.

Acceptance: gate green; `scripts/db test-psql -c '\d reader.work_prominence'` shows the
table; the migration applies on the dev DB on next connect.

### U2. OpenAlex metrics, refresh orchestration, `lysilogy prominence`, and incremental hooks

Files:
- `crates/lysilogy-ingest/src/openalex.rs`: `OPENALEX_METRICS_KIND = "openalex_metrics"`,
  `works_metrics_url(ids: &[&str]) -> String` (≤ `SUMMARY_BATCH`, `select=id,cited_by_count`),
  `fetch_cited_by_counts_cached(records, client, ids: &[String], refresh: bool) ->
  Result<CitedByFetch, ProviderError>` where `CitedByFetch { counts: Vec<(String /*lower W
  id*/, i64)>, requests: usize }`; cached records are reused unless `refresh`; items whose
  `cited_by_count` is not a non-negative integer are skipped (not cached).
- `crates/lysilogy-ingest/src/prominence.rs` (new), `lib.rs`:
  `ProminenceContext<'a, T> { works, citations, records, prominence, openalex:
  Option<&'a OpenAlexClient<T>> }`; `neighborhood_of(citations, work) -> Vec<WorkId>` (work +
  outgoing + incoming, deduped); `refresh_prominence(ctx, workspace, &[WorkId],
  RefreshOptions { fetch_metrics: bool, refresh_metrics: bool, current_year: i32 }) ->
  Result<RefreshSummary>`; `RefreshSummary { works: usize, with_global_count: usize,
  metrics_fetched: usize, metric_requests: usize, warnings: Vec<String> }`. Metrics fetch
  failure → warning `prominence_metrics_failed:<error>`, refresh continues with cached data.
- Root `src/prominence.rs` (new), `src/lib.rs`, `src/graph.rs` (add `curation:
  CurationRepository`, `prominence: ProminenceRepository`):
  `AppState::refresh_prominence_around(work, fetch_metrics: bool) -> Result<RefreshSummary>`
  and `AppState::recompute_prominence(scope: ProminenceScope, fetch_metrics: bool)` with
  `ProminenceScope::{Paper(PaperId), All}`; current year from `chrono::Utc::now().year()`.
  `AppState::refresh_prominence_quietly(work)` wraps the former with `tracing::warn!` on
  error (used by hooks).
- `src/main.rs`: `Command::Prominence { paper: Option<String>, #[arg(long, conflicts_with =
  "paper")] all: bool, #[arg(long)] fetch_metrics: bool }` (exactly one of paper/`--all`, a
  clap `ArgGroup` with `required = true`); `lit` CLI calls `refresh_prominence_quietly` with
  metrics after the flow (and after local evidence/labels) when a Work was linked.
- `src/api/literature.rs` `run_job`: on success with a linked Work, call
  `refresh_prominence_quietly(work)` with metrics. `src/api/acquisition.rs`: after an
  acquisition ends `mapped`/`already_local`, refresh that Work without metrics.

Tests:
- ingest (pure): `should_join_ids_and_select_cited_by_count_when_building_metrics_url`,
  `should_skip_non_integer_and_negative_counts_when_parsing_metrics_batch`.
- ingest DB-gated with `RecordingTransport`:
  `should_batch_uncached_ids_and_reuse_cache_when_fetching_counts` (60 IDs → 2 requests;
  second call → 0 requests), `should_refetch_when_refresh_requested`,
  `should_keep_refreshing_with_cached_counts_when_metrics_fetch_fails` (transport error →
  warning, rows still stored), `should_refresh_center_and_both_directions_when_refreshing_neighborhood`,
  `should_skip_metrics_when_no_client_given`.
- root: `should_refresh_prominence_after_lit_job_succeeds` (API job with the existing
  recording transport; the linked Work and a cited stub get rows),
  `should_not_fail_lit_job_when_prominence_refresh_fails` (metrics transport errors),
  `should_require_exactly_one_scope_when_parsing_prominence_command` (clap parse tests:
  none, both, paper only, `--all` only),
  `should_recompute_all_participating_works_when_scope_is_all` (DB-gated, asserts the rows the
  test created exist with the current version).

Live check (record in "U2 implementation notes"): `lysilogy prominence "On the
computational complexity" --fetch-metrics` against the dev DB prints counts (expect ~333
Works, 7 requests), then `lysilogy prominence --all` (no network) and the run time.

Acceptance: gate green; live check recorded; no network from tests.

### U3. Curation API: tags, work tags, marks

Files: `src/api/curation.rs` (new), `src/api.rs` (merge routes), `src/api/neighbors.rs` is
untouched in this unit. `GET /api/works/{id}/citations` gains `marks` and `work_tags`
(computed with the new repository methods).

Handlers per the API table. Bodies are parsed by hand like `fetch` (empty body only allowed
for none of these; `PUT /mark` and `POST /tags` require a JSON object). Request structs:
`CreateTagBody { name: String, color: Option<String>, kind: Option<TagKind> }`,
`UpdateTagBody { name: Option<String>, color: Option<String>, kind: Option<TagKind> }` (all
None → 400 `invalid_body`), `SetMarkBody { mark: WorkMark, note: Option<String> }`, all
`#[serde(deny_unknown_fields)]`. Response structs `TagView`, `MarkView`, `WorkTagsResponse`,
`WorkMarkResponse`. Validation through `lysilogy_core::curation`; `CurationError::code()` is
the error code.

Tests (all DB-gated except the 503/400 ones):
- `should_return_503_when_tags_requested_without_graph`
- `should_create_tag_with_palette_color_when_color_omitted`
- `should_normalize_color_and_name_when_creating_tag`
- `should_return_409_when_creating_tag_with_existing_name_in_other_case`
- `should_return_400_when_tag_body_has_unknown_field_or_wrong_type` (also non-object,
  empty body, `name: ""`, `name` of 49 chars, `color: "red"`, `kind: "public"`)
- `should_return_400_when_patch_body_is_empty_object`
- `should_return_404_when_patching_or_deleting_unknown_tag`
- `should_return_400_when_tag_id_is_malformed` (a `w…` ID in the tag slot)
- `should_assign_idempotently_and_list_tags_when_putting_work_tag`
- `should_return_404_when_assigning_unknown_tag_or_work`
- `should_succeed_when_unassigning_absent_tag`
- `should_follow_redirect_when_tagging_merged_work` (response carries the surviving ID)
- `should_set_replace_and_clear_mark_when_using_mark_routes`
- `should_return_400_when_mark_is_unknown_or_note_too_long`
- `should_return_null_mark_when_work_has_none`
- `should_drop_assignments_when_tag_deleted` (GET work tags afterwards is empty)
- `should_include_marks_and_work_tags_when_reading_citations`

Acceptance: gate green; a curl session against `lysilogy serve` on the dev DB (create tag,
assign, mark, read back, delete) recorded in "U3 implementation notes".

### U4. Ranked neighbors: top-N, mark modulation, clusters, tag filter

Files: `src/relevance.rs` (new, pure), `src/lib.rs`, `src/api/neighbors.rs` (new; move
`get_graph_neighbors`, `NeighborsQuery`, `NeighborsResponse`, `NeighborDirection` out of
`src/api.rs`), `src/api.rs` (route now points at the moved handler).

Handler flow: parse ID, direction, `limit`, `expand`, `tags` (400s first) → graph → center →
outgoing/incoming (as today) → default workspace → `prominence_for(all neighbor IDs)` →
`marks_for_works` + `tags_for_works` → `rank_neighborhood` → edges/local papers/acquisitions/
prominence/marks/work_tags for `center + visible` only → response with `ranking` and
`clusters`. `NeighborsQuery { direction: Option<String>, limit: Option<String>, expand:
Option<String>, tags: Option<String> }` (strings so a bad value is our 400, not Axum's
rejection). Parsers: `parse_limit(Option<&str>) -> Result<Option<usize>, …>` (1..=500,
surrounding whitespace rejected), `parse_expand`, `parse_tag_filter` (empty segments
ignored; duplicates collapse).

`rank_neighborhood` implements Decisions 8–11 and returns `RankedNeighborhood`.

Tests:
- pure (`src/relevance.rs`):
  `should_order_by_score_then_year_then_title_then_id_when_ranking`,
  `should_return_every_unmarked_neighbor_when_limit_absent`,
  `should_keep_all_pinned_visible_when_pinned_exceed_limit`,
  `should_count_pinned_toward_limit_when_ranking`,
  `should_collapse_downplayed_even_when_score_is_highest`,
  `should_exclude_hidden_from_total_and_visible_when_ranking`,
  `should_assign_each_remaining_work_to_exactly_one_cluster` (property over a 300-node
  generated input: visible ∪ clusters = all non-center inputs, pairwise disjoint),
  `should_cluster_by_alphabetically_first_tag_before_decade`,
  `should_label_decades_and_unknown_year_when_clustering` (1999 → `1990s`, 2000 → `2000s`,
  negative/absurd years → unknown),
  `should_order_clusters_tags_decades_unknown_downplayed_hidden`,
  `should_append_expanded_cluster_members_after_top_n`,
  `should_ignore_unknown_expand_keys`,
  `should_filter_by_any_selected_tag_before_ranking_and_drop_unmatched_pinned`,
  `should_return_empty_visible_and_no_clusters_when_unknown_tag_filters_everything`,
  `should_never_rank_or_cluster_the_center_when_it_appears_as_neighbor`,
  `should_be_deterministic_when_input_order_is_shuffled`,
  `should_limit_sample_titles_to_three_and_use_placeholder_for_untitled`.
- API (DB-gated unless noted): `should_return_400_when_limit_is_zero_negative_or_not_a_number`
  (`0`, `-1`, `abc`, `501`, `1.5`, ` 5`; no DB needed), `should_return_400_when_tag_filter_id_is_malformed`,
  `should_return_400_when_expand_has_too_many_or_too_long_keys`,
  `should_return_top_n_and_clusters_when_limit_given` (center + 30 seeded cited Works with
  seeded prominence signals, `limit=10`: 10 visible, `ranking.total = 30`, clusters cover the
  other 20), `should_keep_ramp_a_shape_when_limit_absent` (existing test still passes; new
  fields present), `should_hide_hidden_and_pin_pinned_when_marks_exist`,
  `should_expand_cluster_when_expand_key_given`,
  `should_compute_missing_prominence_on_read_when_cache_empty` (rows exist afterwards),
  `should_scope_edges_and_local_papers_to_visible_nodes`,
  `should_filter_by_tag_when_tags_param_given`.

Acceptance: gate green; against the dev DB, `GET /api/graph/neighbors/<work of "On the
computational complexity of ethics">?limit=40` returns 40 visible nodes, `ranking.total` ≈
332 and a handful of clusters; response size and latency recorded in "U4 implementation
notes" (target < 300 ms on the dev DB with a warm cache).

### U5. UI: ranked fanout, clusters, and mark controls

Files: `web/src/types.ts`, `web/src/lib/api.ts`, `web/src/lib/relevance.ts` (new),
`web/scripts/relevance.test.mjs` (new), `web/package.json` (`test:relevance`),
`web/src/lib/literatureLayout.ts` (+ `layoutRankedFanout`), `web/scripts/literature-layout.test.mjs`,
`web/src/components/LiteratureGraph.tsx`, `web/src/components/LiteratureCuration.tsx` (new;
marks part only in this unit), `web/src/components/LiteratureGraph.css`,
`web/src/components/HelpOverlay.tsx`.

- Types: `ProminenceSignals`, `ProminenceView`, `WorkMarkName`, `MarkView`, `NeighborCluster`,
  `NeighborRanking`; extend `GraphNeighbors` and `WorkCitations` with the additive fields
  (optional on `WorkCitations` readers so old mocks do not crash).
- API client: `graphNeighbors(workId, direction, options: {limit?, expand?, tags?}, signal)`
  using `neighborQuery`; `workMark`, `setWorkMark(workId, mark, note)`, `clearWorkMark`.
- `layoutRankedFanout` per the UI design; placements gain `kind` and `clusterKey?`.
- `LiteratureGraph`: top-N state and `+`/`-`, cluster nodes with expand on `↵`/click, `c`
  collapses expansions, selection kept by ID across refetches, node radius/marks styling,
  legend status line ("Showing V of T · P pinned · H hidden"), Work panel "Relevance"
  section with Pin/Downplay/Hide (`p`/`d`/`h`), note, and the signal summary. Mark changes
  are optimistic and trigger a neighbors refetch; an API failure reverts and shows the error.
  Hidden-cluster chip in the legend ("Hidden (n) — show") expands `hidden`.
- Help overlay: Decision 19 keys.

Tests (`web/scripts/relevance.test.mjs`, node test runner):
`should clamp top-N within bounds when stepping`, `should clear mark when toggling the
active mark`, `should switch mark when toggling a different mark`, `should scale radius
monotonically and clamp scores outside 0..1`, `should build neighbor query with sorted
deduplicated expand and tags`, `should omit empty parameters when building neighbor
query`, `should describe signals in words when summarizing prominence` (unknown global count
and year omitted), `should describe cluster with count when labeling for screen readers`.
Layout (`literature-layout.test.mjs`): `should keep server rank order when laying out
ranked fanout`, `should place clusters after the last ranked node`, `should open a new ring
when clusters overflow the outer ring`, `should ignore duplicate and center ids when laying
out ranked fanout`.

Acceptance: gate green incl. `npm run typecheck`, `lint`, `build`, `test:relevance`,
`test:literature-layout`, `test:acquisition`; a quick mock Playwright pass (top-N, cluster
expand, pin/hide) under `~/.cache/lysilogy-smoke/` noted in "U5 implementation notes".

### U6. UI: tags (create with color, assign, legend, filters), 150+ node navigability check, report

Files: `web/src/lib/api.ts` (`listTags`, `createTag`, `updateTag`, `deleteTag`,
`workTags`, `assignTag`, `unassignTag`), `web/src/lib/relevance.ts` (tag helpers),
`web/src/components/LiteratureCuration.tsx` (tags section),
`web/src/components/LiteratureLegend.tsx` (new; the legend moves out of
`LiteratureGraph.tsx`), `LiteratureGraph.tsx`, `LiteratureGraph.css`, `HelpOverlay.tsx`,
`docs/ramps/ramp-d.md` (notes, report).

- Tags load once per overlay (`GET /api/tags`) and after any tag mutation.
- Panel tags section per the UI design (`t` focuses input; Enter assigns or creates; Esc in
  the input blurs it first, then the existing quit chain applies; the color input defaults to
  `nextTagColor`). Tag chips on nodes (≤ 3 ticks, `+n`).
- Legend tag chips toggle the `tags` filter (`1`–`9`, `0` clears); chips show this graph's
  count from `work_tags`; filtered state shows "Filtered by: …".
- Helpers + tests in `relevance.test.mjs`: `should normalize short hex and reject invalid
  colors`, `should choose the first unused palette color then wrap`, `should match existing
  tag names case-insensitively after trimming`, `should toggle tag filter membership`.

Scratch Playwright check `~/.cache/lysilogy-smoke/relevance-smoke.mjs` (not committed;
imports `~/Projects/Lysilogy/web/node_modules/playwright/index.mjs`, serves `web/dist`
with a tiny static server, mocks every `/api/*` route used by the overlay). The mock
neighbors handler generates a deterministic 180-Work fanout (scores descending, years
1960–2025, 12 with tags, 3 pre-pinned, 4 hidden, 6 downplayed) and implements `limit`,
`expand`, `tags` like the server (port of Decisions 8–11, small). Assertions:
1. Pressing `L` renders ≤ 41 work nodes plus cluster nodes (center + top 40), and
   `ranking` text "Showing 40 of 176".
2. All 3 pinned Works are visible; no hidden Work is visible; no downplayed Work is visible.
3. Pressing `j` repeatedly cycles through every placement (nodes and clusters) and returns
   to the center; each focus target is inside the SVG viewBox; no two node label bounding
   boxes on the same ring overlap more than 30 % (computed with `getBBox`).
4. `↵` on a decade cluster sends `expand=decade:…` and the node count grows by that
   cluster's count; `c` removes the expansion.
5. `+` sends `limit=60`; `-` twice sends `limit=20`.
6. In the panel, `p` sends `PUT /mark {"mark":"pinned"}` and the node gets
   `data-mark="pinned"`; `p` again sends `DELETE /mark`; `h` hides the node (it leaves the
   canvas and "Hidden (5)" appears).
7. Typing a new tag name + Enter sends `POST /api/tags` (with the color input's value) then
   `PUT /api/works/<id>/tags/<tag>`; the node gains a tick of that color; the legend shows the
   tag; `1` toggles a filter and the request carries `tags=<id>`; `0` clears.
8. No console errors; screenshots `relevance-default.png`, `relevance-expanded.png`,
   `relevance-filtered.png` in `~/.cache/lysilogy-smoke/`.

Live check: `lysilogy serve --bind 127.0.0.1:7394 --web web/dist` on the dev DB, open "On
the computational complexity of ethics", `L`, confirm the 40-node view, expand one cluster,
pin one Work and tag one Work (then clear both so the dev DB is left as found), and record
it.

Report: append "U6 implementation notes", "Result", and "Report" (what shipped, how to
user-test, smoke results, known gaps) to this file.

Acceptance: gate green incl. all web scripts above; the Playwright script passes in mock
mode; live check recorded; dev DB curation rows created by the check removed.

## Open questions for the user (defaults chosen above)

1. Default top-N of 40 and step 20 — or a smaller default (25) for laptop screens?
2. Should `limit` absent keep returning everything (compat, chosen), or default to 40 on
   the server too?
3. Clustering the remainder by tag then decade (chosen) vs by relation label
   (background/method) once labels are more common.
4. Should downplayed Works still count in `ranking.total` (chosen: yes) and hidden ones not
   (chosen)?
5. Score weights (Decision 7) are a first guess; tune after using the 332-node fanout.

## U1 implementation notes

- No deviations from the plan. Extra DB tests: `should_chunk_and_page_when_many_works_participate`.
- Test helpers `new_workspace` and `cite` were added to `test_support` in `lysilogy-db`.
- The dev Postgres cluster had stopped between runs (root-crate DB tests then fail with
  `PoolTimedOut`); `scripts/db start` fixes it.

## U2 implementation notes

- Added beyond the plan: `AppState::refresh_prominence_alone_quietly(work)` (the acquisition
  hook refreshes that Work only, with no neighbors and no metrics, per the unit text; the
  neighborhood variant would have done more), `parse_cited_by_count` (pure, public), and
  tests `should_not_cache_invalid_or_omitted_counts_when_fetching`,
  `should_make_no_requests_when_counts_already_cached`,
  `should_return_empty_summary_when_no_works_given`, and an assertion in the acquisition
  mapped test that the Work gets a `local_available` row.
- The `lit` API hook wraps the old job body (`run_job` now calls `run_job_steps`, then
  `refresh_prominence_quietly(work, true)`), so it runs after local evidence and labels.
- `RefreshSummary.metrics_fetched` counts Works that have a cited-by count after the fetch
  step (cached or fetched); `metric_requests` is requests actually made.
- Live check against the dev DB (library read from the main checkout, data dir under
  `~/.cache`, no contact email set):
  - `lysilogy prominence "On the computational complexity" --fetch-metrics`: 333 works,
    308 with global counts, 7 requests, 2.1 s.
  - Same command again: 0.4 s, 1 request. The provider omits the 25 Works that have no
    count and Decision 14 deliberately has no negative cache, so those are retried each time.
  - `lysilogy prominence --all` (no network): 3,240 works, 308 with global counts, 0.8 s.
  - `lysilogy prominence` with no scope exits with the clap usage error.

## U3 implementation notes

- `src/api/curation.rs` holds all routes. Bodies are parsed to a `serde_json::Value` first and
  must be an object (otherwise a sequence such as `["x"]` would deserialize into a struct), then
  into the `deny_unknown_fields` structs. `SetMarkBody.mark` stays a raw `Value` so a wrong value
  or type is `invalid_mark` while a missing field is `invalid_body`. An invalid `kind` is
  `invalid_body` (serde rejects the enum).
- `PATCH /api/tags/{tag}` computes `work_count` with a `list_tags` lookup (no repository
  method for a single count); fine at personal-library tag counts.
- `GET /api/works/{id}/tags` reports each tag's global `work_count`, same as `GET /api/tags`.
- `curation::curation_for(state, work_ids)` returns the `marks` and `work_tags` maps and is what
  U4 should reuse for neighbors.
- Tests: 18 in `api::curation::tests` (the 17 listed plus
  `should_check_path_ids_and_body_before_graph_when_graph_unavailable`). The palette test
  accepts the palette color for any tag count in `before..=before+2`, since other DB tests
  create tags in the shared default workspace concurrently.
- Live check against the dev DB (`lysilogy serve --bind 127.0.0.1:7391`, library/data/notes
  under `~/.cache`), on an existing Work, all rows removed afterwards:
  - `POST /api/tags {"name":"u3 curl check"}` -> 201, color `#4e79a7` (first palette color).
  - `PUT /api/works/{w}/tags/{t}` -> 204; `GET .../tags` -> 200 with the tag, `work_count` 1.
  - `PUT .../mark {"mark":"pinned","note":"curl check"}` -> 200; `GET .../mark` -> same.
  - `PUT .../mark {"mark":"loud"}` -> 400 `invalid_mark`.
  - `GET /api/works/{w}/citations` -> `marks` and `work_tags` carry both entries.
  - `DELETE .../mark` -> 204, `DELETE /api/tags/{t}` -> 204, `GET /api/tags` -> `{"tags":[]}`,
    `GET .../mark` -> `mark: null`. `reader.tag`, `work_tag`, `work_mark` counts are 0 after.

## U4 implementation notes

- `src/relevance.rs` is pure (`rank_neighborhood`, `Cluster`, `ClusterKind`, `RankedNeighborhood`);
  the handler lives in `src/api/neighbors.rs`. Parameter errors (`invalid_limit`,
  `invalid_expand`, `invalid_tag_id`, `invalid_direction`) are returned before the graph is
  consulted.
- `curation::curation_rows` loads stored marks and tags once; `curation_for` and the neighbors
  handler both shape payloads from it (`marks_payload`, `work_tags_payload`).
- Adjustment: `should_create_tag_with_palette_color_when_color_omitted` now asserts the color is
  a palette color, not a specific palette index. The new tag-filter test creates tags in the
  shared default workspace concurrently, so the index window from U3 was racy.
- Test DB hygiene: DB tests leave workspaces behind, and `ensure_default_workspace` picks the
  oldest, so a long-lived `lysilogy_test` can make
  `should_report_mapped_acquisition_and_paper_id_when_job_finished` fail. Fix by
  `scripts/db dropdb lysilogy_test && scripts/db createdb lysilogy_test`.
- Live check against the dev DB (`lysilogy serve --bind 127.0.0.1:7391`, release build, scratch
  library/data/notes under `~/.cache`), Work of "On the computational complexity of ethics",
  `direction=both&limit=40`:
  - 40 visible neighbors (41 nodes with the center), `ranking.total` 332, pinned 0, hidden 0.
  - 14 clusters, all decades (2020s 19, 2010s 65, 2000s 78, 1990s 43, 1980s 25, 1970s 15,
    1960s 3, 1950s 8, 1940s 2, 1930s 5, 1920s 1, 1850s 1, 1780s 1, unknown 26); they sum to 292
    = 332 - 40. No tags exist in the dev DB yet, so no tag clusters.
  - Response 37,513 bytes; latency 52 ms cold, 12-23 ms warm (target < 300 ms). Without
    `limit`: 219,422 bytes, 20 ms.
