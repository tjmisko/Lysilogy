import { useEffect, useMemo, useRef, useState } from "react";

import { api } from "../../lib/api";
import { acquisitionStatusText } from "../../lib/acquisition";
import { jobAfterPoll, type AcquireJob } from "../../lib/acquireJobs";
import { acquire } from "../../lib/listViews";
import { StubRow } from "./StubRow";
import type { ListViewProps } from "./props";

const POLL_MILLISECONDS = 2000;

type Job = AcquireJob;

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** View 6: works not held; `f` fetches the selected one, `F` every fetchable one, one after another. */
export function Acquire(props: ListViewProps) {
  const { detail, actions, selectedId, keyboardEnabled } = props;
  const model = useMemo(() => acquire(detail), [detail]);
  // Per-Work job state: running while polled, failed with the server message until fetched again.
  const [jobs, setJobs] = useState<ReadonlyMap<string, Job>>(() => new Map());
  const jobsRef = useRef(jobs);
  useEffect(() => {
    jobsRef.current = jobs;
  }, [jobs]);
  const fetchable = useMemo(() => new Set(model.fetchable), [model]);

  const setJob = (workId: string, job: Job | null): void => {
    setJobs((current) => {
      const next = new Map(current);
      if (job === null) next.delete(workId);
      else next.set(workId, job);
      return next;
    });
  };

  const mounted = useRef(true);
  const isMounted = (): boolean => mounted.current;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Fetch one Work: queue it, then poll it alone until its job ends.
  const fetchOne = async (workId: string): Promise<void> => {
    try {
      await api.fetchWork(workId);
    } catch (reason: unknown) {
      setJob(workId, null);
      actions.fail(reason instanceof Error ? reason.message : "The fetch failed.");
      return;
    }
    // The POST has returned, so an idle status now means the job is over.
    for (;;) {
      await new Promise<void>((resolve) => window.setTimeout(resolve, POLL_MILLISECONDS));
      if (!isMounted()) return;
      try {
        const status = await api.workAcquisition(workId);
        const step = jobAfterPoll(status.job.state, status.job.message);
        if (!step.finished) continue;
        if (!isMounted()) return;
        setJob(workId, step.job);
        void actions.reload();
        return;
      } catch (reason: unknown) {
        if (!isMounted()) return;
        setJob(workId, { state: "failed", message: reason instanceof Error ? reason.message : null });
        return;
      }
    }
  };

  const fetchWorks = (requested: readonly string[]): void => {
    if (requested.length === 0) {
      actions.notify("Nothing to fetch here.");
      return;
    }
    const workIds = requested.filter((workId) => jobsRef.current.get(workId)?.state !== "running");
    if (workIds.length === 0) {
      actions.notify("Those fetches are already running.");
      return;
    }
    // All show as running (queued) at once; the loop below runs them strictly one after another.
    setJobs((current) => {
      const next = new Map(current);
      for (const workId of workIds) next.set(workId, { state: "running", message: null });
      return next;
    });
    void (async () => {
      for (const workId of workIds) {
        if (!isMounted()) return;
        await fetchOne(workId);
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
      const job = row.workId === null ? undefined : jobs.get(row.workId);
      const text = job?.state === "failed"
        ? `${acquisitionStatusText(view, "failed")}${job.message === null ? "" : ` ${job.message}`}`
        : acquisitionStatusText(view, job?.state === "running" && fetchable.has(row.workId ?? "") ? "running" : "none");
      return <StubRow key={row.id} row={row} props={props} detail={text} />;
    })}
  </ol>;
}
