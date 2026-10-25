import { useEffect, useRef, useState } from "react";

import { api } from "../lib/api";
import { announceListsChanged, type PickerTarget } from "../lib/listEvents";
import { listErrorMessage } from "../lib/listKeys";
import { restoreRemovedNode } from "../lib/useListDetail";
import type { ListMembership as Membership, RemovedListNode } from "../types";
import { DialogShell } from "./ListDialogShell";
import { dialogDirection, useDialogKeys } from "../lib/dialogKeys";

type Undo = { listId: string; title: string; removed: RemovedListNode };

/** The lists a paper or Work is in (`M`): `↵` opens one, `x` removes it from that list, `u` undoes. */
export function ListMembership(props: {
  target: PickerTarget;
  onClose: () => void;
  onOpenList: (listId: string) => void;
  /** Called after a removal or undo so the paper heading's chips refresh. */
  onChanged: () => void;
}) {
  const { target, onClose, onOpenList, onChanged } = props;
  const [memberships, setMemberships] = useState<Membership[] | null>(null);
  const [linked, setLinked] = useState(true);
  const [active, setActive] = useState(0);
  const [message, setMessage] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [reloads, setReloads] = useState(0);
  const undo = useRef<Undo | null>(null);
  const busy = useRef(false);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    const controller = new AbortController();
    const load = target.workId !== undefined
      ? api.workLists(target.workId, controller.signal).then((found) => ({ list: found.memberships, linked: true }))
      : target.paperId !== undefined
        ? api.paperLists(target.paperId, controller.signal).then((found) => ({ list: found.memberships, linked: found.work_id !== null }))
        : Promise.resolve({ list: [] as Membership[], linked: false });
    void load.then((found) => {
      if (controller.signal.aborted) return;
      setMemberships(found.list);
      setLinked(found.linked);
    }).catch((reason: unknown) => {
      if (controller.signal.aborted) return;
      setMemberships([]);
      setProblem(reason instanceof Error ? reason.message : "Could not load the lists.");
    });
    return () => controller.abort();
  }, [reloads, target.paperId, target.workId]);

  const rows = memberships ?? [];
  const last = Math.max(0, rows.length - 1);
  const at = Math.min(active, last);

  useEffect(() => {
    itemRefs.current[at]?.scrollIntoView({ block: "nearest" });
  }, [at]);

  const remove = async (membership: Membership): Promise<void> => {
    busy.current = true;
    setProblem(null);
    try {
      const removed = await api.removeListNode(membership.list_id, membership.node_id);
      undo.current = { listId: membership.list_id, title: membership.title, removed };
      setMessage(`Removed from “${membership.title}”. Press u to undo.`);
      setMemberships((current) => current?.filter((entry) => entry.node_id !== membership.node_id) ?? current);
      announceListsChanged();
      onChanged();
    } catch (reason: unknown) {
      setProblem(reason instanceof Error ? reason.message : "Could not remove.");
    } finally {
      busy.current = false;
    }
  };

  const restore = async (): Promise<void> => {
    const pending = undo.current;
    if (pending === null) {
      setMessage("Nothing to undo.");
      return;
    }
    busy.current = true;
    setProblem(null);
    try {
      await restoreRemovedNode(pending.listId, pending.removed);
      undo.current = null;
      setMessage(`Put back on “${pending.title}”.`);
      setReloads((count) => count + 1);
      announceListsChanged();
      onChanged();
    } catch (reason: unknown) {
      setProblem(reason instanceof Error ? listErrorMessage(reason) : "Could not undo.");
    } finally {
      busy.current = false;
    }
  };

  useDialogKeys((event) => {
    if (event.key === "Escape" || event.key === "q" || event.key === "M") {
      onClose();
      return true;
    }
    if (busy.current) return true;
    const move = dialogDirection(event);
    if (move !== null) {
      setActive((index) => Math.max(0, Math.min(last, index + move)));
      return true;
    }
    const current = rows[at];
    if (event.key === "Enter" && current !== undefined) {
      onOpenList(current.list_id);
      return true;
    }
    if (event.key === "x" && current !== undefined) {
      void remove(current);
      return true;
    }
    if (event.key === "u") {
      void restore();
      return true;
    }
    return false;
  });

  const label = target.title ?? "this paper";
  return (
    <DialogShell
      eyebrow={label.length > 70 ? `${label.slice(0, 69)}…` : label}
      title="On these lists"
      hint="M"
      onClose={onClose}
      footer={<>
        <span><kbd>j/k</kbd> move</span>
        <span><kbd>↵</kbd> open</span>
        <span><kbd>x</kbd> remove</span>
        <span><kbd>u</kbd> undo</span>
        <span><kbd>esc</kbd> close</span>
      </>}
    >
      <div className="switcher-results" role="listbox">
        {rows.map((membership, index) => (
          <button
            key={membership.node_id}
            ref={(node) => { itemRefs.current[index] = node; }}
            type="button"
            role="option"
            aria-selected={index === at}
            className={index === at ? "is-active" : ""}
            tabIndex={-1}
            onMouseEnter={() => setActive(index)}
            onClick={() => onOpenList(membership.list_id)}
          >
            <span className="list-choice-mark" aria-hidden="true" />
            <span>
              <strong>{membership.title}</strong>
              <small>{[membership.topics.join(", "), membership.note ?? ""].filter((part) => part !== "").join(" · ") || "No topic"}</small>
            </span>
            {membership.archived && <em>archived</em>}
          </button>
        ))}
        {memberships === null && <p>Loading…</p>}
        {memberships !== null && rows.length === 0 && (
          <p>{linked ? "Not on any list. Press Y to add it to one." : "This paper is not linked to a Work yet, so it cannot be on a list. Press Y to link and add it."}</p>
        )}
      </div>
      {(message !== null || problem !== null) && (
        <p className={problem === null ? "list-dialog-note" : "list-dialog-problem"} role={problem === null ? "status" : "alert"}>{problem ?? message}</p>
      )}
    </DialogShell>
  );
}
