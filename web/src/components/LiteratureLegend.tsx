import type { LiteratureView } from "../lib/histogramLayout";
import type { GraphNeighbors, TagView } from "../types";

type LiteratureLegendProps = {
  graph: GraphNeighbors;
  placementCount: number;
  expanded: ReadonlySet<string>;
  clusterLabels: ReadonlyMap<string, string>;
  onToggleExpansion: (key: string) => void;
  tags: readonly TagView[];
  tagFilter: ReadonlySet<string>;
  onToggleTag: (tagId: string) => void;
  onClearTagFilter: () => void;
  error: string | null;
  view?: LiteratureView;
};

/** Index keys `1`..`9` address the first nine tags. */
export const TAG_KEY_LIMIT = 9;

function tagCounts(graph: GraphNeighbors): Map<string, number> {
  const counts = new Map<string, number>();
  for (const tagIds of Object.values(graph.work_tags ?? {})) {
    for (const tagId of new Set(tagIds)) counts.set(tagId, (counts.get(tagId) ?? 0) + 1);
  }
  return counts;
}

/** Keys, status line, tag filter chips, and expanded-cluster chips below the canvas. */
export function LiteratureLegend({
  graph, placementCount, expanded, clusterLabels, onToggleExpansion, tags, tagFilter, onToggleTag, onClearTagFilter, error, view = "radial",
}: LiteratureLegendProps) {
  const ranking = graph.ranking;
  const visible = ranking?.visible ?? graph.nodes.filter((node) => node.id !== graph.center).length;
  const total = ranking?.total ?? visible;
  const hiddenCount = ranking?.hidden ?? 0;
  const hiddenExpanded = expanded.has("hidden");
  const expandedKeys = [...expanded].filter((key) => key !== "hidden" || hiddenExpanded);
  const counts = tagCounts(graph);
  const filterNames = tags.filter((tag) => tagFilter.has(tag.id)).map((tag) => tag.name);
  return (
    <div className="literature-legend">
      <p className="literature-legend-row">
        <span><i className="literature-key is-local" /> held locally</span>
        <span><i className="literature-key" /> cited work</span>
        <span><i className="literature-line is-labeled" /> reason known</span>
        <span><i className="literature-line is-local-only" /> found in this paper only</span>
        <span className="literature-ranking" data-placement-count={placementCount}>
          Showing {visible} of {total} · {ranking?.pinned ?? 0} pinned · {hiddenCount} hidden
        </span>
        {hiddenCount > 0 && !hiddenExpanded && (
          <button type="button" className="literature-chip" onClick={() => onToggleExpansion("hidden")}>Hidden ({hiddenCount}) — show</button>
        )}
        {expandedKeys.map((key) => (
          <button key={key} type="button" className="literature-chip is-expanded" aria-label={`Collapse ${clusterLabels.get(key) ?? key}`} onClick={() => onToggleExpansion(key)}>
            {clusterLabels.get(key) ?? key} ×
          </button>
        ))}
        {view === "histogram"
          ? <span><kbd>h</kbd>/<kbd>l</kbd> year · <kbd>j</kbd>/<kbd>k</kbd> within year · <kbd>↵</kbd> open · <kbd>c</kbd> collapse · <kbd>v</kbd> radial view · <kbd>Esc</kbd> close</span>
          : <span><kbd>j</kbd>/<kbd>k</kbd> move · <kbd>↵</kbd> open or expand · <kbd>+</kbd>/<kbd>-</kbd> more/fewer · <kbd>c</kbd> collapse · <kbd>v</kbd> year view · <kbd>Esc</kbd> close</span>}
        {error !== null && <span className="literature-legend-error" role="alert">{error}</span>}
      </p>
      {(tags.length > 0 || tagFilter.size > 0) && (
        <p className="literature-legend-row literature-tag-legend" aria-label="Tag filters">
          {tags.map((tag, index) => {
            const pressed = tagFilter.has(tag.id);
            const count = counts.get(tag.id) ?? 0;
            return (
              <button
                key={tag.id}
                type="button"
                className={`literature-chip literature-tag-chip ${count === 0 && !pressed ? "is-empty" : ""}`}
                aria-pressed={pressed}
                data-tag-id={tag.id}
                onClick={() => onToggleTag(tag.id)}
              >
                <i className="literature-swatch" style={{ background: tag.color }} />
                {tag.name} · {count}
                {index < TAG_KEY_LIMIT && <kbd>{index + 1}</kbd>}
              </button>
            );
          })}
          {tagFilter.size > 0 && (
            <>
              <span className="literature-filtered" role="status">Filtered by: {filterNames.length > 0 ? filterNames.join(", ") : `${tagFilter.size} unknown tag${tagFilter.size === 1 ? "" : "s"}`}</span>
              <button type="button" className="literature-chip" onClick={onClearTagFilter}>Clear filters <kbd>0</kbd></button>
            </>
          )}
        </p>
      )}
    </div>
  );
}
