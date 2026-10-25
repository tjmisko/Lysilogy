import { useCallback, useEffect, useRef, useState } from "react";

import { acquisitionStatusText, canFetch, isRetry } from "../lib/acquisition";
import { api } from "../lib/api";
import { nextReadingStatus } from "../lib/listKeys";
import { pathBreadcrumb } from "../lib/listSidebar";
import { itemRows, statusGlyph, type ItemRow } from "../lib/listViews";
import type { WalkListsApi } from "../lib/useWalkLists";
import type { AcquisitionJobState, ExpansionView } from "../types";

const POLL_MILLISECONDS = 1000;
const BREADCRUMB_LIMIT = 3;
const CRUMB_TITLE_LIMIT = 28;
const NEW_LIST_VALUE = "__new__";

type LiteratureListSidebarProps = {
  lists: WalkListsApi;
  selectedWorkId: string | null;
  /** Show the Work on the canvas when it is there. */
  onSelectWork: (workId: string) => void;
  onOpenPaper: (paperId: string) => void;
};

type RowFetch = { job: AcquisitionJobState; message: string | null };

function crumb(title: string): string {
  return title.length <= CRUMB_TITLE_LIMIT ? title : `${title.slice(0, CRUMB_TITLE_LIMIT - 1).trimEnd()}…`;
}

function rowTitle(row: ItemRow): string {
  return row.title === "Untitled" && row.workId !== null ? `Untitled (${row.workId.slice(0, 8)}…)` : row.title;
}

function expansionBadge(view: ExpansionView | undefined): string | null {
  if (view === undefined) return null;
  if (view.state === "unavailable") return "refs unavailable";
  if (view.state === "known") return `${view.references_count} refs`;
  return `expanded · ${view.references_count}`;
}

function failureMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "The request failed.";
}

/** An input that saves on Enter and cancels on Esc or when it loses focus. */
function InlineInput(props: {
  label: string;
  initial: string;
  className: string;
  onSave: (value: string) => void;
  onCancel: () => void;
}) {
  const { label, initial, className, onSave, onCancel } = props;
  const [value, setValue] = useState(initial);
  const finished = useRef(false);
  const finish = (save: boolean): void => {
    if (finished.current) return;
    finished.current = true;
    if (save) onSave(value);
    else onCancel();
  };
  return (
    <input
      className={className}
      aria-label={label}
      value={value}
      autoFocus
      maxLength={2000}
      onChange={(event) => setValue(event.target.value)}
      onBlur={() => finish(false)}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing) return;
        if (event.key === "Enter") {
          event.preventDefault();
          event.stopPropagation();
          finish(true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          finish(false);
        }
      }}
    />
  );
}

