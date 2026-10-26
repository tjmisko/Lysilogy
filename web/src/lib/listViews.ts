// Pure projections of a ListDetail into the rows each list view shows (spec: Views).
// Nothing here writes to the list. Sorting, filtering, grouping and folding are view state.
//
// Proposed nodes and edges (D11) belong to the Review view only; every other view sees the
// accepted graph, so a pending agent suggestion never changes what you read next.

import type {
  AcquisitionState,
  ClaimState,
  ContextLink,
  ListDetail,
  ListEdgeView,
  ListNodeView,
  ReadingStatus,
  ReadingStatusCounts,
} from "../types";
import { blockersByWork, frontier, linearize, type GraphEdge, type LinearItem } from "./listGraph.ts";

// ---- View ids, sort cycle, status glyphs ---------------------------------------------------

export type ListViewId = 1 | 2 | 3 | 4 | 5 | 6 | 7;
export const LIST_VIEWS: readonly { id: ListViewId; name: string }[] = [
  { id: 1, name: "Sequence" },
  { id: 2, name: "Topics" },
  { id: 3, name: "Timeline" },
  { id: 4, name: "Graph" },
  { id: 5, name: "Next up" },
  { id: 6, name: "Acquire" },
  { id: 7, name: "Review" },
];

/** The view `v` moves to; past the last it wraps to the first. */
export function nextView(view: ListViewId): ListViewId {
  return view === 7 ? 1 : ((view + 1) as ListViewId);
}

/** A view number from a route or key, or Sequence when it is not 1 to 7. */
export function parseView(value: string | number | null | undefined): ListViewId {
  const number = typeof value === "string" ? Number(value) : value;
  if (number === undefined || number === null || !Number.isInteger(number) || number < 1 || number > 7) return 1;
  return number as ListViewId;
}

export type SortKey =
  | "manual" | "topological" | "year_asc" | "year_desc" | "prominence" | "title" | "status" | "added";

/** `s` steps through these in order and wraps. */
export const SORT_CYCLE: readonly SortKey[] = [
  "manual", "topological", "year_asc", "year_desc", "prominence", "title", "status", "added",
];

export function nextSort(sort: SortKey): SortKey {
  const index = SORT_CYCLE.indexOf(sort);
  return SORT_CYCLE[(index + 1) % SORT_CYCLE.length] ?? "manual";
}

export const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  manual: "manual",
  topological: "topological",
  year_asc: "year ↑",
  year_desc: "year ↓",
  prominence: "prominence",
  title: "title",
  status: "status",
  added: "added",
};

export type StatusGlyph = "○" | "◐" | "●";

export function statusGlyph(status: ReadingStatus): StatusGlyph {
  if (status === "read") return "●";
  return status === "reading" ? "◐" : "○";
}

// ---- Rows ----------------------------------------------------------------------------------

export type ItemRow = {
  node: ListNodeView;
  id: string;
  /** Counting from 1 in the order the view shows. */
  ordinal: number;
  kind: "work" | "claim";
  workId: string | null;
  title: string;
  year: number | null;
  authors: string[];
  status: ReadingStatus;
  /** Labels of the directly covering topics, in topic position order. */
  topics: string[];
  topicIds: string[];
  /** A local PDF of the Work is held. */
  held: boolean;
  paperId: string | null;
  /** Acquisition state of a Work that has one; null for claims and Works never fetched. */
  fetch: AcquisitionState | null;
  note: string | null;
  claimState: ClaimState | null;
  contextLinks: ContextLink[];
};

const UNTITLED = "Untitled";

function byPosition(left: ListNodeView, right: ListNodeView): number {
  return left.position - right.position || compareText(left.id, right.id);
}

function compareText(left: string, right: string): number {
  if (left < right) return -1;
  return left > right ? 1 : 0;
}

/** Lowercase letters and digits of any script, single-spaced; the key titles sort and match by. */
export function titleKey(title: string): string {
  return title.normalize("NFKC").toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
}

function nodeTitle(node: ListNodeView): string {
  const title = node.work?.display_title ?? node.claim?.title ?? node.label ?? "";
  return title.trim() === "" ? UNTITLED : title;
}

