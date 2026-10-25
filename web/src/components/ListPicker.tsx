import { useEffect, useMemo, useRef, useState } from "react";

import { ApiError, api } from "../lib/api";
import { listErrorMessage } from "../lib/listKeys";
import { announceListsChanged, type PickerTarget } from "../lib/listEvents";
import { rankByTitle } from "../lib/listSearch";
import type { ListNodeAdd, ListNodeView, ListSummaryView } from "../types";
import { DialogShell } from "./ListDialogShell";
import { dialogDirection, useDialogKeys } from "../lib/dialogKeys";

type Stage =
  | { kind: "lists" }
  | { kind: "topics"; list: ListSummaryView; topics: ListNodeView[]; active: number };

const NO_TOPIC = "";

function targetAdd(target: PickerTarget, topicIds: string[]): ListNodeAdd | null {
  const topics = topicIds.length === 0 ? {} : { topic_ids: topicIds };
  if (target.workId !== undefined) return { work_id: target.workId, ...topics };
  if (target.paperId !== undefined) return { paper_id: target.paperId, ...topics };
  return null;
}

/** Add the target to a list: fuzzy-find, `↵` add, `ctrl+↵` create from the typed title, `Tab` pick a topic first. */
export function ListPicker(props: {
  target: PickerTarget;
  onClose: () => void;
  /** Called after a successful add with a line for the status toast. */
  onDone: (message: string) => void;
}) {
  const { target, onClose, onDone } = props;
  const [lists, setLists] = useState<ListSummaryView[] | null>(null);
  const [memberOf, setMemberOf] = useState<ReadonlySet<string>>(() => new Set());
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [stage, setStage] = useState<Stage>({ kind: "lists" });
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => { inputRef.current?.focus(); }, []);

  useEffect(() => {
    const controller = new AbortController();
    void api.listLists({}, controller.signal).then((found) => {
      if (!controller.signal.aborted) setLists(found.lists);
    }).catch((reason: unknown) => {
      if (controller.signal.aborted) return;
      setLists([]);
      setProblem(reason instanceof Error ? reason.message : "Could not load the lists.");
    });
    const memberships = target.workId !== undefined
      ? api.workLists(target.workId, controller.signal)
      : target.paperId !== undefined ? api.paperLists(target.paperId, controller.signal) : null;
    void memberships?.then((found) => {
      if (!controller.signal.aborted) setMemberOf(new Set(found.memberships.map((entry) => entry.list_id)));
    }).catch(() => {
      // Without memberships the picker still adds; the server answers item_exists.
    });
    return () => controller.abort();
  }, [target.paperId, target.workId]);

  const ranked = useMemo(() => rankByTitle(lists ?? [], query), [lists, query]);
  const last = Math.max(0, ranked.length - 1);
  const at = Math.min(active, last);
  const stageLast = stage.kind === "topics" ? stage.topics.length : 0;

  useEffect(() => {
    itemRefs.current[stage.kind === "lists" ? at : stage.active]?.scrollIntoView({ block: "nearest" });
  }, [at, stage]);

  const finish = (list: { title: string }): void => {
    announceListsChanged();
    onDone(`Added to “${list.title}”.`);
  };

  const add = async (list: ListSummaryView, topicIds: string[]): Promise<void> => {
    const request = targetAdd(target, topicIds);
    if (request === null) {
      setProblem("Nothing to add: this page has no paper or Work.");
      return;
    }
    setBusy(true);
    setProblem(null);
    try {
      await api.addListNode(list.id, request);
      finish(list);
    } catch (reason: unknown) {
      if (reason instanceof ApiError && reason.code === "item_exists") {
        setMemberOf((current) => new Set(current).add(list.id));
        setProblem(`Already on “${list.title}”.`);
      } else setProblem(reason instanceof ApiError ? listErrorMessage(reason) : reason instanceof Error ? reason.message : "Could not add.");
    } finally {
      setBusy(false);
    }
  };

  const createAndAdd = async (): Promise<void> => {
    const title = query.trim();
    const request = targetAdd(target, []);
    if (title === "") {
      setProblem("Type a title first, then press ctrl+↵ to create the list.");
      return;
    }
    if (request === null) {
      setProblem("Nothing to add: this page has no paper or Work.");
      return;
    }
    setBusy(true);
    setProblem(null);
    try {
      const created = await api.createList({ title });
      try {
        await api.addListNode(created.list.id, request);
        finish(created.list);
      } catch (reason: unknown) {
        // The list exists now; say so rather than leave the user to guess.
        const why = reason instanceof ApiError ? listErrorMessage(reason) : "Could not add.";
        setProblem(`Created “${created.list.title}”, but ${why.charAt(0).toLowerCase()}${why.slice(1)}`);
        announceListsChanged();
      }
    } catch (reason: unknown) {
      setProblem(reason instanceof Error ? reason.message : "Could not create the list.");
    } finally {
      setBusy(false);
    }
  };

  const openTopics = async (list: ListSummaryView): Promise<void> => {
    setBusy(true);
    setProblem(null);
    try {
      const detail = await api.readList(list.id);
      const topics = detail.nodes.filter((node) => node.kind === "topic" && node.review === "accepted").sort((a, b) => a.position - b.position);
      setStage({ kind: "topics", list, topics, active: 0 });
    } catch (reason: unknown) {
      setProblem(reason instanceof Error ? reason.message : "Could not read the list.");
    } finally {
      setBusy(false);
    }
  };

  useDialogKeys((event) => {
    if (busy) return true;
    if (event.key === "Escape") {
      if (stage.kind === "topics") setStage({ kind: "lists" });
      else onClose();
      return true;
    }
    if (stage.kind === "topics") {
      const move = dialogDirection(event);
      if (move !== null) {
        setStage({ ...stage, active: Math.max(0, Math.min(stageLast, stage.active + move)) });
        return true;
      }
      if (event.key === "Enter") {
        const topic = stage.topics[stage.active - 1];
        void add(stage.list, topic === undefined ? [] : [topic.id]);
        return true;
      }
      return false;
    }
    const move = dialogDirection(event);
    if (move !== null) {
      setActive((index) => Math.max(0, Math.min(last, index + move)));
      return true;
    }
    if (event.key === "Tab") {
      const list = ranked[at];
      if (list !== undefined) void openTopics(list);
      return true;
    }
    if (event.key === "Enter") {
      if (event.ctrlKey) void createAndAdd();
      else if (ranked[at] !== undefined) void add(ranked[at], []);
      else if (query.trim() !== "") setProblem("No list matches. Press ctrl+↵ to create it.");
      return true;
    }
    return false;
  });

  const label = target.title ?? "this paper";
  if (stage.kind === "topics") {
    return (
      <DialogShell
        eyebrow={`Add to “${stage.list.title}”`}
        title="Choose a topic"
        hint="Tab"
        onClose={() => setStage({ kind: "lists" })}
        footer={<><span><kbd>j/k</kbd> move</span><span><kbd>↵</kbd> add</span><span><kbd>esc</kbd> back</span></>}
      >
        <div className="switcher-results" role="listbox">
          {[{ id: NO_TOPIC, label: "No topic" }, ...stage.topics.map((topic) => ({ id: topic.id, label: topic.label ?? "Topic" }))].map((choice, index) => (
            <button
              key={choice.id}
              ref={(node) => { itemRefs.current[index] = node; }}
              type="button"
              role="option"
              aria-selected={index === stage.active}
              className={index === stage.active ? "is-active" : ""}
              tabIndex={-1}
              onMouseEnter={() => setStage({ ...stage, active: index })}
              onClick={() => {
                const topic = stage.topics[index - 1];
                void add(stage.list, topic === undefined ? [] : [topic.id]);
              }}
            >
              <span className="list-choice-mark" aria-hidden="true" />
              <span><strong>{choice.label}</strong></span>
            </button>
          ))}
        </div>
        {problem !== null && <p className="list-dialog-problem" role="alert">{problem}</p>}
      </DialogShell>
    );
  }

  return (
    <DialogShell
      eyebrow={`Add “${label.length > 60 ? `${label.slice(0, 59)}…` : label}”`}
      title="Add to a list"
      hint="Y"
      onClose={onClose}
      footer={<>
        <span><kbd>↑/↓</kbd> or <kbd>ctrl+n/p</kbd> move</span>
        <span><kbd>↵</kbd> add</span>
        <span><kbd>ctrl+↵</kbd> new list</span>
        <span><kbd>tab</kbd> topic</span>
        <span><kbd>esc</kbd> cancel</span>
      </>}
    >
      <label className="switcher-search">
        <span>›</span>
        <input
          ref={inputRef}
          value={query}
          maxLength={120}
          onChange={(event) => { setQuery(event.target.value); setActive(0); setProblem(null); }}
          placeholder="Find a list, or type a new title"
          aria-label="Find a list"
          autoComplete="off"
          spellCheck={false}
        />
      </label>
      <div className="switcher-results" role="listbox">
        {ranked.map((list, index) => (
          <button
            key={list.id}
            ref={(node) => { itemRefs.current[index] = node; }}
            type="button"
            role="option"
            aria-selected={index === at}
            className={index === at ? "is-active" : ""}
            tabIndex={-1}
            onMouseEnter={() => setActive(index)}
            onClick={() => void add(list, [])}
          >
            <span className="list-choice-mark" aria-hidden="true">{memberOf.has(list.id) ? "✓" : ""}</span>
            <span>
              <strong>{list.title}</strong>
              <small>{list.item_count} {list.item_count === 1 ? "item" : "items"}{list.topic_count > 0 ? ` · ${list.topic_count} topics` : ""}</small>
            </span>
            {memberOf.has(list.id) && <em>on list</em>}
          </button>
        ))}
        {lists === null && <p>Loading lists…</p>}
        {lists !== null && ranked.length === 0 && <p>{query.trim() === "" ? "No lists yet. Type a title and press ctrl+↵." : `No list matches “${query.trim()}”. Press ctrl+↵ to create it.`}</p>}
      </div>
      {problem !== null && <p className="list-dialog-problem" role="alert">{problem}</p>}
    </DialogShell>
  );
}
