import type { GraphEdge, GraphNeighbors, WorkMarkName } from "../types";

/** Which view of the literature overlay is showing. */
export type LiteratureView = "radial" | "histogram";
export const LITERATURE_VIEW_STORAGE_KEY = "lysilogy.literature.view";

/** "histogram" is the only stored value that selects the histogram; anything else is radial. */
export function parseLiteratureView(stored: string | null | undefined): LiteratureView {
  return stored === "histogram" ? "histogram" : "radial";
}

/** Server `LIMIT_MAX`. */
export const HISTOGRAM_LIMIT = 500;
/** Server `VALID_YEARS`. */
export const HISTOGRAM_YEAR_MIN = 1;
export const HISTOGRAM_YEAR_MAX = 2200;
/** Year columns use `year:<n>`. */
export const UNKNOWN_COLUMN_KEY = "unknown";
/** Longer runs of empty years collapse to one gap slot. */
export const EMPTY_RUN_MAX = 3;
export const COLLAPSED_AUTHOR_LIMIT = 3;
export const EXPANDED_AUTHOR_LIMIT = 8;

/** Layout geometry in content pixels. */
export const HISTOGRAM_GEOMETRY = {
  paddingX: 16,
  axisHeight: 30,
  cardTop: 38,
  paddingBottom: 24,
  cardWidth: 176,
  columnGap: 10,
  unknownLaneGap: 28,
  emptyYearWidth: 14,
  gapSlotWidth: 40,
  collapsedHeight: 46,
  expandedHeldHeight: 236,
  expandedUnheldHeight: 150,
  rowGap: 6,
} as const;

export type MarkRank = 0 | 1 | 2 | 3;
export type HistogramEntry = {
  id: string;
  year: number | null;
  score: number;
  markRank: MarkRank;
  titleKey: string;
};

/** The year when it is an integer in the server's valid range; otherwise null ("Year unknown"). */
export function histogramYear(year: number | null | undefined): number | null {
  if (year === null || year === undefined) return null;
  if (!Number.isInteger(year)) return null;
  if (year < HISTOGRAM_YEAR_MIN || year > HISTOGRAM_YEAR_MAX) return null;
  return year;
}

/** Pinned 0, unmarked 1, downplayed 2, hidden 3. */
export function markRank(mark: WorkMarkName | null | undefined): MarkRank {
  if (mark === "pinned") return 0;
  if (mark === "downplayed") return 2;
  if (mark === "hidden") return 3;
  return 1;
}

export function titleKey(title: string | null | undefined): string {
  return (title ?? "").trim().toLowerCase();
}

/** Mark rank, score desc, titled before untitled, title key asc, id asc. Nonzero for distinct ids. */
export function compareHistogramEntries(left: HistogramEntry, right: HistogramEntry): number {
  if (left.markRank !== right.markRank) return left.markRank - right.markRank;
  if (left.score !== right.score) return right.score - left.score;
  const leftUntitled = left.titleKey === "";
  const rightUntitled = right.titleKey === "";
  if (leftUntitled !== rightUntitled) return leftUntitled ? 1 : -1;
  if (left.titleKey < right.titleKey) return -1;
  if (left.titleKey > right.titleKey) return 1;
  if (left.id < right.id) return -1;
  if (left.id > right.id) return 1;
  return 0;
}

/** Entries for every node except the center; duplicate ids keep the first; missing or NaN score is 0. */
export function histogramEntries(
  neighbors: Pick<GraphNeighbors, "center" | "nodes">,
  markOf: (workId: string) => WorkMarkName | null,
  scoreOf: (workId: string) => number | undefined,
): HistogramEntry[] {
  const seen = new Set<string>();
  const entries: HistogramEntry[] = [];
  for (const work of neighbors.nodes) {
    if (work.id === neighbors.center) continue;
    if (seen.has(work.id)) continue;
    seen.add(work.id);
    const rawScore = scoreOf(work.id);
    entries.push({
      id: work.id,
      year: histogramYear(work.display_year),
      score: rawScore === undefined || !Number.isFinite(rawScore) ? 0 : rawScore,
      markRank: markRank(markOf(work.id)),
      titleKey: titleKey(work.display_title),
    });
  }
  return entries;
}

export type HistogramColumn = {
  key: string;
  year: number | null;
  entries: readonly HistogramEntry[];
};
export type HistogramSlot =
  | { kind: "column"; columnIndex: number }
  | { kind: "empty_year"; year: number }
  | { kind: "gap"; fromYear: number; toYear: number };