const accepted = (node: { review: string }): boolean => node.review === "accepted";

function acceptedEdges(detail: ListDetail): ListEdgeView[] {
  return detail.edges.filter(accepted);
}

function graphEdges(detail: ListDetail): GraphEdge[] {
  return acceptedEdges(detail).map(({ from, to, kind }) => ({ from, to, kind }));
}

/** Directly covering topics of every node, in topic position order. */
function topicsOfNodes(detail: ListDetail): Map<string, ListNodeView[]> {
  const topics = new Map(detail.nodes.filter((node) => node.kind === "topic" && accepted(node)).map((node) => [node.id, node]));
  const found = new Map<string, ListNodeView[]>();
  for (const edge of acceptedEdges(detail)) {
    const topic = topics.get(edge.from);
    if (edge.kind !== "covers" || topic === undefined) continue;
    const list = found.get(edge.to) ?? [];
    list.push(topic);
    found.set(edge.to, list);
  }
  for (const list of found.values()) list.sort(byPosition);
  return found;
}

export function statusOf(detail: ListDetail, workId: string | null): ReadingStatus {
  if (workId === null) return "unread";
  return detail.reading_states[workId]?.status ?? "unread";
}

function rowOf(detail: ListDetail, node: ListNodeView, topics: ReadonlyMap<string, ListNodeView[]>): ItemRow {
  const workId = node.work_id;
  const papers = workId === null ? undefined : detail.local_papers[workId];
  const topicNodes = topics.get(node.id) ?? [];
  return {
    node,
    id: node.id,
    ordinal: node.ordinal,
    kind: node.kind === "claim" ? "claim" : "work",
    workId,
    title: nodeTitle(node),
    year: node.work?.display_year ?? node.claim?.year ?? null,
    authors: node.work?.display_authors ?? node.claim?.authors ?? [],
    status: statusOf(detail, workId),
    topics: topicNodes.map((topic) => topic.label ?? UNTITLED),
    topicIds: topicNodes.map((topic) => topic.id),
    held: papers !== undefined && papers.length > 0,
    paperId: papers?.[0]?.paper_id ?? null,
    fetch: workId === null ? null : (detail.acquisitions[workId]?.state ?? null),
    note: node.note,
    claimState: node.claim_state,
    contextLinks: node.context_links,
  };
}

/** Accepted Work and claim nodes in manual (position) order, numbered from 1. */
export function itemRows(detail: ListDetail): ItemRow[] {
  const topics = topicsOfNodes(detail);
  return detail.nodes
    .filter((node) => node.kind !== "topic" && accepted(node))
    .sort(byPosition)
    .map((node, index) => ({ ...rowOf(detail, node, topics), ordinal: index + 1 }));
}

function renumber(rows: ItemRow[]): ItemRow[] {
  return rows.map((row, index) => ({ ...row, ordinal: index + 1 }));
}

// ---- Prominence ----------------------------------------------------------------------------

/**
 * How prominent each member Work is: the number of other members that cite it. The server's
 * graph prominence is not part of ListDetail, so callers that have it pass it instead.
 */
export function memberProminence(detail: ListDetail): Record<string, number> {
  const score: Record<string, number> = {};
  const seen = new Set<string>();
  for (const [citing, cited] of detail.citations) {
    const pair = `${citing}\u0000${cited}`;
    if (citing === cited || seen.has(pair)) continue;
    seen.add(pair);
    score[cited] = (score[cited] ?? 0) + 1;
  }
  return score;
}

// ---- Sequence ------------------------------------------------------------------------------

export type SequenceOptions = {
  sort?: SortKey;
  /** `S`: reverse whatever order the sort gave. */
  reverse?: boolean;
  /** `/`: every whitespace-separated term must match somewhere in the row. */
  filter?: string;
  prominence?: Readonly<Record<string, number>>;
};

const STATUS_ORDER: Readonly<Record<ReadingStatus, number>> = { reading: 0, unread: 1, read: 2 };

function compareYear(left: number | null, right: number | null, direction: 1 | -1): number {
  if (left === right) return 0;
  if (left === null) return 1;
  if (right === null) return -1;
  return (left - right) * direction;
}

