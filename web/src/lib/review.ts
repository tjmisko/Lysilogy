// Types and pure logic for the review panel. Its only runtime import is the
// equally pure readerOutput.ts, so node's test runner can load it directly
// (see scripts/review.test.mjs).

import type {
  ContextNote, ContextSource, EdgeLabelCounts, LabelledEdge, PaperAnalysis, PaperContext, PaperSection,
} from "../types";
import { groupedHeadings, groupedRangeLabel, outlineLabel, outlineRows, paperContext, type OutlineRow } from "./readerOutput.ts";

export type DimensionKind = "score" | "pass_fail" | "choice" | "count";

export type RubricOption = { id: string; label: string };

export type RubricDimension = {
  id: string;
  kind: DimensionKind;
  label: string;
  question: string;
  anchors?: Record<string, string>;
  options?: RubricOption[];
  when?: { dimension: string; equals: boolean | number | string };
};

export type RubricComponent = {
  id: string;
  label: string;
  question: string;
  guidance?: string;
  priority?: number;
  reads: string[];
  dimensions: RubricDimension[];
  hard_reject?: { id: string; label: string; question: string };
};

export type Rubric = {
  id: string;
  name: string;
  purpose: string;
  time_budget_minutes: number;
  scale: { min: number; max: number; anchors: Record<string, string> };
  components: RubricComponent[];
  overall: RubricDimension;
  issues: { purpose: string; severities: RubricOption[]; tags: string[] };
};

export type ManifestNeighbour = {
  id: string;
  relation: string;
  title: string;
  authors?: string[];
  year?: number;
  why: string;
  doi?: string | null;
  arxiv?: string | null;
  urls?: string[];
};

export type ManifestPaper = {
  id: string;
  field: string;
  title: string;
  authors: string[];
  year: number;
  venue: string;
  doi?: string | null;
  arxiv?: string | null;
  filename: string;
  oracle?: string | null;
  notes?: string;
  neighbours: ManifestNeighbour[];
};

export type Manifest = { id: string; note: string; papers: ManifestPaper[] };

export type GradeValue = boolean | number | string;

export type ComponentGrade = {
  not_produced: boolean;
  hard_reject: boolean;
  values: Record<string, GradeValue>;
  notes: string;
};

export type IssueFlag = {
  id: string;
  component: string;
  severity: string;
  tag: string | null;
  note: string;
  quote: string | null;
};

export type ReviewSnapshot = {
  snapshot_id: string;
  generated_at: string | null;
  pipeline_version: string | null;
  provider: string | null;
  fields: ReviewedFields;
};

export type ReviewDraft = {
  components: Record<string, ComponentGrade>;
  overall: number | null;
  notes: string;
  issues: IssueFlag[];
};

export type Review = ReviewDraft & {
  paper_id: string;
  paper_title: string;
  manifest_id: string | null;
  snapshot: ReviewSnapshot;
  created_at: string;
  updated_at: string;
};

export type ReviewInput = ReviewDraft & { snapshot_id: string };

export type EarlierReview = {
  snapshot_id: string;
  generated_at: string | null;
  pipeline_version: string | null;
  updated_at: string;
  overall: number | null;
};

export type PaperReview = {
  paper_id: string;
  paper_title: string;
  manifest_id: string | null;
  snapshot: ReviewSnapshot | null;
  review: Review | null;
  earlier: EarlierReview[];
};

export type ProgressRow = {
  manifest_id: string;
  title: string;
  field: string;
  year: number | null;
  filename: string;
  paper_id: string | null;
  analysed: boolean;
  reviewed: boolean;
  reviewed_earlier: boolean;
};

export const GENERAL_COMPONENT = "general";

// ---------------------------------------------------------------------------
// Reading the reviewed output. A snapshot copies the graded fields of the
// stored analysis verbatim (src/review/snapshot.rs), so they have the reader
// model's types; fields an older analysis lacks are simply absent.

/**
 * The citation labels pinned beside the analysis fields (`edge_labels`): a
 * sample of the last labelling run, pinned at a review's first save.
 */
