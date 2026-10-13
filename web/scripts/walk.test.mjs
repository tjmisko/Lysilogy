import assert from 'node:assert/strict';
import test from 'node:test';
import {
  EXPANSION_TOP_N, MAX_UI_DEPTH, MAX_UI_EXPANSIONS, addExpansion, canExpand, collapseExpansion, fanoutKnown, walkPathTo,
} from '../src/lib/walk.ts';

const view = (state) => ({ state, method: null, references_count: 0, unavailable_reason: null, expanded_at: null });
const prominence = (cited) => ({ score: 0.5, signals: { cited_works_count: cited } });

test('should expose the documented limits', () => {
  assert.equal(EXPANSION_TOP_N, 8);
  assert.equal(MAX_UI_EXPANSIONS, 6);
  assert.equal(MAX_UI_DEPTH, 3);
});

test('should add expansion with depth one more than parent', () => {
  const list = addExpansion([], 'a', ['b', 'c', 'b', 'a'], 1);
  assert.deepEqual(list, [{ parentId: 'a', childIds: ['b', 'c'], depth: 2 }]);
});

test('should replace the expansion when the same parent is added again', () => {
  const first = addExpansion([], 'a', ['b'], 1);
  const second = addExpansion(first, 'a', ['c'], 1);
  assert.deepEqual(second, [{ parentId: 'a', childIds: ['c'], depth: 2 }]);
  assert.equal(first[0].childIds[0], 'b');
});

test('should collapse expansion and its descendants', () => {
  let list = addExpansion([], 'a', ['b', 'c'], 0);
  list = addExpansion(list, 'b', ['d'], 1);
  list = addExpansion(list, 'd', ['e'], 2);
  list = addExpansion(list, 'x', ['y'], 0);
  assert.deepEqual(collapseExpansion(list, 'b').map((e) => e.parentId), ['a', 'x']);
  assert.deepEqual(collapseExpansion(list, 'a').map((e) => e.parentId), ['x']);
});

test('should leave the list unchanged when collapsing an unexpanded work', () => {
  const list = addExpansion([], 'a', ['b'], 0);
  assert.deepEqual(collapseExpansion(list, 'zzz'), list);
});

test('should keep an expansion of a shared node when a cross-linked parent collapses', () => {
  let list = addExpansion([], 'a', ['b'], 0);
  list = addExpansion(list, 'b', ['k'], 1);
  list = addExpansion(list, 'k', ['m'], 0);
  assert.deepEqual(collapseExpansion(list, 'b').map((e) => e.parentId), ['a', 'k']);
});

test('should refuse expansion beyond max depth or count', () => {
  const depths = { c: 0, a: 1, b: 2, d: 3 };
  const depthOf = (id) => depths[id] ?? 1;
  assert.equal(canExpand([], 'c', depthOf), 'ok');
  assert.equal(canExpand([], 'b', depthOf), 'ok');
  assert.equal(canExpand([], 'd', depthOf), 'too_deep');
  const full = Array.from({ length: MAX_UI_EXPANSIONS }, (_, i) => ({ parentId: `p${i}`, childIds: [], depth: 1 }));
  assert.equal(canExpand(full, 'a', depthOf), 'too_many');
  assert.equal(canExpand(full, 'p0', depthOf), 'ok');
});

test('should return path from center through expansion parents', () => {
  let list = addExpansion([], 'a', ['b'], 1);
  list = addExpansion(list, 'b', ['d'], 2);
  assert.deepEqual(walkPathTo(list, 'c', 'd', new Set(['a'])), ['c', 'a', 'b', 'd']);
});

test('should return center only path when work is a base node', () => {
  const list = addExpansion([], 'a', ['b'], 1);
  assert.deepEqual(walkPathTo(list, 'c', 'a', new Set(['a'])), ['c', 'a']);
  assert.deepEqual(walkPathTo(list, 'c', 'c'), ['c']);
  assert.deepEqual(walkPathTo([], 'c', 'q'), ['c', 'q']);
});

test('should not loop when expansions list each other as children', () => {
  const list = [{ parentId: 'a', childIds: ['b'], depth: 2 }, { parentId: 'b', childIds: ['a'], depth: 3 }];
  const path = walkPathTo(list, 'c', 'a');
  assert.equal(path[0], 'c');
  assert.equal(path.at(-1), 'a');
  assert.ok(path.length <= 6);
});

test('should treat known and expanded views as fanout known', () => {
  assert.equal(fanoutKnown(view('expanded')), true);
  assert.equal(fanoutKnown(view('known')), true);
  assert.equal(fanoutKnown(view('unavailable')), false);
  assert.equal(fanoutKnown(undefined), false);
  assert.equal(fanoutKnown(undefined, prominence(0)), false);
  assert.equal(fanoutKnown(undefined, prominence(3)), true);
});
