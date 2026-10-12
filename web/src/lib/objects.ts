import type { TextRect } from "../types";
import { ApiError, request } from "./api.ts";

export type StatementKind = "theorem" | "lemma" | "proposition" | "corollary" | "definition" | "remark";

export type PaperObjectKind =
  | { kind: "figure" | "table" | "equation" | "algorithm" | "bib_entry" }
  | { kind: "statement"; statement_kind: StatementKind }
  | { kind: "proof"; statement_id: string | null };

/** Half-open UTF-16 offsets in the complete text of the artifact's reading-index generation. */
export type ReadingIndexAnchor = { page: number; start: number; end: number };
export type ObjectMention = { anchor: ReadingIndexAnchor; rects: TextRect[] };

export type PaperObject = PaperObjectKind & {
  id: string;
  label: string;
  page: number;
  text: string;
  /** Authored caption/text, kept separate from diagram labels and table cells. */
  anchor: ReadingIndexAnchor;
  member_anchors: ReadingIndexAnchor[];
  region: TextRect | null;
  confidence: string;
  mentions: ObjectMention[];
};

export type ObjectsArtifact = {
  schema_version: number;
  paper_id: string;
  reading_index_generation: string;
  /** Detector build and input generation; informational in graded truth. */
  figure_detector_generation?: string;
  objects: PaperObject[];
};

/** One grader's judgement of the detector output; see eval/graded-objects-contract.md. */
export type ObjectVerdictKind = "correct" | "region" | "reject";
export type ObjectVerdict = { verdict: ObjectVerdictKind; region: TextRect | null; note: string };
export type ObjectCaption = { start: number; end: number };
export type ObjectAddition = {
  id: string; kind: "figure" | "table"; printed_label: string; page: number; region: TextRect;
  /** Half-open UTF-16 span in the reading-index text; null makes the paper unexportable. */
  caption: ObjectCaption | null; note: string;
};
export type ObjectGrades = {
  schema_version: number;
  paper_id: string;
  /** `reading_index_generation` without its surrounding quotes. */
  index_sha256: string;
  objects_generation: string;
  grader: string;
  updated_at: string | null;
  /** Revision loaded from the server; null until first saved. */
  revision: string | null;
  complete: boolean;
  verdicts: Record<string, ObjectVerdict>;
  additions: ObjectAddition[];
};
export type GradingStatus = "complete" | "partial" | "ungraded";
export type GradingQueue = {
  summary: Record<GradingStatus, number>;
  papers: { paper_id: string; title: string; stratum: string | null; status: GradingStatus; position: number }[];
};
export type GradingNext = { paper_id: string; title: string };

const paperRoute = (paperId: string) => `/api/papers/${encodeURIComponent(paperId)}`;
const missing = (reason: unknown) => reason instanceof ApiError && reason.status === 404;

export const objectsApi = {
  get: (paperId: string, signal?: AbortSignal): Promise<ObjectsArtifact> =>
    request(`${paperRoute(paperId)}/objects`, { signal }),
};

export const gradesApi = {
  /** Resolves null for `404 grades_not_found`. */
  get: async (paperId: string, signal?: AbortSignal): Promise<ObjectGrades | null> => {
    try { return await request<ObjectGrades>(`${paperRoute(paperId)}/objects/grades`, { signal }); }
    catch (reason: unknown) { if (missing(reason)) return null; throw reason; }
  },
  /** `grades.revision` must be the loaded revision; `409` reports a conflict or a stale index. */
  save: (paperId: string, grades: ObjectGrades): Promise<ObjectGrades> =>
    request(`${paperRoute(paperId)}/objects/grades`, { method: "PUT", body: JSON.stringify(grades) }),
  queue: (limit?: number): Promise<GradingQueue> =>
    request(`/api/grading/queue${limit === undefined ? "" : `?limit=${encodeURIComponent(limit)}`}`),
  /** Resolves null for `404 grading_queue_exhausted`. */
  next: async (after?: string): Promise<GradingNext | null> => {
    try { return await request<GradingNext>(`/api/grading/next${after === undefined ? "" : `?after=${encodeURIComponent(after)}`}`); }
    catch (reason: unknown) { if (missing(reason)) return null; throw reason; }
  },
};
