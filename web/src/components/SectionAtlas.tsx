import { useEffect, useRef, type CSSProperties } from "react";

import type { LayoutSentence, PaperAnalysis, PaperMap, PaperSection } from "../types";
import { SourceMap } from "./SourceMap";

type AtlasProps = {
  analysis: PaperAnalysis;
  activeIndex: number;
  onActiveIndex: (index: number) => void;
  onOpen: (section: PaperSection, index: number) => void;
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
  onOpenPage: (page: number) => void;
  onToggleHighlight: (start: LayoutSentence, end?: LayoutSentence) => void;
  onClarifySentence: (text: string, page: number) => void;
};

type TileStyle = CSSProperties & {
  "--tile-width": number;
  "--tile-height": number;
};

const FAMILY_LABELS = [
  ["context", "Context"],
  ["question", "Question"],
  ["method", "Method"],
  ["evidence", "Evidence"],
  ["interpretation", "Meaning"],
  ["caveat", "Caveat"],
  ["reference", "Reference"],
] as const;

export function SectionAtlas({
  analysis,
  activeIndex,
  onActiveIndex,
  onOpen,
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
  onOpenPage,
  onToggleHighlight,
  onClarifySentence,
}: AtlasProps) {
  const tileRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const mounted = useRef(false);

  useEffect(() => {
    if (!mounted.current) {
      mounted.current = true;
      return;
    }
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && focused.closest(".source-map") !== null) return;
    const tile = tileRefs.current[activeIndex];
    if (tile === undefined || tile === null) return;
    tile.focus({ preventScroll: true });
    tile.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }, [activeIndex]);

  return (
    <section className="atlas-shell" aria-label="Paper overview">
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
          onOpenPage={onOpenPage}
          onToggleHighlight={onToggleHighlight}
          onClarify={onClarifySentence}
        />
      )}
      <section className="conceptual-atlas" aria-labelledby="conceptual-atlas-heading">
        <header className="conceptual-atlas-header">
          <span className="eyebrow">Conceptual weight</span>
          <h2 id="conceptual-atlas-heading">The argument by emphasis</h2>
        </header>
        <div className="atlas-intro">
          <div className="thesis-mark" aria-hidden="true">
            ∴
          </div>
          <div>
            <span className="eyebrow">Central claim</span>
            <p>{analysis.thesis}</p>
          </div>
        </div>
        <div className="atlas-legend" aria-label="Tile color key">
          {FAMILY_LABELS.map(([family, label]) => (
            <span key={family}>
              <i data-family={family} /> {label}
            </span>
          ))}
        </div>
        <div className="section-atlas" role="list">
          {analysis.sections.map((section, index) => {
            const style: TileStyle = {
              "--tile-width": Math.max(1, Math.min(4, section.tile_width)),
              "--tile-height": Math.max(1, Math.min(2, section.tile_height)),
            };
            return (
              <button
                key={section.id}
                ref={(node) => {
                  tileRefs.current[index] = node;
                }}
                type="button"
                role="listitem"
                className={`section-tile ${index === activeIndex ? "is-active" : ""}`}
                data-family={section.family}
                style={style}
                tabIndex={index === activeIndex ? 0 : -1}
                aria-label={`${section.title}, pages ${section.pages.start} to ${section.pages.end}. ${section.summary}`}
                onFocus={() => onActiveIndex(index)}
                onMouseEnter={() => onActiveIndex(index)}
                onClick={() => onOpen(section, index)}
              >
                <span className="tile-index">{String(index + 1).padStart(2, "0")}</span>
                <span className="tile-pages">
                  p. {section.pages.start}
                  {section.pages.end === section.pages.start ? "" : `–${section.pages.end}`}
                </span>
                <strong>{section.title}</strong>
                <span className="tile-summary">{section.summary}</span>
                <span className="tile-action">
                  Open digest <kbd>↵</kbd>
                </span>
              </button>
            );
          })}
        </div>
        <p className="atlas-footnote">
          Tile area follows conceptual weight, not page count. Hover or focus for the short reading;
          open for quotes and context.
        </p>
      </section>
    </section>
  );
}