/** The linearization of Work and claim rows (D9), as ids. */
export function topologicalOrder(
  detail: ListDetail,
  rows: readonly ItemRow[],
  prominence: Readonly<Record<string, number>>,
): string[] {
  const seed = detail.list.seed_work_id;
  const items: LinearItem[] = rows.map((row) => ({
    id: row.id,
    workId: row.workId,
    depth: row.node.depth,
    score: row.workId === null ? 0 : (prominence[row.workId] ?? 0),
    year: row.year,
    titleKey: titleKey(row.title),
    isSeed: seed !== null && row.workId === seed,
  }));
  return linearize(items, graphEdges(detail), detail.citations);
}

function searchText(row: ItemRow): string {
  return [row.title, row.authors.join(" "), row.topics.join(" "), row.note ?? "", row.year === null ? "" : String(row.year), row.status]
    .join(" ")
    .normalize("NFKC")
    .toLowerCase();
}

/** Rows matching every term of `filter`; a blank filter keeps everything. */
export function filterRows(rows: readonly ItemRow[], filter: string): ItemRow[] {
  const terms = filter.normalize("NFKC").toLowerCase().split(/\s+/u).filter((term) => term !== "");
  if (terms.length === 0) return [...rows];
  return rows.filter((row) => {
    const text = searchText(row);
    return terms.every((term) => text.includes(term));
  });
}

/** The flat Sequence rows: sorted, optionally reversed and filtered, numbered from 1. */
export function sequenceRows(detail: ListDetail, options: SequenceOptions = {}): ItemRow[] {
  const manual = itemRows(detail);
  const prominence = options.prominence ?? memberProminence(detail);
  const score = (row: ItemRow): number => (row.workId === null ? 0 : (prominence[row.workId] ?? 0));
  const manualRank = new Map(manual.map((row, index) => [row.id, index]));
  const stable = (left: ItemRow, right: ItemRow): number => (manualRank.get(left.id) ?? 0) - (manualRank.get(right.id) ?? 0);
  let sorted: ItemRow[];
  switch (options.sort ?? "manual") {
    case "manual":
      sorted = manual;
      break;
    case "topological": {
      const rank = new Map(topologicalOrder(detail, manual, prominence).map((id, index) => [id, index]));
      sorted = [...manual].sort((a, b) => (rank.get(a.id) ?? 0) - (rank.get(b.id) ?? 0));
      break;
    }
    case "year_asc":
      sorted = [...manual].sort((a, b) => compareYear(a.year, b.year, 1) || stable(a, b));
      break;
    case "year_desc":
      sorted = [...manual].sort((a, b) => compareYear(a.year, b.year, -1) || stable(a, b));
      break;
    case "prominence":
      sorted = [...manual].sort((a, b) => score(b) - score(a) || stable(a, b));
      break;
    case "title":
      sorted = [...manual].sort((a, b) => compareText(titleKey(a.title), titleKey(b.title)) || stable(a, b));
      break;
    case "status":
      sorted = [...manual].sort((a, b) => STATUS_ORDER[a.status] - STATUS_ORDER[b.status] || stable(a, b));
      break;
    case "added":
      sorted = [...manual].sort((a, b) => compareText(b.node.added_at, a.node.added_at) || stable(a, b));
      break;
  }
  if (options.reverse === true) sorted = [...sorted].reverse();
  return renumber(filterRows(sorted, options.filter ?? ""));
}

// ---- Fold state ----------------------------------------------------------------------------

/** The key of the group of rows under no topic. */
export const UNFILED_KEY = "unfiled";

/** Ids of folded topics (and UNFILED_KEY). A topic not in the set is open. */
export type FoldState = ReadonlySet<string>;

export const foldOpenAll = (): FoldState => new Set<string>(); // zR
export const foldCloseAll = (keys: Iterable<string>): FoldState => new Set(keys); // zM

export function foldOpen(folded: FoldState, key: string): FoldState { // zo
  const next = new Set(folded);
  next.delete(key);
  return next;
}

export function foldClose(folded: FoldState, key: string): FoldState { // zc
  return new Set(folded).add(key);
}

