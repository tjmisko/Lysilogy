import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";

import { spatialNeighbor, type Direction } from "../lib/spatialNavigation";
import type { PaperOverview } from "../types";
import "./HomePage.css";

type HomePageProps = {
  name: string;
  papers: PaperOverview[];
  query: string;
  onQuery: (query: string) => void;
  onSelect: (id: string) => void;
  onImport: () => void;
  keyboardEnabled: boolean;
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

export function HomePage({ name, papers, query, onQuery, onSelect, onImport, keyboardEnabled }: HomePageProps) {
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

  useEffect(() => {
    if (!keyboardEnabled) return;
    const search = (event: globalThis.KeyboardEvent): void => {
      if (event.key !== "/" || event.ctrlKey || event.metaKey || event.altKey) return;
      const target = event.target;
      if (target instanceof HTMLElement && (target.matches("input, textarea, select") || target.isContentEditable)) return;
      event.preventDefault();
      searchRef.current?.focus();
    };
    window.addEventListener("keydown", search);
    return () => window.removeEventListener("keydown", search);
  }, [keyboardEnabled]);

  const moveInGrid = (event: KeyboardEvent<HTMLUListElement>): void => {
    if (!keyboardEnabled || event.ctrlKey || event.metaKey || event.altKey) return;
    const directions: Partial<Record<string, Direction>> = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };
    const direction = directions[event.key];
    if (direction === undefined || !(event.target instanceof HTMLButtonElement)) return;
    const cards = Array.from(gridRef.current?.querySelectorAll<HTMLButtonElement>(".paper-card") ?? []);
    const origin = cards.indexOf(event.target);
    if (origin < 0) return;
    event.preventDefault();
    const next = spatialNeighbor(cards.map((card, section) => {
      const { left, right, top, bottom } = card.getBoundingClientRect();
      return { left, right, top, bottom, section };
    }), origin, direction);
    if (next !== null) cards[next]?.focus();
  };

  return <section className="home-page" aria-labelledby="home-title">
    <header className="home-intro">
      <div>
        <p className="home-eyebrow">{name}</p>
        <h1 id="home-title">The reading room<span aria-hidden="true">.</span></h1>
      </div>
      <p className="home-counts"><strong>{papers.length}</strong> {papers.length === 1 ? "paper" : "papers"}<span aria-hidden="true"> / </span><strong>{mapped}</strong> mapped</p>
    </header>

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
    </div>

    <div className="home-results-line"><span role="status">{visible.length === papers.length ? "On the shelves" : `${visible.length} of ${papers.length} papers`}</span><button type="button" onClick={onImport}>Add a paper <span aria-hidden="true">↗</span></button></div>

    {visible.length > 0 ? <ul className="paper-grid" ref={gridRef} onKeyDown={moveInGrid} aria-label="Papers">
      {visible.map((paper) => <li key={paper.id} style={{ "--paper-accent": paperAccent(paper.id) } as CSSProperties}>
        <button type="button" className={`paper-card ${paper.status.state === "ready" ? "is-mapped" : ""}`} aria-labelledby={`card-title-${paper.id}`} onClick={() => onSelect(paper.id)}>
          <span className="paper-card-top"><span className={`paper-card-status state-${paper.status.state}`}>{paperStatus(paper)}</span><span>{paper.metadata.year ?? "Undated"}</span></span>
          <h2 id={`card-title-${paper.id}`} title={paper.metadata.title}>{paper.metadata.title}</h2>
          <p className="paper-card-authors" title={paper.metadata.authors.join(", ")}>{paper.metadata.authors.join(", ") || "Author not listed"}</p>
          {paper.one_line_summary !== null && paper.status.state === "ready" && <p className="paper-card-summary">{paper.one_line_summary}</p>}
          <span className="paper-card-bottom"><span>{paper.status.state === "ready" ? "Explore paper" : "Read paper"}</span><span aria-hidden="true">↗</span></span>
        </button>
      </li>)}
    </ul> : <div className="home-empty">
      <h2>{papers.length === 0 ? "Room for your next idea." : "No papers match this view."}</h2>
      <p>{papers.length === 0 ? "Add a paper to start your reading library." : "Try another title, author, year, or filter."}</p>
      {papers.length === 0 ? <button type="button" onClick={onImport}>Add a paper ↗</button> : <button type="button" onClick={() => { onQuery(""); setFilter("all"); }}>Show all papers</button>}
    </div>}
  </section>;
}
