import type { TextRect } from "../types";
import type { ObjectAddition, ObjectCaption, ObjectGrades, ObjectVerdictKind, ObjectsArtifact, PaperObject } from "./objects.ts";
import type { ReadingIndex } from "./readingIndex.ts";

export type GradableKind = "figure" | "table";
export type GradableObject = PaperObject & { kind: GradableKind };
export type GradingBlocker = { id: string; reason: string };
export type GradingProgress = { graded: number; total: number; ungraded: string[]; blockers: GradingBlocker[] };
export type Box = { left: number; top: number; width: number; height: number };
export type PdfPoint = { x: number; y: number };

/** Detector figures and tables in reading order: page, then region top; objects without a region close their page. */
export function gradableObjects(artifact: Pick<ObjectsArtifact, "objects">): GradableObject[] {
  const top = (object: GradableObject) => object.region?.y_min ?? Infinity;
  return artifact.objects.filter((object): object is GradableObject => object.kind === "figure" || object.kind === "table")
    .sort((a, b) => a.page - b.page || top(a) - top(b) || a.anchor.start - b.anchor.start || a.id.localeCompare(b.id));
}

export function emptyGrades(artifact: ObjectsArtifact): ObjectGrades {
  return {
    schema_version: 1, paper_id: artifact.paper_id,
    index_sha256: artifact.reading_index_generation.replace(/^"(.*)"$/su, "$1"),
    objects_generation: artifact.figure_detector_generation ?? "",
    grader: "", updated_at: null, revision: null, complete: false, verdicts: {}, additions: [],
  };
}

export function applyVerdict(grades: ObjectGrades, objectId: string, verdict: ObjectVerdictKind, region: TextRect | null = null): ObjectGrades {
  const note = grades.verdicts[objectId]?.note ?? "";
  return { ...grades, verdicts: { ...grades.verdicts, [objectId]: { verdict, region: verdict === "region" ? region : null, note } } };
}

export function clearVerdict(grades: ObjectGrades, objectId: string): ObjectGrades {
  if (!(objectId in grades.verdicts)) return grades;
  return { ...grades, verdicts: Object.fromEntries(Object.entries(grades.verdicts).filter(([id]) => id !== objectId)) };
}

/** Appends a missed object; an omitted id becomes the next free `add-N`. */
export function addObject(grades: ObjectGrades, addition: Omit<ObjectAddition, "id" | "note"> & { id?: string; note?: string }): ObjectGrades {
  const taken = new Set(grades.additions.map((item) => item.id));
  let id = addition.id;
  if (id === undefined || taken.has(id)) {
    let count = grades.additions.length + 1;
    while (taken.has(`add-${count}`)) count++;
    id = `add-${count}`;
  }
  return { ...grades, additions: [...grades.additions, { ...addition, id, note: addition.note ?? "" }] };
}

export function removeAddition(grades: ObjectGrades, id: string): ObjectGrades {
  const additions = grades.additions.filter((item) => item.id !== id);
  return additions.length === grades.additions.length ? grades : { ...grades, additions };
}

/** Export eligibility per the contract: every object graded, no `correct` without a region, no addition without a caption. */
export function gradingProgress(artifact: Pick<ObjectsArtifact, "objects">, grades: ObjectGrades): GradingProgress {
  const objects = gradableObjects(artifact);
  const ungraded = objects.filter((object) => !(object.id in grades.verdicts)).map((object) => object.id);
  const blockers: GradingBlocker[] = [];
  for (const object of objects) {
    const verdict = grades.verdicts[object.id];
    if (verdict?.verdict === "correct" && object.region === null) blockers.push({ id: object.id, reason: "correct verdict without a region; press e to draw one" });
    if (verdict?.verdict === "region" && verdict.region === null) blockers.push({ id: object.id, reason: "region verdict without a drawn box" });
  }
  for (const addition of grades.additions) {
    if (addition.caption === null) blockers.push({ id: addition.id, reason: "addition without a caption" });
  }
  return { graded: objects.length - ungraded.length, total: objects.length, ungraded, blockers };
}

/** Mirrors `label()` in scripts/eval/object-metrics.py: strip the kind prefix and ` .:()`, lowercase, accept `S1`, `2.3`, `iv`. */
export function normalizeLabel(value: string, kind: GradableKind): string | null {
  const prefix = kind === "figure" ? /^(?:fig(?:ure)?\.?)\s*/iu : /^table\s*/iu;
  const stripped = value.trim().replace(prefix, "").replace(/^[ .:()]+|[ .:()]+$/gu, "").toLowerCase();
  return /^(?:[a-z]?[0-9]+(?:\.[0-9]+)*[a-z]?|[ivxlcdm]+)$/u.test(stripped) ? stripped : null;
}

/** The first caption paragraph on `page` whose printed label matches, as the addition's caption span. */
export function proposeCaption(index: Pick<ReadingIndex, "text" | "pages" | "objects">, page: number, kind: GradableKind, printedLabel: string): ObjectCaption | null {
  const wanted = normalizeLabel(printedLabel, kind);
  const bounds = index.pages.find((item) => item.number === page);
  if (wanted === null || bounds === undefined) return null;
  const captions = index.objects.paragraph.filter((paragraph) => paragraph.kind === "caption" && paragraph.start >= bounds.start && paragraph.start < bounds.end)
    .sort((a, b) => a.start - b.start);
  for (const paragraph of captions) {
    const head = /^\s*((?:fig(?:ure)?\.?|table)\s*\S*)/iu.exec(index.text.slice(paragraph.start, Math.min(paragraph.end, paragraph.start + 64)))?.[1];
    if (head !== undefined && normalizeLabel(head, kind) === wanted) return { start: paragraph.start, end: paragraph.end };
  }
  return null;
}

/** Inverse of `projectRect`: a client point on the page surface in PDF user-space units, clamped to the page. */
export function clientToPdf(surfaceBox: Box, pdfWidth: number, pdfHeight: number, clientX: number, clientY: number): PdfPoint | null {
  if (!(surfaceBox.width > 0 && surfaceBox.height > 0 && pdfWidth > 0 && pdfHeight > 0)) return null;
  return {
    x: Math.max(0, Math.min(pdfWidth, (clientX - surfaceBox.left) * pdfWidth / surfaceBox.width)),
    y: Math.max(0, Math.min(pdfHeight, (clientY - surfaceBox.top) * pdfHeight / surfaceBox.height)),
  };
}

/** Normalises a drag between two PDF points; null when it is too small to be a region. */
export function dragRect(start: PdfPoint, end: PdfPoint, minimum = 2): TextRect | null {
  const rect = { x_min: Math.min(start.x, end.x), x_max: Math.max(start.x, end.x), y_min: Math.min(start.y, end.y), y_max: Math.max(start.y, end.y) };
  return rect.x_max - rect.x_min < minimum || rect.y_max - rect.y_min < minimum ? null : rect;
}