export type PinnedEdgeLabels = {
  generated_at: string | null;
  status: "completed" | "skipped" | "failed";
  detail: string | null;
  labeler: string | null;
  prompt_version: number;
  label_version: string;
  counts: Partial<EdgeLabelCounts>;
  unresolved_keys: number | null;
  left_for_later: number | null;
  rejected_for_grounding: number | null;
  sample: LabelledEdge[];
};

/** The analysis fields a review snapshot carries, plus the pinned citation labels. */
export type ReviewedFields = { edge_labels?: PinnedEdgeLabels } & Partial<Pick<PaperAnalysis,
  | "thesis"
  | "author_abstract"
  | "abstract_extraction"
  | "abstract_agreement"
  | "outsider_brief"
  | "context"
  | "context_notes"
  | "context_sources"
  | "outline"
  | "sections"
  | "quality_warnings"
  | "component_versions"
>>;

function text(value: string | null | undefined): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

export type ContextOutput = {
  /** The one-line brief kept for older readers; the lead's first claim on new analyses. */
  brief: string | null;
  /** The context node's output, when it produced anything. */
  node: PaperContext | null;
  /** Web-researched notes from the earlier design, with their checked sources. */
  legacyNotes: ContextNote[];
  sources: ContextSource[];
};

export function readContext(fields: ReviewedFields): ContextOutput {
  const node = paperContext(fields);
  return {
    brief: text(fields.outsider_brief),
    node,
    // Notes the node wrote duplicate `node`; only show the older, sourced ones.
    legacyNotes: (fields.context_notes ?? []).filter((note) => note.basis == null && text(note.text) !== null),
    sources: fields.context_sources ?? [],
  };
}

export function readOutline(fields: ReviewedFields): OutlineRow[] {
  return outlineRows(fields.outline ?? [], fields.sections ?? []);
}

export type LogicalUnit = {
  section: PaperSection;
  /** "§ 2–3.1": the printed headings it groups; null for a page-span unit. */
  range: string | null;
  /** The grouped headings, labelled with their numbers. */
  covers: string[];
};

export type LogicalOutput = {
  /** True when the units group outline entries; false for page-span units made without an outline. */
  grouped: boolean;
  units: LogicalUnit[];
};

export function readLogical(fields: ReviewedFields): LogicalOutput {
  const outline = fields.outline ?? [];
  const units = (fields.sections ?? []).map((section) => ({
    section,
    range: groupedRangeLabel(section, outline),
    covers: groupedHeadings(section, outline).map(outlineLabel),
  }));
  return { grouped: units.some((unit) => unit.covers.length > 0), units };
}

export function readAbstract(fields: ReviewedFields): string | null {
  return text(fields.author_abstract);
}

export function readThesis(fields: ReviewedFields): string | null {
  return text(fields.thesis);
}

export type EdgeLabelOutput = {
  pinned: PinnedEdgeLabels | null;
  /** Sampled edges that carry a label. */
  sample: LabelledEdge[];
  /** One line of the run's counts, for the grader. */
  countsLine: string | null;
};

export function readEdgeLabels(fields: ReviewedFields): EdgeLabelOutput {
  const pinned = fields.edge_labels ?? null;
  if (pinned === null) return { pinned: null, sample: [], countsLine: null };
  const counts = pinned.counts;
  const parts: string[] = [];
  if (counts.edges != null) parts.push(`${counts.labelled ?? 0} of ${counts.edges} edges labelled`);
  if ((counts.unlabelled_with_passages ?? 0) > 0) parts.push(`${counts.unlabelled_with_passages} with passages but no label`);
  if ((counts.perfunctory ?? 0) > 0) parts.push(`${counts.perfunctory} perfunctory`);
  if ((counts.not_shown ?? 0) > 0) parts.push(`${counts.not_shown} where the passages do not show why`);
  if ((pinned.unresolved_keys ?? 0) > 0) parts.push(`${pinned.unresolved_keys} core reference keys unresolved`);
  if ((pinned.left_for_later ?? 0) > 0) parts.push(`${pinned.left_for_later} left for a later run`);
  if ((pinned.rejected_for_grounding ?? 0) > 0) parts.push(`${pinned.rejected_for_grounding} answers failed grounding`);
  return {
    pinned,
    sample: pinned.sample.filter((edge) => edge.label !== null),
    countsLine: parts.length === 0 ? null : parts.join(" · "),
  };
}

