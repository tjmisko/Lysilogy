import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { PDFDocumentProxy } from "pdfjs-dist";
import type { TextRect } from "../types";
import { ApiError } from "../lib/api";
import { gradesApi, objectsApi, type ObjectCaption, type ObjectGrades, type ObjectsArtifact } from "../lib/objects";
import { addObject, applyVerdict, clearVerdict, clientToPdf, dragRect, emptyGrades, gradableObjects, gradingProgress, proposeCaption, removeAddition, type GradableKind, type PdfPoint } from "../lib/objectGrading";
import { overlaps, projectRect, type HintBox } from "../lib/linkHintGeometry";
import { tokensInSpan, type ReadingIndex } from "../lib/readingIndex";
import "./PdfObjectGrading.css";

type Options = {
  paperId: string | null; url: string; page: number; document: PDFDocumentProxy | null; root: RefObject<HTMLElement | null>;
  enabled: boolean; context: string; loadIndex: () => Promise<ReadingIndex>; onPage: (page: number) => void; onOpenPaper?: (id: string) => void;
};
type EntryState = "ungraded" | "correct" | "region" | "reject" | "addition";
type Entry = { id: string; kind: GradableKind; label: string; page: number; region: TextRect | null; anchorRect: TextRect | null; source: "detector" | "addition"; state: EntryState };
type Session = { artifact: ObjectsArtifact; index: ReadingIndex | null };
type Draw = { purpose: "region" | "add"; page: number | null; start: PdfPoint | null };
type AddForm = { stage: "kind" | "label" | "caption"; kind: GradableKind; label: string; page: number; region: TextRect; caption: ObjectCaption | null };
type SaveState = "saved" | "dirty" | "saving" | "conflict" | "error";
type Placement = { kind: "box" | "anchor"; box: HintBox };
type Layout = { boxes: Map<string, Placement>; view: HintBox | null; drag: HintBox | null };

/** Capital G enters the mode from the full-paper PDF text view; `:grade` opens the next queued paper. */
export const OBJECT_GRADING_KEY = "G";
const OPEN_EVENT = "lysilogy:grade-objects";
const SAVE_DELAY = 500;
const KEY_HELP = "[j/k] move · [y] correct · [n] reject · [e] draw region · [a] add missed · [x] remove addition · [u] clear · [c] complete · [N] save + next paper · [?] keys · [Esc] exit";
const KEY_HINT = "[y] correct [n] reject [e] region [a] add [c] complete [N] next";
const CORNER: TextRect = { x_min: 0, y_min: 0, x_max: 1, y_max: 1 };
const SAVE_LABELS: Record<SaveState, string> = { saved: "saved", dirty: "unsaved", saving: "saving…", conflict: "conflict", error: "save failed" };
let pendingPaper: string | null = null;

/** Enter grading on `paperId` once its reader is mounted and its PDF has loaded. */
export function requestObjectGrading(paperId: string): void {
  pendingPaper = paperId;
  window.dispatchEvent(new CustomEvent(OPEN_EVENT, { detail: paperId }));
}

const kindLabel = (kind: GradableKind) => kind === "figure" ? "Figure" : "Table";
const clip = (box: HintBox, view: HintBox): HintBox => ({ left: Math.max(box.left, view.left), top: Math.max(box.top, view.top), right: Math.min(box.right, view.right), bottom: Math.min(box.bottom, view.bottom) });

function surfacePoint(surface: HTMLElement, clientX: number, clientY: number): PdfPoint | null {
  const host = surface.querySelector<HTMLElement>(".pdf-text-layer-host") ?? surface;
  const { left, top, width, height } = host.getBoundingClientRect();
  return clientToPdf({ left, top, width, height }, Number(surface.dataset.pdfWidth), Number(surface.dataset.pdfHeight), clientX, clientY);
}

