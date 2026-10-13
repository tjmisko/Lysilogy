# Literature histogram view: a paper's references by publication year

Status: planned 2026-10-13. Branch `feat/lit-histogram-view`, based on
`integration/literature-viewer` at `14d8d19` (Ramps A–E merged). Frontend only: no migrations,
no backend or schema changes. A parallel track, `feat/lit-provenance`, owns migration 0009 and
every backend change; the two branches meet only at the cross-track contract below.

Related: [literature-viewer-status.md](literature-viewer-status.md),
[ramps/ramp-d.md](ramps/ramp-d.md) (ranking, marks, clusters), [ramps/ramp-e.md](ramps/ramp-e.md)
(walk, ramps, `L` overlay keys).

## What exists (verified at 14d8d19)

- `web/src/components/LiteratureGraph.tsx` (1357 lines): the `L` overlay. A single window
  `keydown` handler (lines 778–867) owns every graph key: `r` (run lit), `w` (walk), `y` (add to
  ramp), `R` (ramp sidebar); with the Work panel open `p`/`d`/`h` (marks), `t`, `f`, `a`/`x`/`e`/`g`;
  otherwise `0`–`9` (tag filter), `j`/`k`/`h`/`l` and arrows (cycle through placements in rank
  order), `Enter`/`o`, `c`, `+`/`=`/`-`. `Esc`/`q` arrive through App's close chain
  (`LITERATURE_QUIT_EVENT`, closes the panel, then the ramp sidebar, then the overlay). App also
  captures `T`, `E` and `/` globally. **Free key chosen for the toggle: `v`.**
- Selection is `selectedId` (a placement id, Work or cluster); `activeWork` is derived from the
  *radial* placements (`allPlacements[activeIndex]`), and `panelWorkGone` closes the panel when
  `selectedId` leaves the radial placements. `activateIndex` opens the Work panel (clears editing
  and error state, sets `citations` to loading). `WorkPanel` is a local component in the same file.
- Neighbors come from `GET /api/graph/neighbors/{id}?limit&expand&tags`. The server
  (`src/relevance.rs::rank_neighborhood`) sorts by mark class (pinned 0, none 1, downplayed 2,
  hidden 3), score desc, year desc, title key, id. With a `limit`, pinned + unmarked fill the top N;
  everything else becomes clusters; downplayed and hidden Works are *always* clustered
  (`downplayed`, `hidden`) unless their key is in `expand`. `limit` max is 500 (`LIMIT_MAX`).
  Valid years are `1..=2200` (`VALID_YEARS`); others count as "Year unknown".
- The radial view fetches `limit = topN` (default 40). The legend (`LiteratureLegend.tsx`)
  shows the ranking line, a "Hidden (n) — show" chip, chips for expanded clusters, key hints, and
  tag filter chips. Downplayed and decade/tag clusters are pills on the radial canvas.
