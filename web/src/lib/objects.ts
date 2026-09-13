import type { TextRect } from "../types";
import { request } from "./api.ts";

export type StatementKind = "theorem" | "lemma" | "proposition" | "corollary" | "definition" | "remark";

export type PaperObjectKind =
  | { kind: "figure" | "table" | "equation" | "algorithm" }
  | { kind: "bib_entry"; bibliography: BibliographicFields }
  | { kind: "statement"; statement_kind: StatementKind }
  | { kind: "proof"; statement_id: string | null };

/** Half-open UTF-16 offsets in the complete text of the artifact's reading-index generation. */
export type ReadingIndexAnchor = { page: number; start: number; end: number };
export type ObjectMention = { anchor: ReadingIndexAnchor; rects: TextRect[]; sentence_anchor?: ReadingIndexAnchor };
export type ParsedField<T> = { value: T | null; confidence: "missing" | "heuristic" | "explicit" };
export type BibliographicFields = {
  printed_key: ParsedField<string>; authors: ParsedField<string[]>; title: ParsedField<string>;
  year: ParsedField<string>; venue: ParsedField<string>; doi: ParsedField<string>; arxiv_id: ParsedField<string>;
};
export type UnresolvedCitation = ObjectMention & {
  text: string; key: string; style: "numeric" | "superscript" | "author_year";
  reason: "missing_target" | "ambiguous_target" | "unsupported_range" | "ambiguous_marker"; candidate_ids: string[];
};

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
  objects: PaperObject[];
  unresolved_citations: UnresolvedCitation[];
};

export const objectsApi = {
  get: (paperId: string, signal?: AbortSignal): Promise<ObjectsArtifact> =>
    request(`/api/papers/${encodeURIComponent(paperId)}/objects`, { signal, cache: "no-cache" }),
};

/** Restrict source-derived object requests to the current app's paper routes. */
export function sourcePaperId(source: string): string | null {
  const base = typeof location === "undefined" ? "http://localhost/" : location.href;
  const url = new URL(source, base);
  if (url.origin !== new URL(base).origin) return null;
  return /^\/api\/papers\/([a-f\d]{16})\/source$/u.exec(url.pathname)?.[1] ?? null;
}

/** Offsets can be used only with the exact index generation and paper. */
export function objectsMatchGeneration(value: unknown, generation: string | null, paperId?: string): boolean {
  if (typeof value !== "object" || value === null) return false;
  const artifact = value as Partial<ObjectsArtifact>;
  return artifact.schema_version === 2 && generation !== null
    && artifact.reading_index_generation === generation && (paperId === undefined || artifact.paper_id === paperId)
    && Array.isArray(artifact.objects) && Array.isArray(artifact.unresolved_citations) && artifact.objects.every(validObject);
}

function validObject(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const object = value as Partial<PaperObject>;
  return typeof object.kind === "string" && typeof object.id === "string" && typeof object.label === "string"
    && validAnchor(object.anchor) && Array.isArray(object.member_anchors) && object.member_anchors.every(validAnchor)
    && Array.isArray(object.mentions) && object.mentions.every(validMention);
}
function validMention(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const mention = value as Partial<ObjectMention>;
  return validAnchor(mention.anchor) && Array.isArray(mention.rects) && mention.rects.every(validRect);
}
function validAnchor(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const anchor = value as Partial<ReadingIndexAnchor>;
  return typeof anchor.page === "number" && Number.isInteger(anchor.page) && anchor.page > 0
    && typeof anchor.start === "number" && Number.isInteger(anchor.start) && anchor.start >= 0
    && typeof anchor.end === "number" && Number.isInteger(anchor.end) && anchor.end > anchor.start;
}
function validRect(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const rect = value as TextRect;
  return [rect.x_min, rect.x_max, rect.y_min, rect.y_max].every(Number.isFinite)
    && rect.x_min < rect.x_max && rect.y_min < rect.y_max;
}
