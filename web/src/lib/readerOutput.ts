// Pure helpers that turn the stored analysis into what the Abstract view, the
// Overview and the review panel show. Type-only imports keep this file
// loadable by node's test runner (see scripts/reader-output.test.mjs).

import type {
  ContextBasis,
  ContextClaim,
  NeighbourRelation,
  OutlineEntry,
  OutlineRole,
  PaperAnalysis,
  PaperContext,
  PaperSection,
  QualityWarning,
} from "../types";

// ---------------------------------------------------------------------------
// Context: why the paper matters.

/** How a source class is labelled next to a claim. */
export type BasisLabel = { short: string; long: string };

const BASIS_LABELS: Partial<Record<ContextBasis, BasisLabel>> & { background: BasisLabel } = {
  paper: { short: "From the paper", long: "The paper says this; the quote below was found verbatim in its text." },
  dossier: { short: "Citation graph", long: "Supported by a record in Lysilogy’s citation graph for a neighbouring work." },
  background: { short: "Background · unverified", long: "The model’s own background knowledge. Nothing in the paper or the citation graph checked it." },
};

export function basisLabel(basis: ContextBasis | null | undefined): BasisLabel {
  return BASIS_LABELS[basis ?? "background"] ?? BASIS_LABELS.background;
}

/** Why a claim asked for as `paper` or `dossier` is shown as background, in words. */
export function downgradeText(claim: ContextClaim): string | null {
  switch (claim.downgraded) {
    case "quote_not_found": return "Its quote could not be found in the paper.";
    case "unknown_work": return "The work it cited is not in the citation graph.";
    case "other": return "Its support did not check out.";
    default: return null;
  }
}

const RELATION_LABELS: Partial<Record<NeighbourRelation, string>> & { other: string } = {
  predecessor: "Came before",
  successor: "Came after",
  critique: "Critique",
  replication: "Replication",
  survey: "Survey",
  parallel: "Parallel work",
  other: "Related",
};

export function relationLabel(relation: NeighbourRelation | null | undefined): string {
  return RELATION_LABELS[relation ?? "other"] ?? RELATION_LABELS.other;
}

export type ContextPart = "lead" | "before" | "contribution" | "after";

export const CONTEXT_PARTS: { part: Exclude<ContextPart, "lead">; eyebrow: string; heading: string }[] = [
  { part: "before", eyebrow: "Before the paper", heading: "The problem it entered" },
  { part: "contribution", eyebrow: "The paper", heading: "What it changed" },
  { part: "after", eyebrow: "After the paper", heading: "What followed" },
];

/**
 * The context node's output when it produced anything worth showing. Older
 * analyses have no `context`; an empty one (every list empty) also reads as
 * absent so the view falls back to the earlier notes.
 */
export function paperContext(analysis: Pick<PaperAnalysis, "context">): PaperContext | null {
  const context = analysis.context;
  if (context == null) return null;
  const claims = context.lead.length + context.before.length + context.contribution.length + context.after.length;
  return claims > 0 || context.read_alongside.length > 0 ? context : null;
}

/** Claims per source class, for a one-line provenance summary. */
export function basisCounts(context: PaperContext): Record<ContextBasis, number> {
  const counts: Record<ContextBasis, number> = { paper: 0, dossier: 0, background: 0 };
  for (const claim of [...context.lead, ...context.before, ...context.contribution, ...context.after]) {
    // A value written by a newer backend counts as the least trusted class, as the server reads it.
    const basis = Object.hasOwn(counts, claim.basis) ? claim.basis : "background";
    counts[basis] += 1;
  }
  return counts;
}

// ---------------------------------------------------------------------------
// Outline and logical units.

/** "2.1 Data", or the title alone when the heading is unnumbered. */
export function outlineLabel(entry: Pick<OutlineEntry, "number" | "title">): string {
  const number = entry.number?.trim();
  if (number == null || number === "") return entry.title;
  return entry.title.startsWith(number) ? entry.title : `${number} ${entry.title}`;
}

const ROLE_LABELS: Partial<Record<OutlineRole, string>> & { other: string } = {
  front_matter: "Front matter",
  main: "Main text",
  back_matter: "Back matter",
  references: "References",
  appendix: "Appendix",
  other: "Other",
};

export function roleLabel(role: OutlineRole | null | undefined): string {
  return ROLE_LABELS[role ?? "main"] ?? ROLE_LABELS.other;
}

/** One row of the outline view, with its indent and the logical unit holding it. */
export type OutlineRow = {
  index: number;
  entry: OutlineEntry;
  /** 0 for the shallowest heading level the paper uses. */
  depth: number;
  /** Index into `sections` of the unit that groups this heading, if any. */
  unit: number | null;
};

export function outlineRows(outline: readonly OutlineEntry[], sections: readonly PaperSection[]): OutlineRow[] {
  const owner = new Map<number, number>();
  sections.forEach((section, sectionIndex) => {
    for (const entry of section.outline_entries ?? []) if (!owner.has(entry)) owner.set(entry, sectionIndex);
  });
  const shallowest = outline.reduce((low, entry) => Math.min(low, Math.max(1, entry.level)), Number.POSITIVE_INFINITY);
  return outline.map((entry, index) => ({
    index,
    entry,
    depth: Math.min(4, Math.max(0, Math.max(1, entry.level) - (Number.isFinite(shallowest) ? shallowest : 1))),
    unit: owner.get(index) ?? null,
  }));
}

/** The printed headings a logical unit groups, in order. Out-of-range indices are skipped. */
export function groupedHeadings(section: Pick<PaperSection, "outline_entries">, outline: readonly OutlineEntry[]): OutlineEntry[] {
  return (section.outline_entries ?? []).map((index) => outline[index]).filter((entry): entry is OutlineEntry => entry !== undefined);
}

/**
 * A compact label for the headings a unit groups: "§2", "§2–3.1" for a run,
 * "§3 Data" for a single unnumbered heading. Null when the unit groups none.
 */
export function groupedRangeLabel(section: Pick<PaperSection, "outline_entries">, outline: readonly OutlineEntry[]): string | null {
  const headings = groupedHeadings(section, outline);
  const first = headings[0];
  const last = headings.at(-1);
  if (first === undefined || last === undefined) return null;
  const short = (entry: OutlineEntry) => entry.number?.trim().replace(/\.$/u, "") || entry.title;
  if (headings.length === 1) return `§ ${outlineLabel(first)}`;
  return `§ ${short(first)}–${short(last)}`;
}

/** Whether the Overview has a printed outline to show. */
export function hasOutline(analysis: Pick<PaperAnalysis, "outline">): boolean {
  return (analysis.outline?.length ?? 0) > 0;
}

// ---------------------------------------------------------------------------
// Pipeline warnings.

/** Warnings the reader should see, deduplicated by message. */
export function readerWarnings(analysis: Pick<PaperAnalysis, "quality_warnings">): QualityWarning[] {
  const seen = new Set<string>();
  return (analysis.quality_warnings ?? []).filter((warning) => {
    const key = `${warning.kind}:${warning.message}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/** The pipeline version without its digest, for a compact label: "model@providers". */
export function pipelineLabel(version: string | null | undefined): string | null {
  if (version == null || version.trim() === "") return null;
  return version.replace(/#[0-9a-f]+$/u, "");
}
