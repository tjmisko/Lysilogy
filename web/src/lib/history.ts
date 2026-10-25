// Client capture of reading history (spec: History, Capture). An event queue with a session
// id and an increasing client_seq, flushed every 15 s and on pagehide, plus a tracker that
// counts dwell only while the reader is actually active.
//
// Everything that touches the browser is injected, so the queue and tracker run under Node.

import type { HistoryEventDraft, HistoryEventInput } from "../types";

/** Dwell counts only with input or scrolling in the last minute. */
export const ACTIVE_IDLE_MS = 60_000;
export const FLUSH_INTERVAL_MS = 15_000;
/** Events kept while the server is unreachable; the oldest go first. */
export const QUEUE_CAP = 2000;
/** The server refuses larger batches. */
export const BATCH_MAX = 200;
/** A Work panel counts as inspected after this long. */
export const INSPECT_MIN_MS = 2000;
/** The reading position is saved at most this often. */
export const POSITION_DEBOUNCE_MS = 2000;
export const STORAGE_KEY = "lysilogy.history.queue";

export type QueueStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

export type HistoryEnvironment = {
  /** Milliseconds since the epoch. */
  now: () => number;
  /** sessionStorage, or null when unavailable. Every access is guarded. */
  storage: QueueStorage | null;
  newSessionId: () => string;
  /** Rejects when the batch did not reach the server. */
  send: (events: HistoryEventInput[]) => Promise<unknown>;
  /** navigator.sendBeacon; returns false when the browser refused to queue the batch. */
  beacon: (events: HistoryEventInput[]) => boolean;
};

function keyOf(event: { session_id: string; client_seq: number }): string {
  return `${event.session_id}#${event.client_seq}`;
}

function isStoredEvent(value: unknown): value is HistoryEventInput {
  if (typeof value !== "object" || value === null) return false;
  const event = value as Record<string, unknown>;
  return typeof event.session_id === "string" && typeof event.client_seq === "number"
    && Number.isInteger(event.client_seq) && typeof event.kind === "string" && typeof event.occurred_at === "string";
}

