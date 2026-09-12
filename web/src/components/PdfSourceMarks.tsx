import type { SourceMark } from "./PdfSourceTools";
import type { SectionCrop } from "../lib/sectionCrop";

export function PdfSourceMarks({ marks, page, width, height, crop }: { marks: SourceMark[]; page: number; width: number; height: number; crop?: SectionCrop }) {
  const bounds = crop?.bounds ?? { x_min: 0, y_min: 0, x_max: width, y_max: height };
  const regions = crop?.regions ?? [bounds];
  return <div className="pdf-source-marks" aria-hidden="true">{marks.filter((mark) => mark.page === page).flatMap((mark, index) => regions.map((region, part) => {
    const left = Math.max(mark.rect.x_min, region.x_min); const right = Math.min(mark.rect.x_max, region.x_max);
    const top = Math.max(mark.rect.y_min, region.y_min); const bottom = Math.min(mark.rect.y_max, region.y_max);
    if (right <= left || bottom <= top) return null;
    return <span key={`${index}:${part}`} data-source-offset={mark.start} className={`pdf-source-mark is-${mark.kind}`} style={{ left: `${100 * (left - bounds.x_min) / (bounds.x_max - bounds.x_min)}%`, top: `${100 * (top - bounds.y_min) / (bounds.y_max - bounds.y_min)}%`, width: `${100 * (right - left) / (bounds.x_max - bounds.x_min)}%`, height: `${100 * (bottom - top) / (bounds.y_max - bounds.y_min)}%` }} />;
  }))}</div>;
}
