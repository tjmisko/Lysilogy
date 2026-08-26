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
};

const collator = new Intl.Collator(undefined, { sensitivity: "base" });

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
}: LibraryRailProps) {
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return papers
      .filter((paper) => {
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
  }, [papers, query]);

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
        event.target === searchRef.current ||
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
      <aside className={`library-rail ${open ? "is-open" : ""}`} aria-label="Paper library">
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
          <span>{readyCount} mapped</span>
        </div>
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
                <strong>{paper.metadata.title}</strong>
                <small>
                  {paper.metadata.authors[0] ?? "Unknown author"}
                  {paper.metadata.year === null ? "" : ` · ${paper.metadata.year}`}
                </small>
              </span>
            </button>
          ))}
          {filtered.length === 0 && <p className="quiet-message">No papers match “{query}”.</p>}
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
