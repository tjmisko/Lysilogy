import { useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";

import { acquisitionStatusText } from "../lib/acquisition";
import { api } from "../lib/api";
import {
  COLLAPSED_AUTHOR_LIMIT, EXPANDED_AUTHOR_LIMIT, authorLine, buildHistogramGrid, directionForKey,
  histogramEntries, layoutHistogram, moveCursor, positionOf, reconcileCursor, rememberCursor,
  scrollToReveal, seedLinkPath, seedLinkTarget, type CursorMemory, type HistogramCard, type Point, type Rect,
} from "../lib/histogramLayout";
import type {
  AcquisitionsByWork, GraphNeighbors, LocalPapersByWork, MarkView, ProminenceView, Work,
} from "../types";
import { PaperPreview } from "./PaperPreview";
import "./LiteratureHistogram.css";

type LiteratureHistogramProps = {
  centerWork: Work;
  /** Null while the first fetch runs. */
  neighbors: GraphNeighbors | null;
  error: string | null;
  /** LiteratureGraph's selectedId. */
  focusedWorkId: string | null;
  markFor: (workId: string) => MarkView | null;
  prominence: Readonly<Record<string, ProminenceView>>;
  localPapers: LocalPapersByWork;
  acquisitions: AcquisitionsByWork;
  expandedClusters: ReadonlySet<string>;
  onToggleCluster: (key: string) => void;
  /** Move the cursor; the Work panel follows when open. */
  onFocusWork: (workId: string) => void;
  /** Focus and open the Work panel. */
  onActivateWork: (workId: string) => void;
};

function workTitle(work: Work): string {
  const title = work.display_title?.trim() ?? "";
  return title === "" ? `Untitled (${work.id.slice(0, 8)}…)` : title;
}

function yearText(year: number | null): string {
  return year === null ? "Year unknown" : String(year);
}

/** The Work's year as the grid reads it: valid years only, so "Year unknown" matches the lane. */
function metaYearText(work: Work): string {
  const year = work.display_year;
  const valid = year !== null && Number.isInteger(year) && year >= 1 && year <= 2200;
  return yearText(valid ? year : null);
}

function isEditableTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement;
}

function isActivatable(target: EventTarget | null): boolean {
  return target instanceof HTMLButtonElement || target instanceof HTMLAnchorElement;
}

/** The element's box in the origin's coordinates; width and height may be overridden (client box). */
function boxRelativeTo(element: Element, origin: DOMRect, size?: { width: number; height: number }): Rect {
  const box = element.getBoundingClientRect();
  return {
    x: box.left - origin.left, y: box.top - origin.top,
    width: size?.width ?? box.width, height: size?.height ?? box.height,
  };
}

type LinkMeasure = { scroll: { left: number; top: number }; viewportBox: Rect; seedAnchor: Point };

const EMPTY_MEASURE: LinkMeasure = {
  scroll: { left: 0, top: 0 }, viewportBox: { x: 0, y: 0, width: 0, height: 0 }, seedAnchor: { x: 0, y: 0 },
};

/**
 * The one edge: a curve from the seed card to the focused card, in section coordinates.
 * Owns the scroll and size tracking so scrolling re-renders only this overlay.
 */
function SeedLink({ sectionRef, scrollRef, seedRef, focusedCard }: {
  sectionRef: RefObject<HTMLElement | null>;
  scrollRef: RefObject<HTMLDivElement | null>;
  seedRef: RefObject<HTMLElement | null>;
  focusedCard: HistogramCard | null;
}) {
  const [measure, setMeasure] = useState<LinkMeasure>(EMPTY_MEASURE);

  // A passive effect: a parent's ref attaches after its children's layout effects run.
  useEffect(() => {
    const section = sectionRef.current;
    const scroller = scrollRef.current;
    const seed = seedRef.current;
    if (section === null || scroller === null || seed === null) return;
    const readAll = (): void => {
      const origin = section.getBoundingClientRect();
      const seedBox = boxRelativeTo(seed, origin);
      setMeasure({
        scroll: { left: scroller.scrollLeft, top: scroller.scrollTop },
        viewportBox: boxRelativeTo(scroller, origin, { width: scroller.clientWidth, height: scroller.clientHeight }),
        seedAnchor: { x: seedBox.x + seedBox.width / 2, y: seedBox.y + seedBox.height },
      });
    };
    let frame = 0;
    const onScroll = (): void => {
      if (frame !== 0) return;
      frame = window.requestAnimationFrame(() => {
        frame = 0;
        setMeasure((current) => ({ ...current, scroll: { left: scroller.scrollLeft, top: scroller.scrollTop } }));
      });
    };
    readAll();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    const observer = new ResizeObserver(readAll);
    observer.observe(section);
    observer.observe(scroller);
    observer.observe(seed);
    return () => {
      scroller.removeEventListener("scroll", onScroll);
      observer.disconnect();
      if (frame !== 0) window.cancelAnimationFrame(frame);
    };
  }, [scrollRef, sectionRef, seedRef]);

  const target = focusedCard === null ? null : seedLinkTarget(focusedCard, measure.scroll, measure.viewportBox);
  return (
    <svg className="literature-histogram-link-layer" aria-hidden="true">
      {target !== null && (
        <path className="literature-histogram-link" d={seedLinkPath(measure.seedAnchor, target)} />
      )}
    </svg>
  );
}

