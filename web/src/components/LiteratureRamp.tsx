import { useCallback, useEffect, useRef, useState } from "react";

import { acquisitionStatusText, canFetch, isRetry } from "../lib/acquisition";
import { api } from "../lib/api";
import { nextStatus, pathBreadcrumb } from "../lib/ramp";
import type { RampsApi } from "../lib/useRamps";
import type { AcquisitionJobState, ExpansionView, RampItemView } from "../types";

const POLL_MILLISECONDS = 1000;
const BREADCRUMB_LIMIT = 3;
const CRUMB_TITLE_LIMIT = 28;
const NEW_RAMP_VALUE = "__new__";

type LiteratureRampProps = {
  ramps: RampsApi;
  selectedWorkId: string | null;
  /** Show the Work on the canvas when it is there. */
  onSelectWork: (workId: string) => void;
  onOpenPaper: (paperId: string) => void;
};

type RowFetch = { job: AcquisitionJobState; message: string | null };

function crumb(title: string): string {
  return title.length <= CRUMB_TITLE_LIMIT ? title : `${title.slice(0, CRUMB_TITLE_LIMIT - 1).trimEnd()}…`;
}

function itemTitle(item: RampItemView): string {
  const title = item.work.display_title?.trim() ?? "";
  return title === "" ? `Untitled (${item.work.id.slice(0, 8)}…)` : title;
}

function expansionBadge(view: ExpansionView | undefined): string | null {
  if (view === undefined) return null;
  if (view.state === "unavailable") return "refs unavailable";
  if (view.state === "known") return `${view.references_count} refs`;
  return `expanded · ${view.references_count}`;
}

function statusSymbol(status: RampItemView["status"]): string {
  if (status === "read") return "●";
  return status === "reading" ? "◐" : "○";
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

export function LiteratureRamp({ ramps, selectedWorkId, onSelectWork, onOpenPaper }: LiteratureRampProps) {
  const { detail, error, loaded, reload } = ramps;
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

  const listed = ramps.ramps;
  const shownError = error ?? fetchError;

  return (
    <aside className="literature-ramp" aria-label="Reading ramp" data-ramp-id={detail?.ramp.id}>
      <header className="literature-ramp-head">
        <h3>Reading ramp</h3>
        <label className="literature-ramp-picker">
          <select
            aria-label="Ramp"
            value={detail?.ramp.id ?? ""}
            onChange={(event) => {
              if (event.target.value === NEW_RAMP_VALUE) void ramps.create();
              else void ramps.select(event.target.value);
            }}
          >
            {detail === null && <option value="">No ramp yet</option>}
            {listed.map((ramp) => <option key={ramp.id} value={ramp.id}>{ramp.title} ({ramp.item_count})</option>)}
            <option value={NEW_RAMP_VALUE}>New ramp</option>
          </select>
        </label>
      </header>

      {shownError !== null && <p className="literature-ramp-error" role="alert">{shownError}</p>}

      {detail === null && (
        <div className="literature-ramp-empty" data-ramp-empty="true">
          <p>{loaded ? "No reading ramp for this paper yet." : "Loading ramps…"}</p>
          <p className="literature-hint">Select a work and press <kbd>y</kbd> to start one, or</p>
          <button type="button" className="literature-action" disabled={!loaded} onClick={() => void ramps.create()}>New ramp</button>
        </div>
      )}

      {detail !== null && (
        <>
          <div className="literature-ramp-title">
            {editingTitle
              ? (
                <InlineInput
                  label="Ramp title"
                  className="literature-ramp-input"
                  initial={detail.ramp.title}
                  onSave={(value) => {
                    setEditingTitle(false);
                    const title = value.trim();
                    if (title !== "" && title !== detail.ramp.title) void ramps.rename(title);
                  }}
                  onCancel={() => setEditingTitle(false)}
                />
              )
              : <button type="button" className="literature-ramp-name" title="Rename" onClick={() => setEditingTitle(true)}>{detail.ramp.title}</button>}
          </div>
          <p className="literature-ramp-counts" data-ramp-counts="true">
            {detail.ramp.status_counts.read} read · {detail.ramp.status_counts.reading} reading · {detail.ramp.status_counts.unread} unread
          </p>
          <button type="button" className="literature-action" onClick={() => void ramps.suggest()}>Suggest order</button>

          <ol className="literature-ramp-items">
            {detail.items.map((item, index) => {
              const workId = item.work.id;
              const isSeed = workId === detail.ramp.seed_work_id;
              const papers = detail.local_papers[workId] ?? [];
              const held = papers[0];
              const view = detail.acquisitions[workId] ?? null;
              const rowFetch = fetches.get(workId);
              const jobState = rowFetch?.job ?? "none";
              const fetchable = held === undefined && canFetch(workId, detail.local_papers, detail.acquisitions, jobState);
              const crumbs = pathBreadcrumb(item.path, BREADCRUMB_LIMIT);
              const badge = expansionBadge(detail.expansions[workId]);
              const upcoming = nextStatus(item.status);
              return (
                <li
                  key={workId}
                  className={`literature-ramp-item ${selectedWorkId === workId ? "is-selected" : ""}`}
                  data-ramp-item={workId}
                  data-status={item.status}
                  data-depth={item.depth}
                  style={{ paddingLeft: `${8 + Math.min(item.depth, 4) * 10}px` }}
                >
                  <div className="literature-ramp-line">
                    <span className="literature-ramp-ordinal">{item.ordinal}</span>
                    <button
                      type="button"
                      className="literature-ramp-status"
                      data-status-button={item.status}
                      aria-label={`${item.status}: mark as ${upcoming}`}
                      title={`${item.status} — click to mark as ${upcoming}`}
                      onClick={() => void ramps.setStatus(workId, upcoming)}
                    >{statusSymbol(item.status)}</button>
                    <button type="button" className="literature-ramp-work" onClick={() => onSelectWork(workId)}>
                      <span className="literature-ramp-work-title">{itemTitle(item)}</span>
                      {item.work.display_year !== null && <span className="literature-ramp-year"> · {item.work.display_year}</span>}
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
                      <button type="button" aria-label={`Move ${itemTitle(item)} up`} disabled={index === 0} onClick={() => void ramps.move(workId, -1)}>↑</button>
                      <button type="button" aria-label={`Move ${itemTitle(item)} down`} disabled={index === detail.items.length - 1} onClick={() => void ramps.move(workId, 1)}>↓</button>
                      {!isSeed && (
                        <button type="button" aria-label={`Remove ${itemTitle(item)} from ramp`} onClick={() => void ramps.remove(workId)}>×</button>
                      )}
                    </span>
                  </div>
                  <div className="literature-ramp-note">
                    {editingNoteId === workId
                      ? (
                        <InlineInput
                          label="Note"
                          className="literature-ramp-input"
                          initial={item.note ?? ""}
                          onSave={(value) => {
                            setEditingNoteId(null);
                            const note = value.trim();
                            if (note !== (item.note ?? "")) void ramps.setNote(workId, note === "" ? null : note);
                          }}
                          onCancel={() => setEditingNoteId(null)}
                        />
                      )
                      : (
                        <button type="button" className="literature-ramp-note-text" onClick={() => setEditingNoteId(workId)}>
                          {item.note ?? "Add a note"}
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
