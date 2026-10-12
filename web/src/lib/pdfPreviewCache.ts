type PreviewStorage = {
  read: (url: string) => Promise<string | undefined>;
  write: (url: string, image: string) => Promise<void>;
};
type CacheOptions = {
  render: (url: string) => Promise<string>;
  storage?: PreviewStorage;
  concurrency?: number;
  maxBytes?: number;
  maxEntries?: number;
};
type Priority = () => number;

/** The cache owns work, so scrolling, filtering and navigation cannot discard a render. */
export function createPdfPreviewCache({ render, storage, concurrency = 3, maxBytes = 32 * 1024 * 1024, maxEntries = 512 }: CacheOptions) {
  const images = new Map<string, string>();
  const pending = new Map<string, { promise: Promise<string>; priorities: Set<Priority> }>();
  const queue: Array<{ priorities: Set<Priority>; start: () => void }> = [];
  let bytes = 0;
  let rendering = 0;

  const peek = (url: string): string | undefined => {
    const image = images.get(url);
    if (image !== undefined) { images.delete(url); images.set(url, image); }
    return image;
  };
  const remember = (url: string, image: string): string => {
    const previous = images.get(url);
    if (previous !== undefined) { bytes -= previous.length * 2; images.delete(url); }
    if (image.length * 2 > maxBytes || maxEntries <= 0) return image;
    images.set(url, image);
    bytes += image.length * 2;
    while (images.size > maxEntries || bytes > maxBytes) {
      const oldest = images.keys().next().value;
      if (oldest === undefined) break;
      bytes -= (images.get(oldest)?.length ?? 0) * 2;
      images.delete(oldest);
    }
    return image;
  };
  const drain = () => {
    // Re-evaluate positions when a slot opens: a fast jump must not leave the
    // newly visible row waiting behind all the rows the user just passed.
    const ranked = queue.map((job) => ({ job, priority: Math.min(...Array.from(job.priorities, (priority) => priority())) }));
    ranked.sort((a, b) => a.priority - b.priority);
    for (const { job } of ranked) {
      if (rendering >= Math.max(1, concurrency)) break;
      queue.splice(queue.indexOf(job), 1);
      rendering += 1;
      job.start();
    }
  };
  const load = (url: string, priority: Priority = () => 0): Promise<string> => {
    const cached = peek(url);
    if (cached !== undefined) return Promise.resolve(cached);
    const existing = pending.get(url);
    if (existing !== undefined) { existing.priorities.add(priority); return existing.promise; }
    const priorities = new Set([priority]);
    const promise = (async () => {
      // Disk hits bypass the PDF render queue, even when slow PDFs occupy it.
      const saved = await storage?.read(url).catch(() => undefined);
      if (saved !== undefined) return remember(url, saved);
      const image = await new Promise<string>((resolve, reject) => {
        queue.push({ priorities, start: () => {
          void Promise.resolve().then(() => render(url)).then(resolve, reject).finally(() => {
            rendering -= 1;
            drain();
          });
        } });
        queueMicrotask(drain);
      });
      remember(url, image);
      // Persistence never delays displaying the completed thumbnail.
      void storage?.write(url, image).catch(() => {});
      return image;
    })().finally(() => { pending.delete(url); });
    pending.set(url, { promise, priorities });
    return promise;
  };
  return { peek, load };
}
