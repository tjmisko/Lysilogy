# Reading lists and reading history

Status: MVP built on `feat/reading-lists` (see [MVP status](#mvp-status)); the graph view, `:export` and the live generation smoke are deferred. Drafted 2026-10-24 on `docs/reading-lists-plan`. Generalizes Ramp E
([ramps/ramp-e.md](../ramps/ramp-e.md)). Written to be decomposed into work units for an
autonomous implementation run; every task in [Tasks](#tasks) has files, dependencies and
acceptance tests.

## Goals

1. A reading list is app data in the default workspace. A Work belongs to zero or more lists.
   From any paper or Work you can see, add to and remove from its lists.
2. Every list is a graph: Work nodes, topic nodes, and typed edges. Every view of a list
   (flat sequence, topic DAG, timeline, citation graph, next up, acquire, review) is a
   projection of that one graph.
3. The user adds and removes items by name (library, graph and provider search), from the
   literature graph, and from a paper's references.
4. An agent builds lists for a learning goal ("learn RLHF reward modelling"). It draws on the
   citation graph, provider search, and what the model itself remembers. Every Work it claims
   is checked against a provider before it is admitted as real. Assume most items are **not
   held** as PDFs. The list tracks what to read, not what is on disk.
5. A detailed, append-only reading history: what was opened, for how long, where, and what was
   inspected without being opened. You can return to any paper at its last position.
6. Everything works from the keyboard with vim-style bindings, and looks like the existing
   pages (home grid, literature overlay, ramp sidebar).

Out: sharing lists or multiple workspaces; collaborative editing; a free-form agent tool loop
(the OpenRouter client has no tool calling; generation is a staged pass pipeline, see
[Generation](#generation)); auto-marking items `read`; web search beyond provider registries
(task 04 owns that); cited-by in the literature graph overlay (the overlay stays references
only; cited-by is used by lists and generation, see D18).

## Decisions taken

From the 2026-10-24 interview:

- **D1 Ramps become reading lists.** `reader.ramp` and `reader.ramp_item` migrate into the
  new tables, keeping their IDs. A ramp is a list with `origin = 'walk'` and a `seed_work_id`.
  Per-item citation paths survive as node provenance. `/api/ramps`, `lysilogy ramp` and the
  ramp sidebar are replaced, with no aliases kept.
- **D2 The graph holds papers and topic nodes.** Nodes are `work`, `topic` or `claim`
  (an unverified citation the agent proposed). Stored edges are typed: `prerequisite`
  (work or topic → work or topic: read the source first), `covers` (topic → work) and
  `subtopic` (topic → topic). Citation edges among member Works are derived from
  `graph.citation_assertion` and never stored on the list.
- **D3 Agent candidates come from the graph, provider search, and model memory.** Memory
  proposals are claims until verified. The application admits; the model proposes.
- **D4 History records events plus dwell time** (see [History](#history)).

Taken here (each has a Q in [Open questions](#open-questions) when it is uncertain):

- **D5 Membership is by Work, not paper.** It matches ramps, handles Works you do not hold,
  and survives merges. A paper resolves to its linked Work. A held paper with no linked Work
  is linked first (the `lit` flow); on failure the add returns 409 `paper_not_linked`.
- **D6 Reading status belongs to the Work, not the list.** If you read a paper, it is read in
  every list. New `reader.work_reading_state (workspace, work, status, changed_at)`. Ramp item
  statuses migrate as the max per Work (`read > reading > unread`). Notes stay per list node,
  because why a paper is on a list depends on the list.
- **D7 IDs.** List IDs keep `RampId`'s prefix `'p'`, renamed to `ListId`, so migrated IDs and
  the `localStorage["lysilogy.ramp.*"]` keys keep working. New `NodeId` prefix `'n'` and
  `GenerationId` prefix `'g'`, both currently unused (in use: `w v i s e k d f l c m r t p`).
  History events use `bigserial` and have no public prefix.
- **D8 Acyclicity.** `prerequisite` ∪ `subtopic` must be acyclic. This is checked in pure core
  code before insert (409 `edge_cycle`, naming the cycle). `covers` edges go only
  topic → work. A Work can sit under many topics, so the topic view is a DAG, not a tree.
  Derived citation edges may have cycles; views that need an order break them as
  `suggest_order` already does.
- **D9 Flattening is a topological linearization**, generalizing Ramp E's foundational-first
  order. Constraints: prerequisite edges, then derived citation edges (cited before citing),
  then topic order (a topic's works stay contiguous where the constraints allow). Ties break by
  depth, prominence, older year, title key, ID. When only cycles remain, take the best
  remaining item. The order is deterministic under any shuffle of the input.
- **D10 Manual order wins.** A list has sparse `position`s on Work, claim and topic nodes,
  like ramp items. `PUT order {strategy}` rewrites positions from a strategy. Sorting in a
  view never writes to the list.
- **D11 Agent output is staged by context.** Generating a **new** list admits verified items
  directly, since the whole list is the proposal. Extending an **existing** list adds nodes with
  `review = 'proposed'`, and the user accepts (`a`) or rejects (`x`) them in the Review view,
  matching the graph's accept/reject keys.
- **D12 Unverified citations never create `graph.work` rows.** They stay as `claim` nodes with
  the claimed fields and any provider candidates, until the user resolves one to a Work
  (`c`) or discards it.
- **D13 No confirm dialogs.** Destructive keys act immediately and can be undone with `u`
  (one level per page). `:delete-list` needs the list title typed after it (`:delete-list RLHF
  basics`), the only exception. This follows the browser-dialog rule.
- **D14 Removing the seed is allowed.** It clears `seed_work_id`. The ramp-era 409
  `seed_item` goes away. `via` stays optional. When `via` is given it must be a member, and an
  edge `via → work` must exist (`no_citation_edge`), as today.
- **D15 Limits.** ≤ 500 Work and claim nodes per list (`list_full`), ≤ 200 topics,
  ≤ 4,000 edges. Titles reuse `RampTitle` rules, renamed `ListTitle`. Descriptions are
  ≤ 4,000 chars and notes ≤ 2,000, both via `parse_mark_note`'s rules. Topic labels are
  1..=80 chars.

From the 2026-10-24 follow-up:

- **D16 Generation uses history two ways.** (a) To estimate what you already know: a
  knowledge profile (Works `read`/`reading`, dwell, topics of past lists) lets the plan pass
  skip or shorten basics. Skipped topics are recorded in the report as `assumed_known`, with
  the evidence Works. (b) To connect new items to papers you have already read: a node may
  carry `context_links` to Works from the supplied profile only. Each link has a relation
  (`builds_on`, `extends`, `contrasts`, `same_method`, `same_problem`) and a note of
  ≤ 200 chars. The profile is capped at 120 Works and about 6k tokens, ranked by recency ×
  dwell. Links to Works outside the supplied profile are rejected and counted.
- **D17 Opening a paper nudges it to `reading`.** On the first visit with ≥ 3 min of active
  dwell, an `unread` Work becomes `reading`. Nothing becomes `read` automatically.
- **D18 Cited-by is in scope.** Expansion gains a direction. `cited_by` fetches OpenAlex
  `filter=cites:<id>`, sorted by `cited_by_count` desc, capped at 50 per Work, cached as
  source record `openalex_cited_by`. The results are stored as ordinary
  `graph.citation_assertion` rows (citing → this). Used by: the `A` picker (`Tab` toggles
  references / cited-by), the `frontier` role in generation, and the Next up view's "after
  this" hint.
- **D19 Uppercase resolves key collisions.** Vim's own meanings keep the lowercase keys
  (`dd`/`p`/`P` move rows). List-structure commands take capitals where lowercase is taken
  (`E` edge, `X` remove edge, `T` topics, `O` new topic).
- **D20 Test fixtures come from the vault.** See [Verifier fixtures](#verifier-fixtures).

## Data model: migration `0011_reading_lists.sql`

```sql
CREATE TABLE reader.reading_list (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 120 AND title = btrim(title)),
    description text CHECK (description IS NULL OR char_length(description) BETWEEN 1 AND 4000),
    origin text NOT NULL CHECK (origin IN ('manual', 'walk', 'generated')),
    seed_work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    goal text CHECK (goal IS NULL OR char_length(goal) BETWEEN 1 AND 2000),   -- generated lists
    archived_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id)
);
CREATE INDEX reading_list_updated ON reader.reading_list (workspace_id, updated_at DESC);

CREATE TABLE reader.list_node (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    list_id uuid NOT NULL REFERENCES reader.reading_list(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('work', 'topic', 'claim')),
    work_id uuid REFERENCES graph.work(id) ON DELETE CASCADE,
    label text CHECK (label IS NULL OR char_length(label) BETWEEN 1 AND 80),        -- topic
    summary text CHECK (summary IS NULL OR char_length(summary) BETWEEN 1 AND 2000), -- topic
    claim jsonb,           -- {title, authors[], year, venue, doi, arxiv, raw}
    claim_state text CHECK (claim_state IN ('unverified', 'ambiguous', 'contradicted', 'pending')),
    candidates jsonb NOT NULL DEFAULT '[]',   -- ≤ 5 provider candidates for a claim
    position integer NOT NULL CHECK (position >= 0),
    note text CHECK (note IS NULL OR char_length(note) BETWEEN 1 AND 2000),
    rationale text CHECK (rationale IS NULL OR char_length(rationale) BETWEEN 1 AND 2000),
    context_links jsonb NOT NULL DEFAULT '[]',  -- D16: [{work_id, relation, note}] ≤ 5
    source text NOT NULL CHECK (source IN ('user', 'walk', 'citation', 'agent')),
    review text NOT NULL DEFAULT 'accepted' CHECK (review IN ('accepted', 'proposed')),
    via_work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    path_work_ids uuid[],                       -- ramp heritage; NULL when not walked
    generation_id uuid,                         -- FK added below
    added_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 'work') = (work_id IS NOT NULL)),
    CHECK ((kind = 'topic') = (label IS NOT NULL)),
    CHECK ((kind = 'claim') = (claim IS NOT NULL AND claim_state IS NOT NULL)),
    CONSTRAINT list_node_position UNIQUE (list_id, position) DEFERRABLE INITIALLY DEFERRED
);
CREATE UNIQUE INDEX list_node_work ON reader.list_node (list_id, work_id) WHERE kind = 'work';
CREATE UNIQUE INDEX list_node_topic ON reader.list_node (list_id, lower(label)) WHERE kind = 'topic';
CREATE INDEX list_node_by_work ON reader.list_node (work_id) WHERE work_id IS NOT NULL;

CREATE TABLE reader.list_edge (
    list_id uuid NOT NULL REFERENCES reader.reading_list(id) ON DELETE CASCADE,
    from_node uuid NOT NULL REFERENCES reader.list_node(id) ON DELETE CASCADE,
    to_node uuid NOT NULL REFERENCES reader.list_node(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('prerequisite', 'covers', 'subtopic')),
    source text NOT NULL CHECK (source IN ('user', 'agent')),
    review text NOT NULL DEFAULT 'accepted' CHECK (review IN ('accepted', 'proposed')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_node, to_node, kind),
    CHECK (from_node <> to_node)
);
CREATE INDEX list_edge_list ON reader.list_edge (list_id);

CREATE TABLE reader.work_reading_state (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    status text NOT NULL CHECK (status IN ('unread', 'reading', 'read')),
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id)
);

CREATE TABLE ops.list_generation (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    list_id uuid REFERENCES reader.reading_list(id) ON DELETE SET NULL,
    goal text NOT NULL,
    options jsonb NOT NULL,             -- size, depth, anchors, model, budget
    state text NOT NULL CHECK (state IN ('running', 'succeeded', 'failed', 'over_budget')),
    report jsonb,                       -- counts per pass and verification state
    spend_usd numeric(10, 5) NOT NULL DEFAULT 0,
    model_key text,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);
ALTER TABLE reader.list_node ADD FOREIGN KEY (generation_id)
    REFERENCES ops.list_generation(id) ON DELETE SET NULL;

-- Data move from Ramp E, then drop the old tables.
-- reader.ramp      → reading_list (same id, origin 'walk', seed kept)
-- reader.ramp_item → list_node (kind 'work', source 'walk', via/path kept, same positions)
-- statuses         → work_reading_state (max per work; changed_at = latest status_changed_at)
DROP TABLE reader.ramp_item; DROP TABLE reader.ramp;
```

History tables are in `0012_reading_history.sql` (see [History](#history)), and cited-by is in
`0013_expansion_direction.sql`, so the tracks land independently:

```sql
ALTER TABLE ops.work_expansion ADD COLUMN direction text NOT NULL DEFAULT 'references'
    CHECK (direction IN ('references', 'cited_by'));
ALTER TABLE ops.work_expansion DROP CONSTRAINT work_expansion_pkey,
    ADD PRIMARY KEY (work_id, direction);
```

`expand_work` gains `direction` (default `references`, so Ramp E behavior is unchanged), as
do `POST /api/works/{id}/expand` (`{direction?}`) and `lysilogy expand --cited-by`.

**Merges** (`move_work_content`): `move_lists` repoints `list_node.work_id`. When `into` is
already a node in that list, the `from` node is deleted, its note is appended to the
survivor's when both exist, and its edges move to the survivor (duplicates dropped). Every
`path_work_ids` gets `array_replace`, and `via_work_id`/`seed_work_id` are repointed.
`work_reading_state` keeps the max status. History rows are repointed.

## Verification

Every claim (a citation proposed by the model from memory, or typed in by name with no exact
match) passes through a pure decision over gathered evidence. Gathering is in
`lysilogy-ingest`. The decision is in `lysilogy-core::claim`. Order, first decisive rule wins:

1. **Local exact.** A Work with the same `title_key` and `|year Δ| ≤ 1` (or no year on
   either side) → `matched` (existing Work).
2. **Identifier.** A DOI or arXiv ID is resolved (OpenAlex, then Crossref). If the record's
   title agrees (`!titles_disagree`) → `verified`. If it resolves but the title disagrees →
   `contradicted` (a likely hallucinated or wrong identifier). Fall through to rule 3 with the
   identifier dropped, and keep `contradicted` only when rule 3 also fails.
3. **Title search.** `search_title_cached` → `select_title_match`, plus: year within ±1 when
   claimed, and at least one claimed author surname present when authors are claimed. One
   confident match → `verified`. Two or more plausible matches → `ambiguous` (keep the top 5
   as `candidates`). None → `unverified`.
4. **Transport errors** (timeouts, 5xx) → `pending`, retried by `:verify`. A transport error is
   never `unverified`.

`verified` and `matched` become `work` nodes (`resolve_or_create(Openalex, …)` for new
stubs, then `hydrate_stub_works`). The rest become `claim` nodes. Every state is counted in
the generation report, with no silent drops. Verification is deterministic and network-cached,
and it runs the same for agent claims and user-typed claims.

### Verifier fixtures

The verifier's fixtures are drawn from real citations in the vault, not hand-written ones.
`scripts/build-claim-fixtures` runs once, needs the network, and its output is committed.

- **Real set.** About 40 `source.bibliography_entry` rows from held papers whose resolution
  is `accepted` and has an OpenAlex ID. Spread them across papers, with at least 10 papers and
  ≤ 5 entries per paper. Include ≥ 8 entries with a DOI, ≥ 5 arXiv-only entries and ≥ 10
  title-only entries. The claim is built from the parsed entry fields, not the resolved Work.
  Ground truth = the resolved Work's OpenAlex ID.
- **Fake set.** About 40 perturbations of the real set, each tagged with its kind:
  - a fabricated DOI on a real title;
  - a real DOI on a different real title;
  - a plausible invented title (title words recombined across entries) with real authors;
  - a real title with the year off by ≥ 3;
  - a real title with all authors swapped;
  - an entirely invented entry.
- **Near-miss set.** About 15 real entries with typos, a dropped subtitle or abbreviated
  authors. These should still verify.
- **Recording.** Provider responses are recorded into `RecordingTransport` fixtures
  (`tests/fixtures/claims/`), so tests never touch the network. The script refuses to
  overwrite fixtures without `--rerecord`.

Pass thresholds (`claims_eval` test, report printed):

- **0** fakes `verified` or `matched`. A fake may be `contradicted`, `unverified` or
  `ambiguous`.
- ≥ 90% of the real set `verified`/`matched` to the correct Work, and 0 resolved to the wrong
  Work.
- ≥ 80% of the near-miss set verified correctly.

## Generation

The pipeline is staged and budgeted: no tool loop, OpenRouter by default, the spend ledger
(`src/analysis/spend.rs`), and `model_key` plus the funding source recorded per call, all per
the [agent harness rules](../agent-harnesses.md). A trait `ReadingListPlanner` follows the
`RelationshipLabeler` pattern, with an OpenRouter implementation and a recording fake for
tests. Prompts live in `prompts/reading-list/`.

Input `GenerateRequest {goal, size: 8..=60 (default 20), level: intro | survey | deep,
anchors: [WorkId], into_list: Option<ListId>, budget_usd (default Q4)}`.

1. **Context (deterministic).** Resolve anchors. Build the knowledge profile (D16, pure
   `src/history.rs::knowledge_profile`): Works marked `read`/`reading` and the most-dwelt Works
   of the last 180 days, each with ID, title, year, status, total dwell and the lists it is
   in. Add the titles and topics of existing lists. Gather graph context: title-key search of
   `graph.work` for goal keywords (top 40), plus anchor neighbors in both directions.
2. **Plan pass (model).** The model outputs:
   - a topic DAG (≤ 12 topics, subtopic and prerequisite edges);
   - `assumed_known` topics with the profile Works that justify them;
   - per topic, 2–8 **claims** (title, authors, year, venue, optional DOI/arXiv, role
     `foundation | core | survey | frontier`, rationale ≤ 300 chars, and ≤ 3
     `context_links` to profile Works).

   It can also cite graph-context Works by ID. Output is schema-validated. An ID not in the
   supplied context or profile is rejected and counted.
3. **Verify (deterministic).** Run [Verification](#verification) on every claim. Expand each
   verified Work whose fanout is unknown, capped at 10 expansions in total, using Ramp E's
   `expand_work` at depth 1: references for `foundation`/`core` Works, cited-by (D18) for
   `core`/`frontier` Works.
4. **Fill pass (model, closed world).** For topics with fewer than 2 verified works, and to
   reach `size`, offer the top-ranked neighbors of verified members in both directions
   (prominence order, ≤ 60 candidates, IDs only with title/year), plus the profile. The model
   may pick only from the offered IDs, which prevents hallucination at this stage. It also
   proposes prerequisite edges among chosen works and `context_links` for them.
5. **Admit (deterministic).** Validate the DAG (D8; edges that would close a cycle are dropped
   and counted). Write nodes and edges in one transaction (D11 staging). Apply `topological`
   order positions (D9). Write the report.

Budget: stop at the next pass boundary when the ledger would exceed `budget_usd`. The state
becomes `over_budget`, and everything verified so far is admitted. Jobs mirror
`ExpansionJobs`: one running generation per goal, a semaphore of 1, and progress on the `Q`
queue.

## Views

Every view is a pure projection in `web/src/lib/listViews.ts` (Node tests in
`web/scripts/listViews.test.mjs`) over `ListDetail`. Views never write. `1`–`7` switch views
on the list page, and `v` cycles them.

| # | View | Shows | Notes |
| --- | --- | --- | --- |
| 1 | **Sequence** | Flat rows: ordinal, status glyph (`○ ◐ ●`, as the ramp sidebar), title, year, topics, held / fetch state, note | Sort `s` cycles manual → topological → year ↑ → year ↓ → prominence → title → status → added; `S` reverses. Filter with `/`. Group by topic with `zg`. |
| 2 | **Topics** | Layered DAG of topics (longest-path layering), works under each | `h/l` move between layers, `j/k` within, `zo/zc/za` fold a topic, `zR/zM` open / close all |
| 3 | **Timeline** | Year axis like the year histogram; derived citation edges among members | Reuses `LiteratureHistogram` layout; `h/l` years, `j/k` within a year |
| 4 | **Graph** | Members + derived citations + prerequisite edges, radial or layered | Reuses `literatureLayout`; prerequisite edges solid, citations dashed |
| 5 | **Next up** | Unread works whose prerequisites (direct and through topics) are all `read`, ranked by linearization | The "what now" view; empty-state explains what blocks the first item |
| 6 | **Acquire** | Works not held, with fetch state | `f` fetch selected, `F` fetch all fetchable (sequential, existing acquisition jobs) |
| 7 | **Review** | Proposed nodes/edges, `claim` nodes by state | `a` accept, `x` reject, `c` resolve claim to a candidate, `:verify` retry pending |

A progress strip (read / reading / unread per topic, monospace counts like `.home-counts`)
sits above every view.

## History

Migration `0012_reading_history.sql`:

```sql
CREATE TABLE reader.history_event (
    id bigserial PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    occurred_at timestamptz NOT NULL,
    session_id uuid NOT NULL,          -- one per tab load
    kind text NOT NULL CHECK (kind IN (
        'paper_opened', 'paper_left', 'view_changed', 'section_focused', 'figure_viewed',
        'page_reached', 'search_run', 'question_asked', 'link_followed', 'digest_opened',
        'graph_opened', 'work_inspected', 'list_opened', 'list_changed', 'list_generated',
        'fetch_requested')),
    paper_id text,
    work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    list_id uuid REFERENCES reader.reading_list(id) ON DELETE SET NULL,
    dwell_ms integer CHECK (dwell_ms IS NULL OR dwell_ms >= 0),
    detail jsonb NOT NULL DEFAULT '{}',   -- view, section id, page, figure, query, target
    client_seq integer NOT NULL,          -- per-session ordering and dedup
    UNIQUE (session_id, client_seq)
);
CREATE INDEX history_recent ON reader.history_event (workspace_id, occurred_at DESC);
CREATE INDEX history_paper ON reader.history_event (paper_id, occurred_at DESC);
CREATE INDEX history_work ON reader.history_event (work_id, occurred_at DESC);

CREATE TABLE reader.reading_position (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    paper_id text NOT NULL,
    view text NOT NULL,                -- pdf | markdown | overview | section
    page integer, offset_utf16 integer, section_id text, scroll_ratio real,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, paper_id)
);
```

**Capture (client, `web/src/lib/history.ts`).** An event queue with `session_id` and an
incrementing `client_seq`, flushed every 15 s and on `pagehide` via `navigator.sendBeacon`.
The batch goes to `POST /api/history/events` (≤ 200 events, idempotent on
`(session_id, client_seq)`). Dwell counts only while the tab is visible and focused and there
has been input or scrolling in the last 60 s (`ACTIVE_IDLE_MS`). `paper_left` carries the
visit's dwell. `section_focused` and `figure_viewed` carry their own dwell. `page_reached`
fires at most once per page per visit. `work_inspected` fires when a Work panel stays open
≥ 2 s. Searches record the query text. Network failures re-queue, bounded at 2,000 events
in memory and in `sessionStorage` (wrapped in try/catch). The reading position is upserted
(debounced 2 s) on scroll or page turn.

**Work backfill (server).** On ingest, an event that names a paper but no Work gets the
paper's linked Work before it is stored, and the paper's earlier events without a Work get it
too. Earlier rows are filled after the D17 check, so a qualifying visit stored before the
paper was linked never counts as an earlier nudge.

**Derived (server, pure in `src/history.rs`).** Per-paper summary (first/last seen, visits,
total dwell, sections visited with dwell), "inspected but never opened" Works, and
"opened but barely read" papers (total dwell < 2 min).

**Reading state nudge (D17).** On the first `paper_left` with ≥ 3 min dwell for an `unread`
Work, set it to `reading`. Nothing is set to `read` automatically.

**Knowledge profile (D16).** `knowledge_profile(events, states, lists, now) -> Profile` is a
pure function, so generation and tests share it. A Work's score is
`status_weight × ln(1 + dwell_minutes) × recency`, where `status_weight` is read 1.0,
reading 0.6, untouched-but-dwelt 0.3, and `recency = 0.5^(age_days / 90)`. Take the top 120
Works. Ties break by ID.

**Retention.** Forever, local. `lysilogy history export --since` writes JSONL;
`lysilogy history prune --before` deletes rows.

## API

All routes resolve Work IDs through redirects and answer with resolved IDs. Check order and
error style follow Ramp E (`invalid_*` 400, `*_not_found` 404, conflicts 409).

| Route | Behavior |
| --- | --- |
| `GET /api/lists?work=&archived=` | Summaries by `updated_at` desc: `{id, title, origin, seed_work_id, item_count, topic_count, status_counts, proposed_count, claim_count, updated_at}` |
| `POST /api/lists` | `{title, description?, seed_work_id?}` → 201 `ListDetail` |
| `GET /api/lists/{list}` | `ListDetail {list, nodes, edges, citations: [from,to][], reading_states, local_papers, acquisitions, expansions}` |
| `PATCH /api/lists/{list}` | `{title?, description?, archived?}` |
| `DELETE /api/lists/{list}` | 204 |
| `POST /api/lists/{list}/nodes` | One of `{work_id, via_work_id?, topic_ids?}`, `{paper_id}` (D5), `{topic: {label, summary?}}`, `{claim: {...}}` (runs verification first; 201 with the resulting kind). 409 `item_exists`, `list_full`, `via_not_in_list`, `no_citation_edge`, `paper_not_linked` |
| `POST /api/lists/{list}/nodes:batch` | ≤ 100 of the above, all or nothing; used by multi-select from references |
| `PATCH /api/lists/{list}/nodes/{node}` | `{note?, label?, summary?, review?: "accepted"}`, `{resolve_to: work_id}` for claims |
| `DELETE /api/lists/{list}/nodes/{node}` | 204; cascades its edges. The response body carries the deleted node and edges so the client can undo |
| `POST/DELETE /api/lists/{list}/edges` | `{from, to, kind}`. 409 `edge_cycle {cycle: [node…]}`, `invalid_edge_kind` |
| `PUT /api/lists/{list}/order` | `{node_ids}` exact permutation, or `{strategy: "topological" \| "year" \| "foundational_first"}` |
| `PUT /api/works/{id}/reading-state` | `{status}` |
| `GET /api/works/{id}/lists` | Memberships `{list_id, title, node_id, topics, note}`; also served by `GET /api/papers/{id}/lists` |
| `GET /api/lists/search?q=` | Fuzzy match over list titles (for the picker) |
| `GET /api/works/search?q=&provider=` | Library and graph title-key search; `provider=true` adds OpenAlex title search results marked not-yet-in-graph |
| `POST /api/lists/generate` | `GenerateRequest` → 202 `{generation_id}` |
| `GET /api/generations/{id}` | `{state, report, spend_usd, list_id}` |
| `POST /api/lists/{list}/verify` | Retries `pending` claims |
| `POST /api/history/events` | Batch ingest; 202 `{accepted, duplicates}` |
| `GET /api/history?before=&kinds=&q=&limit=` | Timeline grouped by session |
| `GET /api/history/papers/{id}` | Per-paper summary + position |
| `PUT /api/papers/{id}/position` | Reading position upsert |

## CLI

```
lysilogy list new <title> [--seed <work>] [--description <d>]
lysilogy list ls [--work <work>] [--archived]
lysilogy list show <list> [--view sequence|topics|next|acquire|review]
lysilogy list add <list> (<work> | --paper <paper> | --topic <label> | --claim <text>) [--via <work>] [--topic-of <label>]
lysilogy list rm <list> <node-or-work>
lysilogy list edge <list> <from> <to> --kind prerequisite|covers|subtopic [--remove]
lysilogy list order <list> (--strategy topological|year|foundational-first | <node>...)
lysilogy list generate "<goal>" [--size N] [--level intro|survey|deep] [--anchor <work>]... [--into <list>] [--budget 0.15]
lysilogy list verify <list>
lysilogy list delete <list>
lysilogy read <work> --status unread|reading|read
lysilogy history [--since 7d] [--paper <paper>] [--kinds ...]
lysilogy history export|prune ...
```

`<list>` is a `p…` ID or a unique case-insensitive title prefix (ramp rule). `<work>` uses
`resolve_work_argument`.

## UI

**Style.** Match `HomePage.css` and `LiteratureRamp.tsx`. Use the existing tokens (`--ink`,
`--ink-soft`, `--ink-faint`, `--signal`, `--line`, `--line-bright`, `--panel`,
`--panel-bright`, `--bg-raised`, `--danger`), 11px monospace labels and counts, 5 px radii,
and dense rows. There are no new colors, except that topics reuse tag colors when a topic
label matches a tag (case-insensitive). The selected row uses the overlay selection ring from
`f28c030`, never a second outline. Every control has a `kbd` hint.

**Routes.** `#lists`, `#list=<id>&view=<n>`, `#history`. Add `ListsPage.tsx`,
`ListPage.tsx`, `HistoryPage.tsx`, `ListPicker.tsx`, `ListMembership.tsx` and
`ListViews/*.tsx`. `LiteratureRamp.tsx` becomes `LiteratureListSidebar.tsx`, the same
sidebar backed by lists, with the current list per seed kept in `localStorage`.

### Keymap

Global (outside editable targets; collisions to be checked in T12):

| Keys | Action |
| --- | --- |
| `g l` | Lists index |
| `g r` | Reading history ("recent") |
| `Y` | Add the current paper (reader) / selected paper (home grid) / selected Work (graph) to a list: opens the picker |
| `M` | Lists this paper or Work is in: popover, `j/k`, `↵` open the list, `x` remove from it, `u` undo |
| `:lists` `:list <title>` `:list-new <title>` `:generate <goal>` `:history` `:read` `:reading` `:unread` | Command menu equivalents; `:read` etc. set the current Work's status |

**Picker** (`ListPicker`, styled like F10's PaperSwitcher): type to fuzzy filter lists,
`j/k` or `Ctrl-n/Ctrl-p` move, `↵` add, `Ctrl-↵` create a list with the typed title and add,
`Tab` choose a topic in the target list before adding, `Esc` cancel.

Lists index (`#lists`): `j/k`, `gg/G`, `↵`/`o` open, `/` filter, `n` new (inline input),
`R` rename, `A` archive / unarchive, `za` show archived, `:generate`, `:delete-list <title>`.

List page (`#list=…`):

| Keys | Action |
| --- | --- |
| `j/k` `gg/G` `Ctrl-d/u` | Move |
| `1`–`7` / `v` | Switch view / cycle views |
| `↵` / `o` | Open: reader at the saved position if held, else the Work panel (provenance, `f` fetch) |
| `a` | Add by name: search input over library + graph; `Tab` toggles provider search; claims typed as free text go through verification |
| `A` | Add from the selected Work's references: a checklist (`Tab` toggles references / cited-by, `space` toggle, `↵` add all with `via`) |
| `O` / `T` | New topic below / assign the selected Work to topics (multi-select picker) |
| `E` then target | Draw a prerequisite edge: `E`, move to the target, `↵` (the status line shows "prerequisite of …"); `Esc` cancels |
| `X` | Remove an edge of the selected node: picker over its edges |
| `dd` · `p` / `P` (Sequence) | Cut a row, put below / above (vim reorder) |
| `C` | Show the selected item's context links to papers you have read; `↵` opens one |
| `J/K` | Move the row down / up |
| `x` | Remove the node; `u` undo |
| `r` | Cycle status unread → reading → read (global, D6) |
| `e` | Edit the note inline (Enter saves, Esc cancels, as the ramp sidebar) |
| `s` / `S` | Cycle sort / reverse (view-only); `:order topological` writes it |
| `f` / `F` | Fetch / fetch all (Acquire view) |
| `a` / `x` / `c` (Review) | Accept / reject / resolve a claim |
| `zo zc za zR zM` | Fold topics (Topics, grouped Sequence) |
| `:generate <goal>` | Extend this list (D11 staging) |
| `q` / `Esc` | Leave a mode, then back |

Rows with context links show a faint `↔ <title of read paper>` under the title (first link,
"+n" for more).

History page (`#history`): sessions grouped by day. `j/k` move, `[`/`]` jump day, `↵` reopen
at position, `Y` add to list, `/` filter by text, `1`–`5` filter kinds (papers, sections,
searches, graph, lists), `zo/zc` expand a session, `I` toggles "inspected, never opened".

In the reader: `PaperHeading` shows list chips (title + topic, ≤ 3, "+n"). `M` opens them.
`Ctrl-o` keeps its link-return meaning. Reopening from history restores the position via
`reading_position`.

In the graph overlay: `y` adds to the current list (the ramp behavior, with ancestors via
`via`). `R` toggles the list sidebar. `Y` opens the picker for any list.

## Tasks

Groups run in order. Tasks within a group share no files. Shared hot files (`src/api.rs`,
`src/main.rs`, `src/graph.rs`, `web/src/types.ts`, `web/src/lib/api.ts`, `web/src/App.tsx`,
`HelpOverlay.tsx`) each have exactly one owner per group, named below. Every task follows the
repo gate (`cargo test`, `cargo clippy`, `npm run typecheck`, the relevant `npm run test:*`)
and names tests "should … when …".

| Task | Group | Owns | Depends |
| --- | --- | --- | --- |
| T1 Core types | 1 | `lysilogy-core/src/{id,state,list,list_graph,claim}.rs`, `ramp.rs` → `list.rs` | — |
| T2 Lists schema + repos | 2 | migration 0011, `lysilogy-db/src/{list,reading_state}.rs`, `work.rs` (merge), `ramp.rs` removal | T1 |
| T3 History schema + repo | 2 | migration 0012, `lysilogy-db/src/history.rs` | T1 |
| T4 Claim verification + vault fixtures | 2 | `lysilogy-ingest/src/verify_claim.rs`, `scripts/build-claim-fixtures`, `tests/fixtures/claims/*`, `tests/claims_eval.rs` | T1 |
| T13 Cited-by expansion | 2 | migration 0013, `lysilogy-ingest/src/expand.rs`, `lysilogy-ingest/src/openalex.rs`, `lysilogy-db/src/expansion.rs`, `src/expansion.rs`, `src/api/expansion.rs` | T1 |
| T5 List service, API, CLI | 3 | `src/lists.rs`, `src/api/lists.rs`, `src/api/ramps.rs` removal, `src/api.rs`, `src/main.rs` (incl. `expand --cited-by` flag), `src/graph.rs` | T2, T4, T13 |
| T6 History service + API | 3 | `src/history.rs`, `src/api/history.rs` (mounted by T5's owner in group 4 follow-up, see note) | T3 |
| T7 Generation pipeline | 4 | `src/reading_list_planner/*`, `prompts/reading-list/*`, `src/api/generation.rs`, `src/api.rs`, `src/main.rs` (generate subcommand) | T5 |
| T8 Web foundations | 4 | `web/src/types.ts`, `web/src/lib/{api,listViews,listGraph,history}.ts` + `web/scripts/*.test.mjs`, `web/package.json` | T5, T6 |
| T9 Lists pages + picker + membership | 5 | `ListsPage`, `ListPage` (Sequence view), `ListPicker`, `ListMembership`, `PaperHeading.tsx`, `App.tsx`, CSS | T8 |
| T10 History capture + page | 5 | `HistoryPage.tsx`, `web/src/hooks/useHistoryCapture.ts`, `PdfReader.tsx`/`SectionFocus.tsx` event hooks | T8 |
| T11 Remaining views + graph sidebar | 6 | `ListViews/{Topics,Timeline,Graph,NextUp,Acquire,Review}.tsx`, `LiteratureGraph.tsx`, `LiteratureRamp.tsx` → `LiteratureListSidebar.tsx`, `lib/ramp.ts` removal | T9 |
| T12 Keymap audit, help, docs, smoke | 7 | `HelpOverlay.tsx`, `docs/reading-lists/*`, `docs/architecture.md`, `web/scripts/smoke-lists.mjs` | all |

Note: T6 adds its router via a `history_routes()` fn. T7 mounts both in `src/api.rs`
(group 4 is the only one where `src/api.rs` changes after T5).

Acceptance per task (the test lists below are candidates; see Q10):

- **T1.** `ListTitle` (ramp tests carried over). `validate_edge` rejects
  prerequisite/subtopic cycles, naming the cycle, and rejects covers edges not
  topic → work. `linearize` is deterministic under all permutations of 6 nodes (as
  `suggest_order`'s test), puts a prerequisite before its dependant, keeps a topic contiguous
  when unconstrained, breaks cycles deterministically, and matches `suggest_order` when there
  are no topics or prerequisites. `frontier` excludes items with unread transitive
  prerequisites through topics. `decide_claim` covers each rule in
  [Verification](#verification), including a DOI with a disagreeing title → falls to title
  search, and a transport error → `pending`.
- **T2.** Migration on a DB with ramps keeps IDs, positions, paths and notes, and statuses
  become max per Work. Unique Work per list. Topic label unique case-insensitively. Merge
  rules in [Data model](#data-model-migration-0011_reading_listssql). Positions stay dense
  after reorder.
- **T3.** Batch ingest is idempotent on `(session, seq)`. Rejects > 200 events, unknown kinds
  and negative dwell. Recent query ordering. Position upsert.
- **T4.** Unit tests for each rule plus a timeout → `pending`. `claims_eval` over the vault
  fixtures meets the [thresholds](#verifier-fixtures). The fixture script is checked in and
  documented. No network in tests.
- **T13.** References behavior is unchanged (Ramp E tests pass untouched). Cited-by stores
  ≤ 50 assertions in the citing → this direction. It is cached per direction in the ledger,
  `refresh` reruns it, and both directions on one Work are independent. Uses
  `RecordingTransport` only.
- **T5.** Every route's success and each error code. Batch add is all-or-nothing. Paper add
  links the paper first. Delete returns the undo payload. CLI parse tests. Ramp routes are gone
  (404 via `unknown_api`).
- **T6.** Summary derivation (dwell sums per paper/section), "inspected never opened", the
  `reading` nudge threshold, export/prune.
- **T7.** A fake planner drives the full pipeline: a context ID not supplied → rejected and
  counted. Fill pass restricted to offered IDs. A cycle-closing edge is dropped and counted.
  Budget stop → `over_budget` with partial admission. `into_list` → nodes `proposed`. Report
  counts sum to the claims seen. A `context_link` to a non-profile Work → rejected and
  counted. `assumed_known` is recorded with evidence Works. `knowledge_profile` ranking and
  the 120 cap are tested in T6.
- **T8.** `listViews` projections for each view, sort cycle, filters, grouping, fold state.
  `history` queue: dwell only while active, flush/requeue, cap 2,000, dedup seq.
- **T9.** Picker flow, membership popover add/remove/undo, routes. Sequence keys incl.
  `dd`/`p` reorder and `u` undo (component-level tests where the repo has them; otherwise the
  smoke).
- **T10.** Event emission points fire once (page_reached per page per visit, inspected ≥ 2 s).
  Reopen restores position.
- **T11.** Each view renders the projection and keys move selection correctly. The graph
  sidebar keeps the ramp's `y` semantics against lists.
- **T12.** No key in the new tables collides with an existing binding in the same context
  (scripted check, `npm run test:keymap`, over the table in `web/src/lib/keymap.ts` that
  `HelpOverlay` renders from). Live smoke on the dev DB: create a list from a
  held paper's references and cited-by with `A`, then generate a 10-item list anchored on a
  held paper with the real OpenRouter planner under a $0.15 budget. Pass requires ≥ 7 of the
  10 items to verify, ≥ 1 valid `context_link` when the profile is non-empty, and no item that
  `claims_eval` would classify as fake. The report is saved to
  `docs/experiment-reports/`.

## Resolved (2026-10-24 follow-up)

Per-Work read status (D6). History feeds generation, both for what you know and for
connections (D16). The `reading` nudge stays (D17). Cited-by is in scope (D18). Capitals
resolve key collisions (D19). Fixtures come from the vault (D20).

## Open questions

- **Q2** Should claims typed by the user (by name, no match) also be allowed to stay as
  `claim` nodes indefinitely? Proposal: yes, same as agent claims.
- **Q4** Default generation budget. Proposal: $0.15, matching the per-paper extraction
  ceiling, with `--budget` to raise it. The model is DeepSeek via OpenRouter, the current
  default.
- **Q6** Archive vs delete: is archiving lists useful, or is delete enough?
- **Q8** Should the prefixes be `g l` (lists) and `g r` (history)?
- **Q9** Export: Markdown outline and BibTeX of a list (`:export md|bib`). In this round or
  later?
- **Q10** UI tests. Proposal: Node projection tests, plus one scripted smoke
  (`npm run smoke:lists`, intercepted HTTP like `smoke:paragraph-selection`) that drives every
  new key once. No full browser suite.

## MVP status

What shipped on `feat/reading-lists`, against the tasks above.

Shipped:

- Core types, the lists schema with the Ramp E migration, per-Work reading state, the history
  schema, cited-by expansion and claim verification with vault fixtures (T1 to T4, T13).
- The list service, API and CLI in place of ramps; the history service and API (T5, T6).
- Generation: the staged planner pipeline with a budget stop, the generation run store and the
  `generate` subcommand (T7). The OpenRouter planner is implemented and never exercised live;
  every test drives a recording fake planner.
- Web foundations, the lists index, list page, picker, membership popover, history capture and
  page (T8 to T10), and the Topics, Timeline, Next up, Acquire and Review views with the
  graph sidebar backed by lists (T11).
- Archive (Q6), `g l` / `g r` (Q8), the $0.15 default budget (Q4), claims kept as nodes (Q2).
- T12: the declared keymap with a collision audit (`npm run test:keymap`), the key map overlay
  groups for Lists, List page, the Y picker, the M popover and History, and
  `npm run smoke:lists`, an intercepted-HTTP Playwright smoke against `web/dist` (run
  `npm run build` first). It presses the keys of every list, history, picker and popover surface
  and asserts the HTTP effect of the writes (archive, add, create-and-add, topic add, remove,
  undo, references with `via_work_id`, fetch, context link). Keys pressed without an assertion on
  the result: Review `x` (reject) and `c` then `Enter` (claim resolution), the lists `z a` toggle,
  History `Enter` / `o` reopen, the movement keys, the `z` folds, `h` / `l`, the sort and group keys, and the
  History kind filters.

Deferred:

- View 4, Graph. Key `4` shows "Graph view is not in this build".
- `:export md|bib` (Q9).
- The live smoke of T12: the real planner under a $0.15 budget, and the saved report in
  `docs/experiment-reports/`. Generation quality is therefore unmeasured.
- In the picker, `j` / `k` type into the search on the list stage, so `Ctrl-n` / `Ctrl-p` move
  there; `j` / `k` move only in the topic choice.
- The graph overlay's `y`, `R`, `Y` and `M` are covered by the projection tests and the keymap audit,
  not by `smoke:lists`, which needs no Literature fixture.

Known keymap overlap, waived in the audit with a reason: in the graph with a Work open, `g`
shows the citing sentence at once, so `g l` / `g r` also run that action before the route
changes.
