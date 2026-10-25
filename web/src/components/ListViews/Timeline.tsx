import { timeline } from "../../lib/listViews";
import { StubRow } from "./StubRow";
import type { ListViewProps } from "./props";

/** View 3, until the year axis replaces it: works by year, oldest first. */
export function Timeline(props: ListViewProps) {
  const { buckets, undated } = timeline(props.detail);
  if (buckets.length === 0 && undated.length === 0) {
    return <div className="list-empty"><p>Nothing to place on a timeline yet.</p></div>;
  }
  return <ol className="list-sequence" aria-label="Timeline">
    {buckets.map((bucket) => <li key={bucket.year} className="list-stub-group">
      <p className="list-group-head"><span className="list-group-label">{bucket.year}</span></p>
      <ol>{bucket.rows.map((row) => <StubRow key={row.id} row={row} props={props} />)}</ol>
    </li>)}
    {undated.length > 0 && <li className="list-stub-group">
      <p className="list-group-head"><span className="list-group-label">Undated</span></p>
      <ol>{undated.map((row) => <StubRow key={row.id} row={row} props={props} />)}</ol>
    </li>}
  </ol>;
}
