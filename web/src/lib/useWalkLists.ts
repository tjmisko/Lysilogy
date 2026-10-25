import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError, api } from "./api";
import { LISTS_CHANGED_EVENT } from "./listEvents";
import { listErrorMessage, manualOrder, sameOrder, stepAmongRows } from "./listKeys";
import { listStorageKey, missingAncestors, pickCurrentList, walkListTitle } from "./listSidebar";
import { itemRows } from "./listViews";
import type { ListDetail, ListNodeAdd, ListSummaryView, ReadingStatus } from "../types";

/** What `addWork` did, for the notice the canvas shows. */
export type ListAddResult =
  | { kind: "added" | "exists" | "started"; message: string }
  | { kind: "error"; message: string };

type Outcome<T> = { ok: true; value: T } | { ok: false; message: string };

function readRemembered(centerId: string): string | null {
  try {
    return window.localStorage.getItem(listStorageKey(centerId));
  } catch {
    return null;
  }
}

function remember(centerId: string, listId: string): void {
  try {
    window.localStorage.setItem(listStorageKey(centerId), listId);
  } catch {
    // Storage can be blocked or full; the list still works without being remembered.
  }
}

function failureMessage(reason: unknown): string {
  if (reason instanceof ApiError) return listErrorMessage(reason);
  return reason instanceof Error ? reason.message : "The list request failed.";
}

function byUpdatedDescending(left: ListSummaryView, right: ListSummaryView): number {
  return left.updated_at < right.updated_at ? 1 : left.updated_at > right.updated_at ? -1 : 0;
}

function countStatuses(detail: ListDetail): ListSummaryView["status_counts"] {
  const counts = { unread: 0, reading: 0, read: 0 };
  for (const row of itemRows(detail)) {
    if (row.kind === "work") counts[row.status] += 1;
  }
  return counts;
}

/**
 * The reading lists a paper's graph works with and the current one's detail: the lists
 * seeded at the center Work (and any list picked by hand). Every call returns after the
 * server answered; failures land in `error`.
 */