/**
 * The fields a panel shows: the current output, with the citation labels a
 * saved review pinned taking the place of the current run's sample.
 */
export function shownFields(snapshot: ReviewSnapshot | null, review: Review | null): ReviewedFields | null {
  if (snapshot === null) return null;
  const pinned = review?.snapshot.fields.edge_labels;
  return pinned === undefined ? snapshot.fields : { ...snapshot.fields, edge_labels: pinned };
}

/** Whether the reader produced anything for a rubric component. */
export function isProduced(componentId: string, fields: ReviewedFields): boolean {
  switch (componentId) {
    case "abstract": return readAbstract(fields) !== null;
    case "thesis": return readThesis(fields) !== null;
    case "context": {
      const context = readContext(fields);
      return context.node !== null || context.legacyNotes.length > 0;
    }
    case "outline": return (fields.outline?.length ?? 0) > 0;
    case "logical": return (fields.sections?.length ?? 0) > 0;
    case "edge_labels": return readEdgeLabels(fields).sample.length > 0;
    default: return false;
  }
}

// ---------------------------------------------------------------------------
// Grading state.

export function emptyGrade(): ComponentGrade {
  return { not_produced: false, hard_reject: false, values: {}, notes: "" };
}

export function emptyDraft(): ReviewDraft {
  return { components: {}, overall: null, notes: "", issues: [] };
}

export function draftFromReview(review: Review | null): ReviewDraft {
  if (review === null) return emptyDraft();
  const components: Record<string, ComponentGrade> = {};
  for (const [id, grade] of Object.entries(review.components)) {
    components[id] = { ...emptyGrade(), ...grade, values: { ...grade.values } };
  }
  return { components, overall: review.overall, notes: review.notes, issues: [...review.issues] };
}

export function gradeOf(draft: ReviewDraft, componentId: string): ComponentGrade {
  return draft.components[componentId] ?? emptyGrade();
}

/** Dimensions that apply given the grades so far (e.g. the abstract's error type only when it is not exact). */
export function visibleDimensions(component: RubricComponent, grade: ComponentGrade): RubricDimension[] {
  return component.dimensions.filter((dimension) => dimension.when === undefined || grade.values[dimension.when.dimension] === dimension.when.equals);
}

export function setValue(draft: ReviewDraft, componentId: string, dimensionId: string, value: GradeValue | null): ReviewDraft {
  const grade = gradeOf(draft, componentId);
  const values = Object.fromEntries(Object.entries(grade.values).filter(([key]) => key !== dimensionId));
  if (value !== null) values[dimensionId] = value;
  return { ...draft, components: { ...draft.components, [componentId]: { ...grade, values } } };
}

export function updateGrade(draft: ReviewDraft, componentId: string, change: Partial<ComponentGrade>): ReviewDraft {
  return { ...draft, components: { ...draft.components, [componentId]: { ...gradeOf(draft, componentId), ...change } } };
}

/** A focusable grading row: a component dimension, or the overall score. */
export type FocusRow = { componentId: string; dimension: RubricDimension };

export const OVERALL = "overall";

export function focusRows(rubric: Rubric, draft: ReviewDraft): FocusRow[] {
  const rows: FocusRow[] = [];
  for (const component of rubric.components) {
    for (const dimension of visibleDimensions(component, gradeOf(draft, component.id))) rows.push({ componentId: component.id, dimension });
  }
  rows.push({ componentId: OVERALL, dimension: rubric.overall });
  return rows;
}

