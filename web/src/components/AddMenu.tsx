import { useEffect, useRef, useState } from "react";

type AddMenuProps = {
  onImportUrl: () => void;
  onUpload: (file: File) => Promise<void>;
};

export function AddMenu({ onImportUrl, onUpload }: AddMenuProps) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent): void => {
      if (event.target instanceof Node && root.current?.contains(event.target) === true) return;
      setOpen(false);
    };
    window.addEventListener("pointerdown", dismiss);
    return () => window.removeEventListener("pointerdown", dismiss);
  }, [open]);

  const upload = async (file: File): Promise<void> => {
    setBusy(true);
    try {
      await onUpload(file);
      setOpen(false);
    } catch {
      // The app surfaces the failure in its own error banner; keep the menu open to retry.
    } finally {
      setBusy(false);
    }
  };

  return <div className="add-menu" ref={root} onKeyDown={(event) => {
    if (event.key !== "Escape" || !open) return;
    event.stopPropagation();
    setOpen(false);
  }}>
    <button type="button" className="add-menu-trigger" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
      Add <span aria-hidden="true">▾</span>
    </button>
    {open && <div className="add-menu-popover" role="menu" aria-label="Add to library">
      <button type="button" role="menuitem" disabled={busy} onClick={() => { setOpen(false); onImportUrl(); }}>Import PDF from URL</button>
      <button type="button" role="menuitem" disabled={busy} onClick={() => fileInput.current?.click()}>Upload a paper…</button>
    </div>}
    <input ref={fileInput} type="file" accept="application/pdf,.pdf" hidden aria-label="Upload a PDF" onChange={(event) => {
      const file = event.target.files?.[0];
      event.target.value = "";
      if (file !== undefined) void upload(file);
    }} />
  </div>;
}
