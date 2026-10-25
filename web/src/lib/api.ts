import { neighborQuery, type NeighborOptions } from "./relevance.ts";
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
  ExpandWorkOptions,
  ExpansionJob,
  ExpansionStatus,
  AcquisitionStatus,
  FetchWorkOptions,
  GraphNeighbors,
  MarkView,
  TagView,
  WorkMarkName,
  LiteratureJob,
  LiteratureOptions,
  PaperWork,
  ProvenanceView,
  RampDetail,
  RampItemAdd,
  RampItemPatch,
  RampItemView,
  RampOrderRequest,
  RampSummaryView,
  WorkCitations,
  BarelyReadPaper,
  ExpansionDirection,
  GenerateRequest,
  GenerationStatus,
  HistoryEventInput,
  HistoryIngestResult,
  HistoryQuery,
  HistoryTimeline,
  InspectedWork,
  ListCreate,
  ListDetail,
  ListEdgeRef,
  ListEdgeView,
  ListNodeAdd,
  ListNodePatch,
  ListNodeView,
  ListOrderRequest,
  ListPatch,
  ListRecord,
  ListSummaryView,
  ListsQuery,
  PaperHistorySummary,
  PaperMemberships,
  ReadingPosition,
  ReadingPositionInput,
  ReadingStatus,
  RemovedListNode,
  VerifyReport,
  WorkHit,
  WorkMemberships,
  WorkReadingState,
} from "../types";

export class ApiError extends Error {
  readonly status: number;
  readonly kind: "http" | "invalid_response";
  /** The server's machine-readable error code, such as `edge_cycle` or `item_exists`. */
  readonly code: string | null;
  /** The node ids of a 409 `edge_cycle`, in cycle order. */
  readonly cycle: string[] | null;

