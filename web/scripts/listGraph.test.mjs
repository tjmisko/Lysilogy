import assert from 'node:assert/strict';
import test from 'node:test';
import {
  blockersByWork, frontier, linearize, unmetPrerequisites, validateEdge,
} from '../src/lib/listGraph.ts';

const item = (id, over = {}) => ({
  id, workId: `w${id}`, depth: 0, score: 0, year: null, titleKey: `title ${id}`, isSeed: false, ...over,
});
const edge = (from, to, kind = 'prerequisite') => ({ from, to, kind });
const kinds = (spec) => new Map(Object.entries(spec));

function permutations(values) {
  if (values.length <= 1) return [values];
  return values.flatMap((value, index) =>
    permutations([...values.slice(0, index), ...values.slice(index + 1)]).map((rest) => [value, ...rest]));
}

test('should accept an edge between existing nodes when it closes no cycle', () => {
  const nodes = kinds({ a: 'work', b: 'work' });
  assert.deepEqual(validateEdge(nodes, [], edge('a', 'b')), { ok: true });
});

test('should reject a self prerequisite when from equals to', () => {
  const nodes = kinds({ a: 'work' });
  assert.deepEqual(validateEdge(nodes, [], edge('a', 'a')), { ok: false, reason: 'edge_cycle', cycle: ['a'] });
});

test('should name the cycle starting at the new source when prerequisites close a loop', () => {
  const nodes = kinds({ a: 'work', b: 'work', c: 'work' });
  const existing = [edge('a', 'b'), edge('b', 'c')];
  assert.deepEqual(validateEdge(nodes, existing, edge('c', 'a')), {
    ok: false, reason: 'edge_cycle', cycle: ['c', 'a', 'b'],
  });
});

test('should reject a subtopic cycle when topics point at each other', () => {
  const nodes = kinds({ t1: 'topic', t2: 'topic' });
  const result = validateEdge(nodes, [edge('t1', 't2', 'subtopic')], edge('t2', 't1', 'subtopic'));
  assert.equal(result.ok, false);
  assert.deepEqual(result.cycle, ['t2', 't1']);
});

test('should reject a cycle that only appears once topics are expanded to their works', () => {
  const nodes = kinds({ t: 'topic', a: 'work', b: 'work' });
  const existing = [edge('t', 'a', 'covers'), edge('t', 'b', 'covers'), edge('a', 'b')];
  const result = validateEdge(nodes, existing, edge('b', 't'));
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'edge_cycle');
  assert.deepEqual([...result.cycle].sort(), ['a', 'b']);
});

test('should reject covers edges that are not topic to work', () => {
  const nodes = kinds({ t: 'topic', u: 'topic', a: 'work', b: 'work', c: 'claim' });
  for (const bad of [edge('a', 'b', 'covers'), edge('t', 'u', 'covers'), edge('a', 't', 'covers'), edge('t', 'c', 'covers')]) {
    assert.deepEqual(validateEdge(nodes, [], bad), { ok: false, reason: 'invalid_edge_kind' });
  }
  assert.deepEqual(validateEdge(nodes, [], edge('a', 't', 'subtopic')), { ok: false, reason: 'invalid_edge_kind' });
});

test('should reject a prerequisite on a claim when either end is unverified', () => {
  const nodes = kinds({ a: 'work', c: 'claim' });
  assert.deepEqual(validateEdge(nodes, [], edge('a', 'c')), { ok: false, reason: 'invalid_edge_kind' });
  assert.deepEqual(validateEdge(nodes, [], edge('c', 'a')), { ok: false, reason: 'invalid_edge_kind' });
});

test('should report an unknown node when an endpoint is not in the list', () => {
  assert.deepEqual(validateEdge(kinds({ a: 'work' }), [], edge('a', 'zz')), { ok: false, reason: 'node_not_found' });
});

test('should accept a stored edge again when it is already present', () => {
  const nodes = kinds({ a: 'work', b: 'work' });
  assert.deepEqual(validateEdge(nodes, [edge('a', 'b')], edge('a', 'b')), { ok: true });
});

test('should put a prerequisite before its dependant when priority would say otherwise', () => {
  const items = [item('a', { depth: 5 }), item('b', { depth: 0 })];
  assert.deepEqual(linearize(items, [edge('b', 'a')], []), ['b', 'a']);
});

test('should order cited works before citing works when there are no stored edges', () => {
  const items = [item('a'), item('b')];
  assert.deepEqual(linearize(items, [], [['wa', 'wb']]), ['b', 'a']);
});

test('should match priority order when there are no topics or prerequisites', () => {
  const items = [
    item('a', { year: 2020 }), item('b', { year: 2010 }), item('c', { year: null }),
    item('d', { year: 2010, score: 3 }), item('e', { depth: 2, year: 2022 }), item('s', { isSeed: true }),
  ];
  assert.deepEqual(linearize(items, [], []), ['e', 'd', 'b', 'a', 'c', 's']);
});