export function foldToggle(folded: FoldState, key: string): FoldState { // za
  return folded.has(key) ? foldOpen(folded, key) : foldClose(folded, key);
}

/** Forget folds of topics that no longer exist. */
export function pruneFolds(folded: FoldState, detail: ListDetail): FoldState {
  const live = new Set(detail.nodes.filter((node) => node.kind === "topic").map((node) => node.id));
  live.add(UNFILED_KEY);
  return new Set([...folded].filter((key) => live.has(key)));
}

export type TopicGroup = {
  /** The topic id, or UNFILED_KEY. */
  key: string;
  topic: ListNodeView | null;
  label: string;
  /** Every row of the group. */
  rows: ItemRow[];
  folded: boolean;
  /** The rows to draw: none when folded. */
  visible: ItemRow[];
};

/**
 * Group sorted rows by topic (`zg`). A Work under several topics sits once, under the first
 * topic by position; Works under none come last. Topics with no rows still appear, so a
 * new topic can be selected. Row order within a group is the order given. A filter hides
 * topics with no matching rows when `hideEmpty` is set.
 */
export function groupByTopic(
  detail: ListDetail,
  rows: readonly ItemRow[],
  folded: FoldState,
  hideEmpty = false,
): TopicGroup[] {
  const topics = detail.nodes.filter((node) => node.kind === "topic" && accepted(node)).sort(byPosition);
  const groups = new Map<string, ItemRow[]>(topics.map((topic) => [topic.id, []]));
  const unfiled: ItemRow[] = [];
  for (const row of rows) {
    const home = topics.find((topic) => row.topicIds.includes(topic.id));
    if (home === undefined) unfiled.push(row);
    else groups.get(home.id)?.push(row);
  }
  const result: TopicGroup[] = topics.map((topic) => {
    const members = groups.get(topic.id) ?? [];
    const isFolded = folded.has(topic.id);
    return {
      key: topic.id,
      topic,
      label: topic.label ?? UNTITLED,
      rows: members,
      folded: isFolded,
      visible: isFolded ? [] : members,
    };
  });
  if (unfiled.length > 0 || topics.length === 0) {
    const isFolded = folded.has(UNFILED_KEY);
    result.push({
      key: UNFILED_KEY,
      topic: null,
      label: "Unfiled",
      rows: unfiled,
      folded: isFolded,
      visible: isFolded ? [] : unfiled,
    });
  }
  return hideEmpty ? result.filter((group) => group.rows.length > 0) : result;
}

// ---- Topics view ---------------------------------------------------------------------------

export type TopicEntry = {
  topic: ListNodeView;
  /** Works filed directly under the topic, in manual order. */
  works: ItemRow[];
  folded: boolean;
  progress: ReadingStatusCounts;
};

export type TopicLayers = {
  /** Longest-path layers: a topic sits one layer after the latest topic that must come before it. */
  layers: TopicEntry[][];
  unfiled: TopicGroup | null;
};

/** Counts of Work rows per status. */
export function statusCounts(rows: readonly { status: ReadingStatus }[]): ReadingStatusCounts {
  const counts: ReadingStatusCounts = { unread: 0, reading: 0, read: 0 };
  for (const row of rows) counts[row.status] += 1;
  return counts;
}

/** Layer index of every topic. Edges that would close a cycle are ignored, not fatal. */
function layerIndexes(topics: readonly ListNodeView[], edges: readonly ListEdgeView[]): Map<string, number> {
  const ids = new Set(topics.map((topic) => topic.id));
  const before = new Map<string, string[]>();
  for (const edge of edges) {
    if (edge.kind === "covers" || !ids.has(edge.from) || !ids.has(edge.to) || edge.from === edge.to) continue;
    const list = before.get(edge.to) ?? [];
    list.push(edge.from);
    before.set(edge.to, list);
  }
  const layer = new Map<string, number>();
  const onStack = new Set<string>();
  const visit = (id: string): number => {
    const known = layer.get(id);
    if (known !== undefined) return known;
    onStack.add(id);
    let depth = 0;
    for (const parent of before.get(id) ?? []) {
      if (onStack.has(parent)) continue;
      depth = Math.max(depth, visit(parent) + 1);
    }
    onStack.delete(id);
    layer.set(id, depth);
    return depth;
  };
  for (const topic of topics) visit(topic.id);
  return layer;
}

