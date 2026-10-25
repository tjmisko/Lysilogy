import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "../lib/api";
import { LISTS_CHANGED_EVENT, LIST_COMMAND_EVENT } from "../lib/listEvents";
import {
  SEQUENCE_TIMEOUT_MS,
  describeEdge,
  manualOrder,
  moveRelative,
  moveSelection,
  nextReadingStatus,
  sameOrder,
  selectionAfterRemoval,
  stepAmongRows,
  stepSequence,
  topicChange,
} from "../lib/listKeys";
import type { AppCommand } from "../lib/listRoute";
import {
  INITIAL_VIEW_STATE,
  foldKeyAt,
  foldKeys,
  groupKeyOf,
  nodeIdOf,
  reorderBlockReason,
  sequenceModel,
  type ListViewState,
} from "../lib/listViewModel";
import {
  LIST_VIEWS,
  SORT_LABELS,
  countsLabel,
  foldCloseAll,
  foldOpen,
  foldOpenAll,
  foldClose,
  foldToggle,
  nextSort,
  nextView,
  progressStrip,
  pruneFolds,
  statusOf,
  type ListViewId,
} from "../lib/listViews";
import { useListDetail } from "../lib/useListDetail";
import { useWorkTitles } from "../lib/useWorkTitles";
import type { AcquisitionJobState, ListDetail, ListNodeAdd, ListNodeView } from "../types";
import { InlineInput } from "./InlineInput";
import { AddByNameDialog, ChoiceDialog, ReferencesDialog, type Choice, type ReferenceDirection } from "./ListDialogs";
import { ListWorkPanel } from "./ListWorkPanel";
import { viewDef } from "./ListViews";
import "./ListPage.css";

const PAGE_ROWS = 10;
const MESSAGE_MILLISECONDS = 4000;
const BATCH_LIMIT = 100;

type ListPageProps = {
  listId: string;
  view: ListViewId;
  keyboardEnabled: boolean;
  /** Bumped by App after a generation finishes. */
  reloadToken: number;
  onView: (view: ListViewId) => void;
  onBack: () => void;
  onOpenPaper: (paperId: string) => void;
};

type Dialog =
  | { kind: "add" }
  | { kind: "references"; nodeId: string; workId: string; title: string }
  | { kind: "topics"; nodeId: string }
  | { kind: "edges"; nodeId: string }
  | { kind: "context"; nodeId: string };

type Mode = "normal" | "filter" | "topic";

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

function titleOfNode(node: ListNodeView | undefined): string {
  return node?.work?.display_title ?? node?.claim?.title ?? node?.label ?? "Untitled";
}

function failureMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "The request failed.";
}

/** The Work-or-claim node a cursor id stands on. */
function nodeAt(detail: ListDetail | null, cursor: string | null): ListNodeView | null {
  const id = nodeIdOf(cursor);
  if (detail === null || id === null) return null;
  return detail.nodes.find((node) => node.id === id) ?? null;
}

