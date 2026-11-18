import type {
  DossierEdgeLabel, EdgeEngagement, EdgeLabelPath, LabelledEdge, PaperContext, PaperEdgeLabels,
} from "../types";

/** Plain-language names for the edge relation vocabulary, read as "this paper … the cited work". */
const RELATION_PHRASES: Record<string, string> = {
  background: "Background",
  uses_method: "Uses its method",
  uses_data: "Uses its data",
  extends: "Builds on it",
  supports: "Supports it",
  contrasts: "Contrasts with it",
  critiques: "Critiques it",
  compares: "Compares against it",
  other: "Other",
};

/** Relations in the order a per-paper list groups them: what the paper stands on first. */
export const EDGE_RELATION_ORDER = [
  "uses_method", "uses_data", "extends", "compares", "supports", "contrasts", "critiques", "background", "other",
] as const;

export function edgeRelationLabel(relation: string): string {
  return RELATION_PHRASES[relation] ?? relation.replace(/_/gu, " ");
}

export function engagementLabel(engagement: EdgeEngagement | null | undefined): string | null {
  switch (engagement) {
    case "perfunctory": return "Cited in passing";
    case "not_shown": return "The citing text does not show why";
    default: return null;
  }
}

export function pathLabel(path: EdgeLabelPath | null | undefined): string {
  switch (path) {
    case "core_relation": return "from the paper reading";
    case "user": return "written by you";
    default: return "from the citation labeler";
  }
}

/** One edge's "why", whichever source it came from. */
export type EdgeWhy = {
  workId: string;
  /** `cites`: this paper cites the work. `cited_by`: the work cites this paper. */
  direction: "cites" | "cited_by";
  relation: string;
  reason: string;
  quote: string | null;
  engagement: EdgeEngagement | null;
  path: EdgeLabelPath | null;
  reviewState: string;
  /** `graph`: read live from the citation graph. `dossier`: as the context node read it. */
  source: "graph" | "dossier";
};

function fromDossier(workId: string, citing: boolean, edge: DossierEdgeLabel): EdgeWhy {
  return {
    workId,
    direction: citing ? "cited_by" : "cites",
    relation: edge.relation,
    reason: edge.reason,
    quote: edge.quote,
    engagement: edge.engagement,
    path: edge.path,
    reviewState: edge.review_state,
    source: "dossier",
  };
}

function fromGraph(edge: LabelledEdge, direction: EdgeWhy["direction"]): EdgeWhy | null {
  const label = edge.label;
  if (label === null || label.review_state === "rejected") return null;
  return {
    workId: edge.work_id,
    direction,
    relation: label.relation,
    reason: label.reason,
    quote: label.quote,
    engagement: label.engagement,
    path: label.path,
    reviewState: label.review_state,
    source: "graph",
  };
}

/**
 * The "why" of every edge around the paper, by Work id. The live graph wins
 * over the copy stored with the context, so a label accepted, edited or
 * rejected since the analysis shows as it is now; a rejected label is not shown.
 */
export function edgeWhyByWork(context: PaperContext | null, labels: PaperEdgeLabels | null): Map<string, EdgeWhy> {
  const whys = new Map<string, EdgeWhy>();
  for (const work of context?.dossier?.works ?? []) {
    if (work.edge != null && work.edge.review_state !== "rejected") whys.set(work.id, fromDossier(work.id, work.citing, work.edge));
  }
  if (labels !== null && labels.status === "ok") {
    for (const [edges, direction] of [[labels.outgoing, "cites"], [labels.incoming, "cited_by"]] as const) {
      for (const edge of edges) {
        const why = fromGraph(edge, direction);
        if (why === null) whys.delete(edge.work_id);
        else whys.set(edge.work_id, why);
      }
    }
  }
  return whys;
}

/** The lead-in for an edge's reason. */
export function whyHeading(why: EdgeWhy): string {
  return why.direction === "cites" ? "Why this paper cites it" : "How it cites this paper";
}

/** Labelled edges grouped by relation, in [`EDGE_RELATION_ORDER`], rejected labels left out. */
export function groupByRelation<T extends { relation: string }>(items: readonly T[]): { relation: string; items: T[] }[] {
  const groups = new Map<string, T[]>();
  for (const item of items) groups.set(item.relation, [...(groups.get(item.relation) ?? []), item]);
  const order: readonly string[] = EDGE_RELATION_ORDER;
  return [...groups.entries()]
    .sort(([left], [right]) => {
      const a = order.indexOf(left);
      const b = order.indexOf(right);
      return (a === -1 ? order.length : a) - (b === -1 ? order.length : b) || left.localeCompare(right);
    })
    .map(([relation, grouped]) => ({ relation, items: grouped }));
}

/** Incoming labelled edges: how later papers in the graph cite this one. */
export function laterCitations(labels: PaperEdgeLabels | null): EdgeWhy[] {
  if (labels === null || labels.status !== "ok") return [];
  return labels.incoming.map((edge) => fromGraph(edge, "cited_by")).filter((why): why is EdgeWhy => why !== null);
}

/**
 * One line saying how complete a paper's labels are, from its last run: how
 * many say what role the work plays, and what went wrong (failed calls, the
 * budget, answers that failed grounding), so "nothing went wrong yet" and
 * "every call failed" read differently.
 */
export function runSummary(labels: PaperEdgeLabels | null): string | null {
  const run = labels?.run;
  if (run == null) return null;
  if (run.status === "skipped" || run.status === "failed") {
    return run.detail === null ? null : `Citation labels ${run.status === "failed" ? "failed" : "skipped"}: ${run.detail}`;
  }
  const counts = run.counts;
  const pass = run.labels ?? {};
  const parts = [`${counts.labelled} of ${counts.edges} citations labelled`];
  if (counts.labelled_with_role !== undefined && counts.labelled > 0) parts.push(`${counts.labelled_with_role} say what role the work plays`);
  if (counts.unlabelled_with_passages > 0) parts.push(`${counts.unlabelled_with_passages} still to label`);
  if (counts.perfunctory > 0) parts.push(`${counts.perfunctory} cited in passing`);
  const failed = pass.failed ?? 0;
  if (failed > 0) parts.push(`${failed} labeler calls failed${pass.first_error ? ` (${pass.first_error})` : ""}`);
  if ((pass.over_budget ?? 0) > 0) parts.push(`${pass.over_budget} over the label budget`);
  if ((pass.over_limit ?? 0) > 0) parts.push(`${pass.over_limit} over the call limit`);
  if ((pass.rejected_for_grounding ?? 0) > 0) parts.push(`${pass.rejected_for_grounding} answers failed grounding`);
  if ((pass.misattributed ?? 0) > 0) parts.push(`${pass.misattributed} whose passages cite another work`);
  if (run.status === "partial") parts.push("run the labels step again to finish");
  return parts.join(" · ");
}
