import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { PaperAnalysis, PaperMap, PaperSection } from "../types";
import { sectionPages } from "../lib/sectionScope";
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
  snapshots: PageSnapshot[];
  onToggleInk: () => void;
  onSection: (section: PaperSection, index: number) => void;
  onClose: () => void;
  onFullPaper: (page: number) => void;
  onClarify: (text: string, page: number) => void;
  onSaveReference: (text: string, page: number) => void;
  digest: (onPage: (page: number) => void, keyboardEnabled: boolean) => ReactNode;
};

const ignore = () => {};

export function SectionFocus({ url, title, analysis, section, index, paperMap, darkInk, snapshots,
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
      const target = event.target;
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement
        || (target instanceof HTMLElement && target.isContentEditable)) return;
      if (event.key === "I") { event.preventDefault(); event.stopPropagation(); onToggleInk(); return; }
      if (pane !== "source" || event.metaKey || event.altKey || (event.ctrlKey && !["d", "u"].includes(event.key))) return;
      let distance: number | null = null;
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); return; }
      if (["j", "ArrowDown"].includes(event.key)) distance = 100;
      if (["k", "ArrowUp"].includes(event.key)) distance = -100;
      if (event.key === "PageDown" || (event.ctrlKey && event.key === "d")) distance = (scrollRef.current?.clientHeight ?? 700) * (event.ctrlKey ? .5 : .9);
      if (event.key === "PageUp" || (event.ctrlKey && event.key === "u")) distance = -(scrollRef.current?.clientHeight ?? 700) * (event.ctrlKey ? .5 : .9);
      if (distance !== null) { event.preventDefault(); event.stopPropagation(); scrollRef.current?.scrollBy({ top: distance, behavior: "auto" }); }
      if (["h", "l", "ArrowLeft", "ArrowRight"].includes(event.key)) {
        event.preventDefault(); event.stopPropagation();
        const host = scrollRef.current;
        const top = host?.getBoundingClientRect().top ?? 0;
        const visible = Array.from(host?.querySelectorAll<HTMLElement>("[data-pdf-page]") ?? []).find((frame) => frame.getBoundingClientRect().bottom > top + 80);
        const offset = ["h", "ArrowLeft"].includes(event.key) ? -1 : 1;
        const next = pages[pages.indexOf(Number(visible?.dataset.pdfPage ?? currentPage)) + offset];
        if (next !== undefined) openPage(next);
      }
      if (event.key === "]" || event.key === "[") { event.preventDefault(); event.stopPropagation(); selectSection(index + (event.key === "]" ? 1 : -1)); }
      if (event.key === "+" || event.key === "=" || event.key === "-") {
        event.preventDefault(); event.stopPropagation(); setZoom((value) => Math.max(.6, Math.min(2, value + (event.key === "-" ? -.1 : .1))));
      }
    }}>
    <header className="section-focus-header">
      <button type="button" onClick={onClose} className="return-to-map">← Whole paper</button>
      <label className="section-choice"><span className="eyebrow">Reading section</span>
        <select aria-label="Selected section" value={index} onChange={(event) => selectSection(Number(event.target.value))}>
          {analysis.sections.map((item, i) => <option key={item.id} value={i}>{item.title}</option>)}
        </select>
      </label>
      <div className="section-step">
        <button type="button" aria-label="Previous section" disabled={index === 0} onClick={() => selectSection(index - 1)}>←</button>
        <span>{index + 1} / {analysis.sections.length}</span>
        <button type="button" aria-label="Next section" disabled={index + 1 >= analysis.sections.length} onClick={() => selectSection(index + 1)}>→</button>
      </div>
    </header>
    <nav className="paper-position" aria-label="Position in the whole paper">
      <span className="eyebrow">Whole paper</span>
      <div>{Array.from({ length: pageCount }, (_, i) => i + 1).map((page) => <button type="button" key={page}
        className={pages.includes(page) ? "is-selected" : ""} aria-current={pages.includes(page) ? "location" : undefined}
        aria-label={`PDF page ${page}${pages.includes(page) ? ", in selected section" : ""}`} onClick={() => {
          if (pages.includes(page)) { openPage(page); return; }
          const next = analysis.sections.findIndex((item) => sectionPages(item, pageCount).includes(page));
          if (next >= 0) selectSection(next); else onFullPaper(page);
        }}>{page}</button>)}</div>
    </nav>
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
            onZoom={(delta) => setZoom((value) => Math.max(.6, Math.min(2, value + delta)))}
            onPage={openPage} onPageCount={setPageCount} onToggleInk={onToggleInk} onToggleSpread={ignore}
            onOpenFullPaper={onFullPaper} onClarifySelection={onClarify} onSaveReference={onSaveReference} />}
      </div>
      <div className="section-digest-slot" onPointerDownCapture={() => setPane("digest")} onFocusCapture={() => setPane("digest")}>
        {digest(openPage, pane === "digest")}
      </div>
    </div>
  </section>;
}
