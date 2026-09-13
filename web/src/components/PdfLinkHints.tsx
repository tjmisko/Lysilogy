import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { discoverPaperLinks, hintCodes, type PaperDestination, type PaperLink } from "../lib/paperLinks";
import { nativePageLinks } from "../lib/pdfNativeLinks";
import { overlaps, placeHintBadges, positionLinks, projectRect, type PositionedLink } from "../lib/linkHintGeometry";
import { visiblePdfPage } from "../lib/pdfViewport";
import type { ReadingIndex } from "../lib/readingIndex";
import { loadReadingIndex, readingIndexGeneration } from "../lib/readingIndexCache";
import { objectsApi, objectsMatchGeneration, sourcePaperId, type ObjectsArtifact } from "../lib/objects";
import type { SectionCrop } from "../lib/sectionCrop";
import "./PdfLinkHints.css";

type Options = {
  url: string; page: number; document: PDFDocumentProxy | null; root: RefObject<HTMLElement | null>;
  enabled: boolean; context: string; crops: Map<number, SectionCrop | null>; pageSubset?: number[];
  loadIndex: () => Promise<ReadingIndex>; onPage: (page: number) => void; onOpenFullPaper?: (page: number) => void;
};
type Session = { items: (PositionedLink & { code: string })[]; prefix: string; loading: boolean; message: string; focused: number };
type Jump = { destination: PaperDestination; history: PaperDestination[] };
// A section reader is replaced by the full-paper reader when following outside its crop.
const pendingJumps = new Map<string, Jump>();

