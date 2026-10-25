// Client-side mirror of crates/lysilogy-core/src/list_graph.rs, for optimistic UI only.
// The server stays the authority: it re-validates every edge and orders every strategy.
// Ids are compared as plain strings; the server compares UUIDs, so ties among ids may
// resolve differently, which only shows up as a reorder after the next fetch.

import type { ListEdgeKind, ListNodeKind, ReadingStatus } from "../types";

export type GraphEdge = { from: string; to: string; kind: ListEdgeKind };

export type EdgeRejection =
  | { reason: "invalid_edge_kind" }
  | { reason: "node_not_found" }
  /** The nodes in cycle order, starting at the new edge's source; a self edge is `[from]`. */
  | { reason: "edge_cycle"; cycle: string[] };

export type EdgeCheck = { ok: true } | ({ ok: false } & EdgeRejection);

export type LinearItem = {
  id: string;
  /** null for claims, which take no part in citation constraints. */
  workId: string | null;
  depth: number;
  score: number;
  year: number | null;
  titleKey: string;
  isSeed: boolean;
};

function compareText(left: string, right: string): number {
  if (left < right) return -1;
  return left > right ? 1 : 0;
}

function compareNullableText(left: string | null, right: string | null): number {
  if (left === right) return 0;
  if (left === null) return -1;
  if (right === null) return 1;
  return compareText(left, right);
}

const isOrdering = (kind: ListEdgeKind): boolean => kind === "prerequisite" || kind === "subtopic";

function kindsFit(kind: ListEdgeKind, from: ListNodeKind, to: ListNodeKind): boolean {
  if (kind === "covers") return from === "topic" && to === "work";
  if (kind === "subtopic") return from === "topic" && to === "topic";
  // Claims are unverified, so nothing may depend on them yet.
  return from !== "claim" && to !== "claim";
}

function sortedSet(values: Iterable<string>): string[] {
  return [...new Set(values)].sort(compareText);
}

/** Direct `covers` and `subtopic` children of every topic. */
function coveredBy(edges: readonly GraphEdge[]): Map<string, Set<string>> {
  const covered = new Map<string, Set<string>>();
  for (const edge of edges) {
    if (edge.kind !== "covers" && edge.kind !== "subtopic") continue;
    const children = covered.get(edge.from) ?? new Set<string>();
    children.add(edge.to);
    covered.set(edge.from, children);
  }
  return covered;
}

/** The nodes a node stands for: itself when it is a leaf, else every leaf under a topic. */
function members(isLeaf: (id: string) => boolean, covered: ReadonlyMap<string, ReadonlySet<string>>, node: string): Set<string> {
  const found = new Set<string>();
  const seen = new Set<string>();
  const stack = [node];
  for (let current = stack.pop(); current !== undefined; current = stack.pop()) {
    if (seen.has(current)) continue;
    seen.add(current);
    if (isLeaf(current)) {
      found.add(current);
      continue;
    }
    stack.push(...(covered.get(current) ?? []));
  }
  return found;
}

/** The shortest cycle the new prerequisite/subtopic edge closes among the stored ones. */
function rawCycle(existing: readonly GraphEdge[], edge: GraphEdge): string[] | null {
  const successors = new Map<string, Set<string>>();
  for (const stored of existing) {
    if (!isOrdering(stored.kind)) continue;
    const next = successors.get(stored.from) ?? new Set<string>();
    next.add(stored.to);
    successors.set(stored.from, next);
  }
  const parent = new Map<string, string>();
  const seen = new Set([edge.to]);
  const queue = [edge.to];
  for (let head = 0; head < queue.length; head += 1) {
    const node = queue[head];
    if (node === undefined) break;
    if (node === edge.from) {
      const path = [node];
      for (let cursor = parent.get(node); cursor !== undefined; cursor = parent.get(cursor)) path.push(cursor);
      path.reverse();
      return [edge.from, ...path.slice(0, -1)];
    }
    for (const successor of sortedSet(successors.get(node) ?? [])) {
      if (seen.has(successor)) continue;
      seen.add(successor);
      parent.set(successor, node);
      queue.push(successor);
    }
  }
  return null;
}

/** Work-to-Work "read first" relation: prerequisite edges expanded through covers and subtopic. */
function workOrder(
  isWork: (id: string) => boolean,
  edges: readonly GraphEdge[],
): Map<string, Set<string>> {
  const covered = coveredBy(edges);
  const order = new Map<string, Set<string>>();
  for (const edge of edges) {
    if (edge.kind !== "prerequisite") continue;
    const after = members(isWork, covered, edge.to);
    for (const first of members(isWork, covered, edge.from)) {
      const targets = order.get(first) ?? new Set<string>();
      for (const second of after) if (second !== first) targets.add(second);
      order.set(first, targets);
    }
  }
  return order;
}