- `web/src/components/PaperPreview.tsx` renders a lazy page-1 thumbnail of a held paper:
  `<PaperPreview url={api.source(paperId)} title={…} />` (pdf.js via `lib/pdfPreview.ts`,
  2 concurrent renders, LRU of 80 images). `HomePage.css` gives `.paper-preview` a 4:3 box; we
  override it locally. This is the pattern to reuse (SourceMap's canvas renderer is for full pages).
- Web tests are `node --experimental-strip-types --test web/scripts/<name>.test.mjs` importing
  `../src/lib/<name>.ts` directly (see `literature-layout.test.mjs`, `walk.test.mjs`), one
  `test:<name>` script per file in `web/package.json`. **No vitest; none is added.** Modules under
  test may import only *types* from other files (type imports are erased by strip-types).
- UI smoke checks are scratch Playwright scripts under `~/.cache/lysilogy-smoke/` that serve
  `web/dist` and mock `/api/*` (see `relevance-smoke.mjs`, `ramp-smoke.mjs`); `npm run smoke` is
  broken locally (missing fixture).

## Cross-track contract

- Each `Work` in `GraphNeighbors.nodes` gains an optional `display_authors: string[] | null`
  (printed author strings, in order). This branch adds exactly this line to `web/src/types.ts`
  `Work`, immediately after `year_state: FieldState;`, with no comment:

  ```ts
    display_authors?: string[] | null;
  ```

  The histogram renders an author line when the array has at least one non-blank string and
  renders nothing otherwise. Until the provenance branch merges, the server never sends the field,
  so every card shows title only; that is expected.
- New files only for the view (`LiteratureHistogram.tsx`, `LiteratureHistogram.css`,
  `lib/histogramLayout.ts`, `scripts/histogram-layout.test.mjs`). Edits to shared files
  (`LiteratureGraph.tsx`, `LiteratureLegend.tsx`, `types.ts`, `HelpOverlay.tsx`,
  `package.json`) are small and localized. `LiteratureGraph.css` is not edited (all histogram
  styles live in `LiteratureHistogram.css`). `WorkPanel` is not edited (the provenance track mounts
  its component there). `docs/literature-viewer-status.md` is not edited on this branch (the
  merge updates it).

## Decisions taken

User decisions (verbatim in substance):

1. **View toggle, radial kept.** The histogram is a second view of the same overlay. Key `v`
   toggles radial ↔ histogram, shown in the legend's key hints and the help overlay.
2. **Layout.** x axis = publication year with year gridlines and labels; each year's references
   stack vertically as one histogram column. The seed (current) paper's card sits **above** the
   histogram. References with unknown year go in a "Year unknown" lane at the far left.
3. **Cards with one expanded.** Every reference is a card. Exactly one card is fully visible, the
   focused one: thumbnail space (page-1 thumbnail when the Work has a held local paper, otherwise
   a compact placeholder), title, small author list. Every other card collapses to title +
   authors only. Authors come from optional `display_authors`; the line is omitted when absent.
4. **Cursor.** `h`/`l` and `←`/`→` move between year columns; `j`/`k` and `↓`/`↑` move within a
   column; mouse hover also moves the cursor. Moving the cursor pulls that Work's context into the
   existing Work panel (same panel, same selection state as the radial view).
5. **Seed link only for the focused card.** One curve from the seed card to the focused card; no
   other edges are drawn.
6. **Ordering within a column:** highest prominence first, stable tie-break by title then id;
   hidden/downplayed marks are respected the same way the radial view respects them.
7. **Walk and ramps stay radial-only.** In the histogram, `w`, `y` and `R` are inert and show the
   hint "Expansion and reading ramps live in the radial view — press v to switch." (choice: inert
   with a hint, never an automatic view switch, so a stray key never moves the user).
8. **Large neighborhoods stay usable** (225+ refs, 90+ in a decade): one column per year, the
   canvas scrolls in both directions, the focused card is kept in view.

Plan decisions:

9. **Data.** The histogram fetches its own payload, `graphNeighbors(center, "references",
   { limit: HISTOGRAM_LIMIT = 500, expand, tags })`, using the *same* `expanded` cluster set and
   `tagFilter` as the radial view, refetched on the same `neighborsToken` (marks, tags, fetches).
   It is stored in `histogramData` and does not replace the radial `graph.neighbors`, so switching
   back never flashes a 500-node radial layout and walk state survives. The fetch runs only while
   the histogram view is active; the stored payload is merged into the panel lookups only while the
   view is active (a stale payload must never override fresher radial marks).
10. **Marks.** Exactly like the radial server ranking: pinned and unmarked Works are shown;
    downplayed and hidden Works are shown only when their cluster key is expanded. The histogram
    renders every *collapsed* cluster from the payload except `hidden` (the legend already offers
    "Hidden (n) — show") as a chip in the seed band ("Downplayed · 4 — show"); clicking it calls
    the same `toggleExpansion(key)`. With > 500 references, the overflow arrives as decade/tag
    clusters and gets the same chips. Within a column, order is mark rank (pinned 0, none 1,
    downplayed 2, hidden 3), then score desc (missing or NaN = 0), then title key asc (trimmed,
    lower-cased; untitled after titled), then id asc. Downplayed cards are dimmed, hidden cards
    dimmed with a dashed border, pinned cards carry a pin ring; marks use `markFor` (optimistic
    overrides included), so pinning reorders the column at once.
11. **Year axis.** Valid years are integers in `1..=2200` (server `VALID_YEARS`); anything else
    (null, NaN, non-integer, out of range) is "Year unknown". The axis runs from the earliest to
    the latest occupied year. Occupied years are full card-width columns labelled
    "`<year>` · `<count>`". A run of 1–3 empty years is drawn year by year as narrow 14 px
    gridline slots (no label, `title` = the year); a run of 4+ empty years collapses to one 40 px
    gap slot labelled "`<from>`–`<to>`". The "Year unknown" lane (only when non-empty) is the
    leftmost column, followed by a 28 px separation. Navigation visits occupied columns only, so
    empty years are skipped.
12. **Orientation.** The year axis sits at the top of the scroll area (sticky), directly under the
    seed band; columns hang downward from it, row 0 (most prominent) nearest the axis. `j`/`↓` moves
    to the next less prominent card. (Assumption 1: the user may prefer bars growing upward.)
13. **Cursor model.** The cursor is a Work id (the shared `selectedId`), never an index, so it
    survives refreshes and reorders. Navigation clamps at every edge (no wrap). Moving to another
    column keeps the row index, clamped to that column's last row ("nearest row": all collapsed
    cards share one height and the focused card's top is at the same y as that row in every
    column). If the focused Work disappears (hidden, filtered), focus goes to the same column (by
    key) at the clamped row; if the column is gone, the column at the clamped old index; if nothing
    is remembered, the most prominent Work overall (best entry by the ordering above among every
    column's row 0, ties to the leftmost). Entering the histogram keeps the current selection if it
    is a Work in the grid.
14. **Panel follows the cursor.** Entering the histogram (`v`) opens the Work panel for the focused
    card. While the panel is open, every cursor move (keys or hover) retargets it (same reset as
    `activateIndex`: editing off, errors cleared, citations loading). `Esc` closes the panel; it
    stays closed until `Enter`/`o`/click. Hover never opens the panel (it would resize the canvas
    under the mouse). In the histogram, `panelWorkGone` does not apply: focus reconciliation moves
    the selection instead and the panel follows.
15. **Key precedence in the histogram.** `h`/`j`/`k`/`l`, the arrows, `Enter` and `o` are handled
    by the histogram's own listener and skipped by `LiteratureGraph`'s handler **before** its panel
    branch, so `h` moves left even with the panel open. Hide stays available through the panel's
    "Hide" button (`p` and `d` keep working). `c`, `0`–`9`, `t`, `f`, `a`/`x`/`e`/`g`, `p`/`d` work
    as in the radial view. `+`/`-` are inert with the hint "The year view shows every reference;
    top-N applies to the radial view." `w`, `y`, `R` per Decision 7. Switching to the histogram
    closes the ramp sidebar.
16. **Hover** moves the cursor on `pointermove` over a card (not `mouseenter`), so the layout shift
    of a newly expanded card under a stationary pointer never steals focus. Hover never scrolls.
17. **Keep in view.** After a keyboard move or a reconciliation, the scroll container scrolls the
    minimum needed to reveal the focused (expanded) card with a 24 px side margin, a top margin that
    clears the sticky axis, and a 16 px bottom margin; a card taller than the viewport aligns its
    top. `behavior: "smooth"` unless `prefers-reduced-motion: reduce`.
18. **Seed link geometry.** The seed band is outside the scroll container (always visible). An
    overlay SVG covering the histogram section draws one cubic curve from the seed card's
    bottom-centre to the focused card's top-centre in section coordinates (card position minus
    scroll offset). The target y is clamped into the scroll viewport; when the focused card is
    scrolled fully out of view horizontally, no link is drawn.
19. **Thumbnail.** Only the focused card renders a thumbnail, and only when it has a held local
    paper (`local_papers[workId][0]`): `<PaperPreview url={api.source(paperId)} title={…}/>` in a
    112 px high box. Unheld focused cards show a 30 px placeholder strip ("No local copy", or the
    acquisition status text when a brief exists). The expanded height therefore depends on "held"
    and is known to the pure layout. The seed card shows no thumbnail.
20. **View persistence.** The view is remembered in `localStorage["lysilogy.literature.view"]`
    (`"radial"` | `"histogram"`, try/catch around storage, anything else → radial). The root gets
    `data-view="radial|histogram"`.
21. **Card DOM.** HTML, absolutely positioned from the pure layout (not SVG), inside a scroll
    container. Each column is `role="list"` (`aria-label="1998, 4 references"`), each card
    `role="listitem"` with `data-work-id`, `data-focused`, `data-expanded`, `data-mark`,
    `data-local`, and `aria-current="true"` when focused. Click focuses and opens the panel.
    Keyboard handling is global (DOM focus is not moved), like the radial view.

## Design

### Modules

