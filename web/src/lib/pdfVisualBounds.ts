import type { PDFPageProxy } from "pdfjs-dist";
import type { SectionCrop } from "./sectionCrop";
import type { TextRect } from "../types";

/** Bounds of visible ink, in image pixels. White/transparent pages have no bounds. */
export function inkBounds({ data, width, height }: Pick<ImageData, "data" | "width" | "height">): TextRect | null {
  let left = width, top = height, right = -1, bottom = -1;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const offset = (y * width + x) * 4;
      const alpha = (data[offset + 3] ?? 0) / 255;
      const darkest = Math.min(data[offset] ?? 255, data[offset + 1] ?? 255, data[offset + 2] ?? 255);
      if ((255 - darkest) * alpha < 16) continue;
      left = Math.min(left, x); right = Math.max(right, x);
      top = Math.min(top, y); bottom = Math.max(bottom, y);
    }
  }
  return right < left ? null : { x_min: left, y_min: top, x_max: right + 1, y_max: bottom + 1 };
}

const cache = new WeakMap<PDFPageProxy, Map<string, Promise<TextRect | null>>>();

/** Normalized viewport bounds, including a small paper margin around text and figures.
 * Measuring the rendered page also handles scans, rotation, and PDFs without text.
 */
export function pdfVisualBounds(page: PDFPageProxy, crop?: SectionCrop, dimensions?: { width: number; height: number }): Promise<TextRect | null> {
  const base = page.getViewport({ scale: 1 });
  const width = dimensions?.width ?? base.width, height = dimensions?.height ?? base.height;
  const regions = crop?.regions.map((r) => ({ x_min: r.x_min / width, x_max: r.x_max / width,
    y_min: r.y_min / height, y_max: r.y_max / height }));
  const key = JSON.stringify(regions ?? null);
  let entries = cache.get(page);
  if (entries === undefined) { entries = new Map(); cache.set(page, entries); }
  const existing = entries.get(key);
  if (existing !== undefined) return existing;
  const measure = async (): Promise<TextRect | null> => {
    const viewport = page.getViewport({ scale: Math.min(1.5, 1000 / Math.max(base.width, base.height)) });
    const canvas = document.createElement("canvas");
    canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
    const context = canvas.getContext("2d", { alpha: false, willReadFrequently: true });
    if (context === null) return null;
    try {
      context.fillStyle = "white"; context.fillRect(0, 0, canvas.width, canvas.height);
      if (regions !== undefined) {
        context.beginPath();
        for (const r of regions) context.rect(r.x_min * viewport.width, r.y_min * viewport.height,
          (r.x_max - r.x_min) * viewport.width, (r.y_max - r.y_min) * viewport.height);
        context.clip();
      }
      await page.render({ canvas, canvasContext: context, viewport }).promise;
      const bounds = inkBounds(context.getImageData(0, 0, canvas.width, canvas.height));
      if (bounds === null) return null;
      // Eight PDF points of breathing room, plus one sample pixel for antialiasing.
      const marginX = 8 / base.width + 1 / viewport.width, marginY = 8 / base.height + 1 / viewport.height;
      return { x_min: Math.max(0, bounds.x_min / viewport.width - marginX),
        y_min: Math.max(0, bounds.y_min / viewport.height - marginY),
        x_max: Math.min(1, bounds.x_max / viewport.width + marginX),
        y_max: Math.min(1, bounds.y_max / viewport.height + marginY) };
    } finally {
      canvas.width = 0; canvas.height = 0;
    }
  };
  // Fitting should still work if a preview cannot be measured; the normal render
  // remains responsible for reporting PDF errors. Allow a later attempt to retry.
  const pending = measure().catch(() => { entries.delete(key); return null; });
  entries.set(key, pending);
  return pending;
}
