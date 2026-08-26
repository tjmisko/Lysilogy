import { useEffect, useMemo, useRef, useState } from "react";

import type { GlossaryEntry } from "../types";

type GlossPanelProps = {
  entries: GlossaryEntry[];
  onClose: () => void;
  onSection: (sectionId: string) => void;
};

export function GlossPanel({ entries, onClose, onSection }: GlossPanelProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [expanded, setExpanded] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const itemRefs = useRef<Array<HTMLDivElement | null>>([]);
  const filtered = useMemo(() => {
    const needle = query.toLocaleLowerCase().trim();
    return entries.filter((entry) =>
      `${entry.term} ${entry.plain_language}`.toLocaleLowerCase().includes(needle),
    );
  }, [entries, query]);

  useEffect(() => {
    itemRefs.current[active]?.focus({ preventScroll: true });
    itemRefs.current[active]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [active]);

  useEffect(() => {
    let pendingG = false;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        const searchInput = searchRef.current;
        if (searchInput !== null && document.activeElement === searchInput) {
          searchInput.blur();
          setQuery("");
        } else {
          onClose();
        }
        return;
      }
      if (event.target === searchRef.current) return;
      const last = Math.max(0, filtered.length - 1);
      if (event.key === "j" || event.key === "l" || event.key === "ArrowDown" || event.key === "ArrowRight") {
        event.preventDefault();
        setActive((index) => Math.min(last, index + 1));
      } else if (event.key === "k" || event.key === "h" || event.key === "ArrowUp" || event.key === "ArrowLeft") {
        event.preventDefault();
        setActive((index) => Math.max(0, index - 1));
      } else if (event.key === "G") {
        event.preventDefault();
        setActive(last);
      } else if (event.key === "g") {
        event.preventDefault();
        if (pendingG) {
          setActive(0);
          pendingG = false;
        } else {
          pendingG = true;
          window.setTimeout(() => {
            pendingG = false;
          }, 400);
        }
      } else if (event.key === "/") {
        event.preventDefault();
        searchRef.current?.focus();
      } else if (event.key === "Enter") {
        const entry = filtered[active];
        if (entry !== undefined) {
          event.preventDefault();
          setExpanded((term) => (term === entry.term ? null : entry.term));
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, filtered, onClose]);

  return (
    <aside className="context-panel gloss-panel" aria-label="Glossary">
      <header className="panel-header">
        <div>
          <span className="eyebrow">Field guide</span>
          <h2>Gloss</h2>
        </div>
        <button className="icon-button" type="button" onClick={onClose} aria-label="Close Gloss">
          ×
        </button>
      </header>
      <label className="search-box gloss-search">
        <span aria-hidden="true">/</span>
        <input
          ref={searchRef}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
          }}
          placeholder="Find a concept"
          aria-label="Find a concept"
        />
      </label>
      <div className="gloss-list">
        {filtered.map((entry, index) => {
          const isExpanded = expanded === entry.term;
          return (
            <div
              key={entry.term}
              ref={(node) => {
                itemRefs.current[index] = node;
              }}
              className={`gloss-entry ${index === active ? "is-active" : ""}`}
              tabIndex={index === active ? 0 : -1}
              onFocus={() => setActive(index)}
              onClick={() => {
                setActive(index);
                setExpanded(isExpanded ? null : entry.term);
              }}
            >
              <span className="gloss-letter">{entry.term.charAt(0).toUpperCase()}</span>
              <div>
                <h3>{entry.term}</h3>
                <p>{entry.plain_language}</p>
                {isExpanded && (
                  <div className="gloss-detail">
                    <span>Technical</span>
                    <p>{entry.technical_definition}</p>
                    <span>Why it matters here</span>
                    <p>{entry.why_it_matters}</p>
                    {entry.section_ids.map((sectionId) => (
                      <button key={sectionId} type="button" onClick={() => onSection(sectionId)}>
                        {sectionId.replaceAll("-", " ")} ↗
                      </button>
                    ))}
                  </div>
                )}
              </div>
              <kbd>↵</kbd>
            </div>
          );
        })}
        {filtered.length === 0 && (
          <p className="quiet-message">No glossary entry matches “{query}”.</p>
        )}
      </div>
      <footer className="panel-footer">
        <span><kbd>j/k</kbd> move</span>
        <span><kbd>↵</kbd> expand</span>
        <span>{filtered.length} terms</span>
      </footer>
    </aside>
  );
}
