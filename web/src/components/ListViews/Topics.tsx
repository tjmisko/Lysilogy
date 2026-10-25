import { useCallback, useMemo, type CSSProperties } from "react";

import { groupCursorId } from "../../lib/listViewModel";
import { topicLayerMove } from "../../lib/listViewNav";
import { countsLabel, statusGlyph, topicLayers, UNFILED_KEY, type ItemRow } from "../../lib/listViews";
import { useColumnKeys } from "./ColumnKeys";
import type { ListViewProps } from "./props";
import "./ListColumns.css";

function WorkCard({ row, props }: { row: ItemRow; props: ListViewProps }) {
  return (
    <li
      className={`list-col-work ${props.selectedId === row.id ? "is-selected" : ""}`}
      data-node-id={row.id}
      data-kind={row.kind}
      data-status={row.status}
      onClick={() => props.onSelect(row.id)}
      onDoubleClick={() => props.onOpen(row.id)}
    >
      <span className="list-col-glyph" aria-hidden="true">{row.kind === "claim" ? "?" : statusGlyph(row.status)}</span>
      <span className="list-col-title">{row.title}</span>
      {row.year !== null && <span className="list-col-year">{row.year}</span>}
    </li>
  );
}

/** View 2: topics in longest-path layers, left to right, each with its works. */
export function Topics(props: ListViewProps) {
  const { detail, viewState, selectedId, onSelect, keyboardEnabled } = props;
  const model = useMemo(() => topicLayers(detail, viewState.folded), [detail, viewState.folded]);
  const move = useCallback((delta: -1 | 1) => topicLayerMove(model, selectedId, delta), [model, selectedId]);
  useColumnKeys(keyboardEnabled, move, onSelect);

  const names = useMemo(() => new Map(detail.nodes.filter((node) => node.kind === "topic").map((node) => [node.id, node.label ?? "Untitled"])), [detail.nodes]);
  const after = useMemo(() => {
    const found = new Map<string, string[]>();
    for (const edge of detail.edges) {
      if (edge.review !== "accepted" || edge.kind === "covers" || edge.from === edge.to) continue;
      const from = names.get(edge.from);
      if (from === undefined || !names.has(edge.to)) continue;
      found.set(edge.to, [...(found.get(edge.to) ?? []), from]);
    }
    return found;
  }, [detail.edges, names]);

  if (model.layers.length === 0 && model.unfiled === null) {
    return <div className="list-empty"><p>No topics yet. Press <kbd>O</kbd> to add one.</p></div>;
  }
  const unfiledCursor = groupCursorId(UNFILED_KEY);
  return <div className="list-columns" aria-label="Topics">
    {model.layers.map((layer, index) => <section key={index} className="list-col" data-layer={index}>
      <h3 className="list-col-head">Layer {index + 1}</h3>
      {layer.map((entry) => {
        const color = entry.topic.label === null ? undefined : props.tagColors[entry.topic.label.toLowerCase()];
        const prior = after.get(entry.topic.id);
        return <div key={entry.topic.id} className="list-col-topic" data-folded={entry.folded ? "true" : "false"}>
          <p
            className={`list-group-head ${selectedId === entry.topic.id ? "is-selected" : ""}`}
            data-node-id={entry.topic.id}
            style={color === undefined ? undefined : ({ "--topic-color": color } as CSSProperties)}
            onClick={() => onSelect(entry.topic.id)}
            onDoubleClick={() => props.onOpen(entry.topic.id)}
          >
            <span aria-hidden="true">{entry.folded ? "▸" : "▾"}</span>
            <span className="list-group-label">{entry.topic.label ?? "Untitled"}</span>
            <span className="list-group-counts">{entry.works.length} · {countsLabel(entry.progress)}</span>
          </p>
          {prior !== undefined && <p className="list-col-after">after {prior.join(", ")}</p>}
          {!entry.folded && <ol>{entry.works.map((row) => <WorkCard key={row.id} row={row} props={props} />)}</ol>}
        </div>;
      })}
    </section>)}
    {model.unfiled !== null && <section className="list-col" data-layer="unfiled">
      <h3 className="list-col-head">Unfiled</h3>
      <div className="list-col-topic" data-folded={model.unfiled.folded ? "true" : "false"}>
        <p
          className={`list-group-head ${selectedId === unfiledCursor ? "is-selected" : ""}`}
          data-group-key={UNFILED_KEY}
          onClick={() => onSelect(unfiledCursor)}
        >
          <span aria-hidden="true">{model.unfiled.folded ? "▸" : "▾"}</span>
          <span className="list-group-label">Unfiled</span>
          <span className="list-group-counts">{model.unfiled.rows.length}</span>
        </p>
        {!model.unfiled.folded && <ol>{model.unfiled.rows.map((row) => <WorkCard key={row.id} row={row} props={props} />)}</ol>}
      </div>
    </section>}
  </div>;
}
