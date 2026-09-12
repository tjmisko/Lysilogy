import { useEffect, useState } from "react";
import { getDocument, type PDFDocumentProxy, type PDFDocumentLoadingTask } from "pdfjs-dist";
import "../lib/pdfWorker";

type Entry = { task: PDFDocumentLoadingTask; document: PDFDocumentProxy | null; users: number; releaseTimer: number | null };
const documents = new Map<string, Entry>();

/** Map, full reader, and section reader share one loading task per source URL. */
export function usePdfDocument(url: string) {
  const [state, setState] = useState<{ url: string; document: PDFDocumentProxy | null; error: string | null }>({
    url, document: documents.get(url)?.document ?? null, error: null,
  });
  useEffect(() => {
    let entry = documents.get(url);
    if (entry === undefined) {
      entry = { task: getDocument({ url }), document: null, users: 0, releaseTimer: null };
      documents.set(url, entry);
    }
    const active = entry;
    if (active.releaseTimer !== null) window.clearTimeout(active.releaseTimer);
    active.releaseTimer = null;
    active.users += 1;
    let cancelled = false;
    void active.task.promise.then((document) => {
      active.document = document;
      if (!cancelled) setState({ url, document, error: null });
    }).catch((reason: unknown) => {
      if (!cancelled) setState({ url, document: null, error: reason instanceof Error ? reason.message : "Could not load the PDF" });
    });
    return () => {
      cancelled = true;
      active.users -= 1;
      if (active.users === 0) {
        // Retain across a view transition; cancellation happens after page renders unmount.
        active.releaseTimer = window.setTimeout(() => {
          if (active.users !== 0) return;
          documents.delete(url);
          void active.task.destroy();
        }, 1000);
      }
    };
  }, [url]);
  const current = state.url === url ? state : { document: null, error: null };
  return { ...current, loading: current.document === null && current.error === null };
}
