import type {
  AnalysisProvider,
  Clarification,
  LibraryResponse,
  PaperOverview,
  PaperView,
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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  headers.set("Accept", "application/json");
  if (init?.body !== undefined) headers.set("Content-Type", "application/json");
  const response = await fetch(path, {
    ...init,
    headers,
  });
  if (!response.ok) {
    let payload: ErrorPayload = {};
    try {
      payload = (await response.json()) as ErrorPayload;
    } catch {
      // The status text remains a useful fallback for a non-JSON proxy error.
    }
    throw new ApiError(response.status, payload.message ?? response.statusText);
  }
  return (await response.json()) as T;
}

export const api = {
  library: (): Promise<LibraryResponse> => request("/api/library"),
  scan: (): Promise<LibraryResponse> =>
    request("/api/library/scan", { method: "POST" }),
  paper: (id: string): Promise<PaperView> => request(`/api/papers/${id}`),
  analyze: (
    id: string,
    provider: AnalysisProvider,
    force = false,
  ): Promise<PaperOverview> =>
    request(`/api/papers/${id}/analyze`, {
      method: "POST",
      body: JSON.stringify({ provider, force }),
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
};
