# Literature viewer: high-level plan

Status: active plan, 2026-10-12. This is the working plan for building the literature
viewer from the ground up. It supersedes the execution order in the old knowledge-base
plans where they conflict; the [Postgres refactor proposal](postgres-knowledge-base-refactor-plan.md)
remains the storage/runtime reference.

## Goal

Turn Lysilogy from a reader for one PDF into a viewer for a literature: from a paper,
see the surrounding papers it cites, understand *why* it cites them, fetch the ones we
do not hold, and navigate the graph without drowning in it.

The end state is a graph view that makes relationships between papers legible. This
document defines the ramp of user-testable capabilities toward that end.

## Settled architecture decisions

| Decision | Choice |
| --- | --- |
| Repository layout | Cargo workspace: `lysilogy-core`, `lysilogy-db`, `lysilogy-ingest`, `lysilogy-api`, `lysilogy-cli` (dependency direction api/ingest → db → core). |
| Database driver | `sqlx` (async, `sqlx::migrate!`, migrations owned in-repo). |
| Schema organization | Postgres schemas: `source`, `graph`, `reader`, `publish`, `ops`. |
| Canonical storage | Postgres is canonical. Per-paper file artifacts remain as a **cache** (full-fidelity display payload), never the source of truth. |
| Local database | System PostgreSQL binaries in a self-contained userspace cluster on port 5433 (no root, no Docker). One dev DB and one test DB. |

### Development database

The dev cluster uses the system PostgreSQL binaries but its own data directory under
`~/.local/share/lysilogy/`, on port 5433, so it never collides with a system service.

```sh
scripts/db start     # start and wait for readiness
scripts/db status    # pg_isready
scripts/db psql      # psql into the dev database `lysilogy`
scripts/db test-psql # psql into `lysilogy_test`
scripts/db stop      # stop the cluster
```

### What we keep

- The reader (Axum + React): PDF geometry, notes, highlights, link hints.
- Native PDF index, paragraph detection, figure/table detection, graphics region evidence.
- Content-hash identity logic (ports to `document_version.sha256`; move-tolerant).
- Name parser and title normalizer (pure, tested; resolution needs them).
- KB domain vocabulary (seed for `lysilogy-core`; expected to be reshaped).
- Provider adapters and cache/rate budgets.

### What we scrap

- The SQLite store mechanics: journal, `decisions.jsonl`, `canonical_records`,
  `projection_state`, replay checkpoints, and the "rebuild identical IDs from an empty
  database" identity contract. Postgres WAL, transactions, and `work_redirect` replace it.
- The E0–E8 phase/wave lattice and the K0–K7 / 5-gate / 30-objective apparatus. Keep a
  thin regression corpus, not a truth-admission pipeline.
- `objects.json` / `reading-index.json` as canonical state.
- `PaperId` as the identity handed to clients.

## Core model in one line

```
immutable evidence   →   attributed claims   →   selected/curated state   →   rebuildable caches
source_record            field_claim             work.display_title            FTS indexes
bibliography_entry       work_identifier         entry_resolution              graph tiles/layout
citation_mention         citation_assertion      work_redirect                 neighborhoods
                         authorship              resolution_decision
```

Evidence is never mutated. Claims are never destroyed, only superseded. Entities get
immutable opaque IDs. Caches are the only thing regenerated.

## Where citation edges come from

There are two independent producers of the same canonical edge:

- **Provider edges** — OpenAlex `referenced_works` (or Crossref deposited references):
  cheap topology with identifiers; no full text.
- **Local edges** — extracted PDF bibliography and in-text mentions: the raw entry and
  the passage, but needs resolution to align with the cited Work.

A canonical edge is the `(citing_work, cited_work)` pair. Each source contributes its own
`citation_assertion` with independent provenance. We build **hybrid**: provider supplies
topology, local text supplies the *why*. Provider is the faster first step; local evidence
is the product differentiator and must be layered on regardless.

## The ramp

Each ramp is independently testable and additive. Ramp B carries the core product value.

### Ramp A — "Show me what this paper cites" (topology only)

Open a local paper → `L` opens a graph centered on its Work → fanout to cited Works from
one provider call.

- Needs: local paper → Work match; provider ingest of referenced Works; canonical edges;
  graph API + UI.
- No AI, no acquisition, no PDF bibliography parsing.
- Testable moment: point at a paper with a DOI and see a real citation fanout.