export function usePdfLinkHints({ url, page, document: pdf, root, enabled, context, crops, pageSubset, loadIndex, onPage, onOpenFullPaper }: Options) {
  const [initial] = useState(() => pendingJumps.get(url));
  const [landing, setLanding] = useState<PaperDestination | null>(initial?.destination ?? null);
  const landingRef = useRef(landing);
  const [session, setSession] = useState<Session | null>(null);
  const [destinationBox, setDestinationBox] = useState<ReturnType<typeof projectRect>>(null);
  const history = useRef<PaperDestination[]>(initial?.history ?? []);
  const generation = useRef(0);
  const active = useRef(false);
  const quit = useCallback(() => {
    if (!active.current) return false;
    active.current = false; ++generation.current; setSession(null); return true;
  }, []);
  useEffect(() => { pendingJumps.delete(url); }, [url]);
  useLayoutEffect(() => { landingRef.current = landing; }, [landing]);
  const ownsPageJump = useCallback((number: number) => landingRef.current?.page === number, []);
  useLayoutEffect(() => {
    const requests = generation;
    return () => { active.current = false; ++requests.current; };
  }, [url]);
  useEffect(() => { quit(); }, [enabled, context, page, quit]);

  const navigate = useCallback((destination: PaperDestination) => {
    quit();
    const crop = crops.get(destination.page);
    if (pageSubset !== undefined && (!pageSubset.includes(destination.page) || crop == null || !crop.regions.some((region) =>
      destination.rect.x_min >= region.x_min - 1 && destination.rect.x_max <= region.x_max + 1 && destination.rect.y_min >= region.y_min - 1 && destination.rect.y_max <= region.y_max + 1))) {
      if (onOpenFullPaper !== undefined) {
        pendingJumps.set(url, { destination, history: history.current });
        onOpenFullPaper(destination.page);
      }
      return;
    }
    setDestinationBox(null); setLanding({ ...destination }); onPage(destination.page);
  }, [crops, onOpenFullPaper, onPage, pageSubset, quit, url]);

  const follow = (link: PaperLink) => {
    if ("url" in link.destination) {
      window.open(link.destination.url, "_blank", "noopener,noreferrer"); quit(); return;
    }
    const reader = root.current;
    const viewport = reader?.querySelector<HTMLElement>(".pdf-viewport");
    if (viewport != null) {
      const number = visiblePdfPage(viewport) ?? page;
      const host = reader?.querySelector<HTMLElement>(`.pdf-text-layer-host[data-page="${number}"]`);
      const surface = host?.closest<HTMLElement>(".pdf-page-surface");
      if (host != null && surface != null) {
        const box = host.getBoundingClientRect(), view = viewport.getBoundingClientRect();
        const width = Number(surface.dataset.pdfWidth), height = Number(surface.dataset.pdfHeight);
        const x = Math.max(0, (view.left - box.left) * width / box.width), y = Math.max(0, (view.top - box.top + 24) * height / box.height);
        history.current = [...history.current.slice(-19), { page: number, label: "Previous reading position", rect: { x_min: x, x_max: x + 1, y_min: y, y_max: y + 1 } }];
      }
    }
    navigate(link.destination);
  };

  const open = () => {
    const host = root.current;
    if (!enabled || pdf === null || host === null) return;
    const next = ++generation.current;
    const cancelled = () => generation.current !== next || !active.current;
    active.current = true;
    setLanding(null); setDestinationBox(null);
    setSession({ items: [], prefix: "", loading: true, message: "Finding links…", focused: -1 });
    const viewport = host.querySelector(".pdf-viewport")?.getBoundingClientRect();
    const pages = Array.from(host.querySelectorAll<HTMLElement>("[data-pdf-page]")).filter((frame) => viewport !== undefined && overlaps(frame.getBoundingClientRect(), viewport)).map((frame) => Number(frame.dataset.pdfPage));
    // Native annotations remain usable if extraction fails or a scan has no text index.
    void (async () => {
      const paperId = sourcePaperId(url);
      const [indexed, extracted, embedded] = await Promise.allSettled([
        loadIndex(), paperId === null ? Promise.resolve(null) : objectsApi.get(paperId, AbortSignal.timeout(30_000)),
        Promise.allSettled(pages.map((number) => nativePageLinks(pdf, number, null))),
      ]);
      if (cancelled()) return;
      let index = indexed.status === "fulfilled" ? indexed.value : null;
      let artifact: ObjectsArtifact | null = extracted.status === "fulfilled" ? extracted.value : null;
      // A cache generation can change while objects are being derived. Refresh
      // once, then withhold reference links if the two requests still disagree.
      if (index !== null && artifact !== null && paperId !== null && !objectsMatchGeneration(artifact, readingIndexGeneration(index), paperId)) {
        const [freshIndex, freshObjects] = await Promise.allSettled([
          loadReadingIndex(url, { revalidate: true }), objectsApi.get(paperId, AbortSignal.timeout(30_000)),
        ]);
        if (cancelled()) return;
        if (freshIndex.status === "fulfilled") index = freshIndex.value;
        artifact = freshObjects.status === "fulfilled" ? freshObjects.value : null;
      }
      const referencesAvailable = index !== null && paperId !== null && objectsMatchGeneration(artifact, readingIndexGeneration(index), paperId);
      let links: PaperLink[] = [];
      if (index !== null) {
        links = discoverPaperLinks(index, referencesAvailable ? artifact : null, readingIndexGeneration(index));
      }
      const source = positionLinks(host, links.filter((link) => pages.includes(link.page)), index, crops);
      const native = embedded.status === "fulfilled" ? embedded.value.flatMap((result) => result.status === "fulfilled" ? result.value : []) : [];
      const nativePositions = positionLinks(host, native, null, crops);
      // An explicit PDF destination wins over inference at the same printed link.
      // Keep separate members of a grouped citation when one annotation covers the group.
      const merged = [...source];
      for (const item of nativePositions) {
        const matches = source.filter((candidate) => candidate.link.page === item.link.page && overlaps(candidate.box, item.box));
        if (matches.length === 1 && matches[0] !== undefined) {
          const candidate = matches[0];
          const at = merged.indexOf(candidate);
          if (at >= 0) merged[at] = { ...candidate, link: { ...candidate.link, destination: item.link.destination } };
        } else if (matches.length === 0) merged.push(item);
      }
      merged.sort((a, b) => a.link.page - b.link.page || a.box.top - b.box.top || a.box.left - b.box.left);
      const codes = hintCodes(merged.length);
      setSession({ items: merged.map((item, at) => ({ ...item, code: codes[at] ?? "" })), prefix: "", loading: false, focused: -1,
        message: index === null ? "Text links unavailable; showing embedded PDF links." : !referencesAvailable ? "Reference links unavailable; figure, table and embedded links are still included. Press Escape, then f to retry." : index.gaps.length ? "Some pages have no searchable text; embedded links are still included." : "" });
    })().catch(() => {
      if (generation.current === next && active.current) setSession({ items: [], prefix: "", loading: false, focused: -1, message: "Could not load links. Press Escape, then f to retry." });
    });
  };

  const showing = session !== null;
  useEffect(() => {
    if (!showing) return;
    const cancel = () => { quit(); };
    const pointer = (event: Event) => { if (!(event.target instanceof Element) || event.target.closest(".pdf-link-hints") === null) quit(); };
    const focus = (event: Event) => { if (event.target instanceof Element && event.target.closest(".notes-panel, input, textarea, select, [contenteditable=true]") !== null) quit(); };
    window.addEventListener("blur", cancel);
    window.addEventListener("resize", cancel);
    window.addEventListener("pointerdown", pointer, true);
    window.addEventListener("focusin", focus, true);
    const viewport = root.current?.querySelector(".pdf-viewport");
    const startTop = viewport?.scrollTop, startLeft = viewport?.scrollLeft;
    const scroll = () => { if (viewport?.scrollTop !== startTop || viewport?.scrollLeft !== startLeft) quit(); };
    viewport?.addEventListener("scroll", scroll, { passive: true });
    return () => {
      window.removeEventListener("blur", cancel); window.removeEventListener("resize", cancel);
      window.removeEventListener("pointerdown", pointer, true); window.removeEventListener("focusin", focus, true);
      viewport?.removeEventListener("scroll", scroll);
    };
  }, [showing, quit, root]);

  useEffect(() => {
    if (landing === null || root.current === null) return;
    const host = root.current, viewport = host.querySelector<HTMLElement>(".pdf-viewport");
    if (viewport === null) return;
    let revealed = false, requestedPage = false, frame = 0;
    let timer: number | undefined;
    const update = () => {
      const box = projectRect(host, landing.page, landing.rect);
      const canvas = host.querySelector<HTMLCanvasElement>(`[data-pdf-page="${landing.page}"] .pdf-canvas`);
      if (box === null || canvas?.dataset.rendered !== "true") {
        // Continuous reading renders lazily. Bring the placeholder into view
        // first, then refine to the citation once its page has been drawn.
        const pageFrame = host.querySelector(`[data-pdf-page="${landing.page}"]`);
        if (!requestedPage && pageFrame !== null) {
          requestedPage = true;
          const target = pageFrame.getBoundingClientRect(), view = viewport.getBoundingClientRect();
          if (host.dataset.axis === "horizontal") viewport.scrollLeft += target.left - view.left - 12;
          else viewport.scrollTop += target.top - view.top - 12;
        }
        return;
      }
      if (!revealed) {
        revealed = true;
        timer = window.setTimeout(() => { setLanding(null); setDestinationBox(null); }, 5000);
        const view = viewport.getBoundingClientRect();
        viewport.scrollTop += box.top - view.top - 24;
        if (box.left < view.left || box.right > view.right) viewport.scrollLeft += box.left - view.left - 12;
      }
      const positioned = projectRect(host, landing.page, landing.rect);
      const view = viewport.getBoundingClientRect();
      setDestinationBox(positioned !== null && overlaps(positioned, view) ? {
        left: Math.max(positioned.left, view.left), right: Math.min(positioned.right, view.right),
        top: Math.max(positioned.top, view.top), bottom: Math.min(positioned.bottom, view.bottom),
      } : null);
    };
    const schedule = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(update); };
    const observer = new MutationObserver(schedule);
    observer.observe(viewport, { subtree: true, childList: true, attributes: true, attributeFilter: ["style", "data-rendered"] });
    viewport.addEventListener("scroll", schedule, { passive: true }); window.addEventListener("resize", schedule);
    schedule();
    return () => { observer.disconnect(); cancelAnimationFrame(frame); clearTimeout(timer); viewport.removeEventListener("scroll", schedule); window.removeEventListener("resize", schedule); };
  }, [landing, root]);

  const onKey = (event: KeyboardEvent): boolean => {
    if (!enabled || event.isComposing || event.metaKey || event.altKey) return false;
    if (event.ctrlKey) {
      if (event.key === "o" && history.current.length > 0 && !active.current) {
        const previous = history.current.pop(); if (previous !== undefined) navigate(previous); return true;
      }
      return false;
    }
    if (!active.current) { if (event.key === "f") { if (!event.repeat) open(); return true; } return false; }
    if (["Escape", "q"].includes(event.key)) return quit();
    if (event.repeat || session === null || session.loading) return true;
    const filtered = session.items.filter((item) => item.code.startsWith(session.prefix));
    if (event.key === "Backspace") setSession({ ...session, prefix: session.prefix.slice(0, -1), focused: -1 });
    else if (event.key === "Tab") setSession({ ...session, focused: (session.focused + (event.shiftKey ? -1 : 1) + filtered.length) % Math.max(1, filtered.length) });
    else if (event.key === "Enter") {
      const item = session.focused >= 0 ? filtered[session.focused] : filtered.length === 1 ? filtered[0] : undefined;
      if (item !== undefined) follow(item.link);
    } else if (event.key.length === 1) {
      const prefix = session.prefix + event.key.toLocaleLowerCase();
      const match = session.items.find((item) => item.code === prefix);
      if (match !== undefined) follow(match.link);
      else setSession({ ...session, prefix, focused: -1 });
    }
    return true;
  };

  const filtered = session?.items.filter((item) => item.code.startsWith(session.prefix)) ?? [];
  const view = root.current?.querySelector(".pdf-viewport")?.getBoundingClientRect();
  const positions = session === null || view === undefined ? [] : placeHintBadges(session.items, 18 + (session.items[0]?.code.length ?? 1) * 9, view);
  const selected = filtered[session?.focused ?? -1];
  const panel = enabled && session !== null ? createPortal(<div className="pdf-link-hints" aria-label="Follow paper links">
    {filtered.map((item) => {
      const at = session.items.indexOf(item), box = positions[at];
      const destination = "url" in item.link.destination ? item.link.destination.url : `${item.link.destination.label} · p. ${item.link.destination.page}`;
      return box === undefined ? null : <button key={item.link.id} type="button" className={`pdf-link-hint${selected === item ? " is-focused" : ""}`} data-hint-code={item.code} data-link-kind={item.link.kind}
        style={{ left: box.left, top: box.top }} title={`${item.link.label} → ${destination}`} aria-label={`${item.code}: ${item.link.label} → ${destination}`}
        onClick={() => follow(item.link)} onFocus={() => setSession({ ...session, focused: filtered.indexOf(item) })}>
        <span>{item.code.slice(0, session.prefix.length)}</span>{item.code.slice(session.prefix.length)}
      </button>;
    })}
    <div className="pdf-link-hint-status" role="status" style={{ left: Math.max(8, view?.left ?? 8), bottom: Math.max(8, window.innerHeight - (view?.bottom ?? window.innerHeight) + 8), maxWidth: Math.max(160, (view?.width ?? 600) - 16) }}>
      <strong>{session.loading ? "Finding links…" : filtered.length ? `Follow link${session.prefix ? `: ${session.prefix}` : ""}` : session.items.length ? `No hint matches “${session.prefix}”` : "No links visible"}</strong>
      <span>{selected !== undefined ? selected.link.label + ("url" in selected.link.destination ? ` → ${selected.link.destination.url}` : ` → ${selected.link.destination.label}`) : session.loading ? "You can cancel while links load." : "Type a hint · Tab to inspect · Enter to follow · Backspace to edit · Esc to cancel"}</span>
      {!session.loading && session.message && <span>{session.message}</span>}
      {!session.loading && session.items.length === 0 && <span>Only links with a known destination are shown. Scroll to a citation, figure, or table and try f again.</span>}
      <button type="button" onClick={quit} aria-label="Cancel link hints">×</button>
    </div>
  </div>, window.document.body) : null;
  const marker = enabled && landing !== null && destinationBox !== null ? createPortal(<div className="pdf-link-destination" aria-label={`Link destination: ${landing.label}`}
    style={{ left: destinationBox.left, top: destinationBox.top, width: Math.max(3, destinationBox.right - destinationBox.left), height: Math.max(3, destinationBox.bottom - destinationBox.top) }} />,
  window.document.body) : null;
  return { onKey, quit, ownsPageJump, active: enabled && session !== null, panel: <>{panel}{marker}</> };
}