test('should keep a topic contiguous when the constraints leave it free', () => {
  const items = [
    item('a', { year: 2001 }), item('x', { year: 2002 }), item('b', { year: 2003 }), item('y', { year: 2004 }),
  ];
  const edges = [edge('t', 'a', 'covers'), edge('t', 'b', 'covers')];
  assert.deepEqual(linearize(items, edges, []), ['a', 'b', 'x', 'y']);
});

test('should expand a topic prerequisite to every work under it when linearizing', () => {
  const items = [item('a', { depth: 9 }), item('b'), item('c')];
  const edges = [edge('t', 'b', 'covers'), edge('t', 'c', 'covers'), edge('t', 'a')];
  const order = linearize(items, edges, []);
  assert.equal(order.at(-1), 'a');
});

test('should give the same order under every shuffle of six items when cycles and ties exist', () => {
  const items = [
    item('a', { year: 2000 }), item('b', { year: 2000 }), item('c', { year: 2001 }),
    item('d', { year: 2001, depth: 1 }), item('e', { score: 2 }), item('f', { isSeed: true }),
  ];
  const edges = [edge('a', 'c'), edge('c', 'e'), edge('e', 'a'), edge('t', 'b', 'covers'), edge('t', 'd', 'covers')];
  const cites = [['wa', 'wb'], ['wb', 'wa'], ['wd', 'wc']];
  const expected = linearize(items, edges, cites);
  assert.equal(expected.length, 6);
  assert.equal(expected.at(-1), 'f');
  for (const shuffled of permutations(items)) {
    assert.deepEqual(linearize(shuffled, edges, cites), expected);
  }
});

test('should break a prerequisite cycle deterministically when only cycles remain', () => {
  const items = [item('a', { year: 2001 }), item('b', { year: 2000 })];
  const edges = [edge('a', 'b'), edge('b', 'a')];
  assert.deepEqual(linearize(items, edges, []), ['b', 'a']);
  assert.deepEqual(linearize([...items].reverse(), edges, []), ['b', 'a']);
});

test('should drop citation constraints before prerequisites when both cycle', () => {
  const items = [item('a'), item('b')];
  // b is a prerequisite of a; a cites b (cited-first agrees); b cites a would conflict.
  const order = linearize(items, [edge('b', 'a')], [['wb', 'wa']]);
  assert.deepEqual(order, ['b', 'a']);
});

test('should keep the first entry and ignore unknown ids when items repeat or edges dangle', () => {
  const items = [item('a'), item('a', { depth: 9 }), item('b')];
  const order = linearize(items, [edge('ghost', 'a')], [['wghost', 'wa']]);
  assert.deepEqual([...order].sort(), ['a', 'b']);
});

test('should return an empty order when there are no items', () => {
  assert.deepEqual(linearize([], [], []), []);
});

test('should put the seed last and ignore constraints involving it', () => {
  const items = [item('s', { isSeed: true, depth: 9 }), item('a'), item('b')];
  assert.deepEqual(linearize(items, [edge('s', 'a')], [['wa', 'ws']]), ['a', 'b', 's']);
});

test('should treat claims as plain items without citation constraints', () => {
  const items = [item('a'), item('c', { workId: null })];
  assert.equal(linearize(items, [], [['wa', 'wc']]).length, 2);
});

const works = (spec) => Object.entries(spec).map(([id, status]) => ({ id, status }));

test('should list unread works with no unread prerequisites when computing next up', () => {
  const ws = works({ a: 'read', b: 'unread', c: 'unread', d: 'reading' });
  const edges = [edge('a', 'b'), edge('b', 'c'), edge('d', 'c')];
  assert.deepEqual(frontier(ws, edges, ['c', 'b', 'a', 'd']), ['b']);
});

test('should block through topics when any work under the prerequisite topic is unread', () => {
  const ws = works({ a: 'read', b: 'unread', c: 'unread' });
  const edges = [edge('t', 'a', 'covers'), edge('t', 'b', 'covers'), edge('t', 'c')];
  assert.deepEqual(frontier(ws, edges, []), ['b']);
  assert.deepEqual(unmetPrerequisites(ws, edges, 'c'), ['b']);
});

test('should rank by the given order and put works missing from it last by id', () => {
  const ws = works({ a: 'unread', b: 'unread', c: 'unread' });
  assert.deepEqual(frontier(ws, [], ['c']), ['c', 'a', 'b']);
});

test('should be ready when the only prerequisite is the work itself through a shared topic', () => {
  const ws = works({ a: 'unread' });
  const edges = [edge('t', 'a', 'covers'), edge('t', 't2'), edge('t2', 'a', 'covers')];
  assert.deepEqual(frontier(ws, edges, []), ['a']);
  assert.equal(blockersByWork(ws, edges).get('a')?.size ?? 0, 0);
});

test('should return no works when the list is empty', () => {
  assert.deepEqual(frontier([], [], []), []);
});
