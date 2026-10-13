import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, SyntheticEvent } from "react";
import { createPortal } from "react-dom";

import { api } from "../lib/api";
import { LITERATURE_QUIT_EVENT } from "../lib/literatureEvents";
import {
  PROVENANCE_COLUMN_ORDER,
  activeOverride,
  cellValues,
  fieldLabel,
  formatAuthors,
  overrideDraft,
  parseOverrideInput,
  type CellMark,
  type CellValue,
} from "../lib/provenance";
import type { ClaimView, FieldProvenance, ProvenanceView, Work } from "../types";
import "./WorkProvenance.css";

type Field = FieldProvenance["field"];

const MARK_GLYPH: Record<CellMark, string> = { selected: "◆", agrees: "✓", disagrees: "✗", excluded: "–" };
const MARK_TEXT: Record<Exclude<CellMark, "excluded">, string> = {
  selected: "selected value",
  agrees: "agrees with selected value",
  disagrees: "disagrees with selected value",
};

const FOCUSABLE = "button:not(:disabled), input, textarea, summary";

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || target.isContentEditable;
}

function markText(value: CellValue): string {
  if (value.mark !== "excluded") return MARK_TEXT[value.mark];
  return `excluded: ${value.claims[0]?.excluded_reason ?? "rejected"}`;
}

function summaryText(field: Field, display: string): string {
  return field === "authors" ? formatAuthors(display.split("; "), 4) : display;
}

type Editing = { workId: string; field: Field; draft: string; error: string | null };

