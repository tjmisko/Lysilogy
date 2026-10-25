import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { api } from "../lib/api";
import { spatialNeighbor, type Direction } from "../lib/spatialNavigation";
import type { PaperOverview } from "../types";
import { authorSurnames } from "../lib/authors";
import { AddMenu } from "./AddMenu";
import { PaperPreview } from "./PaperPreview";
import "./HomePage.css";

type HomePageProps = {
  papers: PaperOverview[];
  query: string;
  onQuery: (query: string) => void;
  activeId: string | null;
  onActive: (id: string) => void;
  onSelect: (id: string) => void;
  onImport: () => void;
  onUpload: (file: File) => Promise<void>;
  keyboardEnabled: boolean;
  darkInk: boolean;
  onToggleInk: () => void;
};

type Filter = "all" | "mapped" | "unmapped";
const collator = new Intl.Collator(undefined, { sensitivity: "base" });
const accents = ["#dec282", "#aaa0d8", "#8dc1af", "#d49e87", "#9cb4d4", "#c2bf94"];

function paperAccent(id: string): string {
  let hash = 0;
  for (const letter of id) hash = (hash * 31 + letter.charCodeAt(0)) >>> 0;
  return accents[hash % accents.length] ?? "#dec282";
}

function paperStatus(paper: PaperOverview): string {
  switch (paper.status.state) {
    case "ready": return "Mapped";
    case "queued": return "Queued";
    case "extracting":
    case "analyzing": return "Processing";
    case "failed": return "Needs attention";
    default: return "PDF";
  }
}

/** Splits "Main: Sub" or "Main — Sub" so the subtitle can sit quieter under the title. */
function TitleLines({ id, title }: { id: string; title: string }) {
  const match = /^(.+?)\s*(?:—|–|:)\s+(.+)$/u.exec(title);
  return <>
    <h2 id={id} title={title}>{match?.[1] ?? title}</h2>
    {match?.[2] !== undefined && <span className="paper-card-subtitle" aria-hidden="true">{match[2]}</span>}
  </>;
}

