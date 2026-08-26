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

type SectionRegion = {
  section: PaperSection;
  index: number;
  left: number;
  right: number;
  top: number;
  bottom: number;
  verified: boolean;
  label: boolean;
};

function rectStyle(rect: TextRect, page: LayoutPage): PercentStyle {
  return {
    left: `${(rect.x_min / page.width) * 100}%`,
    top: `${(rect.y_min / page.height) * 100}%`,
    width: `${((rect.x_max - rect.x_min) / page.width) * 100}%`,
    height: `${((rect.y_max - rect.y_min) / page.height) * 100}%`,
  };
}

function inferredRegions(
  page: LayoutPage,
  candidates: Array<{ section: PaperSection; index: number }>,
): SectionRegion[] {
  const count = candidates.length;
  if (count === 0) return [];
  const positions = candidates.map(({ section }, index) => {
    const rectangles = section.key_quotes
      .flatMap((quote) => quote.anchor?.page === page.number ? quote.anchor.rects : []);
    if (rectangles.length === 0) return (index + 0.5) / count;
    const center = rectangles.reduce((sum, rect) => sum + (rect.y_min + rect.y_max) / 2, 0)
      / rectangles.length;
    return Math.max(0.02, Math.min(0.98, center / page.height));
  });
  for (let index = 1; index < positions.length; index += 1) {
    positions[index] = Math.max(positions[index] ?? 0, (positions[index - 1] ?? 0) + 0.025);
  }
  return candidates.map(({ section, index }, candidateIndex) => ({
    section,
    index,
    left: 0,
    right: 1,
    top: candidateIndex === 0
      ? 0
      : ((positions[candidateIndex - 1] ?? 0) + (positions[candidateIndex] ?? 1)) / 2,
    bottom: candidateIndex === count - 1
      ? 1
      : ((positions[candidateIndex] ?? 0) + (positions[candidateIndex + 1] ?? 1)) / 2,
    verified: false,
    label: true,
  }));
}

function selectedLineRects(page: LayoutPage, startToken: number, endToken: number): TextRect[] {
  const lines: TextRect[] = [];
  for (const token of page.tokens) {
    if (token.index < startToken || token.index > endToken) continue;
    for (const rect of token.rects) {
      const previous = lines.at(-1);
      const sameLine = previous !== undefined
        && previous.y_min <= rect.y_max
        && rect.y_min <= previous.y_max;
      if (sameLine) {
        previous.x_min = Math.min(previous.x_min, rect.x_min);
        previous.y_min = Math.min(previous.y_min, rect.y_min);
        previous.x_max = Math.max(previous.x_max, rect.x_max);
        previous.y_max = Math.max(previous.y_max, rect.y_max);
      } else {
        lines.push({ ...rect });
      }
    }
  }
  return lines;
}

function sourceBlocks(page: LayoutPage, startToken: number, endToken: number): TextRect[] {
  const lines = selectedLineRects(page, startToken, endToken);
  const blocks: Array<TextRect & { lastLine: TextRect }> = [];
  for (const line of lines) {
    const block = blocks.at(-1);
    if (block === undefined) {
      blocks.push({ ...line, lastLine: line });
      continue;
    }
    const previous = block.lastLine;
    const overlap = Math.max(0, Math.min(previous.x_max, line.x_max) - Math.max(previous.x_min, line.x_min));
    const narrowWidth = Math.max(1, Math.min(previous.x_max - previous.x_min, line.x_max - line.x_min));
    const sameColumn = overlap / narrowWidth >= 0.28
      || Math.abs(previous.x_min - line.x_min) <= page.width * 0.08;
    const readsDownward = line.y_min >= previous.y_min - 1;
    const nearby = line.y_min - previous.y_max <= page.height * 0.075;
    if (sameColumn && readsDownward && nearby) {
      block.x_min = Math.min(block.x_min, line.x_min);
      block.y_min = Math.min(block.y_min, line.y_min);
      block.x_max = Math.max(block.x_max, line.x_max);
      block.y_max = Math.max(block.y_max, line.y_max);
      block.lastLine = line;
    } else {
      blocks.push({ ...line, lastLine: line });
    }
  }
  return blocks.map(({ x_min, y_min, x_max, y_max }) => ({ x_min, y_min, x_max, y_max }));
}

