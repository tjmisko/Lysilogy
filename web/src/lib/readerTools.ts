import type { AnalysisProvider } from "../types";
import { request } from "./api";

export type CutFormat = "ten_paragraphs" | "six_pages";
export type CutSegment = { kind: "source" | "connector"; text: string; source_page: number | null };
export type CutParagraph = { cut_page: number; segments: CutSegment[] };
export type Supercut = {
  id: string; format: CutFormat; agent: string; provider: AnalysisProvider; created_at: string;
  source_words: number; total_words: number; paragraphs: CutParagraph[];
};
export type ReferenceConnection = {
  verdict: "supports" | "contradicts" | "qualifies" | "unclear" | "context";
  connector: string; limitation: string;
  evidence: Array<{ paper_id: string; source_page: number; quote: string }>;
};
export type SavedReference = {
  id: string; citation: string; source_page: number | null; note: string; created_at: string;
  candidate: { title: string; landing_url: string | null; pdf_url: string | null; explanation: string } | null;
  linked_paper_id: string | null; connection: ReferenceConnection | null; question: string | null;
};
export type ToolAction =
  | { kind: "supercut"; format: CutFormat }
  | { kind: "find_reference"; reference_id: string }
  | { kind: "connect_reference"; reference_id: string; question: string };
export type ToolJob = {
  id: string; action: ToolAction; provider: AnalysisProvider;
  status: "running" | "completed" | "failed"; created_at: string; error: string | null;
};
export type ReaderTools = { supercuts: Supercut[]; references: SavedReference[]; jobs: ToolJob[] };
const paperPath = (id: string): string => `/api/papers/${id}`;

export const readerToolsApi = {
  load: (id: string, signal?: AbortSignal): Promise<ReaderTools> =>
    request(`${paperPath(id)}/reader-tools`, { signal }),
  start: (id: string, action: ToolAction, provider: AnalysisProvider): Promise<ToolJob> =>
    request(`${paperPath(id)}/reader-tools/jobs`, { method: "POST", body: JSON.stringify({ action, provider }) }),
  save: (id: string, citation: string, source_page: number | null, note: string): Promise<SavedReference> =>
    request(`${paperPath(id)}/references`, { method: "POST", body: JSON.stringify({ citation, source_page, note }) }),
  link: (id: string, referenceId: string, linked_paper_id: string): Promise<ReaderTools> =>
    request(`${paperPath(id)}/references/${encodeURIComponent(referenceId)}`, { method: "PATCH", body: JSON.stringify({ linked_paper_id }) }),
  remove: async (id: string, referenceId: string): Promise<void> => {
    const response = await fetch(`${paperPath(id)}/references/${encodeURIComponent(referenceId)}`, { method: "DELETE" });
    if (!response.ok) {
      const error = await response.json() as { message?: string };
      throw new Error(error.message ?? "Could not remove reference");
    }
  },
};

export function cutMarkdown(cut: Supercut, title: string): string {
  const ratio = Math.floor(100 * cut.source_words / cut.total_words);
  let page = 0;
  const paragraphs = cut.paragraphs.map((paragraph) => {
    let heading = "";
    if (cut.format === "six_pages" && page !== paragraph.cut_page) {
      page = paragraph.cut_page;
      heading = `## Cut page ${page}\n\n`;
    }
    return heading + paragraph.segments.map((segment) => segment.kind === "connector"
      ? `[Lysilogos connector: ${segment.text}]`
      : `${segment.text} [PDF p. ${segment.source_page}]`).join(" ");
  });
  return `# ${title} — Supercut\n\nBy ${cut.agent} · ${ratio}% exact source words · ${cut.total_words} words\n\n${paragraphs.join("\n\n")}\n`;
}

export function downloadText(filename: string, text: string, mime = "text/markdown"): void {
  const url = URL.createObjectURL(new Blob([text], { type: mime }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
