import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildRows, dayKey, dayLabel, describeEvent, formatDwell, kindsForGroups, mergeSessions, rowTarget, selectableKeys,
  sessionPapers, sessionSummary, stepDay, toggleGroup,
} from '../src/lib/historyView.ts';

const at = (day, hour, minute = 0) => new Date(2026, 9, day, hour, minute).toISOString();
let nextId = 1;
const event = (over) => ({
  id: nextId++, occurred_at: at(25, 10), session_id: 's1', kind: 'paper_opened', paper_id: null, work_id: null, list_id: null,
  dwell_ms: null, detail: {}, client_seq: 0, work_title: null, ...over,
});
const session = (id, day, hour, events) => ({ session_id: id, started_at: at(day, hour), ended_at: at(day, hour, 40), events });
const titleOf = (id) => (id === 'p1' ? 'Attention Is All You Need' : null);
const NOW = new Date(2026, 9, 25, 15);

test('should format dwell as seconds, minutes or hours when given milliseconds', () => {
  assert.equal(formatDwell(0), '0s');
  assert.equal(formatDwell(-5), '0s');
  assert.equal(formatDwell(Number.NaN), '0s');
  assert.equal(formatDwell(null), '0s');
  assert.equal(formatDwell(42_000), '42s');
  assert.equal(formatDwell(252_000), '4m 12s');
  assert.equal(formatDwell(3_900_000), '1h 05m');
});

test('should label days as today, yesterday or by date when given a day key', () => {
  assert.equal(dayLabel(dayKey(at(25, 9)), NOW), 'Today');
  assert.equal(dayLabel(dayKey(at(24, 9)), NOW), 'Yesterday');
  assert.equal(dayLabel('2026-10-17', NOW), 'Sat 17 Oct 2026');
  assert.equal(dayLabel('', NOW), 'Unknown day');
  assert.equal(dayKey('not a time'), '');
});

test('should describe each event kind without throwing when detail is missing', () => {
  const kinds = ['paper_opened', 'paper_left', 'view_changed', 'section_focused', 'figure_viewed', 'page_reached', 'search_run',
    'question_asked', 'link_followed', 'digest_opened', 'graph_opened', 'work_inspected', 'list_opened', 'list_changed',
    'list_generated', 'fetch_requested'];
  for (const kind of kinds) assert.ok(describeEvent(event({ kind }), titleOf).length > 0, kind);
});

test('should show the paper title and dwell when describing paper_left', () => {
  const text = describeEvent(event({ kind: 'paper_left', paper_id: 'p1', dwell_ms: 252_000 }), titleOf);
  assert.equal(text, 'Left Attention Is All You Need · 4m 12s');
});

test('should fall back to the paper id when the library has no title', () => {
  assert.equal(describeEvent(event({ kind: 'paper_opened', paper_id: 'gone' }), titleOf), 'Opened gone');
});

test('should keep unicode and quotes in a search query when describing it', () => {
  assert.equal(describeEvent(event({ kind: 'search_run', detail: { query: 'Schrödinger "cat" 猫' } }), titleOf), 'Searched “Schrödinger "cat" 猫”');
});

test('should ignore a non-string query when describing a search', () => {
  assert.equal(describeEvent(event({ kind: 'search_run', detail: { query: 7 } }), titleOf), 'Searched “”');
});

test('should ask for every kind when no group is on, and the union when some are', () => {
  assert.deepEqual(kindsForGroups(new Set()), []);
  assert.deepEqual(kindsForGroups(new Set(['3'])), ['search_run']);
  assert.deepEqual(kindsForGroups(new Set(['2', '3'])), ['section_focused', 'figure_viewed', 'search_run']);
});

test('should toggle a group and ignore keys that name no group', () => {
  const on = toggleGroup(new Set(), '1');
  assert.deepEqual([...on], ['1']);
  assert.deepEqual([...toggleGroup(on, '1')], []);
  assert.deepEqual([...toggleGroup(on, '9')], ['1']);
  assert.deepEqual([...toggleGroup(on, '0')], ['1']);
});

test('should rank session papers by dwell and sum repeated visits', () => {
  const s = session('s1', 25, 9, [
    event({ kind: 'paper_left', paper_id: 'b', dwell_ms: 90_000, occurred_at: at(25, 10) }),
    event({ kind: 'paper_left', paper_id: 'a', dwell_ms: 30_000, occurred_at: at(25, 9, 50) }),
    event({ kind: 'paper_left', paper_id: 'a', dwell_ms: 70_000, occurred_at: at(25, 9, 30) }),
    event({ kind: 'paper_opened', paper_id: 'c', occurred_at: at(25, 9, 5) }),
  ]);
  assert.deepEqual(sessionPapers(s), [{ paperId: 'a', dwellMs: 100_000 }, { paperId: 'b', dwellMs: 90_000 }, { paperId: 'c', dwellMs: 0 }]);
});