function regionsForPage(page: LayoutPage, sections: PaperSection[]): SectionRegion[] {
  const candidates = sections.flatMap((section, index) =>
    page.number >= section.pages.start && page.number <= section.pages.end
      ? [{ section, index }]
      : [],
  );
  if (candidates.length === 0) return [];
  const verified = candidates.flatMap(({ section, index }) => {
    const span = section.source_span;
    if (span == null || page.number < span.start.page || page.number > span.end.page) return [];
    const startToken = page.number === span.start.page ? span.start.start_token : 0;
    const endToken = page.number === span.end.page
      ? span.end.end_token
      : (page.tokens.at(-1)?.index ?? 0);
    const blocks = sourceBlocks(page, startToken, endToken);
    const labelIndex = blocks.reduce(
      (largest, rect, blockIndex) => {
        const area = (rect.x_max - rect.x_min) * (rect.y_max - rect.y_min);
        return area > largest.area ? { index: blockIndex, area } : largest;
      },
      { index: 0, area: -1 },
    ).index;
    return blocks.map((rect, blockIndex) => ({
      section,
      index,
      left: Math.max(0, rect.x_min / page.width),
      right: Math.min(1, rect.x_max / page.width),
      top: Math.max(0, rect.y_min / page.height),
      bottom: Math.min(1, rect.y_max / page.height),
      verified: true,
      label: blockIndex === labelIndex,
    }));
  });
  const inferred = inferredRegions(
    page,
    candidates.filter(({ section }) => section.source_span == null),
  );
  return [...verified, ...inferred];
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
  const sentenceRefs = useRef(new Map<string, HTMLButtonElement>());

  const mappedPages = useMemo(() => {
    const pageNumbers = new Set<number>();
    for (const section of analysis.sections) {
      for (let page = section.pages.start; page <= section.pages.end; page += 1) {
        pageNumbers.add(page);
      }
    }
    const selected = paperMap.layout.pages.filter((page) => pageNumbers.has(page.number));
    return selected.length > 0 ? selected : paperMap.layout.pages;
  }, [analysis.sections, paperMap.layout.pages]);

  const sentences = useMemo(
    () => mappedPages.flatMap((page) => page.sentences.filter((sentence) => sentence.text.length > 1)),
    [mappedPages],
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
    <section className={`source-map ${markMode ? "is-marking" : ""}`} aria-label="Coordinate-aligned source map">
      <header className="source-map-header">
        <div>
          <span className="eyebrow">Coordinate source map</span>
          <h2>Paper pages beneath the map</h2>
          <p>
            PDF pages retain their exact aspect and boundaries. Solid section edges are verified
            start/end lines; dashed edges are page-level estimates awaiting a fresh analysis.
          </p>
        </div>
        <div className="source-map-controls" role="group" aria-label="Source map layers">
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
        <span>{mappedPages.length} mapped PDF pages</span>
        <span>{anchored} / {citationCount} citations deterministically anchored</span>
        {markMode && (
          <strong>
            <kbd>h/j/k/l</kbd> or arrows move · <kbd>v</kbd> range · <kbd>space</kbd> save · <kbd>c</kbd> clarify
          </strong>
        )}
      </div>
      {error !== null && <p className="inline-error">{error}</p>}
      <div className="source-pages" role="list" aria-label={`Mapped pages from ${title}`}>
        {mappedPages.map((page) => {
          const pageHighlights = paperMap.highlights.filter((highlight) =>
            highlight.anchor.page === page.number
              && ((highlight.origin.type === "ai" && showAi) || (highlight.origin.type === "user" && showUser)),
          );
          const regions = regionsForPage(page, analysis.sections);
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
                      top: `${region.top * 100}%`,
                      height: `${Math.max(0.8, (region.bottom - region.top) * 100)}%`,
                    }}
                    onClick={() => onOpenSection(region.section, region.index)}
                    aria-label={`${region.section.title}; ${region.verified ? "verified" : "estimated"} source region`}
                  >
                    {region.label && <span>{region.section.title}</span>}
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
