import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError, api } from "./api";
import { listErrorMessage, orderAfterRestore } from "./listKeys";
import type {
  ListDetail,
  ListEdgeRef,
  ListNodeAdd,
  ListNodeView,
  ListOrderStrategy,
  ReadingStatus,
  RemovedListNode,
  VerifyReport,
} from "../types";

export type ListMessage = { kind: "notice" | "error"; text: string };

/** What a list view can do to the list it shows. Every call resolves after the server answered. */
export type ListActions = {
  setStatus: (workId: string, status: ReadingStatus) => Promise<void>;
  setNote: (nodeId: string, note: string | null) => Promise<void>;
  /** Removes the node; `undo` brings it back. One level. */
  remove: (nodeId: string) => Promise<void>;
  undo: () => Promise<boolean>;
  canUndo: boolean;
  /** An exact permutation of every node id. */
  reorder: (nodeIds: string[]) => Promise<void>;
  orderBy: (strategy: ListOrderStrategy) => Promise<void>;
  addNode: (add: ListNodeAdd) => Promise<ListNodeView | null>;
  addNodes: (adds: ListNodeAdd[]) => Promise<ListNodeView[] | null>;
  addEdge: (edge: ListEdgeRef) => Promise<boolean>;
  removeEdge: (edge: ListEdgeRef) => Promise<boolean>;
  acceptNode: (nodeId: string) => Promise<void>;
  acceptEdge: (edge: ListEdgeRef) => Promise<void>;
  resolveClaim: (nodeId: string, workId: string) => Promise<void>;
  verify: () => Promise<VerifyReport | null>;
  reload: () => Promise<void>;
  notify: (text: string) => void;
  fail: (text: string) => void;
};

function titleOfNode(detail: ListDetail | null, nodeId: string): string {
  const node = detail?.nodes.find((entry) => entry.id === nodeId);
  return node?.work?.display_title ?? node?.claim?.title ?? node?.label ?? nodeId.slice(0, 8);
}

function describe(reason: unknown, detail: ListDetail | null): string {
  if (reason instanceof ApiError) return listErrorMessage(reason, (id) => titleOfNode(detail, id));
  return reason instanceof Error ? reason.message : "The request failed.";
}

/** The node's add request, for undoing its removal. */
function addForRemoved(node: RemovedListNode["node"], withVia: boolean): ListNodeAdd | null {
  switch (node.kind) {
    case "work": {
      if (node.work_id === null) return null;
      return {
        work_id: node.work_id,
        ...(withVia && node.via_work_id !== null ? { via_work_id: node.via_work_id } : {}),
        ...(node.note === null ? {} : { note: node.note }),
      };
    }
    case "topic":
      return node.label === null ? null : { topic: { label: node.label, ...(node.summary === null ? {} : { summary: node.summary }) } };
    case "claim":
      return node.claim === null ? null : { claim: node.claim, ...(node.note === null ? {} : { note: node.note }) };
  }
}

/**
 * Put a removed node, its edges and its place in the order back. Returns the list as it then
 * stands. An edge the server now refuses (a cycle made since) is skipped.
 */
export async function restoreRemovedNode(listId: string, removed: RemovedListNode): Promise<ListDetail> {
  const first = addForRemoved(removed.node, true);
  if (first === null) throw new Error("That item cannot be restored.");
  let restored: ListNodeView;
  try {
    restored = await api.addListNode(listId, first);
  } catch (reason: unknown) {
    const retry = addForRemoved(removed.node, false);
    // A walk-through paper that has since left the list only fails the `via` check.
    if (!(reason instanceof ApiError) || reason.status !== 409 || retry === null || removed.node.via_work_id === null) throw reason;
    restored = await api.addListNode(listId, retry);
  }
  for (const edge of removed.edges) {
    const from = edge.from === removed.node.id ? restored.id : edge.from;
    const to = edge.to === removed.node.id ? restored.id : edge.to;
    try {
      await api.addListEdge(listId, { from, to, kind: edge.kind });
    } catch {
      // The graph moved on; a link that no longer fits is left out.
    }
  }
  const detail = await api.readList(listId);
  const others = detail.nodes.filter((node) => node.id !== restored.id);
  const order = orderAfterRestore(others, removed.node.position, restored.id);
  return api.orderList(listId, { node_ids: order });
}

/**
 * One list and the operations on it. Mutations run one at a time, each patching the local
 * copy from the server's answer (or reloading when it cannot).
 */
