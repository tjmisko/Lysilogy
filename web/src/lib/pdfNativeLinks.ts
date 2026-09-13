import type { PDFDocumentProxy } from "pdfjs-dist";
import type { ReadingIndex } from "./readingIndex";
import type { PaperLink } from "./paperLinks";

export function safeLinkUrl(value: unknown): string | null {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    return ["https:", "http:", "mailto:"].includes(url.protocol) ? url.href : null;
  } catch { return null; }
}

/** Respect the PDF's own destinations, even when its citation style is unfamiliar. */
export async function nativePageLinks(document: PDFDocumentProxy, number: number, index: ReadingIndex | null): Promise<PaperLink[]> {
  const page = await document.getPage(number);
  const viewport = page.getViewport({ scale: 1 });
  const dimensions = index?.pages.find((item) => item.number === number);
  const sx = (dimensions?.width ?? viewport.width) / viewport.width;
  const sy = (dimensions?.height ?? viewport.height) / viewport.height;
  const annotations: unknown[] = await page.getAnnotations({ intent: "display" });
  const results = await Promise.allSettled(annotations.map(async (value, at): Promise<PaperLink | null> => {
    if (typeof value !== "object" || value === null) return null;
    const annotation = value as { subtype?: string; rect?: unknown[]; dest?: unknown; url?: unknown; contentsObj?: { str?: string } };
    if (annotation.subtype !== "Link" || annotation.rect?.length !== 4 || !annotation.rect.every((item) => typeof item === "number" && Number.isFinite(item))) return null;
    const box = viewport.convertToViewportRectangle(annotation.rect as number[]) as number[];
    const [x1 = 0, y1 = 0, x2 = 0, y2 = 0] = box;
    const rects = [{ x_min: Math.min(x1, x2) * sx, x_max: Math.max(x1, x2) * sx,
      y_min: Math.min(y1, y2) * sy, y_max: Math.max(y1, y2) * sy }];
    const base = { id: `native:${number}:${at}`, kind: "link" as const, page: number, rects };
    const url = safeLinkUrl(annotation.url);
    if (url !== null) return { ...base, label: annotation.contentsObj?.str || url, destination: { url } };
    const dest: unknown = typeof annotation.dest === "string" ? await document.getDestination(annotation.dest) : annotation.dest;
    if (!Array.isArray(dest) || dest.length < 2) return null;
    const ref: unknown = dest[0];
    let pageIndex: number;
    if (typeof ref === "number" && Number.isInteger(ref)) pageIndex = ref;
    else if (typeof ref === "object" && ref !== null && "num" in ref && "gen" in ref && typeof ref.num === "number" && typeof ref.gen === "number") pageIndex = await document.getPageIndex({ num: ref.num, gen: ref.gen });
    else return null;
    if (pageIndex < 0 || pageIndex >= document.numPages) return null;
    const target = await document.getPage(pageIndex + 1);
    const targetViewport = target.getViewport({ scale: 1 });
    const targetDimensions = index?.pages.find((item) => item.number === pageIndex + 1);
    const width = targetDimensions?.width ?? targetViewport.width, height = targetDimensions?.height ?? targetViewport.height;
    const fit: unknown = dest[1];
    const name = typeof fit === "object" && fit !== null && "name" in fit ? fit.name : "";
    const x: unknown = name === "XYZ" || name === "FitR" ? dest[2] : null;
    const y: unknown = name === "XYZ" ? dest[3] : name === "FitR" ? dest[5] : name === "FitH" || name === "FitBH" ? dest[2] : null;
    const point = targetViewport.convertToViewportPoint(typeof x === "number" ? x : target.view[0] ?? 0, typeof y === "number" ? y : target.view[3] ?? height);
    const left = Math.max(0, Math.min(width - 1, (point[0] ?? 0) * width / targetViewport.width));
    const top = Math.max(0, Math.min(height - 1, (point[1] ?? 0) * height / targetViewport.height));
    const label = annotation.contentsObj?.str || `Page ${pageIndex + 1}`;
    return { ...base, label, destination: { page: pageIndex + 1, label,
      rect: { x_min: left, x_max: Math.min(width, left + width * .35), y_min: top, y_max: Math.min(height, top + 18) } } };
  }));
  return results.flatMap((result) => result.status === "fulfilled" && result.value !== null ? [result.value] : []);
}
