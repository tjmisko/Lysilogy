import { useEffect, useMemo, useRef, useState, type RefObject } from "react";

import type { PaperOverview } from "../types";

type LibraryRailProps = {
  open: boolean;
  keyboardMode: boolean;
  name: string;
  papers: PaperOverview[];
  selectedId: string | null;
  query: string;
  searchRef: RefObject<HTMLInputElement | null>;
  onQuery: (query: string) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
  onScan: () => void;
  onHome: () => void;
  onImport: (url: string) => Promise<void>;
  onVisiblePapersChange: (ids: string[]) => void;
};

const collator = new Intl.Collator(undefined, { sensitivity: "base" });

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
}

export function LibraryRail({
  open,
  keyboardMode,
  name,
  papers,
  selectedId,
  query,
  searchRef,
  onQuery,
  onSelect,
  onClose,
  onScan,
  onHome,
  onImport,
  onVisiblePapersChange,
}: LibraryRailProps) {
  const [mappedOnly, setMappedOnly] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [importUrl, setImportUrl] = useState("");
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return papers
      .filter((paper) => {
        if (mappedOnly && paper.status.state !== "ready") return false;
        if (needle.length === 0) return true;
        const haystack = [
          paper.metadata.title,
          ...paper.metadata.authors,
          paper.metadata.year?.toString() ?? "",
        ]
          .join(" ")
          .toLocaleLowerCase();
        return haystack.includes(needle);
      })
      .sort((left, right) => collator.compare(left.metadata.title, right.metadata.title));
  }, [mappedOnly, papers, query]);

  useEffect(() => {
    onVisiblePapersChange(filtered.map((paper) => paper.id));
  }, [filtered, onVisiblePapersChange]);

  const readyCount = papers.filter((paper) => paper.status.state === "ready").length;
  const [active, setActive] = useState(0);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const lastIndex = Math.max(0, filtered.length - 1);
  const activeIndex = Math.min(active, lastIndex);

  useEffect(() => {
    if (!keyboardMode) return;
    itemRefs.current[activeIndex]?.focus({ preventScroll: true });
    itemRefs.current[activeIndex]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [activeIndex, keyboardMode]);

  useEffect(() => {
    if (!open) return;
    const onFilterKey = (event: KeyboardEvent): void => {
      if (event.key === "f" && !isEditable(event.target) && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        setActive(0);
        setMappedOnly((value) => !value);
      }
    };
    window.addEventListener("keydown", onFilterKey);
    return () => window.removeEventListener("keydown", onFilterKey);
  }, [open]);

  useEffect(() => {
    if (!keyboardMode) return;
    let pendingG = false;
    let pendingTimer: number | null = null;
    const clearPending = (): void => {
      pendingG = false;
      if (pendingTimer !== null) window.clearTimeout(pendingTimer);
      pendingTimer = null;
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        isEditable(event.target) ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey
      ) {
        return;
      }
      switch (event.key) {
        case "j":
        case "ArrowDown":
          event.preventDefault();
          clearPending();
          setActive((index) => Math.min(lastIndex, index + 1));
          break;
        case "k":
        case "ArrowUp":
          event.preventDefault();
          clearPending();
          setActive((index) => Math.max(0, index - 1));
          break;
        case "G":
          event.preventDefault();
          clearPending();
          setActive(lastIndex);
          break;
        case "g":
          event.preventDefault();
          if (pendingG) {
            clearPending();
            setActive(0);
          } else {
            pendingG = true;
            pendingTimer = window.setTimeout(clearPending, 420);
          }
          break;
        case "Enter":
        case "o": {
          const paper = filtered[activeIndex];
          if (paper !== undefined) {
            event.preventDefault();
            clearPending();
            onSelect(paper.id);
          }
          break;
        }
        case "/":
          event.preventDefault();
          clearPending();
          searchRef.current?.focus();
          break;
        case "r":
          event.preventDefault();
          clearPending();
          onScan();
          break;
        case "Escape":
        case "b":
          event.preventDefault();
          clearPending();
          onClose();
          break;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      clearPending();
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [activeIndex, filtered, keyboardMode, lastIndex, onClose, onScan, onSelect, searchRef]);

  return (
    <>
      <button
        className={`rail-scrim ${open ? "is-open" : ""}`}
        aria-label="Close library"
        onClick={onClose}
        tabIndex={open ? 0 : -1}
      />
      <aside className={`library-rail ${open ? "is-open" : ""}`} aria-label="Paper library" inert={!open} aria-hidden={!open || undefined}>
        <button className="brand rail-brand" type="button" onClick={onHome} aria-label="Go to library home"><img className="brand-mark" src="/lambda-mark.svg" alt="" /><strong>LYSILOGY</strong></button>
        <div className="rail-heading">
          <div>
            <span className="eyebrow">Vault</span>
            <h2>{name}</h2>
          </div>
          <button className="icon-button mobile-only" onClick={onClose} aria-label="Close library">
            ×
          </button>
        </div>
        <div className="library-counts">
          <span>{papers.length} papers</span>
          <span className="library-count-actions">
            <button
              type="button"
              className={importOpen ? "is-active" : ""}
              aria-expanded={importOpen}
              onClick={() => {
                setImportError(null);
                setImportOpen((value) => !value);
              }}
            >
              + URL
            </button>
            <button
              type="button"
              className={mappedOnly ? "is-active" : ""}
              aria-pressed={mappedOnly}
              aria-label={mappedOnly ? "Show all papers" : "Show mapped papers only"}
              title={mappedOnly ? "Show all papers" : "Show mapped papers only"}
              onClick={() => {
                setActive(0);
                setMappedOnly((value) => !value);
              }}
            >
              <i aria-hidden="true" /> {mappedOnly ? "Mapped only" : `${readyCount} mapped`} <kbd>f</kbd>
            </button>
          </span>
        </div>
        {importOpen && (
          <form
            className="library-import"
            onSubmit={(event) => {
              event.preventDefault();
              const url = importUrl.trim();
              if (url.length === 0 || importing) return;
              setImporting(true);
              setImportError(null);
              void onImport(url)
                .then(() => {
                  setImportUrl("");
                  setImportOpen(false);
                })
                .catch((reason: unknown) => {
                  setImportError(reason instanceof Error ? reason.message : "Could not import PDF");
                })
                .finally(() => setImporting(false));
            }}
          >
            <label htmlFor="remote-pdf-url">PDF URL</label>
            <div>
              <input
                id="remote-pdf-url"
                type="url"
                inputMode="url"
                autoComplete="url"
                required
                value={importUrl}
                onChange={(event) => setImportUrl(event.target.value)}
                placeholder="https://…/paper.pdf"
                aria-describedby={importError === null ? undefined : "remote-pdf-error"}
              />
              <button type="submit" disabled={importing || importUrl.trim().length === 0}>
                {importing ? "Importing…" : "Import"}
              </button>
            </div>
            {importError !== null && <p id="remote-pdf-error">{importError}</p>}
            <small>Public HTTP(S), up to 100 MiB. Redirects are checked.</small>
          </form>
        )}
        <label className="search-box">
          <span aria-hidden="true">/</span>
          <input
            ref={searchRef}
            type="search"
            value={query}
            onChange={(event) => {
              setActive(0);
              onQuery(event.target.value);
            }}
            placeholder="Filter title, author, year"
            aria-label="Filter papers"
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                onQuery("");
                event.currentTarget.blur();
              } else if (event.key === "Enter" && filtered[activeIndex] !== undefined) {
                onSelect(filtered[activeIndex].id);
              }
            }}
          />
          {query.length > 0 && (
            <button onClick={() => onQuery("")} aria-label="Clear search" type="button">
              ×
            </button>
          )}
        </label>
        <div className="paper-list" role="listbox" aria-label="Discovered papers">
          {filtered.map((paper, index) => (
            <button
              key={paper.id}
              ref={(node) => {
                itemRefs.current[index] = node;
              }}
              type="button"
              role="option"
              aria-selected={paper.id === selectedId}
              className={`paper-list-item ${paper.id === selectedId ? "is-selected" : ""}`}
              tabIndex={keyboardMode ? (index === activeIndex ? 0 : -1) : 0}
              onFocus={() => setActive(index)}
              onClick={() => onSelect(paper.id)}
            >
              <span className={`status-pip status-${paper.status.state}`} aria-hidden="true" />
              <span className="paper-list-copy">
                <strong>
                  {paper.metadata.authors[0] ?? "Unknown Author"}{" "}
                  {paper.metadata.year === null ? "" : `- ${paper.metadata.year}`} - {paper.metadata.title}
                </strong>
              </span>
            </button>
          ))}
          {filtered.length === 0 && (
            <p className="quiet-message">
              {mappedOnly && query.length === 0
                ? "No mapped papers yet."
                : `No papers match “${query}”.`}
            </p>
          )}
        </div>
        <button className="rail-footer" type="button" onClick={onScan}>
          <span>↻</span>
          Rescan vault
          <kbd>r</kbd>
        </button>
      </aside>
    </>
  );
}
