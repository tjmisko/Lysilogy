// The state the list page keeps for its views, and which node ids each view lets the cursor visit.
// Pure, so the cursor order is tested without a DOM.

import type { ListDetail } from "../types";
import {
  acquire,
  graphModel,
  groupByTopic,
  nextUp,
  review,
  reviewItems,
  sequenceRows,
  timeline,
  topicLayers,
  UNFILED_KEY,
  type FoldState,
  type ItemRow,
  type ListViewId,
  type SortKey,
  type TopicGroup,
} from "./listViews.ts";

export type ListViewState = {
  /** `s` cycles it, `S` reverses; view-only, never written to the list. */
  sort: SortKey;
  reverse: boolean;
  /** `/` filter text. */
  filter: string;
  /** `zg`: Sequence grouped by topic. */
  grouped: boolean;
  folded: FoldState;
  /** The row `dd` cut, waiting for `p` / `P`. */
  cutId: string | null;
  /** The node `E` started an edge from. */
  edgeFrom: string | null;
};

export const INITIAL_VIEW_STATE: ListViewState = {
  sort: "manual",
  reverse: false,
  filter: "",
  grouped: false,
  folded: new Set<string>(),
  cutId: null,
  edgeFrom: null,
};

/** The prefix of a topic group header's cursor id in grouped Sequence and the Topics view. */
export const GROUP_PREFIX = "group:";

export const groupCursorId = (key: string): string => `${GROUP_PREFIX}${key}`;

/** The group key a cursor id names, or null for a node id. */
export function groupKeyOf(cursorId: string | null): string | null {
  return cursorId?.startsWith(GROUP_PREFIX) === true ? cursorId.slice(GROUP_PREFIX.length) : null;
}

/** The node a cursor id stands on: a row's own node, a topic header's topic, else null. */
export function nodeIdOf(cursorId: string | null): string | null {
  if (cursorId === null) return null;
  const key = groupKeyOf(cursorId);
  if (key === null) return cursorId;
  return key === UNFILED_KEY ? null : key;
}

export type SequenceModel = {
  rows: ItemRow[];
  /** Null when the Sequence is flat. */
  groups: TopicGroup[] | null;
  /** Cursor ids in display order: rows, and group headers when grouped. */
  ids: string[];
};

export function sequenceModel(detail: ListDetail, state: ListViewState): SequenceModel {
  const rows = sequenceRows(detail, { sort: state.sort, reverse: state.reverse, filter: state.filter });
  if (!state.grouped) return { rows, groups: null, ids: rows.map((row) => row.id) };
  const groups = groupByTopic(detail, rows, state.folded, state.filter.trim() !== "");
  return {
    rows,
    groups,
    ids: groups.flatMap((group) => [groupCursorId(group.key), ...group.visible.map((row) => row.id)]),
  };
}

/** Why `dd` / `p` / `J` cannot reorder right now, or null when they can. */
export function reorderBlockReason(state: ListViewState): string | null {
  if (state.sort !== "manual" || state.reverse) return "Reordering needs the manual order: press s until it reads manual.";
  if (state.filter.trim() !== "") return "Clear the filter before reordering.";
  if (state.grouped) return "Ungroup (zg) before reordering.";
  return null;
}

/** The fold key of the group the cursor is in or on, for zo / zc / za. */
export function foldKeyAt(detail: ListDetail, state: ListViewState, view: ListViewId, cursorId: string | null): string | null {
  if (cursorId === null) return null;
  const header = groupKeyOf(cursorId);
  if (header !== null) return header;
  if (view === 1) {
    const model = sequenceModel(detail, state);
    return model.groups?.find((group) => group.rows.some((row) => row.id === cursorId))?.key ?? null;
  }
  if (view === 2) {
    const node = detail.nodes.find((entry) => entry.id === cursorId);
    if (node?.kind === "topic") return node.id;
    const layers = topicLayers(detail, state.folded);
    for (const entry of layers.layers.flat()) if (entry.works.some((row) => row.id === cursorId)) return entry.topic.id;
    return layers.unfiled?.rows.some((row) => row.id === cursorId) === true ? UNFILED_KEY : null;
  }
  return null;
}

/** Every fold key that exists in the view, for zM. */
export function foldKeys(detail: ListDetail, state: ListViewState, view: ListViewId): string[] {
  if (view === 1) return sequenceModel(detail, { ...state, grouped: true, folded: new Set() }).groups?.map((group) => group.key) ?? [];
  if (view === 2) {
    const layers = topicLayers(detail);
    return [...layers.layers.flat().map((entry) => entry.topic.id), ...(layers.unfiled === null ? [] : [UNFILED_KEY])];
  }
  return [];
}

function unique(ids: readonly string[]): string[] {
  return [...new Set(ids)];
}

/** Cursor ids of one view, in the order the cursor walks them. */
export function viewOrder(view: ListViewId, detail: ListDetail, state: ListViewState): string[] {
  switch (view) {
    case 1: return sequenceModel(detail, state).ids;
    case 2: {
      const { layers, unfiled } = topicLayers(detail, state.folded);
      const ids = layers.flat().flatMap((entry) => [entry.topic.id, ...(entry.folded ? [] : entry.works.map((row) => row.id))]);
      if (unfiled !== null) ids.push(groupCursorId(UNFILED_KEY), ...unfiled.visible.map((row) => row.id));
      return unique(ids);
    }
    case 3: {
      const { buckets, undated } = timeline(detail);
      return [...buckets.flatMap((bucket) => bucket.rows.map((row) => row.id)), ...undated.map((row) => row.id)];
    }
    case 4: return graphModel(detail).layers.flatMap((layer) => layer.map((row) => row.id));
    case 5: return nextUp(detail).items.map((item) => item.row.id);
    case 6: return acquire(detail).rows.map((entry) => entry.row.id);
    case 7: return reviewItems(review(detail)).flatMap((item) => (item.kind === "edge" ? [] : [item.node.id]));
  }
}
