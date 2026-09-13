import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import type { SectionCrop } from "../lib/sectionCrop";
import { nearestToken, objectAt, selectionSpans, selectionText, selectionWithinSpan, skipSelectionGap, sourceSpaceAt, tokensInSpan, type ReadingIndex, type SourceSelection, type TextSpan } from "../lib/readingIndex";
import { loadReadingIndex, peekReadingIndex } from "../lib/readingIndexCache";
import type { RegexResult } from "../lib/regexSearch";
import { inclusiveSourceSpan, moveSourceCursor, moveSourceLine, sourceCursor, sourceGrapheme } from "../lib/sourceMotions";
import { resolveSourceMarks, type SourceMark } from "../lib/sourceGeometry";
import { cursorDocument, cursorTextDocument, cursorBlock, cursorCopyText, cursorGrapheme, cursorLine, cursorSelection, moveCursorScreen, originalCursor, virtualCursor } from "../lib/cursorDocument";
import { visiblePdfPage } from "../lib/pdfViewport";
import "./PdfSourceTools.css";

export type { SourceMark } from "../lib/sourceGeometry";
type Options = {
  url: string;
  page: number;
  root: RefObject<HTMLElement | null>;
  pageSubset?: number[];
  crops: Map<number, SectionCrop | null>;
  markPages: number[];
  prefetchReady: boolean;
  onPage: (page: number) => void;
  onOpenFullPaper?: (page: number) => void;
  onClarify: (text: string, page: number) => void;
  onSave: (text: string, page: number) => void;
};
type Resume = { query: string; matches: TextSpan[]; cursor: number; current: number };
const resumes = new Map<string, Resume>();

