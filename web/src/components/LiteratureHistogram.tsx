import { useEffect, useMemo, useRef } from "react";

import { acquisitionStatusText } from "../lib/acquisition";
import { api } from "../lib/api";
import {
  COLLAPSED_AUTHOR_LIMIT, EXPANDED_AUTHOR_LIMIT, authorLine, buildHistogramGrid, histogramEntries,
  layoutHistogram, positionOf, reconcileCursor, rememberCursor, type CursorMemory,
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

  // Keep the cursor on a real card: remember it while it is valid, reconcile when it vanishes.
  const memory = useRef<CursorMemory | null>(null);
  useEffect(() => {
    if (effectiveFocus !== null) {
      memory.current = rememberCursor(grid, effectiveFocus);
      return;
    }
    if (grid.columns.length === 0) return;
    const next = reconcileCursor(grid, memory.current);
    if (next !== null) onFocusWork(next);
  }, [effectiveFocus, grid, onFocusWork]);

  const chips = (neighbors?.clusters ?? []).filter((cluster) => cluster.key !== "hidden");
  const centerAuthors = authorLine(centerWork.display_authors, COLLAPSED_AUTHOR_LIMIT);
  const centerYear = metaYearText(centerWork);
  const columnBoxes = layout.axis.filter((slot) => slot.kind === "column" || slot.kind === "unknown");
  const showEmptyStatus = neighbors !== null && entries.length === 0 && error === null;

  return (
    <section
      className="literature-histogram"
      aria-label="References by publication year"
      data-card-count={layout.cards.length}
      data-focused-id={effectiveFocus ?? undefined}
    >
      <div className="literature-histogram-seed">
        <article className="literature-histogram-seed-card">
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

      <div className="literature-histogram-scroll">
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

      {neighbors === null && error === null && (
        <p className="literature-status" role="status">Loading references by year…</p>
      )}
      {error !== null && <p className="literature-status" role="alert">{error}</p>}
      {showEmptyStatus && <p className="literature-status" role="status">No references are known for this paper.</p>}
    </section>
  );
}
