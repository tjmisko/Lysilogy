import { useEffect, useState } from "react";

import { api } from "./api";

const MAX_LOOKUPS = 60;
/** Titles found so far, shared by every list page; null remembers a Work that could not be read. */
const known = new Map<string, string | null>();
const inflight = new Set<string>();

/**
 * Titles of Works outside the list that a row mentions (the papers a context link points at).
 * Looked up once each and cached; Works that cannot be read stay untitled.
 */
export function useWorkTitles(workIds: readonly string[]): Record<string, string> {
  // A bump re-renders the page once a title arrives; the titles themselves live in `known`.
  const [, setVersion] = useState(0);
  const key = [...new Set(workIds)].slice(0, MAX_LOOKUPS).join(",");

  useEffect(() => {
    const ids = key === "" ? [] : key.split(",");
    const controller = new AbortController();
    for (const id of ids) {
      if (known.has(id) || inflight.has(id)) continue;
      inflight.add(id);
      void api.workCitations(id, controller.signal).then((result) => {
        known.set(id, result.work.display_title);
      }).catch(() => {
        if (!controller.signal.aborted) known.set(id, null);
      }).finally(() => {
        inflight.delete(id);
        if (!controller.signal.aborted) setVersion((now) => now + 1);
      });
    }
    return () => controller.abort();
  }, [key]);

  const found: Record<string, string> = {};
  for (const id of key === "" ? [] : key.split(",")) {
    const title = known.get(id);
    if (typeof title === "string") found[id] = title;
  }
  return found;
}
