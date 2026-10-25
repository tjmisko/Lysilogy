/** What a selected link is, relative to the current list detail. */
export type EdgeSelection = "none" | "proposed" | "settled" | "missing";

export type ReviewRoute =
  | "chooser-close"
  | "chooser-resolve"
  | "chooser-next"
  | "edge-accept"
  | "edge-reject"
  | "node-accept"
  | "node-reject"
  | "node-resolve"
  | "ignore";

export type ReviewKeyRouting = {
  route: ReviewRoute;
  /** The local link selection is stale or spent and must be dropped. */
  clearEdge: boolean;
};

/**
 * Which target a Review key acts on. An open chooser takes Escape, Enter and `c` first. A selected
 * link takes `a` and `x` only while it is still proposed (so still visible in the list); a link that
 * is accepted or gone is dropped and the key falls through to the cursor node.
 */
export function routeReviewKey(key: string, chooserOpen: boolean, edge: EdgeSelection): ReviewKeyRouting {
  if (chooserOpen) {
    if (key === "Escape") return { route: "chooser-close", clearEdge: false };
    if (key === "Enter") return { route: "chooser-resolve", clearEdge: false };
    if (key === "c") return { route: "chooser-next", clearEdge: false };
  }
  const acts = key === "a" || key === "x";
  const clearEdge = edge !== "none" && acts;
  if (acts && edge === "proposed") return { route: key === "a" ? "edge-accept" : "edge-reject", clearEdge };
  if (key === "a") return { route: "node-accept", clearEdge };
  if (key === "x") return { route: "node-reject", clearEdge };
  if (key === "c") return { route: "node-resolve", clearEdge };
  return { route: "ignore", clearEdge };
}
