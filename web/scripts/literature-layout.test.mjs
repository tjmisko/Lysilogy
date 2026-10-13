import assert from 'node:assert/strict';
import test from 'node:test';
import { layoutFanout, MAX_NODES_PER_RING } from '../src/lib/literatureLayout.ts';

const size = { width: 800, height: 600 };
const makeNodes = (count) => Array.from({ length: count }, (_, i) => ({
  id: `w${String(i).padStart(4, '0')}`, title: `Paper ${i}`, year: 1990 + (i % 30),
}));
const ringSizes = (placements) => {
  const counts = new Map();
  for (const placement of placements) counts.set(placement.ring, (counts.get(placement.ring) ?? 0) + 1);
  return counts;
};

test('should place only the center when there are no nodes', () => {
  const placements = layoutFanout('c', [], size);
  assert.deepEqual(placements, [{ id: 'c', x: 400, y: 300, ring: 0 }]);
});

test('should place one node on the first ring when given a single node', () => {
  const placements = layoutFanout('c', [{ id: 'a', title: 'A', year: 2000 }], size);
  assert.equal(placements.length, 2);
  assert.equal(placements[1].ring, 1);
  assert.notDeepEqual([placements[1].x, placements[1].y], [400, 300]);
});

test('should spill to a second ring when given 37 nodes', () => {
  const placements = layoutFanout('c', makeNodes(37), size);
  const counts = ringSizes(placements);
  assert.equal(counts.get(1), MAX_NODES_PER_RING);
  assert.equal(counts.get(2), 1);
  assert.equal(placements.length, 38);
});

test('should cap every ring at 36 nodes when given 120 nodes', () => {
  const placements = layoutFanout('c', makeNodes(120), size);
  const counts = ringSizes(placements);
  assert.equal(counts.size, 5);
  for (const [ring, count] of counts) if (ring > 0) assert.ok(count <= MAX_NODES_PER_RING);
  assert.equal(counts.get(4), 12);
});

test('should keep every placement inside the canvas when given 120 nodes', () => {
  for (const placement of layoutFanout('c', makeNodes(120), size)) {
    assert.ok(placement.x >= 0 && placement.x <= size.width);
    assert.ok(placement.y >= 0 && placement.y <= size.height);
  }
});

test('should be deterministic when the input order differs', () => {
  const nodes = makeNodes(50);
  const first = layoutFanout('c', nodes, size);
  const second = layoutFanout('c', [...nodes].reverse(), size);
  assert.deepEqual(second, first);
});

test('should order by year then title when assigning positions', () => {
  const placements = layoutFanout('c', [
    { id: 'b', title: 'Beta', year: 2001 },
    { id: 'a', title: 'Alpha', year: 2001 },
    { id: 'old', title: 'Zeta', year: 1990 },
    { id: 'none', title: 'Early', year: null },
  ], size);
  assert.deepEqual(placements.slice(1).map((placement) => placement.id), ['old', 'a', 'b', 'none']);
});

test('should place untitled nodes after titled ones when years match', () => {
  const placements = layoutFanout('c', [
    { id: 'u2', title: null, year: 2000 },
    { id: 'u1', year: 2000 },
    { id: 't', title: 'Titled', year: 2000 },
    { id: 'blank', title: '   ', year: 2000 },
  ], size);
  assert.deepEqual(placements.slice(1).map((placement) => placement.id), ['t', 'blank', 'u1', 'u2']);
});

test('should collapse duplicate ids when nodes repeat', () => {
  const placements = layoutFanout('c', [
    { id: 'a', title: 'A', year: 2000 },
    { id: 'a', title: 'A again', year: 2001 },
    { id: 'b', title: 'B', year: 2002 },
  ], size);
  assert.deepEqual(placements.map((placement) => placement.id).sort(), ['a', 'b', 'c']);
});

test('should omit the center from neighbors when the center id appears in nodes', () => {
  const placements = layoutFanout('c', [{ id: 'c', title: 'Self', year: 2000 }, { id: 'a' }], size);
  assert.equal(placements.filter((placement) => placement.id === 'c').length, 1);
  assert.equal(placements.find((placement) => placement.id === 'c').ring, 0);
});

test('should stay finite when the canvas is tiny', () => {
  for (const placement of layoutFanout('c', makeNodes(10), { width: 10, height: 10 })) {
    assert.ok(Number.isFinite(placement.x) && Number.isFinite(placement.y));
  }
});
