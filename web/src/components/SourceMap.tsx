import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  GlobalWorkerOptions,
  getDocument,
  type PDFDocumentProxy,
  type RenderTask,
} from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

import type {
  LayoutPage,
  LayoutSentence,
  PaperAnalysis,
  PaperMap,
  PaperSection,
  TextRect,
} from "../types";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

type SourceMapProps = {
  url: string;
  title: string;
  analysis: PaperAnalysis;
  paperMap: PaperMap;
  activeSection: number;
  darkInk: boolean;
  showAi: boolean;
  showUser: boolean;
  markMode: boolean;
  onShowAi: () => void;
  onShowUser: () => void;
  onMarkMode: () => void;
  onOpenSection: (section: PaperSection, index: number) => void;
  onOpenPage: (page: number) => void;
  onToggleHighlight: (start: LayoutSentence, end?: LayoutSentence) => void;
  onClarify: (text: string, page: number) => void;
};

type PercentStyle = CSSProperties & {
  left: string;
  top: string;
  width: string;
  height: string;
};

type PageGridStyle = CSSProperties & {
  "--page-columns": number;
};

type SectionRegion = {
  section: PaperSection;
  index: number;
  left: number;
  right: number;
  verified: boolean;
};

const MAX_PAGE_COLUMNS = 10;

function rectStyle(rect: TextRect, page: LayoutPage): PercentStyle {
  return {
    left: `${(rect.x_min / page.width) * 100}%`,
    top: `${(rect.y_min / page.height) * 100}%`,
    width: `${((rect.x_max - rect.x_min) / page.width) * 100}%`,
    height: `${((rect.y_max - rect.y_min) / page.height) * 100}%`,
  };
}

function anchorProgress(page: LayoutPage, tokenIndex: number, includeToken: boolean): number | null {
  if (page.tokens.length === 0) return null;
  const ordered = [...page.tokens].sort((left, right) => left.index - right.index);
  const exact = ordered.findIndex((token) => token.index >= tokenIndex);
  const position = exact < 0 ? ordered.length - 1 : exact;
  return Math.max(0, Math.min(1, (position + (includeToken ? 1 : 0)) / ordered.length));
}

function transitionProgress(
  page: LayoutPage,
  previous: PaperSection,
  next: PaperSection,
  fallback: number,
): { progress: number; verified: boolean } {
  const previousEnd = previous.source_span?.end.page === page.number
    ? anchorProgress(page, previous.source_span.end.end_token, true)
    : null;
  const nextStart = next.source_span?.start.page === page.number
    ? anchorProgress(page, next.source_span.start.start_token, false)
    : null;
  const estimates = [previousEnd, nextStart].filter((value): value is number => value !== null);
  if (estimates.length > 0) {
    return {
      progress: estimates.reduce((sum, value) => sum + value, 0) / estimates.length,
      verified: true,
    };
  }

  const previousQuotes = previous.key_quotes.flatMap((quote) =>
    quote.anchor?.page === page.number
      ? [anchorProgress(page, quote.anchor.end_token, true)]
      : [],
  ).filter((value): value is number => value !== null);
  const nextQuotes = next.key_quotes.flatMap((quote) =>
    quote.anchor?.page === page.number
      ? [anchorProgress(page, quote.anchor.start_token, false)]
      : [],
  ).filter((value): value is number => value !== null);
  const inferred = [...previousQuotes, ...nextQuotes];
  return {
    progress: inferred.length === 0
      ? fallback
      : inferred.reduce((sum, value) => sum + value, 0) / inferred.length,
    verified: false,
  };
}

