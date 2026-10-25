import { nextUp } from "../../lib/listViews";
import { StubRow } from "./StubRow";
import type { ListViewProps } from "./props";

/** View 5, until the ranked view replaces it: unread works whose prerequisites are read. */
export function NextUp(props: ListViewProps) {
  const result = nextUp(props.detail);
  if (result.items.length === 0) {
    const text = result.empty === "no_works" ? "No papers on this list yet."
      : result.empty === "all_started" ? "Every paper is already started."
        : `“${result.blocked?.row.title ?? "The first unread paper"}” waits on ${result.blocked?.blockedBy.map((row) => row.title).join(", ") ?? "its prerequisites"}.`;
    return <div className="list-empty" data-empty={result.empty ?? undefined}><p>{text}</p></div>;
  }
  return <ol className="list-sequence" aria-label="Next up">
    {result.items.map((item) => <StubRow key={item.row.id} row={item.row} props={props} detail={item.unlocks > 0 ? `unlocks ${item.unlocks}` : undefined} />)}
  </ol>;
}
