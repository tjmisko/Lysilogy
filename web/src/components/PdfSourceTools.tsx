import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import type { SectionCrop } from "../lib/sectionCrop";
import { nearestToken, objectAt, readingIndexUrl, sourceSpaceAt, tokensInSpan, type ReadingIndex, type TextSpan } from "../lib/readingIndex";
import type { RegexResult } from "../lib/regexSearch";
import { inclusiveSourceSpan, moveSourceCursor, moveSourceLine, sourceCursor, sourceGrapheme } from "../lib/sourceMotions";
import { resolveSourceMarks, type SourceMark } from "../lib/sourceGeometry";
import "./PdfSourceTools.css";

export type { SourceMark } from "../lib/sourceGeometry";
type Options = {
  url: string;
  page: number;
  root: RefObject<HTMLElement | null>;
  pageSubset?: number[];
  crops: Map<number, SectionCrop | null>;
  markPages: number[];
  onPage: (page: number) => void;
  onOpenFullPaper?: (page: number) => void;
  onClarify: (text: string, page: number) => void;
  onSave: (text: string, page: number) => void;
};
type Resume = { query: string; matches: TextSpan[]; cursor: number; current: number };
const indices = new Map<string, ReadingIndex>();
const resumes = new Map<string, Resume>();

export function usePdfSourceTools({ url, page, root, pageSubset, crops, markPages, onPage, onOpenFullPaper, onClarify, onSave }: Options) {
  const [resume] = useState(() => resumes.get(url));
  const [index, setIndex] = useState<ReadingIndex | null>(indices.get(url) ?? null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState(resume?.query ?? "");
  const [matches, setMatches] = useState<TextSpan[]>(resume?.matches ?? []);
  const [current, setCurrent] = useState(resume?.current ?? 0);
  const [cursor, setCursor] = useState<number | null>(resume?.cursor ?? null);
  const [visual, setVisual] = useState<TextSpan | null>(null);
  const [pendingObject, setPendingObject] = useState<"a" | "i" | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [outside, setOutside] = useState<TextSpan | null>(null);
  const [manualCopy, setManualCopy] = useState<string | null>(null);
  const [jump, setJump] = useState(0);
  const anchor = useRef<number | null>(null);
  const motionCount = useRef("");
  const motionPrefix = useRef("");
  const input = useRef<HTMLInputElement>(null);
  const controller = useRef<AbortController | null>(null);
  const request = useRef<Promise<ReadingIndex> | null>(null);
  const worker = useRef<Worker | null>(null);
  const deadline = useRef<ReturnType<typeof setTimeout> | null>(null);
  const generation = useRef(0);
  const mounted = useRef(true);

  useEffect(() => {
    resumes.delete(url);
    mounted.current = true;
    return () => { mounted.current = false; controller.current?.abort(); worker.current?.terminate(); if (deadline.current !== null) clearTimeout(deadline.current); };
  }, [url]);
  useEffect(() => { if (searchOpen) { input.current?.focus(); input.current?.select(); } }, [searchOpen]);

  const loadIndex = useCallback(async () => {
    const cached = indices.get(url);
    if (cached !== undefined) return cached;
    if (request.current !== null) return request.current;
    const abort = new AbortController();
    controller.current = abort;
    const timeout = setTimeout(() => abort.abort(), 180000);
    request.current = fetch(readingIndexUrl(url), { signal: abort.signal }).then(async (response) => {
      if (!response.ok) throw new Error(`Source indexing failed (${response.status}). Try again after extraction finishes.`);
      const data = await response.json() as ReadingIndex;
      if (typeof data.text !== "string" || !Array.isArray(data.tokens) || ![1, 2].includes(data.schema_version)) throw new Error("The source index is unavailable. Restart the backend to enable paper search.");
      if (indices.size >= 4) indices.delete(indices.keys().next().value ?? "");
      indices.set(url, data);
      if (mounted.current) setIndex(data);
      return data;
    }).finally(() => { clearTimeout(timeout); request.current = null; });
    return request.current;
  }, [url]);

  const inside = useCallback((data: ReadingIndex, span: TextSpan) => {
    if (pageSubset === undefined) return true;
    const tokens = tokensInSpan(data, span);
    if (tokens.length === 0) {
      const space = sourceSpaceAt(data, span.start);
      if (space !== null && span.end <= space.end) tokens.push(space);
    }
    return tokens.length > 0 && tokens.every((token) => pageSubset.includes(token.page) && token.rects.every((rect) =>
      crops.get(token.page)?.regions.some((region) => rect.x_min >= region.x_min - 1 && rect.x_max <= region.x_max + 1 && rect.y_min >= region.y_min - 1 && rect.y_max <= region.y_max + 1)));
  }, [crops, pageSubset]);

  const land = useCallback((data: ReadingIndex, span: TextSpan) => {
    if (!inside(data, span)) { setOutside(span); return; }
    setOutside(null);
    const token = tokensInSpan(data, span)[0];
    if (token !== undefined) onPage(token.page);
    setCursor(span.start);
    setJump((value) => value + 1);
  }, [inside, onPage]);

  const submit = useCallback((pattern: string) => {
    const nextGeneration = ++generation.current;
    worker.current?.terminate();
    motionPrefix.current = ""; motionCount.current = "";
    if (deadline.current !== null) clearTimeout(deadline.current);
    setError(""); setMessage(""); setVisual(null); setPendingObject(null);
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
    if (pendingObject !== null) { setPendingObject(null); return true; }
    if (visual !== null) { setVisual(null); return true; }
    if (searchOpen || busy) { ++generation.current; worker.current?.terminate(); setBusy(false); setSearchOpen(false); root.current?.focus(); return true; }
    if (outside !== null) { setOutside(null); return true; }
    if (matches.length > 0 || message || error) { setMatches([]); setMessage(""); setError(""); return true; }
    return false;
  }, [busy, error, manualCopy, matches.length, message, outside, pendingObject, root, searchOpen, visual]);

  const yank = useCallback(() => {
    if (index === null || visual === null) return;
    const text = index.text.slice(visual.start, visual.end);
    void navigator.clipboard.writeText(text).then(() => { setMessage(`Yanked ${text.length} characters`); setVisual(null); }).catch(() => {
      setManualCopy(text); setError("Clipboard access was denied. Copy the selected text below.");
    });
  }, [index, visual]);

  const moveMatch = useCallback((direction: number) => {
    if (index === null || matches.length === 0) return;
    motionPrefix.current = ""; motionCount.current = "";
    const next = (current + direction + matches.length) % matches.length;
    setCurrent(next); setVisual(null); setPendingObject(null); setMessage(next < current && direction > 0 || next > current && direction < 0 ? "Search wrapped" : "");
    const span = matches[next]; if (span !== undefined) land(index, span);
  }, [current, index, land, matches]);

  const onKey = useCallback((event: KeyboardEvent): boolean => {
    if (event.ctrlKey || event.metaKey || event.altKey || event.isComposing) return false;
    const target = event.target;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target instanceof HTMLElement && target.isContentEditable) return false;
    if (event.key === "/") { setSearchOpen(true); setError(""); return true; }
    if (["q", "Escape"].includes(event.key)) return quit();
    if (event.key === "n" || event.key === "N") { moveMatch(event.key === "n" ? 1 : -1); return matches.length > 0; }
    if (event.key === "v") {
      motionPrefix.current = ""; motionCount.current = "";
      if (visual !== null) { setVisual(null); setPendingObject(null); return true; }
      setError("");
      void loadIndex().then((data) => {
        if (!mounted.current) return;
        if (data.text.length === 0) { setError("No searchable source text is available. Check the extraction limits below."); return; }
        const at = sourceCursor(data.text, cursor ?? data.pages.find((item) => item.number === page)?.start ?? 0);
        const span = sourceGrapheme(data.text, at);
        if (!inside(data, span)) { land(data, span); return; }
        window.getSelection()?.removeAllRanges();
        anchor.current = at; setCursor(at); setVisual(span);
      }).catch((reason: unknown) => setError(reason instanceof Error ? reason.message : "Source indexing failed"));
      return true;
    }
    if (visual === null || index === null || cursor === null) return false;
    if (event.key === "a" || event.key === "i") { motionPrefix.current = ""; motionCount.current = ""; setPendingObject(event.key); return true; }
    if (pendingObject !== null) {
      const span = objectAt(index, cursor, event.key, pendingObject === "a");
      setPendingObject(null);
      if (span !== null) { if (!inside(index, span)) land(index, span); else { anchor.current = span.start; setCursor(sourceCursor(index.text, span.end - 1)); setVisual(span); } }
      return true;
    }
    if (event.key === "y") { motionPrefix.current = ""; motionCount.current = ""; yank(); return true; }
    if (/^[0-9]$/u.test(event.key) && (event.key !== "0" || motionCount.current !== "")) { motionCount.current = (motionCount.current + event.key).slice(0, 3); return true; }
    if (event.key === "g" && motionPrefix.current === "") { motionPrefix.current = "g"; return true; }
    if (event.key === "o") {
      motionPrefix.current = ""; motionCount.current = "";
      const first = anchor.current ?? cursor;
      anchor.current = cursor; setCursor(first); setJump((value) => value + 1); return true;
    }
    const key = motionPrefix.current + event.key;
    motionPrefix.current = "";
    const count = Number(motionCount.current || 1); motionCount.current = "";
    let next = moveSourceCursor(index, cursor, key, count);
    if (next === null && ["j", "k", "ArrowDown", "ArrowUp"].includes(key)) {
      next = moveSourceLine(index, cursor, ["k", "ArrowUp"].includes(key) ? -1 : 1, count) ?? cursor;
    }
    if (next === null) return false;
    next = sourceCursor(index.text, next);
    const span = inclusiveSourceSpan(index.text, anchor.current ?? cursor, next);
    if (!inside(index, span)) { land(index, span); return true; }
    const destination = tokensInSpan(index, { start: next, end: next + 1 })[0] ?? sourceSpaceAt(index, next);
    setCursor(next); setVisual(span);
    onPage(destination?.page ?? index.pages.find((item) => item.start <= next && item.end > next)?.number ?? page);
    setJump((value) => value + 1);
    return true;
  }, [cursor, index, inside, land, loadIndex, matches.length, moveMatch, onPage, page, pendingObject, quit, visual, yank]);

  const pointerCursor = useCallback((pageNumber: number, x: number, y: number) => {
    void loadIndex().then((data) => {
      if (!mounted.current) return;
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
        motionPrefix.current = ""; motionCount.current = "";
      }
    }).catch(() => { /* Native selection remains available when indexing fails. */ });
  }, [loadIndex, root]);

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
        for (const rect of token.rects) {
          if (kind === "match" && result.length >= 4000) return;
          result.push({ page: token.page, rect, kind, start: Math.max(token.start, span.start), end: Math.min(token.end, span.end), token, group: span.start });
        }
      }
    };
    for (const match of matches.slice(0, 10000)) add(match, "match");
    const active = matches[current];
    if (active !== undefined && outside === null) add(active, "current");
    if (visual !== null) add(visual, "visual");
    if (cursor !== null && visual !== null) add(sourceGrapheme(index.text, cursor), "cursor");
    return result;
  }, [current, cursor, index, markPagesKey, matches, outside, visual]);

  useEffect(() => {
    if (cursor === null || outside !== null) return;
    const reveal = () => {
      const mark = visual !== null
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
  }, [cursor, jump, outside, root, visual]);

  const currentMatch = matches[current];
  const matchToken = index !== null && currentMatch !== undefined ? tokensInSpan(index, currentMatch)[0] : undefined;
  const selectedTokens = index !== null && visual !== null ? tokensInSpan(index, visual) : [];
  const selectedPage = selectedTokens[0]?.page ?? page;
  const provenance = selectedTokens.some((token) => token.provenance === "ocr") ? "OCR" : "Source";
  const openOutside = () => {
    if (index === null || outside === null) return;
    const token = tokensInSpan(index, outside)[0];
    if (token === undefined) return;
    resumes.set(url, { query, matches, cursor: outside.start, current });
    onOpenFullPaper?.(token.page);
  };
  const panel = searchOpen || busy || matches.length > 0 || visual !== null || message || error || outside !== null ? <div className="pdf-source-tools" aria-label="Paper search and selection">
    {searchOpen && <form onSubmit={(event) => { event.preventDefault(); submit(query); }}>
      <label><span>/</span><input ref={input} aria-label="Search paper with regular expression" value={query} onChange={(event) => setQuery(event.target.value)} autoComplete="off" spellCheck={false} onKeyDown={(event) => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); quit(); }
      }} /></label><button type="submit" disabled={busy}>Search</button>
    </form>}
    <div className="pdf-source-status" role="status">
      {busy ? <span>Indexing and searching source…</span> : visual !== null ? <><strong>VISUAL{pendingObject !== null ? ` ${pendingObject}` : ""}</strong><span>{visual.end - visual.start} chars · {provenance} · p. {selectedPage}</span><button type="button" onClick={yank}>Yank</button><button type="button" onClick={() => { if (index !== null) onClarify(index.text.slice(visual.start, visual.end), selectedPage); }}>Ask</button><button type="button" onClick={() => { if (index !== null) onSave(index.text.slice(visual.start, visual.end), selectedPage); }}>Save citation</button></> : matches.length > 0 ? <><span>/{query} · {current + 1} / {matches.length}{matchToken !== undefined ? ` · ${matchToken.provenance === "ocr" ? "OCR · " : ""}p. ${matchToken.page}` : ""}</span><button type="button" aria-label="Previous search match" onClick={() => moveMatch(-1)}>↑</button><button type="button" aria-label="Next search match" onClick={() => moveMatch(1)}>↓</button></> : null}
      {message && <span>{message}</span>}{error && <span className="pdf-source-error">{error}</span>}
      <button type="button" aria-label="Close paper search or selection" onClick={quit}>×</button>
    </div>
    {outside !== null && <p>This match is outside the section. <button type="button" onClick={openOutside}>Open match in full paper</button></p>}
    {index !== null && index.gaps.length > 0 && <p className="pdf-source-gap">Search incomplete: {index.gaps.map((gap) => `p. ${gap.page}: ${gap.reason}`).join("; ")}</p>}
    {manualCopy !== null && <textarea aria-label="Text to copy manually" value={manualCopy} readOnly onFocus={(event) => event.target.select()} autoFocus />}
  </div> : null;
  return { panel, marks, index, onKey, pointerCursor, quit, localMode: panel !== null, visualMode: visual !== null };
}
