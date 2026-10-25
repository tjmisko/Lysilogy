import { useEffect, useRef, useState } from "react";

import type { LibraryDirectory } from "../types";

type AddMenuProps = {
  onImportUrl: () => void;
  onUpload: (file: File) => Promise<void>;
  directories: LibraryDirectory[];
  onAddDirectory: (path: string) => Promise<void>;
  onRemoveDirectory: (id: string) => Promise<void>;
};

export function AddMenu({ onImportUrl, onUpload, directories, onAddDirectory, onRemoveDirectory }: AddMenuProps) {
  const [open, setOpen] = useState(false);
  const [path, setPath] = useState("");
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

  const run = async (action: () => Promise<void>, after: () => void): Promise<void> => {
    setBusy(true);
    try {
      await action();
      after();
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
      <div className="add-menu-folders" role="group" aria-label="Library folders">
        <span className="add-menu-heading">Library folders</span>
        {directories.map((directory) => <div key={directory.id} className={`add-menu-folder-row${directory.available ? "" : " is-missing"}`}>
          <span title={directory.path}>{directory.name}{directory.available ? "" : " (missing)"}<small>{directory.papers}</small></span>
          {!directory.primary && <button type="button" aria-label={`Remove ${directory.name} from the library`} disabled={busy}
            onClick={() => void run(() => onRemoveDirectory(directory.id), () => undefined)}>×</button>}
        </div>)}
        <form className="add-menu-folder" onSubmit={(event) => {
          event.preventDefault();
          const trimmed = path.trim();
          if (trimmed === "") return;
          void run(() => onAddDirectory(trimmed), () => setPath(""));
        }}>
          <input aria-label="Add a library folder" placeholder="/path/to/folder" value={path} onChange={(event) => setPath(event.target.value)} />
          <button type="submit" disabled={busy || path.trim() === ""}>Add</button>
        </form>
      </div>
    </div>}
    <input ref={fileInput} type="file" accept="application/pdf,.pdf" hidden aria-label="Upload a PDF" onChange={(event) => {
      const file = event.target.files?.[0];
      event.target.value = "";
      if (file !== undefined) void run(() => onUpload(file), () => setOpen(false));
    }} />
  </div>;
}
