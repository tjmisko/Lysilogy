import type { ReadingIndex } from "./readingIndex";

export type ReadingIndexLoadOptions = {
  priority?: "background" | "interactive";
  /** Validate once when opening a paper; subsequent operations can reuse it. */
  revalidate?: boolean;
};
type Entry = { index: ReadingIndex; etag: string | null; bytes: number; validated: boolean };
type CacheOptions = { maxEntries?: number; maxBytes?: number; fetcher?: typeof fetch; timeoutMs?: number };

function canonicalUrl(url: string): string {
  const resolved = new URL(url, typeof location === "undefined" ? "http://localhost/" : location.href);
  if (resolved.pathname.endsWith("/source")) resolved.pathname = resolved.pathname.slice(0, -7) + "/reading-index";
  resolved.searchParams.delete("priority");
  resolved.searchParams.sort();
  resolved.hash = "";
  return resolved.href;
}

function readingIndex(value: unknown): ReadingIndex {
  if (typeof value !== "object" || value === null) throw new Error("The source index response is invalid.");
  const index = value as Partial<Omit<ReadingIndex, "objects">> & { objects?: ReadingIndex["objects"] | null };
  if (typeof index.schema_version !== "number" || ![1, 2, 3].includes(index.schema_version)
    || typeof index.text !== "string" || !Array.isArray(index.tokens) || !Array.isArray(index.pages)
    || !Array.isArray(index.figures) || !Array.isArray(index.gaps) || index.objects === undefined || index.objects === null
    || !["word", "WORD", "sentence", "paragraph"].every((key) => Array.isArray(index.objects?.[key as keyof ReadingIndex["objects"]]))) {
    throw new Error("The source index is incompatible with this reader. Restart the Lysilogy backend to enable paper search.");
  }
  return index as ReadingIndex;
}

/** Conservative parsed-object estimate, including coordinate/range overhead. */
function estimatedBytes(index: ReadingIndex): number {
  let bytes = 256 + index.text.length * 2 + index.pages.length * 160;
  for (const token of index.tokens) bytes += 192 + token.text.length * 2 + token.rects.length * 80;
  for (const ranges of Object.values(index.objects)) bytes += ranges.length * 64;
  for (const figure of index.figures) bytes += 256 + (figure.label.length + figure.caption.length) * 2 + figure.references.length * 160;
  for (const gap of index.gaps) bytes += 64 + gap.reason.length * 2;
  return bytes;
}

/**
 * Shared ownership is deliberate: a component disappearing never aborts an
 * extraction or another component's wait. Browser HTTP caching persists the
 * response across reloads; this LRU avoids parsing it again within the tab.
 */
export function createReadingIndexCache({ maxEntries = 16, maxBytes = 64 * 1024 * 1024, timeoutMs = 5 * 60 * 1000, fetcher = (...args) => fetch(...args) }: CacheOptions = {}) {
  const entries = new Map<string, Entry>();
  const pending = new Map<string, Promise<ReadingIndex>>();
  const entryLimit = Math.max(0, Math.floor(maxEntries));
  const byteLimit = Math.max(0, maxBytes);
  let retainedBytes = 0;

  const touch = (key: string): Entry | undefined => {
    const entry = entries.get(key);
    if (entry !== undefined) { entries.delete(key); entries.set(key, entry); }
    return entry;
  };
  const remove = (key: string) => {
    const entry = entries.get(key);
    if (entry !== undefined) { retainedBytes -= entry.bytes; entries.delete(key); }
  };
  const remember = (key: string, index: ReadingIndex, etag: string | null): ReadingIndex => {
    const bytes = estimatedBytes(index);
    remove(key);
    // An oversized result remains available to its caller, but cannot displace
    // the bounded cache with an entry larger than its entire memory budget.
    if (bytes > byteLimit || entryLimit === 0) return index;
    entries.set(key, { index, etag, bytes, validated: true });
    retainedBytes += bytes;
    while (entries.size > entryLimit || retainedBytes > byteLimit) {
      const oldest = entries.keys().next().value;
      if (oldest === undefined) break;
      remove(oldest);
    }
    return index;
  };

  const fetchIndex = async (key: string, priority: "background" | "interactive", cached: Entry | undefined, signal: AbortSignal): Promise<ReadingIndex> => {
    const headers = new Headers({ Accept: "application/json", "X-Reading-Priority": priority });
    if (cached?.etag) headers.set("If-None-Match", cached.etag);
    const init: RequestInit = { cache: "no-cache", credentials: "same-origin", signal, headers, priority: priority === "background" ? "low" : "high" };
    let response = await fetcher(key, init);
    signal.throwIfAborted();
    // Browsers normally combine a 304 with their stored response and expose a
    // 200. Explicit conditionals can expose 304 when that HTTP body was evicted.
    if (response.status === 304) {
      if (cached !== undefined) return remember(key, cached.index, response.headers.get("etag") ?? cached.etag);
      headers.delete("If-None-Match");
      response = await fetcher(key, { ...init, cache: "reload" });
      signal.throwIfAborted();
      if (response.status === 304) throw new Error("The server returned an unchanged source index without its cached body. Try again.");
    }
    if (!response.ok) throw new Error(`Source indexing failed (${response.status}). Try again after extraction finishes.`);
    const index = readingIndex(await response.json() as unknown);
    signal.throwIfAborted();
    return remember(key, index, response.headers.get("etag"));
  };

  const peek = (url: string): ReadingIndex | null => touch(canonicalUrl(url))?.index ?? null;
  const load = (url: string, { priority = "interactive", revalidate = false }: ReadingIndexLoadOptions = {}): Promise<ReadingIndex> => {
    const key = canonicalUrl(url);
    const inFlight = pending.get(key);
    if (inFlight !== undefined) return inFlight;
    const cached = touch(key);
    if (cached?.validated && !revalidate) return Promise.resolve(cached.index);
    if (cached !== undefined) cached.validated = false;
    // The timeout belongs to the shared request, never to a mounted consumer.
    // A finite deadline also frees the pending slot if a fetch implementation
    // ignores AbortSignal; the signal checks prevent late results being stored.
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        controller.abort();
        reject(new Error("Source indexing timed out. Try again; completed backend extraction remains cached."));
      }, Math.max(1, timeoutMs));
    });
    const task = Promise.race([fetchIndex(key, priority, cached, controller.signal), deadline]).finally(() => {
      clearTimeout(timer);
      pending.delete(key);
    });
    pending.set(key, task);
    return task;
  };
  return { peek, load };
}

const cache = createReadingIndexCache();
export const peekReadingIndex = (url: string): ReadingIndex | null => cache.peek(url);
export const loadReadingIndex = (url: string, options?: ReadingIndexLoadOptions): Promise<ReadingIndex> => cache.load(url, options);
