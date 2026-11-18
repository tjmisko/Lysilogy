// Types and pure logic for the review panel. Kept free of runtime imports so
// node's test runner can load it directly (see scripts/review.test.mjs).

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
  fields: Record<string, unknown>;
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
// Reading the reviewed output defensively. The reader model is still gaining
// fields, so each reader accepts several plausible shapes and reports
// "not produced" (null or an empty list) when it finds nothing.

type Loose = Record<string, unknown>;

function isRecord(value: unknown): value is Loose {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function firstText(record: Loose, keys: string[]): string | null {
  for (const key of keys) {
    const found = text(record[key]);
    if (found !== null) return found;
  }
  return null;
}

function firstList(record: Loose, keys: string[]): unknown[] | null {
  for (const key of keys) {
    const value = record[key];
    if (Array.isArray(value) && value.length > 0) return value as unknown[];
  }
  return null;
}

function listOf(value: unknown): unknown[] {
  return Array.isArray(value) ? (value as unknown[]) : [];
}

function numberOf(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function authorsOf(value: unknown): string | null {
  if (typeof value === "string") return text(value);
  if (Array.isArray(value)) {
    const names = value.map((name) => (typeof name === "string" ? name : isRecord(name) ? firstText(name, ["name", "display_name"]) : null))
      .filter((name): name is string => name !== null);
    if (names.length === 0) return null;
    return names.length > 2 ? `${names[0]} et al.` : names.join(" & ");
  }
  return null;
}

function pagesOf(record: Loose): string | null {
  const pages = isRecord(record.pages) ? record.pages : record;
  const start = numberOf(pages.start) ?? numberOf(record.start_page) ?? numberOf(record.page);
  const end = numberOf(pages.end) ?? numberOf(record.end_page);
  if (start === null) return null;
  return end === null || end === start ? `p. ${start}` : `pp. ${start}–${end}`;
}

export type ContextOutput = {
  paragraphs: { label: string | null; text: string }[];
  readAlongside: { title: string; byline: string | null; why: string | null; link: string | null }[];
  sources: { title: string; supports: string | null; url: string | null }[];
};

const CONTEXT_TEXT_KEYS: Array<[string, string | null]> = [
  ["why_it_matters", "Why it matters"],
  ["summary", null],
  ["overview", null],
  ["text", null],
  ["significance", "Significance"],
  ["before", "Before"],
  ["after", "After"],
  ["what_changed", "What changed"],
  ["reception", "Reception"],
];

function contextItem(value: unknown): ContextOutput["readAlongside"][number] | null {
  if (typeof value === "string") return text(value) === null ? null : { title: value.trim(), byline: null, why: null, link: null };
  if (!isRecord(value)) return null;
  const title = firstText(value, ["title", "name", "work", "citation"]);
  if (title === null) return null;
  const year = numberOf(value.year);
  const authors = authorsOf(value.authors);
  const byline = [authors, year === null ? null : String(year)].filter((part) => part !== null).join(", ");
  const doi = text(value.doi);
  const arxiv = text(value.arxiv);
  return {
    title,
    byline: byline.length > 0 ? byline : null,
    why: firstText(value, ["why", "reason", "relationship", "relation", "note", "summary"]),
    link: text(value.url) ?? (doi === null ? null : `https://doi.org/${doi}`) ?? (arxiv === null ? null : `https://arxiv.org/abs/${arxiv}`),
  };
}

export function readContext(fields: Record<string, unknown>): ContextOutput {
  const paragraphs: ContextOutput["paragraphs"] = [];
  const brief = text(fields.outsider_brief);
  if (brief !== null) paragraphs.push({ label: null, text: brief });
  const context = fields.context;
  if (typeof context === "string" && text(context) !== null) paragraphs.push({ label: null, text: context.trim() });
  const nested = isRecord(context) ? context : {};
  for (const [key, label] of CONTEXT_TEXT_KEYS) {
    const found = text(nested[key]);
    if (found !== null) paragraphs.push({ label, text: found });
  }
  const notes = [...listOf(fields.context_notes), ...listOf(nested.notes)];
  for (const note of notes) {
    if (typeof note === "string" && text(note) !== null) paragraphs.push({ label: null, text: note.trim() });
    else if (isRecord(note)) {
      const body = firstText(note, ["text", "claim", "summary"]);
      const kind = text(note.kind);
      if (body !== null) paragraphs.push({ label: kind === null || kind === "legacy" ? null : kind.charAt(0).toUpperCase() + kind.slice(1), text: body });
    }
  }
  const alongside = firstList(fields, ["read_alongside", "neighbours", "neighbors", "surrounding_papers"])
    ?? firstList(nested, ["read_alongside", "neighbours", "neighbors", "papers", "related"]) ?? [];
  const sourceList = firstList(fields, ["context_sources"]) ?? firstList(nested, ["sources"]) ?? [];
  return {
    paragraphs,
    readAlongside: alongside.map(contextItem).filter((item) => item !== null),
    sources: sourceList.filter(isRecord).map((source) => ({
      title: firstText(source, ["title", "name"]) ?? "Untitled source",
      supports: firstText(source, ["supports", "relationship", "why"]),
      url: text(source.url),
    })),
  };
}

export type OutlineEntry = { title: string; level: number; pages: string | null };

/** Prefixes a heading with its printed number unless the title already starts with it. */
function numbered(number: string | null, title: string): string {
  if (number === null) return title;
  const rest = title.slice(number.length);
  const alreadyNumbered = title.startsWith(number) && (rest === "" || /^[.\s:)]/u.test(rest));
  return alreadyNumbered ? title : `${number} ${title}`;
}

export function readOutline(fields: Record<string, unknown>): OutlineEntry[] {
  const outline = fields.outline;
  const list = Array.isArray(outline) ? outline
    : isRecord(outline) ? firstList(outline, ["sections", "entries", "headings", "items"]) ?? []
    : firstList(fields, ["outline_sections", "printed_sections", "printed_outline"]) ?? [];
  const entries: OutlineEntry[] = [];
  const visit = (items: unknown[], depth: number): void => {
    for (const item of items) {
      if (typeof item === "string") {
        if (text(item) !== null) entries.push({ title: item.trim(), level: depth, pages: null });
        continue;
      }
      if (!isRecord(item)) continue;
      const title = firstText(item, ["title", "heading", "text", "name"]);
      const number = firstText(item, ["number", "label"]);
      if (title !== null) {
        entries.push({
          title: numbered(number, title),
          level: numberOf(item.level) ?? numberOf(item.depth) ?? depth,
          pages: pagesOf(item),
        });
      }
      const children = firstList(item, ["children", "subsections", "sections"]);
      if (children !== null) visit(children, depth + 1);
    }
  };
  visit(list, 1);
  return entries;
}

export type LogicalUnit = { title: string; summary: string | null; pages: string | null; covers: string[] };

export type LogicalOutput = { source: string | null; units: LogicalUnit[] };

const LOGICAL_KEYS = ["logical_sections", "section_groups", "reading_units", "logical_segmentation", "logical"];

export function readLogical(fields: Record<string, unknown>): LogicalOutput {
  let source: string | null = null;
  let list: unknown[] = [];
  for (const key of [...LOGICAL_KEYS, "sections"]) {
    const value = fields[key];
    const found = Array.isArray(value) ? value : isRecord(value) ? firstList(value, ["units", "sections", "groups"]) : null;
    if (found !== null && found.length > 0) {
      source = key;
      list = found;
      break;
    }
  }
  const units = list.filter(isRecord).map((unit) => {
    const covers = firstList(unit, ["covers", "members", "printed_sections", "outline_sections", "includes"]) ?? [];
    return {
      title: firstText(unit, ["title", "heading", "name"]) ?? "Untitled unit",
      summary: firstText(unit, ["summary", "digest", "question", "description"]),
      pages: pagesOf(unit),
      covers: covers.map((entry) => (typeof entry === "string" ? entry : isRecord(entry) ? firstText(entry, ["title", "heading", "id"]) : null))
        .filter((entry): entry is string => entry !== null),
    };
  });
  return { source, units };
}

export function readAbstract(fields: Record<string, unknown>): string | null {
  return text(fields.author_abstract);
}

export function readThesis(fields: Record<string, unknown>): string | null {
  return text(fields.thesis);
}

/** Whether the reader produced anything for a rubric component. */
export function isProduced(componentId: string, fields: Record<string, unknown>): boolean {
  switch (componentId) {
    case "abstract": return readAbstract(fields) !== null;
    case "thesis": return readThesis(fields) !== null;
    case "context": {
      const context = readContext(fields);
      return context.paragraphs.length > 0 || context.readAlongside.length > 0;
    }
    case "outline": return readOutline(fields).length > 0;
    case "logical": return readLogical(fields).units.length > 0;
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
