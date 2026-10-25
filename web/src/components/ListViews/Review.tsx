import { useEffect, useMemo } from "react";

import { review } from "../../lib/listViews";
import type { ListViewProps } from "./props";

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** View 7, until the full view replaces it: proposed items and links, then claims by state. `a` accepts, `x` rejects. */
export function Review(props: ListViewProps) {
  const { detail, actions, selectedId, keyboardEnabled } = props;
  const sections = useMemo(() => review(detail), [detail]);

  useEffect(() => {
    if (!keyboardEnabled) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || isEditable(event.target)) return;
      if (selectedId === null || (event.key !== "a" && event.key !== "x")) return;
      const node = detail.nodes.find((entry) => entry.id === selectedId);
      if (node === undefined) return;
      event.preventDefault();
      if (event.key === "x") void actions.remove(node.id);
      else if (node.review === "proposed") void actions.acceptNode(node.id);
      else actions.notify("That item is already accepted.");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [actions, detail.nodes, keyboardEnabled, selectedId]);

  if (sections.length === 0) {
    return <div className="list-empty"><p>Nothing to review. Proposed items and unresolved claims show here.</p></div>;
  }
  const rowOf = (nodeId: string) => props.detail.nodes.find((node) => node.id === nodeId);
  return <div className="list-review">
    {sections.map((section) => <section key={section.key} className="list-stub-group">
      <p className="list-group-head"><span className="list-group-label">{section.label}</span><span className="list-group-counts">{section.items.length}</span></p>
      <ol className="list-sequence">
        {section.items.map((item) => {
          if (item.kind === "edge") {
            const edge = item.edge;
            return <li key={`${edge.from}${edge.to}${edge.kind}`} className="list-row" data-edge-kind={edge.kind}>
              <span className="list-row-main">
                <span className="list-row-line">
                  <span className="list-row-title">{item.fromTitle} → {item.toTitle}</span>
                  <span className="list-row-held">{edge.kind}</span>
                  <button type="button" className="list-row-action" onClick={() => void actions.acceptEdge({ from: edge.from, to: edge.to, kind: edge.kind })}>accept</button>
                  <button type="button" className="list-row-action" onClick={() => void actions.removeEdge({ from: edge.from, to: edge.to, kind: edge.kind })}>reject</button>
                </span>
              </span>
            </li>;
          }
          const node = rowOf(item.node.id);
          if (node === undefined) return null;
          return <li key={node.id} className={`list-row ${selectedId === node.id ? "is-selected" : ""}`} data-node-id={node.id} onClick={() => props.onSelect(node.id)}>
            <span className="list-row-main">
              <span className="list-row-line">
                <span className="list-row-title">{item.title}</span>
                {item.kind === "claim" && <span className="list-row-claim" data-claim-state={item.state}>claim · {item.state}</span>}
                {node.review === "proposed" && <span className="list-row-proposed">proposed</span>}
              </span>
              {node.rationale !== null && <span className="list-row-note">{node.rationale}</span>}
            </span>
          </li>;
        })}
      </ol>
    </section>)}
  </div>;
}
