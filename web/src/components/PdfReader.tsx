import { sectionPageCrop, type SectionCrop } from "../lib/sectionCrop";
import { cropTextLayer } from "../lib/cropTextLayer";
import { visiblePdfPage } from "../lib/pdfViewport";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type PDFDocumentProxy,
  type PageViewport,
  type RenderTask,
  type TextLayerImages,
} from "pdfjs-dist";
import { usePdfDocument } from "../hooks/usePdfDocument";
import type { LayoutPage, PaperSection } from "../types";
import { TextLayerBuilder } from "pdfjs-dist/web/pdf_viewer.mjs";

export type PdfSelectionEndpoint = {
  page: number;
  itemIndex: number;
  offset: number;
};

export type PdfSelectionRect = {
  page: number;
  xMin: number;
  yMin: number;
  xMax: number;
  yMax: number;
};

export type PdfTextSelection = {
  text: string;
  pages: number[];
  start: PdfSelectionEndpoint | null;
  end: PdfSelectionEndpoint | null;
  rects: PdfSelectionRect[];
};

type PdfReaderProps = {
  url: string;
  title: string;
  page: number;
  zoom: number;
  darkInk: boolean;
  spread: boolean;
  onPage: (page: number) => void;
  onPageCount: (count: number) => void;
  onToggleInk: () => void;
  onToggleSpread: () => void;
  onClarifySelection: (text: string, page: number) => void;
  onSaveReference: (text: string, page: number) => void;
  pageSubset?: number[];
  pageJump?: number;
  pageLayouts?: LayoutPage[];
  section?: PaperSection;
  onOpenFullPaper?: (page: number) => void;
  onZoom?: (delta: number) => void;
  onReturnToMap?: () => void;
};

type PdfPageCanvasProps = {
  document: PDFDocumentProxy;
  page: number;
  slotWidth: number;
  zoom: number;
  darkInk: boolean;
  pageCount: number;
  onError: (message: string) => void;
  onTextLayer: (page: number, viewport: PageViewport | null, hasText: boolean) => void;
  lazy?: boolean;
  layout?: LayoutPage;
  crop?: SectionCrop;
  readingWidth?: number;
};

type SelectionState = {
  value: PdfTextSelection;
  menuLeft: number;
  menuTop: number;
};

function selectionEndpoint(node: Node, offset: number): PdfSelectionEndpoint | null {
  const element = node instanceof Element ? node : node.parentElement;
  if (element === null) return null;
  const textItem = element.closest<HTMLElement>("[data-text-index]");
  const textLayer = element.closest<HTMLElement>(".pdf-text-layer");
  if (textItem === null || textLayer === null) return null;
  const page = Number(textLayer.dataset.page);
  const itemIndex = Number(textItem.dataset.textIndex);
  if (!Number.isInteger(page) || !Number.isInteger(itemIndex)) return null;
  const textLength = textItem.textContent.length;
  const itemOffset = node.nodeType === Node.TEXT_NODE ? offset : offset === 0 ? 0 : textLength;
  return { page, itemIndex, offset: Math.max(0, Math.min(textLength, itemOffset)) + Number(textItem.dataset.textOffset ?? 0) };
}

function selectionText(selection: Selection): string {
  return selection
    .toString()
    .replace(/[\t\f\v ]+/gu, " ")
    .replace(/ *\n */gu, "\n")
    .trim();
}

