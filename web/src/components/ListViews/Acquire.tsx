import { useEffect, useMemo, useState } from "react";

import { api } from "../../lib/api";
import { acquisitionStatusText } from "../../lib/acquisition";
import { acquire } from "../../lib/listViews";
import { StubRow } from "./StubRow";
import type { ListViewProps } from "./props";

const POLL_MILLISECONDS = 2000;

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** View 6, until the full view replaces it: works not held, `f` fetches the selected one, `F` all. */
export function Acquire(props: ListViewProps) {
  const { detail, actions, selectedId, keyboardEnabled } = props;
  const model = useMemo(() => acquire(detail), [detail]);
  const [started, setStarted] = useState<ReadonlySet<string>>(() => new Set());
  const fetchable = useMemo(() => new Set(model.fetchable), [model]);
  const running = [...started].filter((workId) => fetchable.has(workId));
  const runningKey = running.sort().join(",");

  // A fetched Work leaves the fetchable set, which is how a finished fetch shows; reload to see it.
  useEffect(() => {
    if (runningKey === "") return;
    const timer = window.setInterval(() => void actions.reload(), POLL_MILLISECONDS);
    return () => window.clearInterval(timer);
  }, [actions, runningKey]);

  const fetchWorks = (workIds: readonly string[]): void => {
    if (workIds.length === 0) {
      actions.notify("Nothing to fetch here.");
      return;
    }
    setStarted((current) => new Set([...current, ...workIds]));
    // One at a time, as the acquisition jobs expect.
    void (async () => {
      for (const workId of workIds) {
        try {
          await api.fetchWork(workId);
        } catch (reason: unknown) {
          actions.fail(reason instanceof Error ? reason.message : "The fetch failed.");
        }
      }
    })();
  };

  useEffect(() => {
    if (!keyboardEnabled) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || isEditable(event.target)) return;
      if (event.key === "F") {
        event.preventDefault();
        fetchWorks(model.fetchable);
      } else if (event.key === "f") {
        event.preventDefault();
        const row = model.rows.find((entry) => entry.row.id === selectedId);
        fetchWorks(row?.fetchable === true && row.row.workId !== null ? [row.row.workId] : []);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // fetchWorks only closes over props already listed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keyboardEnabled, model, selectedId]);

  if (model.rows.length === 0) {
    return <div className="list-empty"><p>Every paper on this list is held, or the list has no papers yet.</p></div>;
  }
  return <ol className="list-sequence" aria-label="Papers to acquire">
    {model.rows.map(({ row }) => {
      const view = row.workId === null ? null : (detail.acquisitions[row.workId] ?? null);
      const job = row.workId !== null && started.has(row.workId) && fetchable.has(row.workId) ? "running" : "none";
      return <StubRow key={row.id} row={row} props={props} detail={acquisitionStatusText(view, job)} />;
    })}
  </ol>;
}
