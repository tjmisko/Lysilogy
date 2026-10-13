import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "./api";
import { missingAncestors, pickCurrentRamp, rampStorageKey, swapOrder } from "./ramp";
import type { RampDetail, RampItemStatus, RampItemView, RampSummaryView } from "../types";

/** What `addWork` did, for the notice the canvas shows. */
export type RampAddResult =
  | { kind: "added" | "exists" | "started"; message: string }
  | { kind: "error"; message: string };

type Outcome<T> = { ok: true; value: T } | { ok: false; message: string };

function readRemembered(centerId: string): string | null {
  try {
    return window.localStorage.getItem(rampStorageKey(centerId));
  } catch {
    return null;
  }
}

function remember(centerId: string, rampId: string): void {
  try {
    window.localStorage.setItem(rampStorageKey(centerId), rampId);
  } catch {
    // Storage can be blocked or full; the ramp still works without being remembered.
  }
}

function failureMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "The ramp request failed.";
}

function byUpdatedDescending(left: RampSummaryView, right: RampSummaryView): number {
  return left.updated_at < right.updated_at ? 1 : left.updated_at > right.updated_at ? -1 : 0;
}

function countStatuses(items: readonly RampItemView[]): RampSummaryView["status_counts"] {
  const counts = { unread: 0, reading: 0, read: 0 };
  for (const item of items) counts[item.status] += 1;
  return counts;
}

/** The detail with one item replaced by the server's copy and the counts recomputed. */
function withItem(detail: RampDetail, updated: RampItemView): RampDetail {
  const items = detail.items.map((item) => item.work.id === updated.work.id ? updated : item);
  return { ...detail, items, ramp: { ...detail.ramp, status_counts: countStatuses(items) } };
}

/**
 * The reading ramps seeded at one Work and the current one's detail. Every call
 * returns after the server answered; failures land in `error`.
 */
