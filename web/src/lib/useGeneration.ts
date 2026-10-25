import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "./api";
import type { GenerateRequest, GenerationStatus } from "../types";

const POLL_MILLISECONDS = 1000;

export type GenerationRun = {
  id: string;
  goal: string;
  /** The list being extended, or null for a new list. */
  intoListId: string | null;
  status: GenerationStatus | null;
  error: string | null;
};

/**
 * One list generation at a time: start it, poll it once a second until it leaves `running`,
 * then call `onFinished` once. Generation runs on the server, so a closed tab loses only the
 * progress strip, not the job.
 */
export function useGeneration(onFinished: (run: GenerationRun) => void) {
  const [run, setRun] = useState<GenerationRun | null>(null);
  const finished = useRef(onFinished);
  const latest = useRef<GenerationRun | null>(null);
  useEffect(() => { finished.current = onFinished; }, [onFinished]);
  useEffect(() => { latest.current = run; }, [run]);
  // Keep polling until a terminal status arrives, even after a failed poll: the server job is still running.
  const runId = run !== null && (run.status === null || run.status.state === "running") ? run.id : null;

  useEffect(() => {
    if (runId === null) return;
    const controller = new AbortController();
    const timer = window.setInterval(() => {
      void api.generation(runId, controller.signal).then((status) => {
        if (controller.signal.aborted) return;
        const base = latest.current;
        if (base === null || base.id !== runId) return;
        const next = { ...base, status, error: null };
        latest.current = next;
        setRun(next);
        if (status.state !== "running") finished.current(next);
      }).catch((reason: unknown) => {
        if (controller.signal.aborted) return;
        const message = reason instanceof Error ? reason.message : "Could not read the generation status.";
        setRun((current) => current === null || current.id !== runId ? current : { ...current, error: message });
      });
    }, POLL_MILLISECONDS);
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [runId]);

  const start = useCallback(async (request: GenerateRequest): Promise<void> => {
    const { generation_id: id } = await api.generateList(request);
    latest.current = null;
    setRun({ id, goal: request.goal, intoListId: request.into_list ?? null, status: null, error: null });
  }, []);

  const dismiss = useCallback((): void => setRun(null), []);
  return { run, start, dismiss };
}
