import { topicLayers } from "../../lib/listViews";
import { StubRow } from "./StubRow";
import type { ListViewProps } from "./props";

/** View 2, until the layered topic view replaces it: topics in layer order with their works. */
export function Topics(props: ListViewProps) {
  const { layers, unfiled } = topicLayers(props.detail, props.viewState.folded);
  if (layers.length === 0 && unfiled === null) {
    return <div className="list-empty"><p>No topics yet. Press <kbd>O</kbd> to add one.</p></div>;
  }
  return <ol className="list-sequence" aria-label="Topics">
    {layers.flat().map((entry) => <li key={entry.topic.id} className="list-stub-group">
      <p
        className={`list-group-head ${props.selectedId === entry.topic.id ? "is-selected" : ""}`}
        data-node-id={entry.topic.id}
        onClick={() => props.onSelect(entry.topic.id)}
      >
        <span className="list-group-label">{entry.topic.label}</span>
        <span className="list-group-counts">{entry.progress.read} read · {entry.progress.reading} reading · {entry.progress.unread} unread</span>
      </p>
      <ol>{entry.works.map((row) => <StubRow key={row.id} row={row} props={props} />)}</ol>
    </li>)}
    {unfiled !== null && <li className="list-stub-group">
      <p className="list-group-head"><span className="list-group-label">Unfiled</span></p>
      <ol>{unfiled.rows.map((row) => <StubRow key={row.id} row={row} props={props} />)}</ol>
    </li>}
  </ol>;
}
