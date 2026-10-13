# Literature viewer: status and test guide

Status: 2026-10-13. Branch `integration/literature-viewer` (off `main` at `9c886a1`). Not pushed.

All five ramps from [the plan](literature-viewer-plan.md) are built, reviewed, and merged into this
branch. Each ramp was planned, implemented in units, independently reviewed, smoke-tested live against
the dev database, and merged with `--no-ff`:

| Ramp | Merge | Capability |
| --- | --- | --- |
| A | `b2ad011` | `lysilogy lit` links a paper to a Work and ingests its OpenAlex references; `L` opens the citation graph |
| B | `eae14f6` | Local bibliography + in-text mentions; AI edge labels (claude/codex) grounded by exact quote |
| C | `528297e` | Open-access acquisition of cited Works (`fetch`), resumable state machine |
| D | `a30611c` | Prominence ranking, top-N view with clusters, pin/downplay/hide, private tags |
| E | `af7ad57` | Multi-hop expansion in place, persistent reading ramps |

Gate on the merged branch: `cargo test --workspace` 782 passed / 0 failed with DB tests enabled;
`cargo clippy --workspace --all-targets -- -D warnings` clean; web `typecheck`, `lint`, `build` pass.

Per-ramp design, decisions, smoke evidence and known gaps: `docs/ramps/ramp-{a..e}.md` (see each
file's **Report** section).

## Setup

From the repository root (paths like `local-articles/` and `.lysilogy/` are cwd-relative defaults):

```sh
scripts/db start
eval "$(scripts/db env)"          # exports LYSILOGY_DATABASE_URL (dev) and the test URL
cargo build --release
(cd web && npm run build)
./target/release/lysilogy serve   # http://127.0.0.1:7319
```

Migrations apply automatically on connect. Optional:

- `LYSILOGY_CONTACT_EMAIL=<you>` enables Unpaywall in `fetch` and the OpenAlex polite pool. Without
  it, both still work (Unpaywall is skipped with `unpaywall_skipped_no_email`).
- `--sidecar claude|codex` requires the respective CLI to be logged in.

The dev DB already holds data from the live smoke runs (see "State left behind" below), so several of
the papers named here are already ingested; reruns are idempotent and mostly cached.

## Test script

Each step builds on the previous one. Keys apply inside the reader with the graph overlay open.

### Ramp A: what does this paper cite?

```sh
./target/release/lysilogy lit "China Syndrome"      # expect W2097916756, 114 references
./target/release/lysilogy lit "Fine-Tuning Language" # Ziegler 2019, 45 references
```

In the browser: open "Fine-Tuning Language Models from Human Preferences", press `L`.

- Fanout of cited Works with titles/years; filled dots are papers you hold locally.
- Click a dot (or `j`/`k` then `Enter`) for the Work panel: local copy, local citing papers, open button.
- On a paper with no Work yet, `L` offers `r` to run the lit flow in place.
- `Esc` closes the panel, then the overlay.

### Ramp B: why does it cite them?

```sh
./target/release/lysilogy lit "Deep Reinforcement Learning from Human Preferences" \
  --labels --sidecar claude --label-limit 10
```

Expect a summary with entries extracted / resolved / mentions / labeled / rejected-for-grounding.
A rerun makes 0 model calls. Try `--sidecar codex` too.

In the graph, select a labeled node: the "Why it is cited" card shows relation, reason and quote.
`g` jumps to the exact sentence in the citing PDF; `a` accept, `x` reject, `e` edit.

### Ramp C: get the ones I don't have

```sh
./target/release/lysilogy fetch --cited-by "Fine-Tuning Language" --limit 4
./target/release/lysilogy fetch arxiv:1906.03671    # rerun: already_mapped, no network
```

In the graph, select a hollow (non-local) node and press `f`. Progress shows, then "Fetched." with
**Open in reader**. Closed-access Works end as Unavailable with a reason; `f` retries.
Fetched PDFs land in `local-articles/Fetched/`.

### Ramp D: show me only what matters

Open a large fanout (e.g. "On the computational complexity of ethics", ~330 references) and press `L`.

- Expect "Showing 40 of N"; the rest are cluster pills (by decade, tag, mark).
- `Enter` on a cluster expands it, `c` collapses; `+`/`-` change top-N.
- In a Work panel: `p` pin, `d` downplay, `h` hide; `t`, type a name, `Enter` to create/assign a tag.
- Legend: `1`-`9` filter by tag, `0` clears.
- `lysilogy prominence --all [--fetch-metrics]` recomputes scores.

### Ramp E: walk backward

In the graph: select a cited Work, `w` expands its references in place (again on a child for hop 2).
`y` adds the node (and the path to it) to a reading ramp seeded at the current paper. `R` opens the
ramp sidebar: cycle status, add notes, reorder, **Suggest order**, Open/Fetch per item.

```sh
./target/release/lysilogy expand --paper "Deep Reinforcement Learning from Human Preferences" --depth 2 --fanout 3
./target/release/lysilogy ramp list
./target/release/lysilogy ramp show <title prefix>
```

## Known issues, by priority

Fix before merging to `main`:

1. **Bibliography title parsing** (`crates/lysilogy-ingest/src/bibliography/fields.rs:78-88`). An
   author list ending in "et al." is not treated as the author/title boundary, so the venue tail
   becomes the title (~6% of entries; e.g. TensorFlow parsed as "arXiv preprint arXiv:1603.04467,
   2016"). These become junk stub Works, and Ramp E's expansion now spreads them into walks and ramps.
2. **Identifier insert race** (`crates/lysilogy-db/src/work.rs` `add_identifier`). Parallel `lit` jobs
   resolving to the same new Work can hit a unique violation; `ON CONFLICT` covers only the partial
   active index.
3. **`/api/import` regression** (`src/remote.rs` `download_public_resource`). It now rejects PDFs
   served with an unusual Content-Type instead of sniffing `%PDF-`; also affects acquisition.
4. **Expansion reference count** (`src/expansion.rs` `expand_locally`). Counts only OpenAlex
   references, so a paper can show "expanded · 0" while 34 local edges exist.

Worth fixing soon:

- Exact-title-only matching leaves visible duplicate nodes ("Picbreeder" vs its full printed title).
  Fuzzy matching was deliberately deferred.
- A paper linked with 0 references has no way to re-run lit from the UI.
- When the identifier and title paths disagree, a paper can keep a separate identifier Work and never
  converge with the provider Work (`crates/lysilogy-ingest/src/literature.rs`).
- Labels: edges with two entries resolving to the same Work can get two AI claims; claims on
  withdrawn assertions still display.
- `fetch --cited-by` aborts the batch on the first hard error; `--limit` silently clamps to 1..=100.
- Ramp ordering breaks same-depth ties by prominence before year, which can look un-foundational.
- Prominence scores are untuned; cached cited-by counts never refresh.

Cosmetic / UI gaps: label overlap on dense rings; no UI for tag rename/recolor/delete or ramp
delete; graph expansion state is per overlay session.

## State left behind by the live smoke runs

- `local-articles/Fetched/`: 9 PDFs downloaded by Ramp C's smoke (listed in `docs/ramps/ramp-c.md`),
  with blobs under `.lysilogy/blobs/`. Keep or delete.
- Dev DB `lysilogy`: ingested Works, citation edges, local evidence, stub Works, and ~68 AI labels from
  the smokes. All curation rows (tags, marks, ramps) were removed after testing.
- Scratch DB `lysilogy_review_c` and `~/.cache/lysilogy-*` review scripts/screenshots. Remove with
  `scripts/db dropdb lysilogy_review_c` and `rm -rf ~/.cache/lysilogy-ramp-c-review*` when done.
- The Playwright scratch scripts under `~/.cache/lysilogy-smoke/` and `~/.cache/lysilogy-review*/`
  are the UI checks (`npm run smoke` is still broken locally: missing fixture).

## Not done

- Person/authorship resolution, fuzzy title matching, real workspaces/auth, evaluation harness.
- Acquisition beyond open access (Crossref links, Semantic Scholar, EPUB/HTML).
- Public/curated tags (tags are private to the implicit local workspace).
