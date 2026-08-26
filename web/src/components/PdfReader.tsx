import { useCallback, useEffect, useRef, useState } from "react";
import {
  GlobalWorkerOptions,
  getDocument,
  type PDFDocumentProxy,
  type RenderTask,
} from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

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
};

type PdfPageCanvasProps = {
  document: PDFDocumentProxy;
  page: number;
  slotWidth: number;
  zoom: number;
  darkInk: boolean;
  pageCount: number;
  onError: (message: string) => void;
};

function PdfPageCanvas({
  document,
  page,
  slotWidth,
  zoom,
  darkInk,
  pageCount,
  onError,
}: PdfPageCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) return;
    let cancelled = false;
    let renderTask: RenderTask | null = null;
    void document
      .getPage(page)
      .then((pdfPage) => {
        if (cancelled) return;
        const base = pdfPage.getViewport({ scale: 1 });
        const fitScale = Math.max(0.2, (slotWidth - 28) / base.width);
        const viewport = pdfPage.getViewport({ scale: fitScale * zoom });
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
        const context = canvas.getContext("2d", { alpha: false });
        if (context === null) throw new Error("Canvas rendering is unavailable");
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
        return renderTask.promise;
      })
      .catch((reason: unknown) => {
        if (!cancelled && reason instanceof Error && reason.name !== "RenderingCancelledException") {
          onError(reason.message);
        }
      });
    return () => {
      cancelled = true;
      renderTask?.cancel();
    };
  }, [document, onError, page, slotWidth, zoom]);

  return (
    <figure className="pdf-page-frame">
      <canvas
        ref={canvasRef}
        className={darkInk ? "pdf-canvas dark-ink" : "pdf-canvas"}
        aria-label={`Page ${page} of ${pageCount}`}
      />
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
}: PdfReaderProps) {
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [containerWidth, setContainerWidth] = useState(900);
  const containerRef = useRef<HTMLDivElement>(null);
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
    const task = getDocument({ url });
    void task.promise
      .then((loaded) => {
        if (cancelled) return;
        setDocument(loaded);
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

  const pageCount = document?.numPages ?? 1;
  const step = spread ? 2 : 1;
  const visiblePages = spread && page < pageCount ? [page, page + 1] : [page];
  const slotWidth = spread ? Math.max(320, (containerWidth - 36) / 2) : containerWidth;
  const lastVisiblePage = visiblePages[visiblePages.length - 1] ?? page;

  return (
    <section className="pdf-reader" aria-label={`PDF: ${title}`}>
      <div className="pdf-toolbar">
        <div>
          <span className="eyebrow">Source document</span>
          <strong>{title}</strong>
        </div>
        <div className="pdf-controls">
          <button type="button" onClick={() => onPage(Math.max(1, page - step))} disabled={page <= 1}>
            ← <span>Prev</span>
          </button>
          <span>
            {spread && lastVisiblePage !== page ? `${page}–${lastVisiblePage}` : page} / {pageCount}
          </span>
          <button
            type="button"
            onClick={() => onPage(Math.min(pageCount, page + step))}
            disabled={lastVisiblePage >= pageCount}
          >
            <span>Next</span> →
          </button>
          <button type="button" className={spread ? "is-active" : ""} onClick={onToggleSpread}>
            {spread ? "Two pages" : "One page"} <kbd>2</kbd>
          </button>
          <button type="button" className={darkInk ? "is-active" : ""} onClick={onToggleInk}>
            {darkInk ? "Dark ink" : "True colour"} <kbd>I</kbd>
          </button>
        </div>
      </div>
      <div className={`pdf-viewport ${spread ? "is-spread" : ""}`} ref={containerRef}>
        {loading && <div className="reader-message"><span className="loader" /> Rendering source…</div>}
        {error !== null && <div className="reader-message error-message">{error}</div>}
        {document !== null && visiblePages.map((visiblePage) => (
          <PdfPageCanvas
            key={visiblePage}
            document={document}
            page={visiblePage}
            slotWidth={slotWidth}
            zoom={zoom}
            darkInk={darkInk}
            pageCount={pageCount}
            onError={handleRenderError}
          />
        ))}
      </div>
      <div className="pdf-mode-note">
        <kbd>Ctrl-d</kbd> / <kbd>PageDown</kbd> page forward; <kbd>Ctrl-u</kbd> / <kbd>PageUp</kbd> page back.
        Dark ink is the default; press <kbd>I</kbd> for original figure colours.
      </div>
    </section>
  );
}