/** Drop repeated (session, seq) pairs, keeping the first, then the oldest beyond the cap. */
function normalized(events: readonly HistoryEventInput[]): HistoryEventInput[] {
  const seen = new Set<string>();
  const unique = events.filter((event) => {
    const key = keyOf(event);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return unique.length > QUEUE_CAP ? unique.slice(unique.length - QUEUE_CAP) : unique;
}

export class HistoryQueue {
  readonly sessionId: string;
  private readonly env: HistoryEnvironment;
  private events: HistoryEventInput[];
  private seq = 0;
  private flushing = false;
  /** The batch being sent. Still persisted, so a page that dies mid-send resends it. */
  private inflight: HistoryEventInput[] = [];

  constructor(env: HistoryEnvironment) {
    this.env = env;
    this.sessionId = env.newSessionId();
    this.events = this.load();
  }

  /** Events waiting to be sent. */
  get pending(): number {
    return this.events.length;
  }

  /** Give the waiting events of `paperId` that lack a Work the Work `workId`. */
  attachWork(paperId: string, workId: string): void {
    const waiting = (event: HistoryEventInput): boolean => event.paper_id === paperId && event.work_id === undefined;
    if (!this.events.some(waiting)) return;
    this.events = this.events.map((event) => (waiting(event) ? { ...event, work_id: workId } : event));
    this.save();
  }

  /** A copy of the waiting events, oldest first. */
  snapshot(): HistoryEventInput[] {
    return [...this.events];
  }

  /** Queue an event stamped with the current time, this session and the next sequence number. */
  record(draft: HistoryEventDraft): HistoryEventInput {
    const event: HistoryEventInput = {
      ...draft,
      occurred_at: new Date(this.env.now()).toISOString(),
      session_id: this.sessionId,
      client_seq: this.seq,
    };
    this.seq += 1;
    if (event.dwell_ms !== undefined) event.dwell_ms = Math.max(0, Math.round(event.dwell_ms));
    this.events = normalized([...this.events, event]);
    this.save();
    return event;
  }

  /**
   * Send everything queued, oldest first, in batches of at most BATCH_MAX. A failed batch goes
   * back to the front of the queue and the flush stops, so nothing is lost or reordered.
   * Returns the number of events the server received. Overlapping calls do nothing.
   */
  async flush(): Promise<number> {
    if (this.flushing) return 0;
    this.flushing = true;
    let sent = 0;
    try {
      while (this.events.length > 0) {
        const batch = this.events.slice(0, BATCH_MAX);
        this.events = this.events.slice(batch.length);
        this.inflight = batch;
        this.save();
        try {
          await this.env.send(batch);
        } catch {
          this.inflight = [];
          this.requeue(batch);
          return sent;
        }
        this.inflight = [];
        this.save();
        sent += batch.length;
      }
      return sent;
    } finally {
      this.flushing = false;
    }
  }

  /**
   * For pagehide: hand every batch to the beacon. A batch the browser refuses stays queued
   * (and in sessionStorage) for the next load to send. Returns the number handed over.
   */
  flushBeacon(): number {
    let handed = 0;
    while (this.events.length > 0) {
      const batch = this.events.slice(0, BATCH_MAX);
      let accepted = false;
      try {
        accepted = this.env.beacon(batch);
      } catch {
        accepted = false;
      }
      if (!accepted) break;
      this.events = this.events.slice(batch.length);
      handed += batch.length;
    }
    this.save();
    return handed;
  }

  /** Put events back ahead of anything recorded meanwhile; duplicates collapse, the cap applies. */
  private requeue(batch: readonly HistoryEventInput[]): void {
    this.events = normalized([...batch, ...this.events]);
    this.save();
  }

  private load(): HistoryEventInput[] {
    const storage = this.env.storage;
    if (storage === null) return [];
    try {
      const text = storage.getItem(STORAGE_KEY);
      if (text === null) return [];
      const parsed: unknown = JSON.parse(text);
      return Array.isArray(parsed) ? normalized(parsed.filter(isStoredEvent)) : [];
    } catch {
      return [];
    }
  }

  private save(): void {
    const storage = this.env.storage;
    if (storage === null) return;
    const stored = [...this.inflight, ...this.events];
    try {
      if (stored.length === 0) storage.removeItem(STORAGE_KEY);
      else storage.setItem(STORAGE_KEY, JSON.stringify(stored));
    } catch {
      // Quota or privacy mode: the in-memory queue still works.
    }
  }
}

/**
 * Active dwell of one visit. Time counts only while the tab is visible and focused and the
 * last input or scroll was at most `idleMs` ago. Clock readings that go backwards add nothing.
 */
export class DwellTracker {
  private readonly idleMs: number;
  private running = false;
  private visible = true;
  private focused = true;
  private lastInput = 0;
  private mark = 0;
  private total = 0;

  constructor(idleMs = ACTIVE_IDLE_MS) {
    this.idleMs = idleMs;
  }

  /** Begin counting at `at`; opening a paper counts as input. */
  start(at: number): void {
    this.running = true;
    this.total = 0;
    this.lastInput = at;
    this.mark = at;
  }

  /** Input, scroll or page turn at `at`. */
  input(at: number): void {
    if (!this.running) return;
    this.account(at);
    this.lastInput = Math.max(this.lastInput, at);
  }

  setVisible(visible: boolean, at: number): void {
    if (this.running) this.account(at);
    this.visible = visible;
  }

  setFocused(focused: boolean, at: number): void {
    if (this.running) this.account(at);
    this.focused = focused;
  }

  /** Active milliseconds so far; does not change the tracker. */
  dwell(at: number): number {
    return Math.round(this.total + this.activeBetween(at));
  }

  /** Stop and return the active milliseconds. Starting again resets the count. */
  stop(at: number): number {
    if (!this.running) return Math.round(this.total);
    this.account(at);
    this.running = false;
    return Math.round(this.total);
  }

  private activeBetween(at: number): number {
    if (!this.running || !this.visible || !this.focused) return 0;
    const until = Math.min(at, this.lastInput + this.idleMs);
    return Math.max(0, until - this.mark);
  }

  private account(at: number): void {
    if (at < this.mark) return;
    this.total += this.activeBetween(at);
    this.mark = at;
  }
}

/**
 * One visit to a paper: dwell, and the events that fire once per visit. `end` records
 * `paper_left` with the visit's dwell and cannot run twice.
 */
export class PaperVisit {
  private readonly queue: HistoryQueue;
  private readonly env: Pick<HistoryEnvironment, "now">;
  private readonly tracker: DwellTracker;
  private readonly base: Pick<HistoryEventDraft, "paper_id" | "work_id">;
  private readonly pages = new Set<number>();
  private ended = false;

  constructor(
    queue: HistoryQueue,
    env: Pick<HistoryEnvironment, "now">,
    base: Pick<HistoryEventDraft, "paper_id" | "work_id">,
  ) {
    this.queue = queue;
    this.env = env;
    this.base = base;
    this.tracker = new DwellTracker();
    this.tracker.start(env.now());
    queue.record({ ...base, kind: "paper_opened" });
  }

  input(): void {
    this.tracker.input(this.env.now());
  }

  setVisible(visible: boolean): void {
    this.tracker.setVisible(visible, this.env.now());
  }

  setFocused(focused: boolean): void {
    this.tracker.setFocused(focused, this.env.now());
  }

  dwell(): number {
    return this.tracker.dwell(this.env.now());
  }

  /** `page_reached`, at most once per page per visit. Returns whether it was recorded. */
  pageReached(page: number): boolean {
    if (this.ended || !Number.isInteger(page) || page < 1 || this.pages.has(page)) return false;
    this.pages.add(page);
    this.input();
    this.queue.record({ ...this.base, kind: "page_reached", detail: { page } });
    return true;
  }

  /** Record `paper_left`. Returns the dwell, or null when the visit had already ended. */
  end(): number | null {
    if (this.ended) return null;
    this.ended = true;
    const dwell = this.tracker.stop(this.env.now());
    this.queue.record({ ...this.base, kind: "paper_left", dwell_ms: dwell });
    return dwell;
  }
}

/** Whether a Work panel open for `openMs` counts as `work_inspected`. */
export function isInspection(openMs: number): boolean {
  return openMs >= INSPECT_MIN_MS;
}

export type Timers = {
  setTimeout: (callback: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
};

/**
 * Trailing debounce for position saves: the latest value is saved `delayMs` after the last
 * call. `flushNow` saves a waiting value at once (page leave); `cancel` drops it.
 */
export class Debouncer<T> {
  private handle: unknown = null;
  private waiting: { value: T } | null = null;
  private readonly save: (value: T) => void;
  private readonly timers: Timers;
  private readonly delayMs: number;

  constructor(save: (value: T) => void, timers: Timers, delayMs = POSITION_DEBOUNCE_MS) {
    this.save = save;
    this.timers = timers;
    this.delayMs = delayMs;
  }

  push(value: T): void {
    this.waiting = { value };
    if (this.handle !== null) this.timers.clearTimeout(this.handle);
    this.handle = this.timers.setTimeout(() => {
      this.flushNow();
    }, this.delayMs);
  }

  flushNow(): void {
    if (this.handle !== null) this.timers.clearTimeout(this.handle);
    this.handle = null;
    const waiting = this.waiting;
    this.waiting = null;
    if (waiting !== null) this.save(waiting.value);
  }

  cancel(): void {
    if (this.handle !== null) this.timers.clearTimeout(this.handle);
    this.handle = null;
    this.waiting = null;
  }
}

/** The browser environment: sessionStorage in try/catch, fetch for sends, sendBeacon for pagehide. */
export function browserHistoryEnvironment(
  send: (events: HistoryEventInput[]) => Promise<unknown>,
): HistoryEnvironment {
  let storage: QueueStorage | null = null;
  try {
    storage = window.sessionStorage;
  } catch {
    storage = null;
  }
  return {
    now: () => Date.now(),
    storage,
    newSessionId: () => crypto.randomUUID(),
    send,
    beacon: (events) => {
      if (typeof navigator.sendBeacon !== "function") return false;
      // text/plain keeps the beacon a simple request; the server does not check the type.
      return navigator.sendBeacon("/api/history/events", new Blob([JSON.stringify({ events })], { type: "text/plain" }));
    },
  };
}

/**
 * Start the 15 s flush and the pagehide beacon. Returns a function that stops both.
 * Call once per page load.
 */
export function attachHistoryFlush(queue: HistoryQueue): () => void {
  const timer = window.setInterval(() => {
    void queue.flush();
  }, FLUSH_INTERVAL_MS);
  const onPageHide = (): void => {
    queue.flushBeacon();
  };
  window.addEventListener("pagehide", onPageHide);
  void queue.flush();
  return () => {
    window.clearInterval(timer);
    window.removeEventListener("pagehide", onPageHide);
  };
}