/** The layered topic DAG with each topic's own Works (view 2). */
export function topicLayers(detail: ListDetail, folded: FoldState = new Set()): TopicLayers {
  const topics = detail.nodes.filter((node) => node.kind === "topic" && accepted(node)).sort(byPosition);
  const rows = itemRows(detail);
  const index = layerIndexes(topics, acceptedEdges(detail));
  const layers: TopicEntry[][] = [];
  for (const topic of topics) {
    const works = rows.filter((row) => row.topicIds.includes(topic.id));
    const entry: TopicEntry = { topic, works, folded: folded.has(topic.id), progress: statusCounts(works) };
    const at = index.get(topic.id) ?? 0;
    while (layers.length <= at) layers.push([]);
    layers[at]?.push(entry);
  }
  const filed = rows.filter((row) => row.topicIds.length === 0);
  const isFolded = folded.has(UNFILED_KEY);
  return {
    layers,
    unfiled: filed.length === 0
      ? null
      : { key: UNFILED_KEY, topic: null, label: "Unfiled", rows: filed, folded: isFolded, visible: isFolded ? [] : filed },
  };
}

// ---- Timeline ------------------------------------------------------------------------------

export type TimelineBucket = { year: number; rows: ItemRow[] };

export type Timeline = {
  /** Years that have rows, oldest first. */
  buckets: TimelineBucket[];
  undated: ItemRow[];
  /** Derived citations among the shown nodes, as node ids (citing, cited). */
  links: { from: string; to: string }[];
};

export function timeline(detail: ListDetail): Timeline {
  const rows = itemRows(detail);
  const byYear = new Map<number, ItemRow[]>();
  const undated: ItemRow[] = [];
  for (const row of rows) {
    if (row.year === null) {
      undated.push(row);
      continue;
    }
    const list = byYear.get(row.year) ?? [];
    list.push(row);
    byYear.set(row.year, list);
  }
  const buckets = [...byYear.entries()].sort(([a], [b]) => a - b).map(([year, members]) => ({ year, rows: members }));
  const nodeOf = new Map<string, string>();
  for (const row of rows) if (row.workId !== null && !nodeOf.has(row.workId)) nodeOf.set(row.workId, row.id);
  const links: { from: string; to: string }[] = [];
  const seen = new Set<string>();
  for (const [citing, cited] of detail.citations) {
    const from = nodeOf.get(citing);
    const to = nodeOf.get(cited);
    if (from === undefined || to === undefined || from === to || seen.has(`${from}\u0000${to}`)) continue;
    seen.add(`${from}\u0000${to}`);
    links.push({ from, to });
  }
  return { buckets, undated, links };
}

// ---- Graph ---------------------------------------------------------------------------------

export type GraphLink = { from: string; to: string; kind: "prerequisite" | "citation" };

export type GraphTopic = { id: string; label: string; rowIds: string[] };

export type GraphModel = {
  /** Layers left to right; each holds node ids in manual order. */
  layers: ItemRow[][];
  /** Prerequisite edges between members and derived citation edges, `from` read before `to`. */
  links: GraphLink[];
  /** Topics with the members they cover, for the faint group labels. */
  topics: GraphTopic[];
};

/** Members a node stands for: itself when a Work or claim, else every member under a topic. */
function memberIds(start: string, memberSet: ReadonlySet<string>, below: ReadonlyMap<string, string[]>): string[] {
  const found = new Set<string>();
  const seen = new Set<string>();
  const stack = [start];
  for (let current = stack.pop(); current !== undefined; current = stack.pop()) {
    if (seen.has(current)) continue;
    seen.add(current);
    if (memberSet.has(current)) found.add(current);
    else stack.push(...(below.get(current) ?? []));
  }
  return [...found];
}

/**
 * The layered DAG of view 4. A member's layer is the longest path to it over prerequisite edges
 * and derived citation edges (cited before citing). Prerequisites are acyclic by construction;
 * a citation edge that would close a cycle is left out of the layering, citations taken in id
 * order so the result is stable. Claims have no citations and sit by their prerequisites.
 */
