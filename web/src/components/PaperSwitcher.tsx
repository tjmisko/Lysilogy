import { useEffect, useMemo, useRef, useState } from "react";

import type { PaperOverview } from "../types";

type PaperSwitcherProps = {
  papers: PaperOverview[];
  selectedId: string | null;
  onClose: () => void;
  onSelect: (id: string) => void;
};

type Candidate = { paper: PaperOverview; score: number };

function fuzzyScore(query: string, candidate: string): number | null {
  const needle = query.toLocaleLowerCase().replaceAll(/\s+/gu, "");
  const haystack = candidate.toLocaleLowerCase();
  if (needle.length === 0) return 0;
  let score = 0;
  let cursor = -1;
  for (const character of needle) {
    const next = haystack.indexOf(character, cursor + 1);
    if (next < 0) return null;
    const boundary = next === 0 || /[\s\-—/]/u.test(haystack[next - 1] ?? "");
    score += boundary ? 8 : 2;
    score += next === cursor + 1 ? 5 : Math.max(-8, cursor - next + 1);
    cursor = next;
  }
  return score - haystack.length / 200;
}

function searchableText(paper: PaperOverview): string {
  return [paper.metadata.title, ...paper.metadata.authors, paper.metadata.year ?? ""].join(" ");
}

export function PaperSwitcher({ papers, selectedId, onClose, onSelect }: PaperSwitcherProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const candidates = useMemo(() => {
    const matches = papers.flatMap<Candidate>((paper) => {
      const score = fuzzyScore(query, searchableText(paper));
      return score === null ? [] : [{ paper, score }];
    });
    return matches.sort((left, right) => {
      if (query.length > 0 && left.score !== right.score) return right.score - left.score;
      const readyDifference = Number(right.paper.status.state === "ready") - Number(left.paper.status.state === "ready");
      return readyDifference || left.paper.metadata.title.localeCompare(right.paper.metadata.title);
    });
  }, [papers, query]);
  const lastIndex = Math.max(0, candidates.length - 1);
  const activeIndex = Math.min(active, lastIndex);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    itemRefs.current[activeIndex]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [activeIndex]);

  useEffect(() => {
    const choose = (): void => {
      const candidate = candidates[activeIndex];
      if (candidate !== undefined) onSelect(candidate.paper.id);
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      const backward = event.key === "ArrowUp" || (event.ctrlKey && ["k", "p"].includes(event.key));
      const forward = event.key === "ArrowDown" || (event.ctrlKey && ["j", "n"].includes(event.key));
      if (backward || (event.key === "Tab" && event.shiftKey)) {
        event.preventDefault();
        setActive((index) => Math.max(0, index - 1));
      } else if (forward || event.key === "Tab") {
        event.preventDefault();
        setActive((index) => Math.min(lastIndex, index + 1));
      } else if (event.key === "Enter") {
        event.preventDefault();
        choose();
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeIndex, candidates, lastIndex, onClose, onSelect]);

  return (
    <div className="switcher-backdrop" role="presentation" onMouseDown={onClose}>
      <section
        className="paper-switcher"
        role="dialog"
        aria-modal="true"
        aria-labelledby="switcher-heading"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <span className="eyebrow">Jump anywhere</span>
            <h2 id="switcher-heading">Switch article</h2>
          </div>
          <kbd>F10</kbd>
        </header>
        <label className="switcher-search">
          <span>›</span>
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            placeholder="Fuzzy-find title, author, or year"
            aria-label="Fuzzy-find an article"
          />
        </label>
        <div className="switcher-results" role="listbox">
          {candidates.map(({ paper }, index) => (
            <button
              key={paper.id}
              ref={(node) => {
                itemRefs.current[index] = node;
              }}
              type="button"
              role="option"
              aria-selected={paper.id === selectedId}
              className={index === activeIndex ? "is-active" : ""}
              onMouseEnter={() => setActive(index)}
              onClick={() => onSelect(paper.id)}
              tabIndex={-1}
            >
              <span className={`status-pip status-${paper.status.state}`} />
              <span>
                <strong>{paper.metadata.title}</strong>
                <small>
                  {paper.metadata.authors.join(", ") || "Unknown author"}
                  {paper.metadata.year === null ? "" : ` · ${paper.metadata.year}`}
                </small>
              </span>
              {paper.status.state === "ready" && <em>Mapped</em>}
            </button>
          ))}
          {candidates.length === 0 && <p>No article matches “{query}”.</p>}
        </div>
        <footer>
          <span><kbd>↑/↓</kbd> or <kbd>ctrl+n/p</kbd> move</span>
          <span><kbd>↵</kbd> open</span>
          <span>{candidates.length} matches</span>
        </footer>
      </section>
    </div>
  );
}