function PdfPageCanvas({
  document,
  page,
  slotWidth,
  zoom,
  darkInk,
  pageCount,
  onError,
  onTextLayer,
  lazy = false,
  layout,
  crop,
  readingWidth,
}: PdfPageCanvasProps) {
  const frameRef = useRef<HTMLElement>(null);
  const [visible, setVisible] = useState(!lazy);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const textLayerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (visible || frameRef.current === null) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) setVisible(true);
    }, { root: frameRef.current.closest(".section-source-scroll"), rootMargin: "700px" });
    observer.observe(frameRef.current);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    const surface = surfaceRef.current;
    const canvas = canvasRef.current;
    const textLayerContainer = textLayerRef.current;
    if (!visible || surface === null || canvas === null || textLayerContainer === null) return;
    const controller = new AbortController();
    let renderTask: RenderTask | null = null;
    let textLayer: TextLayerBuilder | null = null;
    textLayerContainer.replaceChildren();
    textLayerContainer.removeAttribute("data-text-ready");
    onTextLayer(page, null, false);

    void document
      .getPage(page)
      .then(async (pdfPage) => {
        if (controller.signal.aborted) return;
        const base = pdfPage.getViewport({ scale: 1 });
        const fitScale = Math.max(0.2, crop === undefined ? (slotWidth - 28) / base.width
          : Math.min(2.2, (slotWidth - 28) / (readingWidth ?? base.width)));
        const viewport = pdfPage.getViewport({ scale: fitScale * zoom });
        const scaleX = viewport.width / (layout?.width ?? base.width);
        const scaleY = viewport.height / (layout?.height ?? base.height);
        const left = (crop?.bounds.x_min ?? 0) * scaleX;
        const top = (crop?.bounds.y_min ?? 0) * scaleY;
        const width = crop === undefined ? viewport.width : (crop.bounds.x_max - crop.bounds.x_min) * scaleX;
        const height = crop === undefined ? viewport.height : (crop.bounds.y_max - crop.bounds.y_min) * scaleY;
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
        const context = canvas.getContext("2d", { alpha: false });
        if (context === null) throw new Error("Canvas rendering is unavailable");

        surface.style.width = `${width}px`;
        surface.style.height = `${height}px`;
        surface.style.setProperty("--total-scale-factor", String(viewport.scale));
        canvas.width = Math.ceil(width * pixelRatio);
        canvas.height = Math.ceil(height * pixelRatio);
        canvas.style.width = `${width}px`;
        canvas.style.height = `${height}px`;
        textLayerContainer.style.width = `${viewport.width}px`;
        textLayerContainer.style.height = `${viewport.height}px`;
        textLayerContainer.style.left = `${-left}px`;
        textLayerContainer.style.top = `${-top}px`;
        // Render directly into the cropped canvas. No hidden full page can be
        // revealed by scrolling, and transition snapshots contain only the crop.
        context.save();
        if (crop !== undefined) {
          context.fillStyle = "white";
          context.fillRect(0, 0, canvas.width, canvas.height);
          context.beginPath();
          for (const r of crop.regions) context.rect((r.x_min * scaleX - left) * pixelRatio,
            (r.y_min * scaleY - top) * pixelRatio, (r.x_max - r.x_min) * scaleX * pixelRatio, (r.y_max - r.y_min) * scaleY * pixelRatio);
          context.clip();
        }

        renderTask = pdfPage.render({
          canvas,
          canvasContext: context,
          viewport,
          transform: [pixelRatio, 0, 0, pixelRatio, -left * pixelRatio, -top * pixelRatio],
        });
        textLayer = new TextLayerBuilder({
          pdfPage,
          abortSignal: controller.signal,
        });
        textLayer.div.classList.add("pdf-text-layer");
        textLayer.div.dataset.page = String(page);
        textLayer.div.setAttribute("aria-label", `Selectable text for page ${page}`);
        textLayerContainer.append(textLayer.div);
        await Promise.all([
          renderTask.promise,
          textLayer.render({
            viewport,
            images: undefined as unknown as TextLayerImages,
          }),
        ]);
        controller.signal.throwIfAborted();
        context.restore();
        canvas.dataset.rendered = "true";

        const textDivs = Array.from(textLayer.div.querySelectorAll<HTMLElement>("span"));
        textDivs.forEach((textDiv, index) => {
          textDiv.dataset.textIndex = String(index);
        });
        if (crop !== undefined) cropTextLayer(textLayer.div, crop.regions, scaleX, scaleY);
        const hasText = Array.from(textLayer.div.querySelectorAll("span")).some((textDiv) => textDiv.textContent.trim().length > 0);
        textLayerContainer.dataset.textReady = "true";
        onTextLayer(page, viewport, hasText);
      })
      .catch((reason: unknown) => {
        if (!controller.signal.aborted && reason instanceof Error &&
          reason.name !== "RenderingCancelledException" && reason.name !== "AbortException") {
          onError(reason.message);
        }
      });
    return () => {
      controller.abort();
      renderTask?.cancel();
      textLayer?.cancel();
      onTextLayer(page, null, false);
    };
  }, [crop, document, layout, onError, onTextLayer, page, readingWidth, slotWidth, visible, zoom]);

  return (
    <figure className="pdf-page-frame" ref={frameRef} data-pdf-page={page} data-section-crop={crop === undefined ? undefined : "true"}
      data-page-crop={crop === undefined || layout === undefined ? undefined : JSON.stringify({
        x: crop.bounds.x_min / layout.width, y: crop.bounds.y_min / layout.height,
        width: (crop.bounds.x_max - crop.bounds.x_min) / layout.width, height: (crop.bounds.y_max - crop.bounds.y_min) / layout.height,
      })}>
      <div className="pdf-page-surface" ref={surfaceRef} style={lazy ? {
        width: crop === undefined ? Math.max(100, (slotWidth - 28) * zoom)
          : (crop.bounds.x_max - crop.bounds.x_min) * Math.max(.2, Math.min(2.2, (slotWidth - 28) / (readingWidth ?? 612))) * zoom,
        aspectRatio: crop === undefined ? `${layout?.width ?? 612} / ${layout?.height ?? 792}`
          : `${crop.bounds.x_max - crop.bounds.x_min} / ${crop.bounds.y_max - crop.bounds.y_min}`,
      } : undefined}>
        <canvas
          ref={canvasRef}
          className={darkInk ? "pdf-canvas dark-ink" : "pdf-canvas"}
          aria-label={`Page ${page} of ${pageCount}`}
        />
        <div
          ref={textLayerRef}
          className="pdf-text-layer-host"
          data-page={page}
        />
      </div>
      <figcaption>{page}</figcaption>
    </figure>
  );
}