export type HistogramGrid = {
  columns: readonly HistogramColumn[];
  slots: readonly HistogramSlot[];
};

/** Unknown lane first (only if non-empty), then occupied years ascending; entries sorted. */
export function buildHistogramGrid(entries: readonly HistogramEntry[]): HistogramGrid {
  const seen = new Set<string>();
  const unknownEntries: HistogramEntry[] = [];
  const entriesByYear = new Map<number, HistogramEntry[]>();
  for (const entry of entries) {
    if (seen.has(entry.id)) continue;
    seen.add(entry.id);
    const year = histogramYear(entry.year);
    if (year === null) {
      unknownEntries.push(entry);
      continue;
    }
    const bucket = entriesByYear.get(year);
    if (bucket) bucket.push(entry);
    else entriesByYear.set(year, [entry]);
  }

  const columns: HistogramColumn[] = [];
  const slots: HistogramSlot[] = [];
  if (unknownEntries.length > 0) {
    columns.push({
      key: UNKNOWN_COLUMN_KEY,
      year: null,
      entries: unknownEntries.sort(compareHistogramEntries),
    });
    slots.push({ kind: "column", columnIndex: 0 });
  }

  const years = [...entriesByYear.keys()].sort((a, b) => a - b);
  let previousYear: number | null = null;
  for (const year of years) {
    if (previousYear !== null) {
      const emptyCount = year - previousYear - 1;
      if (emptyCount > EMPTY_RUN_MAX) {
        slots.push({ kind: "gap", fromYear: previousYear + 1, toYear: year - 1 });
      } else {
        for (let empty = previousYear + 1; empty < year; empty += 1) {
          slots.push({ kind: "empty_year", year: empty });
        }
      }
    }
    columns.push({
      key: `year:${year}`,
      year,
      entries: (entriesByYear.get(year) ?? []).sort(compareHistogramEntries),
    });
    slots.push({ kind: "column", columnIndex: columns.length - 1 });
    previousYear = year;
  }
  return { columns, slots };
}

export type HistogramPosition = { columnIndex: number; row: number };

export function positionOf(grid: HistogramGrid, workId: string | null): HistogramPosition | null {
  if (workId === null) return null;
  for (let columnIndex = 0; columnIndex < grid.columns.length; columnIndex += 1) {
    const row = (grid.columns[columnIndex]?.entries ?? []).findIndex((entry) => entry.id === workId);
    if (row >= 0) return { columnIndex, row };
  }
  return null;
}

/** The Work at exactly this position; null when out of range. */
export function workAt(grid: HistogramGrid, position: HistogramPosition): string | null {
  const column = grid.columns[position.columnIndex];
  if (!column) return null;
  return column.entries[position.row]?.id ?? null;
}

/** Best row-0 entry by the column ordering; ties go to the leftmost column. */
export function initialCursor(grid: HistogramGrid): string | null {
  let best: HistogramEntry | null = null;
  for (const column of grid.columns) {
    const candidate = column.entries[0];
    if (!candidate) continue;
    if (best === null || compareHistogramEntries(candidate, best) < 0) best = candidate;
  }
  return best?.id ?? null;
}

export type HistogramDirection = "left" | "right" | "up" | "down";

export function directionForKey(key: string): HistogramDirection | null {
  if (key === "h" || key === "ArrowLeft") return "left";
  if (key === "l" || key === "ArrowRight") return "right";
  if (key === "k" || key === "ArrowUp") return "up";
  if (key === "j" || key === "ArrowDown") return "down";
  return null;
}

/** The eight direction keys plus Enter and o. */
export const HISTOGRAM_NAV_KEYS: ReadonlySet<string> = new Set([
  "h", "j", "k", "l", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Enter", "o",
]);

/** Clamped move (no wrap). A null or unknown workId starts at `initialCursor`; an empty grid gives null. */
export function moveCursor(
  grid: HistogramGrid,
  workId: string | null,
  direction: HistogramDirection,
): string | null {
  if (grid.columns.length === 0) return null;
  const position = positionOf(grid, workId);
  if (position === null) return initialCursor(grid);

  let { columnIndex, row } = position;
  if (direction === "left") columnIndex = Math.max(0, columnIndex - 1);
  if (direction === "right") columnIndex = Math.min(grid.columns.length - 1, columnIndex + 1);
  const column = grid.columns[columnIndex];
  if (!column) return null;
  const columnLength = column.entries.length;
  if (direction === "up") row = Math.max(0, row - 1);
  if (direction === "down") row = Math.min(columnLength - 1, row + 1);
  row = Math.min(row, columnLength - 1);
  return workAt(grid, { columnIndex, row });
}

