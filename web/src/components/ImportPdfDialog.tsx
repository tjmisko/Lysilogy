import { useEffect, useRef, useState } from "react";

export function ImportPdfDialog({ onClose, onImport }: {
  onClose: () => void;
  onImport: (url: string) => Promise<void>;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { dialog.current?.showModal(); }, []);
  return <dialog ref={dialog} className="import-pdf-dialog" aria-labelledby="import-pdf-title"
    onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}
    onKeyDown={(event) => event.stopPropagation()}>
    <form onSubmit={(event) => {
      event.preventDefault();
      if (busy || url.trim() === "") return;
      setBusy(true); setError(null);
      void onImport(url.trim()).then(onClose).catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not import PDF");
      }).finally(() => setBusy(false));
    }}>
      <span className="eyebrow">Add to your library</span>
      <h2 id="import-pdf-title">Import a PDF</h2>
      <label htmlFor="import-pdf-url">PDF URL</label>
      <input id="import-pdf-url" type="url" required autoFocus placeholder="https://…/paper.pdf"
        value={url} onChange={(event) => setUrl(event.target.value)} disabled={busy} />
      <p>Paste a public PDF link to save a copy in your library.</p>
      {error !== null && <p role="alert">{error}</p>}
      <div className="import-pdf-actions">
        <button type="button" disabled={busy} onClick={onClose}>Cancel</button>
        <button type="submit" disabled={busy || url.trim() === ""}>{busy ? "Importing…" : "Import PDF"}</button>
      </div>
    </form>
  </dialog>;
}