function regionsForPage(page: LayoutPage, sections: PaperSection[]): SectionRegion[] {
  const candidates = sections.flatMap((section, index) =>
    page.number >= section.pages.start && page.number <= section.pages.end
      ? [{ section, index }]
      : [],
  );
  if (candidates.length === 0) return [];
  const boundaries = candidates.slice(0, -1).map(({ section }, boundaryIndex) =>
    transitionProgress(
      page,
      section,
      candidates[boundaryIndex + 1]?.section ?? section,
      (boundaryIndex + 1) / candidates.length,
    ),
  );
  let prior = 0;
  for (let index = 0; index < boundaries.length; index += 1) {
    const remaining = boundaries.length - index;
    const boundary = boundaries[index];
    if (boundary === undefined) continue;
    boundary.progress = Math.max(
      prior + 0.015,
      Math.min(1 - remaining * 0.015, boundary.progress),
    );
    prior = boundary.progress;
  }

  return candidates.map(({ section, index }, candidateIndex) => ({
    section,
    index,
    left: candidateIndex === 0 ? 0 : (boundaries[candidateIndex - 1]?.progress ?? 0),
    right: candidateIndex === candidates.length - 1
      ? 1
      : (boundaries[candidateIndex]?.progress ?? 1),
    verified:
      (candidateIndex === 0 || boundaries[candidateIndex - 1]?.verified === true)
      && (candidateIndex === candidates.length - 1 || boundaries[candidateIndex]?.verified === true),
  }));
}

function defaultColumnCount(pageCount: number): number {
  const available = Math.max(1, Math.min(MAX_PAGE_COLUMNS, pageCount));
  if (window.innerWidth < 560) return Math.min(2, available);
  if (window.innerWidth < 860) return Math.min(3, available);
  if (window.innerWidth < 1180) return Math.min(4, available);
  return Math.min(6, available);
}

function PageCanvas({ document, page, darkInk }: {
  document: PDFDocumentProxy;
  page: LayoutPage;
  darkInk: boolean;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const holderRef = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const holder = holderRef.current;
    if (holder === null || visible) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setVisible(true);
      },
      { rootMargin: "500px" },
    );
    observer.observe(holder);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!visible || canvas === null) return;
    let cancelled = false;
    let renderTask: RenderTask | null = null;
    void document.getPage(page.number).then((pdfPage) => {
      if (cancelled) return;
      const base = pdfPage.getViewport({ scale: 1 });
      const cssWidth = 330;
      const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
      const viewport = pdfPage.getViewport({ scale: (cssWidth / base.width) * pixelRatio });
      const context = canvas.getContext("2d", { alpha: false });
      if (context === null) throw new Error("Canvas rendering is unavailable");
      canvas.width = Math.floor(viewport.width);
      canvas.height = Math.floor(viewport.height);
      renderTask = pdfPage.render({ canvas, canvasContext: context, viewport });
      return renderTask.promise;
    }).catch((reason: unknown) => {
      if (!cancelled && reason instanceof Error && reason.name !== "RenderingCancelledException") {
        // Keep the page frame and coordinate overlays usable when a thumbnail fails.
        console.warn(`Could not render PDF page ${page.number}: ${reason.message}`);
      }
    });
    return () => {
      cancelled = true;
      renderTask?.cancel();
    };
  }, [document, page.number, visible]);

  return (
    <div ref={holderRef} className="source-page-canvas" aria-hidden="true">
      <canvas ref={canvasRef} className={darkInk ? "dark-ink" : ""} />
    </div>
  );
}

function selectionText(sentences: LayoutSentence[], first: number, last: number): string {
  return sentences
    .slice(Math.min(first, last), Math.max(first, last) + 1)
    .map((sentence) => sentence.text)
    .join(" ");
}

