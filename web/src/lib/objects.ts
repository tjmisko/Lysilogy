import type { TextRect } from "../types";
import { request } from "./api.ts";

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
  objects: PaperObject[];
};

export const objectsApi = {
  get: (paperId: string, signal?: AbortSignal): Promise<ObjectsArtifact> =>
    request(`/api/papers/${encodeURIComponent(paperId)}/objects`, { signal }),
};