/** The Work's printed authors and the Sources dialog: every claimed value of a field, by source. */
export function WorkProvenance({ work }: { work: Work }) {
  const [open, setOpen] = useState(false);
  const [loaded, setLoaded] = useState<{ workId: string; view: ProvenanceView } | null>(null);
  const [failure, setFailure] = useState<{ workId: string; message: string } | null>(null);
  const [editingState, setEditing] = useState<Editing | null>(null);
  const [busy, setBusy] = useState(false);
  const headingId = useId();
  const dialog = useRef<HTMLElement>(null);
  const opener = useRef<HTMLButtonElement>(null);

  const editing = editingState !== null && editingState.workId === work.id ? editingState : null;
  const isEditing = editing !== null;
  const view = loaded !== null && loaded.workId === work.id ? loaded.view : null;
  const error = failure !== null && failure.workId === work.id ? failure.message : null;

  const openDialog = useCallback(() => {
    setLoaded(null);
    setFailure(null);
    setEditing(null);
    setOpen(true);
  }, []);
  const closeDialog = useCallback(() => {
    setOpen(false);
    setEditing(null);
    opener.current?.focus();
  }, []);

  // `s` opens the dialog while this Work panel is mounted.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== "s" || open) return;
      if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
      if (isEditableTarget(event.target)) return;
      event.preventDefault();
      openDialog();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, openDialog]);

  // Esc and q reach the App close chain first. While open, this layer takes the press so
  // neither the Work panel nor the overlay closes with it.
  useEffect(() => {
    if (!open) return;
    const onQuit = (event: Event): void => {
      event.preventDefault();
      event.stopImmediatePropagation();
      if (isEditing) setEditing(null);
      else closeDialog();
    };
    document.addEventListener(LITERATURE_QUIT_EVENT, onQuit, true);
    return () => document.removeEventListener(LITERATURE_QUIT_EVENT, onQuit, true);
  }, [open, isEditing, closeDialog]);

  useEffect(() => {
    if (!open) return;
    const controller = new AbortController();
    const workId = work.id;
    api.workProvenance(workId, controller.signal)
      .then((next) => setLoaded({ workId, view: next }))
      .catch((reason: unknown) => {
        if (controller.signal.aborted) return;
        setFailure({ workId, message: reason instanceof Error ? reason.message : "Could not load sources" });
      });
    return () => controller.abort();
  }, [open, work.id]);

  // Keep focus inside the dialog when the control that held it goes away.
  useEffect(() => {
    const node = dialog.current;
    if (!open || node === null || node.contains(document.activeElement)) return;
    node.focus();
  });

  const apply = (request: Promise<ProvenanceView>): void => {
    const workId = work.id;
    setBusy(true);
    setFailure(null);
    request
      .then((next) => {
        setLoaded({ workId, view: next });
        setEditing(null);
      })
      .catch((reason: unknown) => {
        setFailure({ workId, message: reason instanceof Error ? reason.message : "Could not save" });
      })
      .finally(() => setBusy(false));
  };

  const save = (event: SyntheticEvent): void => {
    event.preventDefault();
    if (editing === null || busy) return;
    const parsed = parseOverrideInput(editing.field, editing.draft);
    if (!parsed.ok) {
      setEditing({ ...editing, error: parsed.error });
      return;
    }
    apply(api.setFieldOverride(work.id, editing.field, parsed.value));
  };

  const onDialogKeyDown = (event: ReactKeyboardEvent<HTMLElement>): void => {
    event.stopPropagation();
    if (event.key === "Escape" && !event.defaultPrevented) {
      event.preventDefault();
      if (isEditing) setEditing(null);
      else closeDialog();
      return;
    }
    if (event.key !== "Tab") return;
    const dialogNode = event.currentTarget;
    const controls = Array.from(dialogNode.querySelectorAll<HTMLElement>(FOCUSABLE))
      .filter((control) => control.getClientRects().length > 0);
    const first = controls[0];
    const last = controls.at(-1);
    if (event.shiftKey && (event.target === first || event.target === dialogNode)) {
      event.preventDefault();
      last?.focus();
    } else if (!event.shiftKey && event.target === last) {
      event.preventDefault();
      first?.focus();
    }
  };

  const authors = work.display_authors ?? [];
  const columns = view === null ? [] : PROVENANCE_COLUMN_ORDER
    .map((key) => view.columns.find((column) => column.key === key))
    .filter((column) => column !== undefined);

  const renderClaim = (claim: ClaimView) => (
    <div key={claim.id} className="work-provenance-claim">
      <p>
        {claim.extractor} · {claim.confidence}
        {claim.source_record !== null && ` · ${claim.source_record.kind}`}
        {claim.locator !== null && ` · ${claim.locator}`}
        {claim.review_state === "rejected" && " · rejected"}
      </p>
      {claim.entry?.citing_title != null && <p>Cited by {claim.entry.citing_title}</p>}
      {claim.entry !== null && <blockquote>{claim.entry.raw_text}</blockquote>}
      {claim.origin !== "user" && (
        <button
          type="button"
          disabled={busy}
          onClick={() => apply(api.reviewFieldClaim(claim.id, claim.review_state === "rejected" ? "unreviewed" : "rejected"))}
        >
          {claim.review_state === "rejected" ? "Restore" : "Reject"}
        </button>
      )}
    </div>
  );

  const renderValue = (field: FieldProvenance, value: CellValue) => (
    <li key={value.display} data-mark={value.mark}>
      <span className="work-provenance-glyph" aria-hidden="true">{MARK_GLYPH[value.mark]}</span>
      <span className="work-provenance-hidden">{markText(value)}: </span>
      <details>
        <summary>
          {summaryText(field.field, value.display)}
          {value.count > 1 && <span className="work-provenance-count">×{value.count}</span>}
        </summary>
        {field.field === "authors" && <p className="work-provenance-claim">{value.display}</p>}
        {value.claims.map(renderClaim)}
      </details>
    </li>
  );

  const renderUserCell = (field: FieldProvenance) => {
    const override = activeOverride(field);
    if (editing !== null && editing.field === field.field) {
      const label = `Override ${fieldLabel(field.field).toLowerCase()}`;
      const common = {
        "aria-label": label,
        autoFocus: true,
        value: editing.draft,
        onChange: (event: { target: { value: string } }) =>
          setEditing({ ...editing, draft: event.target.value, error: null }),
      };
      return (
        <form onSubmit={save}>
          {field.field === "authors" ? (
            <textarea
              {...common}
              rows={4}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) save(event);
              }}
            />
          ) : (
            <input {...common} type="text" />
          )}
          {editing.error !== null && <p className="work-provenance-error">{editing.error}</p>}
          <div className="work-provenance-actions">
            <button type="submit" disabled={busy}>Save</button>
            <button type="button" onClick={() => setEditing(null)}>Cancel</button>
            {override !== null && (
              <button type="button" disabled={busy} onClick={() => apply(api.clearFieldOverride(work.id, field.field))}>
                Clear
              </button>
            )}
          </div>
        </form>
      );
    }
    const startEditing = (): void => setEditing({
      workId: work.id,
      field: field.field,
      draft: overrideDraft(field.field, override?.display ?? field.selected.display),
      error: null,
    });
    const values = cellValues(field, "user");
    return (
      <>
        {values.length > 0 && <ul>{values.map((value) => renderValue(field, value))}</ul>}
        <button type="button" className="work-provenance-cell-button" onClick={startEditing}>
          {override === null ? "Override" : "Edit"}
          <span className="work-provenance-hidden"> {fieldLabel(field.field).toLowerCase()}</span>
        </button>
      </>
    );
  };

  const renderCell = (field: FieldProvenance, columnKey: (typeof PROVENANCE_COLUMN_ORDER)[number]) => {
    if (columnKey === "user") return renderUserCell(field);
    const values = cellValues(field, columnKey);
    if (values.length === 0) return <span className="work-provenance-hidden">no claim</span>;
    return <ul>{values.map((value) => renderValue(field, value))}</ul>;
  };

  const renderSelected = (field: FieldProvenance): string => {
    const { value, display } = field.selected;
    if (display === null) return "unknown";
    return field.field === "authors" && Array.isArray(value)
      ? formatAuthors(value.map(String), 4)
      : display;
  };

  return (
    <>
      {authors.length > 0 && (
        <p className="work-provenance-authors" title={authors.join("; ")}>{formatAuthors(authors)}</p>
      )}
      <button ref={opener} type="button" className="work-provenance-open" onClick={openDialog}>
        Sources <kbd>s</kbd>
      </button>
      {open && createPortal(
        <div
          className="work-provenance-backdrop"
          onMouseDown={(event) => { if (event.target === event.currentTarget) closeDialog(); }}
        >
          <section
            ref={dialog}
            className="work-provenance"
            role="dialog"
            aria-modal="true"
            aria-labelledby={headingId}
            tabIndex={-1}
            onKeyDown={onDialogKeyDown}
          >
            <header>
              <h2 id={headingId}>Sources</h2>
              <button type="button" className="work-provenance-close" aria-label="Close sources (Esc)" onClick={closeDialog}>×</button>
            </header>
            {error !== null && <p className="work-provenance-error" role="alert">{error}</p>}
            {view === null && error === null && <p>Loading…</p>}
            {view !== null && (
              <div className="work-provenance-table-wrap">
                <table>
                  <caption>
                    Every value each source claims. Outlined is the selected value; green agrees with it, red disagrees; struck through is rejected.
                  </caption>
                  <thead>
                    <tr>
                      <th scope="col">Field</th>
                      {columns.map((column) => <th key={column.key} scope="col">{column.label}</th>)}
                    </tr>
                  </thead>
                  <tbody>
                    {view.fields.map((field) => (
                      <tr key={field.field}>
                        <th scope="row">
                          <span className="work-provenance-field">{fieldLabel(field.field)}</span>
                          <span className="work-provenance-selected">{renderSelected(field)}</span>
                          {field.selected.state === "conflicting" && <span className="work-provenance-badge">conflicting</span>}
                        </th>
                        {columns.map((column) => <td key={column.key}>{renderCell(field, column.key)}</td>)}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        </div>,
        document.body,
      )}
    </>
  );
}