export function usePdfObjectGrading({ paperId, url, page, document: pdf, root, enabled, context, loadIndex, onPage, onOpenPaper }: Options) {
  const [active, setActive] = useState(false);
  const [loading, setLoading] = useState(false);
  const [session, setSession] = useState<Session | null>(null);
  const [grades, setGrades] = useState<ObjectGrades | null>(null);
  const [focused, setFocused] = useState<string | null>(null);
  const [draw, setDraw] = useState<Draw | null>(null);
  const [form, setForm] = useState<AddForm | null>(null);
  const [message, setMessage] = useState("");
  const [help, setHelp] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>("saved");
  const [layout, setLayout] = useState<Layout>({ boxes: new Map(), view: null, drag: null });
  const activeRef = useRef(false);
  const generation = useRef(0);
  const gradesRef = useRef<ObjectGrades | null>(null);
  const focusedRef = useRef<string | null>(null);
  const paperRef = useRef(paperId);
  const dirty = useRef(false);
  const saveTimer = useRef<number | undefined>(undefined);
  const inflight = useRef<Promise<void> | null>(null);
  const reveal = useRef<{ id: string; requestedPage: boolean } | null>(null);
  const drawRef = useRef<Draw | null>(null);
  const dragEnd = useRef<PdfPoint | null>(null);
  const schedule = useRef<(() => void) | null>(null);
  const openRef = useRef<() => void>(() => undefined);
  useLayoutEffect(() => { paperRef.current = paperId; }, [paperId]);
  useLayoutEffect(() => { focusedRef.current = focused; }, [focused]);
  useLayoutEffect(() => { drawRef.current = draw; if (draw === null) dragEnd.current = null; }, [draw]);

  const live = useCallback((gen: number) => generation.current === gen && activeRef.current, []);

  const flush = useCallback(async (): Promise<void> => {
    window.clearTimeout(saveTimer.current);
    const paper = paperRef.current;
    if (inflight.current !== null) await inflight.current;
    if (!dirty.current || gradesRef.current === null || paper === null) return;
    const gen = generation.current;
    const snapshot = gradesRef.current;
    dirty.current = false;
    if (live(gen)) setSaveState("saving");
    inflight.current = (async () => {
      try {
        const saved = await gradesApi.save(paper, snapshot);
        if (!live(gen)) return;
        const merged = { ...gradesRef.current ?? snapshot, revision: saved.revision, updated_at: saved.updated_at, grader: saved.grader };
        gradesRef.current = merged; setGrades(merged);
        // Edits made during the request keep their own timer, which waits on this promise.
        setSaveState(dirty.current ? "dirty" : "saved");
      } catch (reason: unknown) {
        if (!live(gen)) return;
        const detail = reason instanceof Error ? reason.message : "unknown error";
        if (reason instanceof ApiError && reason.status === 409) {
          // Another save or a rebuilt index owns the file now; the server copy replaces local edits.
          const current = await gradesApi.get(paper).catch(() => null);
          if (!live(gen)) return;
          if (current !== null) { gradesRef.current = current; setGrades(current); }
          setSaveState("conflict"); setMessage(current !== null ? `conflict: reloaded the saved grades (${detail})` : `conflict: ${detail}`);
        } else { dirty.current = true; setSaveState("error"); setMessage(`save failed: ${detail}`); }
      } finally { inflight.current = null; }
    })();
    await inflight.current;
  }, [live]);

  const commit = useCallback((next: ObjectGrades) => {
    gradesRef.current = next; setGrades(next); dirty.current = true; setSaveState("dirty");
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => { void flush(); }, SAVE_DELAY);
  }, [flush]);

  const quit = useCallback(() => {
    if (!activeRef.current) return false;
    void flush();
    activeRef.current = false; ++generation.current;
    setActive(false); setSession(null); setGrades(null); setFocused(null); setDraw(null); setForm(null); setHelp(false); setLoading(false);
    setLayout({ boxes: new Map(), view: null, drag: null });
    return true;
  }, [flush]);
  useLayoutEffect(() => () => { quit(); }, [url, quit]);

  const focus = (id: string | null, target?: number) => {
    setFocused(id); focusedRef.current = id;
    reveal.current = id === null ? null : { id, requestedPage: false };
    if (target !== undefined && root.current?.querySelector(`[data-pdf-page="${target}"]`) === null) onPage(target);
  };

  const open = () => {
    const host = root.current;
    if (!enabled || pdf === null || host === null || paperId === null || activeRef.current) return;
    const gen = ++generation.current;
    activeRef.current = true; dirty.current = false; gradesRef.current = null;
    setActive(true); setLoading(true); setSession(null); setGrades(null); setFocused(null); setDraw(null); setForm(null); setHelp(false);
    setMessage(""); setSaveState("saved");
    void (async () => {
      const [artifact, stored, index] = await Promise.all([objectsApi.get(paperId), gradesApi.get(paperId), loadIndex().catch(() => null)]);
      if (!live(gen)) return;
      const fresh = emptyGrades(artifact);
      const loaded = stored ?? fresh;
      gradesRef.current = loaded; setGrades(loaded); setSession({ artifact, index }); setLoading(false);
      const objects = gradableObjects(artifact);
      const first = objects.find((object) => !(object.id in loaded.verdicts)) ?? objects[0] ?? loaded.additions[0];
      focus(first?.id ?? null, first?.page);
      if (stored !== null && stored.index_sha256 !== fresh.index_sha256) setMessage("Saved grades refer to an older reading index; the server will refuse saves until this paper is regraded.");
      else if (objects.length === 0) setMessage("No detector figures or tables on this paper; press a to add a missed one.");
      else if (index === null) setMessage("Reading index unavailable: captions cannot be proposed for additions.");
    })().catch((reason: unknown) => {
      if (!live(gen)) return;
      setLoading(false); setMessage(`Could not load objects: ${reason instanceof Error ? reason.message : "unknown error"}`);
    });
  };
  useLayoutEffect(() => { openRef.current = open; });
  useEffect(() => {
    const consume = () => {
      if (pendingPaper === null || pendingPaper !== paperId || pdf === null || !enabled) return;
      pendingPaper = null; openRef.current();
    };
    consume();
    window.addEventListener(OPEN_EVENT, consume);
    return () => window.removeEventListener(OPEN_EVENT, consume);
  }, [enabled, paperId, pdf]);

  const entries = useMemo<Entry[]>(() => {
    if (session === null || grades === null) return [];
    const index = session.index;
    const anchorRect = (object: ReturnType<typeof gradableObjects>[number]): TextRect | null => {
      const token = index === null ? undefined : tokensInSpan(index, object.anchor).find((item) => item.page === object.page);
      return token?.rects[0] ?? object.mentions[0]?.rects[0] ?? null;
    };
    const detected = gradableObjects(session.artifact).map((object): Entry => ({
      id: object.id, kind: object.kind, label: object.label, page: object.page, region: object.region, source: "detector",
      anchorRect: object.region === null ? anchorRect(object) : null, state: grades.verdicts[object.id]?.verdict ?? "ungraded",
    }));
    const added = grades.additions.map((addition): Entry => ({
      id: addition.id, kind: addition.kind, label: `${kindLabel(addition.kind)} ${addition.printed_label}`, page: addition.page,
      region: addition.region, anchorRect: null, source: "addition", state: "addition",
    }));
    // Same order as gradableObjects: objects without a region close their page.
    const top = (entry: Entry) => entry.region?.y_min ?? Infinity;
    return [...detected, ...added].sort((a, b) => a.page - b.page || top(a) - top(b) || a.id.localeCompare(b.id));
  }, [grades, session]);

  useLayoutEffect(() => { root.current?.toggleAttribute("data-object-drawing", draw !== null); }, [draw, root]);

  useEffect(() => {
    if (!active) return;
    const host = root.current, viewport = host?.querySelector<HTMLElement>(".pdf-viewport");
    if (host == null || viewport == null) return;
    let frame = 0;
    const update = () => {
      const view = viewport.getBoundingClientRect();
      const visible: HintBox = { left: Math.max(0, view.left), top: Math.max(0, view.top), right: Math.min(window.innerWidth, view.right), bottom: Math.min(window.innerHeight, view.bottom) };
      const pending = reveal.current;
      if (pending !== null) {
        const entry = entries.find((item) => item.id === pending.id);
        if (entry === undefined) reveal.current = null;
        else {
          const box = projectRect(host, entry.page, entry.region ?? entry.anchorRect ?? CORNER);
          const canvas = host.querySelector<HTMLCanvasElement>(`[data-pdf-page="${entry.page}"] .pdf-canvas`);
          if (box === null || canvas?.dataset.rendered !== "true") {
            // Continuous reading renders lazily: bring the placeholder in first, refine once drawn.
            const pageFrame = host.querySelector(`[data-pdf-page="${entry.page}"]`);
            if (!pending.requestedPage && pageFrame !== null) {
              pending.requestedPage = true;
              const target = pageFrame.getBoundingClientRect();
              if (host.dataset.axis === "horizontal") viewport.scrollLeft += target.left - view.left - 12;
              else viewport.scrollTop += target.top - view.top - 12;
            }
          } else {
            reveal.current = null;
            if (box.top < view.top + 24 || box.bottom > view.bottom - 24) viewport.scrollTop += box.top - view.top - 48;
            if (box.left < view.left || box.right > view.right) viewport.scrollLeft += box.left - view.left - 12;
          }
        }
      }
      const boxes = new Map<string, Placement>();
      for (const entry of entries) {
        const box = projectRect(host, entry.page, entry.region ?? entry.anchorRect ?? CORNER);
        if (box === null || !overlaps(box, visible)) continue;
        boxes.set(entry.id, entry.region === null ? { kind: "anchor", box: { left: box.left, top: box.top, right: box.left, bottom: box.top } } : { kind: "box", box: clip(box, visible) });
      }
      let drag: HintBox | null = null;
      const drawing = drawRef.current, end = dragEnd.current;
      if (drawing?.page != null && drawing.start !== null && end !== null) {
        const rect = { x_min: Math.min(drawing.start.x, end.x), x_max: Math.max(drawing.start.x, end.x), y_min: Math.min(drawing.start.y, end.y), y_max: Math.max(drawing.start.y, end.y) };
        const box = projectRect(host, drawing.page, rect);
        drag = box === null ? null : clip(box, visible);
      }
      setLayout({ boxes, view: { left: view.left, top: view.top, right: view.right, bottom: view.bottom }, drag });
    };
    const run = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(update); };
    schedule.current = run;
    const observer = new MutationObserver(run);
    observer.observe(viewport, { subtree: true, childList: true, attributes: true, attributeFilter: ["style", "data-rendered"] });
    viewport.addEventListener("scroll", run, { passive: true }); window.addEventListener("resize", run);
    run();
    return () => { schedule.current = null; observer.disconnect(); cancelAnimationFrame(frame); viewport.removeEventListener("scroll", run); window.removeEventListener("resize", run); };
  }, [active, context, entries, focused, root]);

  const finishDrag = useCallback((drawing: Draw, pageNumber: number, rect: TextRect) => {
    const current = gradesRef.current;
    if (current === null) return;
    if (drawing.purpose === "add") { setForm({ stage: "kind", kind: "figure", label: "", page: pageNumber, region: rect, caption: null }); setMessage(""); return; }
    const id = focusedRef.current;
    if (id === null) return;
    const addition = current.additions.find((item) => item.id === id);
    if (addition !== undefined) commit({ ...current, additions: current.additions.map((item) => item.id === id ? { ...item, page: pageNumber, region: rect } : item) });
    else commit(applyVerdict(current, id, "region", rect));
    setMessage("");
  }, [commit]);

  useEffect(() => {
    if (draw === null) return;
    const host = root.current, viewport = host?.querySelector<HTMLElement>(".pdf-viewport");
    if (host == null || viewport == null) return;
    const down = (event: PointerEvent) => {
      const drawing = drawRef.current;
      if (drawing === null || event.button !== 0 || drawing.start !== null) return;
      const surface = event.target instanceof Element ? event.target.closest<HTMLElement>(".pdf-page-surface") : null;
      const frame = surface?.closest<HTMLElement>("[data-pdf-page]");
      if (surface == null || frame == null) return;
      const number = Number(frame.dataset.pdfPage);
      if (drawing.purpose === "region" && drawing.page !== null && number !== drawing.page) { setMessage(`Draw the box on page ${drawing.page}.`); return; }
      const point = surfacePoint(surface, event.clientX, event.clientY);
      if (point === null) return;
      event.preventDefault(); event.stopPropagation();
      window.getSelection()?.removeAllRanges();
      dragEnd.current = point;
      setDraw({ ...drawing, page: number, start: point });
    };
    const move = (event: PointerEvent) => {
      const drawing = drawRef.current;
      if (drawing?.start === null || drawing?.page == null) return;
      const surface = host.querySelector<HTMLElement>(`[data-pdf-page="${drawing.page}"] .pdf-page-surface`);
      const point = surface === null ? null : surfacePoint(surface, event.clientX, event.clientY);
      if (point === null) return;
      dragEnd.current = point; schedule.current?.();
    };
    const up = (event: PointerEvent) => {
      const drawing = drawRef.current;
      if (drawing?.start === null || drawing?.page == null) return;
      move(event);
      const rect = dragEnd.current === null ? null : dragRect(drawing.start, dragEnd.current);
      if (rect === null) { setMessage("Drag a larger rectangle."); setDraw({ ...drawing, start: null }); return; }
      setDraw(null); finishDrag(drawing, drawing.page, rect);
    };
    viewport.addEventListener("pointerdown", down, true);
    window.addEventListener("pointermove", move, true);
    window.addEventListener("pointerup", up, true);
    return () => { viewport.removeEventListener("pointerdown", down, true); window.removeEventListener("pointermove", move, true); window.removeEventListener("pointerup", up, true); };
  }, [draw, finishDrag, root]);

  const entry = entries.find((item) => item.id === focused);
  const progress = session !== null && grades !== null ? gradingProgress(session.artifact, grades) : null;

  const formKey = (key: string, current: AddForm, index: ReadingIndex | null, gradesNow: ObjectGrades): boolean => {
    if (key === "Escape") { setForm(null); setMessage("Add cancelled."); return true; }
    if (current.stage === "kind") {
      if (key === "f" || key === "t") setForm({ ...current, kind: key === "f" ? "figure" : "table", stage: "label" });
      else if (key === "Enter") setForm({ ...current, stage: "label" });
      return true;
    }
    if (current.stage === "label") {
      if (key === "Enter") {
        if (current.label.length === 0) { setMessage("Type the printed label first (4, IV, S1, 2.3)."); return true; }
        setForm({ ...current, stage: "caption", caption: index === null ? null : proposeCaption(index, current.page, current.kind, current.label) });
      } else if (key === "Backspace") setForm({ ...current, label: current.label.slice(0, -1) });
      else if (key === "Tab") setForm({ ...current, kind: current.kind === "figure" ? "table" : "figure" });
      else if (key.length === 1 && /[\p{L}\p{N}.-]/u.test(key) && current.label.length < 32) setForm({ ...current, label: current.label + key });
      return true;
    }
    if (key === "c") setForm({ ...current, caption: null });
    else if (key === "Enter") {
      const next = addObject(gradesNow, { kind: current.kind, printed_label: current.label, page: current.page, region: current.region, caption: current.caption });
      commit(next); setForm(null);
      focus(next.additions.at(-1)?.id ?? null, current.page);
      setMessage(current.caption === null ? "Added without a caption: the paper cannot be exported until one is set." : "Added.");
    }
    return true;
  };

  const onKey = (event: KeyboardEvent): boolean => {
    if (!enabled || event.isComposing || event.metaKey || event.altKey || event.ctrlKey) return false;
    if (!activeRef.current) { if (event.key === OBJECT_GRADING_KEY) { if (!event.repeat) open(); return true; } return false; }
    const key = event.key;
    if (form !== null && grades !== null) return formKey(key, form, session?.index ?? null, grades);
    if (draw !== null) { if (key === "Escape" || key === "q") { setDraw(null); setMessage("Drawing cancelled."); } return true; }
    if (key === "Escape" || key === "q") return quit();
    if (key === "?") { setHelp((value) => !value); return true; }
    if (event.repeat && key !== "j" && key !== "k") return true;
    if (loading || session === null || grades === null) return true;
    const at = entries.findIndex((item) => item.id === focused);
    if (key === "j" || key === "k") {
      const fallback = key === "j" ? entries.findIndex((item) => item.page >= page) : entries.length - 1;
      const next = entries[at < 0 ? Math.max(0, fallback) : Math.max(0, Math.min(entries.length - 1, at + (key === "j" ? 1 : -1)))];
      if (next !== undefined) { focus(next.id, next.page); setMessage(""); }
      return true;
    }
    if (key === "a") { setDraw({ purpose: "add", page: null, start: null }); setMessage("Drag a box around the missed figure or table (Esc cancels)."); return true; }
    if (key === "c") {
      if (grades.complete) { commit({ ...grades, complete: false }); setMessage("Marked incomplete."); return true; }
      const state = gradingProgress(session.artifact, grades);
      const blocker = state.blockers[0];
      if (state.ungraded.length > 0) setMessage(`${state.ungraded.length} object${state.ungraded.length === 1 ? "" : "s"} still ungraded (first: ${state.ungraded[0] ?? ""}).`);
      else if (blocker !== undefined) setMessage(`${blocker.id}: ${blocker.reason}.`);
      else { commit({ ...grades, complete: true }); setMessage("Marked complete."); }
      return true;
    }
    if (key === "N") {
      const gen = generation.current, paper = paperId;
      if (paper === null) return true;
      setMessage("Saving, then finding the next paper…");
      void flush().then(() => gradesApi.next(paper)).then((next) => {
        if (!live(gen)) return;
        if (next === null) { setMessage("Grading queue exhausted: every queued paper is complete."); return; }
        if (onOpenPaper === undefined) { setMessage(`Next to grade: ${next.paper_id} (${next.title}).`); return; }
        requestObjectGrading(next.paper_id); onOpenPaper(next.paper_id);
      }).catch((reason: unknown) => { if (live(gen)) setMessage(`Could not find the next paper: ${reason instanceof Error ? reason.message : "unknown error"}`); });
      return true;
    }
    if (entry === undefined) { setMessage(entries.length === 0 ? "Nothing to grade here; press a to add a missed object." : "Press j to focus an object."); return true; }
    if (key === "e") { setDraw({ purpose: "region", page: entry.page, start: null }); setMessage(`Drag the correct box for ${entry.id} on page ${entry.page} (Esc cancels).`); return true; }
    if (entry.source === "addition") {
      if (key === "x") {
        const remaining = entries.filter((item) => item.id !== entry.id);
        commit(removeAddition(grades, entry.id));
        const neighbour = remaining[Math.min(at, remaining.length - 1)];
        focus(neighbour?.id ?? null, neighbour?.page); setMessage(`Removed ${entry.id}.`);
      } else if (["y", "n", "u"].includes(key)) setMessage("Additions are truth already: x removes one, e redraws its box.");
      return true;
    }
    if (key === "y") {
      if (entry.region === null) { setMessage(`${entry.id} has no detector region: press e to draw the box, or n to reject.`); return true; }
      commit(applyVerdict(grades, entry.id, "correct")); setMessage("");
    } else if (key === "n") { commit(applyVerdict(grades, entry.id, "reject")); setMessage(""); }
    else if (key === "u") { commit(clearVerdict(grades, entry.id)); setMessage(""); }
    else if (key === "x") setMessage("Only additions can be removed; reject a detector object with n.");
    return true;
  };

  const view = layout.view;
  const captionText = form === null || form.caption === null || session?.index == null ? "" : session.index.text.slice(form.caption.start, form.caption.end).replace(/\s+/gu, " ").trim().slice(0, 96);
  const formText = form === null ? "" : form.stage === "kind" ? "Add: kind? [f] figure · [t] table · Enter keeps figure · Esc cancels"
    : form.stage === "label" ? `Add ${form.kind} p.${form.page} · printed label: ${form.label || "…"} · type it, Enter continues, Tab toggles kind, Esc cancels`
    : `Add ${kindLabel(form.kind)} ${form.label} p.${form.page} · caption: ${form.caption === null ? "none" : captionText} · Enter confirms · c clears the caption · Esc cancels`;
  const panel = active ? createPortal(<div className="pdf-object-grading" data-drawing={draw !== null ? "true" : undefined} aria-label="Grade figures and tables">
    {entries.map((item) => {
      const placed = layout.boxes.get(item.id);
      if (placed === undefined) return null;
      const isFocused = item.id === focused;
      const badge = isFocused ? `${item.id} · ${item.label} · p.${item.page}` : item.id;
      if (placed.kind === "anchor") return <button key={item.id} type="button" className="pdf-object-anchor" data-object-id={item.id} data-state={item.state} data-focused={isFocused ? "true" : undefined}
        style={{ left: placed.box.left, top: placed.box.top }} title={`${item.label}: no detector region`} onClick={() => focus(item.id)}>{badge} · no region</button>;
      return <div key={item.id} className="pdf-object-box" data-object-id={item.id} data-state={item.state} data-focused={isFocused ? "true" : undefined}
        style={{ left: placed.box.left, top: placed.box.top, width: Math.max(2, placed.box.right - placed.box.left), height: Math.max(2, placed.box.bottom - placed.box.top) }}>
        <button type="button" className="pdf-object-badge" title={item.label} onClick={() => focus(item.id)}>{badge}</button>
      </div>;
    })}
    {layout.drag !== null && <div className="pdf-object-drag" style={{ left: layout.drag.left, top: layout.drag.top, width: Math.max(1, layout.drag.right - layout.drag.left), height: Math.max(1, layout.drag.bottom - layout.drag.top) }} />}
    <div className="pdf-object-grading-status" role="status" style={{ left: Math.max(8, view?.left ?? 8), bottom: Math.max(8, window.innerHeight - (view?.bottom ?? window.innerHeight) + 8), maxWidth: Math.max(160, (view === null ? 600 : view.right - view.left) - 16) }}>
      <strong>Grade objects{progress === null ? "" : ` · ${progress.graded}/${progress.total} graded`}{grades?.complete === true ? " · complete" : ""}{grades !== null && grades.additions.length > 0 ? ` · ${grades.additions.length} added` : ""}</strong>
      <span data-object-focus={entry?.id ?? ""}>{form !== null ? formText : draw !== null ? `Drawing: ${draw.purpose === "add" ? "new object" : entry?.id ?? ""} · drag on the page · Esc cancels`
        : loading ? "Loading objects…" : entry !== undefined ? `${entry.id} ${entry.label} p.${entry.page}${entry.region === null ? " · no region" : ""} · ${entry.state}` : "No object focused"}</span>
      <span>{help ? KEY_HELP : KEY_HINT}</span>
      {message.length > 0 && <span className="pdf-object-message">{message}</span>}
      <em data-save-state={saveState}>{SAVE_LABELS[saveState]}</em>
      <button type="button" onClick={quit} aria-label="Exit object grading">×</button>
    </div>
  </div>, window.document.body) : null;
  return { onKey, quit, active, panel };
}
