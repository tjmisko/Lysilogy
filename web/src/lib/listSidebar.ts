import type { ListPathStep, ListSummaryView } from "../types";

/**
 * The walk ancestors of the last Work in `path` that the list does not hold yet,
 * in path order. The Work itself is never listed. Repeated ids are listed once.
 * Topic and claim nodes carry no `work_id` and never count as held.
 */
export function missingAncestors(
  nodes: readonly { work_id: string | null }[],
  path: readonly string[],
): string[] {
  const held = new Set<string>();
  for (const node of nodes) {
    if (node.work_id !== null) held.add(node.work_id);
  }
  const missing: string[] = [];
  for (const id of path.slice(0, -1)) {
    if (held.has(id) || missing.includes(id)) continue;
    missing.push(id);
  }
  return missing;
}

const BREADCRUMB_ELLIPSIS = "…";
const UNTITLED = "Untitled";

/**
 * Titles of the Works the item was reached through (the path without the item
 * itself). More than `limit` entries keep the first and last ones and put an
 * ellipsis in the middle, so the result never has more than `limit` entries.
 */
export function pathBreadcrumb(path: readonly ListPathStep[], limit: number): string[] {
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

type PickableList = Pick<ListSummaryView, "id" | "updated_at" | "seed_work_id">;

/**
 * The remembered list when it is still listed, else the most recently updated list seeded at
 * `centerId`, else none. A list seeded elsewhere is never picked on its own: the sidebar of one
 * paper does not open an unrelated list.
 */
export function pickCurrentList<T extends PickableList>(
  lists: readonly T[],
  rememberedId: string | null,
  centerId: string,
): T | null {
  if (rememberedId !== null) {
    const remembered = lists.find((list) => list.id === rememberedId);
    if (remembered !== undefined) return remembered;
  }
  let latest: T | null = null;
  for (const list of lists) {
    if (list.seed_work_id !== centerId) continue;
    if (latest === null || list.updated_at > latest.updated_at) latest = list;
  }
  return latest;
}

/** Same key the ramp used, so remembered choices from before the migration keep working. */
export function listStorageKey(centerId: string): string {
  return `lysilogy.ramp.${centerId}`;
}

const DEFAULT_TITLE = "Reading list";
const TITLE_LIMIT = 100;

/** The title of a list started at a paper: "Reading list: <paper title>", kept within the title limit. */
export function walkListTitle(centerTitle: string | null | undefined): string {
  const title = (centerTitle ?? "").replace(/\s+/g, " ").trim();
  if (title === "") return DEFAULT_TITLE;
  const full = `${DEFAULT_TITLE}: ${title}`;
  const characters = Array.from(full);
  if (characters.length <= TITLE_LIMIT) return full;
  return `${characters.slice(0, TITLE_LIMIT - 1).join("").trimEnd()}…`;
}
