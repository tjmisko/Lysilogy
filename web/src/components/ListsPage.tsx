import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "../lib/api";
import { LISTS_CHANGED_EVENT } from "../lib/listEvents";
import { moveSelection, stepSequence, SEQUENCE_TIMEOUT_MS } from "../lib/listKeys";
import { countsLabel } from "../lib/listViews";
import type { GenerationRun } from "../lib/useGeneration";
import type { ListSummaryView } from "../types";
import { InlineInput } from "./InlineInput";
import "./ListsPage.css";

const PAGE_ROWS = 10;

type ListsPageProps = {
  keyboardEnabled: boolean;
  onOpenList: (listId: string) => void;
  /** `q` or Esc with nothing to clear: back to the paper grid. */
  onHome: () => void;
  /** Bumped by App when a generation finishes, so the index reloads. */
  reloadToken: number;
  onNotice: (text: string) => void;
  onError: (text: string) => void;
};

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

function dateOf(timestamp: string): string {
  const match = /^\d{4}-\d{2}-\d{2}/u.exec(timestamp);
  return match === null ? "" : match[0];
}

function matches(list: ListSummaryView, filter: string): boolean {
  const terms = filter.normalize("NFKC").toLowerCase().split(/\s+/u).filter((term) => term !== "");
  const text = [list.title, list.description ?? "", list.goal ?? "", list.seed_title ?? ""].join(" ").normalize("NFKC").toLowerCase();
  return terms.every((term) => text.includes(term));
}

