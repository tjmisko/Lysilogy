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
  EdgeLabel,
  EdgeLabelEdit,
  AcquisitionJobState,
  AcquisitionStatus,
  FetchWorkOptions,
  GraphNeighbors,
  LiteratureJob,
  LiteratureOptions,
  PaperWork,
  WorkCitations,
} from "../types";

export class ApiError extends Error {
  readonly status: number;
  readonly kind: "http" | "invalid_response";

  constructor(status: number, message: string, kind: "http" | "invalid_response" = "http") {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.kind = kind;
  }
}

function responseError(response: Response, description: string): ApiError {
  return new ApiError(response.status, `${description} Restart the Lysilogy backend and check that the frontend’s /api proxy reaches it.`, "invalid_response");
}

async function responsePayload(response: Response): Promise<unknown> {
  const body = await response.text();
  // Older servers may serve the app shell for an unknown API endpoint with 200.
  // Report that deployment mismatch without exposing HTML or a JSON parser error.
  if (response.headers.get("Content-Type")?.toLowerCase().includes("text/html") === true || /^\s*</u.test(body)) {
    throw responseError(response, "The API returned a web page instead of JSON.");
  }
  try { return JSON.parse(body) as unknown; }
  catch { throw responseError(response, "The API returned an invalid JSON response."); }
}

function httpError(response: Response, payload: unknown): ApiError {
  const message = typeof payload === "object" && payload !== null && "message" in payload && typeof payload.message === "string"
    ? payload.message : `API request failed (HTTP ${response.status}).`;
  return new ApiError(response.status, message);
}

async function apiError(response: Response): Promise<ApiError> {
  try { return httpError(response, await responsePayload(response)); }
  catch (reason: unknown) {
    if (reason instanceof ApiError) return reason;
    throw reason;
  }
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  headers.set("Accept", "application/json");
  if (init?.body !== undefined) headers.set("Content-Type", "application/json");
  const response = await fetch(path, { ...init, headers });
  const payload = await responsePayload(response);
  if (!response.ok) {
    throw httpError(response, payload);
  }
  return payload as T;
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
  paperWork: (id: string, signal?: AbortSignal): Promise<PaperWork> =>
    request(`/api/papers/${id}/work`, { signal }),
  startLiterature: (id: string, options?: LiteratureOptions): Promise<LiteratureJob> =>
    request(`/api/papers/${id}/lit`, {
      method: "POST",
      ...(options === undefined ? {} : { body: JSON.stringify(options) }),
    }),
  acceptLabel: (claimId: string): Promise<EdgeLabel> =>
    request(`/api/relationship-claims/${encodeURIComponent(claimId)}/accept`, { method: "POST" }),
  rejectLabel: (claimId: string): Promise<EdgeLabel> =>
    request(`/api/relationship-claims/${encodeURIComponent(claimId)}/reject`, { method: "POST" }),
  editLabel: (claimId: string, edit: EdgeLabelEdit): Promise<EdgeLabel> =>
    request(`/api/relationship-claims/${encodeURIComponent(claimId)}`, {
      method: "PUT",
      body: JSON.stringify(edit),
    }),
  literatureJob: (id: string, signal?: AbortSignal): Promise<LiteratureJob> =>
    request(`/api/papers/${id}/lit`, { signal }),
  workCitations: (workId: string, signal?: AbortSignal): Promise<WorkCitations> =>
    request(`/api/works/${encodeURIComponent(workId)}/citations`, { signal }),
  fetchWork: (workId: string, options?: FetchWorkOptions): Promise<{ state: AcquisitionJobState }> =>
    request(`/api/works/${encodeURIComponent(workId)}/fetch`, {
      method: "POST",
      ...(options === undefined ? {} : { body: JSON.stringify(options) }),
    }),
  workAcquisition: (workId: string, signal?: AbortSignal): Promise<AcquisitionStatus> =>
    request(`/api/works/${encodeURIComponent(workId)}/acquisition`, { signal }),
  graphNeighbors: (
    workId: string,
    direction: GraphNeighbors["direction"] = "references",
    signal?: AbortSignal,
  ): Promise<GraphNeighbors> =>
    request(`/api/graph/neighbors/${encodeURIComponent(workId)}?direction=${direction}`, { signal }),
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
    sectionId: string | null,
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
