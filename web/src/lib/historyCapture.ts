// Where reading events are decided (spec: History, Capture). A recorder owns the open paper's
// visit, the reading position (saved debounced) and the timed events (section, figure, inspected
// Work). It takes the queue and the clock as arguments, so Node tests drive it.

import {
  Debouncer, isInspection, PaperVisit, type HistoryQueue, type Timers,
} from "./history.ts";
import type { HistoryEventDraft, ReadingPositionInput } from "../types";

/** What the reader is showing, as `view_changed` and the saved position name it. */
export type ReaderView = "abstract" | "overview" | "glossary" | "pdf" | "markdown" | "sources";

/** The `view` values the server accepts for a saved position. */
export type PositionView = "pdf" | "markdown" | "overview" | "section";

export type RecorderDeps = {
  queue: HistoryQueue;
  now: () => number;
  timers: Timers;
  /** Save the position; failures are the caller's to swallow. */
  savePosition: (paperId: string, position: ReadingPositionInput) => void;
};

type PendingPosition = { paperId: string; position: ReadingPositionInput };

type TimedEntry = {
  kind: HistoryEventDraft["kind"];
  detail: Record<string, unknown>;
  paperId: string;
  visit: PaperVisit;
  startDwell: number;
};

const QUERY_MAX = 500;

/** Longest text kept in event details. Counts code points, so a cut never splits a character. */
export function clipText(text: string, max = QUERY_MAX): string {
  const characters = Array.from(text);
  return characters.length <= max ? text : `${characters.slice(0, max).join("")}…`;
}

export function positionViewOf(view: ReaderView): PositionView {
  return view === "pdf" || view === "markdown" ? view : "overview";
}

export type RestoreTarget = { view: "overview" | "text"; textMode: "pdf" | "markdown"; page: number | null };

/** Where a saved position reopens: markdown, the overview, or a PDF page (a section opens its page). */
export function restoreTarget(
  position: { view: string; page: number | null } | null,
): RestoreTarget | null {
  if (position === null) return null;
  if (position.view === "markdown") return { view: "text", textMode: "markdown", page: null };
  if (position.view === "overview" && position.page === null) return { view: "overview", textMode: "pdf", page: null };
  const page = position.page === null ? null : Math.max(1, position.page);
  return { view: "text", textMode: "pdf", page };
}

export class HistoryRecorder {
  private readonly deps: RecorderDeps;
  private readonly debounce: Debouncer<PendingPosition>;
  private visit: PaperVisit | null = null;
  private paperId: string | null = null;
  private view: ReaderView = "abstract";
  private page: number | null = null;
  private sectionId: string | null = null;
  private readonly timed = new Set<TimedEntry>();

  constructor(deps: RecorderDeps) {
    this.deps = deps;
    this.debounce = new Debouncer<PendingPosition>(
      (pending) => deps.savePosition(pending.paperId, pending.position),
      deps.timers,
    );
  }

  /** The open paper, or null. */
  get currentPaperId(): string | null {
    return this.paperId;
  }

  /**
   * Start a visit to `paperId` (ending any other) or, for the paper already open, note the view.
   * A null `paperId` ends the visit.
   */
  openPaper(paperId: string | null, view: ReaderView): void {
    if (paperId === null || paperId.trim() === "") {
      this.leavePaper();
      return;
    }
    if (paperId === this.paperId) {
      this.setView(view);
      return;
    }
    this.leavePaper();
    this.paperId = paperId;
    this.view = view;
    this.visit = new PaperVisit(this.deps.queue, this.deps, { paper_id: paperId });
  }

  /** Close open timed events, record `paper_left`, and save the waiting position. */
  leavePaper(): void {
    if (this.visit === null) return;
    for (const entry of [...this.timed]) this.finish(entry);
    this.debounce.flushNow();
    this.visit.end();
    this.visit = null;
    this.paperId = null;
    this.page = null;
    this.sectionId = null;
  }

  /** `view_changed` when the view differs from the last one. */
  setView(view: ReaderView): void {
    if (this.visit === null || view === this.view) return;
    const from = this.view;
    this.view = view;
    this.visit.input();
    this.record({ kind: "view_changed", detail: { view, from } });
    this.pushPosition();
  }

  /** Active input (keys, pointer, scroll). */
  input(): void {
    this.visit?.input();
  }

  setVisible(visible: boolean): void {
    this.visit?.setVisible(visible);
  }

  setFocused(focused: boolean): void {
    this.visit?.setFocused(focused);
  }

  /** Record an event for the open paper, unless the draft names its own. */
  record(draft: HistoryEventDraft): void {
    const base = this.paperId === null ? {} : { paper_id: this.paperId };
    this.deps.queue.record({ ...base, ...draft });
  }

  /** The reader shows `page`: `page_reached` once per page per visit, and the position follows. */
  pageReached(page: number): void {
    if (this.visit === null || !Number.isInteger(page) || page < 1) return;
    this.visit.pageReached(page);
    this.page = page;
    this.pushPosition();
  }

  /**
   * A section is open in the reader. Returns the function that closes it, recording
   * `section_focused` with the active dwell. The saved position names the section while it is open.
   */
  focusSection(sectionId: string, title: string): () => void {
    this.sectionId = sectionId;
    this.pushPosition();
    const end = this.beginTimed("section_focused", { section_id: sectionId, title: clipText(title, 200) });
    return () => {
      end();
      if (this.sectionId === sectionId) this.sectionId = null;
    };
  }

  /** A figure is open. Returns the function that closes it, recording `figure_viewed`. */
  viewFigure(label: string, page: number): () => void {
    return this.beginTimed("figure_viewed", { figure: clipText(label, 200), page });
  }

  /**
   * A Work panel is open. The returned function records `work_inspected` when it ran for at
   * least INSPECT_MIN_MS, once, and says whether it did.
   */
  beginInspection(workId: string): () => boolean {
    const started = this.deps.now();
    let done = false;
    return () => {
      if (done) return false;
      done = true;
      const open = this.deps.now() - started;
      if (!isInspection(open)) return false;
      this.record({ kind: "work_inspected", work_id: workId, dwell_ms: open });
      return true;
    };
  }

  /** Save a waiting position now (page leave). */
  flushPosition(): void {
    this.debounce.flushNow();
  }

  private beginTimed(kind: TimedEntry["kind"], detail: Record<string, unknown>): () => void {
    const visit = this.visit;
    const paperId = this.paperId;
    if (visit === null || paperId === null) return () => {};
    visit.input();
    const entry: TimedEntry = { kind, detail, paperId, visit, startDwell: visit.dwell() };
    this.timed.add(entry);
    return () => this.finish(entry);
  }

  private finish(entry: TimedEntry): void {
    if (!this.timed.delete(entry)) return;
    const dwell = Math.max(0, entry.visit.dwell() - entry.startDwell);
    this.deps.queue.record({ kind: entry.kind, paper_id: entry.paperId, detail: entry.detail, dwell_ms: dwell });
  }

  private pushPosition(): void {
    // Without a page there is nothing worth restoring, and a null page would erase the old one.
    if (this.paperId === null || this.page === null) return;
    const position: ReadingPositionInput = {
      view: this.sectionId !== null ? "section" : positionViewOf(this.view),
      page: this.page,
      section_id: this.sectionId,
    };
    this.debounce.push({ paperId: this.paperId, position });
  }
}
