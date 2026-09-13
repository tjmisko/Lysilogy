import { useEffect, useRef, useState } from "react";
import { resolveSourceMarks, type SourceMark } from "../lib/sourceGeometry";
import type { SectionCrop } from "../lib/sectionCrop";

export function PdfSourceMarks({ marks, page, width, height, crop, text }: { marks: SourceMark[]; page: number; width: number; height: number; crop?: SectionCrop; text: string }) {
  const overlay = useRef<HTMLDivElement>(null);
  const [resolved, setResolved] = useState<SourceMark[]>([]);
  useEffect(() => {
    const host = overlay.current?.parentElement?.querySelector<HTMLElement>(".pdf-text-layer-host") ?? null;
    let frame = 0;
    const update = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => setResolved(resolveSourceMarks(marks.filter((mark) => mark.page === page && mark.kind !== "line-number"), host, width, height, text)));
    };
    update();
    // PDF.js installs and scales its text layer asynchronously. Watch only the
    // source layer, so updating the highlight overlay cannot retrigger itself.
    const observer = new MutationObserver(update);
    const resize = new ResizeObserver(update);
    if (host !== null) {
      observer.observe(host, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["style", "data-text-ready"] });
      resize.observe(host);
    }
    return () => { cancelAnimationFrame(frame); observer.disconnect(); resize.disconnect(); };
  }, [marks, page, width, height, text, crop]);
  const bounds = crop?.bounds ?? { x_min: 0, y_min: 0, x_max: width, y_max: height };
  const regions = crop?.regions ?? [bounds];
  return <div ref={overlay} className="pdf-source-marks" aria-hidden="true">{resolved.flatMap((mark, index) => regions.map((region, part) => {
    const left = Math.max(mark.rect.x_min, region.x_min); const right = Math.min(mark.rect.x_max, region.x_max);
    const top = Math.max(mark.rect.y_min, region.y_min); const bottom = Math.min(mark.rect.y_max, region.y_max);
    if (right <= left || bottom <= top) return null;
    return <span key={`${index}:${part}`} data-source-offset={mark.start} data-source-end={mark.end} data-geometry={mark.geometry} className={`pdf-source-mark is-${mark.kind}`} style={{ left: `${100 * (left - bounds.x_min) / (bounds.x_max - bounds.x_min)}%`, top: `${100 * (top - bounds.y_min) / (bounds.y_max - bounds.y_min)}%`, width: `${100 * (right - left) / (bounds.x_max - bounds.x_min)}%`, height: `${100 * (bottom - top) / (bounds.y_max - bounds.y_min)}%` }} />;
  }))}</div>;
}


/** Numbers live outside the clipped PDF surface, so section margins cannot hide them. */
export function PdfSourceLineNumbers({ marks, page, width, height, crop }: { marks: SourceMark[]; page: number; width: number; height: number; crop?: SectionCrop }) {
  const overlay = useRef<HTMLDivElement>(null);
  const [positions, setPositions] = useState<{ mark: SourceMark; left: number; top: number }[]>([]);
  useEffect(() => {
    const frame = overlay.current?.parentElement;
    const surface = frame?.querySelector<HTMLElement>(".pdf-page-surface");
    const window = frame?.querySelector<HTMLElement>(".pdf-page-window");
    if (frame == null || surface == null || window == null) return;
    let scheduled = 0;
    const update = () => {
      cancelAnimationFrame(scheduled);
      scheduled = requestAnimationFrame(() => {
        const origin = frame.getBoundingClientRect(), box = surface.getBoundingClientRect(), clip = window.getBoundingClientRect();
        const bounds = crop?.bounds ?? {x_min:0, y_min:0, x_max:width, y_max:height};
        const sx = box.width / (bounds.x_max-bounds.x_min), sy = box.height / (bounds.y_max-bounds.y_min);
        const regions = crop?.regions ?? [bounds];
        setPositions(marks.filter(mark => mark.page === page && mark.kind === "line-number").flatMap(mark => {
          const region = regions.find(region => mark.rect.x_max > region.x_min && mark.rect.x_min < region.x_max && mark.rect.y_max > region.y_min && mark.rect.y_min < region.y_max);
          if (region === undefined) return [];
          const left = Math.max(clip.left, box.left + (Math.max(region.x_min,mark.rect.x_min)-bounds.x_min)*sx);
          const top = Math.max(clip.top, box.top + (Math.max(region.y_min,mark.rect.y_min)-bounds.y_min)*sy);
          const bottom = box.top + (Math.min(region.y_max,mark.rect.y_max)-bounds.y_min)*sy;
          if (left >= clip.right || top >= clip.bottom || bottom <= clip.top || box.width === 0) return [];
          return [{mark,left:left-origin.left,top:top-origin.top}];
        }));
      });
    };
    update();
    const resize = new ResizeObserver(update); resize.observe(surface); resize.observe(window);
    const observer = new MutationObserver(update);
    observer.observe(surface, {attributes:true,attributeFilter:["style"]});
    return () => {cancelAnimationFrame(scheduled);resize.disconnect();observer.disconnect();};
  }, [marks,page,width,height,crop]);
  return <div ref={overlay} className="pdf-source-line-numbers" aria-hidden="true">{positions.map(({mark,left,top}) => <span key={mark.group} className={`pdf-source-line-number${mark.active ? " is-active" : ""}`} data-line-number={mark.group} style={{left,top}}>{mark.label}</span>)}</div>;
}