/** `#lists`: every reading list, newest activity first. */
export function ListsPage({ keyboardEnabled, onOpenList, onHome, reloadToken, onNotice, onError }: ListsPageProps) {
  const [lists, setLists] = useState<ListSummaryView[] | null>(null);
  const [showArchived, setShowArchived] = useState(false);
  const [filter, setFilter] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [mode, setMode] = useState<"normal" | "filter" | "new" | "rename">("normal");
  const [reloads, setReloads] = useState(0);
  const pending = useRef("");
  const pendingTimer = useRef<number | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onChanged = (): void => setReloads((count) => count + 1);
    window.addEventListener(LISTS_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(LISTS_CHANGED_EVENT, onChanged);
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    void api.listLists({ archived: showArchived ? true : undefined }, controller.signal).then((found) => {
      if (!controller.signal.aborted) setLists(found.lists);
    }).catch((reason: unknown) => {
      if (controller.signal.aborted) return;
      setLists((current) => current ?? []);
      onError(reason instanceof Error ? reason.message : "Could not load the lists.");
    });
    return () => controller.abort();
  }, [onError, reloadToken, reloads, showArchived]);

  const visible = useMemo(() => (lists ?? []).filter((list) => matches(list, filter)), [lists, filter]);
  const ids = useMemo(() => visible.map((list) => list.id), [visible]);
  const current = selectedId !== null && ids.includes(selectedId) ? selectedId : (ids[0] ?? null);
  const selected = visible.find((list) => list.id === current) ?? null;

  useEffect(() => {
    if (current === null) return;
    document.querySelector(`.lists-page [data-list-id="${CSS.escape(current)}"]`)?.scrollIntoView({ block: "nearest" });
  }, [current]);

  const clearPending = useCallback((): void => {
    pending.current = "";
    if (pendingTimer.current !== null) window.clearTimeout(pendingTimer.current);
    pendingTimer.current = null;
  }, []);

  useEffect(() => clearPending, [clearPending]);

  const archive = useCallback(async (list: ListSummaryView): Promise<void> => {
    const archiving = list.archived_at === null;
    try {
      await api.patchList(list.id, { archived: archiving });
      onNotice(archiving ? `Archived “${list.title}”. Press za to show archived lists.` : `Restored “${list.title}”.`);
      setReloads((count) => count + 1);
    } catch (reason: unknown) {
      onError(reason instanceof Error ? reason.message : "Could not archive the list.");
    }
  }, [onError, onNotice]);

  useEffect(() => {
    if (!keyboardEnabled || mode !== "normal") return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.isComposing || event.metaKey || event.altKey || isEditable(event.target)) return;
      const key = event.key;
      const move = (delta: number): void => {
        event.preventDefault();
        setSelectedId(moveSelection(ids, current, delta));
      };
      if (event.ctrlKey) {
        if (key === "d") move(PAGE_ROWS);
        else if (key === "u") move(-PAGE_ROWS);
        return;
      }
      if (key === "Escape" || key === "q") {
        clearPending();
        event.preventDefault();
        if (filter !== "") setFilter("");
        else onHome();
        return;
      }
      const step = stepSequence(pending.current, key);
      if (step.consumed) {
        event.preventDefault();
        clearPending();
        if (step.pending !== "") {
          pending.current = step.pending;
          pendingTimer.current = window.setTimeout(clearPending, SEQUENCE_TIMEOUT_MS);
        } else if (step.action === "top") setSelectedId(ids[0] ?? null);
        else if (step.action === "fold-toggle") setShowArchived((on) => !on);
        return;
      }
      if (key === "j" || key === "ArrowDown") move(1);
      else if (key === "k" || key === "ArrowUp") move(-1);
      else if (key === "G") {
        event.preventDefault();
        setSelectedId(ids.at(-1) ?? null);
      } else if (key === "Enter" || key === "o") {
        if (selected === null) return;
        event.preventDefault();
        onOpenList(selected.id);
      } else if (key === "/") {
        event.preventDefault();
        setMode("filter");
      } else if (key === "n") {
        event.preventDefault();
        setMode("new");
      } else if (key === "R" && selected !== null) {
        event.preventDefault();
        setMode("rename");
      } else if (key === "A" && selected !== null) {
        event.preventDefault();
        void archive(selected);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [archive, clearPending, current, filter, ids, keyboardEnabled, mode, onHome, onOpenList, selected]);

  useEffect(() => {
    if (mode === "filter") filterRef.current?.focus();
  }, [mode]);

  const create = (title: string): void => {
    setMode("normal");
    const trimmed = title.trim();
    if (trimmed === "") return;
    void api.createList({ title: trimmed }).then((created) => onOpenList(created.list.id)).catch((reason: unknown) => {
      onError(reason instanceof Error ? reason.message : "Could not create the list.");
    });
  };

  const rename = (list: ListSummaryView, title: string): void => {
    setMode("normal");
    const trimmed = title.trim();
    if (trimmed === "" || trimmed === list.title) return;
    void api.patchList(list.id, { title: trimmed }).then(() => setReloads((count) => count + 1)).catch((reason: unknown) => {
      onError(reason instanceof Error ? reason.message : "Could not rename the list.");
    });
  };

  const total = lists?.length ?? 0;
  return <section className="lists-page" aria-label="Reading lists">
    <header className="lists-head">
      <h1>Reading lists</h1>
      <label className="home-search lists-filter">
        <span aria-hidden="true">⌕</span>
        <input
          ref={filterRef}
          type="search"
          aria-label="Filter lists"
          value={filter}
          placeholder="Filter lists"
          onChange={(event) => setFilter(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === "Escape" || event.key === "ArrowDown") {
              event.preventDefault();
              event.stopPropagation();
              if (event.key === "Escape") setFilter("");
              setMode("normal");
              filterRef.current?.blur();
            }
          }}
          onBlur={() => setMode((now) => now === "filter" ? "normal" : now)}
        />
        <kbd aria-hidden="true">/</kbd>
      </label>
      <p className="home-counts" role="status">
        {visible.length === total
          ? <><strong>{total}</strong> {total === 1 ? "list" : "lists"}</>
          : <><strong>{visible.length}</strong> of <strong>{total}</strong> lists</>}
        {showArchived && <><span aria-hidden="true"> / </span>archived shown</>}
      </p>
    </header>

    {mode === "new" && <div className="lists-new">
      <span>New list</span>
      <InlineInput label="New list title" className="lists-input" initial="" maxLength={120} placeholder="Title, for example RLHF basics" onSave={create} onCancel={() => setMode("normal")} />
    </div>}

    {lists === null && <div className="lists-empty"><p>Loading lists…</p></div>}
    {lists !== null && visible.length === 0 && <div className="lists-empty">
      <h2>{total === 0 ? "No reading lists yet." : "No list matches this filter."}</h2>
      <p>{total === 0
        ? <>Press <kbd>n</kbd> to start one, <kbd>Y</kbd> on any paper to add it to a list, or <kbd>:</kbd> <code>generate &lt;goal&gt;</code> to have one drafted.</>
        : "Try another word, or press Esc to clear the filter."}</p>
    </div>}

    {visible.length > 0 && <ol className="lists-rows" aria-label="Lists">
      {visible.map((list) => <li
        key={list.id}
        className={`lists-row ${list.id === current ? "is-selected" : ""} ${list.archived_at === null ? "" : "is-archived"}`}
        data-list-id={list.id}
        data-origin={list.origin}
        onClick={() => setSelectedId(list.id)}
        onDoubleClick={() => onOpenList(list.id)}
      >
        <div className="lists-row-main">
          {mode === "rename" && list.id === current
            ? <InlineInput label="List title" className="lists-input" initial={list.title} maxLength={120} onSave={(value) => rename(list, value)} onCancel={() => setMode("normal")} />
            : <button type="button" className="lists-row-title" tabIndex={-1} onClick={() => onOpenList(list.id)}>{list.title}</button>}
          <p className="lists-row-meta">
            <span className="lists-row-origin">{list.origin}</span>
            <span>{list.item_count} {list.item_count === 1 ? "item" : "items"}</span>
            {list.topic_count > 0 && <span>{list.topic_count} {list.topic_count === 1 ? "topic" : "topics"}</span>}
            <span>{countsLabel(list.status_counts)}</span>
            {list.proposed_count > 0 && <span className="lists-row-flag">{list.proposed_count} to review</span>}
            {list.claim_count > 0 && <span className="lists-row-flag">{list.claim_count} unverified</span>}
            {list.archived_at !== null && <span className="lists-row-flag">archived</span>}
            {list.seed_title !== null && <span className="lists-row-seed">from {list.seed_title}</span>}
            <span className="lists-row-date">{dateOf(list.updated_at)}</span>
          </p>
          {list.description !== null && <p className="lists-row-description">{list.description}</p>}
        </div>
      </li>)}
    </ol>}

    <footer className="lists-keys" aria-label="Keys">
      <span><kbd>j/k</kbd> move</span>
      <span><kbd>↵</kbd> open</span>
      <span><kbd>n</kbd> new</span>
      <span><kbd>R</kbd> rename</span>
      <span><kbd>A</kbd> archive</span>
      <span><kbd>za</kbd> archived</span>
      <span><kbd>/</kbd> filter</span>
      <span><kbd>q</kbd> home</span>
      <span><kbd>:</kbd> generate · delete-list</span>
    </footer>
  </section>;
}

/** The corner strip that follows a running `:generate`. */
export function GenerationStrip({ run, onOpenList, onDismiss }: {
  run: GenerationRun;
  onOpenList: (listId: string) => void;
  onDismiss: () => void;
}) {
  const state = run.status?.state ?? "running";
  const spend = run.status === null ? 0 : run.status.spend_usd;
  const listId = run.status?.list_id ?? run.intoListId;
  const label = state === "running" ? "Drafting"
    : state === "succeeded" ? "Drafted"
      : state === "over_budget" ? "Stopped at the budget"
        : "Failed";
  return <aside className="generation-strip" data-state={state} role="status" aria-live="polite">
    <span className="generation-strip-label">{label}</span>
    <span className="generation-strip-goal" title={run.goal}>{run.goal}</span>
    <span className="generation-strip-spend">${spend.toFixed(4)}</span>
    {run.error !== null && <span className="generation-strip-error">{run.error}</span>}
    {state !== "running" && listId !== null && <button type="button" onClick={() => onOpenList(listId)}>Open</button>}
    {state !== "running" && <button type="button" aria-label="Dismiss" onClick={onDismiss}>×</button>}
  </aside>;
}