### Ramp B — "Tell me why" (AI edge labels from local evidence)

Each edge shows a 1–2 sentence reason grounded in the citing passage.

- Needs: local bibliography extraction + in-text mention detection; mapping mentions to
  cited Works (resolution/merge, where stub Works may merge with provider Works); model
  labeling; `relationship_claim` with the exact supporting quote.
- Grounding rule: the quote must exactly match the source. Labels carry
  `origin='ai'`, `review_state='unreviewed'`, and are user-editable/rejectable.
- Testable moment: edges carry plausible reasons that click through to the exact sentence.

### Ramp C — "Get me the ones I don't have" (acquisition)

A per-work "fetch" that pulls the open-access copy.

- Needs: identifier resolution → Unpaywall/OpenAlex OA location → download → hash verify →
  `document_version` → link to Work.
- State machine: `unresolved → identifier_found → link_found → downloaded → mapped`, else
  `unavailable{reason, checked_at}`; resumable, retries skip completed stages.
- Agent is deterministic-first; model fallback only for hard cases.
- Testable moment: fetch a missing cited work and read it locally.

### Ramp D — "Show me only what matters" (scale)

Make a 100–300-work fanout navigable.

- See the relevance/tagging model below.
- Default view: top-N by prominence, remainder clustered/collapsed.
- Testable moment: a large fanout is navigable instead of a hairball.

### Ramp E — "Walk backward" (reading ramps)

Click a cited work → its own fanout → accrete a multi-hop reading ramp.

- Needs: iterative expansion, cached neighborhoods, reading lists/ramps.
- Testable moment: build a multi-hop reading path from one seed paper.

## Relevance, marks, and tags (Ramp D, designed early)

Separate three things that are often conflated:

1. **Signals** (computed, per Work): neighborhood degree, global cited-by count, recency,
   local availability, mention count, and citation section (background vs method). Stored
   as a derived `work_prominence` cache, not canonical.
2. **Marks** (explicit, per workspace, one value): `pinned` | `downplayed` | `hidden`.
   This is the downplay/hide control.
3. **Tags** (explicit, per workspace, many): named + colored, e.g. `foundational`,
   `methods`, `read`, `ignore`.

Private workspace state:

- `workspace.tag(id, name, color, kind)`
- `workspace.work_tag(workspace_id, work_id, tag_id)`
- `workspace.work_mark(workspace_id, work_id, mark, note)`

UI semantics: **downplay** dims/collapses into a cluster; **hide** filters out; **pin**
forces visible; tags drive filters and the legend. Ranking is `prominence` modulated by
marks. Keeping marks and tags separate avoids conflating "unimportant" with "not wanted".

## Slice scope for Ramp A

**In**

- Cargo workspace skeleton; `sqlx` + `sqlx::migrate!`; system Postgres, one dev/test DB.
- Slice schema under `source`/`graph`/`reader`: `source_record`, `bibliography_entry`,
  `citation_mention`, `work`, `work_identifier`, `work_version`, `field_claim`,
  `entry_resolution`, `citation_assertion`, `resolution_decision`, `document`,
  `document_version`, and a single implicit local `workspace`.
- Scanner → `document`/`document_version` (hash-keyed, move-tolerant).
- Provider ingest of outbound references from a known identifier.
- Canonical edge assembly grouped from assertions.
- API: `/api/works/{id}`, `/api/works/{id}/citations` (both directions),
  `/api/graph/neighbors/{id}`.
- Graph surface in the frontend, centered on the current paper's Work.

**Out**

- AI labels (Ramp B), acquisition (Ramp C), relevance/marks/tags (Ramp D), multi-hop
  ramps (Ramp E).
- Real workspace membership/upload/auth (single implicit local workspace for now).
- Blob store beyond the local artifact cache.
- Fuzzy/ML resolution scoring (exact-key and identifier matching only until Ramp B).

## Open decisions

1. Ramp order: A→B→C→D→E, or pull labels (B) ahead of provider topology (A)?
2. Edge provenance for Ramp A: provider-only (recommended), or local-only from the start?
3. Ramp B mention mapping: full local bibliography extraction now, or a lighter
   author-year/title string heuristic first?
4. Tags: private workspace-only to start (recommended), or design public/curated now?
5. Acquisition agent: deterministic-first (recommended), or model-driven?