```
web/src/lib/histogramLayout.ts         (new, pure)  ordering, grid, navigation, reconciliation,
                                                    geometry, scroll/link helpers, author line,
                                                    view parsing
web/scripts/histogram-layout.test.mjs  (new)        node:test suite
web/src/components/LiteratureHistogram.tsx (new)    the view (seed band, cluster chips, scroll
                                                    area with sticky axis, gridlines, cards,
                                                    seed link overlay, key + hover handling)
web/src/components/LiteratureHistogram.css (new)
web/src/components/LiteratureGraph.tsx  (edit)      view state + `v`, histogram fetch, merged
                                                    lookups, view-aware activeWork, focus/open
                                                    helpers, key precedence, inert keys, mount
web/src/components/LiteratureLegend.tsx (edit)      optional `view` prop: key hints per view
web/src/components/HelpOverlay.tsx      (edit)      `v` and histogram keys
web/src/types.ts                        (edit)      Work.display_authors (contract)
web/package.json                        (edit)      "test:histogram-layout"
```

### `histogramLayout.ts` API (pure; imports types only)

```ts
import type { GraphNeighbors, WorkMarkName } from "../types";

export type LiteratureView = "radial" | "histogram";
export const LITERATURE_VIEW_STORAGE_KEY = "lysilogy.literature.view";
/** "histogram" → "histogram"; anything else (null, "", "Histogram", "radial") → "radial". */
export function parseLiteratureView(stored: string | null | undefined): LiteratureView;

export const HISTOGRAM_LIMIT = 500;            // server LIMIT_MAX
export const HISTOGRAM_YEAR_MIN = 1;           // server VALID_YEARS
export const HISTOGRAM_YEAR_MAX = 2200;
export const UNKNOWN_COLUMN_KEY = "unknown";   // year columns use `year:<n>`
export const EMPTY_RUN_MAX = 3;                // longer empty runs collapse to one gap slot
export const COLLAPSED_AUTHOR_LIMIT = 3;
export const EXPANDED_AUTHOR_LIMIT = 8;
export const HISTOGRAM_GEOMETRY = {
  paddingX: 16, axisHeight: 30, cardTop: 38, paddingBottom: 24,
  cardWidth: 176, columnGap: 10, unknownLaneGap: 28, emptyYearWidth: 14, gapSlotWidth: 40,
  collapsedHeight: 46, expandedHeldHeight: 236, expandedUnheldHeight: 150, rowGap: 6,
} as const;

export type MarkRank = 0 | 1 | 2 | 3;
export type HistogramEntry = { id: string; year: number | null; score: number; markRank: MarkRank; titleKey: string };

export function histogramYear(year: number | null | undefined): number | null; // integer in range, else null
export function markRank(mark: WorkMarkName | null | undefined): MarkRank;     // pinned 0, none 1, downplayed 2, hidden 3
export function titleKey(title: string | null | undefined): string;             // trim + toLowerCase
/** Mark rank, score desc, titled before untitled, title key asc, id asc. Never returns 0 for distinct ids. */
export function compareHistogramEntries(left: HistogramEntry, right: HistogramEntry): number;
/** Entries for every node except the center; duplicate ids keep the first; missing/NaN score → 0. */
export function histogramEntries(
  neighbors: Pick<GraphNeighbors, "center" | "nodes">,
  markOf: (workId: string) => WorkMarkName | null,
  scoreOf: (workId: string) => number | undefined,
): HistogramEntry[];

export type HistogramColumn = { key: string; year: number | null; entries: readonly HistogramEntry[] };
export type HistogramSlot =
  | { kind: "column"; columnIndex: number }
  | { kind: "empty_year"; year: number }
  | { kind: "gap"; fromYear: number; toYear: number };
export type HistogramGrid = { columns: readonly HistogramColumn[]; slots: readonly HistogramSlot[] };
/** Unknown lane first (only if non-empty), then occupied years ascending; entries sorted. */
export function buildHistogramGrid(entries: readonly HistogramEntry[]): HistogramGrid;

export type HistogramPosition = { columnIndex: number; row: number };
export function positionOf(grid: HistogramGrid, workId: string | null): HistogramPosition | null;
export function workAt(grid: HistogramGrid, position: HistogramPosition): string | null; // exact; null if out of range
export function initialCursor(grid: HistogramGrid): string | null;

export type HistogramDirection = "left" | "right" | "up" | "down";
/** h/ArrowLeft → left, l/ArrowRight → right, k/ArrowUp → up, j/ArrowDown → down, else null. */
export function directionForKey(key: string): HistogramDirection | null;
/** The eight direction keys plus "Enter" and "o". */
export const HISTOGRAM_NAV_KEYS: ReadonlySet<string>;
/** Clamped move; null/absent workId → initialCursor; empty grid → null. */
export function moveCursor(grid: HistogramGrid, workId: string | null, direction: HistogramDirection): string | null;

export type CursorMemory = { workId: string; columnKey: string; position: HistogramPosition };
export function rememberCursor(grid: HistogramGrid, workId: string | null): CursorMemory | null;
/** Decision 13: same Work → same column key at clamped row → clamped column index → initialCursor. */
export function reconcileCursor(grid: HistogramGrid, memory: CursorMemory | null): string | null;

export type HistogramCard = {
  id: string; columnIndex: number; row: number;
  x: number; y: number; width: number; height: number; expanded: boolean;
};
export type HistogramAxisSlot = {
  kind: "column" | "unknown" | "empty_year" | "gap";
  x: number; width: number;
  label: string;          // "1998", "Year unknown", "" (empty_year), "1901–1949" (gap)
  title: string;          // full text for the `title` attribute, e.g. "1998 · 4 references", "1950"
  count: number;          // references in the slot (0 for empty_year/gap)
};
export type HistogramLayout = { cards: readonly HistogramCard[]; axis: readonly HistogramAxisSlot[]; width: number; height: number };
/** Exactly one card is expanded iff `focusedWorkId` is in the grid. */
export function layoutHistogram(
  grid: HistogramGrid, focusedWorkId: string | null, isHeld: (workId: string) => boolean,
): HistogramLayout;

export type Rect = { x: number; y: number; width: number; height: number };
export type ScrollViewport = { left: number; top: number; width: number; height: number };
export type RevealMargins = { x: number; top: number; bottom: number };
export const REVEAL_MARGINS: RevealMargins; // { x: 24, top: axisHeight + 8, bottom: 16 }
/** Minimal scroll so `rect` (content coordinates) is visible; tall rects align their top. Never negative. */
export function scrollToReveal(rect: Rect, viewport: ScrollViewport, margins?: RevealMargins): { left: number; top: number };

export type Point = { x: number; y: number };
/** Card top-centre in section coordinates, y clamped into the viewport box; null when x is outside it. */
export function seedLinkTarget(card: Rect, scroll: { left: number; top: number }, viewportBox: Rect): Point | null;
/** "M fx fy C fx my, tx my, tx ty" with my = (fy + ty) / 2, numbers rounded to 0.1. */
export function seedLinkPath(from: Point, to: Point): string;

/** Trimmed non-blank names; null when none; more than `limit` → first `limit` joined ", " + " +<rest>". */
export function authorLine(authors: readonly string[] | null | undefined, limit: number): string | null;
```

