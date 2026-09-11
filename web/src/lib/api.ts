import type {
  AnalysisProvider,
  Clarification,
  LibraryResponse,
  AnalysisJob,
  PaperOverview,
  Highlight,
  HighlightKind,
  ImportPdfResponse,
  PaperMap,
  PaperView,
  ProcessingQueue,
  ExperimentCatalog,
  ExperimentJudgment,
  ExperimentView,
} from "../types";

type ErrorPayload = {
  error?: string;
  message?: string;
};

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

async function apiError(response: Response): Promise<ApiError> {
  let payload: ErrorPayload = {};
  try {
    payload = (await response.json()) as ErrorPayload;
  } catch {
    // The status text remains a useful fallback for a non-JSON proxy error.
  }
  return new ApiError(response.status, payload.message ?? response.statusText);
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  headers.set("Accept", "application/json");
  if (init?.body !== undefined) headers.set("Content-Type", "application/json");
  const response = await fetch(path, {
    ...init,
    headers,
  });
  if (!response.ok) {
    throw await apiError(response);
  }
  return (await response.json()) as T;
}

export const api = {
  library: (): Promise<LibraryResponse> => request("/api/library"),
  scan: (): Promise<LibraryResponse> =>
    request("/api/library/scan", { method: "POST" }),
  importPdf: (url: string): Promise<ImportPdfResponse> =>
    request("/api/library/import", {
      method: "POST",
      body: JSON.stringify({ url }),
    }),
  queue: (): Promise<ProcessingQueue> => request("/api/queue"),
  experimentCatalog: (): Promise<ExperimentCatalog> => request("/api/experiments"),
  experiments: (id: string): Promise<ExperimentView[]> =>
    request(`/api/papers/${id}/experiments`),
  experiment: (id: string, runId: string): Promise<ExperimentView> =>
    request(`/api/papers/${id}/experiments/${encodeURIComponent(runId)}`),
  startExperiment: (
    id: string,
    experimentId: string,
    provider: AnalysisProvider,
  ): Promise<ExperimentView> =>
    request(`/api/papers/${id}/experiments`, {
      method: "POST",
      body: JSON.stringify({ experiment_id: experimentId, provider }),
    }),
  judgeExperiment: (
    id: string,
    runId: string,
    judgment: ExperimentJudgment,
  ): Promise<ExperimentView> =>
    request(`/api/papers/${id}/experiments/${encodeURIComponent(runId)}/judgment`, {
      method: "POST",
      body: JSON.stringify(judgment),
    }),
  paper: (id: string): Promise<PaperView> => request(`/api/papers/${id}`),
  paperMap: (id: string, signal?: AbortSignal): Promise<PaperMap> =>
    request(`/api/papers/${id}/map`, { signal }),
  analyze: (
    id: string,
    provider: AnalysisProvider,
    force = false,
  ): Promise<PaperOverview> =>
    request(`/api/papers/${id}/analyze`, {
      method: "POST",
      body: JSON.stringify({ provider, force }),
    }),
  refreshComponent: (id: string, provider: AnalysisProvider, component: "abstract" | "context" | "structure"): Promise<PaperView> =>
    request(`/api/papers/${id}/${component}/refresh`, {
      method: "POST", body: JSON.stringify({ provider, force: true }),
    }),
  feedback: (
    id: string,
    feedback: string,
    provider: AnalysisProvider,
  ): Promise<AnalysisJob> =>
    request(`/api/papers/${id}/feedback`, {
      method: "POST",
      body: JSON.stringify({ feedback, provider }),
    }),
  clarify: (
    id: string,
    sectionId: string,
    selection: string,
    question: string,
    provider: AnalysisProvider,
  ): Promise<Clarification> =>
    request(`/api/papers/${id}/clarify`, {
      method: "POST",
      body: JSON.stringify({
        section_id: sectionId,
        selection,
        question,
        provider,
      }),
    }),
  source: (id: string): string => `/api/papers/${id}/source`,
  createHighlight: (
    id: string,
    startSentenceId: string,
    endSentenceId: string | null = null,
    kind: HighlightKind = "note",
    note = "",
  ): Promise<Highlight> =>
    request(`/api/papers/${id}/highlights`, {
      method: "POST",
      body: JSON.stringify({
        start_sentence_id: startSentenceId,
        end_sentence_id: endSentenceId,
        kind,
        note,
      }),
    }),
  deleteHighlight: async (id: string, highlightId: string): Promise<void> => {
    const response = await fetch(
      `/api/papers/${id}/highlights/${encodeURIComponent(highlightId)}`,
      { method: "DELETE", headers: { Accept: "application/json" } },
    );
    if (!response.ok) throw await apiError(response);
  },
  markdown: async (id: string, signal?: AbortSignal): Promise<string> => {
    const response = await fetch(`/api/papers/${id}/markdown`, {
      headers: { Accept: "text/markdown" },
      signal,
    });
    if (!response.ok) throw await apiError(response);
    return response.text();
  },
};
