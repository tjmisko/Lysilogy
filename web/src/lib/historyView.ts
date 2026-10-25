// The projection behind the history page (spec: UI, History page): sessions grouped by day,
// the rows `j/k` walk, kind filters and the label of each event. Pure, so Node tests drive it.

import type { HistoryEventKind, HistoryEventView, HistorySession } from "../types";

export type KindGroup = {
  /** The key that toggles the group. */
  key: string;
  label: string;
  kinds: readonly HistoryEventKind[];
};

export const KIND_GROUPS: readonly KindGroup[] = [
  { key: "1", label: "papers", kinds: ["paper_opened", "paper_left", "view_changed", "page_reached", "link_followed", "question_asked", "digest_opened"] },
  { key: "2", label: "sections", kinds: ["section_focused", "figure_viewed"] },
  { key: "3", label: "searches", kinds: ["search_run"] },
  { key: "4", label: "graph", kinds: ["graph_opened", "work_inspected", "fetch_requested"] },
  { key: "5", label: "lists", kinds: ["list_opened", "list_changed", "list_generated"] },
];

/** The kinds to ask the server for; empty when no group is on, which means every kind. */
export function kindsForGroups(active: ReadonlySet<string>): HistoryEventKind[] {
  return KIND_GROUPS.filter((group) => active.has(group.key)).flatMap((group) => [...group.kinds]);
}

/** Toggle a group key; keys that name no group leave the set as it was. */
export function toggleGroup(active: ReadonlySet<string>, key: string): Set<string> {
  const next = new Set(active);
  if (!KIND_GROUPS.some((group) => group.key === key)) return next;
  if (!next.delete(key)) next.add(key);
  return next;
}

const pad = (value: number): string => String(value).padStart(2, "0");

