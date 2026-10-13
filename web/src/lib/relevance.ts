import type { NeighborCluster, ProminenceView, WorkMarkName } from "../types";

export const DEFAULT_TOP_N = 40;
export const TOP_N_STEP = 20;
export const TOP_N_MIN = 10;
export const TOP_N_MAX = 200;

export type NeighborOptions = {
  limit?: number;
  expand?: Iterable<string>;
  tags?: Iterable<string>;
};

/** Step the top-N by `delta` steps of `TOP_N_STEP`, clamped to the allowed range. */
export function nextTopN(current: number, delta: number): number {
  const stepped = current + delta * TOP_N_STEP;
  return Math.min(TOP_N_MAX, Math.max(TOP_N_MIN, stepped));
}

/** Pressing the active mark clears it; any other mark replaces it. */
export function markAfterToggle(current: WorkMarkName | null, requested: WorkMarkName): WorkMarkName | null {
  return current === requested ? null : requested;
}

/** Dot radius for a prominence score. Scores outside 0..1 and NaN are clamped. */
export function nodeRadius(score: number): number {
  const clamped = Number.isNaN(score) ? 0 : Math.min(1, Math.max(0, score));
  return 3.5 + 3.5 * clamped;
}

function sortedUnique(values: Iterable<string> | undefined): string[] {
  if (values === undefined) return [];
  const cleaned = new Set<string>();
  for (const value of values) {
    if (value !== "") cleaned.add(value);
  }
  return [...cleaned].sort();
}

/** The query string (without `?`) for the neighbors endpoint. Empty parameters are omitted. */
export function neighborQuery(options: NeighborOptions & { direction?: string }): string {
  const params = new URLSearchParams();
  if (options.direction !== undefined && options.direction !== "") params.set("direction", options.direction);
  if (options.limit !== undefined && Number.isFinite(options.limit)) params.set("limit", String(Math.trunc(options.limit)));
  const expand = sortedUnique(options.expand);
  if (expand.length > 0) params.set("expand", expand.join(","));
  const tags = sortedUnique(options.tags);
  if (tags.length > 0) params.set("tags", tags.join(","));
  return params.toString();
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

/** One line describing why a Work ranks where it does. Unknown signals are left out. */
export function signalSummary(prominence: ProminenceView): string {
  const { signals } = prominence;
  const parts: string[] = [`cited by ${signals.citing_works_count} here`];
  if (signals.cited_by_global_count !== null) {
    parts.push(`${signals.cited_by_global_count.toLocaleString("en-US")} overall`);
  }
  if (signals.mention_count > 0) parts.push(plural(signals.mention_count, "mention"));
  if (signals.method_citation_count > 0) parts.push("method citation");
  else if (signals.background_citation_count > 0) parts.push("background citation");
  if (signals.local_available) parts.push("held locally");
  if (signals.display_year !== null) parts.push(String(signals.display_year));
  return parts.join(" · ");
}

/** Screen reader label for a cluster node. */
export function clusterAriaLabel(cluster: Pick<NeighborCluster, "label" | "count">): string {
  return `${cluster.label}: ${plural(cluster.count, "more work")}, press Enter to expand`;
}

/** A copy of `set` with `value` added, or removed when already present. */
export function toggleInSet<T>(set: ReadonlySet<T>, value: T): Set<T> {
  const next = new Set(set);
  if (!next.delete(value)) next.add(value);
  return next;
}