export function valueOf(draft: ReviewDraft, row: FocusRow): GradeValue | undefined {
  if (row.componentId === OVERALL) return draft.overall ?? undefined;
  return gradeOf(draft, row.componentId).values[row.dimension.id];
}

export function setRowValue(draft: ReviewDraft, row: FocusRow, value: GradeValue | null): ReviewDraft {
  if (row.componentId === OVERALL) return { ...draft, overall: typeof value === "number" ? value : null };
  return setValue(draft, row.componentId, row.dimension.id, value);
}

/**
 * Applies one grading key to the focused row: 0-4 scores, y/n or space for
 * pass/fail, 1-9 picks a choice, digits, + and - set a count, Backspace
 * clears. Returns null when the key means nothing for this row.
 */
export function applyGradingKey(draft: ReviewDraft, row: FocusRow, key: string, scale: { min: number; max: number }): ReviewDraft | null {
  const current = valueOf(draft, row);
  if (key === "Backspace" || key === "Delete") return setRowValue(draft, row, null);
  const digit = /^[0-9]$/u.test(key) ? Number(key) : null;
  switch (row.dimension.kind) {
    case "score":
      return digit !== null && digit >= scale.min && digit <= scale.max ? setRowValue(draft, row, digit) : null;
    case "pass_fail":
      if (key === "y" || key === "p") return setRowValue(draft, row, true);
      if (key === "n" || key === "f") return setRowValue(draft, row, false);
      if (key === " ") return setRowValue(draft, row, current !== true);
      return null;
    case "choice": {
      const options = row.dimension.options ?? [];
      const option = digit === null ? undefined : options[digit - 1];
      return option === undefined ? null : setRowValue(draft, row, option.id);
    }
    case "count": {
      const count = typeof current === "number" ? current : 0;
      if (digit !== null) return setRowValue(draft, row, typeof current === "number" && current > 0 && current < 100 ? current * 10 + digit : digit);
      if (key === "+" || key === "=") return setRowValue(draft, row, count + 1);
      if (key === "-") return setRowValue(draft, row, Math.max(0, count - 1));
      return null;
    }
  }
}

export type Completion = { graded: number; total: number; components: Record<string, boolean>; done: boolean };

/** How much of the rubric is filled in. A component marked not produced counts once any of its dimensions is graded or it is flagged. */
export function completion(rubric: Rubric, draft: ReviewDraft): Completion {
  let graded = 0;
  let total = 0;
  const components: Record<string, boolean> = {};
  for (const component of rubric.components) {
    const grade = gradeOf(draft, component.id);
    const dimensions = visibleDimensions(component, grade);
    const filled = dimensions.filter((dimension) => grade.values[dimension.id] !== undefined).length;
    graded += filled;
    total += dimensions.length;
    components[component.id] = filled === dimensions.length;
  }
  total += 1;
  if (draft.overall !== null) graded += 1;
  return { graded, total, components, done: graded === total };
}

export function toInput(snapshotId: string, draft: ReviewDraft): ReviewInput {
  return { snapshot_id: snapshotId, ...draft };
}

export function nextIssueId(issues: IssueFlag[]): string {
  const used = new Set(issues.map((issue) => issue.id));
  let index = issues.length + 1;
  while (used.has(`issue-${index}`)) index += 1;
  return `issue-${index}`;
}

export type ProgressCounts = { total: number; inLibrary: number; analysed: number; reviewed: number };

export function progressCounts(rows: ProgressRow[]): ProgressCounts {
  return {
    total: rows.length,
    inLibrary: rows.filter((row) => row.paper_id !== null).length,
    analysed: rows.filter((row) => row.analysed).length,
    reviewed: rows.filter((row) => row.reviewed).length,
  };
}

export function progressState(row: ProgressRow): "missing" | "in-library" | "analysed" | "reviewed" | "stale" {
  if (row.reviewed) return "reviewed";
  if (row.analysed) return row.reviewed_earlier ? "stale" : "analysed";
  return row.paper_id === null ? "missing" : "in-library";
}