export function useListDetail(listId: string | null) {
  const [detail, setDetail] = useState<ListDetail | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [message, setMessage] = useState<ListMessage | null>(null);
  const [canUndo, setCanUndo] = useState(false);
  const detailRef = useRef<ListDetail | null>(null);
  const idRef = useRef<string | null>(listId);
  const chain = useRef<Promise<unknown>>(Promise.resolve());
  const removed = useRef<RemovedListNode | null>(null);

  const apply = useCallback((next: ListDetail | null): void => {
    detailRef.current = next;
    setDetail(next);
  }, []);

  // The page is keyed by list, so a new list is a new hook; this loads the one it was given.
  useEffect(() => {
    idRef.current = listId;
    if (listId === null) return;
    const controller = new AbortController();
    void api.readList(listId, controller.signal).then((next) => {
      if (controller.signal.aborted) return;
      apply(next);
      setLoaded(true);
    }).catch((reason: unknown) => {
      if (controller.signal.aborted) return;
      setLoaded(true);
      setMessage({ kind: "error", text: describe(reason, null) });
    });
    return () => controller.abort();
  }, [apply, listId]);

  /** Run `work` after earlier mutations; failures become an error message and yield null. */
  const run = useCallback(<T,>(work: (id: string) => Promise<T>): Promise<T | null> => {
    const task = chain.current.then(async () => {
      const id = idRef.current;
      if (id === null) return null;
      try {
        return await work(id);
      } catch (reason: unknown) {
        setMessage({ kind: "error", text: describe(reason, detailRef.current) });
        return null;
      }
    });
    chain.current = task;
    return task;
  }, []);

  const reload = useCallback(async (): Promise<void> => {
    await run(async (id) => {
      const next = await api.readList(id);
      if (idRef.current === id) apply(next);
    });
  }, [apply, run]);

  const patchNode = useCallback((updated: ListNodeView): void => {
    const current = detailRef.current;
    if (current === null) return;
    apply({ ...current, nodes: current.nodes.map((node) => node.id === updated.id ? updated : node) });
  }, [apply]);

  const actions: ListActions = {
    setStatus: async (workId, status) => {
      await run(async () => {
        const state = await api.setReadingState(workId, status);
        const current = detailRef.current;
        if (current === null) return;
        apply({ ...current, reading_states: { ...current.reading_states, [workId]: { status: state.status, changed_at: state.changed_at } } });
      });
    },
    setNote: async (nodeId, note) => {
      await run(async (id) => {
        // The API clears a note with an empty string.
        patchNode(await api.patchListNode(id, nodeId, { note: note ?? "" }));
      });
    },
    remove: async (nodeId) => {
      await run(async (id) => {
        const result = await api.removeListNode(id, nodeId);
        removed.current = result;
        setCanUndo(true);
        const current = detailRef.current;
        if (current === null) return;
        apply({
          ...current,
          nodes: current.nodes.filter((node) => node.id !== nodeId),
          edges: current.edges.filter((edge) => edge.from !== nodeId && edge.to !== nodeId),
        });
        // Ordinals, depth and counts shift; the server's copy is the source of truth.
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
      });
    },
    undo: async () => {
      const result = await run(async (id) => {
        const pending = removed.current;
        if (pending === null) return false;
        const next = await restoreRemovedNode(id, pending);
        removed.current = null;
        setCanUndo(false);
        if (idRef.current === id) apply(next);
        return true;
      });
      return result === true;
    },
    canUndo,
    reorder: async (nodeIds) => {
      await run(async (id) => {
        const next = await api.orderList(id, { node_ids: nodeIds });
        if (idRef.current === id) apply(next);
      });
    },
    orderBy: async (strategy) => {
      await run(async (id) => {
        const next = await api.orderList(id, { strategy });
        if (idRef.current === id) apply(next);
      });
    },
    addNode: async (add) => {
      return run(async (id) => {
        const node = await api.addListNode(id, add);
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
        return node;
      });
    },
    addNodes: async (adds) => {
      return run(async (id) => {
        const { nodes } = await api.addListNodes(id, adds);
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
        return nodes;
      });
    },
    addEdge: async (edge) => {
      const done = await run(async (id) => {
        await api.addListEdge(id, edge);
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
        return true;
      });
      return done === true;
    },
    removeEdge: async (edge) => {
      const done = await run(async (id) => {
        await api.removeListEdge(id, edge);
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
        return true;
      });
      return done === true;
    },
    acceptNode: async (nodeId) => {
      await run(async (id) => {
        await api.patchListNode(id, nodeId, { review: "accepted" });
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
      });
    },
    acceptEdge: async (edge) => {
      await run(async (id) => {
        await api.acceptListEdge(id, edge);
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
      });
    },
    resolveClaim: async (nodeId, workId) => {
      await run(async (id) => {
        await api.patchListNode(id, nodeId, { resolve_to: workId });
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
      });
    },
    verify: async () => {
      return run(async (id) => {
        const report = await api.verifyList(id);
        const fresh = await api.readList(id);
        if (idRef.current === id) apply(fresh);
        return report;
      });
    },
    reload,
    notify: (text) => setMessage({ kind: "notice", text }),
    fail: (text) => setMessage({ kind: "error", text }),
  };

  return { detail, loaded, message, clearMessage: () => setMessage(null), actions };
}
