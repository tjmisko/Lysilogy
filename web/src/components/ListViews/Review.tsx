import { useEffect, useMemo, useRef, useState } from "react";

import { api } from "../../lib/api";
import { candidateLabel, matchCandidateWork, nextCandidateIndex } from "../../lib/claimCandidates";
import { review } from "../../lib/listViews";
import { routeReviewKey, type EdgeSelection } from "../../lib/reviewKeys";
import "./ListViewsReview.css";
import type { ListViewProps } from "./props";

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

type Chooser = { nodeId: string; index: number };
type EdgeKey = { from: string; to: string; kind: string };

const sameEdge = (left: EdgeKey | null, right: EdgeKey): boolean =>
  left !== null && left.from === right.from && left.to === right.to && left.kind === right.kind;

/**
 * View 7: proposed items and links, then claims by state. `a` accepts, `x` rejects, `c` walks the
 * selected claim's candidates (Enter resolves the highlighted one, Escape closes); `:verify`
 * retries pending claims from the command line.
 */
export function Review(props: ListViewProps) {
  const { detail, actions, selectedId, keyboardEnabled } = props;
  const sections = useMemo(() => review(detail), [detail]);
  const [openChooser, setChooser] = useState<Chooser | null>(null);
  // Proposed links are not on the page cursor, so a clicked link keeps a local selection.
  const [selectedEdge, setSelectedEdge] = useState<EdgeKey | null>(null);
  const resolving = useRef<Set<string>>(new Set());
  const [lastSelectedId, setLastSelectedId] = useState<string | null>(selectedId);
  if (lastSelectedId !== selectedId) {
    // The cursor moved to another node: close the chooser and drop the link selection.
    setLastSelectedId(selectedId);
    setChooser(null);
    setSelectedEdge(null);
  }

  // A chooser for a claim that has resolved, left the list or lost its candidates is stale.
  const chooserNode = openChooser === null ? undefined : detail.nodes.find((entry) => entry.id === openChooser.nodeId);
  const chooserLive = chooserNode !== undefined && chooserNode.kind === "claim" && chooserNode.candidates.length > 0;
  const chooser = chooserLive ? openChooser : null;

  const resolveCandidate = async (nodeId: string, index: number): Promise<void> => {
    const node = detail.nodes.find((entry) => entry.id === nodeId);
    const candidate = node?.candidates[index];
    if (candidate === undefined || resolving.current.has(nodeId)) return;
    resolving.current.add(nodeId);
    setChooser(null);
    try {
      const { results } = await api.searchWorks(candidate.title, true);
      const workId = matchCandidateWork(candidate, results);
      if (workId === null) {
        // Not a Work yet: add the candidate in place, then drop the claim it answers.
        const added = await actions.addNode({ claim: { title: candidate.title, authors: candidate.authors, year: candidate.year, doi: candidate.doi } });
        if (added === null) return;
        if (added.kind === "work") await actions.remove(nodeId);
        else actions.notify(`“${candidate.title}” was added as a claim to verify. Remove the original claim with x once it resolves.`);
        return;
      }
      await actions.resolveClaim(nodeId, workId);
    } catch (reason: unknown) {
      actions.fail(reason instanceof Error ? reason.message : "The candidate lookup failed.");
    } finally {
      resolving.current.delete(nodeId);
    }
  };

  useEffect(() => {
    if (!keyboardEnabled) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || isEditable(event.target)) return;
      const edge = selectedEdge === null ? undefined : detail.edges.find((entry) => sameEdge(selectedEdge, entry));
      const selection: EdgeSelection = selectedEdge === null ? "none" : edge === undefined ? "missing" : edge.review === "proposed" ? "proposed" : "settled";
      const { route, clearEdge } = routeReviewKey(event.key, chooser !== null && chooserNode !== undefined, selection);
      if (clearEdge) setSelectedEdge(null);
      if (route === "chooser-close" || route === "chooser-resolve" || route === "chooser-next") {
        if (chooser === null || chooserNode === undefined) return;
        event.preventDefault();
        if (route === "chooser-close") setChooser(null);
        else if (route === "chooser-resolve") void resolveCandidate(chooser.nodeId, chooser.index);
        else setChooser({ nodeId: chooser.nodeId, index: nextCandidateIndex(chooser.index, chooserNode.candidates.length) });
        return;
      }
      if ((route === "edge-accept" || route === "edge-reject") && edge !== undefined) {
        event.preventDefault();
        const key = { from: edge.from, to: edge.to, kind: edge.kind };
        if (route === "edge-reject") void actions.removeEdge(key);
        else void actions.acceptEdge(key);
        return;
      }
      if (selectedId === null || (route !== "node-accept" && route !== "node-reject" && route !== "node-resolve")) return;
      const node = detail.nodes.find((entry) => entry.id === selectedId);
      if (node === undefined) return;
      event.preventDefault();
      if (route === "node-resolve") {
        if (node.kind !== "claim") {
          actions.notify("Only a claim can be resolved to a candidate.");
        } else if (node.candidates.length === 0) {
          actions.notify(node.claim_state === "pending" ? "This claim has not been checked yet. Run :verify." : "This claim has no candidates to resolve to.");
        } else if (node.candidates.length === 1) {
          void resolveCandidate(node.id, 0);
        } else {
          setChooser({ nodeId: node.id, index: 0 });
        }
      } else if (route === "node-reject") void actions.remove(node.id);
      else if (node.review === "proposed") void actions.acceptNode(node.id);
      else actions.notify("That item is already accepted.");
    };
    // Capture phase, so the chooser's Enter and Escape run before the page's own handlers.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // resolveCandidate only closes over detail and actions, both listed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [actions, chooser, chooserNode, detail.edges, detail.nodes, keyboardEnabled, selectedEdge, selectedId]);

  if (sections.length === 0) {
    return <div className="list-empty"><p>Nothing to review. Proposed items and unresolved claims show here.</p></div>;
  }
  const rowOf = (nodeId: string) => detail.nodes.find((node) => node.id === nodeId);
  return <div className="list-review">
    <p className="list-review-hint"><kbd>a</kbd> accept · <kbd>x</kbd> reject · <kbd>c</kbd> resolve a claim · <kbd>:verify</kbd> retry pending claims (click a link to select it)</p>
    {sections.map((section) => <section key={section.key} className="list-stub-group">
      <p className="list-group-head"><span className="list-group-label">{section.label}</span><span className="list-group-counts">{section.items.length}</span></p>
      <ol className="list-sequence">
        {section.items.map((item) => {
          if (item.kind === "edge") {
            const edge = item.edge;
            return <li key={`${edge.from}${edge.to}${edge.kind}`} className={`list-row ${sameEdge(selectedEdge, edge) ? "is-selected" : ""}`} data-edge-kind={edge.kind} onClick={() => setSelectedEdge({ from: edge.from, to: edge.to, kind: edge.kind })}>
              <span className="list-row-main">
                <span className="list-row-line">
                  <span className="list-row-title">{item.fromTitle} → {item.toTitle}</span>
                  <span className="list-row-held">{edge.kind}</span>
                  <button type="button" className="list-row-action" onClick={(event) => { event.stopPropagation(); void actions.acceptEdge({ from: edge.from, to: edge.to, kind: edge.kind }); }}><kbd>a</kbd> accept</button>
                  <button type="button" className="list-row-action" onClick={(event) => { event.stopPropagation(); void actions.removeEdge({ from: edge.from, to: edge.to, kind: edge.kind }); }}><kbd>x</kbd> reject</button>
                </span>
              </span>
            </li>;
          }
          const node = rowOf(item.node.id);
          if (node === undefined) return null;
          const choosing = chooser?.nodeId === node.id;
          return <li key={node.id} className={`list-row ${selectedId === node.id ? "is-selected" : ""}`} data-node-id={node.id} onClick={() => { setSelectedEdge(null); props.onSelect(node.id); }}>
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
                  {candidateLabel(candidate)}
                </li>)}
                <li className="list-candidate-hint"><kbd>c</kbd> next · <kbd>Enter</kbd> resolve · <kbd>Esc</kbd> close</li>
              </ol>}
            </span>
          </li>;
        })}
      </ol>
    </section>)}
  </div>;
}