export function LiteratureHistogram({
  centerWork, neighbors, error, focusedWorkId, markFor, prominence, localPapers, acquisitions,
  expandedClusters, onToggleCluster, onFocusWork, onActivateWork,
}: LiteratureHistogramProps) {
  const entries = useMemo(
    () => neighbors === null ? [] : histogramEntries(
      neighbors,
      (workId) => markFor(workId)?.mark ?? null,
      (workId) => prominence[workId]?.score,
    ),
    [markFor, neighbors, prominence],
  );
  const grid = useMemo(() => buildHistogramGrid(entries), [entries]);
  const effectiveFocus = positionOf(grid, focusedWorkId) === null ? null : focusedWorkId;
  const layout = useMemo(
    () => layoutHistogram(grid, effectiveFocus, (workId) => (localPapers[workId]?.length ?? 0) > 0),
    [effectiveFocus, grid, localPapers],
  );
  const worksById = useMemo(
    () => new Map((neighbors?.nodes ?? []).map((work) => [work.id, work])),
    [neighbors],
  );

  const sectionRef = useRef<HTMLElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const seedRef = useRef<HTMLElement | null>(null);
  const revealPending = useRef(true);
  // Resize-driven reveals jump: a smooth scroll still in flight would otherwise finish on a stale target.
  const revealInstant = useRef(false);
  const lastPointer = useRef<{ x: number; y: number } | null>(null);

  // Keep the cursor on a real card: remember it while it is valid, reconcile when it vanishes.
  const memory = useRef<CursorMemory | null>(null);
  useEffect(() => {
    if (effectiveFocus !== null) {
      memory.current = rememberCursor(grid, effectiveFocus);
      return;
    }
    if (grid.columns.length === 0) return;
    const next = reconcileCursor(grid, memory.current);
    if (next === null) return;
    revealPending.current = true;
    onFocusWork(next);
  }, [effectiveFocus, grid, onFocusWork]);

  // Keyboard moves, reconciliation and the first focus scroll the card into view; hover never does.
  // A resized scroll area (the Work panel opening, a window resize) re-reveals the focused card.
  const [scrollSize, setScrollSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    const scroller = scrollRef.current;
    if (scroller === null) return;
    const observer = new ResizeObserver(() => {
      revealPending.current = true;
      revealInstant.current = true;
      setScrollSize({ width: scroller.clientWidth, height: scroller.clientHeight });
    });
    observer.observe(scroller);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (!revealPending.current || effectiveFocus === null) return;
    const scroller = scrollRef.current;
    const card = layout.cards.find((candidate) => candidate.id === effectiveFocus);
    if (scroller === null || card === undefined) return;
    revealPending.current = false;
    const instant = revealInstant.current;
    revealInstant.current = false;
    const target = scrollToReveal(card, {
      left: scroller.scrollLeft, top: scroller.scrollTop,
      width: scroller.clientWidth, height: scroller.clientHeight,
    });
    if (!instant && target.left === scroller.scrollLeft && target.top === scroller.scrollTop) return;
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    scroller.scrollTo({ left: target.left, top: target.top, behavior: reduceMotion || instant ? "auto" : "smooth" });
  }, [effectiveFocus, layout, scrollSize]);

  // The year view's keys. LiteratureGraph skips these so they move the cursor even with the panel open.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || isEditableTarget(event.target)) return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const direction = directionForKey(event.key);
      if (direction !== null) {
        event.preventDefault();
        const next = moveCursor(grid, effectiveFocus, direction);
        if (next === null || next === effectiveFocus) return;
        revealPending.current = true;
        onFocusWork(next);
        return;
      }
      if (event.key !== "Enter" && event.key !== "o") return;
      if (effectiveFocus === null || isActivatable(event.target)) return;
      event.preventDefault();
      onActivateWork(effectiveFocus);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [effectiveFocus, grid, onActivateWork, onFocusWork]);

  const focusedCard = layout.cards.find((card) => card.id === effectiveFocus) ?? null;
  const chips = (neighbors?.clusters ?? []).filter((cluster) => cluster.key !== "hidden");
  const centerAuthors = authorLine(centerWork.display_authors, COLLAPSED_AUTHOR_LIMIT);
  const centerYear = metaYearText(centerWork);
  const columnBoxes = layout.axis.filter((slot) => slot.kind === "column" || slot.kind === "unknown");
  const showEmptyStatus = neighbors !== null && entries.length === 0 && error === null;

  return (
    <section
      ref={sectionRef}
      className="literature-histogram"
      aria-label="References by publication year"
      data-card-count={layout.cards.length}
      data-focused-id={effectiveFocus ?? undefined}
    >
      <div className="literature-histogram-seed">
        <article ref={seedRef} className="literature-histogram-seed-card">
          <div className="literature-histogram-seed-head">
            <span className="literature-histogram-kicker">This paper</span>
            <span className="literature-histogram-year">{centerYear}</span>
          </div>
          <h3>{workTitle(centerWork)}</h3>
          {centerAuthors !== null && <p className="literature-histogram-authors">{centerAuthors}</p>}
        </article>
        <div className="literature-histogram-chips">
          {chips.map((cluster) => (
            <button
              key={cluster.key}
              type="button"
              className="literature-chip"
              data-cluster-key={cluster.key}
              aria-pressed={expandedClusters.has(cluster.key)}
              onClick={() => onToggleCluster(cluster.key)}
            >
              {cluster.label} · {cluster.count} — show
            </button>
          ))}
        </div>
      </div>

      <div ref={scrollRef} className="literature-histogram-scroll">
        <div className="literature-histogram-content" style={{ width: layout.width, height: layout.height }}>
          <div className="literature-histogram-axis">
            {layout.axis.map((slot) => (
              <span
                key={`tick-${slot.x}`}
                className="literature-histogram-tick"
                data-slot-kind={slot.kind}
                data-year={slot.kind === "column" ? slot.label : undefined}
                title={slot.title}
                style={{ left: slot.x, width: slot.width }}
              >
                {slot.label}{slot.count > 0 ? ` · ${slot.count}` : ""}
              </span>
            ))}
          </div>
          {layout.axis.map((slot) => (
            <div
              key={`grid-${slot.x}`}
              className="literature-histogram-gridline"
              data-slot-kind={slot.kind}
              style={{ left: slot.x }}
            />
          ))}
          {columnBoxes.map((slot) => (
            <div
              key={`column-${slot.x}`}
              className="literature-histogram-column"
              role="list"
              aria-label={`${slot.label}, ${slot.count} ${slot.count === 1 ? "reference" : "references"}`}
              data-slot-kind={slot.kind}
              data-year={slot.kind === "column" ? slot.label : undefined}
              style={{ left: slot.x, width: slot.width }}
            />
          ))}
          {layout.cards.map((card) => {
            const work = worksById.get(card.id);
            if (work === undefined) return null;
            const mark = markFor(card.id)?.mark ?? null;
            const held = localPapers[card.id] ?? [];
            const heldPaper = held[0];
            const focused = card.id === effectiveFocus;
            const column = grid.columns[card.columnIndex];
            const authors = authorLine(
              work.display_authors, card.expanded ? EXPANDED_AUTHOR_LIMIT : COLLAPSED_AUTHOR_LIMIT,
            );
            const brief = acquisitions[card.id];
            const score = prominence[card.id]?.score;
            return (
              <article
                key={card.id}
                className="literature-histogram-card"
                role="listitem"
                aria-current={focused ? "true" : undefined}
                data-work-id={card.id}
                data-focused={focused ? "true" : "false"}
                data-expanded={card.expanded ? "true" : "false"}
                data-mark={mark ?? "none"}
                data-local={heldPaper === undefined ? "false" : "true"}
                data-year={column?.year ?? "unknown"}
                data-row={card.row}
                style={{ left: card.x, top: card.y, width: card.width, height: card.height }}
                onClick={() => onActivateWork(card.id)}
                onPointerMove={(event) => {
                  // Only a real pointer move retargets; scrolling under a still pointer must not.
                  const previous = lastPointer.current;
                  lastPointer.current = { x: event.clientX, y: event.clientY };
                  if (previous?.x === event.clientX && previous.y === event.clientY) return;
                  if (card.id !== effectiveFocus) onFocusWork(card.id);
                }}
              >
                {heldPaper !== undefined && <i className="literature-histogram-held" title="Held locally" />}
                {card.expanded && (
                  heldPaper !== undefined
                    ? (
                      <div className="literature-histogram-thumb">
                        <PaperPreview url={api.source(heldPaper.paper_id)} title={workTitle(work)} />
                      </div>
                    )
                    : (
                      <div className="literature-histogram-placeholder">
                        {brief === undefined ? "No local copy" : acquisitionStatusText(brief)}
                      </div>
                    )
                )}
                <h4 className="literature-histogram-title">{workTitle(work)}</h4>
                {authors !== null && <p className="literature-histogram-authors">{authors}</p>}
                {card.expanded && (
                  <p className="literature-histogram-meta">
                    {metaYearText(work)} · {work.kind} · score {(score ?? 0).toFixed(2)}
                  </p>
                )}
              </article>
            );
          })}
        </div>
      </div>

      <SeedLink sectionRef={sectionRef} scrollRef={scrollRef} seedRef={seedRef} focusedCard={focusedCard} />

      {neighbors === null && error === null && (
        <p className="literature-status" role="status">Loading references by year…</p>
      )}
      {error !== null && <p className="literature-status" role="alert">{error}</p>}
      {showEmptyStatus && <p className="literature-status" role="status">No references are known for this paper.</p>}
    </section>
  );
}