test('should summarize a session by papers, dwell and searches, or by event count when empty', () => {
  const s = session('s1', 25, 9, [
    event({ kind: 'paper_left', paper_id: 'a', dwell_ms: 252_000 }),
    event({ kind: 'search_run' }),
  ]);
  assert.equal(sessionSummary(s), '1 paper · 4m 12s · 1 search');
  assert.equal(sessionSummary(session('s2', 25, 9, [event({ kind: 'list_opened' })])), '1 event');
  assert.equal(sessionSummary(session('s3', 25, 9, [])), '0 events');
});

test('should group sessions under one heading per day and expand only chosen sessions', () => {
  const sessions = [
    session('s1', 25, 14, [event({ session_id: 's1' })]),
    session('s2', 25, 9, [event({ session_id: 's2' })]),
    session('s3', 24, 20, [event({ session_id: 's3' }), event({ session_id: 's3' })]),
  ];
  const rows = buildRows(sessions, new Set(['s3']), NOW);
  assert.deepEqual(rows.map((row) => row.type), ['day', 'session', 'session', 'day', 'session', 'event', 'event']);
  assert.equal(rows[0].label, 'Today');
  assert.equal(rows[3].label, 'Yesterday');
  assert.equal(selectableKeys(rows).length, 5);
});

test('should build no rows when there are no sessions', () => {
  assert.deepEqual(buildRows([], new Set(), NOW), []);
  assert.deepEqual(selectableKeys([]), []);
  assert.equal(stepDay([], null, 1), null);
});

test('should jump between days with stepDay and stay at the ends', () => {
  const sessions = [
    session('s1', 25, 14, [event({})]),
    session('s2', 25, 9, [event({})]),
    session('s3', 24, 20, [event({})]),
    session('s4', 23, 8, [event({})]),
  ];
  const rows = buildRows(sessions, new Set(), NOW);
  assert.equal(stepDay(rows, 'session:s1', 1), 'session:s3');
  assert.equal(stepDay(rows, 'session:s2', 1), 'session:s3');
  assert.equal(stepDay(rows, 'session:s3', 1), 'session:s4');
  assert.equal(stepDay(rows, 'session:s4', 1), 'session:s4');
  assert.equal(stepDay(rows, 'session:s2', -1), 'session:s1');
  assert.equal(stepDay(rows, 'session:s1', -1), 'session:s1');
  assert.equal(stepDay(rows, 'session:s4', -1), 'session:s3');
  assert.equal(stepDay(rows, null, 1), 'session:s1');
});

test('should step to the day of an event row when jumping from inside an expanded session', () => {
  const sessions = [session('s1', 25, 9, [event({ id: 90 })]), session('s2', 24, 9, [event({ id: 91 })])];
  const rows = buildRows(sessions, new Set(['s1']), NOW);
  const eventKey = selectableKeys(rows)[1];
  assert.match(eventKey, /^event:/u);
  assert.equal(stepDay(rows, eventKey, 1), 'session:s2');
  assert.equal(stepDay(rows, eventKey, -1), 'session:s1');
});

test('should merge a session split across pages and drop repeated events', () => {
  const first = [session('s1', 25, 9, [event({ id: 10, occurred_at: at(25, 9, 30) }), event({ id: 9, occurred_at: at(25, 9, 20) })])];
  const second = [{ ...session('s1', 25, 8, [event({ id: 9, occurred_at: at(25, 9, 20) }), event({ id: 8, occurred_at: at(25, 8, 10) })]) },
    session('s0', 24, 9, [event({ id: 5 })])];
  const merged = mergeSessions(first, second);
  assert.deepEqual(merged.map((item) => item.session_id), ['s1', 's0']);
  assert.deepEqual(merged[0].events.map((item) => item.id), [10, 9, 8]);
  assert.equal(merged[0].started_at, at(25, 8));
});

test('should keep events newest first when merging with equal timestamps', () => {
  const same = at(25, 9);
  const merged = mergeSessions([], [session('s1', 25, 9, [event({ id: 1, occurred_at: same }), event({ id: 3, occurred_at: same }), event({ id: 2, occurred_at: same })])]);
  assert.deepEqual(merged[0].events.map((item) => item.id), [3, 2, 1]);
});

test('should target the event paper, work and list, and the busiest paper for a session', () => {
  const s = session('s1', 25, 9, [
    event({ kind: 'paper_left', paper_id: 'a', dwell_ms: 10 }),
    event({ kind: 'paper_left', paper_id: 'b', dwell_ms: 99 }),
  ]);
  const rows = buildRows([s], new Set(['s1']), NOW);
  const sessionRow = rows.find((row) => row.type === 'session');
  const eventRow = rows.find((row) => row.type === 'event');
  assert.deepEqual(rowTarget(sessionRow), { paperId: 'b' });
  assert.deepEqual(rowTarget(eventRow), { paperId: 'a' });
  assert.deepEqual(rowTarget(rows[0]), {});
  const inspected = buildRows([session('s2', 25, 9, [event({ kind: 'work_inspected', work_id: 'w1' }), event({ kind: 'list_opened', list_id: 'l1' })])], new Set(['s2']), NOW);
  const targets = inspected.filter((row) => row.type === 'event').map(rowTarget);
  assert.deepEqual(targets, [{ workId: 'w1' }, { listId: 'l1' }]);
  assert.deepEqual(rowTarget(inspected.find((row) => row.type === 'session')), {});
});
