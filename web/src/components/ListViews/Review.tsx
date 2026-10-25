import { useEffect, useMemo, useState } from "react";

import { api } from "../../lib/api";
import { candidateLabel, matchCandidateWork, nextCandidateIndex } from "../../lib/claimCandidates";
import { review } from "../../lib/listViews";
import "./ListViewsReview.css";
import type { ListViewProps } from "./props";

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

type Chooser = { nodeId: string; index: number };

/**
 * View 7: proposed items and links, then claims by state. `a` accepts, `x` rejects, `c` walks the
 * selected claim's candidates (Enter resolves the highlighted one, Escape closes); `:verify`
 * retries pending claims from the command line.
 */
export function Review(props: ListViewProps) {
  const { detail, actions, selectedId, keyboardEnabled } = props;
  const sections = useMemo(() => review(detail), [detail]);
  const [openChooser, setChooser] = useState<Chooser | null>(null);

  // A chooser for a claim that has resolved, left the list or lost its candidates is stale.
  const chooserNode = openChooser === null ? undefined : detail.nodes.find((entry) => entry.id === openChooser.nodeId);
  const chooserLive = chooserNode !== undefined && chooserNode.kind === "claim" && chooserNode.candidates.length > 0;
  const chooser = chooserLive ? openChooser : null;

  const resolveCandidate = async (nodeId: string, index: number): Promise<void> => {
    const node = detail.nodes.find((entry) => entry.id === nodeId);
    const candidate = node?.candidates[index];
    if (candidate === undefined) return;
    setChooser(null);
    try {
      const { results } = await api.searchWorks(candidate.title);
      const workId = matchCandidateWork(candidate, results);
      if (workId === null) {
        actions.notify(`“${candidate.title}” is not in the library yet. Add it as a paper, then press c again.`);
        return;
      }
      await actions.resolveClaim(nodeId, workId);
    } catch (reason: unknown) {
      actions.fail(reason instanceof Error ? reason.message : "The candidate lookup failed.");
    }
  };

  useEffect(() => {
    if (!keyboardEnabled) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || isEditable(event.target)) return;
      if (chooser !== null && chooserNode !== undefined) {
        if (event.key === "Escape") {
          event.preventDefault();
          setChooser(null);
          return;
        }
        if (event.key === "Enter") {
          event.preventDefault();
          void resolveCandidate(chooser.nodeId, chooser.index);
          return;
        }
        if (event.key === "c") {
          event.preventDefault();
          setChooser({ nodeId: chooser.nodeId, index: nextCandidateIndex(chooser.index, chooserNode.candidates.length) });
          return;
        }
      }
      if (selectedId === null || (event.key !== "a" && event.key !== "x" && event.key !== "c")) return;
      const node = detail.nodes.find((entry) => entry.id === selectedId);
      if (node === undefined) return;
      event.preventDefault();
      if (event.key === "c") {
        if (node.kind !== "claim") {
          actions.notify("Only a claim can be resolved to a candidate.");
        } else if (node.candidates.length === 0) {
          actions.notify(node.claim_state === "pending" ? "This claim has not been checked yet. Run :verify." : "This claim has no candidates to resolve to.");
        } else if (node.candidates.length === 1) {
          void resolveCandidate(node.id, 0);
        } else {
          setChooser({ nodeId: node.id, index: 0 });
        }
      } else if (event.key === "x") void actions.remove(node.id);
      else if (node.review === "proposed") void actions.acceptNode(node.id);
      else actions.notify("That item is already accepted.");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // resolveCandidate only closes over detail and actions, both listed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [actions, chooser, chooserNode, detail.nodes, keyboardEnabled, selectedId]);

  if (sections.length === 0) {
    return <div className="list-empty"><p>Nothing to review. Proposed items and unresolved claims show here.</p></div>;
  }
  const rowOf = (nodeId: string) => detail.nodes.find((node) => node.id === nodeId);
  return <div className="list-review">
    <p className="list-review-hint">a accept · x reject · c resolve a claim · :verify retry pending claims</p>
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
          const choosing = chooser?.nodeId === node.id;
          return <li key={node.id} className={`list-row ${selectedId === node.id ? "is-selected" : ""}`} data-node-id={node.id} onClick={() => props.onSelect(node.id)}>
            <span className="list-row-main">
              <span className="list-row-line">
                <span className="list-row-title">{item.title}</span>
                {item.kind === "claim" && <span className="list-row-claim" data-claim-state={item.state}>claim · {item.state}</span>}
                {item.kind === "claim" && node.candidates.length > 0 && <span className="list-row-held">{node.candidates.length} candidate{node.candidates.length === 1 ? "" : "s"}</span>}
                {node.review === "proposed" && <span className="list-row-proposed">proposed</span>}
              </span>
              {node.rationale !== null && <span className="list-row-note">{node.rationale}</span>}
              {choosing && <ol className="list-candidates" aria-label="Candidates">
                {node.candidates.map((candidate, index) => <li
                  key={candidate.provider_id}
                  className={`list-candidate ${chooser.index === index ? "is-highlighted" : ""}`}
                  onClick={(event) => { event.stopPropagation(); void resolveCandidate(node.id, index); }}
                >
                  <span className="list-candidate-index">{index + 1}</span>{candidateLabel(candidate)}
                </li>)}
              </ol>}
            </span>
          </li>;
        })}
      </ol>
    </section>)}
  </div>;
}
