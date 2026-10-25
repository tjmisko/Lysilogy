import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "../lib/api";
import {
  buildRows, clockTime, describeEvent, formatDwell, KIND_GROUPS, kindsForGroups, mergeSessions, rowTarget,
  selectableKeys, sessionSummary, stepDay, toggleGroup, type HistoryRow,
} from "../lib/historyView";
import { moveSelection, stepSequence, SEQUENCE_TIMEOUT_MS } from "../lib/listKeys";
import type { PickerTarget } from "../lib/listEvents";
import type { HistorySession, InspectedWork, LibraryResponse, ReadingPosition } from "../types";
import "./ListsPage.css";
import "./HistoryPage.css";

const PAGE_ROWS = 10;
const PAGE_SESSIONS = 40;
const FILTER_DEBOUNCE_MS = 250;

export type HistoryPageProps = {
  /** Titles for the papers the history names. */
  papers: LibraryResponse["papers"];
  /** Open a paper from the history, at its saved position when it has one. */
  onOpenPaper: (paperId: string, position: ReadingPosition | null) => void;
  onOpenList: (listId: string) => void;
  /** Open the list picker for a paper or Work (`Y`). */
  onAddToList: (target: PickerTarget) => void;
  onNotice: (text: string) => void;
  onError: (text: string) => void;
  keyboardEnabled: boolean;
};

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

function timeRange(session: HistorySession): string {
  const start = clockTime(session.started_at);
  const end = clockTime(session.ended_at);
  return start === end || end === "" ? start : `${start}–${end}`;
}

