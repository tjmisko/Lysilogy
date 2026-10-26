import { useCallback, useMemo } from "react";

import { timelineYearMove } from "../../lib/listViewNav";
import { GRAPH_NODE, graphModel, graphPlacements, statusGlyph, type ItemRow } from "../../lib/listViews";
import { useColumnKeys } from "./ColumnKeys";
import type { ListViewProps } from "./props";
import "./Graph.css";

const TITLE_CHARS = 30;

function clip(text: string): string {
  return text.length > TITLE_CHARS ? `${text.slice(0, TITLE_CHARS - 1)}…` : text;
}

/** View 4: members as a layered DAG; prerequisites solid, derived citations dashed (key L). */
export function Graph(props: ListViewProps) {
  const { detail, selectedId, onSelect, onOpen, keyboardEnabled } = props;
  const model = useMemo(() => graphModel(detail), [detail]);
  const layout = useMemo(() => graphPlacements(model), [model]);
  const columns = useMemo(() => model.layers.map((layer) => layer.map((row) => row.id)), [model]);
  const move = useCallback((delta: -1 | 1) => timelineYearMove(columns, selectedId, delta), [columns, selectedId]);
  useColumnKeys(keyboardEnabled, move, onSelect);

  const at = useMemo(() => new Map(layout.placements.map((place) => [place.id, place])), [layout]);
  const topicsOf = useMemo(() => {
    const found = new Map<string, string[]>();
    for (const topic of model.topics) for (const id of topic.rowIds) found.set(id, [...(found.get(id) ?? []), topic.label]);
    return found;
  }, [model.topics]);

  if (model.layers.length === 0) {
    return <div className="list-empty" data-empty="graph"><p>Nothing to draw yet. Press <kbd>a</kbd> to add a paper.</p></div>;
  }

  const { width, height } = GRAPH_NODE;
  const node = (row: ItemRow) => {
    const place = at.get(row.id);
    if (place === undefined) return null;
    const labels = topicsOf.get(row.id) ?? [];
    const linked = selectedId !== null && model.links.some((link) => (link.from === selectedId && link.to === row.id) || (link.to === selectedId && link.from === row.id));
    return <g
      key={row.id}
      className={`list-graph-node ${selectedId === row.id ? "is-selected" : ""} ${linked ? "is-linked" : ""}`}
      transform={`translate(${place.x} ${place.y})`}
      data-node-id={row.id}
      data-kind={row.kind}
      data-status={row.status}
      onClick={() => onSelect(row.id)}
      onDoubleClick={() => onOpen(row.id)}
    >
      <title>{[row.title, ...labels].join(" · ")}</title>
      <rect className="list-graph-card" width={width} height={height} rx={6} />
      <text className="list-graph-glyph" x={10} y={18}>{row.kind === "claim" ? "?" : statusGlyph(row.status)}</text>
      <text className="list-graph-title" x={28} y={18}>{clip(row.title)}</text>
      <text className="list-graph-topics" x={28} y={36}>{clip(labels.join(" · "))}</text>
    </g>;
  };

  return <div className="list-graph" aria-label="Graph">
    <svg width={layout.width} height={layout.height} role="group" aria-label="Reading graph">
      <defs>
        <marker id="list-graph-arrow" viewBox="0 0 8 8" refX={7} refY={4} markerWidth={7} markerHeight={7} orient="auto">
          <path d="M0 0 L8 4 L0 8 z" className="list-graph-arrowhead" />
        </marker>
      </defs>
      {model.links.map((link) => {
        const from = at.get(link.from);
        const to = at.get(link.to);
        if (from === undefined || to === undefined) return null;
        const forward = to.x > from.x;
        const x1 = forward ? from.x + width : from.x;
        const x2 = forward ? to.x : to.x + width;
        const y1 = from.y + height / 2;
        const y2 = to.y + height / 2;
        const bend = forward ? (x2 - x1) / 2 : 60;
        const touches = selectedId === link.from || selectedId === link.to;
        return <path
          key={`${link.kind}:${link.from}:${link.to}`}
          className={`list-graph-edge is-${link.kind} ${touches ? "is-linked" : ""}`}
          data-edge-kind={link.kind}
          d={`M${x1} ${y1} C${x1 + bend} ${y1} ${x2 - bend} ${y2} ${x2} ${y2}`}
          markerEnd="url(#list-graph-arrow)"
        />;
      })}
      {model.layers.flat().map(node)}
    </svg>
    {model.topics.length > 0 && <p className="list-graph-legend">
      {model.topics.map((topic) => <span key={topic.id}>{topic.label} · {topic.rowIds.length}</span>)}
    </p>}
  </div>;
}