Geometry (all in content pixels):

- x starts at `paddingX`. Slot widths: unknown lane `cardWidth + unknownLaneGap`, year column
  `cardWidth + columnGap`, empty year `emptyYearWidth`, gap `gapSlotWidth`. A card's `x` is its
  slot's `x`; `width = cardWidth`. `width` of the layout = end of last slot + `paddingX`
  (minimum `2 * paddingX + cardWidth` for an empty grid).
- Rows: `y(row 0) = cardTop`; each next row starts after the previous card's height + `rowGap`.
  Collapsed height `collapsedHeight`; the focused card `expandedHeldHeight` when `isHeld`, else
  `expandedUnheldHeight`. Rows below the focused card in its column shift by the difference; other
  columns are unaffected.
- `height` = max column bottom + `paddingBottom` (minimum `cardTop + paddingBottom`).

### `LiteratureHistogram.tsx`

```ts
type LiteratureHistogramProps = {
  centerWork: Work;
  neighbors: GraphNeighbors | null;            // null while the first fetch runs
  error: string | null;
  focusedWorkId: string | null;                // LiteratureGraph's selectedId
  markFor: (workId: string) => MarkView | null;
  prominence: Readonly<Record<string, ProminenceView>>;
  localPapers: LocalPapersByWork;
  acquisitions: AcquisitionsByWork;
  expandedClusters: ReadonlySet<string>;
  onToggleCluster: (key: string) => void;
  onFocusWork: (workId: string) => void;       // move the cursor (panel follows if open)
  onActivateWork: (workId: string) => void;    // focus + open the Work panel
};
```

Structure (class names are the CSS contract and the smoke selectors):

```
section.literature-histogram[data-card-count][data-focused-id]
  div.literature-histogram-seed                 seed band (fixed height 84 px)
    article.literature-histogram-seed-card      "This paper" kicker, title (2-line clamp), authors, year
    div.literature-histogram-chips              collapsed cluster chips except "hidden"
  div.literature-histogram-scroll (ref)         overflow: auto; position: relative
    div.literature-histogram-content            width/height from layout; position: relative
      div.literature-histogram-axis             position: sticky; top: 0; height 30 px; z-index 2
        span.literature-histogram-tick[data-slot-kind][data-year]   per axis slot
      div.literature-histogram-gridline          one per axis slot (full height, behind cards)
      div.literature-histogram-column[role=list] one per column (positioned box for aria only)
      div.literature-histogram-card[role=listitem] one per card (absolute; left/top/width/height inline)
  svg.literature-histogram-link-layer            absolute overlay over the section; pointer-events: none
    path.literature-histogram-link               only when a target exists (Decision 5, 18)
  p.literature-status                           loading / error / "No references are known for this paper."
```