export function graphModel(detail: ListDetail): GraphModel {
  const rows = itemRows(detail);
  const byId = new Map(rows.map((row) => [row.id, row]));
  const memberSet = new Set(byId.keys());
  const below = new Map<string, string[]>();
  for (const edge of acceptedEdges(detail)) {
    if (edge.kind !== "covers" && edge.kind !== "subtopic") continue;
    below.set(edge.from, [...(below.get(edge.from) ?? []), edge.to]);
  }
  const seen = new Set<string>();
  const links: GraphLink[] = [];
  const add = (from: string, to: string, kind: GraphLink["kind"]): void => {
    const key = `${from}\u0000${to}`;
    if (from === to || seen.has(key)) return;
    seen.add(key);
    links.push({ from, to, kind });
  };
  for (const edge of acceptedEdges(detail)) {
    if (edge.kind !== "prerequisite") continue;
    const after = memberIds(edge.to, memberSet, below);
    for (const first of memberIds(edge.from, memberSet, below)) for (const second of after) add(first, second, "prerequisite");
  }
  const nodeOf = new Map<string, string>();
  for (const row of rows) if (row.workId !== null && !nodeOf.has(row.workId)) nodeOf.set(row.workId, row.id);
  const successors = new Map<string, Set<string>>();
  const connect = (from: string, to: string): void => {
    successors.set(from, (successors.get(from) ?? new Set<string>()).add(to));
  };
  for (const link of links) connect(link.from, link.to);
  const reaches = (start: string, goal: string): boolean => {
    const visited = new Set<string>();
    const stack = [start];
    for (let current = stack.pop(); current !== undefined; current = stack.pop()) {
      if (current === goal) return true;
      if (visited.has(current)) continue;
      visited.add(current);
      stack.push(...(successors.get(current) ?? []));
    }
    return false;
  };
  const derived: GraphLink[] = [];
  for (const [citing, cited] of detail.citations) {
    const to = nodeOf.get(citing);
    const from = nodeOf.get(cited);
    if (from === undefined || to === undefined || from === to) continue;
    derived.push({ from, to, kind: "citation" });
  }
  derived.sort((a, b) => compareText(a.from, b.from) || compareText(a.to, b.to));
  const ordering: GraphLink[] = [...links];
  for (const link of derived) {
    const key = `${link.from}\u0000${link.to}`;
    if (seen.has(key)) continue;
    seen.add(key);
    links.push(link);
    if (reaches(link.to, link.from)) continue;
    connect(link.from, link.to);
    ordering.push(link);
  }
  const before = new Map<string, string[]>();
  for (const link of ordering) before.set(link.to, [...(before.get(link.to) ?? []), link.from]);
  const layerOf = new Map<string, number>();
  const layerFor = (id: string): number => {
    const known = layerOf.get(id);
    if (known !== undefined) return known;
    layerOf.set(id, 0);
    const depth = Math.max(-1, ...(before.get(id) ?? []).map(layerFor)) + 1;
    layerOf.set(id, depth);
    return depth;
  };
  const layers: ItemRow[][] = [];
  for (const row of rows) {
    const at = layerFor(row.id);
    while (layers.length <= at) layers.push([]);
    layers[at]?.push(row);
  }
  const topics = detail.nodes
    .filter((node) => node.kind === "topic" && accepted(node))
    .sort(byPosition)
    .map((topic) => ({
      id: topic.id,
      label: topic.label ?? UNTITLED,
      rowIds: memberIds(topic.id, memberSet, below).sort((a, b) => (byId.get(a)?.ordinal ?? 0) - (byId.get(b)?.ordinal ?? 0)),
    }))
    .filter((topic) => topic.rowIds.length > 0);
  return { layers, links, topics };
}

export const GRAPH_NODE = { width: 220, height: 46, gapX: 96, gapY: 16, margin: 20 } as const;

export type GraphPlacement = { id: string; x: number; y: number };