export function useWalkLists(centerId: string | undefined, centerTitle?: string | null) {
  const [lists, setLists] = useState<ListSummaryView[]>([]);
  const [detail, setDetail] = useState<ListDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const detailRef = useRef<ListDetail | null>(null);
  const busy = useRef(false);
  const titleRef = useRef(centerTitle);
  useEffect(() => {
    titleRef.current = centerTitle;
  }, [centerTitle]);

  const applyDetail = useCallback((next: ListDetail | null): void => {
    detailRef.current = next;
    setDetail(next);
    if (next === null) return;
    setLists((current) => [next.list, ...current.filter((list) => list.id !== next.list.id)].sort(byUpdatedDescending));
  }, []);

  useEffect(() => {
    if (centerId === undefined) return;
    const controller = new AbortController();
    const aborted = (): boolean => controller.signal.aborted;
    void (async () => {
      try {
        const listed = (await api.listLists({}, controller.signal)).lists;
        const current = pickCurrentList(listed, readRemembered(centerId), centerId);
        if (aborted()) return;
        setLists(listed);
        const loadedDetail = current === null ? null : await api.readList(current.id, controller.signal);
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
      const message = "Another list change is still running.";
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

  /** Read the lists and the current list again; the picker and other pages change lists too. */
  const refresh = useCallback((): Promise<void> => guarded(async () => {
    const listed = (await api.listLists()).lists;
    setLists(listed);
    const current = detailRef.current;
    if (current === null) return;
    if (!listed.some((list) => list.id === current.list.id)) {
      detailRef.current = null;
      setDetail(null);
      return;
    }
    applyDetail(await api.readList(current.list.id));
  }).then(() => undefined), [applyDetail, guarded]);

  useEffect(() => {
    if (centerId === undefined) return;
    const onChanged = (): void => void refresh();
    window.addEventListener(LISTS_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(LISTS_CHANGED_EVENT, onChanged);
  }, [centerId, refresh]);

  const select = useCallback((listId: string): Promise<void> => guarded(async () => {
    const loadedDetail = await api.readList(listId);
    if (centerId !== undefined) remember(centerId, listId);
    applyDetail(loadedDetail);
  }).then(() => undefined), [applyDetail, centerId, guarded]);

  const create = useCallback((): Promise<void> => guarded(async () => {
    if (centerId === undefined) return;
    const created = await api.createList({ title: walkListTitle(titleRef.current), seed_work_id: centerId });
    remember(centerId, created.list.id);
    applyDetail(created);
  }).then(() => undefined), [applyDetail, centerId, guarded]);

  const rename = useCallback((title: string): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    const summary = await api.patchList(current.list.id, { title });
    applyDetail({ ...current, list: summary });
  }).then(() => undefined), [applyDetail, guarded]);

  const setStatus = useCallback((workId: string, status: ReadingStatus): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    const state = await api.setReadingState(workId, status);
    const latest = detailRef.current ?? current;
    const next = { ...latest, reading_states: { ...latest.reading_states, [workId]: { status: state.status, changed_at: state.changed_at } } };
    applyDetail({ ...next, list: { ...next.list, status_counts: countStatuses(next) } });
  }).then(() => undefined), [applyDetail, guarded]);

  const setNote = useCallback((nodeId: string, note: string | null): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    // The API clears a note with an empty string.
    const updated = await api.patchListNode(current.list.id, nodeId, { note: note ?? "" });
    const latest = detailRef.current ?? current;
    applyDetail({ ...latest, nodes: latest.nodes.map((node) => node.id === updated.id ? updated : node) });
  }).then(() => undefined), [applyDetail, guarded]);

  const move = useCallback((nodeId: string, delta: -1 | 1): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    const order = manualOrder(current.nodes);
    const rowIds = itemRows(current).filter((row) => row.kind === "work").map((row) => row.id);
    const moved = stepAmongRows(order, rowIds, nodeId, delta);
    if (sameOrder(moved, order)) return;
    applyDetail(await api.orderList(current.list.id, { node_ids: moved }));
  }).then(() => undefined), [applyDetail, guarded]);

  const suggest = useCallback((): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    applyDetail(await api.orderList(current.list.id, { strategy: "foundational_first" }));
  }).then(() => undefined), [applyDetail, guarded]);

  const remove = useCallback((nodeId: string): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    await api.removeListNode(current.list.id, nodeId);
    applyDetail(await api.readList(current.list.id));
  }).then(() => undefined), [applyDetail, guarded]);

  /** Read the current list again, e.g. after a fetch landed a paper. */
  const reload = useCallback((): Promise<void> => guarded(async () => {
    const current = detailRef.current;
    if (current === null) return;
    applyDetail(await api.readList(current.list.id));
  }).then(() => undefined), [applyDetail, guarded]);

  /**
   * Add a Work to the current list. `path` runs from the center to the Work; its ancestors
   * that the list lacks are added first, each via its predecessor. With no current list one is
   * created, seeded at the center.
   */
  const addWork = useCallback(async (workId: string, path: readonly string[]): Promise<ListAddResult> => {
    const outcome = await guarded<ListAddResult>(async () => {
      if (centerId === undefined) return { kind: "error", message: "There is no paper to seed a list." };
      let current = detailRef.current;
      let started = false;
      if (current === null) {
        current = await api.createList({ title: walkListTitle(titleRef.current), seed_work_id: centerId });
        remember(centerId, current.list.id);
        applyDetail(current);
        started = true;
      }
      if (current.nodes.some((node) => node.work_id === workId)) {
        return started
          ? { kind: "started", message: "Started a list at this paper." }
          : { kind: "exists", message: "Already in list" };
      }
      const listId = current.list.id;
      const ancestors = missingAncestors(current.nodes, path);
      const request = (id: string): ListNodeAdd => {
        const via = path[path.indexOf(id) - 1];
        return via === undefined ? { work_id: id } : { work_id: id, via_work_id: via };
      };
      for (const ancestor of ancestors) {
        await api.addListNode(listId, request(ancestor));
      }
      const via = path[path.length - 2];
      await api.addListNode(listId, via === undefined ? { work_id: workId } : { work_id: workId, via_work_id: via });
      applyDetail(await api.readList(listId));
      const extra = ancestors.length === 0 ? "" : ` with ${ancestors.length} ${ancestors.length === 1 ? "work" : "works"} leading to it`;
      return { kind: "added", message: `Added to list${extra}.` };
    });
    return outcome.ok ? outcome.value : { kind: "error", message: outcome.message };
  }, [applyDetail, centerId, guarded]);

  return { lists, detail, error, loaded, select, create, rename, setStatus, setNote, move, suggest, remove, reload, refresh, addWork };
}

export type WalkListsApi = ReturnType<typeof useWalkLists>;