export function LiteratureListSidebar({ lists, selectedWorkId, onSelectWork, onOpenPaper }: LiteratureListSidebarProps) {
  const { detail, error, loaded, reload } = lists;
  const [editingTitle, setEditingTitle] = useState(false);
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
  const [fetches, setFetches] = useState<ReadonlyMap<string, RowFetch>>(() => new Map());
  const [fetchError, setFetchError] = useState<string | null>(null);

  const runningKey = [...fetches].filter(([, entry]) => entry.job === "running").map(([workId]) => workId).sort().join(",");
  // Poll running fetches once a second. The interval dies with the sidebar.
  useEffect(() => {
    if (runningKey === "") return;
    const controller = new AbortController();
    const timer = window.setInterval(() => {
      for (const workId of runningKey.split(",")) {
        void (async () => {
          try {
            const status = await api.workAcquisition(workId, controller.signal);
            if (controller.signal.aborted || status.job.state === "running") return;
            setFetches((current) => {
              const next = new Map(current);
              if (status.job.state === "failed") next.set(workId, { job: "failed", message: status.job.message ?? null });
              else next.delete(workId);
              return next;
            });
            void reload();
          } catch (reason: unknown) {
            if (controller.signal.aborted) return;
            setFetches((current) => new Map(current).set(workId, { job: "failed", message: failureMessage(reason) }));
          }
        })();
      }
    }, POLL_MILLISECONDS);
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [reload, runningKey]);

  const startFetch = useCallback((workId: string, retry: boolean): void => {
    setFetchError(null);
    setFetches((current) => new Map(current).set(workId, { job: "running", message: null }));
    api.fetchWork(workId, retry ? { retry_unavailable: true } : undefined).catch((reason: unknown) => {
      setFetches((current) => {
        const next = new Map(current);
        next.delete(workId);
        return next;
      });
      setFetchError(failureMessage(reason));
    });
  }, []);

  const listed = lists.lists;
  const shownError = error ?? fetchError;
  const rows = detail === null ? [] : itemRows(detail).filter((row) => row.kind === "work" && row.workId !== null);

  return (
    <aside className="literature-ramp" aria-label="Reading list" data-ramp-id={detail?.list.id}>
      <header className="literature-ramp-head">
        <h3>Reading list</h3>
        <label className="literature-ramp-picker">
          <select
            aria-label="List"
            value={detail?.list.id ?? ""}
            onChange={(event) => {
              if (event.target.value === NEW_LIST_VALUE) void lists.create();
              else void lists.select(event.target.value);
            }}
          >
            {detail === null && <option value="">No list yet</option>}
            {listed.map((list) => <option key={list.id} value={list.id}>{list.title} ({list.item_count})</option>)}
            <option value={NEW_LIST_VALUE}>New list</option>
          </select>
        </label>
      </header>

      {shownError !== null && <p className="literature-ramp-error" role="alert">{shownError}</p>}

      {detail === null && (
        <div className="literature-ramp-empty" data-ramp-empty="true">
          <p>{loaded ? "No reading list for this paper yet." : "Loading lists…"}</p>
          <p className="literature-hint">Select a work and press <kbd>y</kbd> to start one, <kbd>Y</kbd> to pick any list, or</p>
          <button type="button" className="literature-action" disabled={!loaded} onClick={() => void lists.create()}>New list</button>
        </div>
      )}

      {detail !== null && (
        <>
          <div className="literature-ramp-title">
            {editingTitle
              ? (
                <InlineInput
                  label="List title"
                  className="literature-ramp-input"
                  initial={detail.list.title}
                  onSave={(value) => {
                    setEditingTitle(false);
                    const title = value.trim();
                    if (title !== "" && title !== detail.list.title) void lists.rename(title);
                  }}
                  onCancel={() => setEditingTitle(false)}
                />
              )
              : <button type="button" className="literature-ramp-name" title="Rename" onClick={() => setEditingTitle(true)}>{detail.list.title}</button>}
          </div>
          <p className="literature-ramp-counts" data-ramp-counts="true">
            {detail.list.status_counts.read} read · {detail.list.status_counts.reading} reading · {detail.list.status_counts.unread} unread
          </p>
          <button type="button" className="literature-action" onClick={() => void lists.suggest()}>Suggest order</button>

          <ol className="literature-ramp-items">
            {rows.map((row, index) => {
              const workId = row.workId ?? row.id;
              const papers = detail.local_papers[workId] ?? [];
              const held = papers[0];
              const view = detail.acquisitions[workId] ?? null;
              const rowFetch = fetches.get(workId);
              const jobState = rowFetch?.job ?? "none";
              const fetchable = held === undefined && canFetch(workId, detail.local_papers, detail.acquisitions, jobState);
              const crumbs = pathBreadcrumb(row.node.path_steps, BREADCRUMB_LIMIT);
              const badge = expansionBadge(detail.expansions[workId]);
              const upcoming = nextReadingStatus(row.status);
              const title = rowTitle(row);
              return (
                <li
                  key={row.id}
                  className={`literature-ramp-item ${selectedWorkId === workId ? "is-selected" : ""}`}
                  data-ramp-item={workId}
                  data-status={row.status}
                  data-depth={row.node.depth}
                  style={{ paddingLeft: `${8 + Math.min(row.node.depth, 4) * 10}px` }}
                >
                  <div className="literature-ramp-line">
                    <span className="literature-ramp-ordinal">{row.ordinal}</span>
                    <button
                      type="button"
                      className="literature-ramp-status"
                      data-status-button={row.status}
                      aria-label={`${row.status}: mark as ${upcoming}`}
                      title={`${row.status} — click to mark as ${upcoming}`}
                      onClick={() => void lists.setStatus(workId, upcoming)}
                    >{statusGlyph(row.status)}</button>
                    <button type="button" className="literature-ramp-work" onClick={() => onSelectWork(workId)}>
                      <span className="literature-ramp-work-title">{title}</span>
                      {row.year !== null && <span className="literature-ramp-year"> · {row.year}</span>}
                    </button>
                  </div>
                  {crumbs.length > 0 && (
                    <p className="literature-ramp-via" data-ramp-via="true">via {crumbs.map((entry) => crumb(entry)).join(" › ")}</p>
                  )}
                  <div className="literature-ramp-meta">
                    {held !== undefined && <button type="button" className="literature-ramp-open" onClick={() => onOpenPaper(held.paper_id)}>Open</button>}
                    {held === undefined && (
                      <>
                        <span className="literature-ramp-local" data-ramp-local={jobState === "running" ? "running" : view?.state ?? "none"}>
                          {acquisitionStatusText(view, jobState)}{rowFetch?.job === "failed" && rowFetch.message !== null ? ` ${rowFetch.message}` : ""}
                        </span>
                        {fetchable && (
                          <button type="button" className="literature-ramp-fetch" onClick={() => startFetch(workId, isRetry(view))}>
                            {isRetry(view) ? "Retry" : "Fetch"}
                          </button>
                        )}
                      </>
                    )}
                    {badge !== null && <span className="literature-ramp-badge" data-ramp-badge={detail.expansions[workId]?.state}>{badge}</span>}
                    <span className="literature-ramp-actions">
                      <button type="button" aria-label={`Move ${title} up`} disabled={index === 0} onClick={() => void lists.move(row.id, -1)}>↑</button>
                      <button type="button" aria-label={`Move ${title} down`} disabled={index === rows.length - 1} onClick={() => void lists.move(row.id, 1)}>↓</button>
                      <button type="button" aria-label={`Remove ${title} from list`} onClick={() => void lists.remove(row.id)}>×</button>
                    </span>
                  </div>
                  <div className="literature-ramp-note">
                    {editingNoteId === row.id
                      ? (
                        <InlineInput
                          label="Note"
                          className="literature-ramp-input"
                          initial={row.note ?? ""}
                          onSave={(value) => {
                            setEditingNoteId(null);
                            const note = value.trim();
                            if (note !== (row.note ?? "")) void lists.setNote(row.id, note === "" ? null : note);
                          }}
                          onCancel={() => setEditingNoteId(null)}
                        />
                      )
                      : (
                        <button type="button" className="literature-ramp-note-text" onClick={() => setEditingNoteId(row.id)}>
                          {row.note ?? "Add a note"}
                        </button>
                      )}
                  </div>
                </li>
              );
            })}
          </ol>
        </>
      )}
    </aside>
  );
}
