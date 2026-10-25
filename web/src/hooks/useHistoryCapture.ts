import { useEffect, useRef } from "react";

import { api } from "../lib/api";
import { attachHistoryFlush, browserHistoryEnvironment, HistoryQueue } from "../lib/history";
import { HistoryRecorder, type ReaderView } from "../lib/historyCapture";
import { LISTS_CHANGED_EVENT } from "../lib/listEvents";
import type { HistoryEventDraft } from "../types";

// One recorder per page load. Components far from App report through the functions below, which
// do nothing harmful before the hook has mounted.
let queue: HistoryQueue | null = null;
let recorder: HistoryRecorder | null = null;

function ensureRecorder(): { queue: HistoryQueue; recorder: HistoryRecorder } {
  if (queue === null || recorder === null) {
    queue = new HistoryQueue(browserHistoryEnvironment((events) => api.postHistoryEvents(events)));
    recorder = new HistoryRecorder({
      queue,
      now: () => Date.now(),
      timers: {
        setTimeout: (callback, ms) => window.setTimeout(callback, ms),
        clearTimeout: (handle) => window.clearTimeout(handle as number),
      },
      // A position that cannot be saved costs only the restore; reading goes on.
      savePosition: (paperId, position) => {
        void api.putPosition(paperId, position).catch(() => undefined);
      },
    });
  }
  return { queue, recorder };
}

/** Record one event; the open paper is filled in unless the draft names its own. */
export function recordHistory(draft: HistoryEventDraft): void {
  ensureRecorder().recorder.record(draft);
}

/** The reader shows this page (`page_reached` once per visit, and the position follows). */
export function historyPageReached(page: number): void {
  ensureRecorder().recorder.pageReached(page);
}

/** A section opened in the reader; call the result to close it. */
export function historyFocusSection(sectionId: string, title: string): () => void {
  return ensureRecorder().recorder.focusSection(sectionId, title);
}

/** A figure opened; call the result to close it. */
export function historyViewFigure(label: string, page: number): () => void {
  return ensureRecorder().recorder.viewFigure(label, page);
}

/** A Work panel opened; call the result when it closes (records only after 2 s). */
export function historyInspectWork(workId: string): () => boolean {
  return ensureRecorder().recorder.beginInspection(workId);
}

export type HistoryCaptureState = {
  /** The open paper, or null on home, lists and history. */
  paperId: string | null;
  view: ReaderView;
  /** The open list, or null. */
  listId: string | null;
};

/** Mounted once by App. Starts the flush, tracks active dwell and follows the open paper. */
export function useHistoryCapture({ paperId, view, listId }: HistoryCaptureState): void {
  const latest = useRef({ paperId, view, listId });
  // First, so the effects below read this render's values.
  useEffect(() => {
    latest.current = { paperId, view, listId };
  });

  useEffect(() => {
    const { queue: events, recorder: rec } = ensureRecorder();
    const onInput = (): void => rec.input();
    const onVisibility = (): void => rec.setVisible(document.visibilityState === "visible");
    const onFocus = (): void => rec.setFocused(true);
    const onBlur = (): void => rec.setFocused(false);
    // Before the flush's own pagehide listener, so `paper_left` rides in the same beacon.
    const onPageHide = (): void => rec.leavePaper();
    const onPageShow = (event: PageTransitionEvent): void => {
      if (event.persisted) rec.openPaper(latest.current.paperId, latest.current.view);
    };
    const onListsChanged = (): void => {
      const open = latest.current.listId;
      rec.record(open === null ? { kind: "list_changed" } : { kind: "list_changed", list_id: open });
    };
    window.addEventListener("pagehide", onPageHide);
    window.addEventListener("pageshow", onPageShow);
    const detach = attachHistoryFlush(events);
    window.addEventListener("keydown", onInput, true);
    window.addEventListener("pointerdown", onInput, true);
    window.addEventListener("wheel", onInput, { capture: true, passive: true });
    window.addEventListener("scroll", onInput, { capture: true, passive: true });
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener(LISTS_CHANGED_EVENT, onListsChanged);
    return () => {
      rec.leavePaper();
      detach();
      window.removeEventListener("pagehide", onPageHide);
      window.removeEventListener("pageshow", onPageShow);
      window.removeEventListener("keydown", onInput, true);
      window.removeEventListener("pointerdown", onInput, true);
      window.removeEventListener("wheel", onInput, true);
      window.removeEventListener("scroll", onInput, true);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener(LISTS_CHANGED_EVENT, onListsChanged);
    };
  }, []);

  useEffect(() => {
    const { recorder: rec } = ensureRecorder();
    rec.openPaper(paperId, latest.current.view);
    if (paperId === null) return;
    rec.setVisible(document.visibilityState === "visible");
    rec.setFocused(document.hasFocus());
  }, [paperId]);

  useEffect(() => {
    ensureRecorder().recorder.setView(view);
  }, [view]);

  useEffect(() => {
    if (listId === null) return;
    ensureRecorder().recorder.record({ kind: "list_opened", list_id: listId });
  }, [listId]);
}
