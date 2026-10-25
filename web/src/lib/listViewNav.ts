// The h / l moves of the Topics and Timeline views: across layers or years, landing at the same
// depth when the next column is shorter. Pure, so the moves are tested without a DOM.

import { groupCursorId } from "./listViewModel.ts";
import { buildHistogramGrid, type HistogramEntry, type HistogramGrid } from "./histogramLayout.ts";
import { UNFILED_KEY, type ItemRow, type TimelineBucket, type TopicLayers } from "./listViews.ts";

/** Clamp `index` into a column of `length` cursor ids; an empty column has no landing. */
function landing(column: readonly string[], index: number): string | null {
  if (column.length === 0) return null;
  return column[Math.min(Math.max(index, 0), column.length - 1)] ?? null;
}

/**
 * Cursor columns of the Topics view: each layer lists its topic ids with the open topics' works
 * after each; the Unfiled group is one last column. Index 0 of a layer is a topic header.
 */
export function topicColumns(model: TopicLayers): string[][] {
  const columns: string[][] = model.layers.map((layer) => layer.flatMap((entry) => [entry.topic.id, ...(entry.folded ? [] : entry.works.map((row) => row.id))]));
  if (model.unfiled !== null) columns.push([groupCursorId(UNFILED_KEY), ...model.unfiled.visible.map((row) => row.id)]);
  return columns;
}

/** Where `h` (-1) or `l` (+1) goes in the Topics view; lands on the topic header of the layer. */
export function topicLayerMove(model: TopicLayers, cursor: string | null, delta: -1 | 1): string | null {
  const columns = topicColumns(model);
  const from = columns.findIndex((column) => cursor !== null && column.includes(cursor));
  if (from < 0) return null;
  const headers = model.layers.map((layer) => layer.map((entry) => entry.topic.id));
  if (model.unfiled !== null) headers.push([groupCursorId(UNFILED_KEY)]);
  // Land on the topic header at the same place in the layer as the topic the cursor is in.
  const here = headers[from] ?? [];
  const column = columns[from] ?? [];
  const cursorAt = column.indexOf(cursor ?? "");
  let topicIndex = 0;
  for (let index = 0; index < here.length; index += 1) {
    const header = here[index] ?? "";
    if (column.indexOf(header) <= cursorAt) topicIndex = index;
  }
  // Skip empty layers (a layer can only be empty if the DAG has a gap, which layering never makes).
  for (let to = from + delta; to >= 0 && to < headers.length; to += delta) {
    const target = landing(headers[to] ?? [], topicIndex);
    if (target !== null) return target;
  }
  return null;
}

/**
 * The Timeline laid out by the histogram grid: one column per occupied year, narrow empty-year
 * slots, gap slots. Rows keep manual order within a year (score = -ordinal). Undated rows, and
 * years outside the histogram range, fall into the grid's unknown column.
 */
export function timelineGrid(buckets: readonly TimelineBucket[], undated: readonly ItemRow[]): HistogramGrid {
  const entry = (row: ItemRow, year: number | null): HistogramEntry => ({ id: row.id, year, score: -row.ordinal, markRank: 1, titleKey: "" });
  return buildHistogramGrid([
    ...buckets.flatMap((bucket) => bucket.rows.map((row) => entry(row, bucket.year))),
    ...undated.map((row) => entry(row, null)),
  ]);
}

/** Grid columns as cursor ids in walk order: years oldest first, the unknown column last. */
export function timelineGridColumns(grid: HistogramGrid): string[][] {
  const dated = grid.columns.filter((column) => column.year !== null);
  const unknown = grid.columns.filter((column) => column.year === null);
  return [...dated, ...unknown].map((column) => column.entries.map((item) => item.id));
}

/** Cursor columns of the Timeline: one per year with rows, then the undated rows if any. */
export function timelineColumns(buckets: readonly TimelineBucket[], undated: readonly ItemRow[]): string[][] {
  return timelineGridColumns(timelineGrid(buckets, undated));
}

/** Where `h` (-1) or `l` (+1) goes in the Timeline: the same depth in the next year. */
export function timelineYearMove(columns: readonly (readonly string[])[], cursor: string | null, delta: -1 | 1): string | null {
  const from = columns.findIndex((column) => cursor !== null && column.includes(cursor));
  if (from < 0) return null;
  const depth = columns[from]?.indexOf(cursor ?? "") ?? 0;
  for (let to = from + delta; to >= 0 && to < columns.length; to += delta) {
    const target = landing(columns[to] ?? [], depth);
    if (target !== null) return target;
  }
  return null;
}