/** Top-left corners of every node, layers left to right and rows top down; and the canvas size. */
export function graphPlacements(model: GraphModel): { placements: GraphPlacement[]; width: number; height: number } {
  const { width, height, gapX, gapY, margin } = GRAPH_NODE;
  const placements = model.layers.flatMap((layer, column) => layer.map((row, index) => ({
    id: row.id,
    x: margin + column * (width + gapX),
    y: margin + index * (height + gapY),
  })));
  const tallest = Math.max(0, ...model.layers.map((layer) => layer.length));
  return {
    placements,
    width: margin * 2 + Math.max(1, model.layers.length) * width + Math.max(0, model.layers.length - 1) * gapX,
    height: margin * 2 + Math.max(1, tallest) * height + Math.max(0, tallest - 1) * gapY,
  };
}

// ---- Next up -------------------------------------------------------------------------------

export type NextUpItem = {
  row: ItemRow;
  /** Unread or reading Works that wait on this one. */
  unlocks: number;
};

export type NextUp = {
  items: NextUpItem[];
  /** Why the list is empty: no Works, every Work already started, or the first unread Work is blocked. */
  empty: "no_works" | "all_started" | "blocked" | null;
  /** The first unread Work in reading order and what it waits for; set when `empty` is "blocked". */
  blocked: { row: ItemRow; blockedBy: ItemRow[] } | null;
};

/** Unread Works whose prerequisites are all read, in linearized order (view 5). */
export function nextUp(detail: ListDetail, prominence: Readonly<Record<string, number>> = memberProminence(detail)): NextUp {
  const rows = itemRows(detail);
  const works = rows.filter((row) => row.kind === "work");
  if (works.length === 0) return { items: [], empty: "no_works", blocked: null };
  const edges = graphEdges(detail);
  const statuses = works.map((row) => ({ id: row.id, status: row.status }));
  const order = topologicalOrder(detail, rows, prominence);
  const ready = frontier(statuses, edges, order);
  const byId = new Map(works.map((row) => [row.id, row]));
  const blockers = blockersByWork(statuses, edges);
  const unlocks = new Map<string, number>();
  for (const blocking of blockers.values()) for (const id of blocking) unlocks.set(id, (unlocks.get(id) ?? 0) + 1);
  const items: NextUpItem[] = [];
  for (const id of ready) {
    const row = byId.get(id);
    if (row !== undefined) items.push({ row, unlocks: unlocks.get(id) ?? 0 });
  }
  if (items.length > 0) return { items, empty: null, blocked: null };
  const firstUnread = order.map((id) => byId.get(id)).find((row) => row?.status === "unread");
  if (firstUnread === undefined) return { items: [], empty: "all_started", blocked: null };
  const waitingOn = [...(blockers.get(firstUnread.id) ?? [])].sort(compareText)
    .map((id) => byId.get(id))
    .filter((row): row is ItemRow => row !== undefined);
  return { items: [], empty: "blocked", blocked: { row: firstUnread, blockedBy: waitingOn } };
}

// ---- Acquire -------------------------------------------------------------------------------

export type AcquireRow = {
  row: ItemRow;
  /** `f` can start a fetch: nothing is downloaded, mapped or ruled out yet. */
  fetchable: boolean;
};

export type Acquire = {
  rows: AcquireRow[];
  /** Work ids `F` fetches, in row order. */
  fetchable: string[];
};

const NOT_FETCHABLE: ReadonlySet<AcquisitionState> = new Set<AcquisitionState>(["downloaded", "mapped", "unavailable"]);

/** Work nodes with no local paper, with their fetch state (view 6). */
export function acquire(detail: ListDetail): Acquire {
  const rows: AcquireRow[] = itemRows(detail)
    .filter((row) => row.kind === "work" && !row.held)
    .map((row) => ({ row, fetchable: row.fetch === null || !NOT_FETCHABLE.has(row.fetch) }));
  const fetchable: string[] = [];
  for (const { row, fetchable: can } of rows) if (can && row.workId !== null) fetchable.push(row.workId);
  return { rows, fetchable };
}

// ---- Review --------------------------------------------------------------------------------

