import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  GlobalWorkerOptions,
  getDocument,
  type PDFDocumentProxy,
  type PageViewport,
  type RenderTask,
  type TextLayerImages,
} from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { TextLayerBuilder } from "pdfjs-dist/web/pdf_viewer.mjs";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

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
  return { page, itemIndex, offset: Math.max(0, Math.min(textLength, itemOffset)) };
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
}: PdfPageCanvasProps) {
  const surfaceRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const textLayerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const surface = surfaceRef.current;
    const canvas = canvasRef.current;
    const textLayerContainer = textLayerRef.current;
    if (surface === null || canvas === null || textLayerContainer === null) return;
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
        const fitScale = Math.max(0.2, (slotWidth - 28) / base.width);
        const viewport = pdfPage.getViewport({ scale: fitScale * zoom });
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
        const context = canvas.getContext("2d", { alpha: false });
        if (context === null) throw new Error("Canvas rendering is unavailable");

        surface.style.width = `${Math.floor(viewport.width)}px`;
        surface.style.height = `${Math.floor(viewport.height)}px`;
        surface.style.setProperty("--total-scale-factor", String(viewport.scale));
        canvas.width = Math.floor(viewport.width * pixelRatio);
        canvas.height = Math.floor(viewport.height * pixelRatio);
        canvas.style.width = `${Math.floor(viewport.width)}px`;
        canvas.style.height = `${Math.floor(viewport.height)}px`;

        renderTask = pdfPage.render({
          canvas,
          canvasContext: context,
          viewport,
          transform: pixelRatio === 1 ? undefined : [pixelRatio, 0, 0, pixelRatio, 0, 0],
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

        const textDivs = Array.from(textLayer.div.querySelectorAll<HTMLElement>("span"));
        textDivs.forEach((textDiv, index) => {
          textDiv.dataset.textIndex = String(index);
        });
        const hasText = textDivs.some((textDiv) => textDiv.textContent.trim().length > 0);
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
  }, [document, onError, onTextLayer, page, slotWidth, zoom]);

  return (
    <figure className="pdf-page-frame">
      <div className="pdf-page-surface" ref={surfaceRef}>
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
}: PdfReaderProps) {
  const [pdfDocument, setPdfDocument] = useState<PDFDocumentProxy | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [containerWidth, setContainerWidth] = useState(900);
  const [textPages, setTextPages] = useState<Record<number, boolean>>({});
  const [selectionState, setSelectionState] = useState<SelectionState | null>(null);
  const [copyLabel, setCopyLabel] = useState("Copy");
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
    let cancelled = false;
    viewportsRef.current.clear();
    window.queueMicrotask(() => {
      if (cancelled) return;
      setLoading(true);
      setError(null);
      setSelectionState(null);
      setTextPages({});
    });
    const task = getDocument({ url });
    void task.promise
      .then((loaded) => {
        if (cancelled) return;
        setPdfDocument(loaded);
        onPageCount(loaded.numPages);
        setLoading(false);
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setError(reason instanceof Error ? reason.message : "Could not load the PDF");
        setLoading(false);
      });
    return () => {
      cancelled = true;
      void task.destroy();
    };
  }, [onPageCount, url]);

  const pageCount = pdfDocument?.numPages ?? 1;
  const step = spread ? 2 : 1;
  const visiblePages = useMemo(
    () => spread && page < pageCount ? [page, page + 1] : [page],
    [page, pageCount, spread],
  );
  const slotWidth = spread ? Math.max(320, (containerWidth - 36) / 2) : containerWidth;
  const lastVisiblePage = visiblePages[visiblePages.length - 1] ?? page;
  const visibleTextStatus = visiblePages.map((visiblePage) => textPages[visiblePage]);
  const textLayersReady = visibleTextStatus.every((status) => status !== undefined);
  const hasSelectableText = visibleTextStatus.some(Boolean);

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
        <div>
          <span className="eyebrow">Source document</span>
          <strong>{title}</strong>
        </div>
        <div className="pdf-controls">
          <button type="button" onClick={() => onPage(Math.max(1, page - step))} disabled={page <= 1}>
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
          </button>
          <button type="button" className={darkInk ? "is-active" : ""} onClick={onToggleInk}>
            {darkInk ? "Dark" : "Light"} 
          </button>
        </div>
      </div>
      <div className={`pdf-viewport ${spread ? "is-spread" : ""}`} ref={containerRef}>
        {loading && <div className="reader-message"><span className="loader" /> Rendering source…</div>}
        {error !== null && <div className="reader-message error-message">{error}</div>}
        {pdfDocument !== null && visiblePages.map((visiblePage) => (
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
          />
        ))}
      </div>
      <div className={`pdf-mode-note ${textLayersReady && !hasSelectableText ? "is-warning" : ""}`}>
        {textLayersReady && !hasSelectableText
          ? "This page has no embedded text. It needs OCR before passages can be selected."
          : "Select any passage to copy it or ask for an explanation."}
        {" "}<kbd>Ctrl-d</kbd> / <kbd>PageDown</kbd> page forward; <kbd>Ctrl-u</kbd> / <kbd>PageUp</kbd> page back.
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
