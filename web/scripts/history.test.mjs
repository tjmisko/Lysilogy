import assert from 'node:assert/strict';
import test from 'node:test';
import {
  ACTIVE_IDLE_MS, BATCH_MAX, Debouncer, DwellTracker, HistoryQueue, PaperVisit, QUEUE_CAP, STORAGE_KEY, isInspection,
} from '../src/lib/history.ts';

function memoryStorage(initial = {}) {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => { data.set(key, value); },
    removeItem: (key) => { data.delete(key); },
  };
}

function environment(over = {}) {
  const clock = { t: Date.UTC(2026, 9, 25, 12, 0, 0) };
  let sessions = 0;
  const env = {
    clock,
    sent: [],
    beaconed: [],
    failSend: false,
    beaconOk: true,
    now: () => clock.t,
    storage: memoryStorage(),
    newSessionId: () => `session-${++sessions}`,
    send: async (events) => {
      if (env.failSend) throw new Error('offline');
      env.sent.push(events);
    },
    beacon: (events) => {
      env.beaconed.push(events);
      return env.beaconOk;
    },
    ...over,
  };
  return env;
}

const draft = (n = 0) => ({ kind: 'search_run', detail: { n } });

test('should stamp session, time and increasing sequence numbers when recording', () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  const a = queue.record(draft(0));
  env.clock.t += 5000;
  const b = queue.record({ kind: 'paper_left', paper_id: 'p', dwell_ms: 1234.6 });
  assert.equal(a.session_id, 'session-1');
  assert.deepEqual([a.client_seq, b.client_seq], [0, 1]);
  assert.equal(a.occurred_at, '2026-10-25T12:00:00.000Z');
  assert.equal(b.occurred_at, '2026-10-25T12:00:05.000Z');
  assert.equal(b.dwell_ms, 1235);
});

test('should clamp negative dwell to zero when recording', () => {
  const queue = new HistoryQueue(environment());
  assert.equal(queue.record({ kind: 'paper_left', dwell_ms: -50 }).dwell_ms, 0);
});

test('should send queued events in order and empty the queue when the flush succeeds', async () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  queue.record(draft(0));
  queue.record(draft(1));
  assert.equal(await queue.flush(), 2);
  assert.deepEqual(env.sent.map((batch) => batch.map((e) => e.client_seq)), [[0, 1]]);
  assert.equal(queue.pending, 0);
  assert.equal(env.storage.data.has(STORAGE_KEY), false);
});

test('should split a flush into batches of at most the server limit', async () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  for (let i = 0; i < BATCH_MAX * 2 + 5; i += 1) queue.record(draft(i));
  await queue.flush();
  assert.deepEqual(env.sent.map((batch) => batch.length), [BATCH_MAX, BATCH_MAX, 5]);
});

test('should requeue the failed batch ahead of newer events when the send fails', async () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  queue.record(draft(0));
  queue.record(draft(1));
  env.failSend = true;
  assert.equal(await queue.flush(), 0);
  queue.record(draft(2));
  assert.deepEqual(queue.snapshot().map((e) => e.client_seq), [0, 1, 2]);
  env.failSend = false;
  assert.equal(await queue.flush(), 3);
  assert.deepEqual(env.sent[0].map((e) => e.client_seq), [0, 1, 2]);
});

test('should stop at the first failing batch and keep later events when a middle batch fails', async () => {
  let calls = 0;
  const env = environment({
    send: async (events) => {
      calls += 1;
      if (calls === 2) throw new Error('offline');
      env.sent.push(events);
    },
  });
  const queue = new HistoryQueue(env);
  for (let i = 0; i < BATCH_MAX + 10; i += 1) queue.record(draft(i));
  assert.equal(await queue.flush(), BATCH_MAX);
  assert.equal(queue.pending, 10);
  assert.equal(queue.snapshot()[0].client_seq, BATCH_MAX);
});

test('should keep only the newest events when the queue passes its cap', () => {
  const queue = new HistoryQueue(environment());
  for (let i = 0; i < QUEUE_CAP + 25; i += 1) queue.record(draft(i));
  assert.equal(queue.pending, QUEUE_CAP);
  assert.equal(queue.snapshot()[0].client_seq, 25);
});

