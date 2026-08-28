import type { LayoutSentence, PaperAnalysis, PaperMap, PaperSection } from "../types";
import { SourceMap } from "./SourceMap";

type AtlasProps = {
  analysis: PaperAnalysis;
  activeIndex: number;
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

export function SectionAtlas({
  analysis,
  activeIndex,
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
    </section>
  );
}
