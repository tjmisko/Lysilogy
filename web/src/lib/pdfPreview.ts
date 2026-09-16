import { getDocument } from "pdfjs-dist";
import "./pdfWorker";
import { createPdfPreviewCache } from "./pdfPreviewCache";
import { pdfPreviewStorage } from "./pdfPreviewStorage";

async function renderPdfPreview(url: string): Promise<string> {
  // Request only the ranges needed for page one where the source supports ranges.
  const task = getDocument({ url, disableAutoFetch: true, disableStream: true });
  let canvas: HTMLCanvasElement | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error("Preview rendering timed out"));
      void task.destroy().catch(() => {});
    }, 30_000);
  });
  try {
    const document = await Promise.race([task.promise, deadline]);
    const page = await Promise.race([document.getPage(1), deadline]);
    const original = page.getViewport({ scale: 1 });
    const viewport = page.getViewport({ scale: 640 / Math.max(original.width, original.height) });
    canvas = window.document.createElement("canvas");
    canvas.width = Math.ceil(viewport.width);
    canvas.height = Math.ceil(viewport.height);
    await Promise.race([page.render({ canvas, viewport, background: "#ffffff" }).promise, deadline]);
    return canvas.toDataURL("image/jpeg", .85);
  } finally {
    clearTimeout(timer);
    if (canvas !== undefined) canvas.width = canvas.height = 0;
    // Worker teardown must not keep a timed-out job in a queue slot.
    void task.destroy().catch(() => {});
  }
}

const cache = createPdfPreviewCache({ render: renderPdfPreview, storage: pdfPreviewStorage });
export const cachedPdfPreview = cache.peek;
export const loadPdfPreview = cache.load;
