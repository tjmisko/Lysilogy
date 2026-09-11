import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { PaperAnalysis, PaperMap, PaperSection } from "../types";
import { sectionPages } from "../lib/sectionScope";
import { visiblePdfPage } from "../lib/pdfViewport";
import { animatePages, type PageSnapshot } from "../lib/pageTransition";
import { PdfReader } from "./PdfReader";

type Props = {
  url: string;
  title: string;
  analysis: PaperAnalysis;
  section: PaperSection;
  index: number;
  paperMap: PaperMap | null;
  darkInk: boolean;
  keyboardEnabled: boolean;
  snapshots: PageSnapshot[];
  onToggleInk: () => void;
  onSection: (section: PaperSection, index: number) => void;
  onClose: () => void;
  onFullPaper: (page: number) => void;
  onClarify: (text: string, page: number) => void;
  onSaveReference: (text: string, page: number) => void;
  digest: (onPage: (page: number) => void, keyboardEnabled: boolean, navigation: ReactNode) => ReactNode;
};

const ignore = () => {};

export function SectionFocus({ url, title, analysis, section, index, paperMap, darkInk, keyboardEnabled, snapshots,
  onToggleInk, onSection, onClose, onFullPaper, onClarify, onSaveReference, digest }: Props) {
  const [pageCount, setPageCount] = useState(paperMap?.layout.pages.length ?? Math.max(1, section.pages.end));
  const pages = useMemo(() => sectionPages(section, pageCount), [pageCount, section]);
  const [jump, setJump] = useState({ page: pages[0] ?? 1, request: 0 });
  const [zoom, setZoom] = useState(1);
  const [pane, setPane] = useState<"source" | "digest">("source");
  const scrollRef = useRef<HTMLDivElement>(null);
  const currentPage = pages.includes(jump.page) ? jump.page : pages[0] ?? 1;

  useEffect(() => {
    scrollRef.current?.focus({ preventScroll: true });
  }, [jump.request]);

  useLayoutEffect(() => {
    let stop = () => {};
    let second = 0;
    const frame = window.requestAnimationFrame(() => {
      second = window.requestAnimationFrame(() => { stop = animatePages(snapshots, true); });
    });
    return () => { window.cancelAnimationFrame(frame); window.cancelAnimationFrame(second); stop(); };
  }, [snapshots]);

  const selectSection = (nextIndex: number) => {
    const next = analysis.sections[nextIndex];
    if (next !== undefined) onSection(next, nextIndex);
  };
  const openPage = (page: number) => {
    if (!pages.includes(page)) { onFullPaper(page); return; }
    setPane("source");
    setJump((value) => ({ page, request: value.request + 1 }));
  };

  return <section className="section-focus" aria-label={`Read section: ${section.title}`} data-pane={pane}
    onKeyDown={(event) => {
      if (!keyboardEnabled) return;
      const target = event.target;
      const host = scrollRef.current?.querySelector<HTMLElement>(".pdf-viewport") ?? null;
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement
        || (target instanceof HTMLElement && target.isContentEditable)) return;
      if (event.key === "I") { event.preventDefault(); event.stopPropagation(); onToggleInk(); return; }
      if (pane !== "source" || event.metaKey || event.altKey || (event.ctrlKey && !["d", "u"].includes(event.key))) return;
      let distance: number | null = null;
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); return; }
      if (["j", "ArrowDown"].includes(event.key)) distance = 100;
      if (["k", "ArrowUp"].includes(event.key)) distance = -100;
      if (event.key === "PageDown" || (event.ctrlKey && event.key === "d")) distance = (host?.clientHeight ?? 700) * (event.ctrlKey ? .5 : .9);
      if (event.key === "PageUp" || (event.ctrlKey && event.key === "u")) distance = -(host?.clientHeight ?? 700) * (event.ctrlKey ? .5 : .9);
      if (distance !== null) { event.preventDefault(); event.stopPropagation(); host?.scrollBy({ top: distance, behavior: "auto" }); }
      if (["h", "l", "ArrowLeft", "ArrowRight"].includes(event.key)) {
        event.preventDefault(); event.stopPropagation();
        const visible = host === null ? currentPage : visiblePdfPage(host) ?? currentPage;
        const offset = ["h", "ArrowLeft"].includes(event.key) ? -1 : 1;
        const next = pages[pages.indexOf(visible) + offset];
        if (next !== undefined) openPage(next);
      }
      if (event.key === "]" || event.key === "[") { event.preventDefault(); event.stopPropagation(); selectSection(index + (event.key === "]" ? 1 : -1)); }
      if (event.key === "+" || event.key === "=" || event.key === "-") {
        event.preventDefault(); event.stopPropagation(); setZoom((value) => Math.max(.6, Math.min(2, value + (event.key === "-" ? -.1 : .1))));
      }
    }}>
    <div className="section-pane-tabs" aria-label="Section reading panes">
      <button type="button" aria-pressed={pane === "source"} onClick={() => setPane("source")}>Source pages</button>
      <button type="button" aria-pressed={pane === "digest"} onClick={() => setPane("digest")}>Section digest</button>
    </div>
    <div className="section-focus-columns">
      <div className="section-source-scroll" tabIndex={0} ref={scrollRef} aria-label="Selected section source pages"
        onPointerDownCapture={() => setPane("source")} onFocusCapture={() => setPane("source")}>
        {pages.length === 0 ? <p className="reader-message">This section has no usable page range. <button type="button" onClick={() => onFullPaper(1)}>Open full paper</button></p> :
          <PdfReader url={url} title={title} page={currentPage} pageJump={jump.request} zoom={zoom} darkInk={darkInk} spread={false}
            pageSubset={pages} pageLayouts={paperMap?.layout.pages} section={section}
            keyboardEnabled={keyboardEnabled && pane === "source"}
            onZoom={(delta) => setZoom((value) => Math.max(.6, Math.min(2, value + delta)))}
            onPage={openPage} onPageCount={setPageCount} onToggleInk={onToggleInk} onToggleSpread={ignore}
            onReturnToMap={onClose} onOpenFullPaper={onFullPaper} onClarifySelection={onClarify} onSaveReference={onSaveReference} />}
      </div>
      <div className="section-digest-slot" onPointerDownCapture={() => setPane("digest")} onFocusCapture={() => setPane("digest")}>
        {digest(openPage, pane === "digest", <nav className="section-step" aria-label="Section navigation">
          <button type="button" aria-label="Previous section" disabled={index === 0} onClick={() => selectSection(index - 1)}>←</button>
          <label className="section-picker"><span aria-hidden="true">{index + 1} / {analysis.sections.length} ⌄</span>
            <select aria-label="Selected section" value={index} onChange={(event) => selectSection(Number(event.target.value))}>
              {analysis.sections.map((item, i) => <option key={item.id} value={i}>{i + 1} / {analysis.sections.length} · {item.title}</option>)}
            </select>
          </label>
          <button type="button" aria-label="Next section" disabled={index + 1 >= analysis.sections.length} onClick={() => selectSection(index + 1)}>→</button>
        </nav>)}
      </div>
    </div>
  </section>;
}