export function PdfReader({
  url,
  title,
  page,
  zoom,
  darkInk,
  spread,
  onPage,
  onPageCount,
  onToggleInk,
  onToggleSpread,
  onClarifySelection,
  onSaveReference,
  pageSubset,
  pageJump = 0,
  pageLayouts,
  section,
  onOpenFullPaper,
  onZoom,
  onReturnToMap,
}: PdfReaderProps) {
  const { document: pdfDocument, loading, error: loadError } = usePdfDocument(url);
  const [error, setError] = useState<string | null>(null);
  const [containerWidth, setContainerWidth] = useState(900);
  const [textPages, setTextPages] = useState<Record<number, boolean>>({});
  const [selectionState, setSelectionState] = useState<SelectionState | null>(null);
  const [copyLabel, setCopyLabel] = useState("Copy");
  const [readingPage, setReadingPage] = useState(page);
  const readerRef = useRef<HTMLElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const viewportsRef = useRef(new Map<number, PageViewport>());
  const handleRenderError = useCallback((message: string): void => setError(message), []);

  useEffect(() => {
    const node = containerRef.current;
    if (node === null) return;
    const update = (): void => setContainerWidth(node.clientWidth);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (pdfDocument !== null) onPageCount(pdfDocument.numPages);
  }, [onPageCount, pdfDocument]);

  const pageCount = pdfDocument?.numPages ?? 1;
  const crops = useMemo(() => new Map(pageLayouts?.map((layout) => [layout.number,
    section === undefined ? null : sectionPageCrop(section, layout, pageCount)])), [pageCount, pageLayouts, section]);
  const readingWidth = Math.max(1, ...Array.from(crops.values()).flatMap((crop) => crop === null ? [] : [crop.bounds.x_max - crop.bounds.x_min]));
  const step = spread ? 2 : 1;
  const visiblePages = useMemo(
    () => pageSubset !== undefined
      ? [...new Set(pageSubset)].filter((value) => Number.isInteger(value) && value >= 1 && value <= pageCount).sort((a, b) => a - b)
      : spread && page < pageCount ? [page, page + 1] : [page],
    [page, pageCount, pageSubset, spread],
  );
  const slotWidth = spread ? Math.max(320, (containerWidth - 36) / 2) : containerWidth;
  const lastVisiblePage = visiblePages[visiblePages.length - 1] ?? page;
  const visibleTextStatus = visiblePages.map((visiblePage) => textPages[visiblePage]);
  const textLayersReady = visibleTextStatus.every((status) => status !== undefined);
  const hasSelectableText = visibleTextStatus.some(Boolean);

  useEffect(() => {
    if (pageSubset === undefined) return;
    const host = readerRef.current?.closest<HTMLElement>(".section-source-scroll");
    if (host == null) return;
    const update = () => {
      const visible = visiblePdfPage(host);
      if (visible !== null) setReadingPage(visible);
    };
    host.addEventListener("scroll", update, { passive: true });
    return () => host.removeEventListener("scroll", update);
  }, [pageSubset]);

  useEffect(() => {
    if (pageSubset === undefined || pdfDocument === null) return;
    const frame = window.requestAnimationFrame(() => {
      const host = readerRef.current?.closest<HTMLElement>(".section-source-scroll");
      const target = readerRef.current?.querySelector<HTMLElement>(`[data-pdf-page="${page}"]`);
      if (host == null || target == null) return;
      const toolbarHeight = readerRef.current?.querySelector(".pdf-toolbar")?.getBoundingClientRect().height ?? 40;
      host.scrollTop += target.getBoundingClientRect().top - host.getBoundingClientRect().top - toolbarHeight - 12;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [page, pageJump, pageSubset, pdfDocument]);

  const handleTextLayer = useCallback(
    (pageNumber: number, viewport: PageViewport | null, hasText: boolean): void => {
      if (viewport === null) {
        viewportsRef.current.delete(pageNumber);
        setTextPages((current) => {
          if (!(pageNumber in current)) return current;
          return Object.fromEntries(
            Object.entries(current).filter(([key]) => Number(key) !== pageNumber),
          );
        });
        return;
      }
      viewportsRef.current.set(pageNumber, viewport);
      setTextPages((current) => current[pageNumber] === hasText
        ? current
        : { ...current, [pageNumber]: hasText });
    },
    [],
  );

  const readSelection = useCallback((): void => {
    const root = readerRef.current;
    const selection = window.getSelection();
    if (root === null || selection === null || selection.rangeCount === 0 || selection.isCollapsed ||
      !root.contains(selection.anchorNode) || !root.contains(selection.focusNode)) {
      setSelectionState(null);
      setCopyLabel("Copy");
      return;
    }
    const text = selectionText(selection);
    if (text.length === 0) {
      setSelectionState(null);
      return;
    }

    const range = selection.getRangeAt(0);
    const layerElements = Array.from(root.querySelectorAll<HTMLElement>(".pdf-text-layer"));
    const pdfRects: PdfSelectionRect[] = [];
    let menuRect: DOMRect | null = null;
    for (const clientRect of Array.from(range.getClientRects())) {
      if (clientRect.width <= 0 || clientRect.height <= 0) continue;
      for (const layer of layerElements) {
        const layerRect = layer.getBoundingClientRect();
        const left = Math.max(clientRect.left, layerRect.left);
        const top = Math.max(clientRect.top, layerRect.top);
        const right = Math.min(clientRect.right, layerRect.right);
        const bottom = Math.min(clientRect.bottom, layerRect.bottom);
        if (left >= right || top >= bottom) continue;
        const pageNumber = Number(layer.dataset.page);
        const viewport = viewportsRef.current.get(pageNumber);
        if (viewport === undefined) continue;
        const first = viewport.convertToPdfPoint(
          left - layerRect.left,
          top - layerRect.top,
        ) as [number, number];
        const second = viewport.convertToPdfPoint(
          right - layerRect.left,
          bottom - layerRect.top,
        ) as [number, number];
        pdfRects.push({
          page: pageNumber,
          xMin: Math.min(first[0], second[0]),
          yMin: Math.min(first[1], second[1]),
          xMax: Math.max(first[0], second[0]),
          yMax: Math.max(first[1], second[1]),
        });
        menuRect = clientRect;
        break;
      }
    }
    if (pdfRects.length === 0 || menuRect === null) {
      setSelectionState(null);
      return;
    }
    const pages = [...new Set(pdfRects.map((rect) => rect.page))].sort((left, right) => left - right);
    const value: PdfTextSelection = {
      text,
      pages,
      start: selectionEndpoint(range.startContainer, range.startOffset),
      end: selectionEndpoint(range.endContainer, range.endOffset),
      rects: pdfRects,
    };
    setSelectionState({
      value,
      menuLeft: Math.max(12, Math.min(window.innerWidth - 12, menuRect.left + menuRect.width / 2)),
      menuTop: Math.max(12, Math.min(window.innerHeight - 12, menuRect.bottom + 9)),
    });
    setCopyLabel("Copy");
  }, []);

  useEffect(() => {
    let frame = 0;
    const scheduleRead = (): void => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(readSelection);
    };
    window.document.addEventListener("selectionchange", scheduleRead);
    window.addEventListener("resize", scheduleRead);
    window.addEventListener("scroll", scheduleRead, true);
    return () => {
      window.cancelAnimationFrame(frame);
      window.document.removeEventListener("selectionchange", scheduleRead);
      window.removeEventListener("resize", scheduleRead);
      window.removeEventListener("scroll", scheduleRead, true);
    };
  }, [readSelection]);

  const copySelection = useCallback((): void => {
    if (selectionState === null) return;
    void navigator.clipboard.writeText(selectionState.value.text)
      .then(() => setCopyLabel("Copied"))
      .catch(() => setCopyLabel("Copy failed"));
  }, [selectionState]);

  const clearSelection = useCallback((): void => {
    window.getSelection()?.removeAllRanges();
    setSelectionState(null);
  }, []);

  return (
    <section ref={readerRef} className="pdf-reader" aria-label={`PDF: ${title}`}>
      <div className="pdf-toolbar">
        {pageSubset === undefined ? <div>
          <span className="eyebrow">Source document</span>
          <strong>{title}</strong>
        </div> : <button className="return-to-map" type="button" onClick={onReturnToMap}>← Map</button>}
        <div className="pdf-controls">
          {pageSubset === undefined && <><button type="button" onClick={() => onPage(Math.max(1, page - step))} disabled={page <= 1}>
            <span>Prev</span>
          </button>
          <span>
            {spread && lastVisiblePage !== page ? `${page}–${lastVisiblePage}` : page} / {pageCount}
          </span>
          <button
            type="button"
            onClick={() => onPage(Math.min(pageCount, page + step))}
            disabled={lastVisiblePage >= pageCount}
          >
            <span>Next</span>
          </button>
          <button type="button" className={spread ? "is-active" : ""} onClick={onToggleSpread}>
            {spread ? "Two pages" : "One page"}
          </button></>}
          {pageSubset !== undefined && <>
            <select aria-label="Source page" value={visiblePages.includes(readingPage) ? readingPage : page} onChange={(event) => {
              const next = Number(event.target.value); setReadingPage(next); onPage(next);
            }}>
              {visiblePages.map((value) => <option key={value} value={value}>Page {value} / {pageCount}</option>)}
            </select>
            <button type="button" aria-label="Zoom out source pages" disabled={zoom <= .6} onClick={() => onZoom?.(-.1)}>−</button>
            <button type="button" aria-label="Zoom in source pages" disabled={zoom >= 2} onClick={() => onZoom?.(.1)}>+</button>
            <button type="button" onClick={() => {
              onOpenFullPaper?.(readingPage);
            }}>Open full paper ↗</button>
          </>}
          <button type="button" className={darkInk ? "is-active" : ""} onClick={onToggleInk}>
            {darkInk ? "Dark" : "Light"}
          </button>
        </div>
      </div>
      <div className={`pdf-viewport ${spread ? "is-spread" : ""}`} ref={containerRef}>
        {loading && <div className="reader-message"><span className="loader" /> Rendering source…</div>}
        {(error ?? loadError) !== null && <div className="reader-message error-message">{error ?? loadError}</div>}
        {pdfDocument !== null && visiblePages.map((visiblePage) => pageSubset !== undefined && crops.get(visiblePage) == null
          ? <p className="reader-message" key={visiblePage}>Exact section bounds are unavailable for page {visiblePage}.{" "}
            <button type="button" onClick={() => onOpenFullPaper?.(visiblePage)}>Open full page ↗</button></p> : (
          <PdfPageCanvas
            key={visiblePage}
            document={pdfDocument}
            page={visiblePage}
            slotWidth={slotWidth}
            zoom={zoom}
            darkInk={darkInk}
            pageCount={pageCount}
            onError={handleRenderError}
            onTextLayer={handleTextLayer}
            lazy={pageSubset !== undefined}
            layout={pageLayouts?.find((item) => item.number === visiblePage)}
            crop={pageSubset === undefined ? undefined : crops.get(visiblePage) ?? undefined}
            readingWidth={readingWidth}
          />
        ))}
      </div>
      <div className={`pdf-mode-note ${textLayersReady && !hasSelectableText ? "is-warning" : ""}`}>
        {textLayersReady && !hasSelectableText
          ? "This page has no embedded text. It needs OCR before passages can be selected."
          : "Select any passage to copy it or ask for an explanation."}
        {pageSubset === undefined && <>{" "}<kbd>Ctrl-d</kbd> / <kbd>PageDown</kbd> page forward; <kbd>Ctrl-u</kbd> / <kbd>PageUp</kbd> page back.</>}
      </div>
      {selectionState !== null && (
        <div
          className="pdf-selection-menu"
          style={{ left: selectionState.menuLeft, top: selectionState.menuTop }}
          role="toolbar"
          aria-label="Selected PDF text"
          onMouseDown={(event) => event.preventDefault()}
        >
          <span title={selectionState.value.text}>
            {selectionState.value.text.length} chars · {selectionState.value.pages.length === 1
              ? `page ${selectionState.value.pages[0]}`
              : `pages ${selectionState.value.pages[0]}–${selectionState.value.pages.at(-1)}`}
          </span>
          <button type="button" onClick={copySelection}>{copyLabel}</button>
          <button type="button" onClick={() => {
            onSaveReference(selectionState.value.text, selectionState.value.pages[0] ?? page);
            clearSelection();
          }}>Save citation</button>
          <button
            type="button"
            className="is-primary"
            onClick={() => onClarifySelection(selectionState.value.text, selectionState.value.pages[0] ?? page)}
          >
            Ask about this
          </button>
          <button type="button" className="pdf-selection-close" onClick={clearSelection} aria-label="Clear selection">×</button>
        </div>
      )}
    </section>
  );
}
