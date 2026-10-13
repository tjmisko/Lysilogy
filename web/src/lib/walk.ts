import type { ExpansionView, ProminenceView } from "../types";

/** Children shown for one expanded node (`limit` on the neighbors endpoint). */
export const EXPANSION_TOP_N = 8;
/** Expanded nodes allowed on the canvas at once. */
export const MAX_UI_EXPANSIONS = 6;
/** Deepest hop from the center that expansion may reach. */
export const MAX_UI_DEPTH = 3;

/** One expanded node. `depth` is the hop of its children (the parent's hop plus one). */
export type WalkExpansion = {
  parentId: string;
  childIds: string[];
  depth: number;
};

export type ExpandVerdict = "ok" | "too_deep" | "too_many";

/** Add (or replace) the expansion of `parentId`. Duplicate child ids collapse; the parent never lists itself. */
export function addExpansion(
  list: readonly WalkExpansion[],
  parentId: string,
  childIds: readonly string[],
  parentDepth: number,
): WalkExpansion[] {
  const unique = [...new Set(childIds)].filter((id) => id !== parentId);
  const next: WalkExpansion = { parentId, childIds: unique, depth: parentDepth + 1 };
  const existing = list.findIndex((expansion) => expansion.parentId === parentId);
  if (existing < 0) return [...list, next];
  return list.map((expansion, index) => (index === existing ? next : expansion));
}

/**
 * Remove the expansion of `parentId` and every expansion below it: one whose
 * parent is a child of a removed expansion and whose hop is exactly one deeper.
 */
export function collapseExpansion(list: readonly WalkExpansion[], parentId: string): WalkExpansion[] {
  const removed = new Map<string, number>();
  const root = list.find((expansion) => expansion.parentId === parentId);
  if (root === undefined) return [...list];
  removed.set(root.parentId, root.depth);
  const queue: WalkExpansion[] = [root];
  while (queue.length > 0) {
    const current = queue.shift();
    if (current === undefined) break;
    for (const expansion of list) {
      if (removed.has(expansion.parentId)) continue;
      if (expansion.depth !== current.depth + 1 || !current.childIds.includes(expansion.parentId)) continue;
      removed.set(expansion.parentId, expansion.depth);
      queue.push(expansion);
    }
  }
  return list.filter((expansion) => !removed.has(expansion.parentId));
}

/**
 * The walk from the center to `workId`, both included. A Work in `baseIds` (or
 * one no expansion lists) is one hop from the center. The center alone is `[center]`.
 */
export function walkPathTo(
  list: readonly WalkExpansion[],
  centerId: string,
  workId: string,
  baseIds: ReadonlySet<string> = new Set(),
): string[] {
  if (workId === centerId) return [centerId];
  const path = [workId];
  let current = workId;
  for (let step = 0; step <= MAX_UI_DEPTH; step += 1) {
    if (baseIds.has(current)) break;
    const parent = list.find((expansion) => expansion.childIds.includes(current) && expansion.parentId !== current);
    if (parent === undefined || path.includes(parent.parentId)) break;
    path.unshift(parent.parentId);
    current = parent.parentId;
  }
  if (path[0] !== centerId) path.unshift(centerId);
  return path;
}

/** Whether `workId` may be expanded. `depthOf` gives its hop from the center (0 for the center). */
export function canExpand(
  list: readonly WalkExpansion[],
  workId: string,
  depthOf: (workId: string) => number,
): ExpandVerdict {
  const expanded = list.some((expansion) => expansion.parentId === workId);
  if (expanded) return "ok";
  if (depthOf(workId) + 1 > MAX_UI_DEPTH) return "too_deep";
  if (list.length >= MAX_UI_EXPANSIONS) return "too_many";
  return "ok";
}

/** True when the server already holds this Work's references, so no expand job is needed. */
export function fanoutKnown(view: ExpansionView | undefined, prominence?: ProminenceView): boolean {
  if (view !== undefined) return view.state === "expanded" || view.state === "known";
  return prominence !== undefined && prominence.signals.cited_works_count > 0;
}