export type CursorMemory = { workId: string; columnKey: string; position: HistogramPosition };

export function rememberCursor(grid: HistogramGrid, workId: string | null): CursorMemory | null {
  if (workId === null) return null;
  const position = positionOf(grid, workId);
  if (position === null) return null;
  const column = grid.columns[position.columnIndex];
  if (!column) return null;
  return { workId, columnKey: column.key, position };
}

/** Same Work, else same column key at the clamped row, else the clamped column index, else initialCursor. */
export function reconcileCursor(grid: HistogramGrid, memory: CursorMemory | null): string | null {
  if (grid.columns.length === 0) return null;
  if (memory === null) return initialCursor(grid);
  if (positionOf(grid, memory.workId) !== null) return memory.workId;

  const sameKeyIndex = grid.columns.findIndex((column) => column.key === memory.columnKey);
  const columnIndex = sameKeyIndex >= 0
    ? sameKeyIndex
    : Math.max(0, Math.min(grid.columns.length - 1, memory.position.columnIndex));
  const columnLength = grid.columns[columnIndex]?.entries.length ?? 0;
  const row = Math.max(0, Math.min(columnLength - 1, memory.position.row));
  return workAt(grid, { columnIndex, row }) ?? initialCursor(grid);
}

export type HistogramCard = {
  id: string;
  columnIndex: number;
  row: number;
  x: number;
  y: number;
  width: number;
  height: number;
  expanded: boolean;
};
export type HistogramAxisSlot = {
  kind: "column" | "unknown" | "empty_year" | "gap";
  x: number;
  width: number;
  /** "1998", "Year unknown", "" (empty_year), "1901–1949" (gap). */
  label: string;
  /** Full text for the `title` attribute, e.g. "1998 · 4 references", "1950". */
  title: string;
  /** References in the slot (0 for empty_year and gap). */
  count: number;
};
export type HistogramLayout = {
  cards: readonly HistogramCard[];
  axis: readonly HistogramAxisSlot[];
  width: number;
  height: number;
};

function referenceCountText(count: number): string {
  return `${count} ${count === 1 ? "reference" : "references"}`;
}

/** Exactly one card is expanded iff `focusedWorkId` is in the grid. */
export function layoutHistogram(
  grid: HistogramGrid,
  focusedWorkId: string | null,
  isHeld: (workId: string) => boolean,
): HistogramLayout {
  const geometry = HISTOGRAM_GEOMETRY;
  const cards: HistogramCard[] = [];
  const axis: HistogramAxisSlot[] = [];
  let columnBottomMax = geometry.cardTop - geometry.rowGap;
  let cursorX: number = geometry.paddingX;

  for (const slot of grid.slots) {
    if (slot.kind === "empty_year") {
      axis.push({
        kind: "empty_year", x: cursorX, width: geometry.emptyYearWidth,
        label: "", title: String(slot.year), count: 0,
      });
      cursorX += geometry.emptyYearWidth;
      continue;
    }
    if (slot.kind === "gap") {
      const range = `${slot.fromYear}–${slot.toYear}`;
      axis.push({
        kind: "gap", x: cursorX, width: geometry.gapSlotWidth,
        label: range, title: range, count: 0,
      });
      cursorX += geometry.gapSlotWidth;
      continue;
    }

    const column = grid.columns[slot.columnIndex];
    if (!column) continue;
    const isUnknown = column.year === null;
    const slotWidth = geometry.cardWidth + (isUnknown ? geometry.unknownLaneGap : geometry.columnGap);
    const count = column.entries.length;
    const label = isUnknown ? "Year unknown" : String(column.year);
    axis.push({
      kind: isUnknown ? "unknown" : "column", x: cursorX, width: slotWidth,
      label, title: `${label} · ${referenceCountText(count)}`, count,
    });

    let rowY: number = geometry.cardTop;
    column.entries.forEach((entry, row) => {
      const expanded = entry.id === focusedWorkId;
      const height = !expanded
        ? geometry.collapsedHeight
        : isHeld(entry.id) ? geometry.expandedHeldHeight : geometry.expandedUnheldHeight;
      cards.push({
        id: entry.id, columnIndex: slot.columnIndex, row,
        x: cursorX, y: rowY, width: geometry.cardWidth, height, expanded,
      });
      columnBottomMax = Math.max(columnBottomMax, rowY + height);
      rowY += height + geometry.rowGap;
    });
    cursorX += slotWidth;
  }

  const width = grid.slots.length === 0
    ? 2 * geometry.paddingX + geometry.cardWidth
    : cursorX + geometry.paddingX;
  const height = Math.max(columnBottomMax, geometry.cardTop) + geometry.paddingBottom;
  return { cards, axis, width, height };
}