/** `#list=<id>`: one list in one of seven views, with its progress strip and keys. */
export function ListPage({ listId, view, keyboardEnabled, reloadToken, onView, onBack, onOpenPaper }: ListPageProps) {
  const { detail, loaded, message, clearMessage, actions } = useListDetail(listId);
  const [rawViewState, setViewState] = useState<ListViewState>(INITIAL_VIEW_STATE);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("normal");
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [panelNodeId, setPanelNodeId] = useState<string | null>(null);
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
  const [tagColors, setTagColors] = useState<Record<string, string>>({});
  const [fetchJob, setFetchJob] = useState<{ workId: string; job: AcquisitionJobState; message: string | null } | null>(null);
  const pending = useRef("");
  const pendingTimer = useRef<number | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const controller = new AbortController();
    void api.listTags(controller.signal).then((found) => {
      if (controller.signal.aborted) return;
      setTagColors(Object.fromEntries(found.tags.map((tag) => [tag.name.toLowerCase(), tag.color])));
    }).catch(() => {
      // Topics simply keep the default color.
    });
    return () => controller.abort();
  }, []);

  const { reload } = actions;
  useEffect(() => {
    if (reloadToken > 0) void reload();
  }, [reload, reloadToken]);

  useEffect(() => {
    const onChanged = (): void => { void reload(); };
    window.addEventListener(LISTS_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(LISTS_CHANGED_EVENT, onChanged);
  }, [reload]);

  useEffect(() => {
    if (message === null) return;
    const timer = window.setTimeout(clearMessage, MESSAGE_MILLISECONDS);
    return () => window.clearTimeout(timer);
  }, [clearMessage, message]);

  // Folds of topics that were deleted are forgotten.
  const viewState = useMemo<ListViewState>(
    () => (detail === null ? rawViewState : { ...rawViewState, folded: pruneFolds(rawViewState.folded, detail) }),
    [detail, rawViewState],
  );
  const def = viewDef(view);
  const ids = useMemo(() => (detail === null ? [] : def.order(detail, viewState)), [def, detail, viewState]);
  const cursor = selectedId !== null && ids.includes(selectedId) ? selectedId : (ids[0] ?? null);
  const node = nodeAt(detail, cursor);
  const contextWorkIds = useMemo(
    () => (detail?.nodes ?? []).flatMap((entry) => entry.context_links.slice(0, 1).map((link) => link.work_id)),
    [detail],
  );
  const workTitles = useWorkTitles(contextWorkIds);
  const titleOf = useCallback((nodeId: string): string => titleOfNode(detail?.nodes.find((entry) => entry.id === nodeId)), [detail]);

  useEffect(() => {
    if (cursor === null) return;
    document.querySelector(`.list-page [data-node-id="${CSS.escape(cursor)}"], .list-page [data-group-key="${CSS.escape(groupKeyOf(cursor) ?? "")}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const notify = actions.notify;
  const fail = actions.fail;
  const patchView = useCallback((patch: Partial<ListViewState>): void => setViewState((state) => ({ ...state, ...patch })), []);

  const clearPending = useCallback((): void => {
    pending.current = "";
    if (pendingTimer.current !== null) window.clearTimeout(pendingTimer.current);
    pendingTimer.current = null;
  }, []);
  useEffect(() => clearPending, [clearPending]);

  // ---- Operations ------------------------------------------------------------------------

  const existingWorkIds = useMemo(
    () => new Set((detail?.nodes ?? []).flatMap((entry) => (entry.work_id === null ? [] : [entry.work_id]))),
    [detail],
  );

  const addByName = useCallback(async (add: ListNodeAdd): Promise<void> => {
    setDialog(null);
    const added = await actions.addNode(add);
    if (added === null) return;
    setSelectedId(added.id);
    patchView({ filter: "" });
    notify(added.kind === "claim"
      ? `Added “${titleOfNode(added)}” as a claim (${added.claim_state ?? "unverified"}). It is not matched to a paper yet.`
      : `Added “${titleOfNode(added)}”.`);
  }, [actions, notify, patchView]);

  const addReferences = useCallback(async (sourceWorkId: string, workIds: string[], direction: ReferenceDirection): Promise<void> => {
    setDialog(null);
    // A reference is reached through the paper that cites it; a citing paper has no such edge back.
    const adds: ListNodeAdd[] = workIds.map((workId) => direction === "references"
      ? { work_id: workId, via_work_id: sourceWorkId }
      : { work_id: workId });
    let added = 0;
    for (let start = 0; start < adds.length; start += BATCH_LIMIT) {
      const batch = await actions.addNodes(adds.slice(start, start + BATCH_LIMIT));
      if (batch === null) return;
      added += batch.length;
    }
    notify(`Added ${added} ${added === 1 ? "paper" : "papers"} from ${direction === "references" ? "the references" : "the cited-by list"}.`);
  }, [actions, notify]);

  const removeSelected = useCallback(async (): Promise<void> => {
    if (node === null) return;
    const next = selectionAfterRemoval(ids, cursor ?? node.id);
    const title = titleOfNode(node);
    if (viewState.cutId === node.id) patchView({ cutId: null });
    if (panelNodeId === node.id) setPanelNodeId(null);
    await actions.remove(node.id);
    setSelectedId(next);
    notify(`Removed “${title}”. Press u to undo.`);
  }, [actions, cursor, ids, node, notify, panelNodeId, patchView, viewState.cutId]);

  const cycleStatus = useCallback((): void => {
    if (node === null || detail === null) return;
    if (node.work_id === null) {
      notify(node.kind === "claim" ? "A claim has no reading status until it is matched to a paper." : "Topics have no reading status.");
      return;
    }
    void actions.setStatus(node.work_id, nextReadingStatus(statusOf(detail, node.work_id)));
  }, [actions, detail, node, notify]);

  const saveNote = useCallback((nodeId: string, note: string): void => {
    setEditingNoteId(null);
    const trimmed = note.trim();
    const current = detail?.nodes.find((entry) => entry.id === nodeId)?.note ?? "";
    if (trimmed === current) return;
    void actions.setNote(nodeId, trimmed === "" ? null : trimmed);
  }, [actions, detail]);

  const reorderBlocked = (): boolean => {
    if (view !== 1) {
      notify("Reordering is in the Sequence view (1).");
      return true;
    }
    const reason = reorderBlockReason(viewState);
    if (reason !== null) notify(reason);
    return reason !== null;
  };

  const putCut = (where: "before" | "after"): void => {
    if (detail === null || reorderBlocked()) return;
    if (viewState.cutId === null) {
      notify("Nothing is cut. Press dd on a row first.");
      return;
    }
    const target = node?.id ?? null;
    if (target === null) return;
    if (target === viewState.cutId) {
      notify("Move to another row, then put.");
      return;
    }
    const current = manualOrder(detail.nodes);
    const next = moveRelative(current, viewState.cutId, target, where);
    patchView({ cutId: null });
    if (!sameOrder(current, next)) void actions.reorder(next);
  };

  const stepRow = (delta: -1 | 1): void => {
    if (detail === null || node === null || reorderBlocked()) return;
    const rows = sequenceModel(detail, viewState).rows.map((row) => row.id);
    const current = manualOrder(detail.nodes);
    const next = stepAmongRows(current, rows, node.id, delta);
    if (!sameOrder(current, next)) void actions.reorder(next);
  };

  const openCursor = (target: string): void => {
    if (detail === null) return;
    const key = groupKeyOf(target);
    if (key !== null) {
      patchView({ folded: foldToggle(viewState.folded, key) });
      return;
    }
    const opened = detail.nodes.find((entry) => entry.id === target);
    if (opened === undefined) return;
    if (opened.kind === "topic") {
      if (view === 2) patchView({ folded: foldToggle(viewState.folded, opened.id) });
      return;
    }
    const paper = opened.work_id === null ? undefined : detail.local_papers[opened.work_id]?.[0];
    if (paper !== undefined) {
      onOpenPaper(paper.paper_id);
      return;
    }
    setPanelNodeId((now) => now === opened.id ? null : opened.id);
  };

  const startFetch = (): void => {
    const target = nodeAt(detail, panelNodeId);
    if (target?.work_id == null) return;
    const workId = target.work_id;
    setFetchJob({ workId, job: "running", message: null });
    api.fetchWork(workId).then(() => actions.reload()).then(() => {
      setFetchJob((now) => now?.workId === workId ? null : now);
    }).catch((reason: unknown) => {
      setFetchJob({ workId, job: "failed", message: failureMessage(reason) });
    });
  };

  const openContextLink = async (workId: string): Promise<void> => {
    try {
      const found = await api.workCitations(workId);
      const paper = found.local_papers[workId]?.[0];
      if (paper === undefined) notify(`“${found.work.display_title ?? "That paper"}” is not in your library.`);
      else onOpenPaper(paper.paper_id);
    } catch (reason: unknown) {
      fail(failureMessage(reason));
    }
  };

  const changeTopics = async (nodeId: string, wanted: string[]): Promise<void> => {
    setDialog(null);
    if (detail === null) return;
    const current = detail.edges.filter((edge) => edge.kind === "covers" && edge.to === nodeId).map((edge) => edge.from);
    const change = topicChange(current, wanted);
    for (const from of change.add) if (!await actions.addEdge({ from, to: nodeId, kind: "covers" })) return;
    for (const from of change.remove) if (!await actions.removeEdge({ from, to: nodeId, kind: "covers" })) return;
    if (change.add.length + change.remove.length > 0) notify("Topics updated.");
  };

  const drawEdge = async (): Promise<void> => {
    const from = viewState.edgeFrom;
    patchView({ edgeFrom: null });
    if (from === null || node === null) return;
    if (from === node.id) {
      notify("A paper cannot be its own prerequisite. Move to another row.");
      return;
    }
    if (node.kind === "claim") {
      notify("A claim cannot take part in an edge until it is matched to a paper.");
      return;
    }
    if (await actions.addEdge({ from, to: node.id, kind: "prerequisite" })) {
      notify(`“${titleOf(from)}” is now a prerequisite of “${titleOfNode(node)}”.`);
    }
  };

  const addTopic = async (label: string): Promise<void> => {
    setMode("normal");
    const trimmed = label.trim();
    if (trimmed === "" || detail === null) return;
    const anchor = node?.id ?? null;
    const added = await actions.addNode({ topic: { label: trimmed } });
    if (added === null) return;
    if (anchor !== null && added.id !== anchor) {
      // Put the new topic right below the node it was made from.
      const without = manualOrder(detail.nodes).filter((id) => id !== added.id);
      const next = moveRelative([...without, added.id], added.id, anchor, "after");
      await actions.reorder(next);
    }
    notify(`Added topic “${trimmed}”. Press T on a paper to file it there.`);
  };

  // ---- Colon commands sent by App ------------------------------------------------------------

  const commandRef = useRef<(command: AppCommand) => void>(() => undefined);
  useEffect(() => {
    commandRef.current = (command) => {
      if (command.kind === "order") {
        void actions.orderBy(command.strategy).then(() => notify(`Order written: ${command.strategy.replace("_", "-")}.`));
      } else if (command.kind === "verify") {
        void actions.verify().then((report) => {
          if (report !== null) notify(`Checked ${report.checked}: ${report.resolved} matched, ${report.still_pending} still pending.`);
        });
      } else if (command.kind === "read-status") {
        if (node?.work_id == null) {
          notify("Select a paper first.");
          return;
        }
        void actions.setStatus(node.work_id, command.status);
      }
    };
  });
  useEffect(() => {
    const onCommand = (event: Event): void => {
      if (event instanceof CustomEvent) commandRef.current(event.detail as AppCommand);
    };
    window.addEventListener(LIST_COMMAND_EVENT, onCommand);
    return () => window.removeEventListener(LIST_COMMAND_EVENT, onCommand);
  }, []);

  // ---- Keys ----------------------------------------------------------------------------------

  const keysEnabled = keyboardEnabled && dialog === null && mode === "normal" && editingNoteId === null;
  const handle = useRef<(event: KeyboardEvent) => void>(() => undefined);
  useEffect(() => {
    handle.current = (event) => {
      if (event.defaultPrevented || event.isComposing || event.metaKey || event.altKey || isEditable(event.target) || detail === null) return;
      const key = event.key;
      const consume = (): void => event.preventDefault();
      const move = (delta: number): void => {
        consume();
        setSelectedId(moveSelection(ids, cursor, delta));
      };
      if (event.ctrlKey) {
        if (key === "d") move(PAGE_ROWS);
        else if (key === "u") move(-PAGE_ROWS);
        return;
      }
      if (key === "Escape" || key === "q") {
        consume();
        clearPending();
        if (panelNodeId !== null) setPanelNodeId(null);
        else if (viewState.edgeFrom !== null) patchView({ edgeFrom: null });
        else if (viewState.cutId !== null) patchView({ cutId: null });
        else if (viewState.filter !== "") patchView({ filter: "" });
        else onBack();
        return;
      }
      // Sequences first: g, d and z wait for a second key.
      if (view === 1 || view === 2 || pending.current !== "" || key === "g") {
        const step = stepSequence(pending.current, key);
        if (step.consumed) {
          consume();
          clearPending();
          if (step.pending !== "") {
            pending.current = step.pending;
            pendingTimer.current = window.setTimeout(clearPending, SEQUENCE_TIMEOUT_MS);
            return;
          }
          switch (step.action) {
            case "top": setSelectedId(ids[0] ?? null); break;
            case "cut": {
              if (view !== 1 || node === null || node.kind === "topic") {
                notify("Cut works on a paper row in the Sequence view.");
                break;
              }
              if (reorderBlocked()) break;
              patchView({ cutId: node.id });
              notify(`Cut “${titleOfNode(node)}”. Move to a row, then p (below) or P (above).`);
              break;
            }
            case "group-toggle":
              if (view !== 1) { notify("Grouping is in the Sequence view (1)."); break; }
              patchView({ grouped: !viewState.grouped });
              break;
            case "fold-open":
            case "fold-close":
            case "fold-toggle": {
              const foldKey = foldKeyAt(detail, viewState, view, cursor);
              if (foldKey === null) { notify(view === 1 ? "Group by topic first (zg)." : "Folds are for topics."); break; }
              const change = step.action === "fold-open" ? foldOpen : step.action === "fold-close" ? foldClose : foldToggle;
              patchView({ folded: change(viewState.folded, foldKey) });
              break;
            }
            case "fold-open-all": patchView({ folded: foldOpenAll() }); break;
            case "fold-close-all": patchView({ folded: foldCloseAll(foldKeys(detail, viewState, view)) }); break;
            case null: break;
          }
          return;
        }
      }
      if (key >= "1" && key <= "7") {
        consume();
        onView(Number(key) as ListViewId);
        return;
      }
      switch (key) {
        case "v": consume(); onView(nextView(view)); return;
        case "j":
        case "ArrowDown": move(1); return;
        case "k":
        case "ArrowUp": move(-1); return;
        case "G": consume(); setSelectedId(ids.at(-1) ?? null); return;
        case "Enter":
        case "o":
          if (viewState.edgeFrom !== null) {
            consume();
            void drawEdge();
          } else if (cursor !== null) {
            consume();
            openCursor(cursor);
          }
          return;
        case "u":
          consume();
          void actions.undo().then((restored) => notify(restored ? "Put back." : "Nothing to undo."));
          return;
        case "r": consume(); cycleStatus(); return;
        case "e":
          consume();
          if (node === null || node.kind === "topic") notify("Select a paper to edit its note.");
          else setEditingNoteId(node.id);
          return;
        case "J": consume(); stepRow(1); return;
        case "K": consume(); stepRow(-1); return;
        case "p": consume(); putCut("after"); return;
        case "P": consume(); putCut("before"); return;
        case "s":
          consume();
          if (view !== 1) { notify("Sorting is in the Sequence view (1)."); return; }
          patchView({ sort: nextSort(viewState.sort) });
          return;
        case "S":
          consume();
          if (view !== 1) { notify("Sorting is in the Sequence view (1)."); return; }
          patchView({ reverse: !viewState.reverse });
          return;
        case "/":
          consume();
          if (view !== 1) { notify("The filter is in the Sequence view (1)."); return; }
          setMode("filter");
          return;
        case "O": consume(); setMode("topic"); return;
        case "T":
          consume();
          if (node?.kind !== "work") notify("Select a paper to file under topics.");
          else setDialog({ kind: "topics", nodeId: node.id });
          return;
        case "E":
          consume();
          if (node === null || node.kind === "claim") notify("Select a paper or topic to draw an edge from.");
          else {
            patchView({ edgeFrom: node.id });
            notify("Move to the paper that comes after it, then press Enter. Esc cancels.");
          }
          return;
        case "X":
          consume();
          if (node === null) notify("Select a row first.");
          else setDialog({ kind: "edges", nodeId: node.id });
          return;
        case "C":
          consume();
          if (node === null || node.context_links.length === 0) notify("This item has no links to papers you have read.");
          else setDialog({ kind: "context", nodeId: node.id });
          return;
        case "A":
          consume();
          if (node?.kind !== "work" || node.work_id === null) notify("Select a paper to list its references.");
          else setDialog({ kind: "references", nodeId: node.id, workId: node.work_id, title: titleOfNode(node) });
          return;
        case "a":
          if (view === 7) return;
          consume();
          setDialog({ kind: "add" });
          return;
        case "x":
          if (view === 7) return;
          consume();
          void removeSelected();
          return;
        case "f":
        case "F":
          if (view === 6) return;
          if (panelNodeId !== null) {
            consume();
            startFetch();
          }
          return;
      }
    };
  });
  useEffect(() => {
    if (!keysEnabled) return;
    const onKey = (event: KeyboardEvent): void => handle.current(event);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [keysEnabled]);

  // ---- Render --------------------------------------------------------------------------------

  if (!loaded) {
    return <section className="list-page" aria-label="Reading list"><div className="list-empty"><p>Loading the list…</p></div></section>;
  }
  if (detail === null) {
    return <section className="list-page" aria-label="Reading list">
      <div className="list-empty">
        <p>{message?.text ?? "This list could not be loaded."}</p>
        <button type="button" className="list-back" onClick={onBack}>Back to lists <kbd>q</kbd></button>
      </div>
    </section>;
  }

  const strip = progressStrip(detail);
  const View = def.component;
  const panelNode = nodeAt(detail, panelNodeId);
  const targetPaper = node?.work_id == null ? undefined : detail.local_papers[node.work_id]?.[0]?.paper_id;
  const edgeTarget = viewState.edgeFrom === null ? null : titleOf(viewState.edgeFrom);
  const noteNode = editingNoteId !== null && view !== 1 ? detail.nodes.find((entry) => entry.id === editingNoteId) : undefined;

  let dialogNode: ListNodeView | null = null;
  if (dialog !== null && dialog.kind !== "add" && dialog.kind !== "references") {
    dialogNode = detail.nodes.find((entry) => entry.id === dialog.nodeId) ?? null;
  }

  return <section
    className="list-page"
    aria-label={`Reading list: ${detail.list.title}`}
    data-list-id={listId}
    data-view={view}
    data-list-target-work={node?.work_id ?? undefined}
    data-list-target-paper={targetPaper}
  >
    <header className="list-head">
      <button type="button" className="list-back" onClick={onBack} title="All lists (q)">‹ Lists <kbd>q</kbd></button>
      <h1>{detail.list.title}</h1>
      <span className="list-origin">{detail.list.origin}</span>
      <p className="home-counts" role="status">
        <strong>{strip.total.read + strip.total.reading + strip.total.unread}</strong> papers<span aria-hidden="true"> / </span>{countsLabel(strip.total)}
      </p>
    </header>
    {detail.list.description !== null && <p className="list-description">{detail.list.description}</p>}

    <nav className="home-filters list-views" aria-label="List views">
      {LIST_VIEWS.map((entry) => <button
        key={entry.id}
        type="button"
        aria-pressed={entry.id === view}
        onClick={() => onView(entry.id)}
      ><kbd>{entry.id}</kbd> {entry.name}</button>)}
    </nav>

    {strip.topics.length > 0 && <ul className="list-progress" aria-label="Progress by topic">
      {strip.topics.map((topic) => {
        const color = tagColors[topic.label.toLowerCase()];
        return <li key={topic.id} style={color === undefined ? undefined : { borderColor: color }}>
          <span>{topic.label}</span>
          <span className="list-progress-counts">●{topic.counts.read} ◐{topic.counts.reading} ○{topic.counts.unread}</span>
        </li>;
      })}
    </ul>}

    {view === 1 && <div className="list-toolbar">
      <span>sort <strong>{SORT_LABELS[viewState.sort]}{viewState.reverse ? " (reversed)" : ""}</strong>{viewState.sort === "manual" ? "" : " · view only"}</span>
      {viewState.grouped && <span>grouped by topic</span>}
      {mode === "filter"
        ? <input
          ref={filterRef}
          className="list-filter"
          aria-label="Filter the list"
          autoFocus
          value={viewState.filter}
          placeholder="Filter by title, author, topic, note"
          onChange={(event) => patchView({ filter: event.target.value })}
          onBlur={() => setMode("normal")}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === "Enter" || event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              if (event.key === "Escape") patchView({ filter: "" });
              setMode("normal");
            }
          }}
        />
        : viewState.filter !== "" && <span>filter <strong>{viewState.filter}</strong> <kbd>esc</kbd> clears</span>}
    </div>}

    {message !== null && <p className={`list-message ${message.kind === "error" ? "is-error" : ""}`} role={message.kind === "error" ? "alert" : "status"}>{message.text}</p>}

    {mode === "topic" && <div className="list-topic-input">
      <span>New topic</span>
      <InlineInput label="New topic" className="list-row-input" initial="" maxLength={80} placeholder="Topic name" onSave={(value) => void addTopic(value)} onCancel={() => setMode("normal")} />
    </div>}

    {noteNode !== undefined && <div className="list-topic-input">
      <span>Note on “{titleOfNode(noteNode)}”</span>
      <InlineInput label="Note" className="list-row-input" initial={noteNode.note ?? ""} placeholder="Why is this on the list?" onSave={(value) => saveNote(noteNode.id, value)} onCancel={() => setEditingNoteId(null)} />
    </div>}

    <div className={`list-body ${panelNode !== null ? "has-panel" : ""}`}>
      <div className="list-view" data-view-name={def.name}>
        <View
          detail={detail}
          selectedId={cursor}
          onSelect={setSelectedId}
          onOpen={openCursor}
          actions={actions}
          keyboardEnabled={keysEnabled}
          viewState={viewState}
          workTitles={workTitles}
          tagColors={tagColors}
          editingNoteId={view === 1 ? editingNoteId : null}
          onSaveNote={saveNote}
          onCancelNote={() => setEditingNoteId(null)}
        />
      </div>
      {panelNode !== null && <ListWorkPanel
        detail={detail}
        node={panelNode}
        workTitles={workTitles}
        fetchJob={fetchJob?.workId === panelNode.work_id ? fetchJob.job : "none"}
        fetchMessage={fetchJob?.workId === panelNode.work_id ? fetchJob.message : null}
        onFetch={startFetch}
        onOpenPaper={onOpenPaper}
        onClose={() => setPanelNodeId(null)}
      />}
    </div>

    <footer className="list-status" aria-label="Keys">
      {edgeTarget !== null
        ? <span className="list-status-mode">“{edgeTarget}” is a prerequisite of “{node === null ? "…" : titleOfNode(node)}” <kbd>↵</kbd> confirm <kbd>esc</kbd> cancel</span>
        : viewState.cutId !== null
          ? <span className="list-status-mode">Cut “{titleOf(viewState.cutId)}” <kbd>p</kbd> below <kbd>P</kbd> above <kbd>esc</kbd> cancel</span>
          : <>
            <span><kbd>j/k</kbd> move</span>
            <span><kbd>1-7</kbd>/<kbd>v</kbd> views</span>
            <span><kbd>↵</kbd> open</span>
            <span><kbd>a</kbd> add</span>
            <span><kbd>A</kbd> refs</span>
            <span><kbd>r</kbd> status</span>
            <span><kbd>e</kbd> note</span>
            <span><kbd>x</kbd>/<kbd>u</kbd> remove/undo</span>
            <span><kbd>dd</kbd><kbd>p</kbd> reorder</span>
            <span><kbd>E</kbd> edge</span>
            <span><kbd>T</kbd>/<kbd>O</kbd> topics</span>
            <span><kbd>Y</kbd>/<kbd>M</kbd> lists</span>
          </>}
    </footer>

    {dialog?.kind === "add" && <AddByNameDialog existingWorkIds={existingWorkIds} onAdd={(add) => void addByName(add)} onClose={() => setDialog(null)} />}
    {dialog?.kind === "references" && <ReferencesDialog
      workId={dialog.workId}
      title={dialog.title}
      existingWorkIds={existingWorkIds}
      onAdd={(workIds, direction) => void addReferences(dialog.workId, workIds, direction)}
      onClose={() => setDialog(null)}
    />}
    {dialog?.kind === "topics" && dialogNode !== null && <ChoiceDialog
      eyebrow={titleOfNode(dialogNode)}
      title="File under topics"
      multiple
      confirmLabel="apply"
      emptyText="This list has no topics. Press O to add one."
      choices={detail.nodes
        .filter((entry) => entry.kind === "topic" && entry.review === "accepted")
        .sort((a, b) => a.position - b.position)
        .map((topic): Choice => ({
          id: topic.id,
          label: topic.label ?? "Topic",
          checked: detail.edges.some((edge) => edge.kind === "covers" && edge.from === topic.id && edge.to === dialogNode.id),
        }))}
      onConfirm={(wanted) => void changeTopics(dialogNode.id, wanted)}
      onClose={() => setDialog(null)}
    />}
    {dialog?.kind === "edges" && dialogNode !== null && <ChoiceDialog
      eyebrow={titleOfNode(dialogNode)}
      title="Remove an edge"
      multiple={false}
      confirmLabel="remove"
      emptyText="This item has no edges."
      choices={detail.edges
        .filter((edge) => edge.from === dialogNode.id || edge.to === dialogNode.id)
        .map((edge): Choice => ({
          id: `${edge.from}|${edge.to}|${edge.kind}`,
          label: describeEdge(edge, dialogNode.id, titleOf),
          detail: edge.review === "proposed" ? "proposed" : edge.source === "agent" ? "by the agent" : undefined,
        }))}
      onConfirm={([chosen]) => {
        setDialog(null);
        const edge = detail.edges.find((entry) => `${entry.from}|${entry.to}|${entry.kind}` === chosen);
        if (edge !== undefined) void actions.removeEdge({ from: edge.from, to: edge.to, kind: edge.kind }).then((done) => { if (done) notify("Edge removed."); });
      }}
      onClose={() => setDialog(null)}
    />}
    {dialog?.kind === "context" && dialogNode !== null && <ChoiceDialog
      eyebrow={titleOfNode(dialogNode)}
      title="Connects to papers you read"
      multiple={false}
      confirmLabel="open"
      emptyText="No links."
      choices={dialogNode.context_links.map((link): Choice => ({
        id: link.work_id,
        label: workTitles[link.work_id] ?? "A paper you read",
        detail: `${link.relation.replace("_", " ")}${link.note === "" ? "" : ` — ${link.note}`}`,
      }))}
      onConfirm={([chosen]) => {
        setDialog(null);
        if (chosen !== undefined) void openContextLink(chosen);
      }}
      onClose={() => setDialog(null)}
    />}
  </section>;
}
