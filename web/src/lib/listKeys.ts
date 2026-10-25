// Key sequences, reorder helpers and small pure steps behind the list page (spec: UI, Keymap).

import type { ListEdgeKind, ListNodeView, ReadingStatus } from "../types";

// ---- Key sequences -------------------------------------------------------------------------

export type SequenceAction =
  | "top" // gg
  | "cut" // dd
  | "fold-open" // zo
  | "fold-close" // zc
  | "fold-toggle" // za
  | "fold-open-all" // zR
  | "fold-close-all" // zM
  | "group-toggle"; // zg

const SEQUENCES: Readonly<Record<string, SequenceAction>> = {
  gg: "top",
  dd: "cut",
  zo: "fold-open",
  zc: "fold-close",
  za: "fold-toggle",
  zR: "fold-open-all",
  zM: "fold-close-all",
  zg: "group-toggle",
};

/** How long a half-typed sequence waits for its second key. */
export const SEQUENCE_TIMEOUT_MS = 800;

const PREFIXES: ReadonlySet<string> = new Set(["g", "d", "z"]);

export type SequenceStep = {
  action: SequenceAction | null;
  /** The keys typed so far that still await a second. */
  pending: string;
  /** The key belongs to a sequence (or ended one); single-key handlers must not see it. */
  consumed: boolean;
};

/**
 * Feed one key to the sequence matcher. A second key that completes nothing cancels the
 * sequence and is swallowed, as in vim. `g` is also the app-level `g l` / `g r` prefix, which the
 * app consumes before the page sees the second key.
 */
export function stepSequence(pending: string, key: string): SequenceStep {
  const candidate = pending + key;
  if (pending !== "" && Object.hasOwn(SEQUENCES, candidate)) {
    return { action: SEQUENCES[candidate] ?? null, pending: "", consumed: true };
  }
  if (pending !== "") return { action: null, pending: "", consumed: true };
  if (PREFIXES.has(key)) return { action: null, pending: key, consumed: true };
  return { action: null, pending: "", consumed: false };
}

// ---- Reading status ------------------------------------------------------------------------

const STATUS_CYCLE: readonly ReadingStatus[] = ["unread", "reading", "read"];

/** `r`: unread, reading, read, then unread again. */
export function nextReadingStatus(status: ReadingStatus): ReadingStatus {
  const index = Math.max(0, STATUS_CYCLE.indexOf(status));
  return STATUS_CYCLE[(index + 1) % STATUS_CYCLE.length] ?? "unread";
}

// ---- Reorder (dd / p / P / J / K) ----------------------------------------------------------

/** `order` with `id` moved next to `target`. Unknown ids or `id === target` change nothing. */
export function moveRelative(
  order: readonly string[],
  id: string,
  target: string,
  where: "before" | "after",
): string[] {
  const unchanged = [...order];
  if (id === target || !order.includes(id) || !order.includes(target)) return unchanged;
  const rest = order.filter((entry) => entry !== id);
  const at = rest.indexOf(target) + (where === "after" ? 1 : 0);
  rest.splice(at, 0, id);
  return rest;
}

/**
 * `J` / `K`: move `id` one row down (+1) or up (-1) among the visible `rowIds`, leaving
 * the rest of the full `order` (topics, proposed nodes) where it is. Past either end changes nothing.
 */
export function stepAmongRows(
  order: readonly string[],
  rowIds: readonly string[],
  id: string,
  delta: -1 | 1,
): string[] {
  const at = rowIds.indexOf(id);
  const neighbor = at < 0 ? undefined : rowIds[at + delta];
  if (neighbor === undefined) return [...order];
  return moveRelative(order, id, neighbor, delta > 0 ? "after" : "before");
}

/** True when the two orders are the same sequence. */
export function sameOrder(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((id, index) => id === right[index]);
}