Collapsed card: title (2-line clamp, Georgia 11.5 px) and author line (1-line clamp, monospace
9.5 px, `COLLAPSED_AUTHOR_LIMIT`). Expanded card: thumbnail box (held: `PaperPreview`, 112 px;
unheld: 30 px placeholder), title (4-line clamp, 13 px), authors (2-line clamp,
`EXPANDED_AUTHOR_LIMIT`), meta line "`<year or Year unknown>` · `<kind>` · score `<0.00>`".
Held cards (any state) show a small accent dot (legend's "held locally"). Content must fit the
fixed heights; overflow is hidden.

### `LiteratureGraph.tsx` edits (all localized)

1. Imports: `LiteratureHistogram`, and from `histogramLayout`: `HISTOGRAM_LIMIT`,
   `HISTOGRAM_NAV_KEYS`, `LITERATURE_VIEW_STORAGE_KEY`, `parseLiteratureView`, `type LiteratureView`.
2. State: `graphView` (initialised from storage via `parseLiteratureView`, try/catch),
   `histogramData: GraphNeighbors | null`, `histogramError: string | null`.
3. Effect: fetch the histogram payload while `graphView === "histogram"` (deps `centerWork`,
   `expanded`, `neighborsToken`, `tagFilter`, `graphView`); abort on cleanup; keep the previous
   payload on screen while refetching; on error set `histogramError`.
4. `merged`: add `histogramData` to `payloads` before the base graph only when the view is
   `histogram`.
5. `activeWork`: rename the existing expression to `radialActiveWork`;
   `activeWork = graphView === "histogram" ? (selectedId === null ? undefined : worksById.get(selectedId)) : radialActiveWork`.
   Exclude the center (`centerId`) from histogram focus (it is never in the grid).
6. `panelWorkGone`: add `graphView === "radial" &&`.
7. Extract the Work branch of `activateIndex` into `openWorkPanel(workId)`; `activateIndex` calls
   it. Add `focusHistogramWork(workId)`: no-op if equal to `selectedId`; else `setSelectedId`, and
   when `workPanelOpen` also `setEditingId(null)`, clear label/mark/tag errors and
   `setCitations({ kind: "loading" })`. Add `activateHistogramWork(workId)` =
   `openWorkPanel(workId)`.
8. `toggleGraphView()`: flip, persist, clear `walkNotice`; into histogram: `setRampOpen(false)` and
   open the panel for the focused card once focus exists (U3: a `pendingOpen` ref consumed by the
   first `focusHistogramWork` call or immediately if `selectedId` is already a grid Work).
9. Key handler, right after `if (phase.kind !== "graph") return;`:
   - `v` → `toggleGraphView()`.
   - histogram view and `HISTOGRAM_NAV_KEYS.has(event.key)` → `return` (no `preventDefault`).
   - histogram view and `w`/`y`/`R`/`+`/`=`/`-` → `preventDefault`, set the hint, `return`.
10. Root `data-view={graphView}`. Inside `section.literature-canvas`, wrap the existing `<svg>`
    in `{graphView === "radial" && (` … `)}` **without re-indenting it**, followed by
    `{graphView === "histogram" && <LiteratureHistogram …/>}`. The walk notice and legend stay.
11. Legend: in histogram view pass `graph={histogramData ?? graph.neighbors}`,
    `placementCount={histogram card count}`, and `view={graphView}`.

### Legend and help text

- Radial hints (existing line) gain `· <kbd>v</kbd> year view`.
- Histogram hints: `<kbd>h</kbd>/<kbd>l</kbd> year · <kbd>j</kbd>/<kbd>k</kbd> within year ·
  <kbd>↵</kbd> open · <kbd>c</kbd> collapse · <kbd>v</kbd> radial view · <kbd>Esc</kbd> close`.
- Help overlay rows: `["v (graph)", "Switch between the radial graph and the year histogram"]`,
  `["h / l · j / k (year view)", "Move between years / within a year; hovering a card also moves; the open Work panel follows"]`,
  and append "(radial view)" to the descriptions of the `w`, `y`, `R` and `+ / -` rows, plus
  "(in the year view, h moves; use the Hide button)" to the `p / d / h` row.

## Gate (every unit)

```sh
eval "$(~/Projects/Lysilogy/scripts/db env)"     # scripts/db start if scripts/db status fails
cargo test --workspace                           # unchanged backend; DB tests must run
cargo clippy --workspace --all-targets -- -D warnings
cd web && npm run typecheck && npm run lint && npm run build \
  && npm run test:histogram-layout && npm run test:literature-layout && npm run test:walk \
  && npm run test:ramp && npm run test:relevance
```

(`test:histogram-layout` exists from U1 on.) Default cargo target dir inside the worktree; no
caches in `/tmp`; scratch under `~/.cache/lysilogy-smoke/` or `$CLAUDE_JOB_DIR/tmp`. Tests are
named `should <behavior> when <condition>`. Commits are atomic conventional commits ending with
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; no push, merge or PR.

## Implementation units

### U1. Pure histogram logic, tests, and the contract field

Files (owned): `web/src/lib/histogramLayout.ts` (new), `web/scripts/histogram-layout.test.mjs`
(new), `web/package.json` (add `"test:histogram-layout": "node --experimental-strip-types --test
scripts/histogram-layout.test.mjs"` after `test:literature-layout`), `web/src/types.ts` (the one
contract line).

Implement exactly the API in Design. Notes: `compareHistogramEntries` compares `markRank`, then
`score` desc (`b.score - a.score` after NaN → 0 in `histogramEntries`), then untitled (`""`)
after titled, then `titleKey` with `<`/`>` (not `localeCompare`, for determinism), then id with
`<`/`>`. `buildHistogramGrid` dedups by id (first wins), groups by `histogramYear`, sorts each
column, emits slots per Decision 11. `moveCursor` left/right go to `columnIndex ∓ 1` (every column
is non-empty) with `row = min(row, length - 1)`; at an edge return the same id. `layoutHistogram`
places cards per Geometry. `seedLinkTarget` uses `viewportBox` (the scroll viewport's box in
section coordinates). Pure; only `import type` from `../types`.

Tests (`histogram-layout.test.mjs`), fixtures built from small helper `entry(id, year, score,
{ mark, title })` and `gridOf(entries)`:

- `should exclude the center and duplicate ids when building entries`
- `should treat missing and NaN scores as zero when building entries`
- `should order pinned then unmarked then downplayed then hidden when sorting a column`
- `should order by score descending within a mark rank when sorting a column`
- `should break score ties by title then id when sorting a column` (case-insensitive title;
  untitled after titled; equal titles by id)
- `should produce the same columns when entries are shuffled` (several permutations)
- `should put unknown years in a leftmost lane when years are null zero fractional or out of range`
  (`null`, `0`, `NaN`, `1999.5`, `2201`, `-5` → `unknown`; `1` and `2200` are years; the lane is
  column 0 and its card `x` is less than every year card's `x` in `layoutHistogram`)
- `should omit the unknown lane when every year is known`
- `should draw short empty runs year by year and collapse long runs into one gap` (years 2000,
  2002, 2010 → slots column, empty 2001, column, gap 2003–2009, column)
- `should label occupied columns with year and count when building the axis`
- `should skip empty years when moving right and left` (2001 → 2005 → 2001 across 2002–2004)
- `should land on the nearest row when moving between columns` (row 5 → a 3-card column → row 2;
  row 1 → row 1; back to the tall column keeps row 2)
- `should never leave the grid when moving past any edge` (left at column 0, right at the last,
  up at row 0, down at the last row return the same id)
- `should stay inside the grid when making many random moves` (seeded LCG, 1,000 moves over a
  40-entry grid; every result is in the grid; never null)
- `should start at the most prominent work when the cursor is unset or unknown` (null id and a
  foreign id; tie between columns → leftmost)
- `should return null when moving in an empty grid`
- `should keep focus on the same work when data refreshes and reorders` (memory from grid A;
  grid B with changed scores, a new column inserted to the left, the Work moved to a different row
  → same id)
- `should fall back to the same column at the clamped row when the focused work disappears`
- `should fall back to the neighboring column when the focused column disappears`
- `should expand only the focused card when laying out` (exactly one `expanded`; height
  `expandedHeldHeight` vs `expandedUnheldHeight` by `isHeld`; focus null or foreign → zero
  expanded)
- `should move the expansion when focus changes` (two layouts, different single expanded card)
- `should shift only cards below the focused card in its column when expanding` (cards below
  shift by `expanded − collapsed`; other columns' y unchanged)
- `should map vim keys and arrows to directions and ignore other keys`
- `should scroll minimally to reveal a card when it is outside the viewport` (inside → unchanged;
  right/left/below/above; taller than viewport → top aligned; never negative)
- `should clamp the link target into the viewport and drop it when the card is off to the side`
- `should build a cubic path from seed to card when drawing the link`
- `should format authors with a limit and return null when none are printable` (`null`,
  `undefined`, `[]`, `["  "]` → null; 5 names limit 3 → `"A, B, C +2"`)
- `should parse only histogram as the histogram view when reading storage`

Acceptance: gate green; `npm run test:histogram-layout` lists every test above.

### U2. Histogram view: toggle, data, rendering, click focus

Files (owned): `web/src/components/LiteratureHistogram.tsx` (new),
`web/src/components/LiteratureHistogram.css` (new), `web/src/components/LiteratureGraph.tsx`
(Design edits 1–7, 8 without the auto-open, 9 the `v` line and the `HISTOGRAM_NAV_KEYS` skip so
radial cycling never runs against the histogram grid (the keys are simply inert until U3), 10, 11),
`web/src/components/LiteratureLegend.tsx` (optional `view?: LiteratureView` prop, default
`"radial"`; hint lines per "Legend and help text").

Scope:

- `LiteratureHistogram` builds `entries = histogramEntries(neighbors, id => markFor(id)?.mark ??
  null, id => prominence[id]?.score)`, `grid = buildHistogramGrid(entries)` and `layout =
  layoutHistogram(grid, effectiveFocus, isHeld)` in `useMemo`s, where `isHeld(id) =
  (localPapers[id]?.length ?? 0) > 0` and `effectiveFocus = positionOf(grid, focusedWorkId) ===
  null ? null : focusedWorkId`.
- Focus reconciliation effect: keep `memory = useRef<CursorMemory | null>`; after each render
  where `effectiveFocus !== null`, `memory.current = rememberCursor(grid, effectiveFocus)`. When
  `effectiveFocus === null` and the grid has columns, call
  `onFocusWork(reconcileCursor(grid, memory.current))` (skip when it returns null).
- Render the structure in Design (seed band, chips, sticky axis, gridlines, columns, cards,
  status lines). Cards: `onClick → onActivateWork(id)`. No keyboard, hover, scroll or link yet.
  The seed card uses `centerWork` (title via the same "Untitled (…)" fallback as `workTitle`;
  author line from `centerWork.display_authors`; year or "Year unknown").
- Cluster chips: `neighbors.clusters` minus key `hidden`, each
  `<button className="literature-chip" data-cluster-key=…>{label} · {count} — show</button>`
  calling `onToggleCluster(key)`. Expanded clusters already show in the legend.
- Expanded card thumbnail per Decision 19 (`PaperPreview`; placeholder uses
  `acquisitionStatusText` from `lib/acquisition` when `acquisitions[id]` exists, else
  "No local copy").
- CSS: section is a flex column filling `.literature-canvas` above the legend
  (`flex: 1; min-height: 0; position: relative`); scroll area `flex: 1; overflow: auto`;
  gridlines `var(--line)`; focused card `border-color: var(--signal)` and a raised shadow;
  `.literature-histogram-card .paper-preview` overrides HomePage's 4:3 box to `height: 112px;
  aspect-ratio: auto; padding: 4px`; `[data-mark="downplayed"]` opacity 0.55;
  `[data-mark="hidden"]` opacity 0.45 + dashed border; pinned ring via `box-shadow` inset accent;
  reduced-motion: no transitions. Use only existing CSS variables (`--panel`, `--panel-bright`,
  `--ink`, `--ink-soft`, `--ink-faint`, `--line`, `--line-bright`, `--signal`, `--signal-ink`).
- `LiteratureGraph`: Design edits as listed; `v` toggles (no auto-open of the panel yet).

Tests: no new pure logic beyond U1 (any helper added here goes into `histogramLayout.ts` with a
test). Mock Playwright `~/.cache/lysilogy-smoke/histogram-smoke.mjs` (first version; copy the
server/mocks pattern of `relevance-smoke.mjs`; 230 Works spread over 1955–2024 with 95 in the
2010s, 6 with `display_year: null`, 8 held, 3 pinned, 4 downplayed, 2 hidden, half with
`display_authors`): `v` sets `data-view="histogram"` and a neighbors request with `limit=500`
is made; card count = 230 − 6 (downplayed + hidden); the unknown lane is the leftmost column;
exactly one `[data-expanded="true"]`; a held focused card contains `.paper-preview`; clicking a
card opens the Work panel for it; the "Downplayed · 4 — show" chip adds `expand=downplayed` and
the 4 cards appear dimmed at the bottom of their columns; cards without authors have no author
line; `v` again shows the radial `svg` with the same top-40 as before; no console errors;
screenshot `histogram-u2.png`.

Acceptance: gate green; smoke passes; results noted in "U2 implementation notes" (append to this
file).

### U3. Interaction: keys, hover, panel follow, keep-in-view, seed link, inert keys; smoke; report

Files (owned): `web/src/components/LiteratureHistogram.tsx`, `LiteratureHistogram.css`,
`web/src/components/LiteratureGraph.tsx` (Design edits 7–9 completed: `focusHistogramWork`
retargeting, auto-open on entry, key precedence, inert keys),
`web/src/components/HelpOverlay.tsx`, `docs/lit-histogram-view-plan.md` (notes, result, report).

Scope:

- Histogram key listener (`useEffect`, `window` `keydown`, bubble phase), active only while
  mounted: ignore when `event.defaultPrevented`, an editable target (`input`, `textarea`,
  `select`), or any of meta/ctrl/alt. `directionForKey` → `preventDefault`,
  `next = moveCursor(grid, effectiveFocus, direction)`; if `next !== null && next !==
  effectiveFocus` set a `revealPending` ref and `onFocusWork(next)`. `Enter`/`o` → if a card is
  focused, `preventDefault` and `onActivateWork(focused)`.
- Hover: `onPointerMove` on a card → `if (id !== effectiveFocus) onFocusWork(id)` (no reveal).
- Reveal: a `useLayoutEffect` on `[effectiveFocus, layout]`: if `revealPending` (keyboard move or
  reconciliation) find the focused card rect, compute `scrollToReveal(rect, {left: scrollLeft,
  top: scrollTop, width: clientWidth, height: clientHeight})`, `scrollTo` (smooth unless reduced
  motion), clear the flag. Also reveal once when the view mounts with a focus.
- Seed link: track the scroll container's `scrollLeft/scrollTop` (passive `scroll` listener,
  coalesced with `requestAnimationFrame`) and its box and the seed card's bottom-centre relative
  to the section (`ResizeObserver` on the section, scroll container and seed card). Draw
  `<path className="literature-histogram-link" d={seedLinkPath(seedAnchor, target)} />` only
  when `seedLinkTarget` returns a point; nothing for any other card.
- `LiteratureGraph`: `focusHistogramWork` retargets the panel when open (Design 7). Entering the
  histogram opens the panel for the focused card (Design 8): if `selectedId` is a Work in the
  histogram payload, `openWorkPanel(selectedId)` right away; otherwise set `pendingPanelOpen`
  and open on the first `focusHistogramWork` call after the payload arrives. Key precedence and
  inert keys per Design 9 / Decisions 7 and 15 (hints through `setWalkNotice`; the notice clears
  on the next selection change as today). In the histogram, cards do not handle double-click.
- Help overlay rows per "Legend and help text".

Tests: no new pure helpers expected (any added go to `histogramLayout.ts` with tests). Extend
`~/.cache/lysilogy-smoke/histogram-smoke.mjs`:
- `l` ×3 / `h` ×1 move focus across occupied columns only (the focused card's `data-year`
  follows the occupied years list); `j` ×2 then `l` lands on row `min(2, len − 1)`.
- `h` at the leftmost column and `k` at row 0 keep focus; with the panel open `h` moves (no
  `PUT /mark`), and the panel title follows every move.
- Pressing `j` 30 times in the tallest column keeps the focused card fully inside the scroll
  container's client box (bounding boxes).
- Hovering a different card moves focus and the panel, without scrolling (`scrollTop`
  unchanged); exactly one `path.literature-histogram-link` exists and its end x is within 2 px of
  the focused card's centre.
- Pin the focused Work with `p`: it moves to row 0 of its column and stays focused. Hide it via
  the panel "Hide" button: after refetch, focus is on the same column at the clamped row and the
  panel shows that Work.
- `w`, `y`, `R`, `+` in histogram show the hint (`[data-walk-notice]`) and send no
  `POST …/expand`, no ramp request and no new neighbors request with a different limit.
- Esc closes the panel, a second Esc closes the overlay; reopening `L` restores the histogram
  view (storage).
- 390 px wide viewport: the histogram scrolls horizontally inside the overlay without page
  overflow.
- No console errors; screenshots `histogram-focus.png`, `histogram-narrow.png`.

Live check (record in "U3 implementation notes"): release build, `lysilogy serve --bind
127.0.0.1:7396 --web web/dist` on the dev DB; open "On the computational complexity of ethics"
(~330 references) and "Learning to summarize from human feedback" (215), press `L`, `v`;
record card count vs `ranking.total`, columns, the densest year, first-render time, and that
`j`/`l` stay smooth (no long tasks > 100 ms in a Playwright trace or a performance mark);
screenshot `live-histogram.png`. No writes to the DB except mark changes you revert.

Report: append "U3 implementation notes", "Result", and "Report" (what shipped, how to
user-test, smoke results, known gaps) to this file.

Acceptance: gate green; smoke passes in mock mode; live check recorded.

## Assumptions (to confirm with the user)

1. Columns hang downward from an axis under the seed band (row 0 = most prominent, nearest the
   axis) rather than growing upward from a bottom baseline. Flipping is a geometry change in
   `layoutHistogram` plus `j`/`k` semantics.
2. Pinned Works lead their column ahead of higher-scored unmarked ones, mirroring the server's
   radial ranking ("respect marks the same way the radial view does").
3. Downplayed and hidden references are not in the histogram until their cluster is expanded
   (chip or legend), exactly like the radial view; when shown, they sit after unmarked cards.
4. In the histogram, `h` always moves left, so hiding from the keyboard is unavailable there
   (the panel's Hide button remains). `p` and `d` still work.
5. Entering the histogram opens the Work panel for the focused card; afterwards the panel follows
   the cursor while open, and hover never opens it.
6. `w`, `y`, `R`, `+`/`-` are inert in the histogram with a hint (no automatic switch).
7. The histogram shows up to 500 references (server cap); overflow appears as decade/tag cluster
   chips that can be expanded (at most 32 expanded keys per request, a server limit).
8. The seed card is not part of the cursor grid and is not clickable.
9. View choice persists across overlays and papers via `localStorage`.
10. Until the provenance branch merges, no author lines appear (the server does not send
    `display_authors`); cards are title-only. Card heights are fixed whether or not authors exist.
11. Hover-driven panel retargeting can issue one `GET /api/works/{id}/citations` per hovered card
    (previous request aborted). No debounce in this branch; revisit if it shows up in the live
    check.
12. Switching back to radial with a focused Work outside the radial top-N leaves `selectedId` on it;
    the radial view then highlights placement 0 and `panelWorkGone` closes the panel (existing
    behavior, unchanged).
13. The `types.ts` contract line is placed after `year_state` with no comment; if the provenance
    branch places or documents it differently, the merge needs a one-line resolution.

## U1 implementation notes

- Shipped `web/src/lib/histogramLayout.ts` per the API section, `web/scripts/histogram-layout.test.mjs`
  (29 tests: the 26 required plus an empty-layout size test; two required names are covered by
  one test each), the `test:histogram-layout` script, and the `display_authors` contract line.
- Deviation: `seedLinkTarget` adds the viewport box origin (`viewportBox.x`/`y`) to the
  card-minus-scroll position, so the result is in section coordinates as Decision 18 requires
  (the scroll container sits below the seed band). The x test and y clamp use the same box.
  U3 passes the scroll viewport's box relative to the section.
- `tsconfig` has `noUncheckedIndexedAccess`; column lookups are guarded, with no behavior change.
- `scrollToReveal` also aligns the left edge when a rect is wider than the visible width.
- Gate observed: cargo test --workspace all pass (344/20/1/1/77/98/241 per suite, 0 failed);
  clippy clean; typecheck, lint, build clean; test:histogram-layout 29/29, literature-layout 31,
  walk 11, ramp 10, relevance 13, all passing.

## U2 implementation notes

- Shipped `LiteratureHistogram.tsx` and `.css` (seed band, cluster chips, sticky axis, gridlines,
  column list boxes, absolutely positioned cards, status lines), the `LiteratureGraph.tsx` edits
  (view state and persistence, histogram fetch effect, merged payloads, view-aware `activeWork`,
  `openWorkPanel`, `focusHistogramWork`, `activateHistogramWork`, `toggleGraphView`, `v` key and the
  `HISTOGRAM_NAV_KEYS` skip, `data-view`, mount), and the legend `view` prop with per-view hints.
  No new pure helper was needed.
- Deviation: the seed card shows the year on the kicker line ("This paper" left, year right) rather
  than a separate line, so title (2-line clamp), authors and year fit the fixed 84 px band.
- Deviation: the legend's radial hint line also gains `v year view` (as planned); the histogram
  card count passed to the legend is the number of distinct non-center nodes in `histogramData`.
- A cluster click in the histogram goes through `toggleExpansion`, so the seed-band chip and the
  legend's expanded chip both collapse it again. The radial view keeps the expansion set when
  switching back (so the radial view shows the expanded Works until `c`); this is the shared
  `expanded` state by design (Decision 9).
- `c` does nothing while the Work panel is open in the radial view (existing behavior); the smoke
  closes the panel first.
- Smoke `~/.cache/lysilogy-smoke/histogram-smoke.mjs` (mock mode, 230 Works): all checks pass, no
  console errors. It covers `v` toggling and `limit=500`, 224 cards, the unknown lane leftmost,
  95 Works in the 2010s (91 shown as cards, 4 marked ones clustered), exactly one expanded card, the
  held focused card's `.paper-preview` (112 px), authors present only on odd-numbered Works, click
  opening the Work panel, the "Downplayed · 4 — show" chip adding `expand=downplayed` with four
  dimmed cards at the bottom of their columns, return to radial with the identical top 40 after
  `c`, and storage persistence. Screenshots `histogram-u2.png`, `histogram-u2-expanded.png`.
- Gate observed: cargo test --workspace 782 passed, 0 failed; clippy clean; typecheck, lint, build
  clean; test:histogram-layout 29, literature-layout 31, walk 11, ramp 10, relevance 13, all pass.

## U3 implementation notes

- Shipped in `LiteratureHistogram.tsx`: a window `keydown` listener (bubble phase; ignores
  defaultPrevented, input/textarea/select targets and meta/ctrl/alt) that moves the cursor with
  `moveCursor` and activates with Enter/`o`; hover on `pointermove`; the reveal `useLayoutEffect`
  (`scrollToReveal`, smooth unless reduced motion); and a `SeedLink` child that draws the single
  `path.literature-histogram-link`. In `LiteratureGraph.tsx`: auto-open of the panel on entering
  the year view (`pendingPanelOpen` ref), inert `w`/`y`/`R` and `+`/`=`/`-` with the planned hints,
  and `focusHistogramWork` (already retargeting the panel from U2). `HelpOverlay.tsx` rows per plan.
  No new pure helper.
- Deviation 1: `SeedLink` is its own component that owns the scroll offset, the viewport box and
  the seed anchor (passive scroll listener coalesced by `requestAnimationFrame`, `ResizeObserver` on
  section, scroll container and seed card), so scrolling re-renders only the overlay, not 200+
  cards. It reads its refs in a passive `useEffect`: a parent's ref attaches after a child's layout
  effect runs, so a layout effect saw a null section.
- Deviation 2: `pendingPanelOpen` is consumed by an effect on `[graphView, histogramData,
  selectedId]` rather than inside `focusHistogramWork`, because a selection that is already a card
  never triggers a `focusHistogramWork` call. It opens once `selectedId` is a non-center node of the
  histogram payload (immediately when that is already true, otherwise after reconciliation moves
  focus).
- Deviation 3 (found by the narrow-viewport smoke): the overlay is a grid with an auto column, so
  the histogram's 13,000 px content made `.literature-body` as wide as the content and the scroll
  container never scrolled. `LiteratureHistogram.css` adds
  `.literature-graph[data-view="histogram"] .literature-body { min-width: 0 }` (no edit to
  `LiteratureGraph.css`).
- Deviation 4: a `ResizeObserver` on the scroll container re-reveals the focused card (instant, not
  smooth) when the container resizes. Opening the Work panel on entry narrows the container after
  the first reveal, and a smooth scroll still in flight otherwise ends on a stale target.
- Deviation 5: hover ignores `pointermove` events whose client coordinates match the previous one,
  so a scroll or a layout shift under a still pointer does not steal focus. Enter/`o` is not
  hijacked when the event target is a button or link (the panel's Hide button still works on Enter).
- Deviation 6: the U2 smoke asserted that `j` is inert in the histogram; that is now intentionally
  false. The U3 smoke is `~/.cache/lysilogy-smoke/histogram-u3-smoke.mjs` (separate file; the U2 one
  is kept for its rendering checks).
- Environment: the shared dev and test Postgres databases had migration 9 applied by the
  provenance track, so this branch's binary and `cargo test` failed with `VersionMissing(9)`. I used
  a private test database (`lysilogy_test_histogram`, same server) for the gate and a throwaway
  clone of the dev database with the version 9 row deleted for the live check (dropped afterwards).
  The shared databases were not modified. The live server used `--library` pointing at the main
  checkout's `local-articles` (read) with scratch `--data` and `--notes` directories.
- Live check (release build, `lysilogy serve --bind 127.0.0.1:7396 --web web/dist`, dev DB clone):
  - "On the computational complexity of ethics": 332 cards = `ranking.total` 332 (0 pinned, 0
    hidden), 68 columns (including the unknown lane), densest bucket "Year unknown" with 27, densest
    known year 2006 with 16, first render about 100-190 ms after `v`, 0 long tasks (>50 ms
    PerformanceObserver) over 75 `l`/`j`/`h` presses at 60 ms spacing, no console errors.
  - "Learning to summarize from human feedback": the dev DB holds 93 references, not the 215 the plan
    cites. 93 cards = `ranking.total` 93, 19 columns, densest 2019 with 21, first render 64-107 ms,
    0 long tasks, no console errors.
  - Screenshot `~/.cache/lysilogy-smoke/live-histogram.png`. No marks were changed.
- Gate observed: `cargo test --workspace` 344/20/1/1/77/98/241 passed, 0 failed (private test
  database); clippy clean; typecheck, lint, build clean; test:histogram-layout 29, literature-layout
  31, walk 11, ramp 10, relevance 13, all passing.

## Result

U3 done. The year view is fully interactive: keys, hover, panel follow, keep-in-view, one seed link,
inert radial-only keys with hints, and help text. Mock smoke (35 checks) and live check pass.

## Report

What shipped:
- `v` toggles radial and year views (persisted). Entering the year view opens the Work panel for the
  focused card. `h`/`l`/arrows move between occupied years, `j`/`k` within a year; hover moves the
  cursor too; the open panel follows every move. Enter/`o`/click opens the panel. Esc closes the
  panel, then the overlay.
- The focused card scrolls into view on keyboard moves (never on hover). One curve links the seed
  card to the focused card. `w`/`y`/`R` and `+`/`-` show a hint instead of acting.

How to user-test: open a paper with `L`, press `v`. Press `l` and `j` repeatedly across a dense
decade and watch the panel title and the curve follow. Hover another card (the panel retargets, no
scroll). Press `p` to pin (card jumps to the top of its year, focus stays), use the panel Hide
button (focus lands on the neighbour at the same row). Press `w` or `+` to see the hints. Narrow the
window to see horizontal scrolling inside the overlay. `?` shows the help rows.

Smoke results (`histogram-u3-smoke.mjs`, mock data, 230 Works): all 35 checks pass, no console
errors; screenshots `histogram-focus.png`, `histogram-narrow.png`, plus `live-histogram.png`.

Known gaps:
- Hovering sends one citations request per hovered card (no debounce), as flagged in Assumption 11.
- The seed curve draws over cards between the seed and the focused card (pointer-events none, so it
  never blocks input).
- The live "Learning to summarize" neighborhood has 93 references in this DB, so the 215-reference
  case was not exercised live; the 332-reference case was.
- Author lines stay empty until the provenance branch's `display_authors` is served.
- The U2 smoke's "j is inert" assertion is obsolete.

## Review fixes

1. Cited-paper context beyond top-N. `edgeByCited` in `LiteratureGraph.tsx` was built only from the
   radial fetch (limit = topN), and the server scopes edges to returned nodes, so year-view cards
   outside the top-N had no "Why it is cited" section and no `actionableLabel`. It now merges
   `histogramData.edges` (citing = center) while the histogram view is active, with the radial edge
   winning on a clash. The merge is the pure helper `citedEdgesByCited` in `histogramLayout.ts`,
   covered by a unit test in `histogram-layout.test.mjs`. No browser smoke step was added because
   `npm run smoke` has no local fixture; the unit test covers the below-top-N case.
2. Gate note: the shared test DB `lysilogy_test` already has migration 0009 from the provenance
   track, so this branch's Rust tests were run against a separate `lysilogy_test_histogram` database
   (344+ tests pass, clippy clean).
