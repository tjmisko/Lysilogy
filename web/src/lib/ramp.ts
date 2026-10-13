import type { RampItemStatus, RampPathStep, RampSummaryView } from "../types";

const STATUS_CYCLE: readonly RampItemStatus[] = ["unread", "reading", "read"];

/** The status a click on the status button moves to: unread, reading, read, then unread again. */
export function nextStatus(status: RampItemStatus): RampItemStatus {
  const index = Math.max(0, STATUS_CYCLE.indexOf(status));
  return STATUS_CYCLE[(index + 1) % STATUS_CYCLE.length] ?? "unread";
}

/**
 * The walk ancestors of the last Work in `path` that the ramp does not hold yet,
 * in path order. The Work itself is never listed. Repeated ids are listed once.
 */
export function missingAncestors(items: readonly { work: { id: string } }[], path: readonly string[]): string[] {
  const held = new Set(items.map((item) => item.work.id));
  const missing: string[] = [];
  for (const id of path.slice(0, -1)) {
    if (held.has(id) || missing.includes(id)) continue;
    missing.push(id);
  }
  return missing;
}

/** The order after moving the item at `index` by `delta` places. A move past either end changes nothing. */
export function swapOrder(ids: readonly string[], index: number, delta: number): string[] {
  const target = index + delta;
  const unchanged = [...ids];
  if (!Number.isInteger(index) || !Number.isInteger(target)) return unchanged;
  if (index < 0 || index >= ids.length || target < 0 || target >= ids.length) return unchanged;
  const moved = [...ids];
  const [item] = moved.splice(index, 1);
  if (item === undefined) return unchanged;
  moved.splice(target, 0, item);
  return moved;
}

const BREADCRUMB_ELLIPSIS = "…";
const UNTITLED = "Untitled";

/**
 * Titles of the Works the item was reached through (the path without the item
 * itself). More than `limit` entries keep the first and last ones and put an
 * ellipsis in the middle, so the result never has more than `limit` entries.
 */
export function pathBreadcrumb(path: readonly RampPathStep[], limit: number): string[] {
  const titles = path.slice(0, -1).map((step) => {
    const title = step.title?.trim() ?? "";
    return title === "" ? UNTITLED : title;
  });
  const room = Math.max(1, Math.floor(limit));
  if (titles.length <= room) return titles;
  const head = Math.ceil((room - 1) / 2);
  const tail = Math.floor((room - 1) / 2);
  return [...titles.slice(0, head), BREADCRUMB_ELLIPSIS, ...(tail === 0 ? [] : titles.slice(-tail))];
}

/** The remembered ramp when it is still listed, else the most recently updated, else none. */
export function pickCurrentRamp(
  ramps: readonly RampSummaryView[],
  rememberedId: string | null,
): RampSummaryView | null {
  if (rememberedId !== null) {
    const remembered = ramps.find((ramp) => ramp.id === rememberedId);
    if (remembered !== undefined) return remembered;
  }
  let latest: RampSummaryView | null = null;
  for (const ramp of ramps) {
    if (latest === null || ramp.updated_at > latest.updated_at) latest = ramp;
  }
  return latest;
}

export function rampStorageKey(centerId: string): string {
  return `lysilogy.ramp.${centerId}`;
}