export function HomePage({ papers, query, onQuery, activeId, onActive, onSelect, onImport, onUpload, keyboardEnabled, darkInk, onToggleInk }: HomePageProps) {
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState("title");
  const searchRef = useRef<HTMLInputElement>(null);
  const gridRef = useRef<HTMLUListElement>(null);
  const mapped = papers.filter((paper) => paper.status.state === "ready").length;
  const visible = useMemo(() => {
    const words = query.trim().toLocaleLowerCase().split(/\s+/u).filter(Boolean);
    return papers.filter((paper) => {
      const ready = paper.status.state === "ready";
      if ((filter === "mapped" && !ready) || (filter === "unmapped" && ready)) return false;
      const text = [paper.metadata.title, ...paper.metadata.authors, paper.metadata.year ?? "", paper.metadata.subject ?? ""].join(" ").toLocaleLowerCase();
      return words.every((word) => text.includes(word));
    }).sort((a, b) => {
      if (sort === "newest") {
        const difference = (b.metadata.year ?? 0) - (a.metadata.year ?? 0);
        if (difference !== 0) return difference;
      }
      return collator.compare(a.metadata.title, b.metadata.title);
    });
  }, [filter, papers, query, sort]);
  const selectedId = visible.find((paper) => paper.id === activeId)?.id ?? visible[0]?.id;

  useEffect(() => {
    if (!keyboardEnabled) return;
    const frame = window.requestAnimationFrame(() => {
      const card = gridRef.current?.querySelector<HTMLButtonElement>('.paper-card[tabindex="0"]');
      card?.focus({ preventScroll: true });
      card?.scrollIntoView({ block: "nearest", inline: "nearest" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [keyboardEnabled]);

  useEffect(() => {
    if (!keyboardEnabled) return;
    const focusCard = (card: HTMLButtonElement | undefined): void => {
      card?.focus({ preventScroll: true });
      card?.scrollIntoView({ block: "nearest", inline: "nearest" });
    };
    const navigate = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey || event.altKey) return;
      const target = event.target;
      const cards = Array.from(gridRef.current?.querySelectorAll<HTMLButtonElement>(".paper-card") ?? []);
      if (target === searchRef.current) {
        if (["ArrowDown", "Enter", "Escape"].includes(event.key)) {
          event.preventDefault();
          focusCard(cards.find((card) => card.dataset.paperId === selectedId));
        }
        return;
      }
      if (target instanceof HTMLElement && (target.matches("input, textarea, select") || target.isContentEditable)) return;
      if (event.key === "I") {
        event.preventDefault();
        onToggleInk();
        return;
      }
      if (event.key === "/") {
        event.preventDefault();
        searchRef.current?.focus();
        return;
      }
      const directions: Partial<Record<string, Direction>> = {
        ArrowLeft: "left", h: "left", ArrowRight: "right", l: "right",
        ArrowUp: "up", k: "up", ArrowDown: "down", j: "down",
      };
      const direction = directions[event.key];
      if (direction !== undefined) {
        event.preventDefault();
        const focused = cards.findIndex((card) => card === target);
        const origin = focused >= 0 ? focused : cards.findIndex((card) => card.dataset.paperId === selectedId);
        const next = spatialNeighbor(cards.map((card, section) => {
          const { left, right, top, bottom } = card.getBoundingClientRect();
          return { left, right, top, bottom, section };
        }), origin, direction);
        focusCard(cards[next ?? origin]);
      } else if (["Home", "End", "G"].includes(event.key)) {
        event.preventDefault();
        focusCard(event.key === "Home" ? cards[0] : cards.at(-1));
      } else if ((event.key === "Enter" || event.key === "o") && selectedId !== undefined) {
        // Enter on toolbar controls retains its native action.
        if (event.key === "Enter" && target instanceof HTMLElement && target.closest("button, a") !== null
          && !target.closest(".paper-card")) return;
        event.preventDefault();
        onSelect(selectedId);
      }
    };
    window.addEventListener("keydown", navigate);
    return () => window.removeEventListener("keydown", navigate);
  }, [keyboardEnabled, onSelect, onToggleInk, selectedId, visible]);

  return <section className={`home-page${darkInk ? " dark-ink" : ""}`}>
    <div className="home-controls">
      <label className="home-search">
        <span aria-hidden="true">⌕</span>
        <input ref={searchRef} type="search" aria-label="Search papers" value={query} placeholder="Search title, author, or year" onChange={(event) => onQuery(event.target.value)} />
        <kbd aria-hidden="true">/</kbd>
      </label>
      <div className="home-filters" role="group" aria-label="Filter papers">
        {([["all", "All papers"], ["mapped", "Mapped"], ["unmapped", "Unmapped"]] as const).map(([value, label]) =>
          <button type="button" key={value} aria-pressed={filter === value} onClick={() => setFilter(value)}>{label}</button>)}
      </div>
      <label className="home-sort"><span>Sort</span><select aria-label="Sort papers" value={sort} onChange={(event) => setSort(event.target.value)}><option value="title">Title A–Z</option><option value="newest">Newest first</option></select></label>
      <p className="home-counts" role="status">{visible.length === papers.length
        ? <><strong>{papers.length}</strong> {papers.length === 1 ? "paper" : "papers"}<span aria-hidden="true"> / </span><strong>{mapped}</strong> mapped</>
        : <><strong>{visible.length}</strong> of <strong>{papers.length}</strong> papers</>}</p>
      <AddMenu onImportUrl={onImport} onUpload={onUpload} />
    </div>

    {visible.length > 0 ? <ul className="paper-grid" ref={gridRef} aria-label="Papers">
      {visible.map((paper) => <li key={paper.id} style={{ "--paper-accent": paperAccent(paper.id) } as CSSProperties}>
        <button type="button" className={`paper-card ${paper.id === selectedId ? "is-active" : ""} ${paper.status.state === "ready" ? "is-mapped" : ""}`}
          data-paper-id={paper.id} tabIndex={paper.id === selectedId ? 0 : -1}
          aria-labelledby={`card-title-${paper.id}`} onFocus={() => onActive(paper.id)} onClick={() => onSelect(paper.id)}>
          <PaperPreview url={api.source(paper.id)} title={paper.metadata.title} />
          <span className="paper-card-caption">
          <span className="paper-card-top" title={`${paper.metadata.authors.join(", ")} — ${paperStatus(paper)}`}>
            <span className="paper-card-authors">{authorSurnames(paper.metadata.authors) || "Unknown"}</span>
            <span className="paper-card-year">{paper.metadata.year ?? "n.d."}</span>
          </span>
          <TitleLines id={`card-title-${paper.id}`} title={paper.metadata.title} />
          </span>
        </button>
      </li>)}
    </ul> : <div className="home-empty">
      <h2>{papers.length === 0 ? "Room for your next idea." : "No papers match this view."}</h2>
      <p>{papers.length === 0 ? "Use Add to import or upload a paper." : "Try another title, author, year, or filter."}</p>
      {papers.length > 0 && <button type="button" onClick={() => { onQuery(""); setFilter("all"); }}>Show all papers</button>}
    </div>}
  </section>;
}
