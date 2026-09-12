import { flushSync } from "react-dom";
import { PdfSourceMarks } from "./PdfSourceMarks";
import { usePdfSourceTools, type SourceMark } from "./PdfSourceTools";
import { sectionPageCrop, type SectionCrop } from "../lib/sectionCrop";
import { cropTextLayer } from "../lib/cropTextLayer";
import { visiblePdfPage } from "../lib/pdfViewport";
import { handleNotesPaneKey } from "../lib/notesPaneKeys";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
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
  keyboardEnabled?: boolean;
  toolbarVisible?: boolean;
};

type PdfPageCanvasProps = {
  document: PDFDocumentProxy;
  page: number;
  slotWidth: number;
  slotHeight: number;
  fit: "width" | "height";
  zoom: number;
  darkInk: boolean;
  pageCount: number;
  onError: (message: string) => void;
  onTextLayer: (page: number, viewport: PageViewport | null, hasText: boolean) => void;
  lazy?: boolean;
  layout?: LayoutPage;
  crop?: SectionCrop;
  readingWidth?: number;
  readingHeight?: number;
  sourceMarks: SourceMark[];
  sourceText: string;
  sourceDimensions?: { width: number; height: number };
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
  slotHeight,
  fit,
  zoom,
  darkInk,
  pageCount,
  onError,
  onTextLayer,
  lazy = false,
  layout,
  crop,
  readingWidth,
  readingHeight,
  sourceMarks,
  sourceText,
  sourceDimensions,
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
    }, { root: frameRef.current.closest(".pdf-viewport"), rootMargin: "700px" });
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
        surface.dataset.pdfWidth = String(layout?.width ?? base.width);
        surface.dataset.pdfHeight = String(layout?.height ?? base.height);
        const fitScale = Math.max(0.1, fit === "height"
          ? (slotHeight - 40) / (crop === undefined ? base.height : readingHeight ?? base.height)
          : (slotWidth - 28) / (crop === undefined ? base.width : readingWidth ?? base.width));
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
  }, [crop, document, fit, layout, onError, onTextLayer, page, readingHeight, readingWidth, slotHeight, slotWidth, visible, zoom]);

  const placeholderScale = Math.max(.1, fit === "height" ? (slotHeight - 40) / (readingHeight ?? layout?.height ?? 792)
    : (slotWidth - 28) / (readingWidth ?? layout?.width ?? 612)) * zoom;

  return (
    <figure className="pdf-page-frame" ref={frameRef} data-pdf-page={page} data-section-crop={crop === undefined ? undefined : "true"}
      data-page-crop={crop === undefined || layout === undefined ? undefined : JSON.stringify({
        x: crop.bounds.x_min / layout.width, y: crop.bounds.y_min / layout.height,
        width: (crop.bounds.x_max - crop.bounds.x_min) / layout.width, height: (crop.bounds.y_max - crop.bounds.y_min) / layout.height,
      })}>
      <div className="pdf-page-surface" ref={surfaceRef} style={lazy ? {
        width: (crop === undefined ? layout?.width ?? 612 : crop.bounds.x_max - crop.bounds.x_min) * placeholderScale,
        aspectRatio: crop === undefined ? `${layout?.width ?? 612} / ${layout?.height ?? 792}`
          : `${crop.bounds.x_max - crop.bounds.x_min} / ${crop.bounds.y_max - crop.bounds.y_min}`,
      } : undefined}>
        <canvas
          ref={canvasRef}
          className={darkInk ? "pdf-canvas dark-ink" : "pdf-canvas"}
          aria-label={`Page ${page} of ${pageCount}`}
        />
        <PdfSourceMarks marks={sourceMarks} text={sourceText} page={page} width={sourceDimensions?.width ?? layout?.width ?? 612} height={sourceDimensions?.height ?? layout?.height ?? 792} crop={crop} />
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
  keyboardEnabled = true,
  toolbarVisible = true,
}: PdfReaderProps) {
  const { document: pdfDocument, loading, error: loadError } = usePdfDocument(url);
  const [error, setError] = useState<string | null>(null);
  const [containerWidth, setContainerWidth] = useState(900);
  const [containerHeight, setContainerHeight] = useState(650);
  const [fit, setFit] = useState<"width" | "height">("height");
  const [flow, setFlow] = useState<"paged" | "continuous">(pageSubset === undefined ? "paged" : "continuous");
  const [axis, setAxis] = useState<"vertical" | "horizontal">("vertical");
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
    const update = (): void => { setContainerWidth(node.clientWidth); setContainerHeight(node.clientHeight); };
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
  const readingHeight = Math.max(1, ...Array.from(crops.values()).flatMap((crop) => crop === null ? [] : [crop.bounds.y_max - crop.bounds.y_min]));
  const step = flow === "paged" && spread && pageSubset === undefined ? 2 : 1;
  const availablePages = useMemo(() => pageSubset !== undefined
      ? [...new Set(pageSubset)].filter((value) => Number.isInteger(value) && value >= 1 && value <= pageCount).sort((a, b) => a - b)
      : Array.from({ length: pageCount }, (_, index) => index + 1), [pageCount, pageSubset]);
  const visiblePages = flow === "continuous" ? availablePages : step === 2 && page < pageCount ? [page, page + 1] : [page];
  const slotWidth = step === 2 ? Math.max(160, (containerWidth - 36) / 2) : containerWidth;
  const lastVisiblePage = visiblePages[visiblePages.length - 1] ?? page;
  const sourceTools = usePdfSourceTools({ url, page, root: readerRef, pageSubset, crops, markPages: flow === "paged" ? visiblePages : [readingPage - 1, readingPage, readingPage + 1, page], prefetchReady: pdfDocument !== null, onPage, onOpenFullPaper, onClarify: onClarifySelection, onSave: onSaveReference });
  const sourceKey = sourceTools.onKey;
  const sourceQuit = sourceTools.quit;

  useEffect(() => {
    if (flow !== "continuous") return;
    const host = containerRef.current;
    if (host == null) return;
    const update = () => {
      const visible = visiblePdfPage(host);
      if (visible !== null) setReadingPage(visible);
    };
    host.addEventListener("scroll", update, { passive: true });
    return () => host.removeEventListener("scroll", update);
  }, [flow]);

  useEffect(() => {
    if (flow !== "continuous" || pdfDocument === null) return;
    const frame = window.requestAnimationFrame(() => {
      const host = containerRef.current;
      const target = readerRef.current?.querySelector<HTMLElement>(`[data-pdf-page="${page}"]`);
      if (host == null || target == null) return;
      if (axis === "vertical") host.scrollTop += target.getBoundingClientRect().top - host.getBoundingClientRect().top - 12;
      else host.scrollLeft += target.getBoundingClientRect().left - host.getBoundingClientRect().left - 12;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [axis, flow, page, pageJump, pdfDocument]);

  const changeFit = useCallback((next: "width" | "height") => { setFit(next); onZoom?.(1 - zoom); }, [onZoom, zoom]);
  const changeFlow = useCallback(() => { onPage(flow === "continuous" ? readingPage : page); setFlow((value) => value === "paged" ? "continuous" : "paged"); }, [flow, onPage, page, readingPage]);
  const changeAxis = useCallback(() => { onPage(flow === "continuous" ? readingPage : page); setFlow("continuous"); setAxis((value) => value === "vertical" ? "horizontal" : "vertical"); }, [flow, onPage, page, readingPage]);

  useEffect(() => {
    const host = containerRef.current;
    if (host === null || flow !== "continuous" || axis !== "horizontal") return;
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.deltaX !== 0 || event.deltaY === 0) return;
      event.preventDefault();
      host.scrollLeft += event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? host.clientWidth : 1);
    };
    host.addEventListener("wheel", onWheel, { passive: false });
    return () => host.removeEventListener("wheel", onWheel);
  }, [axis, flow]);

  useLayoutEffect(() => {
    if (!keyboardEnabled) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || handleNotesPaneKey(event)) return;
      if (window.document.querySelector(".section-figure-view") !== null || event.target instanceof Element && event.target.closest(".notes-panel") !== null) return;
      const sourceHandled = event.key === "q" || event.key === "Escape" ? flushSync(() => sourceKey(event)) : sourceKey(event);
      if (sourceHandled) { event.preventDefault(); event.stopImmediatePropagation(); return; }
      const target = event.target;
      if (event.metaKey || event.altKey || target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
        || target instanceof HTMLSelectElement || target instanceof HTMLElement && target.isContentEditable) return;
      const host = containerRef.current;
      let action: (() => void) | undefined;
      if (!event.ctrlKey) {
        if (event.key === "W" || event.key === "w") action = () => changeFit("width");
        if (event.key === "H") action = () => changeFit("height");
        if (event.key === "P") action = changeFlow;
        if (event.key === "R") action = changeAxis;
      }
      if (host !== null) {
        const direction = ["k", "ArrowUp", "ArrowLeft", "PageUp", "u"].includes(event.key) ? -1 : 1;
        let distance = 0;
        if (!event.ctrlKey && ["j", "k", "ArrowDown", "ArrowUp"].includes(event.key)) distance = 100;
        if (["PageDown", "PageUp"].includes(event.key) || event.ctrlKey && ["d", "u"].includes(event.key)) distance = (axis === "vertical" ? host.clientHeight : host.clientWidth) * (event.ctrlKey ? .5 : .9);
        if (distance !== 0) action = () => host.scrollBy(flow === "paged" || axis === "vertical" ? { top: direction * distance } : { left: direction * distance });
      }
      const turnPage = !event.ctrlKey && ["h", "l", "ArrowLeft", "ArrowRight"].includes(event.key)
        || flow === "paged" && (["PageUp", "PageDown"].includes(event.key) || event.ctrlKey && ["d", "u"].includes(event.key));
      if (turnPage) action = () => {
        const current = flow === "continuous" && host !== null ? visiblePdfPage(host) ?? page : page;
        const direction = ["h", "ArrowLeft", "PageUp", "u"].includes(event.key) ? -1 : 1;
        onPage(availablePages[Math.max(0, Math.min(availablePages.length - 1, availablePages.indexOf(current) + direction * step))] ?? page);
      };
      if (action !== undefined) { event.preventDefault(); event.stopImmediatePropagation(); action(); }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [availablePages, axis, changeAxis, changeFit, changeFlow, flow, keyboardEnabled, onPage, page, sourceKey, step]);

  const handleTextLayer = useCallback(
    (pageNumber: number, viewport: PageViewport | null): void => {
      if (viewport === null) viewportsRef.current.delete(pageNumber);
      else viewportsRef.current.set(pageNumber, viewport);
    }, [],
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

  useLayoutEffect(() => {
    const node = readerRef.current;
    if (node === null) return;
    const quit = (event: Event) => {
      const closed = flushSync(sourceQuit);
      if (closed) { event.preventDefault(); return; }
      if (selectionState !== null) { clearSelection(); event.preventDefault(); }
    };
    node.addEventListener("source-quit", quit);
    return () => node.removeEventListener("source-quit", quit);
  }, [clearSelection, selectionState, sourceQuit]);

  return (
    <section ref={readerRef} className="pdf-reader" tabIndex={-1} aria-label={`PDF: ${title}`} data-flow={flow} data-axis={axis} data-fit={fit}
      data-source-local-mode={sourceTools.localMode || selectionState !== null ? "true" : undefined}
      data-source-visual={keyboardEnabled && sourceTools.visualMode ? "true" : undefined}
      onPointerUp={(event) => {
        let surface = event.target instanceof Element ? event.target.closest<HTMLElement>(".pdf-page-surface") : null;
        const selection = window.getSelection();
        const range = selection !== null && !selection.isCollapsed && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
        // A drag may end on another page; use the source page where the selection starts.
        if (range !== null) {
          const start = range.startContainer instanceof Element ? range.startContainer : range.startContainer.parentElement;
          const selectedSurface = start?.closest<HTMLElement>(".pdf-page-surface");
          if (selectedSurface != null && readerRef.current?.contains(selectedSurface)) surface = selectedSurface;
        }
        const frame = surface?.closest<HTMLElement>("[data-pdf-page]");
        if (surface === null || frame === null || frame === undefined) return;
        const pageNumber = Number(frame.dataset.pdfPage);
        const dimensions = sourceTools.index?.pages.find((item) => item.number === pageNumber) ?? pageLayouts?.find((item) => item.number === pageNumber);
        const crop = crops.get(pageNumber);
        const bounds = surface.getBoundingClientRect();
        const selectedRect = range?.getClientRects()[0];
        const x = (selectedRect?.left ?? event.clientX) - bounds.left;
        const y = (selectedRect?.top ?? event.clientY) - bounds.top;
        sourceTools.pointerCursor(pageNumber, (crop?.bounds.x_min ?? 0) + x / bounds.width * (crop === null || crop === undefined ? dimensions?.width ?? Number(surface.dataset.pdfWidth ?? 612) : crop.bounds.x_max - crop.bounds.x_min),
          (crop?.bounds.y_min ?? 0) + y / bounds.height * (crop === null || crop === undefined ? dimensions?.height ?? Number(surface.dataset.pdfHeight ?? 792) : crop.bounds.y_max - crop.bounds.y_min));
      }}>
      <div className="pdf-toolbar" hidden={!toolbarVisible}>
        {pageSubset === undefined ? <div>
          <span className="eyebrow">Source document</span>
          <strong>{title}</strong>
        </div> : <button className="return-to-map" type="button" onClick={onReturnToMap}>← Map</button>}
        <div className="pdf-controls">
          {pageSubset === undefined && flow === "paged" && <><button type="button" onClick={() => onPage(Math.max(1, page - step))} disabled={page <= 1}>
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
          {(pageSubset !== undefined || flow === "continuous") && <>
            <select aria-label="Source page" value={flow === "continuous" && availablePages.includes(readingPage) ? readingPage : page} onChange={(event) => {
              const next = Number(event.target.value); setReadingPage(next); onPage(next);
            }}>
              {availablePages.map((value) => <option key={value} value={value}>Page {value} / {pageCount}</option>)}
            </select>
            <button type="button" aria-label="Zoom out source pages" disabled={zoom <= .6} onClick={() => onZoom?.(-.1)}>−</button>
            <button type="button" aria-label="Zoom in source pages" disabled={zoom >= 2} onClick={() => onZoom?.(.1)}>+</button>
            {pageSubset !== undefined && <button type="button" onClick={() => {
              onOpenFullPaper?.(flow === "continuous" ? readingPage : page);
            }}>Open full paper ↗</button>}
          </>}
          <button type="button" aria-label="Fit width" title="Fit width (W)" className={fit === "width" ? "is-active" : ""} onClick={() => changeFit("width")}>W</button>
          <button type="button" aria-label="Fit height" title="Fit height (H)" className={fit === "height" ? "is-active" : ""} onClick={() => changeFit("height")}>H</button>
          <button type="button" aria-label="Toggle paged or continuous reading" title="Paged / continuous (P)" onClick={changeFlow}>{flow === "paged" ? "Paged" : "Continuous"}</button>
          <button type="button" aria-label="Toggle scroll direction" title="Rotate scrolling: vertical / horizontal (R)" onClick={changeAxis}>{axis === "vertical" ? "Vertical" : "Horizontal"}</button>
          <button type="button" className={darkInk ? "is-active" : ""} onClick={onToggleInk}>
            {darkInk ? "Dark" : "Light"}
          </button>
        </div>
      </div>
      <div className={`pdf-viewport ${step === 2 ? "is-spread" : ""}`} ref={containerRef} tabIndex={0} aria-label="PDF pages">
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
            slotHeight={containerHeight}
            fit={fit}
            zoom={zoom}
            darkInk={darkInk}
            pageCount={pageCount}
            onError={handleRenderError}
            onTextLayer={handleTextLayer}
            lazy={flow === "continuous"}
            layout={pageLayouts?.find((item) => item.number === visiblePage)}
            crop={pageSubset === undefined ? undefined : crops.get(visiblePage) ?? undefined}
            readingWidth={pageSubset === undefined ? undefined : readingWidth}
            readingHeight={pageSubset === undefined ? undefined : readingHeight}
            sourceMarks={sourceTools.marks}
            sourceText={sourceTools.index?.text ?? ""}
            sourceDimensions={sourceTools.index?.pages.find((item) => item.number === visiblePage)}
          />
        ))}
      </div>
      {sourceTools.panel}
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
            onClick={() => { onClarifySelection(selectionState.value.text, selectionState.value.pages[0] ?? page); clearSelection(); }}
          >
            Ask about this
          </button>
          <button type="button" className="pdf-selection-close" onClick={clearSelection} aria-label="Clear selection">×</button>
        </div>
      )}
    </section>
  );
}