  constructor(
    status: number,
    message: string,
    kind: "http" | "invalid_response" = "http",
    detail: { code?: string | null; cycle?: string[] | null } = {},
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.kind = kind;
    this.code = detail.code ?? null;
    this.cycle = detail.cycle ?? null;
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
  const code = typeof payload === "object" && payload !== null && "error" in payload && typeof payload.error === "string"
    ? payload.error : null;
  const cycle = typeof payload === "object" && payload !== null && "cycle" in payload && Array.isArray(payload.cycle)
    ? payload.cycle.filter((id): id is string => typeof id === "string") : null;
  return new ApiError(response.status, message, "http", { code, cycle });
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

/** A request whose success is an empty 204, optionally with a JSON body. */
async function sendJsonWithoutResponse(path: string, method: "POST" | "PUT" | "PATCH" | "DELETE", body: unknown): Promise<void> {
  const response = await fetch(path, {
    method,
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw await apiError(response);
}

const segment = encodeURIComponent;

/** `?a=1&b=2` from the defined entries, or an empty string. */
function queryString(entries: Record<string, string | undefined>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(entries)) {
    if (value !== undefined) params.set(key, value);
  }
  const text = params.toString();
  return text === "" ? "" : `?${text}`;
}

async function sendWithoutBody(path: string, method: "PUT" | "DELETE"): Promise<void> {
  const response = await fetch(path, { method, headers: { Accept: "application/json" } });
  if (!response.ok) throw await apiError(response);
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
  uploadPdf: async (file: File): Promise<Omit<ImportPdfResponse, "source">> => {
    const response = await fetch(`/api/library/upload?filename=${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { Accept: "application/json", "Content-Type": "application/pdf" },
      body: file,
    });
    const payload = await responsePayload(response);
    if (!response.ok) throw httpError(response, payload);
    return payload as Omit<ImportPdfResponse, "source">;
  },
  addLibraryDirectory: (path: string): Promise<LibraryResponse> =>
    request("/api/library/directories", {
      method: "POST",
      body: JSON.stringify({ path }),
    }),
  removeLibraryDirectory: async (id: string): Promise<LibraryResponse> => {
    const response = await fetch(`/api/library/directories/${encodeURIComponent(id)}`, {
      method: "DELETE",
      headers: { Accept: "application/json" },
    });
    const payload = await responsePayload(response);
    if (!response.ok) throw httpError(response, payload);
    return payload as LibraryResponse;
  },
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
  expandWork: (workId: string, body?: ExpandWorkOptions): Promise<ExpansionJob> =>
    request(`/api/works/${encodeURIComponent(workId)}/expand`, {
      method: "POST",
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    }),
  expansionStatus: (workId: string, signal?: AbortSignal, direction?: ExpansionDirection): Promise<ExpansionStatus> =>
    request(`/api/works/${encodeURIComponent(workId)}/expansion${queryString({ direction })}`, { signal }),
  listRamps: (seed?: string, signal?: AbortSignal): Promise<{ ramps: RampSummaryView[] }> =>
    request(seed === undefined ? "/api/ramps" : `/api/ramps?seed=${encodeURIComponent(seed)}`, { signal }),
  createRamp: (seedWorkId: string, title?: string): Promise<RampDetail> =>
    request("/api/ramps", {
      method: "POST",
      body: JSON.stringify(title === undefined ? { seed_work_id: seedWorkId } : { seed_work_id: seedWorkId, title }),
    }),
  ramp: (rampId: string, signal?: AbortSignal): Promise<RampDetail> =>
    request(`/api/ramps/${encodeURIComponent(rampId)}`, { signal }),
  renameRamp: (rampId: string, title: string): Promise<RampSummaryView> =>
    request(`/api/ramps/${encodeURIComponent(rampId)}`, { method: "PATCH", body: JSON.stringify({ title }) }),
  deleteRamp: (rampId: string): Promise<void> => sendWithoutBody(`/api/ramps/${encodeURIComponent(rampId)}`, "DELETE"),
  addRampItem: (rampId: string, item: RampItemAdd): Promise<RampItemView> =>
    request(`/api/ramps/${encodeURIComponent(rampId)}/items`, { method: "POST", body: JSON.stringify(item) }),
  updateRampItem: (rampId: string, workId: string, patch: RampItemPatch): Promise<RampItemView> =>
    request(`/api/ramps/${encodeURIComponent(rampId)}/items/${encodeURIComponent(workId)}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  removeRampItem: (rampId: string, workId: string): Promise<void> =>
    sendWithoutBody(`/api/ramps/${encodeURIComponent(rampId)}/items/${encodeURIComponent(workId)}`, "DELETE"),
  orderRamp: (rampId: string, order: RampOrderRequest): Promise<RampDetail> =>
    request(`/api/ramps/${encodeURIComponent(rampId)}/order`, { method: "PUT", body: JSON.stringify(order) }),
  listLists: (query: ListsQuery = {}, signal?: AbortSignal): Promise<{ lists: ListSummaryView[] }> =>
    request(`/api/lists${queryString({
      work: query.work,
      archived: query.archived === undefined ? undefined : String(query.archived),
    })}`, { signal }),
  searchLists: (q: string, signal?: AbortSignal): Promise<{ lists: ListRecord[] }> =>
    request(`/api/lists/search${queryString({ q })}`, { signal }),
  createList: (list: ListCreate): Promise<ListDetail> =>
    request("/api/lists", { method: "POST", body: JSON.stringify(list) }),
  readList: (listId: string, signal?: AbortSignal): Promise<ListDetail> =>
    request(`/api/lists/${segment(listId)}`, { signal }),
  patchList: (listId: string, patch: ListPatch): Promise<ListSummaryView> =>
    request(`/api/lists/${segment(listId)}`, { method: "PATCH", body: JSON.stringify(patch) }),
  deleteList: (listId: string): Promise<void> => sendWithoutBody(`/api/lists/${segment(listId)}`, "DELETE"),
  addListNode: (listId: string, node: ListNodeAdd): Promise<ListNodeView> =>
    request(`/api/lists/${segment(listId)}/nodes`, { method: "POST", body: JSON.stringify(node) }),
  /** All or nothing; at most 100 nodes. */
  addListNodes: (listId: string, nodes: ListNodeAdd[]): Promise<{ nodes: ListNodeView[] }> =>
    request(`/api/lists/${segment(listId)}/nodes:batch`, { method: "POST", body: JSON.stringify({ nodes }) }),
  patchListNode: (listId: string, nodeId: string, patch: ListNodePatch): Promise<ListNodeView> =>
    request(`/api/lists/${segment(listId)}/nodes/${segment(nodeId)}`, { method: "PATCH", body: JSON.stringify(patch) }),
  /** Returns the removed node and its edges so the caller can undo. */
  removeListNode: (listId: string, nodeId: string): Promise<RemovedListNode> =>
    request(`/api/lists/${segment(listId)}/nodes/${segment(nodeId)}`, { method: "DELETE" }),
  /** A 409 `edge_cycle` rejects with an ApiError whose `cycle` names the nodes. */
  addListEdge: (listId: string, edge: ListEdgeRef): Promise<ListEdgeView> =>
    request(`/api/lists/${segment(listId)}/edges`, { method: "POST", body: JSON.stringify(edge) }),
  removeListEdge: (listId: string, edge: ListEdgeRef): Promise<void> =>
    sendJsonWithoutResponse(`/api/lists/${segment(listId)}/edges`, "DELETE", edge),
  acceptListEdge: (listId: string, edge: ListEdgeRef): Promise<void> =>
    sendJsonWithoutResponse(`/api/lists/${segment(listId)}/edges`, "PATCH", { ...edge, review: "accepted" }),
  orderList: (listId: string, order: ListOrderRequest): Promise<ListDetail> =>
    request(`/api/lists/${segment(listId)}/order`, { method: "PUT", body: JSON.stringify(order) }),
  verifyList: (listId: string): Promise<VerifyReport> =>
    request(`/api/lists/${segment(listId)}/verify`, { method: "POST" }),
  setReadingState: (workId: string, status: ReadingStatus): Promise<WorkReadingState> =>
    request(`/api/works/${segment(workId)}/reading-state`, { method: "PUT", body: JSON.stringify({ status }) }),
  workLists: (workId: string, signal?: AbortSignal): Promise<WorkMemberships> =>
    request(`/api/works/${segment(workId)}/lists`, { signal }),
  paperLists: (paperId: string, signal?: AbortSignal): Promise<PaperMemberships> =>
    request(`/api/papers/${segment(paperId)}/lists`, { signal }),
  searchWorks: (q: string, provider = false, signal?: AbortSignal): Promise<{ results: WorkHit[] }> =>
    request(`/api/works/search${queryString({ q, provider: provider ? "true" : undefined })}`, { signal }),
  generateList: (generation: GenerateRequest): Promise<{ generation_id: string }> =>
    request("/api/lists/generate", { method: "POST", body: JSON.stringify(generation) }),
  generation: (generationId: string, signal?: AbortSignal): Promise<GenerationStatus> =>
    request(`/api/generations/${segment(generationId)}`, { signal }),
  postHistoryEvents: (events: HistoryEventInput[]): Promise<HistoryIngestResult> =>
    request("/api/history/events", { method: "POST", body: JSON.stringify({ events }) }),
  history: (query: HistoryQuery = {}, signal?: AbortSignal): Promise<HistoryTimeline> =>
    request(`/api/history${queryString({
      before: query.before,
      kinds: query.kinds === undefined || query.kinds.length === 0 ? undefined : query.kinds.join(","),
      q: query.q,
      limit: query.limit === undefined ? undefined : String(query.limit),
    })}`, { signal }),
  historyInspected: (signal?: AbortSignal): Promise<{ works: InspectedWork[] }> =>
    request("/api/history/inspected", { signal }),
  historyBarelyRead: (signal?: AbortSignal): Promise<{ papers: BarelyReadPaper[] }> =>
    request("/api/history/barely-read", { signal }),
  paperHistory: (paperId: string, signal?: AbortSignal): Promise<PaperHistorySummary> =>
    request(`/api/history/papers/${segment(paperId)}`, { signal }),
  putPosition: (paperId: string, position: ReadingPositionInput): Promise<ReadingPosition> =>
    request(`/api/papers/${segment(paperId)}/position`, { method: "PUT", body: JSON.stringify(position) }),
  graphNeighbors: (
    workId: string,
    direction: GraphNeighbors["direction"] = "references",
    options: NeighborOptions = {},
    signal?: AbortSignal,
  ): Promise<GraphNeighbors> =>
    request(`/api/graph/neighbors/${encodeURIComponent(workId)}?${neighborQuery({ direction, ...options })}`, { signal }),
  workMark: (workId: string, signal?: AbortSignal): Promise<{ work_id: string; mark: MarkView | null }> =>
    request(`/api/works/${encodeURIComponent(workId)}/mark`, { signal }),
  setWorkMark: (workId: string, mark: WorkMarkName, note: string | null): Promise<{ work_id: string; mark: MarkView }> =>
    request(`/api/works/${encodeURIComponent(workId)}/mark`, {
      method: "PUT",
      body: JSON.stringify(note === null || note.trim() === "" ? { mark } : { mark, note }),
    }),
  clearWorkMark: async (workId: string): Promise<void> => {
    const response = await fetch(`/api/works/${encodeURIComponent(workId)}/mark`, {
      method: "DELETE", headers: { Accept: "application/json" },
    });
    if (!response.ok) throw await apiError(response);
  },
  listTags: (signal?: AbortSignal): Promise<{ tags: TagView[] }> => request("/api/tags", { signal }),
  createTag: (name: string, color: string): Promise<TagView> =>
    request("/api/tags", { method: "POST", body: JSON.stringify({ name, color }) }),
  updateTag: (tagId: string, patch: { name?: string; color?: string }): Promise<TagView> =>
    request(`/api/tags/${encodeURIComponent(tagId)}`, { method: "PATCH", body: JSON.stringify(patch) }),
  deleteTag: (tagId: string): Promise<void> => sendWithoutBody(`/api/tags/${encodeURIComponent(tagId)}`, "DELETE"),
  workTags: (workId: string, signal?: AbortSignal): Promise<{ work_id: string; tags: TagView[] }> =>
    request(`/api/works/${encodeURIComponent(workId)}/tags`, { signal }),
  assignTag: (workId: string, tagId: string): Promise<void> =>
    sendWithoutBody(`/api/works/${encodeURIComponent(workId)}/tags/${encodeURIComponent(tagId)}`, "PUT"),
  unassignTag: (workId: string, tagId: string): Promise<void> =>
    sendWithoutBody(`/api/works/${encodeURIComponent(workId)}/tags/${encodeURIComponent(tagId)}`, "DELETE"),
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
  workProvenance: (workId: string, signal?: AbortSignal): Promise<ProvenanceView> =>
    request(`/api/works/${encodeURIComponent(workId)}/provenance`, { signal }),
  setFieldOverride: (workId: string, field: string, value: unknown): Promise<ProvenanceView> =>
    request(`/api/works/${encodeURIComponent(workId)}/provenance/${encodeURIComponent(field)}`, {
      method: "PUT",
      body: JSON.stringify({ value }),
    }),
  clearFieldOverride: (workId: string, field: string): Promise<ProvenanceView> =>
    request(`/api/works/${encodeURIComponent(workId)}/provenance/${encodeURIComponent(field)}`, { method: "DELETE" }),
  reviewFieldClaim: (claimId: string, reviewState: "accepted" | "rejected" | "unreviewed"): Promise<ProvenanceView> =>
    request(`/api/field-claims/${encodeURIComponent(claimId)}/review`, {
      method: "PUT",
      body: JSON.stringify({ review_state: reviewState }),
    }),
};