test('should apply the cap after requeueing a failed batch', async () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  for (let i = 0; i < BATCH_MAX; i += 1) queue.record(draft(i));
  env.failSend = true;
  const flushing = queue.flush();
  for (let i = 0; i < QUEUE_CAP; i += 1) queue.record(draft(i));
  await flushing;
  assert.equal(queue.pending, QUEUE_CAP);
  assert.equal(queue.snapshot().at(-1).client_seq, BATCH_MAX + QUEUE_CAP - 1);
});

test('should not send twice when flushes overlap', async () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  queue.record(draft());
  const [first, second] = await Promise.all([queue.flush(), queue.flush()]);
  assert.equal(first + second, 1);
  assert.equal(env.sent.length, 1);
});

test('should persist the queue and resume it under a new session when the page reloads', async () => {
  const env = environment();
  const first = new HistoryQueue(env);
  first.record(draft(0));
  const reloaded = new HistoryQueue({ ...env, newSessionId: () => 'session-9' });
  assert.deepEqual(reloaded.snapshot().map((e) => [e.session_id, e.client_seq]), [['session-1', 0]]);
  const fresh = reloaded.record(draft(1));
  assert.deepEqual([fresh.session_id, fresh.client_seq], ['session-9', 0]);
  await reloaded.flush();
  assert.equal(env.sent[0].length, 2);
});

test('should drop repeated session and sequence pairs when loading storage', () => {
  const stored = { kind: 'search_run', occurred_at: 'x', session_id: 's', client_seq: 3 };
  const env = environment({ storage: memoryStorage({ [STORAGE_KEY]: JSON.stringify([stored, stored, { ...stored, client_seq: 4 }]) }) });
  assert.deepEqual(new HistoryQueue(env).snapshot().map((e) => e.client_seq), [3, 4]);
});

test('should ignore corrupt storage when loading', () => {
  for (const text of ['not json', '{"a":1}', '[1,"x",null,{"kind":5}]']) {
    const env = environment({ storage: memoryStorage({ [STORAGE_KEY]: text }) });
    assert.equal(new HistoryQueue(env).pending, 0);
  }
});

test('should keep working in memory when storage throws', async () => {
  const throwing = {
    getItem: () => { throw new Error('denied'); },
    setItem: () => { throw new Error('quota'); },
    removeItem: () => { throw new Error('denied'); },
  };
  const env = environment({ storage: throwing });
  const queue = new HistoryQueue(env);
  queue.record(draft());
  assert.equal(await queue.flush(), 1);
  assert.equal(new HistoryQueue(environment({ storage: null })).pending, 0);
});

test('should hand every batch to the beacon and clear the queue when the page hides', () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  for (let i = 0; i < BATCH_MAX + 1; i += 1) queue.record(draft(i));
  assert.equal(queue.flushBeacon(), BATCH_MAX + 1);
  assert.deepEqual(env.beaconed.map((batch) => batch.length), [BATCH_MAX, 1]);
  assert.equal(queue.pending, 0);
});

test('should keep events queued and stored when the beacon refuses or throws', () => {
  const env = environment({ beaconOk: false });
  const queue = new HistoryQueue(env);
  queue.record(draft());
  assert.equal(queue.flushBeacon(), 0);
  assert.equal(queue.pending, 1);
  assert.ok(env.storage.data.has(STORAGE_KEY));
  const throwing = new HistoryQueue(environment({ beacon: () => { throw new Error('no beacon'); } }));
  throwing.record(draft());
  assert.equal(throwing.flushBeacon(), 0);
  assert.equal(throwing.pending, 1);
});

test('should keep an in-flight batch in storage until the server answers', async () => {
  let release;
  const env = environment({ send: () => new Promise((resolve) => { release = resolve; }) });
  const queue = new HistoryQueue(env);
  queue.record(draft());
  const flushing = queue.flush();
  assert.equal(queue.pending, 0);
  assert.equal(JSON.parse(env.storage.data.get(STORAGE_KEY)).length, 1);
  release();
  await flushing;
  assert.equal(env.storage.data.has(STORAGE_KEY), false);
});

test('should count dwell only while input is recent when tracking', () => {
  const tracker = new DwellTracker();
  tracker.start(0);
  assert.equal(tracker.dwell(30_000), 30_000);
  assert.equal(tracker.dwell(ACTIVE_IDLE_MS), ACTIVE_IDLE_MS);
  assert.equal(tracker.dwell(10 * ACTIVE_IDLE_MS), ACTIVE_IDLE_MS);
  tracker.input(10 * ACTIVE_IDLE_MS);
  assert.equal(tracker.dwell(10 * ACTIVE_IDLE_MS + 5000), ACTIVE_IDLE_MS + 5000);
});