/** A cycle in a Work order starting at its smallest node, or null when it is acyclic. */
function findWorkCycle(order: ReadonlyMap<string, ReadonlySet<string>>): string[] | null {
  const done = new Set<string>();
  const visit = (node: string, path: string[]): string[] | null => {
    const at = path.indexOf(node);
    if (at >= 0) {
      const cycle = path.slice(at);
      let smallest = 0;
      cycle.forEach((id, index) => {
        if (compareText(id, cycle[smallest] ?? id) < 0) smallest = index;
      });
      return [...cycle.slice(smallest), ...cycle.slice(0, smallest)];
    }
    if (done.has(node)) return null;
    path.push(node);
    for (const next of sortedSet(order.get(node) ?? [])) {
      const found = visit(next, path);
      if (found !== null) return found;
    }
    path.pop();
    done.add(node);
    return null;
  };
  for (const start of sortedSet(order.keys())) {
    const found = visit(start, []);
    if (found !== null) return found;
  }
  return null;
}

/**
 * Check a new edge against the list (D8). `nodes` maps every node to its kind.
 * prerequisite and subtopic edges must stay acyclic, also once topics are expanded to their
 * Works. An edge that is already stored is accepted.
 */
export function validateEdge(
  nodes: ReadonlyMap<string, ListNodeKind>,
  existing: readonly GraphEdge[],
  edge: GraphEdge,
): EdgeCheck {
  const fromKind = nodes.get(edge.from);
  const toKind = nodes.get(edge.to);
  if (fromKind === undefined || toKind === undefined) return { ok: false, reason: "node_not_found" };
  if (!kindsFit(edge.kind, fromKind, toKind)) return { ok: false, reason: "invalid_edge_kind" };
  if (edge.from === edge.to && isOrdering(edge.kind)) return { ok: false, reason: "edge_cycle", cycle: [edge.from] };
  if (isOrdering(edge.kind)) {
    const cycle = rawCycle(existing, edge);
    if (cycle !== null) return { ok: false, reason: "edge_cycle", cycle };
  }
  const isWork = (id: string): boolean => nodes.get(id) === "work";
  const expanded = findWorkCycle(workOrder(isWork, [...existing, edge]));
  if (expanded === null) return { ok: true };
  // A list whose stored edges already cycle accepts edges that add nothing new.
  if (findWorkCycle(workOrder(isWork, existing)) !== null) return { ok: true };
  return { ok: false, reason: "edge_cycle", cycle: expanded };
}

/** Higher priority sorts first: deeper, more prominent, older (unknown last), then ids. */
function priority(left: LinearItem, right: LinearItem): number {
  if (left.depth !== right.depth) return right.depth - left.depth;
  if (left.score !== right.score) return right.score - left.score;
  if (left.year !== right.year) {
    if (left.year === null) return 1;
    if (right.year === null) return -1;
    return left.year - right.year;
  }
  return compareText(left.titleKey, right.titleKey)
    || compareNullableText(left.workId, right.workId)
    || compareText(left.id, right.id);
}

function bestOf(items: readonly LinearItem[], pool: readonly number[]): number {
  let best = pool[0] ?? 0;
  for (const index of pool) {
    const candidate = items[index];
    const current = items[best];
    if (candidate !== undefined && current !== undefined && priority(candidate, current) < 0) best = index;
  }
  return best;
}

/** Each item's grouping topic: the directly covering topic with the most items, then the smallest id. */
function primaryTopics(body: readonly LinearItem[], edges: readonly GraphEdge[]): (string | null)[] {
  const indexOf = new Map(body.map((item, index) => [item.id, index]));
  const covering: Set<string>[] = body.map(() => new Set<string>());
  const size = new Map<string, number>();
  for (const edge of edges) {
    if (edge.kind !== "covers") continue;
    const index = indexOf.get(edge.to);
    const topics = index === undefined ? undefined : covering[index];
    if (topics === undefined || topics.has(edge.from)) continue;
    topics.add(edge.from);
    size.set(edge.from, (size.get(edge.from) ?? 0) + 1);
  }
  return covering.map((topics) => {
    let best: string | null = null;
    for (const topic of sortedSet(topics)) {
      if (best === null || (size.get(topic) ?? 0) > (size.get(best) ?? 0)) best = topic;
    }
    return best;
  });
}

/**
 * The topological linearization of Work and claim nodes (D9): prerequisite edges first (a
 * topic endpoint stands for every Work under it), then derived citations (`cites` holds
 * `[citing, cited]`; the cited Work first), topic contiguity where free, ties by priority.
 * The seed comes last. When only cycles remain, citations are dropped first, then the best
 * remaining item is taken. A repeated item id keeps its first entry.
 */
