import { ApiError, request } from "./api";

export type NoteDocument = { filename: string; text: string; revision: string | null };

/** Keep the reader's local wall time and offset instead of converting to UTC. */
export function localCreatedAt(date = new Date()): string {
  const pad = (value: number): string => value.toString().padStart(2, "0");
  const offset = date.getTimezoneOffset();
  const hours = pad(Math.floor(Math.abs(offset) / 60));
  const minutes = pad(Math.abs(offset) % 60);
  return `${date.getFullYear().toString().padStart(4, "0")}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}${offset <= 0 ? "+" : "-"}${hours}:${minutes}`;
}

function noteDocument(value: unknown): NoteDocument {
  if (typeof value !== "object" || value === null || !("filename" in value) || typeof value.filename !== "string" || value.filename.length === 0
    || !("text" in value) || typeof value.text !== "string" || !("revision" in value) || !(value.revision === null || (typeof value.revision === "string" && value.revision.length > 0))) {
    throw new ApiError(200, "The notes response is incompatible with this reader. Restart the Lysilogy backend to load the current notes API.", "invalid_response");
  }
  return { filename: value.filename, text: value.text, revision: value.revision };
}

export const notesApi = {
  open: async (paperId: string, signal?: AbortSignal): Promise<NoteDocument> =>
    noteDocument(await request<unknown>(`/api/papers/${encodeURIComponent(paperId)}/notes/open`, {
      method: "POST", body: JSON.stringify({ created_at: localCreatedAt() }), signal,
    })),
  read: (paperId: string, signal?: AbortSignal): Promise<NoteDocument> =>
    request<unknown>(`/api/papers/${encodeURIComponent(paperId)}/notes`, { signal }).then(noteDocument),
  save: (paperId: string, text: string, revision: string | null): Promise<NoteDocument> =>
    request<unknown>(`/api/papers/${encodeURIComponent(paperId)}/notes`, {
      method: "PUT", body: JSON.stringify({ text, revision }),
    }).then(noteDocument),
};