export type Rect = { x: number; y: number; width: number; height: number };
export type ScrollViewport = { left: number; top: number; width: number; height: number };
export type RevealMargins = { x: number; top: number; bottom: number };
export const REVEAL_MARGINS: RevealMargins = {
  x: 24,
  top: HISTOGRAM_GEOMETRY.axisHeight + 8,
  bottom: 16,
};

/** Minimal scroll so `rect` (content coordinates) is visible; tall or wide rects align their leading edge. */
export function scrollToReveal(
  rect: Rect,
  viewport: ScrollViewport,
  margins: RevealMargins = REVEAL_MARGINS,
): { left: number; top: number } {
  let left = viewport.left;
  let top = viewport.top;

  const visibleWidth = viewport.width - 2 * margins.x;
  if (rect.width > visibleWidth || rect.x < viewport.left + margins.x) {
    left = rect.x - margins.x;
  } else if (rect.x + rect.width > viewport.left + viewport.width - margins.x) {
    left = rect.x + rect.width + margins.x - viewport.width;
  }

  const visibleHeight = viewport.height - margins.top - margins.bottom;
  if (rect.height > visibleHeight || rect.y < viewport.top + margins.top) {
    top = rect.y - margins.top;
  } else if (rect.y + rect.height > viewport.top + viewport.height - margins.bottom) {
    top = rect.y + rect.height + margins.bottom - viewport.height;
  }

  return { left: Math.max(0, left), top: Math.max(0, top) };
}

export type Point = { x: number; y: number };

/**
 * Card top-centre in section coordinates (content position minus scroll, plus the viewport box
 * origin), y clamped into the viewport box; null when x falls outside the box.
 */
export function seedLinkTarget(
  card: Rect,
  scroll: { left: number; top: number },
  viewportBox: Rect,
): Point | null {
  const x = viewportBox.x + card.x + card.width / 2 - scroll.left;
  if (x < viewportBox.x || x > viewportBox.x + viewportBox.width) return null;
  const rawY = viewportBox.y + card.y - scroll.top;
  const y = Math.min(Math.max(rawY, viewportBox.y), viewportBox.y + viewportBox.height);
  return { x, y };
}

function roundToTenth(value: number): number {
  const rounded = Math.round(value * 10) / 10;
  return rounded === 0 ? 0 : rounded;
}

/** "M fx fy C fx my, tx my, tx ty" with my = (fy + ty) / 2, numbers rounded to 0.1. */
export function seedLinkPath(from: Point, to: Point): string {
  const fx = roundToTenth(from.x);
  const fy = roundToTenth(from.y);
  const tx = roundToTenth(to.x);
  const ty = roundToTenth(to.y);
  const my = roundToTenth((from.y + to.y) / 2);
  return `M ${fx} ${fy} C ${fx} ${my}, ${tx} ${my}, ${tx} ${ty}`;
}

/** Trimmed non-blank names; null when none; more than `limit` gives the first `limit` plus " +<rest>". */
export function authorLine(authors: readonly string[] | null | undefined, limit: number): string | null {
  if (!authors) return null;
  const names = authors.map((name) => name.trim()).filter((name) => name !== "");
  if (names.length === 0) return null;
  if (names.length <= limit) return names.join(", ");
  return `${names.slice(0, limit).join(", ")} +${names.length - limit}`;
}

/**
 * Edges from the center Work keyed by cited id. Edge lists are ordered by priority: the first
 * list to mention a cited id wins, so the radial fetch overrides the wider histogram fetch.
 */
export function citedEdgesByCited(centerId: string, edgeLists: readonly (readonly GraphEdge[])[]): Map<string, GraphEdge> {
  const map = new Map<string, GraphEdge>();
  for (const edges of edgeLists) {
    for (const edge of edges) {
      if (edge.citing !== centerId || map.has(edge.cited)) continue;
      map.set(edge.cited, edge);
    }
  }
  return map;
}