test('should stop counting while the tab is hidden or unfocused', () => {
  const tracker = new DwellTracker();
  tracker.start(0);
  tracker.setVisible(false, 10_000);
  tracker.input(20_000);
  tracker.setVisible(true, 30_000);
  tracker.input(30_000);
  tracker.setFocused(false, 40_000);
  tracker.setFocused(true, 50_000);
  tracker.input(50_000);
  assert.equal(tracker.stop(55_000), 10_000 + 10_000 + 5_000);
});

test('should not count time that passes before start or after stop', () => {
  const tracker = new DwellTracker();
  tracker.input(5000);
  assert.equal(tracker.dwell(9000), 0);
  tracker.start(10_000);
  assert.equal(tracker.stop(12_000), 2000);
  assert.equal(tracker.stop(99_000), 2000);
  assert.equal(tracker.dwell(99_000), 2000);
});

test('should ignore a clock that goes backwards when tracking', () => {
  const tracker = new DwellTracker();
  tracker.start(10_000);
  tracker.input(8_000);
  assert.equal(tracker.dwell(9_000), 0);
  assert.equal(tracker.stop(11_000), 1000);
});

test('should reset the count when a tracker starts again', () => {
  const tracker = new DwellTracker(1000);
  tracker.start(0);
  tracker.stop(500);
  tracker.start(10_000);
  assert.equal(tracker.stop(10_300), 300);
});

test('should record opened, page_reached once per page and left with active dwell for a visit', () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  const visit = new PaperVisit(queue, env, { paper_id: 'paper1', work_id: 'w1' });
  env.clock.t += 10_000;
  assert.equal(visit.pageReached(1), true);
  assert.equal(visit.pageReached(1), false);
  assert.equal(visit.pageReached(0), false);
  assert.equal(visit.pageReached(1.5), false);
  env.clock.t += 20_000;
  assert.equal(visit.pageReached(2), true);
  env.clock.t += 5 * ACTIVE_IDLE_MS;
  assert.equal(visit.end(), 30_000 + ACTIVE_IDLE_MS);
  assert.equal(visit.end(), null);
  assert.equal(visit.pageReached(3), false);
  const events = queue.snapshot();
  assert.deepEqual(events.map((e) => e.kind), ['paper_opened', 'page_reached', 'page_reached', 'paper_left']);
  assert.deepEqual(events[1].detail, { page: 1 });
  assert.equal(events[3].dwell_ms, 30_000 + ACTIVE_IDLE_MS);
  assert.ok(events.every((e) => e.paper_id === 'paper1' && e.work_id === 'w1'));
});

test('should count a page visit once per visit but again in the next visit', () => {
  const env = environment();
  const queue = new HistoryQueue(env);
  const first = new PaperVisit(queue, env, { paper_id: 'p' });
  first.pageReached(4);
  first.end();
  const second = new PaperVisit(queue, env, { paper_id: 'p' });
  assert.equal(second.pageReached(4), true);
});

test('should treat an inspection as real only after two seconds', () => {
  assert.equal(isInspection(1999), false);
  assert.equal(isInspection(2000), true);
});

test('should save only the latest value after the delay when debouncing', () => {
  const saved = [];
  const timers = {
    next: 1, pending: new Map(),
    setTimeout(callback, ms) { const id = this.next++; this.pending.set(id, { callback, ms }); return id; },
    clearTimeout(id) { this.pending.delete(id); },
  };
  const debouncer = new Debouncer((value) => saved.push(value), timers);
  debouncer.push('a');
  debouncer.push('b');
  assert.equal(timers.pending.size, 1);
  assert.equal([...timers.pending.values()][0].ms, 2000);
  [...timers.pending.values()][0].callback();
  assert.deepEqual(saved, ['b']);
  debouncer.push('c');
  debouncer.flushNow();
  debouncer.flushNow();
  assert.deepEqual(saved, ['b', 'c']);
  debouncer.push('d');
  debouncer.cancel();
  assert.equal(timers.pending.size, 0);
  assert.deepEqual(saved, ['b', 'c']);
});
