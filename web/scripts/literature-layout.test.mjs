import assert from 'node:assert/strict';
import test from 'node:test';
import {
  HOP_RADIUS, labelRect, layoutFanout, layoutRankedFanout, layoutWalk, MAX_NODES_PER_RING, planVisibleLabels, walkViewBox,
} from '../src/lib/literatureLayout.ts';

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

const ids = (count) => Array.from({ length: count }, (_, i) => `r${String(count - i).padStart(3, '0')}`);

test('should keep server rank order when laying out ranked fanout', () => {
  const ranked = ['zeta', 'alpha', 'mid'];
  const placements = layoutRankedFanout('c', ranked, [], size);
  assert.deepEqual(placements.map((p) => p.id), ['c', 'zeta', 'alpha', 'mid']);
  assert.deepEqual(placements.map((p) => p.kind), ['center', 'work', 'work', 'work']);
  const many = layoutRankedFanout('c', ids(40), [], size);
  assert.deepEqual(many.slice(1).map((p) => p.ring), [...Array(36).fill(1), ...Array(4).fill(2)]);
});

test('should place clusters after the last ranked node', () => {
  const placements = layoutRankedFanout('c', ['a', 'b'], ['decade:2010', 'hidden'], size);
  assert.deepEqual(placements.map((p) => p.kind), ['center', 'work', 'work', 'cluster', 'cluster']);
  assert.deepEqual(placements.slice(3).map((p) => p.clusterKey), ['decade:2010', 'hidden']);
  assert.equal(placements[3].id, 'cluster:decade:2010');
  assert.equal(placements[1].clusterKey, undefined);
  assert.equal(new Set(placements.map((p) => p.ring)).size, 2);
});

test('should open a new ring when clusters overflow the outer ring', () => {
  const placements = layoutRankedFanout('c', ids(MAX_NODES_PER_RING), ['k1', 'k2'], size);
  const counts = ringSizes(placements);
  assert.equal(counts.get(1), MAX_NODES_PER_RING);
  assert.equal(counts.get(2), 2);
  assert.ok(placements.filter((p) => p.ring === 2).every((p) => p.kind === 'cluster'));
});

test('should ignore duplicate and center ids when laying out ranked fanout', () => {
  const placements = layoutRankedFanout('c', ['a', 'c', 'a', 'b'], ['k', 'k'], size);
  assert.deepEqual(placements.map((p) => p.id), ['c', 'a', 'b', 'cluster:k']);
  assert.deepEqual(layoutRankedFanout('c', [], [], size), [{ id: 'c', x: 400, y: 300, ring: 0, kind: 'center' }]);
});

const labelAt = (id, x, y, overrides = {}) => ({
  id, x, y, length: 20, anchor: 'start', fontSize: 10, offsetY: 3, forced: false, ...overrides,
});

test('should hide a label when it would overlap one already kept', () => {
  const visible = planVisibleLabels([labelAt('a', 100, 100), labelAt('b', 100, 104), labelAt('c', 100, 140)], []);
  assert.deepEqual([...visible].sort(), ['a', 'c']);
});

test('should keep forced labels and let them displace ranked ones', () => {
  const visible = planVisibleLabels([labelAt('a', 100, 100), labelAt('b', 100, 104, { forced: true })], []);
  assert.deepEqual([...visible], ['b']);
});

test('should hide labels that collide with reserved rectangles', () => {
  const reserved = [{ left: 90, top: 90, right: 300, bottom: 115 }];
  assert.deepEqual([...planVisibleLabels([labelAt('a', 100, 100), labelAt('far', 100, 300)], reserved)], ['far']);
});

test('should keep labels on opposite sides of a dot when they do not collide', () => {
  const visible = planVisibleLabels([labelAt('r', 200, 100), labelAt('l', 200, 100, { anchor: 'end' })], []);
  assert.deepEqual([...visible].sort(), ['l', 'r']);
});

test('should never show overlapping labels when the ring is crowded', () => {
  const placements = layoutRankedFanout('c', ids(36), [], size).slice(1);
  const requests = placements.map((p) => labelAt(p.id, p.x, p.y, { anchor: p.x >= size.width / 2 ? 'start' : 'end', length: 36 }));
  const visible = planVisibleLabels(requests, []);
  const rects = requests.filter((r) => visible.has(r.id)).map(labelRect);
  assert.ok(rects.length > 0);
  for (let i = 0; i < rects.length; i += 1) {
    for (let j = i + 1; j < rects.length; j += 1) {
      const a = rects[i]; const b = rects[j];
      assert.ok(a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top);
    }
  }
});

const walkBase = layoutRankedFanout('c', ['a', 'b', 'd', 'e'], [], size);
const walkCenter = { x: 400, y: 300 };
const at = (placements, id) => placements.find((placement) => placement.id === id);
const kids = (count, prefix = 'k') => Array.from({ length: count }, (_, i) => `${prefix}${i}`);
const angleBetween = (left, right) => {
  const dot = left.x * right.x + left.y * right.y;
  return Math.acos(Math.max(-1, Math.min(1, dot / (Math.hypot(left.x, left.y) * Math.hypot(right.x, right.y)))));
};

test('should place walk children outward from their parent', () => {
  const layout = layoutWalk(walkBase, [{ parentId: 'a', childIds: kids(8), depth: 2 }], walkCenter);
  const parent = at(walkBase, 'a');
  assert.equal(layout.placements.length, 8);
  for (const child of layout.placements) {
    const away = { x: child.x - parent.x, y: child.y - parent.y };
    const outward = { x: parent.x - walkCenter.x, y: parent.y - walkCenter.y };
    assert.ok(away.x * outward.x + away.y * outward.y > 0);
    assert.ok(Math.abs(Math.hypot(away.x, away.y) - HOP_RADIUS) < 0.1);
    assert.equal(child.hop, 2);
    assert.equal(child.parentId, 'a');
  }
});