/** The local calendar day of a timestamp, `YYYY-MM-DD`; "" for one that does not parse. */
export function dayKey(timestamp: string): string {
  const at = new Date(timestamp);
  if (Number.isNaN(at.getTime())) return "";
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

/** Local `HH:MM`; "" for a timestamp that does not parse. */
export function clockTime(timestamp: string): string {
  const at = new Date(timestamp);
  if (Number.isNaN(at.getTime())) return "";
  return `${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;

/** "Today", "Yesterday", or "Sat 24 Oct 2026". */
export function dayLabel(day: string, now: Date): string {
  if (day === "") return "Unknown day";
  if (day === dayKey(now.toISOString())) return "Today";
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1, 12);
  if (day === dayKey(yesterday.toISOString())) return "Yesterday";
  const [year, month, date] = day.split("-").map(Number);
  const at = new Date(year ?? 0, (month ?? 1) - 1, date ?? 1, 12);
  return `${WEEKDAYS[at.getDay()] ?? ""} ${at.getDate()} ${MONTHS[at.getMonth()] ?? ""} ${at.getFullYear()}`;
}

/** "42s", "4m 12s", "1h 05m". Negative or non-finite values read as "0s". */
export function formatDwell(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms <= 0) return "0s";
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${pad(seconds % 60)}s`;
  return `${Math.floor(minutes / 60)}h ${pad(minutes % 60)}m`;
}

export type TitleOf = (paperId: string) => string | null;

function detailText(event: HistoryEventView, key: string): string | null {
  const value = event.detail[key];
  return typeof value === "string" && value.trim() !== "" ? value : null;
}

function detailNumber(event: HistoryEventView, key: string): number | null {
  const value = event.detail[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function paperName(event: HistoryEventView, titleOf: TitleOf): string {
  if (event.paper_id === null) return "a paper";
  return titleOf(event.paper_id) ?? event.paper_id;
}

function withDwell(text: string, event: HistoryEventView): string {
  return event.dwell_ms === null ? text : `${text} · ${formatDwell(event.dwell_ms)}`;
}

/** One line for an event. */
export function describeEvent(event: HistoryEventView, titleOf: TitleOf): string {
  const page = detailNumber(event, "page");
  switch (event.kind) {
    case "paper_opened": return `Opened ${paperName(event, titleOf)}`;
    case "paper_left": return withDwell(`Left ${paperName(event, titleOf)}`, event);
    case "view_changed": return `Switched to the ${detailText(event, "view") ?? "other"} view`;
    case "section_focused": return withDwell(`Read section ${detailText(event, "title") ?? detailText(event, "section_id") ?? ""}`.trimEnd(), event);
    case "figure_viewed": return withDwell(`Viewed ${detailText(event, "figure") ?? "a figure"}${page === null ? "" : ` on page ${page}`}`, event);
    case "page_reached": return `Reached page ${page ?? "?"}`;
    case "search_run": return `Searched “${detailText(event, "query") ?? ""}”`;
    case "question_asked": return `Asked${page === null ? "" : ` on page ${page}`}: “${detailText(event, "question") ?? ""}”`;
    case "link_followed": return `Followed ${detailText(event, "label") ?? "a link"}`;
    case "digest_opened": return "Opened the section digest";
    case "graph_opened": return `Opened the literature graph of ${paperName(event, titleOf)}`;
    case "work_inspected": return withDwell(`Inspected ${event.work_title ?? "a work"}`, event);
    case "list_opened": return "Opened a list";
    case "list_changed": return "Changed a list";
    case "list_generated": return "Generated a list";
    case "fetch_requested": return `Requested a copy of ${event.work_title ?? "a work"}`;
  }
}

/** Newest first, ties by id. */
function newestFirst(left: HistoryEventView, right: HistoryEventView): number {
  const byTime = Date.parse(right.occurred_at) - Date.parse(left.occurred_at);
  if (Number.isFinite(byTime) && byTime !== 0) return byTime;
  return right.id - left.id;
}

/**
 * Add a later page of sessions to what is shown. A session that continues across pages is
 * merged, its events deduplicated by id; sessions end up newest first.
 */
export function mergeSessions(shown: readonly HistorySession[], more: readonly HistorySession[]): HistorySession[] {
  const byId = new Map<string, HistorySession>();
  for (const session of [...shown, ...more]) {
    const known = byId.get(session.session_id);
    if (known === undefined) {
      byId.set(session.session_id, { ...session, events: [...session.events].sort(newestFirst) });
      continue;
    }
    const events = new Map(known.events.map((event) => [event.id, event]));
    for (const event of session.events) events.set(event.id, event);
    byId.set(session.session_id, {
      session_id: known.session_id,
      started_at: Date.parse(session.started_at) < Date.parse(known.started_at) ? session.started_at : known.started_at,
      ended_at: Date.parse(session.ended_at) > Date.parse(known.ended_at) ? session.ended_at : known.ended_at,
      events: [...events.values()].sort(newestFirst),
    });
  }
  return [...byId.values()].sort((left, right) => Date.parse(right.ended_at) - Date.parse(left.ended_at));
}

export type PaperDwell = { paperId: string; dwellMs: number };

/** The papers a session touched, most dwelt first, ties by first appearance (oldest event first). */
export function sessionPapers(session: HistorySession): PaperDwell[] {
  const order: string[] = [];
  const dwell = new Map<string, number>();
  for (const event of [...session.events].reverse()) {
    if (event.paper_id === null) continue;
    if (!dwell.has(event.paper_id)) {
      dwell.set(event.paper_id, 0);
      order.push(event.paper_id);
    }
    if (event.kind === "paper_left") dwell.set(event.paper_id, (dwell.get(event.paper_id) ?? 0) + (event.dwell_ms ?? 0));
  }
  return order
    .map((paperId) => ({ paperId, dwellMs: dwell.get(paperId) ?? 0 }))
    .sort((left, right) => right.dwellMs - left.dwellMs || order.indexOf(left.paperId) - order.indexOf(right.paperId));
}

/** "2 papers · 14m 03s · 1 search" */
export function sessionSummary(session: HistorySession): string {
  const papers = sessionPapers(session);
  const dwell = papers.reduce((sum, paper) => sum + paper.dwellMs, 0);
  const searches = session.events.filter((event) => event.kind === "search_run").length;
  const parts: string[] = [];
  if (papers.length > 0) parts.push(`${papers.length} ${papers.length === 1 ? "paper" : "papers"}`);
  if (dwell > 0) parts.push(formatDwell(dwell));
  if (searches > 0) parts.push(`${searches} ${searches === 1 ? "search" : "searches"}`);
  if (parts.length === 0) parts.push(`${session.events.length} ${session.events.length === 1 ? "event" : "events"}`);
  return parts.join(" · ");
}

export type HistoryRow =
  | { key: string; type: "day"; day: string; label: string }
  | { key: string; type: "session"; day: string; session: HistorySession; expanded: boolean }
  | { key: string; type: "event"; day: string; session: HistorySession; event: HistoryEventView };

/** Day headings, then each session, then its events when it is expanded. */
export function buildRows(sessions: readonly HistorySession[], expanded: ReadonlySet<string>, now: Date): HistoryRow[] {
  const rows: HistoryRow[] = [];
  let day: string | null = null;
  for (const session of sessions) {
    const sessionDay = dayKey(session.started_at);
    if (sessionDay !== day) {
      day = sessionDay;
      rows.push({ key: `day:${sessionDay}`, type: "day", day: sessionDay, label: dayLabel(sessionDay, now) });
    }
    const open = expanded.has(session.session_id);
    rows.push({ key: `session:${session.session_id}`, type: "session", day: sessionDay, session, expanded: open });
    if (!open) continue;
    for (const event of session.events) {
      rows.push({ key: `event:${session.session_id}:${event.id}`, type: "event", day: sessionDay, session, event });
    }
  }
  return rows;
}

/** Keys of the rows the cursor can stand on. */
export function selectableKeys(rows: readonly HistoryRow[]): string[] {
  return rows.filter((row) => row.type !== "day").map((row) => row.key);
}

/**
 * `]`: the first session of the next day. `[`: the first session of this day, or of the previous
 * day when already there. Stays put at either end.
 */
export function stepDay(rows: readonly HistoryRow[], current: string | null, direction: -1 | 1): string | null {
  const keys = selectableKeys(rows);
  if (keys.length === 0) return null;
  const here = rows.find((row) => row.key === current);
  if (here === undefined) return keys[0] ?? null;
  const firsts: HistoryRow[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    if (row.type !== "session" || seen.has(row.day)) continue;
    seen.add(row.day);
    firsts.push(row);
  }
  const dayIndex = firsts.findIndex((row) => row.day === here.day);
  const own = firsts[dayIndex];
  if (direction < 0) {
    if (own !== undefined && own.key !== here.key) return own.key;
    return firsts[dayIndex - 1]?.key ?? here.key;
  }
  return firsts[dayIndex + 1]?.key ?? here.key;
}

export type RowTarget = { paperId?: string; workId?: string; listId?: string };

/** What `↵` and `Y` act on: an event's own paper, Work or list; a session's busiest paper. */
export function rowTarget(row: HistoryRow): RowTarget {
  if (row.type === "day") return {};
  if (row.type === "event") {
    return {
      ...(row.event.paper_id === null ? {} : { paperId: row.event.paper_id }),
      ...(row.event.work_id === null ? {} : { workId: row.event.work_id }),
      ...(row.event.list_id === null ? {} : { listId: row.event.list_id }),
    };
  }
  const paperId = sessionPapers(row.session)[0]?.paperId;
  return paperId === undefined ? {} : { paperId };
}