export function linearize(
  items: readonly LinearItem[],
  edges: readonly GraphEdge[],
  cites: readonly (readonly [string, string])[],
): string[] {
  const seenIds = new Set<string>();
  const unique = items.filter((item) => {
    if (seenIds.has(item.id)) return false;
    seenIds.add(item.id);
    return true;
  });
  const seeds = unique.filter((item) => item.isSeed);
  const body = unique.filter((item) => !item.isSeed);

  const indexOf = new Map(body.map((item, index) => [item.id, index]));
  const covered = coveredBy(edges);
  const leafIndices = (node: string): number[] => {
    const found = new Set<number>();
    const seen = new Set<string>();
    const stack = [node];
    for (let current = stack.pop(); current !== undefined; current = stack.pop()) {
      if (seen.has(current)) continue;
      seen.add(current);
      const index = indexOf.get(current);
      if (index !== undefined) {
        found.add(index);
        continue;
      }
      stack.push(...(covered.get(current) ?? []));
    }
    return [...found];
  };

  const unmetHard = body.map(() => 0);
  const unmetSoft = body.map(() => 0);
  const hardDependants: number[][] = body.map(() => []);
  const softDependants: number[][] = body.map(() => []);
  const hardPairs = new Set<string>();
  for (const edge of edges) {
    if (edge.kind !== "prerequisite") continue;
    const before = leafIndices(edge.from);
    const after = leafIndices(edge.to);
    for (const first of before) {
      for (const second of after) {
        if (first === second || hardPairs.has(`${first}:${second}`)) continue;
        hardPairs.add(`${first}:${second}`);
        unmetHard[second] = (unmetHard[second] ?? 0) + 1;
        hardDependants[first]?.push(second);
      }
    }
  }
  const byWork = new Map<string, number>();
  body.forEach((item, index) => {
    if (item.workId !== null && !byWork.has(item.workId)) byWork.set(item.workId, index);
  });
  const softPairs = new Set<string>();
  for (const [citing, cited] of cites) {
    const citingIndex = byWork.get(citing);
    const citedIndex = byWork.get(cited);
    if (citingIndex === undefined || citedIndex === undefined || citingIndex === citedIndex) continue;
    if (softPairs.has(`${citedIndex}:${citingIndex}`)) continue;
    softPairs.add(`${citedIndex}:${citingIndex}`);
    unmetSoft[citingIndex] = (unmetSoft[citingIndex] ?? 0) + 1;
    softDependants[citedIndex]?.push(citingIndex);
  }

  const primary = primaryTopics(body, edges);
  let remaining = body.map((_, index) => index);
  const ordered: string[] = [];
  let lastTopic: string | null = null;
  while (remaining.length > 0) {
    let pool = remaining.filter((index) => unmetHard[index] === 0 && unmetSoft[index] === 0);
    if (pool.length === 0) pool = remaining.filter((index) => unmetHard[index] === 0);
    if (pool.length === 0) pool = remaining;
    if (lastTopic !== null) {
      const same = pool.filter((index) => primary[index] === lastTopic);
      if (same.length > 0) pool = same;
    }
    const pick = bestOf(body, pool);
    remaining = remaining.filter((index) => index !== pick);
    const picked = body[pick];
    if (picked !== undefined) ordered.push(picked.id);
    lastTopic = primary[pick] ?? null;
    for (const dependant of hardDependants[pick] ?? []) unmetHard[dependant] = Math.max(0, (unmetHard[dependant] ?? 0) - 1);
    for (const dependant of softDependants[pick] ?? []) unmetSoft[dependant] = Math.max(0, (unmetSoft[dependant] ?? 0) - 1);
  }
  seeds.sort(priority);
  return [...ordered, ...seeds.map((item) => item.id)];
}

/** For every Work, the unread-or-reading Works that must be read before it, keyed by node id. */
export function blockersByWork(
  works: readonly { id: string; status: ReadingStatus }[],
  edges: readonly GraphEdge[],
): Map<string, Set<string>> {
  const status = new Map(works.map((work) => [work.id, work.status]));
  const covered = coveredBy(edges);
  const isWork = (id: string): boolean => status.has(id);
  const blockers = new Map<string, Set<string>>();
  for (const edge of edges) {
    if (edge.kind !== "prerequisite") continue;
    const unread = [...members(isWork, covered, edge.from)].filter((id) => status.get(id) !== "read");
    for (const target of members(isWork, covered, edge.to)) {
      const found = blockers.get(target) ?? new Set<string>();
      for (const id of unread) if (id !== target) found.add(id);
      blockers.set(target, found);
    }
  }
  return blockers;
}

/** The unread-or-reading Works that must be read before `target`, in id order. */
export function unmetPrerequisites(
  works: readonly { id: string; status: ReadingStatus }[],
  edges: readonly GraphEdge[],
  target: string,
): string[] {
  return sortedSet(blockersByWork(works, edges).get(target) ?? []);
}

/**
 * "Next up": unread Works whose prerequisites are all read, in the order of `order`
 * (a linearization; Works missing from it go last by id).
 */
export function frontier(
  works: readonly { id: string; status: ReadingStatus }[],
  edges: readonly GraphEdge[],
  order: readonly string[],
): string[] {
  const rank = new Map<string, number>();
  order.forEach((id, index) => {
    if (!rank.has(id)) rank.set(id, index);
  });
  const blockers = blockersByWork(works, edges);
  const ready = sortedSet(
    works.filter((work) => work.status === "unread" && (blockers.get(work.id)?.size ?? 0) === 0).map((work) => work.id),
  );
  return ready.sort((a, b) => (rank.get(a) ?? Infinity) - (rank.get(b) ?? Infinity) || compareText(a, b));
}
