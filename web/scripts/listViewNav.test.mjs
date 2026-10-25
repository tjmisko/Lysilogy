import assert from 'node:assert/strict';
import test from 'node:test';
import { timelineColumns, timelineYearMove, topicColumns, topicLayerMove } from '../src/lib/listViewNav.ts';
import { timeline, topicLayers } from '../src/lib/listViews.ts';

let counter = 0;
function node(kind, id, over = {}) {
  counter += 1;
  return {
    id, list_id: 'p1', kind, work_id: null, label: null, summary: null, claim: null, claim_state: null,
    candidates: [], position: counter, note: null, rationale: null, context_links: [], source: 'user',
    review: 'accepted', via_work_id: null, path: null, generation_id: null,
    added_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z',
    work: null, ordinal: counter, depth: 0, path_steps: [], ...over,
  };
}
const work = (id, year) => node('work', id, {
  work_id: `w-${id}`, work: { id: `w-${id}`, display_title: id, display_year: year, display_authors: [] },
});
const topic = (id) => node('topic', id, { label: id });
const edge = (from, to, kind = 'prerequisite') => ({
  list_id: 'p1', from, to, kind, source: 'user', review: 'accepted', created_at: '2026-01-01T00:00:00Z',
});
const detail = (nodes, edges = [], citations = []) => ({
  list: { id: 'p1', title: 'L', seed_work_id: null }, nodes, edges, citations,
  reading_states: {}, local_papers: {}, acquisitions: {},
});

test('should land on the next layer topic header when l is pressed on a topic', () => {
  const d = detail([topic('a'), topic('b'), work('x', 2000)], [edge('a', 'b'), edge('a', 'x', 'covers')]);
  const model = topicLayers(d);
  assert.equal(topicLayerMove(model, 'a', 1), 'b');
  assert.equal(topicLayerMove(model, 'b', -1), 'a');
});

test('should step from a work to the header of the neighbouring layer when h or l is pressed on a work', () => {
  const d = detail([topic('a'), topic('b'), work('x', 2000)], [edge('a', 'b'), edge('a', 'x', 'covers')]);
  assert.equal(topicLayerMove(topicLayers(d), 'x', 1), 'b');
});

test('should stay put when h is pressed in the first layer or l in the last', () => {
  const d = detail([topic('a'), topic('b')], [edge('a', 'b')]);
  const model = topicLayers(d);
  assert.equal(topicLayerMove(model, 'a', -1), null);
  assert.equal(topicLayerMove(model, 'b', 1), null);
});

test('should clamp to the last topic when the next layer is shorter', () => {
  const d = detail([topic('a'), topic('b'), topic('c')], [edge('a', 'c')]);
  const model = topicLayers(d);
  // layer 0 holds a and b, layer 1 holds c
  assert.equal(topicLayerMove(model, 'b', 1), 'c');
});

test('should reach the Unfiled group as the last column when l is pressed past the last layer', () => {
  const d = detail([topic('a'), work('x', 2000)]);
  const model = topicLayers(d);
  assert.equal(topicLayerMove(model, 'a', 1), 'group:unfiled');
  assert.equal(topicLayerMove(model, 'group:unfiled', -1), 'a');
});

test('should ignore a cycle between topics when laying out columns', () => {
  const d = detail([topic('a'), topic('b')], [edge('a', 'b'), edge('b', 'a')]);
  const columns = topicColumns(topicLayers(d));
  assert.equal(columns.flat().length, 2);
});

test('should omit the works of a folded topic from its column', () => {
  const d = detail([topic('a'), work('x', 2000), work('y', 2001)], [edge('a', 'x', 'covers'), edge('a', 'y', 'covers')]);
  assert.deepEqual(topicColumns(topicLayers(d, new Set(['a']))), [['a']]);
  assert.deepEqual(topicColumns(topicLayers(d)), [['a', 'x', 'y']]);
});

test('should return null when the cursor is not in the Topics view', () => {
  const d = detail([topic('a')]);
  assert.equal(topicLayerMove(topicLayers(d), 'nope', 1), null);
  assert.equal(topicLayerMove(topicLayers(d), null, 1), null);
});

test('should move to the same depth in the next year when l is pressed', () => {
  const t = timeline(detail([work('a', 1990), work('b', 1990), work('c', 2000), work('d', 2000)]));
  const columns = timelineColumns(t.buckets, t.undated);
  assert.equal(timelineYearMove(columns, 'b', 1), 'd');
  assert.equal(timelineYearMove(columns, 'd', -1), 'b');
});

test('should clamp to the last work when the next year has fewer works', () => {
  const t = timeline(detail([work('a', 1990), work('b', 1990), work('c', 2000)]));
  assert.equal(timelineYearMove(timelineColumns(t.buckets, t.undated), 'b', 1), 'c');
});

test('should reach undated works as the last column and stop at the ends', () => {
  const t = timeline(detail([work('a', 1990), work('u', null)]));
  const columns = timelineColumns(t.buckets, t.undated);
  assert.equal(timelineYearMove(columns, 'a', 1), 'u');
  assert.equal(timelineYearMove(columns, 'u', 1), null);
  assert.equal(timelineYearMove(columns, 'a', -1), null);
});

test('should return null when the timeline is empty', () => {
  assert.equal(timelineYearMove([], null, 1), null);
  assert.deepEqual(timelineColumns([], []), []);
});

test('should derive citation links between members only, once, and not to itself', () => {
  const d = detail([work('a', 1990), work('b', 2000)], [], [['w-b', 'w-a'], ['w-b', 'w-a'], ['w-a', 'w-a'], ['w-b', 'w-zzz']]);
  assert.deepEqual(timeline(d).links, [{ from: 'b', to: 'a' }]);
});
