import { nextUp } from "../../lib/listViews";
import "./ListViewsReview.css";
import { StubRow } from "./StubRow";
import type { ListViewProps } from "./props";

/** View 5: unread works whose prerequisites are read; when none, the empty state names the blocker. */
export function NextUp(props: ListViewProps) {
  const result = nextUp(props.detail);
  if (result.items.length === 0) {
    if (result.empty === "no_works") {
      return <div className="list-empty" data-empty="no_works"><p>No papers on this list yet.</p></div>;
    }
    if (result.empty === "all_started") {
      return <div className="list-empty" data-empty="all_started"><p>Every paper is already started. Nothing is waiting to be picked up.</p></div>;
    }
    const first = result.blocked?.row.title ?? "The first unread paper";
    const blockers = result.blocked?.blockedBy ?? [];
    return <div className="list-empty" data-empty="blocked">
      <p>Nothing is ready: “{first}” is the next unread paper, and it waits on {blockers.length === 1 ? "one paper" : "its prerequisites"} you have not finished.</p>
      {blockers.length > 0 && <div className="list-next-blocker">
        Finish first:
        <ul>{blockers.map((row) => <li key={row.id}>{row.title}</li>)}</ul>
      </div>}
    </div>;
  }
  return <ol className="list-sequence" aria-label="Next up">
    {result.items.map((item) => <StubRow key={item.row.id} row={item.row} props={props} detail={item.unlocks > 0 ? `unlocks ${item.unlocks}` : undefined} />)}
  </ol>;
}