test('should spread children within sixty degrees of the outward direction', () => {
  const layout = layoutWalk(walkBase, [{ parentId: 'a', childIds: kids(8), depth: 2 }], walkCenter);
  const parent = at(walkBase, 'a');
  const outward = { x: parent.x - walkCenter.x, y: parent.y - walkCenter.y };
  for (const child of layout.placements) {
    assert.ok(angleBetween({ x: child.x - parent.x, y: child.y - parent.y }, outward) <= Math.PI / 3 + 1e-3);
  }
  const single = layoutWalk(walkBase, [{ parentId: 'a', childIds: ['k0'], depth: 2 }], walkCenter);
  assert.ok(angleBetween({ x: single.placements[0].x - parent.x, y: single.placements[0].y - parent.y }, outward) < 1e-3);
});

test('should keep children in rank order around the arc', () => {
  const layout = layoutWalk(walkBase, [{ parentId: 'a', childIds: kids(4), depth: 2 }], walkCenter);
  assert.deepEqual(layout.placements.map((placement) => placement.id), kids(4));
  assert.deepEqual(layout.walkEdges.map((edge) => edge.to), kids(4));
});

test('should draw cross link instead of duplicating a placed work', () => {
  const layout = layoutWalk(walkBase, [{ parentId: 'a', childIds: ['b', 'k0', 'c', 'a'], depth: 2 }], walkCenter);
  assert.deepEqual(layout.placements.map((placement) => placement.id), ['k0']);
  assert.deepEqual(layout.crossLinks, [{ from: 'a', to: 'b' }, { from: 'a', to: 'c' }]);
  assert.deepEqual(layout.walkEdges, [{ from: 'a', to: 'k0' }]);
});

test('should not duplicate a work two expansions share', () => {
  const layout = layoutWalk(walkBase, [
    { parentId: 'a', childIds: ['s', 'k0'], depth: 2 },
    { parentId: 'b', childIds: ['s', 'k1'], depth: 2 },
  ], walkCenter);
  assert.deepEqual(layout.placements.map((placement) => placement.id), ['s', 'k0', 'k1']);
  assert.deepEqual(layout.crossLinks, [{ from: 'b', to: 's' }]);
});

test('should place a hop two expansion outward from its walk parent', () => {
  const first = layoutWalk(walkBase, [{ parentId: 'a', childIds: kids(3), depth: 2 }], walkCenter);
  const hop1 = at(first.placements, 'k1');
  const layout = layoutWalk(walkBase, [
    { parentId: 'a', childIds: kids(3), depth: 2 },
    { parentId: 'k1', childIds: ['z0', 'z1'], depth: 3 },
  ], walkCenter);
  const parent = at(walkBase, 'a');
  for (const child of layout.placements.filter((placement) => placement.parentId === 'k1')) {
    assert.equal(child.hop, 3);
    assert.ok((child.x - hop1.x) * (hop1.x - parent.x) + (child.y - hop1.y) * (hop1.y - parent.y) > 0);
  }
});

test('should grow bounds to include walk children', () => {
  const none = layoutWalk(walkBase, [], walkCenter);
  const edge = at(walkBase, 'a');
  const layout = layoutWalk(walkBase, [{ parentId: 'a', childIds: kids(8), depth: 2 }], walkCenter);
  assert.ok(layout.bounds.minX <= none.bounds.minX && layout.bounds.maxX >= none.bounds.maxX);
  for (const child of layout.placements) {
    assert.ok(child.x >= layout.bounds.minX && child.x <= layout.bounds.maxX);
    assert.ok(child.y >= layout.bounds.minY && child.y <= layout.bounds.maxY);
  }
  const far = [{ ...edge, x: 990, y: 350 }];
  const grown = layoutWalk([walkBase[0], ...far], [{ parentId: 'a', childIds: kids(3), depth: 2 }], walkCenter);
  assert.ok(grown.bounds.maxX > 1000);
  assert.notEqual(walkViewBox(size, grown.bounds), `0 0 ${size.width} ${size.height}`);
});

test('should keep the canvas viewBox when no walk child leaves it', () => {
  const layout = layoutWalk(walkBase, [], walkCenter);
  assert.equal(walkViewBox({ width: 800, height: 600 }, layout.bounds), '0 0 800 600');
});

test('should ignore expansion whose parent is not placed', () => {
  const layout = layoutWalk(walkBase, [{ parentId: 'ghost', childIds: kids(3), depth: 2 }], walkCenter);
  assert.deepEqual(layout.placements, []);
  assert.deepEqual(layout.walkEdges, []);
  assert.deepEqual(layout.crossLinks, []);
});

test('should ignore expansion of the center and an expansion with no children', () => {
  const layout = layoutWalk(walkBase, [
    { parentId: 'c', childIds: kids(2), depth: 1 }, { parentId: 'a', childIds: [], depth: 2 },
  ], walkCenter);
  assert.deepEqual(layout.placements, []);
});

test('should be deterministic for the same input', () => {
  const expansions = [
    { parentId: 'a', childIds: kids(5), depth: 2 },
    { parentId: 'k2', childIds: kids(3, 'z'), depth: 3 },
  ];
  assert.deepEqual(layoutWalk(walkBase, expansions, walkCenter), layoutWalk(walkBase, expansions, walkCenter));
});