export type ReviewItem =
  | { kind: "node"; node: ListNodeView; title: string }
  | { kind: "edge"; edge: ListEdgeView; fromTitle: string; toTitle: string }
  | { kind: "claim"; node: ListNodeView; title: string; state: ClaimState; proposed: boolean };

export type ReviewSection = {
  key: "nodes" | "edges" | `claim:${ClaimState}`;
  label: string;
  items: ReviewItem[];
};

/** Pending claims first: they are the ones `:verify` can still settle. */
export const CLAIM_STATE_ORDER: readonly ClaimState[] = ["pending", "ambiguous", "contradicted", "unverified"];

/** Proposed nodes and edges, then claims by state; empty sections are left out (view 7). */
export function review(detail: ListDetail): ReviewSection[] {
  const titles = new Map(detail.nodes.map((node) => [node.id, nodeTitle(node)]));
  const proposedNodes: ReviewItem[] = detail.nodes
    .filter((node) => node.review === "proposed" && node.kind !== "claim")
    .sort(byPosition)
    .map((node) => ({ kind: "node", node, title: nodeTitle(node) }));
  const proposedEdges: ReviewItem[] = detail.edges
    .filter((edge) => edge.review === "proposed")
    .sort((a, b) => compareText(`${a.from}${a.to}${a.kind}`, `${b.from}${b.to}${b.kind}`))
    .map((edge) => ({
      kind: "edge",
      edge,
      fromTitle: titles.get(edge.from) ?? UNTITLED,
      toTitle: titles.get(edge.to) ?? UNTITLED,
    }));
  const sections: ReviewSection[] = [
    { key: "nodes", label: "Proposed items", items: proposedNodes },
    { key: "edges", label: "Proposed links", items: proposedEdges },
  ];
  for (const state of CLAIM_STATE_ORDER) {
    const items: ReviewItem[] = detail.nodes
      .filter((node) => node.kind === "claim" && node.claim_state === state)
      .sort(byPosition)
      .map((node) => ({ kind: "claim", node, title: nodeTitle(node), state, proposed: node.review === "proposed" }));
    sections.push({ key: `claim:${state}`, label: `Claims: ${state}`, items });
  }
  return sections.filter((section) => section.items.length > 0);
}

/** Every review item in the order the cursor walks them. */
export function reviewItems(sections: readonly ReviewSection[]): ReviewItem[] {
  return sections.flatMap((section) => section.items);
}

// ---- Progress strip ------------------------------------------------------------------------

export type ProgressStrip = {
  total: ReadingStatusCounts;
  /** Per topic, counting Works under its subtopics too, in topic position order. */
  topics: { id: string; label: string; counts: ReadingStatusCounts }[];
  /** Works under no topic. */
  unfiled: ReadingStatusCounts;
};

/** read / reading / unread above every view; claims are not counted, they have no status. */
export function progressStrip(detail: ListDetail): ProgressStrip {
  const works = itemRows(detail).filter((row) => row.kind === "work");
  const topics = detail.nodes.filter((node) => node.kind === "topic" && accepted(node)).sort(byPosition);
  const children = new Map<string, string[]>();
  for (const edge of acceptedEdges(detail)) {
    if (edge.kind !== "subtopic") continue;
    const list = children.get(edge.from) ?? [];
    list.push(edge.to);
    children.set(edge.from, list);
  }
  const under = (topic: string): Set<string> => {
    const found = new Set<string>();
    const stack = [topic];
    for (let current = stack.pop(); current !== undefined; current = stack.pop()) {
      if (found.has(current)) continue;
      found.add(current);
      stack.push(...(children.get(current) ?? []));
    }
    return found;
  };
  return {
    total: statusCounts(works),
    topics: topics.map((topic) => {
      const ids = under(topic.id);
      return {
        id: topic.id,
        label: topic.label ?? UNTITLED,
        counts: statusCounts(works.filter((row) => row.topicIds.some((id) => ids.has(id)))),
      };
    }),
    unfiled: statusCounts(works.filter((row) => row.topicIds.length === 0)),
  };
}

/** `3 read · 1 reading · 5 unread`, the mono count line. */
export function countsLabel(counts: ReadingStatusCounts): string {
  return `${counts.read} read · ${counts.reading} reading · ${counts.unread} unread`;
}
