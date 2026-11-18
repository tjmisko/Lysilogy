import { useEffect, useRef } from "react";

import { groupedRangeLabel, outlineLabel, outlineRows, roleLabel } from "../lib/readerOutput";
import type { LayoutSentence, PaperAnalysis, PaperMap, PaperSection } from "../types";
import { SourceMap } from "./SourceMap";

type AtlasProps = {
  analysis: PaperAnalysis;
  activeIndex: number;
  onOpen: (section: PaperSection, index: number) => void;
  /** Logical units on the page map, or the printed outline as a list. */
  mode: "units" | "outline";
  onMode: (mode: "units" | "outline") => void;
  activeOutline: number;
  onOpenOutline: (index: number) => void;
  onOpenPage: (page: number) => void;
  sourceUrl: string;
  paperTitle: string;
  paperMap: PaperMap | null;
  mapLoading: boolean;
  darkInk: boolean;
  showAi: boolean;
  showUser: boolean;
  markMode: boolean;
  onShowAi: () => void;
  onShowUser: () => void;
  onMarkMode: () => void;
  onToggleHighlight: (start: LayoutSentence, end?: LayoutSentence) => void;
  onClarifySentence: (text: string, page: number) => void;
};

function SegmentationToggle({ mode, onMode, outlineCount, unitCount }: {
  mode: "units" | "outline";
  onMode: (mode: "units" | "outline") => void;
  outlineCount: number;
  unitCount: number;
}) {
  return (
    <div className="segmentation-toggle" role="group" aria-label="Overview segmentation">
      <button type="button" className={mode === "units" ? "is-active" : ""} aria-pressed={mode === "units"}
        onClick={() => onMode("units")}>
        Logical units <span>{unitCount}</span>
      </button>
      <button type="button" className={mode === "outline" ? "is-active" : ""} aria-pressed={mode === "outline"}
        disabled={outlineCount === 0} title={outlineCount === 0 ? "This paper prints no headings" : undefined}
        onClick={() => onMode("outline")}>
        Outline <span>{outlineCount}</span>
      </button>
      <kbd title="Switch segmentation">O</kbd>
    </div>
  );
}

/**
 * The paper's printed headings as a hierarchy, each with the logical unit that
 * groups it, so the two segmentations can be read side by side.
 */
function OutlineView({ analysis, active, onOpen, onOpenPage }: {
  analysis: PaperAnalysis;
  active: number;
  onOpen: (index: number) => void;
  onOpenPage: (page: number) => void;
}) {
  const outline = analysis.outline ?? [];
  const rows = outlineRows(outline, analysis.sections);
  const listRef = useRef<HTMLOListElement>(null);

  useEffect(() => {
    const row = listRef.current?.querySelector<HTMLElement>(`[data-outline-index="${active}"] .outline-entry`);
    if (row == null) return;
    if (listRef.current?.contains(document.activeElement) || document.activeElement === document.body
      || document.activeElement?.closest(".main-stage") != null) {
      row.focus({ preventScroll: true });
    }
    row.scrollIntoView({ block: "nearest" });
  }, [active]);

  return (
    <section className="outline-view" aria-label="Printed outline">
      <ol ref={listRef} className="outline-list">
        {rows.map((row) => {
          const unit = row.unit === null ? null : analysis.sections[row.unit];
          const startsUnit = row.unit !== null && rows[row.index - 1]?.unit !== row.unit;
          return (
            <li key={row.index} data-outline-index={row.index} data-role={row.entry.role ?? "main"}
              data-family={unit?.family} className={`${startsUnit ? "starts-unit" : ""} ${row.index === active ? "is-active" : ""}`}>
              <button type="button" className="outline-entry" style={{ paddingLeft: `${12 + row.depth * 20}px` }}
                tabIndex={row.index === active ? 0 : -1}
                onClick={() => onOpen(row.index)}
                aria-label={`${outlineLabel(row.entry)}, page ${row.entry.page}${unit == null ? "" : `, in unit ${unit.title}`}`}>
                <span className="outline-number">{row.entry.number ?? ""}</span>
                <span className="outline-title">{row.entry.title}</span>
                {(row.entry.role ?? "main") !== "main" && <span className="outline-role">{roleLabel(row.entry.role)}</span>}
              </button>
              <button type="button" className="outline-page" tabIndex={-1} onClick={() => onOpenPage(row.entry.page)}
                aria-label={`Open PDF page ${row.entry.page}`}>p. {row.entry.page}</button>
              <span className="outline-unit">{startsUnit && unit != null ? unit.title : ""}</span>
            </li>
          );
        })}
      </ol>
      <p className="outline-hint"><kbd>j</kbd>/<kbd>k</kbd> move · <kbd>↵</kbd> open the unit holding the heading · <kbd>O</kbd> back to logical units</p>
    </section>
  );
}

/** A list of the logical units with the printed headings each groups, shown under the page map. */
function UnitIndex({ analysis, activeIndex, onOpen }: {
  analysis: PaperAnalysis;
  activeIndex: number;
  onOpen: (section: PaperSection, index: number) => void;
}) {
  const outline = analysis.outline ?? [];
  if (outline.length === 0) return null;
  return (
    <ol className="unit-index" aria-label="Logical units and the printed sections they group">
      {analysis.sections.map((section, index) => {
        const range = groupedRangeLabel(section, outline);
        return (
          <li key={section.id} data-family={section.family} className={index === activeIndex ? "is-active" : ""}>
            <button type="button" tabIndex={-1} onClick={() => onOpen(section, index)}>
              <strong>{section.title}</strong>
              <span className="unit-index-range">{range ?? "No printed heading"}</span>
              {section.question != null && section.question !== "" && <span className="unit-index-question">{section.question}</span>}
            </button>
          </li>
        );
      })}
    </ol>
  );
}

export function SectionAtlas({
  analysis,
  activeIndex,
  onOpen,
  mode,
  onMode,
  activeOutline,
  onOpenOutline,
  onOpenPage,
  sourceUrl,
  paperTitle,
  paperMap,
  mapLoading,
  darkInk,
  showAi,
  showUser,
  markMode,
  onShowAi,
  onShowUser,
  onMarkMode,
  onToggleHighlight,
  onClarifySentence,
}: AtlasProps) {
  const outlineCount = analysis.outline?.length ?? 0;
  return (
    <section className="atlas-shell" aria-label="Paper overview" data-overview-mode={mode}>
      <SegmentationToggle mode={mode} onMode={onMode} outlineCount={outlineCount} unitCount={analysis.sections.length} />
      {mode === "outline" ? (
        <OutlineView analysis={analysis} active={activeOutline} onOpen={onOpenOutline} onOpenPage={onOpenPage} />
      ) : (
        <>
          {mapLoading && (
            <div className="source-map-loading"><span className="loader" /> Aligning PDF pages and evidence…</div>
          )}
          {paperMap !== null && (
            <SourceMap
              url={sourceUrl}
              title={paperTitle}
              analysis={analysis}
              paperMap={paperMap}
              activeSection={activeIndex}
              darkInk={darkInk}
              showAi={showAi}
              showUser={showUser}
              markMode={markMode}
              onShowAi={onShowAi}
              onShowUser={onShowUser}
              onMarkMode={onMarkMode}
              onOpenSection={onOpen}
              onToggleHighlight={onToggleHighlight}
              onClarify={onClarifySentence}
            />
          )}
          <UnitIndex analysis={analysis} activeIndex={activeIndex} onOpen={onOpen} />
        </>
      )}
    </section>
  );
}
