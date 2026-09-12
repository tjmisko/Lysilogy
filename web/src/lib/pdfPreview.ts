import { getDocument } from "pdfjs-dist";
import "./pdfWorker";

const previews = new Map<string, string>();
const pending: Array<() => void> = [];
let rendering = 0;

export function cachedPdfPreview(url: string): string | undefined {
  return previews.get(url);
}

/** Small first-page images survive navigation; full PDF documents do not. */
export async function loadPdfPreview(url: string, signal: AbortSignal): Promise<string> {
  const cached = previews.get(url);
  if (cached !== undefined) return cached;
  await new Promise<void>((resolve, reject) => {
    const start = () => {
      signal.removeEventListener("abort", cancel);
      rendering += 1;
      resolve();
    };
    const cancel = () => {
      const index = pending.indexOf(start);
      if (index >= 0) pending.splice(index, 1);
      reject(new DOMException("Preview cancelled", "AbortError"));
    };
    if (signal.aborted) { cancel(); return; }
    if (rendering < 2) start();
    else { pending.push(start); signal.addEventListener("abort", cancel, { once: true }); }
  });
  try {
    signal.throwIfAborted();
    // Request only the ranges needed for page one where the source supports ranges.
    const task = getDocument({ url, disableAutoFetch: true, disableStream: true });
    let rejectCancellation: (error: DOMException) => void = () => {};
    const cancelled = new Promise<never>((_, reject) => { rejectCancellation = reject; });
    const cancel = () => {
      // Destroying PDF.js during worker startup need not settle task.promise.
      // Release our queue slot explicitly when filtering removes a preview.
      rejectCancellation(new DOMException("Preview cancelled", "AbortError"));
      void task.destroy().catch(() => {});
    };
    signal.addEventListener("abort", cancel, { once: true });
    try {
      const document = await Promise.race([task.promise, cancelled]);
      const page = await Promise.race([document.getPage(1), cancelled]);
      signal.throwIfAborted();
      const original = page.getViewport({ scale: 1 });
      const viewport = page.getViewport({ scale: 640 / Math.max(original.width, original.height) });
      const canvas = window.document.createElement("canvas");
      canvas.width = Math.ceil(viewport.width);
      canvas.height = Math.ceil(viewport.height);
      await Promise.race([page.render({ canvas, viewport, background: "#ffffff" }).promise, cancelled]);
      signal.throwIfAborted();
      const image = canvas.toDataURL("image/jpeg", .85);
      canvas.width = canvas.height = 0;
      previews.delete(url);
      previews.set(url, image);
      const oldest = previews.keys().next().value;
      if (previews.size > 80 && oldest !== undefined) previews.delete(oldest);
      return image;
    } finally {
      signal.removeEventListener("abort", cancel);
      await task.destroy().catch(() => {});
    }
  } finally {
    rendering -= 1;
    pending.shift()?.();
  }
}