export function SourceMap({
  url,
  title,
  analysis,
  paperMap,
  activeSection,
  darkInk,
  showAi,
  showUser,
  markMode,
  onShowAi,
  onShowUser,
  onMarkMode,
  onOpenSection,
  onOpenPage,
  onToggleHighlight,
  onClarify,
}: SourceMapProps) {
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [activeSentence, setActiveSentence] = useState(0);
  const [visualAnchor, setVisualAnchor] = useState<number | null>(null);
  const maxColumns = Math.max(1, Math.min(MAX_PAGE_COLUMNS, paperMap.layout.pages.length));
  const [columns, setColumns] = useState(() => defaultColumnCount(paperMap.layout.pages.length));
  const sentenceRefs = useRef(new Map<string, HTMLButtonElement>());

  const mappedPageNumbers = useMemo(() => {
    const pageNumbers = new Set<number>();
    for (const section of analysis.sections) {
      for (let page = section.pages.start; page <= section.pages.end; page += 1) {
        pageNumbers.add(page);
      }
    }
    return pageNumbers;
  }, [analysis.sections]);

  const pages = paperMap.layout.pages;
  const pageRegions = useMemo(
    () => pages.map((page) => ({ page, regions: regionsForPage(page, analysis.sections) })),
    [analysis.sections, pages],
  );
  const labelPages = useMemo(() => {
    const widest = new Map<string, { page: number; width: number; midpointDistance: number }>();
    for (const { page, regions } of pageRegions) {
      for (const region of regions) {
        const width = region.right - region.left;
        const midpoint = (region.section.pages.start + region.section.pages.end) / 2;
        const midpointDistance = Math.abs(page.number - midpoint);
        const current = widest.get(region.section.id);
        if (
          current === undefined
          || width > current.width
          || (width === current.width && midpointDistance < current.midpointDistance)
        ) {
          widest.set(region.section.id, { page: page.number, width, midpointDistance });
        }
      }
    }
    return new Map([...widest].map(([sectionId, value]) => [sectionId, value.page]));
  }, [pageRegions]);
  const representedPages = pages.filter((page) => mappedPageNumbers.has(page.number)).length;

  const sentences = useMemo(
    () => pages.flatMap((page) => page.sentences.filter((sentence) => sentence.text.length > 1)),
    [pages],
  );
  const safeActive = Math.max(0, Math.min(sentences.length - 1, activeSentence));

  useEffect(() => {
    const task = getDocument({ url });
    let cancelled = false;
    void task.promise.then((loaded) => {
      if (!cancelled) setDocument(loaded);
    }).catch((reason: unknown) => {
      if (!cancelled) setError(reason instanceof Error ? reason.message : "Could not render source pages");
    });
    return () => {
      cancelled = true;
      void task.destroy();
    };
  }, [url]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target;
      if (
        target instanceof HTMLInputElement
        || target instanceof HTMLTextAreaElement
        || target instanceof HTMLSelectElement
        || (target instanceof HTMLElement && target.isContentEditable)
      ) return;
      if (event.key === "+" || event.key === "=") {
        event.preventDefault();
        setColumns((value) => Math.max(1, value - 1));
      } else if (event.key === "-") {
        event.preventDefault();
        setColumns((value) => Math.min(maxColumns, value + 1));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [maxColumns]);

  useEffect(() => {
    if (!markMode) return;
    const sentence = sentences[safeActive];
    if (sentence !== undefined) {
      window.setTimeout(() => {
        const node = sentenceRefs.current.get(sentence.id);
        node?.focus({ preventScroll: true });
        node?.scrollIntoView({ block: "nearest", inline: "center", behavior: "smooth" });
      }, 0);
    }
  }, [markMode, safeActive, sentences]);

  const anchored = analysis.sections.flatMap((section) => section.key_quotes)
    .filter((quote) => quote.validation === "exact" || quote.validation === "normalized").length;
  const citationCount = analysis.sections.flatMap((section) => section.key_quotes).length;

  const moveSentence = (delta: number): void => {
    const currentPage = sentences[safeActive]?.page;
    const candidate = Math.max(0, Math.min(sentences.length - 1, safeActive + delta));
    if (visualAnchor !== null && sentences[candidate]?.page !== currentPage) return;
    setActiveSentence(candidate);
  };

  const commitSelection = (): void => {
    const end = sentences[safeActive];
    const start = visualAnchor === null ? end : sentences[visualAnchor];
    if (start === undefined || end === undefined) return;
    onToggleHighlight(start, visualAnchor === null ? undefined : end);
    setVisualAnchor(null);
  };

  const toggleMarkMode = (): void => {
    if (markMode) setVisualAnchor(null);
    onMarkMode();
  };

  const clarifySelection = (): void => {
    const sentence = sentences[safeActive];
    if (sentence === undefined) return;
    const text = visualAnchor === null
      ? sentence.text
      : selectionText(sentences, visualAnchor, safeActive);
    onClarify(text, sentence.page);
  };

  return (
    <section className={`source-map ${markMode ? "is-marking" : ""}`} aria-label="Paper page map">
      <header className="source-map-header">
        <div>
          <span className="eyebrow">Paper map</span>
          <h2>The whole paper at a glance</h2>
          <p>
            Every cell is one PDF page. Section changes read left to right within a page, turning
            source progress into an abstract map instead of pretending to mark exact page geometry.
          </p>
        </div>
        <div className="source-map-controls" aria-label="Paper map controls">
          <div className="page-grid-zoom" role="group" aria-label="Page grid zoom">
            <button
              type="button"
              disabled={columns <= 1}
              onClick={() => setColumns((value) => Math.max(1, value - 1))}
              aria-label="Zoom in: show one fewer page column"
            >
              +
            </button>
            <output aria-live="polite">{columns} col</output>
            <button
              type="button"
              disabled={columns >= maxColumns}
              onClick={() => setColumns((value) => Math.min(maxColumns, value + 1))}
              aria-label="Zoom out: show one more page column"
            >
              −
            </button>
          </div>
          <button type="button" className={showAi ? "is-active" : ""} aria-pressed={showAi} onClick={onShowAi}>
            AI evidence <kbd>H</kbd>
          </button>
          <button type="button" className={showUser ? "is-active" : ""} aria-pressed={showUser} onClick={onShowUser}>
            My marks <kbd>U</kbd>
          </button>
          <button type="button" className={markMode ? "is-active" : ""} aria-pressed={markMode} onClick={toggleMarkMode}>
            Mark sentences <kbd>v</kbd>
          </button>
        </div>
      </header>
      <div className="source-map-status">
        <span>{pages.length} PDF pages · {representedPages} represented in the section model</span>
        <span><kbd>+</kbd> fewer columns · <kbd>−</kbd> more columns · 10 maximum</span>
        <span>{anchored} / {citationCount} citations deterministically anchored</span>
        {markMode && (
          <strong>
            <kbd>h/j/k/l</kbd> or arrows move · <kbd>v</kbd> range · <kbd>space</kbd> save · <kbd>c</kbd> clarify
          </strong>
        )}
      </div>
      {error !== null && <p className="inline-error">{error}</p>}
      <div
        className="source-pages"
        role="list"
        aria-label={`All pages from ${title}`}
        style={{ "--page-columns": columns } as PageGridStyle}
      >
        {pageRegions.map(({ page, regions }) => {
          const pageHighlights = paperMap.highlights.filter((highlight) =>
            highlight.anchor.page === page.number
              && ((highlight.origin.type === "ai" && showAi) || (highlight.origin.type === "user" && showUser)),
          );
          return (
            <article
              key={page.number}
              className="source-page"
              role="listitem"
              style={{ aspectRatio: `${page.width} / ${page.height}` }}
            >
              {document === null ? <span className="loader" /> : (
                <PageCanvas document={document} page={page} darkInk={darkInk} />
              )}
              <div className="section-regions" aria-label={`Sections on PDF page ${page.number}`}>
                {regions.map((region, regionIndex) => (
                  <button
                    key={`${region.section.id}-${regionIndex}`}
                    type="button"
                    className={`${region.verified ? "is-verified" : "is-inferred"} ${region.index === activeSection ? "is-active" : ""}`}
                    data-family={region.section.family}
                    style={{
                      left: `${region.left * 100}%`,
                      width: `${Math.max(0.8, (region.right - region.left) * 100)}%`,
                    }}
                    onClick={() => onOpenSection(region.section, region.index)}
                    aria-label={`${region.section.title}; ${region.verified ? "anchored" : "estimated"} share of page ${page.number}`}
                  >
                    {labelPages.get(region.section.id) === page.number && (
                      <span>{region.section.title}</span>
                    )}
                  </button>
                ))}
              </div>
              <div className="highlight-layer" aria-hidden="true">
                {pageHighlights.flatMap((highlight) => highlight.anchor.rects.map((rect, index) => (
                  <i
                    key={`${highlight.id}-${index}`}
                    className={`highlight-rect origin-${highlight.origin.type} kind-${highlight.kind}`}
                    style={rectStyle(rect, page)}
                    title={`${highlight.origin.type === "ai" ? "AI citation" : "Reader highlight"}: ${highlight.text}`}
                  />
                )))}
              </div>
              {markMode && (
                <div className="sentence-layer" aria-label={`Selectable sentences on page ${page.number}`}>
                  {page.sentences.flatMap((sentence) => {
                    const sentenceIndex = sentences.findIndex((candidate) => candidate.id === sentence.id);
                    if (sentenceIndex < 0) return [];
                    const selected = visualAnchor !== null
                      && sentence.page === sentences[visualAnchor]?.page
                      && sentenceIndex >= Math.min(visualAnchor, safeActive)
                      && sentenceIndex <= Math.max(visualAnchor, safeActive);
                    const userMarked = paperMap.highlights.some((highlight) =>
                      highlight.origin.type === "user"
                        && highlight.anchor.page === sentence.page
                        && highlight.anchor.start_token <= sentence.start_token
                        && highlight.anchor.end_token >= sentence.end_token,
                    );
                    return sentence.rects.map((rect, rectIndex) => (
                      <button
                        key={`${sentence.id}-${rectIndex}`}
                        ref={(node) => {
                          if (rectIndex === 0 && node !== null) sentenceRefs.current.set(sentence.id, node);
                        }}
                        type="button"
                        className={`${sentenceIndex === safeActive ? "is-active" : ""} ${selected ? "is-selected" : ""} ${userMarked ? "is-user-marked" : ""}`}
                        style={rectStyle(rect, page)}
                        tabIndex={sentenceIndex === safeActive && rectIndex === 0 ? 0 : -1}
                        title={sentence.text}
                        aria-label={`${userMarked ? "Highlighted" : "Highlight"} sentence: ${sentence.text}`}
                        onFocus={() => setActiveSentence(sentenceIndex)}
                        onClick={(event) => {
                          event.stopPropagation();
                          setActiveSentence(sentenceIndex);
                          onToggleHighlight(sentence);
                        }}
                        onKeyDown={(event) => {
                          const key = event.key;
                          if (["h", "j", "k", "l", "ArrowLeft", "ArrowDown", "ArrowUp", "ArrowRight", " ", "Enter", "v", "o", "c", "y", "Escape"].includes(key)) {
                            event.preventDefault();
                            event.stopPropagation();
                          }
                          if (key === "h" || key === "k" || key === "ArrowLeft" || key === "ArrowUp") moveSentence(-1);
                          else if (key === "j" || key === "l" || key === "ArrowDown" || key === "ArrowRight") moveSentence(1);
                          else if (key === " " || key === "Enter") commitSelection();
                          else if (key === "v") setVisualAnchor((anchor) => anchor === null ? safeActive : null);
                          else if (key === "o" && visualAnchor !== null) {
                            const previous = visualAnchor;
                            setVisualAnchor(safeActive);
                            setActiveSentence(previous);
                          } else if (key === "c") clarifySelection();
                          else if (key === "y") {
                            const text = visualAnchor === null
                              ? sentence.text
                              : selectionText(sentences, visualAnchor, safeActive);
                            void navigator.clipboard.writeText(text);
                          } else if (key === "Escape") toggleMarkMode();
                        }}
                      />
                    ));
                  })}
                </div>
              )}
              <button className="source-page-number" type="button" onClick={() => onOpenPage(page.number)}>
                PDF {page.number} ↗
              </button>
            </article>
          );
        })}
      </div>
    </section>
  );
}