/** Every node id in manual position order, ties broken by id; the permutation PUT order expects. */
export function manualOrder(nodes: readonly Pick<ListNodeView, "id" | "position">[]): string[] {
  return [...nodes]
    .sort((left, right) => left.position - right.position || (left.id < right.id ? -1 : left.id > right.id ? 1 : 0))
    .map((node) => node.id);
}

/**
 * The permutation that puts a restored node back where it was removed from: before the first
 * other node whose position is past `position`, else last. `others` must not contain `restoredId`.
 */
export function orderAfterRestore(
  others: readonly Pick<ListNodeView, "id" | "position">[],
  position: number,
  restoredId: string,
): string[] {
  const ordered = manualOrder(others.filter((node) => node.id !== restoredId));
  const positions = new Map(others.map((node) => [node.id, node.position]));
  const at = ordered.findIndex((id) => (positions.get(id) ?? 0) > position);
  if (at < 0) return [...ordered, restoredId];
  ordered.splice(at, 0, restoredId);
  return ordered;
}

// ---- Selection movement --------------------------------------------------------------------

/** The id `delta` rows from `current` in `ids`, clamped to the ends; the first id when `current` is unknown. */
export function moveSelection(ids: readonly string[], current: string | null, delta: number): string | null {
  if (ids.length === 0) return null;
  const at = current === null ? -1 : ids.indexOf(current);
  if (at < 0) return ids[delta < 0 ? ids.length - 1 : 0] ?? null;
  return ids[Math.max(0, Math.min(ids.length - 1, at + delta))] ?? null;
}

/** The selection after `id` leaves `ids`: the row that took its place, else the one above. */
export function selectionAfterRemoval(ids: readonly string[], removed: string): string | null {
  const at = ids.indexOf(removed);
  if (at < 0) return ids[0] ?? null;
  return ids[at + 1] ?? ids[at - 1] ?? null;
}

// ---- Edges ---------------------------------------------------------------------------------

export type TopicChange = { add: string[]; remove: string[] };

/** The covers edges to add and remove to turn the `current` topic set into `wanted`. */
export function topicChange(current: Iterable<string>, wanted: Iterable<string>): TopicChange {
  const had = new Set(current);
  const want = new Set(wanted);
  return {
    add: [...want].filter((id) => !had.has(id)),
    remove: [...had].filter((id) => !want.has(id)),
  };
}

/** One edge of a node as the `X` picker words it. */
export function describeEdge(
  edge: { from: string; to: string; kind: ListEdgeKind },
  selfId: string,
  titleOf: (nodeId: string) => string,
): string {
  const outgoing = edge.from === selfId;
  const other = titleOf(outgoing ? edge.to : edge.from);
  switch (edge.kind) {
    case "prerequisite": return outgoing ? `prerequisite of ${other}` : `needs ${other} first`;
    case "covers": return outgoing ? `covers ${other}` : `under topic ${other}`;
    case "subtopic": return outgoing ? `has subtopic ${other}` : `subtopic of ${other}`;
  }
}

// ---- Errors --------------------------------------------------------------------------------

export type ErrorLike = { message: string; code?: string | null; cycle?: readonly string[] | null };

/** The server's list error codes in the words the status line shows; unknown codes pass the server's message through. */
export function listErrorMessage(error: ErrorLike, titleOf: (nodeId: string) => string = (id) => id): string {
  switch (error.code) {
    case "item_exists": return "Already on this list.";
    case "list_full": return "This list is full (500 items).";
    case "via_not_in_list": return "The paper it was reached through is not on this list.";
    case "no_citation_edge": return "No citation connects those two papers.";
    case "paper_not_linked": return "This paper is not linked to a Work yet. Open Sources (L) to link it.";
    case "edge_cycle": {
      const names = (error.cycle ?? []).map((id) => titleOf(id));
      return names.length === 0 ? "That would close a cycle." : `That would close a cycle: ${[...names, names[0]].join(" → ")}.`;
    }
    default: return error.message;
  }
}