export function usePdfSourceTools({ url, page, root, pageSubset, crops, markPages, prefetchReady, onPage, onOpenFullPaper, onClarify, onSave }: Options) {
  const [resume] = useState(() => resumes.get(url));
  const [index, setIndex] = useState<ReadingIndex | null>(() => peekReadingIndex(url));
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState(resume?.query ?? "");
  const [matches, setMatches] = useState<TextSpan[]>(resume?.matches ?? []);
  const [current, setCurrent] = useState(resume?.current ?? 0);
  const [cursor, setCursor] = useState<number | null>(resume?.cursor ?? null);
  const [visual, setVisual] = useState<SourceSelection | null>(null);
  const [cursorMode, setCursorMode] = useState(false);
  const [cursorLoading, setCursorLoading] = useState(false);
  const [lineNumberMode, setLineNumberMode] = useState<"relative" | "absolute" | "off">("relative");
  const [yankPending, setYankPending] = useState(false);
  const [textBlock, setTextBlock] = useState<string | null>(null);
  const cursorGeneration = useRef(0);
  const operatorCount = useRef(1);
  const preferredColumn = useRef<number | null>(null);
  const cursorDoc = useMemo(() => index === null ? null : cursorDocument(index), [index]);
  const textDoc = useMemo(() => cursorDoc === null ? null : cursorTextDocument(cursorDoc, textBlock), [cursorDoc, textBlock]);
  const [pendingObject, setPendingObject] = useState<"a" | "i" | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [outside, setOutside] = useState<TextSpan | null>(null);
  const [manualCopy, setManualCopy] = useState<string | null>(null);
  const [jump, setJump] = useState(0);
  const anchor = useRef<number | null>(null);
  const visualObject = useRef<SourceSelection | null>(null);
  const motionCount = useRef("");
  const motionPrefix = useRef("");
  const input = useRef<HTMLInputElement>(null);
  const request = useRef<Promise<ReadingIndex> | null>(null);
  const readyIndex = useRef<ReadingIndex | null>(null);
  const worker = useRef<Worker | null>(null);
  const deadline = useRef<ReturnType<typeof setTimeout> | null>(null);
  const generation = useRef(0);
  const mounted = useRef(true);

  useEffect(() => {
    resumes.delete(url);
    mounted.current = true;
    return () => { mounted.current = false; worker.current?.terminate(); if (deadline.current !== null) clearTimeout(deadline.current); };
  }, [url]);
  useEffect(() => { if (searchOpen) { input.current?.focus(); input.current?.select(); } }, [searchOpen]);

  const loadIndex = useCallback(async (priority: "background" | "interactive" = "interactive") => {
    if (readyIndex.current !== null) return readyIndex.current;
    if (request.current !== null) return request.current;
    // Revalidate once per opened reader, including when parsed data is already
    // in memory. All readers share the request; unmounting never cancels it.
    request.current = loadReadingIndex(url, { priority, revalidate: true }).then((data) => {
      if (mounted.current) { readyIndex.current = data; setIndex(data); }
      return data;
    }).finally(() => { request.current = null; });
    return request.current;
  }, [url]);

  useEffect(() => {
    if (!prefetchReady) return;
    const warm = () => { void loadIndex("background").catch(() => { /* Search can retry; background work stays quiet. */ }); };
    // Let PDF loading and the first paint get ahead of optional source work.
    const idle: { requestIdleCallback?: (callback: () => void, options: { timeout: number }) => number; cancelIdleCallback?: (id: number) => void } = window;
    if (idle.requestIdleCallback !== undefined) {
      const handle = idle.requestIdleCallback(warm, { timeout: 1500 });
      return () => idle.cancelIdleCallback?.(handle);
    }
    const handle = window.setTimeout(warm, 500);
    return () => window.clearTimeout(handle);
  }, [loadIndex, prefetchReady]);

  const inside = useCallback((data: ReadingIndex, span: SourceSelection) => {
    if (pageSubset === undefined) return true;
    const tokens = tokensInSpan(data, span);
    if (tokens.length === 0) {
      const space = sourceSpaceAt(data, span.start);
      if (space !== null && span.end <= space.end) tokens.push(space);
    }
    return tokens.length > 0 && tokens.every((token) => pageSubset.includes(token.page) && token.rects.every((rect) =>
      crops.get(token.page)?.regions.some((region) => rect.x_min >= region.x_min - 1 && rect.x_max <= region.x_max + 1 && rect.y_min >= region.y_min - 1 && rect.y_max <= region.y_max + 1)));
  }, [crops, pageSubset]);

  const toggleCursor = useCallback(() => {
    const revision = ++cursorGeneration.current;
    setTextBlock(null);
    motionPrefix.current = ""; motionCount.current = ""; preferredColumn.current = null;
    setYankPending(false); setPendingObject(null); setVisual(null);
    if (cursorMode) { setCursorMode(false); setCursorLoading(false); return; }
    setCursorMode(true); setCursorLoading(true); setError("");
    root.current?.focus({ preventScroll: true });
    void loadIndex().then((data) => {
      if (!mounted.current || revision !== cursorGeneration.current) return;
      const host = root.current?.querySelector<HTMLElement>(".pdf-viewport");
      const currentPage = host == null ? page : visiblePdfPage(host) ?? page;
      const doc = cursorDocument(data);
      const existing = cursor === null ? null : cursorGrapheme(doc, virtualCursor(doc, cursor));
      const token = data.tokens.find((token) => token.page === currentPage && inside(data, token))
        ?? data.tokens.find((token) => inside(data, token));
      const at = existing !== null && inside(data, existing) ? existing.start : token?.start;
      if (at === undefined) { setCursorMode(false); setError("No indexed text is available in this view."); return; }
      window.getSelection()?.removeAllRanges();
      const position = originalCursor(doc, virtualCursor(doc, at));
      setCursor(position); setJump((value) => value + 1);
      onPage(tokensInSpan(data, { start: position, end: position + 1 })[0]?.page ?? currentPage);
    }).catch((reason: unknown) => {
      if (mounted.current && revision === cursorGeneration.current) { setCursorMode(false); setError(reason instanceof Error ? reason.message : "Could not start Cursor mode"); }
    }).finally(() => { if (mounted.current && revision === cursorGeneration.current) setCursorLoading(false); });
  }, [cursor, cursorMode, inside, loadIndex, onPage, page, root]);

  const land = useCallback((data: ReadingIndex, span: TextSpan) => {
    if (!inside(data, span)) { setOutside(span); return; }
    setOutside(null);
    const token = tokensInSpan(data, span)[0];
    if (token !== undefined) onPage(token.page);
    const doc = cursorDocument(data);
    setTextBlock(cursorMode ? cursorBlock(doc, virtualCursor(doc, span.start))?.id ?? null : null);
    setCursor(span.start);
    preferredColumn.current = null; setYankPending(false);
    setJump((value) => value + 1);
  }, [cursorMode, inside, onPage]);

  const submit = useCallback((pattern: string) => {
    const nextGeneration = ++generation.current;
    worker.current?.terminate();
    motionPrefix.current = ""; motionCount.current = "";
    if (deadline.current !== null) clearTimeout(deadline.current);
    setError(""); setMessage(""); setVisual(null); setPendingObject(null); setYankPending(false);
    if (pattern.length === 0) { setMatches([]); setBusy(false); return; }
    if (pattern.length > 2000) { setError("Keep the pattern below 2,000 characters."); return; }
    setBusy(true);
    void loadIndex().then((data) => {
      if (!mounted.current || generation.current !== nextGeneration) return;
      const runner = new Worker(new URL("../lib/regexSearch.worker.ts", import.meta.url), { type: "module" });
      worker.current = runner;
      const finish = (result: RegexResult) => {
        runner.terminate();
        if (deadline.current !== null) clearTimeout(deadline.current);
        if (!mounted.current || generation.current !== nextGeneration) return;
        setBusy(false); setError(result.error ?? ""); setMatches(result.matches); setCurrent(0);
        if (result.error !== null) return;
        if (result.matches.length === 0) setMessage("No matches");
        else {
          setSearchOpen(false); root.current?.focus({ preventScroll: true });
          const start = cursor ?? data.pages.find((item) => item.number === page)?.start ?? 0;
          const at = Math.max(0, result.matches.findIndex((match) => match.start >= start));
          const first = result.matches[at] ?? result.matches[0];
          setCurrent(at); if (first !== undefined) land(data, first);
          if (result.truncated) setMessage("Showing the first 10,000 matches");
        }
      };
      runner.onmessage = (event: MessageEvent<RegexResult>) => finish(event.data);
      runner.onerror = () => finish({ matches: [], truncated: false, error: "Search could not run. Try a simpler pattern." });
      deadline.current = setTimeout(() => finish({ matches: [], truncated: false, error: "Pattern exceeded the 1-second search limit. Try a simpler expression." }), 1000);
      runner.postMessage({ text: data.text, pattern });
    }).catch((reason: unknown) => {
      if (!mounted.current || generation.current !== nextGeneration) return;
      setBusy(false); setError(reason instanceof Error ? reason.message : "Source indexing failed");
    });
  }, [cursor, land, loadIndex, page, root]);

  const quit = useCallback((): boolean => {
    if (motionPrefix.current || motionCount.current) { motionPrefix.current = ""; motionCount.current = ""; return true; }
    if (manualCopy !== null) { setManualCopy(null); return true; }
    if (yankPending) { setYankPending(false); setPendingObject(null); return true; }
    if (pendingObject !== null) { setPendingObject(null); return true; }
    if (visual !== null) { setVisual(null); return true; }
    if (searchOpen || busy) { ++generation.current; worker.current?.terminate(); setBusy(false); setSearchOpen(false); root.current?.focus(); return true; }
    if (outside !== null) { setOutside(null); return true; }
    if (matches.length > 0 || message || error) { setMatches([]); setMessage(""); setError(""); return true; }
    if (cursorMode) { ++cursorGeneration.current; setCursorMode(false); setCursorLoading(false); return true; }
    return false;
  }, [busy, cursorMode, error, manualCopy, matches.length, message, outside, pendingObject, root, searchOpen, visual, yankPending]);

  const copy = useCallback((selection: SourceSelection, linewise = false, wholeBlock = false) => {
    if (index === null) return;
    const atomic = cursorMode && cursorDoc !== null && (textBlock === null || wholeBlock);
    const body = atomic ? cursorCopyText(cursorDoc, selection, new URL(url, window.location.href).href) : selectionText(index, selection);
    const containsBlock = atomic && cursorDoc.blocks.some(block => selectionSpans(selection).some(span => span.start <= block.start && span.end > block.start));
    const text = body + (linewise && !containsBlock ? "\n" : "");
    setYankPending(false); setPendingObject(null);
    void navigator.clipboard.writeText(text).then(() => { setMessage(`Yanked ${text.length} characters`); setVisual(null); }).catch(() => {
      setManualCopy(text); setError("Clipboard access was denied. Copy the selected text below.");
    });
  }, [cursorDoc, cursorMode, index, textBlock, url]);
  const yank = useCallback(() => { if (visual !== null) copy(visual); }, [copy, visual]);

  const moveCursorKey = useCallback((event: KeyboardEvent): boolean => {
    if (!cursorMode) return false;
    const halfPage = ["PageDown", "PageUp"].includes(event.key) || event.ctrlKey && ["d", "u"].includes(event.key);
    if (event.ctrlKey && !halfPage) return false;
    if (cursorDoc === null || cursor === null || cursorLoading) return halfPage || /^[hjklwbegyv0-9ai]$/iu.test(event.key) || event.key.startsWith("Arrow");
    const atomicMove = visual === null && !yankPending && (halfPage || ["j", "k", "ArrowDown", "ArrowUp", "G"].includes(event.key));
    const doc = atomicMove ? cursorDoc : textDoc ?? cursorDoc;
    const at = virtualCursor(doc, cursor), line = cursorLine(doc, at);
    const navigable = pageSubset === undefined ? doc.lines : doc.lines.filter(line => inside(doc.source, cursorSelection(doc, line)));
    const linePosition = navigable.findIndex(candidate => candidate === line);
    if (event.key === "L") { setLineNumberMode(mode => mode === "relative" ? "absolute" : mode === "absolute" ? "off" : "relative"); return true; }
    if (pendingObject !== null) {
      const selection = objectAt(event.key === "p" ? doc.source : doc.index, event.key === "p" ? cursor : at, event.key, pendingObject === "a");
      setPendingObject(null);
      if (selection !== null) {
        const original = event.key === "p" ? selection : cursorSelection(doc, selection);
        if (inside(doc.source, original)) {
          if (yankPending) copy(original);
          else {
            anchor.current = original.start;
            const end = tokensInSpan(doc.source, original).at(-1)?.end ?? original.end;
            setCursor(originalCursor(doc, virtualCursor(doc, sourceCursor(doc.source.text, end - 1))));
            setVisual(original); visualObject.current = original;
            onPage(tokensInSpan(doc.source, original).at(-1)?.page ?? page); setJump(value => value + 1);
          }
        }
      }
      setYankPending(false); return true;
    }
    if (["a", "i"].includes(event.key)) { if (visual !== null || yankPending) setPendingObject(event.key as "a" | "i"); return true; }
    if (/^[0-9]$/u.test(event.key) && (event.key !== "0" || motionCount.current !== "")) { motionCount.current = (motionCount.current + event.key).slice(0, 3); return true; }
    const count = Number(motionCount.current || 1); motionCount.current = "";
    if (event.key === "y") {
      if (visual !== null) copy(visual);
      else if (yankPending && cursorBlock(cursorDoc, virtualCursor(cursorDoc, cursor)) !== undefined) {
        const block = cursorBlock(cursorDoc, virtualCursor(cursorDoc, cursor));
        if (block !== undefined && inside(cursorDoc.source, block)) copy(block, false, true);
        setYankPending(false);
      }
      else if (yankPending && line !== undefined) {
        const end = navigable[Math.min(navigable.length - 1, linePosition + operatorCount.current * count - 1)]?.end ?? line.end;
        const selection = cursorSelection(doc, { start: line.start, end });
        if (inside(doc.source, selection)) copy(selection, true);
        else setYankPending(false);
      } else { operatorCount.current = count; setYankPending(true); }
      return true;
    }
    if (event.key === "g" && motionPrefix.current === "") { motionPrefix.current = "g"; return true; }
    if (event.key === "o" && visual !== null) {
      const first = anchor.current ?? cursor; anchor.current = cursor; setCursor(first);
      onPage(tokensInSpan(doc.source, { start: first, end: first + 1 })[0]?.page ?? page); setJump(value => value + 1); return true;
    }
    const key = motionPrefix.current + event.key; motionPrefix.current = "";
    const steps = count * (yankPending ? operatorCount.current : 1);
    const vertical = ["j", "k", "ArrowDown", "ArrowUp"].includes(key);
    const backwards = ["h", "k", "ArrowLeft", "ArrowUp", "b", "B", "ge", "gE", "(", "PageUp", "u"].includes(key);
    let next: number | null = null;
    if (halfPage) {
      const viewport = root.current?.querySelector<HTMLElement>(".pdf-viewport");
      const surface = root.current?.querySelector<HTMLElement>(`[data-pdf-page="${line?.page ?? page}"] .pdf-page-surface`);
      const bounds = crops.get(line?.page ?? page)?.bounds;
      const height = bounds === undefined ? doc.source.pages.find(p => p.number === (line?.page ?? page))?.height ?? 792 : bounds.y_max - bounds.y_min;
      const scale = (surface?.getBoundingClientRect().height ?? height) / height;
      next = moveCursorScreen(doc, at, backwards ? -1 : 1, (viewport?.clientHeight ?? 650) / Math.max(.1, scale) * .5 * steps);
    } else if (vertical && line !== undefined) {
      preferredColumn.current ??= at - line.start;
      const target = navigable[Math.max(0, Math.min(navigable.length - 1, linePosition + (backwards ? -steps : steps)))];
      if (target !== undefined) next = sourceCursor(doc.index.text, Math.min(target.end - 1, target.start + preferredColumn.current));
    } else {
      preferredColumn.current = null;
      if (key === "0" || key === "Home") next = line?.start ?? at;
      else if (key === "$" || key === "End") next = sourceCursor(doc.index.text, (line?.end ?? at + 1) - 1);
      else if (key === "gg") next = navigable[0]?.start ?? at;
      else if (key === "G") next = navigable.at(-1)?.start ?? at;
      else if (line?.block !== undefined && ["h", "l", "ArrowLeft", "ArrowRight"].includes(key)) {
        const target = navigable[linePosition + (backwards ? -1 : 1)];
        next = target === undefined ? at : backwards ? sourceCursor(doc.index.text, target.end - 1) : target.start;
      }
      else next = moveSourceCursor(doc.index, at, key, steps);
      if (next !== null && ["h", "l", "ArrowLeft", "ArrowRight"].includes(key) && line !== undefined && line.block === undefined) next = Math.max(line.start, Math.min(line.end - 1, next));
    }
    if (next === null) {
      setYankPending(false);
      return ["i", "a", "o", "O", "s", "S", "c", "d", "D", "x", "X", "r", "p", "u", "U", "."].includes(key);
    }
    next = virtualCursor(doc, originalCursor(doc, next));
    let original = originalCursor(doc, next);
    if (visual !== null) original = skipSelectionGap(visualObject.current, original, original > cursor);
    const selection = cursorSelection(doc, inclusiveSourceSpan(doc.index.text, visual === null ? at : virtualCursor(doc, anchor.current ?? cursor), virtualCursor(doc, original)));
    const selected = visual === null ? selection : selectionWithinSpan(visualObject.current, selection);
    if (yankPending) {
      if (vertical && line !== undefined) {
        const target = cursorLine(doc, next) ?? line;
        const selection = cursorSelection(doc, { start: Math.min(line.start, target.start), end: Math.max(line.end, target.end) });
        if (inside(doc.source, selection)) copy(selection, true);
      } else {
        const exclusive = ["w", "W", "b", "B", "h", "l", "ArrowLeft", "ArrowRight"].includes(key);
        const range = exclusive ? { start: Math.min(at, next), end: Math.max(at, next) } : inclusiveSourceSpan(doc.index.text, at, next);
        const value = cursorSelection(doc, range); if (inside(doc.source, value)) copy(value);
      }
      setYankPending(false); return true;
    }
    const destination = cursorGrapheme(doc, virtualCursor(doc, original));
    if (!inside(doc.source, visual === null ? destination : selected)) return true;
    setCursor(original); if (visual !== null) setVisual(selected);
    if (visual === null && (atomicMove || cursorBlock(cursorDoc, virtualCursor(cursorDoc, original))?.id !== textBlock)) setTextBlock(null);
    const targetPage = tokensInSpan(doc.source, destination)[0]?.page ?? page;
    if (targetPage !== page) onPage(targetPage);
    else if (halfPage) {
      const viewport = root.current?.querySelector<HTMLElement>(".pdf-viewport");
      if (viewport !== null && viewport !== undefined) viewport.scrollTop += (backwards ? -1 : 1) * viewport.clientHeight * .5 * steps;
    }
    setJump(value => value + 1); return true;
  }, [copy, crops, cursor, cursorDoc, cursorLoading, cursorMode, inside, onPage, page, pageSubset, pendingObject, root, textBlock, textDoc, visual, yankPending]);

  const moveMatch = useCallback((direction: number) => {
    if (index === null || matches.length === 0) return;
    motionPrefix.current = ""; motionCount.current = "";
    const next = (current + direction + matches.length) % matches.length;
    setCurrent(next); setVisual(null); setPendingObject(null); setMessage(next < current && direction > 0 || next > current && direction < 0 ? "Search wrapped" : "");
    const span = matches[next]; if (span !== undefined) land(index, span);
  }, [current, index, land, matches]);

  const onKey = useCallback((event: KeyboardEvent): boolean => {
    if (event.metaKey || event.altKey || event.isComposing) return false;
    const target = event.target;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target instanceof HTMLElement && target.isContentEditable) return false;
    if (event.ctrlKey) return moveCursorKey(event);
    if (event.key === "C") { if (!event.repeat) toggleCursor(); return true; }
    if (event.key === "/") { setSearchOpen(true); setError(""); return true; }
    if (["q", "Escape"].includes(event.key)) return quit();
    if (event.key === "n" || event.key === "N") { moveMatch(event.key === "n" ? 1 : -1); return matches.length > 0; }
    if (event.key === "v") {
      motionPrefix.current = ""; motionCount.current = "";
      if (visual !== null) { setVisual(null); setPendingObject(null); return true; }
      setError("");
      const revision = cursorGeneration.current;
      const begin = (data: ReadingIndex) => {
        if (!mounted.current || revision !== cursorGeneration.current) return;
        if (data.text.length === 0) { setError("No searchable source text is available. Check the extraction limits below."); return; }
        const at = sourceCursor(data.text, cursor ?? data.pages.find((item) => item.number === page)?.start ?? 0);
        const doc = cursorMode ? textDoc ?? cursorDocument(data) : null;
        const span = doc !== null ? cursorGrapheme(doc, virtualCursor(doc, at)) : sourceGrapheme(data.text, at);
        if (!inside(data, span)) { land(data, span); return; }
        window.getSelection()?.removeAllRanges();
        visualObject.current = null;
        anchor.current = span.start; setCursor(span.start); setVisual(span);
      };
      if (index !== null) begin(index);
      else void loadIndex().then(begin).catch((reason: unknown) => setError(reason instanceof Error ? reason.message : "Source indexing failed"));
      return true;
    }
    if (cursorMode) return moveCursorKey(event);
    if (visual === null || index === null || cursor === null) return false;
    if (event.key === "a" || event.key === "i") { motionPrefix.current = ""; motionCount.current = ""; setPendingObject(event.key); return true; }
    if (pendingObject !== null) {
      const span = objectAt(index, cursor, event.key, pendingObject === "a");
      setPendingObject(null);
      if (span !== null) { if (!inside(index, span)) land(index, span); else {
        visualObject.current = span;
        anchor.current = span.start; setCursor(sourceCursor(index.text, span.end - 1)); setVisual(span);
        if (span.spans?.length) { onPage(tokensInSpan(index, span).at(-1)?.page ?? page); setJump((value) => value + 1); }
      } }
      return true;
    }
    if (event.key === "y") { motionPrefix.current = ""; motionCount.current = ""; yank(); return true; }
    if (/^[0-9]$/u.test(event.key) && (event.key !== "0" || motionCount.current !== "")) { motionCount.current = (motionCount.current + event.key).slice(0, 3); return true; }
    if (event.key === "g" && motionPrefix.current === "") { motionPrefix.current = "g"; return true; }
    if (event.key === "o") {
      motionPrefix.current = ""; motionCount.current = "";
      const first = anchor.current ?? cursor;
      anchor.current = cursor; setCursor(first);
      onPage(tokensInSpan(index, sourceGrapheme(index.text, first))[0]?.page
        ?? index.pages.find((item) => item.start <= first && item.end > first)?.number ?? page);
      setJump((value) => value + 1); return true;
    }
    const key = motionPrefix.current + event.key;
    motionPrefix.current = "";
    const count = Number(motionCount.current || 1); motionCount.current = "";
    let next: number | null = cursor;
    for (let step = 0; step < count; step++) {
      const moved = moveSourceCursor(index, next, key);
      if (moved === null) { next = null; break; }
      const destination = skipSelectionGap(visualObject.current, moved, moved > next);
      if (destination === next) break;
      next = sourceCursor(index.text, destination);
    }
    if (next === null && ["j", "k", "ArrowDown", "ArrowUp"].includes(key)) {
      next = moveSourceLine(index, cursor, ["k", "ArrowUp"].includes(key) ? -1 : 1, count) ?? cursor;
    }
    if (next === null) return false;
    next = sourceCursor(index.text, next);
    next = sourceCursor(index.text, skipSelectionGap(visualObject.current, next, next > cursor));
    const span = selectionWithinSpan(visualObject.current, inclusiveSourceSpan(index.text, anchor.current ?? cursor, next));
    if (!inside(index, span)) { land(index, span); return true; }
    const destination = tokensInSpan(index, { start: next, end: next + 1 })[0] ?? sourceSpaceAt(index, next);
    setCursor(next); setVisual(span);
    onPage(destination?.page ?? index.pages.find((item) => item.start <= next && item.end > next)?.number ?? page);
    setJump((value) => value + 1);
    return true;
  }, [cursor, cursorMode, index, inside, land, loadIndex, matches.length, moveCursorKey, moveMatch, onPage, page, pendingObject, quit, textDoc, toggleCursor, visual, yank]);

  const pointerCursor = useCallback((pageNumber: number, x: number, y: number) => {
    void loadIndex().then((data) => {
      if (!mounted.current) return;
      if (cursorMode) {
        const block = cursorDocument(data).blocks.find(block => block.page === pageNumber && x >= block.rect.x_min && x <= block.rect.x_max && y >= block.rect.y_min && y <= block.rect.y_max);
        if (block !== undefined && inside(data, block)) {
          setTextBlock(null); setCursor(block.start); setVisual(null); setYankPending(false); setPendingObject(null);
          preferredColumn.current = null; return;
        }
      }
      const token = nearestToken(data, pageNumber, x, y);
      if (token !== null) {
        const dimension = data.pages.find((item) => item.number === pageNumber);
        const glyphs: SourceMark[] = [];
        for (let at = token.start; at < token.end;) {
          const span = sourceGrapheme(data.text, at);
          for (const rect of token.rects) glyphs.push({ ...span, page: pageNumber, rect, token, kind: "cursor", group: at });
          at = span.end;
        }
        const host = root.current?.querySelector<HTMLElement>(`.pdf-text-layer-host[data-page="${pageNumber}"]`) ?? null;
        const resolved = resolveSourceMarks(glyphs, host, dimension?.width ?? 612, dimension?.height ?? 792, data.text);
        const nearest = resolved.reduce<SourceMark | null>((best, mark) => {
          const distance = (value: SourceMark) => Math.max(value.rect.x_min - x, x - value.rect.x_max, 0) ** 2 + 4 * Math.max(value.rect.y_min - y, y - value.rect.y_max, 0) ** 2;
          return best === null || distance(mark) < distance(best) ? mark : best;
        }, null);
        setCursor(nearest?.start ?? token.start); setVisual(null); setPendingObject(null);
        preferredColumn.current = null; setYankPending(false);
        motionPrefix.current = ""; motionCount.current = "";
      }
    }).catch(() => { /* Native selection remains available when indexing fails. */ });
  }, [cursorMode, inside, loadIndex, root]);

  const markPagesKey = markPages.join(",");
  const marks = useMemo<SourceMark[]>(() => {
    if (index === null) return [];
    const pages = new Set(markPagesKey.split(",").map(Number));
    const intervals = index.pages.filter((item) => pages.has(item.number));
    const result: SourceMark[] = [];
    const add = (span: TextSpan, kind: SourceMark["kind"]) => {
      if (!intervals.some((item) => item.end > span.start && item.start < span.end)) return;
      const selected = tokensInSpan(index, span);
      // Interior word spaces are filled when line rectangles merge. Include
      // endpoint spaces explicitly so a character cursor can also land on them.
      const firstSpace = sourceSpaceAt(index, span.start);
      const lastSpace = sourceSpaceAt(index, span.end - 1);
      if (firstSpace !== null) selected.unshift(firstSpace);
      if (lastSpace !== null && lastSpace.start !== firstSpace?.start) selected.push(lastSpace);
      for (const token of selected) {
        if (!pages.has(token.page)) continue;
        if (cursorMode && kind === "visual" && cursorDoc?.blocks.some(block => block.id !== textBlock && selectionSpans(block).some(part => token.start >= part.start && token.end <= part.end))) continue;
        for (const rect of token.rects) {
          if (kind === "match" && result.length >= 4000) return;
          result.push({ page: token.page, rect, kind, start: Math.max(token.start, span.start), end: Math.min(token.end, span.end), token, group: span.start });
        }
      }
    };
    for (const match of matches.slice(0, 10000)) add(match, "match");
    const active = matches[current];
    if (active !== undefined && outside === null) add(active, "current");
    if (visual !== null) for (const part of selectionSpans(visual)) add(part, "visual");
    if (cursorMode && cursorDoc !== null && cursor !== null) {
      const at = virtualCursor(cursorDoc, cursor), activeBlock = cursorBlock(cursorDoc, at);
      for (const block of cursorDoc.blocks) {
        if (!pages.has(block.page)) continue;
        const selected = visual !== null && textBlock !== block.id && selectionSpans(visual).some(part => selectionSpans(block).some(piece => part.start < piece.end && piece.start < part.end));
        if (!selected && block !== activeBlock) continue;
        const token = tokensInSpan(index, block)[0]; if (token === undefined) continue;
        result.push({ start: block.start, end: block.end, page: block.page, rect: block.rect, kind: selected ? "block-visual" : "block-cursor", token, group: block.start });
      }
      if (activeBlock === undefined || activeBlock.id === textBlock) add(sourceGrapheme(index.text, cursor), "cursor");
      const currentLine = cursorLine(cursorDoc, at);
      if (lineNumberMode !== "off") for (const line of cursorDoc.lines) {
        if (!pages.has(line.page) || line.block !== undefined) continue;
        const segment = cursorDoc.segments.find(segment => segment.start === line.start);
        if (segment === undefined || !inside(index, segment.source)) continue;
        const active = currentLine === line;
        result.push({ ...segment.source, page: line.page, rect: line.rect, gutterX: line.gutter, kind: "line-number", token: segment.token,
          group: line.number, label: String(lineNumberMode === "absolute" || active ? line.number : Math.abs(line.number - (currentLine?.number ?? 1))), active });
      }
    } else if (cursor !== null && visual !== null) add(sourceGrapheme(index.text, cursor), "cursor");
    return result;
  }, [current, cursor, cursorDoc, cursorMode, index, inside, lineNumberMode, markPagesKey, matches, outside, textBlock, visual]);

  useEffect(() => {
    if (cursor === null || outside !== null) return;
    const reveal = () => {
      const mark = cursorMode
        ? root.current?.querySelector<HTMLElement>(".pdf-source-mark.is-cursor, .pdf-source-mark.is-block-cursor")
          ?? root.current?.querySelector<HTMLElement>(`.pdf-source-mark.is-block-visual[data-source-offset="${cursor}"]`)
        : visual !== null
        ? root.current?.querySelector<HTMLElement>(`.pdf-source-mark.is-cursor[data-source-offset="${cursor}"]`)
        : root.current?.querySelector<HTMLElement>(`.pdf-source-mark.is-current[data-source-offset="${cursor}"]`);
      const viewport = root.current?.querySelector<HTMLElement>(".pdf-viewport");
      if (mark === null || mark === undefined || viewport === null || viewport === undefined) return false;
      const a = mark.getBoundingClientRect(); const b = viewport.getBoundingClientRect();
      if (a.width === 0) return false;
      if (a.top < b.top + 20 || a.bottom > b.bottom - 20) viewport.scrollTop += a.top - b.top - Math.min(90, b.height / 4);
      if (a.left < b.left + 12 || a.right > b.right - 12) viewport.scrollLeft += a.left - b.left - 12;
      return true;
    };
    const observer = new MutationObserver(() => { if (reveal()) observer.disconnect(); });
    const frame = requestAnimationFrame(() => { if (reveal()) observer.disconnect(); });
    const host = root.current;
    if (host !== null) observer.observe(host, { childList: true, subtree: true, attributes: true, attributeFilter: ["style", "data-rendered"] });
    const timeout = setTimeout(() => observer.disconnect(), 5000);
    return () => { cancelAnimationFrame(frame); observer.disconnect(); clearTimeout(timeout); };
  }, [cursor, cursorMode, jump, outside, root, visual]);

  const currentMatch = matches[current];
  const matchToken = index !== null && currentMatch !== undefined ? tokensInSpan(index, currentMatch)[0] : undefined;
  const selectedTokens = index !== null && visual !== null ? tokensInSpan(index, visual) : [];
  const selectedText = index !== null && visual !== null ? cursorMode && cursorDoc !== null && textBlock === null
    ? cursorCopyText(cursorDoc, visual, new URL(url, window.location.href).href) : selectionText(index, visual) : "";
  const selectedPage = selectedTokens[0]?.page ?? page;
  const provenance = selectedTokens.some((token) => token.provenance === "ocr") ? "OCR" : "Source";
  const openOutside = () => {
    if (index === null || outside === null) return;
    const token = tokensInSpan(index, outside)[0];
    if (token === undefined) return;
    resumes.set(url, { query, matches, cursor: outside.start, current });
    onOpenFullPaper?.(token.page);
  };
  const panel = cursorMode || searchOpen || busy || matches.length > 0 || visual !== null || message || error || outside !== null ? <div className="pdf-source-tools" aria-label="Paper search and selection">
    {searchOpen && <form onSubmit={(event) => { event.preventDefault(); submit(query); }}>
      <label><span>/</span><input ref={input} aria-label="Search paper with regular expression" value={query} onChange={(event) => setQuery(event.target.value)} autoComplete="off" spellCheck={false} onKeyDown={(event) => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); quit(); }
      }} /></label><button type="submit" disabled={busy}>Search</button>
    </form>}
    <div className="pdf-source-status" role="status">
      {cursorMode && <><strong className="pdf-cursor-badge">Cursor mode{cursorLoading ? " · loading…" : yankPending ? ` · y${pendingObject ?? ""}` : ""}</strong><button type="button" className="pdf-cursor-numbers" aria-label={`Line numbers: ${lineNumberMode}`} title="Cycle relative / absolute / off (L)" onClick={() => setLineNumberMode(mode => mode === "relative" ? "absolute" : mode === "absolute" ? "off" : "relative")}>{lineNumberMode === "off" ? "Numbers off" : `${lineNumberMode} lines`}</button></>}
      {busy ? <span>Indexing and searching source…</span> : visual !== null ? <><strong>VISUAL{pendingObject !== null ? ` ${pendingObject}` : ""}</strong><span>{selectedText.length} chars · {provenance} · p. {selectedPage}</span><button type="button" onClick={yank}>Yank</button><button type="button" onClick={() => { if (index !== null) onClarify(selectedText, selectedPage); }}>Ask</button><button type="button" onClick={() => { if (index !== null) onSave(selectedText, selectedPage); }}>Save citation</button></> : matches.length > 0 ? <><span>/{query} · {current + 1} / {matches.length}{matchToken !== undefined ? ` · ${matchToken.provenance === "ocr" ? "OCR · " : ""}p. ${matchToken.page}` : ""}</span><button type="button" aria-label="Previous search match" onClick={() => moveMatch(-1)}>↑</button><button type="button" aria-label="Next search match" onClick={() => moveMatch(1)}>↓</button></> : null}
      {message && <span>{message}</span>}{error && <span className="pdf-source-error">{error}</span>}
      <button type="button" aria-label="Close paper search or selection" onClick={quit}>×</button>
    </div>
    {outside !== null && <p>This match is outside the section. <button type="button" onClick={openOutside}>Open match in full paper</button></p>}
    {index !== null && index.gaps.length > 0 && <p className="pdf-source-gap">Search incomplete: {index.gaps.map((gap) => `p. ${gap.page}: ${gap.reason}`).join("; ")}</p>}
    {manualCopy !== null && <textarea aria-label="Text to copy manually" value={manualCopy} readOnly onFocus={(event) => event.target.select()} autoFocus />}
  </div> : null;
  return { panel, marks, index, onKey, pointerCursor, quit, cursorMode, lineNumberMode, localMode: panel !== null, visualMode: visual !== null };
}
