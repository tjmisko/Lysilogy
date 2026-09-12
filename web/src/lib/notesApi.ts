import { request } from "./api";

export type NoteDocument = { filename: string; text: string; revision: string | null };

export const notesApi = {
  read: (paperId: string, signal?: AbortSignal): Promise<NoteDocument> =>
    request(`/api/papers/${encodeURIComponent(paperId)}/notes`, { signal }),
  save: (paperId: string, text: string, revision: string | null): Promise<NoteDocument> =>
    request(`/api/papers/${encodeURIComponent(paperId)}/notes`, {
      method: "PUT", body: JSON.stringify({ text, revision }),
    }),
};
