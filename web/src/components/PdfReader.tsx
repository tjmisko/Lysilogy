import { useEffect, useRef, useState } from "react";
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
  onPage: (page: number) => void;
  onPageCount: (count: number) => void;
  onToggleInk: () => void;
};

export function PdfReader({
  url,
  title,
  page,
  zoom,
  darkInk,
  onPage,
  onPageCount,
  onToggleInk,
}: PdfReaderProps) {
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [containerWidth, setContainerWidth] = useState(900);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

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

  useEffect(() => {
    const canvas = canvasRef.current;
    if (document === null || canvas === null) return;
    let cancelled = false;
    let renderTask: RenderTask | null = null;
    void document
      .getPage(page)
      .then((pdfPage) => {
        if (cancelled) return;
        const base = pdfPage.getViewport({ scale: 1 });
        const fitScale = Math.max(0.2, (containerWidth - 56) / base.width);
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
          setError(reason.message);
        }
      });
    return () => {
      cancelled = true;
      renderTask?.cancel();
    };
  }, [containerWidth, document, page, zoom]);

  const pageCount = document?.numPages ?? 1;

  return (
    <section className="pdf-reader" aria-label={`PDF: ${title}`}>
      <div className="pdf-toolbar">
        <div>
          <span className="eyebrow">Source document</span>
          <strong>{title}</strong>
        </div>
        <div className="pdf-controls">
          <button type="button" onClick={() => onPage(Math.max(1, page - 1))} disabled={page <= 1}>
            ← <span>Prev</span>
          </button>
          <span>
            {page} / {pageCount}
          </span>
          <button
            type="button"
            onClick={() => onPage(Math.min(pageCount, page + 1))}
            disabled={page >= pageCount}
          >
            <span>Next</span> →
          </button>
          <button type="button" className={darkInk ? "is-active" : ""} onClick={onToggleInk}>
            {darkInk ? "Dark ink" : "True colour"} <kbd>I</kbd>
          </button>
        </div>
      </div>
      <div className="pdf-viewport" ref={containerRef}>
        {loading && <div className="reader-message"><span className="loader" /> Rendering source…</div>}
        {error !== null && <div className="reader-message error-message">{error}</div>}
        <canvas
          ref={canvasRef}
          className={darkInk ? "pdf-canvas dark-ink" : "pdf-canvas"}
          aria-label={`Page ${page} of ${pageCount}`}
        />
      </div>
      <div className="pdf-mode-note">
        Dark ink is the default reading transform. Press <kbd>I</kbd> when a figure needs its
        original colours.
      </div>
    </section>
  );
}
