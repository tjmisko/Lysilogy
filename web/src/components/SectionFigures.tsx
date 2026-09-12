import { useEffect, useMemo, useRef, useState } from "react";
import type { RenderTask } from "pdfjs-dist";
import type { LayoutPage, PaperSection } from "../types";
import { readingIndexUrl, type ReadingIndex } from "../lib/readingIndex";
import { externalSectionFigures } from "../lib/sectionFigures";
import { usePdfDocument } from "../hooks/usePdfDocument";

type Figure = ReadingIndex["figures"][number];
function FigurePage({ url, figure, onClose, onFullPaper, darkInk }: { url: string; figure: Figure; onClose: () => void; onFullPaper: (page: number) => void; darkInk: boolean }) {
  const { document, error } = usePdfDocument(url);
  const canvas = useRef<HTMLCanvasElement>(null);
  const host = useRef<HTMLDivElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  const [width, setWidth] = useState(600);
  const [full, setFull] = useState(figure.rect === null);
  const [renderError, setRenderError] = useState("");
  useEffect(() => {
    close.current?.focus();
    const node = host.current;
    if (node === null) return;
    const observer = new ResizeObserver(() => setWidth(node.clientWidth));
    observer.observe(node);
    const quit = (event: Event) => { event.preventDefault(); onClose(); };
    node.addEventListener("figure-quit", quit);
    return () => { observer.disconnect(); node.removeEventListener("figure-quit", quit); };
  }, [onClose]);
  useEffect(() => {
    if (document === null || canvas.current === null) return;
    let cancelled = false;
    let task: RenderTask | null = null;
    void document.getPage(figure.page).then(async (page) => {
      if (cancelled || canvas.current === null) return;
      const base = page.getViewport({ scale: 1 });
      const rect = full ? null : figure.rect;
      const box = rect ?? { x_min: 0, y_min: 0, x_max: base.width, y_max: base.height };
      const scale = Math.max(.25, Math.min(2, (width - 24) / (box.x_max - box.x_min)));
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const node = canvas.current;
      node.width = Math.ceil((box.x_max - box.x_min) * scale * ratio);
      node.height = Math.ceil((box.y_max - box.y_min) * scale * ratio);
      node.style.width = `${node.width / ratio}px`; node.style.height = `${node.height / ratio}px`;
      const context = node.getContext("2d");
      if (context === null) return;
      task = page.render({ canvas: node, canvasContext: context, viewport: page.getViewport({ scale }),
        transform: [ratio, 0, 0, ratio, -box.x_min * scale * ratio, -box.y_min * scale * ratio] });
      await task.promise;
    }).catch((reason: unknown) => { if (!cancelled) setRenderError(reason instanceof Error ? reason.message : "Could not render figure"); });
    return () => { cancelled = true; task?.cancel(); };
  }, [document, figure, full, width]);
  return <div className="section-figure-view" ref={host} role="dialog" aria-label={`${figure.label} source`} aria-modal="false">
    <header><strong>{figure.label} · page {figure.page}</strong><button type="button" onClick={() => setFull((value) => !value)} disabled={figure.rect === null}>{full ? "Figure region" : "Whole page"}</button>
      <button type="button" onClick={() => onFullPaper(figure.page)}>Open in paper ↗</button><button ref={close} type="button" onClick={onClose} aria-label="Close figure">×</button></header>
    <div className="section-figure-scroll"><canvas ref={canvas} className={darkInk ? "is-dark-figure" : ""} aria-label={`${figure.label} on source page ${figure.page}`} />
      {(error || renderError) && <p role="alert">{error || renderError}</p>}
      <p className="figure-caption">{figure.caption}</p>
      {!full && <button className="figure-page-fallback" type="button" onClick={() => setFull(true)}>Image boundary estimated · show whole page</button>}
    </div>
  </div>;
}

export function SectionFigures({ url, section, pages, darkInk, onFullPaper }: { url: string; section: PaperSection; pages: LayoutPage[]; darkInk: boolean; onFullPaper: (page: number) => void }) {
  const [index, setIndex] = useState<ReadingIndex | null>(null);
  const [selected, setSelected] = useState<Figure | null>(null);
  const [error, setError] = useState("");
  const trigger = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    void fetch(readingIndexUrl(url), { signal: controller.signal }).then(async (response) => {
      if (!response.ok) throw new Error("Figure references unavailable");
      const data = await response.json() as ReadingIndex;
      if (!Array.isArray(data.figures)) throw new Error("Figure references unavailable");
      setIndex(data);
    }).catch(() => { if (!controller.signal.aborted) setError("Figure references unavailable"); });
    return () => controller.abort();
  }, [url]);
  const figures = useMemo(() => index === null ? [] : externalSectionFigures(index, section, pages), [index, section, pages]);
  const close = () => { setSelected(null); trigger.current?.focus(); };
  if (figures.length === 0) return error ? <span className="section-figure-error" title="Open the full paper to inspect figures, or restart the backend if it was updated.">{error}</span> : null;
  return <>
    <nav className="section-figure-links" aria-label="Figures referenced outside this section"><span>Referenced figures</span>{figures.map((figure) => <button key={figure.id} type="button" onClick={(event) => { trigger.current = event.currentTarget; setSelected(figure); }}>{figure.label} ↗</button>)}</nav>
    {selected !== null && <FigurePage url={url} figure={selected} onClose={close} onFullPaper={onFullPaper} darkInk={darkInk} />}
  </>;
}