/** `#history`: sessions by day, newest first, with the events of each one a key away. */
export function HistoryPage({ papers, onOpenPaper, onOpenList, onAddToList, onNotice, onError, keyboardEnabled }: HistoryPageProps) {
  const [sessions, setSessions] = useState<HistorySession[] | null>(null);
  const [nextBefore, setNextBefore] = useState<string | null>(null);
  const [groups, setGroups] = useState<ReadonlySet<string>>(new Set());
  const [query, setQuery] = useState("");
  const [queryInput, setQueryInput] = useState("");
  const [typing, setTyping] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [showInspected, setShowInspected] = useState(false);
  const [inspected, setInspected] = useState<InspectedWork[] | null>(null);
  const [inspectedKey, setInspectedKey] = useState<string | null>(null);
  const pending = useRef("");
  const pendingTimer = useRef<number | null>(null);
  const loadingMore = useRef(false);
  const filterRef = useRef<HTMLInputElement>(null);

  const titles = useMemo(() => new Map(papers.map((paper) => [paper.id, paper.metadata.title])), [papers]);
  const titleOf = useCallback((paperId: string): string | null => titles.get(paperId) ?? null, [titles]);
  const kinds = useMemo(() => kindsForGroups(groups), [groups]);
  const kindsKey = kinds.join(",");

  useEffect(() => {
    const timer = window.setTimeout(() => setQuery(queryInput.trim()), FILTER_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [queryInput]);

  useEffect(() => {
    const controller = new AbortController();
    void api.history({ kinds, ...(query === "" ? {} : { q: query }), limit: PAGE_SESSIONS }, controller.signal).then((timeline) => {
      if (controller.signal.aborted) return;
      setSessions(mergeSessions([], timeline.sessions));
      setNextBefore(timeline.next_before);
    }).catch((reason: unknown) => {
      if (controller.signal.aborted) return;
      setSessions((current) => current ?? []);
      onError(reason instanceof Error ? reason.message : "Could not load the reading history.");
    });
    return () => controller.abort();
    // `kinds` is derived from `kindsKey`; the string is the stable dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kindsKey, onError, query]);

  useEffect(() => {
    if (!showInspected) return;
    const controller = new AbortController();
    void api.historyInspected(controller.signal).then((found) => {
      if (!controller.signal.aborted) setInspected(found.works);
    }).catch((reason: unknown) => {
      if (controller.signal.aborted) return;
      setInspected((current) => current ?? []);
      onError(reason instanceof Error ? reason.message : "Could not load the inspected works.");
    });
    return () => controller.abort();
  }, [onError, showInspected]);

  const rows = useMemo(() => buildRows(sessions ?? [], expanded, new Date()), [sessions, expanded]);
  const keys = useMemo(() => selectableKeys(rows), [rows]);
  const current = selectedKey !== null && keys.includes(selectedKey) ? selectedKey : (keys[0] ?? null);
  const selectedRow = rows.find((row) => row.key === current) ?? null;

  const inspectedIds = useMemo(() => (inspected ?? []).map((work) => work.work_id), [inspected]);
  const inspectedCurrent = inspectedKey !== null && inspectedIds.includes(inspectedKey) ? inspectedKey : (inspectedIds[0] ?? null);

  useEffect(() => {
    const key = showInspected ? inspectedCurrent : current;
    if (key === null) return;
    document.querySelector(`.history-page [data-row-key="${CSS.escape(key)}"]`)?.scrollIntoView({ block: "nearest" });
  }, [current, inspectedCurrent, showInspected]);

  const clearPending = useCallback((): void => {
    pending.current = "";
    if (pendingTimer.current !== null) window.clearTimeout(pendingTimer.current);
    pendingTimer.current = null;
  }, []);

  useEffect(() => clearPending, [clearPending]);

  useEffect(() => {
    if (typing) filterRef.current?.focus();
  }, [typing]);

  const loadMore = useCallback((): void => {
    if (nextBefore === null || loadingMore.current) return;
    loadingMore.current = true;
    void api.history({ before: nextBefore, kinds, ...(query === "" ? {} : { q: query }), limit: PAGE_SESSIONS }).then((timeline) => {
      setSessions((shown) => mergeSessions(shown ?? [], timeline.sessions));
      setNextBefore(timeline.next_before);
    }).catch((reason: unknown) => {
      onError(reason instanceof Error ? reason.message : "Could not load older history.");
    }).finally(() => {
      loadingMore.current = false;
    });
  }, [kinds, nextBefore, onError, query]);

  const setExpansion = useCallback((row: HistoryRow, open: boolean): void => {
    if (row.type === "day") return;
    const id = row.session.session_id;
    setExpanded((now) => {
      const next = new Set(now);
      if (open) next.add(id);
      else next.delete(id);
      return next;
    });
    if (!open && row.type === "event") setSelectedKey(`session:${id}`);
  }, []);

  const reopen = useCallback(async (row: HistoryRow): Promise<void> => {
    const target = rowTarget(row);
    if (target.paperId !== undefined) {
      let position: ReadingPosition | null = null;
      try {
        position = (await api.paperHistory(target.paperId)).position;
      } catch {
        // Without a saved position the paper still opens, at its start.
      }
      onOpenPaper(target.paperId, position);
      return;
    }
    if (target.listId !== undefined) {
      onOpenList(target.listId);
      return;
    }
    if (row.type === "session") {
      setExpansion(row, !row.expanded);
      return;
    }
    onNotice("This event has no paper or list to open. Press Y to add its work to a list.");
  }, [onNotice, onOpenList, onOpenPaper, setExpansion]);

  const addToList = useCallback((target: { paperId?: string; workId?: string }, title?: string): void => {
    if (target.paperId === undefined && target.workId === undefined) {
      onNotice("Nothing here can be added to a list.");
      return;
    }
    onAddToList({ ...target, ...(title === undefined ? {} : { title }) });
  }, [onAddToList, onNotice]);

  useEffect(() => {
    if (!keyboardEnabled || typing) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.isComposing || event.metaKey || event.altKey || isEditable(event.target)) return;
      const key = event.key;
      const ids = showInspected ? inspectedIds : keys;
      const at = showInspected ? inspectedCurrent : current;
      const select = (id: string | null): void => (showInspected ? setInspectedKey(id) : setSelectedKey(id));
      const move = (delta: number): void => {
        event.preventDefault();
        if (!showInspected && delta > 0 && at !== null && at === ids.at(-1)) loadMore();
        select(moveSelection(ids, at, delta));
      };
      if (event.ctrlKey) {
        if (key === "d") move(PAGE_ROWS);
        else if (key === "u") move(-PAGE_ROWS);
        return;
      }
      if (key === "Escape") {
        clearPending();
        if (showInspected) setShowInspected(false);
        else if (query !== "" || queryInput !== "") { setQuery(""); setQueryInput(""); }
        else if (groups.size > 0) setGroups(new Set());
        else return;
        event.preventDefault();
        return;
      }
      const step = stepSequence(pending.current, key);
      if (step.consumed) {
        event.preventDefault();
        clearPending();
        if (step.pending !== "") {
          pending.current = step.pending;
          pendingTimer.current = window.setTimeout(clearPending, SEQUENCE_TIMEOUT_MS);
          return;
        }
        if (step.action === "top") select(ids[0] ?? null);
        if (showInspected || selectedRow === null) return;
        if (step.action === "fold-open") setExpansion(selectedRow, true);
        else if (step.action === "fold-close") setExpansion(selectedRow, false);
        else if (step.action === "fold-toggle" && selectedRow.type !== "day") setExpansion(selectedRow, selectedRow.type === "session" ? !selectedRow.expanded : false);
        else if (step.action === "fold-open-all") setExpanded(new Set((sessions ?? []).map((session) => session.session_id)));
        else if (step.action === "fold-close-all") {
          setExpanded(new Set());
          if (selectedRow.type === "event") setSelectedKey(`session:${selectedRow.session.session_id}`);
        }
        return;
      }
      if (key === "j" || key === "ArrowDown") move(1);
      else if (key === "k" || key === "ArrowUp") move(-1);
      else if (key === "G") {
        event.preventDefault();
        loadMore();
        select(ids.at(-1) ?? null);
      } else if (key === "I") {
        event.preventDefault();
        setShowInspected((on) => !on);
      } else if (key === "Y") {
        event.preventDefault();
        if (showInspected) {
          const work = (inspected ?? []).find((item) => item.work_id === inspectedCurrent);
          addToList(work === undefined ? {} : { workId: work.work_id }, work?.title ?? undefined);
          return;
        }
        if (selectedRow === null) return;
        const target = rowTarget(selectedRow);
        addToList({ ...(target.paperId === undefined ? {} : { paperId: target.paperId }), ...(target.workId === undefined ? {} : { workId: target.workId }) },
          target.paperId === undefined ? undefined : titleOf(target.paperId) ?? undefined);
      } else if (showInspected) {
        return;
      } else if (key === "[" || key === "]") {
        event.preventDefault();
        setSelectedKey(stepDay(rows, current, key === "]" ? 1 : -1));
      } else if (key === "Enter" || key === "o") {
        if (selectedRow === null) return;
        event.preventDefault();
        void reopen(selectedRow);
      } else if (key === "/") {
        event.preventDefault();
        setTyping(true);
      } else if (KIND_GROUPS.some((group) => group.key === key)) {
        event.preventDefault();
        setGroups((now) => toggleGroup(now, key));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [addToList, clearPending, current, groups.size, inspected, inspectedCurrent, inspectedIds, keyboardEnabled, keys, loadMore,
    query, queryInput, reopen, rows, selectedRow, sessions, setExpansion, showInspected, titleOf, typing]);

  const sessionCount = sessions?.length ?? 0;
  const empty = sessions !== null && rows.length === 0;
  const filtered = groups.size > 0 || query !== "";

  return <section className="history-page" aria-label="Reading history" data-keyboard={keyboardEnabled}>
    <header className="lists-head history-head">
      <h1>Reading history</h1>
      <div className="history-kinds" role="group" aria-label="Event kinds">
        {KIND_GROUPS.map((group) => <button
          key={group.key}
          type="button"
          tabIndex={-1}
          aria-pressed={groups.has(group.key)}
          onClick={() => setGroups((now) => toggleGroup(now, group.key))}
        ><kbd>{group.key}</kbd> {group.label}</button>)}
        <button type="button" tabIndex={-1} aria-pressed={showInspected} onClick={() => setShowInspected((on) => !on)}><kbd>I</kbd> inspected, never opened</button>
      </div>
      <label className="home-search lists-filter">
        <span aria-hidden="true">⌕</span>
        <input
          ref={filterRef}
          type="search"
          aria-label="Filter history"
          value={queryInput}
          placeholder="Filter history"
          onChange={(event) => setQueryInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== "Enter" && event.key !== "Escape" && event.key !== "ArrowDown") return;
            event.preventDefault();
            event.stopPropagation();
            if (event.key === "Escape") { setQueryInput(""); setQuery(""); }
            if (event.key === "Enter") setQuery(queryInput.trim());
            setTyping(false);
            filterRef.current?.blur();
          }}
          onBlur={() => setTyping(false)}
        />
        <kbd aria-hidden="true">/</kbd>
      </label>
      <p className="home-counts" role="status">
        {showInspected
          ? <><strong>{inspected?.length ?? 0}</strong> inspected, never opened</>
          : <><strong>{sessionCount}</strong> {sessionCount === 1 ? "session" : "sessions"}{nextBefore !== null && " so far"}{filtered && " (filtered)"}</>}
      </p>
    </header>

    {showInspected && <div className="history-inspected">
      {inspected === null && <div className="lists-empty"><p>Loading inspected works…</p></div>}
      {inspected !== null && inspected.length === 0 && <div className="lists-empty">
        <h2>Nothing inspected without opening.</h2>
        <p>Works you look at in the graph for two seconds or more, and never open, collect here.</p>
      </div>}
      {inspected !== null && inspected.length > 0 && <ol className="history-rows" aria-label="Inspected works">
        {inspected.map((work) => <li
          key={work.work_id}
          className={`history-row history-inspected-row ${work.work_id === inspectedCurrent ? "is-selected" : ""}`}
          data-row-key={work.work_id}
          onClick={() => setInspectedKey(work.work_id)}
        >
          <span className="history-time">{clockTime(work.last_inspected)}</span>
          <span className="history-title">{work.title ?? work.work_id}</span>
          <span className="history-meta">{work.inspects} {work.inspects === 1 ? "look" : "looks"}</span>
        </li>)}
      </ol>}
    </div>}

    {!showInspected && sessions === null && <div className="lists-empty"><p>Loading history…</p></div>}
    {!showInspected && empty && <div className="lists-empty">
      <h2>{filtered ? "No history matches these filters." : "No reading history yet."}</h2>
      <p>{filtered
        ? <>Press <kbd>Esc</kbd> to clear them.</>
        : "Papers you open, sections you read and searches you run are recorded here as you read."}</p>
    </div>}

    {!showInspected && rows.length > 0 && <ol className="history-rows" aria-label="Sessions">
      {rows.map((row) => {
        if (row.type === "day") return <li key={row.key} className="history-day" role="presentation"><h2>{row.label}</h2></li>;
        const selected = row.key === current;
        if (row.type === "session") {
          const count = row.session.events.length;
          return <li
            key={row.key}
            className={`history-row history-session ${selected ? "is-selected" : ""}`}
            data-row-key={row.key}
            aria-expanded={row.expanded}
            onClick={() => setSelectedKey(row.key)}
            onDoubleClick={() => void reopen(row)}
          >
            <span className="history-time">{timeRange(row.session)}</span>
            <span className="history-fold" aria-hidden="true">{row.expanded ? "▾" : "▸"}</span>
            <span className="history-title">{sessionSummary(row.session)}</span>
            <span className="history-meta">{count} {count === 1 ? "event" : "events"}</span>
          </li>;
        }
        return <li
          key={row.key}
          className={`history-row history-event ${selected ? "is-selected" : ""}`}
          data-row-key={row.key}
          data-kind={row.event.kind}
          onClick={() => setSelectedKey(row.key)}
          onDoubleClick={() => void reopen(row)}
        >
          <span className="history-time">{clockTime(row.event.occurred_at)}</span>
          <span className="history-title">{describeEvent(row.event, titleOf)}</span>
          {row.event.dwell_ms !== null && row.event.kind !== "paper_left" && row.event.kind !== "section_focused"
            && row.event.kind !== "figure_viewed" && row.event.kind !== "work_inspected"
            && <span className="history-meta">{formatDwell(row.event.dwell_ms)}</span>}
        </li>;
      })}
    </ol>}
    {!showInspected && nextBefore !== null && <p className="history-more">
      <button type="button" tabIndex={-1} onClick={loadMore}>Load older sessions</button>
    </p>}

    <footer className="lists-keys" aria-label="Keys">
      <span><kbd>j/k</kbd> move</span>
      <span><kbd>[</kbd><kbd>]</kbd> day</span>
      <span><kbd>↵</kbd> reopen at position</span>
      <span><kbd>zo</kbd><kbd>zc</kbd> events</span>
      <span><kbd>Y</kbd> add to list</span>
      <span><kbd>/</kbd> filter</span>
      <span><kbd>1</kbd>–<kbd>5</kbd> kinds</span>
      <span><kbd>I</kbd> inspected</span>
    </footer>
  </section>;
}