export function useRamps(centerId: string | undefined) {
  const [ramps, setRamps] = useState<RampSummaryView[]>([]);
  const [detail, setDetail] = useState<RampDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const detailRef = useRef<RampDetail | null>(null);
  const busy = useRef(false);

  const applyDetail = useCallback((next: RampDetail | null): void => {
    detailRef.current = next;
    setDetail(next);
    if (next === null) return;
    setRamps((current) => [next.ramp, ...current.filter((ramp) => ramp.id !== next.ramp.id)].sort(byUpdatedDescending));
  }, []);

  useEffect(() => {
    if (centerId === undefined) return;
    const controller = new AbortController();
    const aborted = (): boolean => controller.signal.aborted;
    void (async () => {
      try {
        const listed = (await api.listRamps(centerId, controller.signal)).ramps;
        const current = pickCurrentRamp(listed, readRemembered(centerId));
        if (aborted()) return;
        setRamps(listed);
        const loadedDetail = current === null ? null : await api.ramp(current.id, controller.signal);
        if (aborted()) return;
        applyDetail(loadedDetail);
        setError(null);
      } catch (reason: unknown) {
        if (!aborted()) setError(failureMessage(reason));
      } finally {
        if (!aborted()) setLoaded(true);
      }
    })();
    return () => controller.abort();
  }, [applyDetail, centerId]);

  /** Run one change at a time; report a failure instead of throwing. */
  const guarded = useCallback(async <T,>(run: () => Promise<T>): Promise<Outcome<T>> => {
    if (busy.current) {
      const message = "Another ramp change is still running.";
      setError(message);
      return { ok: false, message };
    }
    busy.current = true;
    setError(null);
    try {
      return { ok: true, value: await run() };
    } catch (reason: unknown) {
      const message = failureMessage(reason);
      setError(message);
      return { ok: false, message };
    } finally {
      busy.current = false;
    }
  }, []);

  const select = useCallback((rampId: string): Promise<void> => guarded(async () => {
    const loadedDetail = await api.ramp(rampId);
    if (centerId !== undefined) remember(centerId, rampId);
    applyDetail(loadedDetail);
  }).then(() => undefined), [applyDetail, centerId, guarded]);

  const create = useCallback((): Promise<void> => guarded(async () => {
    if (centerId === undefined) return;
    const created = await api.createRamp(centerId);
    remember(centerId, created.ramp.id);
    applyDetail(created);
  }).then(() => undefined), [applyDetail, centerId, guarded]);

  const rename = useCallback((title: string): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    const summary = await api.renameRamp(current.ramp.id, title);
    applyDetail({ ...current, ramp: summary });
  }).then(() => undefined), [applyDetail, guarded]);

  const setStatus = useCallback((workId: string, status: RampItemStatus): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    const updated = await api.updateRampItem(current.ramp.id, workId, { status });
    applyDetail(withItem(detailRef.current ?? current, updated));
  }).then(() => undefined), [applyDetail, guarded]);

  const setNote = useCallback((workId: string, note: string | null): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    const updated = await api.updateRampItem(current.ramp.id, workId, { note });
    applyDetail(withItem(detailRef.current ?? current, updated));
  }).then(() => undefined), [applyDetail, guarded]);

  const move = useCallback((workId: string, delta: number): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    const ids = current.items.map((item) => item.work.id);
    const swapped = swapOrder(ids, ids.indexOf(workId), delta);
    if (swapped.every((id, index) => id === ids[index])) return;
    applyDetail(await api.orderRamp(current.ramp.id, { work_ids: swapped }));
  }).then(() => undefined), [applyDetail, guarded]);

  const suggest = useCallback((): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    applyDetail(await api.orderRamp(current.ramp.id, { strategy: "foundational_first" }));
  }).then(() => undefined), [applyDetail, guarded]);

  const remove = useCallback((workId: string): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    await api.removeRampItem(current.ramp.id, workId);
    applyDetail(await api.ramp(current.ramp.id));
  }).then(() => undefined), [applyDetail, guarded]);

  /** Read the current ramp again, e.g. after a fetch landed a paper. */
  const reload = useCallback((): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    applyDetail(await api.ramp(current.ramp.id));
  }).then(() => undefined), [applyDetail, guarded]);

  /**
   * Add a Work to the current ramp. `path` runs from the center to the Work; its
   * ancestors that the ramp lacks are added first, each via its predecessor. With no
   * current ramp one is created, seeded at the center.
   */
  const addWork = useCallback(async (workId: string, path: readonly string[]): Promise<RampAddResult> => {
    const outcome = await guarded<RampAddResult>(async () => {
      if (centerId === undefined) return { kind: "error", message: "There is no paper to seed a ramp." };
      let current = detailRef.current;
      let started = false;
      if (current === null) {
        current = await api.createRamp(centerId);
        remember(centerId, current.ramp.id);
        applyDetail(current);
        started = true;
      }
      if (current.items.some((item) => item.work.id === workId)) {
        return started
          ? { kind: "started", message: "Started a ramp at this paper." }
          : { kind: "exists", message: "Already in ramp" };
      }
      const rampId = current.ramp.id;
      const ancestors = missingAncestors(current.items, path);
      for (const ancestor of ancestors) {
        const via = path[path.indexOf(ancestor) - 1];
        await api.addRampItem(rampId, via === undefined ? { work_id: ancestor } : { work_id: ancestor, via_work_id: via });
      }
      const via = path[path.length - 2];
      await api.addRampItem(rampId, via === undefined ? { work_id: workId } : { work_id: workId, via_work_id: via });
      applyDetail(await api.ramp(rampId));
      const extra = ancestors.length === 0 ? "" : ` with ${ancestors.length} ${ancestors.length === 1 ? "work" : "works"} leading to it`;
      return { kind: "added", message: `Added to ramp${extra}.` };
    });
    return outcome.ok ? outcome.value : { kind: "error", message: outcome.message };
  }, [applyDetail, centerId, guarded]);

  return { ramps, detail, error, loaded, select, create, rename, setStatus, setNote, move, suggest, remove, reload, addWork };
}

export type RampsApi = ReturnType<typeof useRamps>;
