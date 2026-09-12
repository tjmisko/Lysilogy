import { useCallback, useEffect, useRef, useState } from "react";
import type { EditorView } from "@codemirror/view";

import { ApiError } from "../lib/api";
import { notesApi, type NoteDocument } from "../lib/notesApi";
import { createNotesEditor } from "../lib/notesEditor";
import "./NotesPanel.css";

type NotesPanelProps = { paperId: string; onClose: () => void; onFocusReader: () => void; onDirtyChange?: (dirty: boolean) => void };
type NotesCloseDetail = { afterClose?: () => void };

export function NotesPanel({ paperId, onClose, onFocusReader, onDirtyChange }: NotesPanelProps) {
  const panelRef = useRef<HTMLElement>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<EditorView | null>(null);
  const savedRef = useRef<NoteDocument | null>(null);
  const textRef = useRef("");
  const savingRef = useRef(false);
  const afterCloseRef = useRef<(() => void) | undefined>(undefined);
  const closeRef = useRef(onClose);
  const focusReaderRef = useRef(onFocusReader);
  const dirtyCallbackRef = useRef(onDirtyChange);
  const [document, setDocument] = useState<NoteDocument | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [diskVersion, setDiskVersion] = useState<NoteDocument | null>(null);
  const [closing, setClosing] = useState(false);
  const [savedNotice, setSavedNotice] = useState(false);
  const [loadAttempt, setLoadAttempt] = useState(0);

  useEffect(() => { closeRef.current = onClose; focusReaderRef.current = onFocusReader; dirtyCallbackRef.current = onDirtyChange; }, [onClose, onFocusReader, onDirtyChange]);

  const reportDirty = useCallback((value: boolean): void => {
    setDirty(value);
    dirtyCallbackRef.current?.(value);
  }, []);

  const finishClose = useCallback((): void => {
    const afterClose = afterCloseRef.current;
    afterCloseRef.current = undefined;
    dirtyCallbackRef.current?.(false);
    closeRef.current();
    // Let React remove this panel before a parent navigation guard runs again.
    if (afterClose !== undefined) window.requestAnimationFrame(afterClose);
  }, []);

  const requestClose = useCallback((afterClose?: () => void): void => {
    afterCloseRef.current = afterClose;
    if (savingRef.current || (savedRef.current !== null && textRef.current !== savedRef.current.text)) setClosing(true);
    else finishClose();
  }, [finishClose]);

  const cancelClose = useCallback((): void => {
    setClosing(false);
    afterCloseRef.current = undefined;
    editorRef.current?.focus();
  }, []);

  const quit = useCallback((discard = false): void => {
    if (discard && !savingRef.current) finishClose();
    else requestClose();
  }, [finishClose, requestClose]);
  const quitRef = useRef(quit);
  useEffect(() => { quitRef.current = quit; }, [quit]);

  const save = useCallback(async (closeAfter = false, force = false): Promise<void> => {
    const saved = savedRef.current;
    if (savingRef.current || saved === null) return;
    if (!force && textRef.current === saved.text) { if (closeAfter) finishClose(); return; }
    const submitted = textRef.current;
    savingRef.current = true;
    setSaving(true);
    setError(null);
    try {
      // Explicit :w! accepts the current disk revision. The following write
      // still checks it, so another edit during this request is not overwritten.
      const revision = force ? (await notesApi.read(paperId)).revision : saved.revision;
      const next = await notesApi.save(paperId, submitted, revision);
      savedRef.current = next;
      setConflict(false);
      setDiskVersion(null);
      setSavedNotice(true);
      const stillDirty = textRef.current !== next.text;
      reportDirty(stillDirty);
      if (closeAfter && !stillDirty) finishClose();
    } catch (reason: unknown) {
      setError(reason instanceof Error ? reason.message : "Could not save notes.");
      setConflict(reason instanceof ApiError && reason.status === 409);
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  }, [finishClose, paperId, reportDirty]);
  const saveRef = useRef(save);
  useEffect(() => { saveRef.current = save; }, [save]);

  useEffect(() => {
    const controller = new AbortController();
    void notesApi.open(paperId, controller.signal).then((note) => {
      if (controller.signal.aborted) return;
      savedRef.current = note;
      textRef.current = note.text;
      setDocument(note);
      setError(null);
      reportDirty(false);
    }).catch((reason: unknown) => {
      if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : "Could not open notes.");
    });
    return () => controller.abort();
  }, [loadAttempt, paperId, reportDirty]);

  useEffect(() => {
    if (document === null || hostRef.current === null) return;
    const editor = createNotesEditor({ text: document.text, parent: hostRef.current,
      onSave: (closeAfter, force) => { void saveRef.current(closeAfter, force); },
      onQuit: (discard) => quitRef.current(discard),
      onFocusReader: () => { requestAnimationFrame(() => focusReaderRef.current()); },
      onCommandError: (message) => { setError(message); setConflict(false); },
      onChange: (text) => {
      textRef.current = text;
      setSavedNotice(false);
      reportDirty(text !== savedRef.current?.text);
    } });
    editorRef.current = editor;
    editor.focus();
    return () => { editorRef.current = null; editor.destroy(); };
  }, [document, reportDirty]);

  useEffect(() => {
    const panel = panelRef.current;
    if (panel === null) return;
    const close = (event: Event): void => {
      event.preventDefault();
      event.stopPropagation();
      requestClose((event as CustomEvent<NotesCloseDetail | null>).detail?.afterClose);
    };
    panel.addEventListener("notes-close", close);
    const focus = (): void => { editorRef.current?.focus(); };
    panel.addEventListener("notes-focus", focus);
    return () => { panel.removeEventListener("notes-close", close); panel.removeEventListener("notes-focus", focus); };
  }, [requestClose]);

  useEffect(() => {
    if (!dirty) return;
    const beforeUnload = (event: BeforeUnloadEvent): void => { event.preventDefault(); };
    window.addEventListener("beforeunload", beforeUnload);
    return () => window.removeEventListener("beforeunload", beforeUnload);
  }, [dirty]);

  const compare = async (): Promise<void> => {
    try { setDiskVersion(await notesApi.read(paperId)); }
    catch (reason: unknown) { setError(reason instanceof Error ? reason.message : "Could not reload notes."); }
  };
  const useDiskVersion = (): void => {
    if (diskVersion === null) return;
    savedRef.current = diskVersion;
    textRef.current = diskVersion.text;
    setDocument(diskVersion);
    setConflict(false);
    setError(null);
    setDiskVersion(null);
    setClosing(false);
    afterCloseRef.current = undefined;
    reportDirty(false);
  };

  return <aside ref={panelRef} className="notes-panel" aria-label="Paper notes" data-dirty={dirty} onKeyDownCapture={(event) => {
    // A pending close belongs above the editor, so Escape cancels that prompt.
    if (event.key === "Escape" && closing) {
      event.preventDefault(); event.stopPropagation(); cancelClose();
    } else if (event.key === "Escape" && event.target instanceof Element && event.target.closest(".notes-disk-version")) {
      event.preventDefault(); event.stopPropagation(); setDiskVersion(null); editorRef.current?.focus();
    }
  }} onKeyDown={(event) => {
    // All editor keys, including otherwise-unhandled Normal-mode Escape, stay
    // local. Vim owns q (macro recording), /, ?, :, and its modal transitions.
    event.stopPropagation();
    if (event.defaultPrevented) return;
    const inEditor = event.target instanceof Element && event.target.closest(".cm-editor") !== null;
    if (inEditor) return;
    const editing = event.target instanceof HTMLElement && (event.target.closest(".cm-editor, input, textarea, select") !== null || event.target.isContentEditable);
    if (event.key === "Escape" || (event.key === "q" && !editing)) { event.preventDefault(); requestClose(); }
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") { event.preventDefault(); void save(); }
  }}>
    <header className="notes-header">
      <div><h2>Notes</h2><span title={document?.filename}>{document?.filename ?? (error === null ? "Opening Markdown…" : "Notes unavailable")}</span></div>
      <button type="button" className="notes-save" disabled={document === null || !dirty || saving} onClick={() => { void save(); }}>{saving ? "Saving…" : "Save"}</button>
      <button type="button" className="notes-close-button" aria-label="Close notes" onClick={() => requestClose()}>×</button>
    </header>
    {closing && <div className="notes-close-prompt" role="alert">
      <p>Save your notes before leaving?</p>
      <div><button type="button" disabled={saving} onClick={() => { void save(true); }}>Save and close</button><button type="button" disabled={saving} onClick={finishClose}>Discard changes</button><button type="button" onClick={cancelClose}>Keep editing</button></div>
    </div>}
    {error !== null && <div className="notes-error" role="alert"><p>{error}</p>
      {document === null && <button type="button" onClick={() => { setError(null); setLoadAttempt((value) => value + 1); }}>Retry</button>}
      {conflict && <button type="button" onClick={() => { void compare(); }}>Compare with disk</button>}
    </div>}
    {diskVersion !== null && <div className="notes-disk-version"><p>Current file on disk</p><pre>{diskVersion.text || "(Empty file)"}</pre><p>Your draft is still in the editor.</p><button type="button" onClick={useDiskVersion}>Discard draft and use disk version</button><button type="button" onClick={() => setDiskVersion(null)}>Keep my draft</button></div>}
    <div className="notes-editor" ref={hostRef} />
    <footer className="notes-footer"><span role="status">{document === null ? error === null ? "Opening notes…" : "Not opened" : dirty ? "Unsaved changes" : savedNotice ? "Saved" : "Saved to Markdown"}</span><span>Markdown <kbd>Ctrl S</kbd></span></footer>
  </aside>;
}
