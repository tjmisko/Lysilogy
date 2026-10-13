import assert from 'node:assert/strict';
import test from 'node:test';
import {
  COLLAPSED_AUTHOR_LIMIT, HISTOGRAM_GEOMETRY, HISTOGRAM_NAV_KEYS, REVEAL_MARGINS, UNKNOWN_COLUMN_KEY,
  authorLine, buildHistogramGrid, compareHistogramEntries, directionForKey, histogramEntries, initialCursor,
  layoutHistogram, markRank, moveCursor, parseLiteratureView, positionOf, reconcileCursor, rememberCursor,
  scrollToReveal, seedLinkPath, seedLinkTarget, titleKey, workAt,
} from '../src/lib/histogramLayout.ts';

const G = HISTOGRAM_GEOMETRY;

function entry(id, year, score = 0, { mark = null, title = id } = {}) {
  return { id, year, score, markRank: markRank(mark), titleKey: titleKey(title) };
}
const gridOf = (entries) => buildHistogramGrid(entries);
const ids = (grid) => grid.columns.map((column) => column.entries.map((e) => e.id));
const work = (id, year, title = id) => ({ id, display_year: year, display_title: title });

test('should exclude the center and duplicate ids when building entries', () => {
  const nodes = [work('c', 2000), work('a', 2001, 'First'), work('a', 2005, 'Second'), work('b', 2002)];
  const entries = histogramEntries({ center: 'c', nodes }, () => null, () => 1);
  assert.deepEqual(entries.map((e) => e.id), ['a', 'b']);
  assert.equal(entries[0].year, 2001);
  assert.equal(entries[0].titleKey, 'first');
});

test('should treat missing and NaN scores as zero when building entries', () => {
  const nodes = [work('a', 2000), work('b', 2000), work('c', 2000), work('d', 2000)];
  const scores = { a: undefined, b: NaN, c: 0.5, d: Infinity };
  const entries = histogramEntries({ center: 'x', nodes }, () => null, (id) => scores[id]);
  assert.deepEqual(entries.map((e) => e.score), [0, 0, 0.5, 0]);
});

test('should order pinned then unmarked then downplayed then hidden when sorting a column', () => {
  const grid = gridOf([
    entry('hid', 2000, 9, { mark: 'hidden' }),
    entry('down', 2000, 9, { mark: 'downplayed' }),
    entry('none', 2000, 1),
    entry('pin', 2000, 0, { mark: 'pinned' }),
  ]);
  assert.deepEqual(ids(grid), [['pin', 'none', 'down', 'hid']]);
});

test('should order by score descending within a mark rank when sorting a column', () => {
  const grid = gridOf([entry('a', 2000, 0.1), entry('b', 2000, 0.9), entry('c', 2000, 0.5)]);
  assert.deepEqual(ids(grid), [['b', 'c', 'a']]);
});

test('should break score ties by title then id when sorting a column', () => {
  const grid = gridOf([
    entry('z1', 2000, 1, { title: 'beta' }),
    entry('n1', 2000, 1, { title: '' }),
    entry('b2', 2000, 1, { title: 'Alpha' }),
    entry('a2', 2000, 1, { title: ' alpha ' }),
    entry('a1', 2000, 1, { title: 'ALPHA' }),
  ]);
  assert.deepEqual(ids(grid), [['a1', 'a2', 'b2', 'z1', 'n1']]);
  assert.equal(compareHistogramEntries(entry('a', 1, 1), entry('a', 1, 1)), 0);
});

test('should produce the same columns when entries are shuffled', () => {
  const base = [
    entry('a', 1999, 0.3), entry('b', 1999, 0.3, { title: 'x' }), entry('c', 2004, 0.9, { mark: 'pinned' }),
    entry('d', null, 0.2), entry('e', 2004, 0.9), entry('f', 1990, 0.1),
  ];
  const expected = JSON.stringify(gridOf(base));
  const permutations = [[5, 4, 3, 2, 1, 0], [2, 0, 4, 1, 5, 3], [3, 5, 1, 0, 2, 4], [1, 3, 5, 0, 4, 2]];
  for (const order of permutations) {
    assert.equal(JSON.stringify(gridOf(order.map((i) => base[i]))), expected);
  }
});

test('should put unknown years in a leftmost lane when years are null zero fractional or out of range', () => {
  const entries = [
    entry('n', null), entry('z', 0), entry('nan', NaN), entry('f', 1999.5), entry('hi', 2201), entry('neg', -5),
    entry('lo', 1), entry('top', 2200), entry('ok', 2000),
  ];
  const grid = gridOf(entries);
  assert.equal(grid.columns[0].key, UNKNOWN_COLUMN_KEY);
  assert.deepEqual(grid.columns[0].entries.map((e) => e.id).sort(), ['f', 'hi', 'n', 'nan', 'neg', 'z']);
  assert.deepEqual(grid.columns.slice(1).map((c) => c.key), ['year:1', 'year:2000', 'year:2200']);
  const layout = layoutHistogram(grid, null, () => false);
  const unknownX = layout.cards.find((c) => c.columnIndex === 0).x;
  for (const card of layout.cards.filter((c) => c.columnIndex > 0)) assert.ok(unknownX < card.x);
});

test('should omit the unknown lane when every year is known', () => {
  const grid = gridOf([entry('a', 2000), entry('b', 2001)]);
  assert.deepEqual(grid.columns.map((c) => c.key), ['year:2000', 'year:2001']);
  assert.equal(layoutHistogram(grid, null, () => false).axis.some((s) => s.kind === 'unknown'), false);
});

test('should draw short empty runs year by year and collapse long runs into one gap', () => {
  const grid = gridOf([entry('a', 2000), entry('b', 2002), entry('c', 2010)]);
  assert.deepEqual(grid.slots, [
    { kind: 'column', columnIndex: 0 },
    { kind: 'empty_year', year: 2001 },
    { kind: 'column', columnIndex: 1 },
    { kind: 'gap', fromYear: 2003, toYear: 2009 },
    { kind: 'column', columnIndex: 2 },
  ]);
  const three = gridOf([entry('a', 2000), entry('b', 2004)]);
  assert.deepEqual(three.slots.map((s) => s.kind), ['column', 'empty_year', 'empty_year', 'empty_year', 'column']);
  const four = gridOf([entry('a', 2000), entry('b', 2005)]);
  assert.deepEqual(four.slots.map((s) => s.kind), ['column', 'gap', 'column']);
});

test('should label occupied columns with year and count when building the axis', () => {
  const grid = gridOf([entry('u', null), entry('a', 2000), entry('b', 2000), entry('c', 2002), entry('d', 2010)]);
  const axis = layoutHistogram(grid, null, () => false).axis;
  assert.deepEqual(axis.map((s) => [s.kind, s.label, s.title, s.count]), [
    ['unknown', 'Year unknown', 'Year unknown · 1 reference', 1],
    ['column', '2000', '2000 · 2 references', 2],
    ['empty_year', '', '2001', 0],
    ['column', '2002', '2002 · 1 reference', 1],
    ['gap', '2003–2009', '2003–2009', 0],
    ['column', '2010', '2010 · 1 reference', 1],
  ]);
  assert.equal(axis[0].x, G.paddingX);
  assert.equal(axis[1].x, G.paddingX + G.cardWidth + G.unknownLaneGap);
  assert.equal(axis[2].x - axis[1].x, G.cardWidth + G.columnGap);
  assert.equal(axis[3].x - axis[2].x, G.emptyYearWidth);
  assert.equal(axis[5].x - axis[4].x, G.gapSlotWidth);
});

test('should skip empty years when moving right and left', () => {
  const grid = gridOf([entry('a', 2001), entry('b', 2005)]);
  assert.equal(moveCursor(grid, 'a', 'right'), 'b');
  assert.equal(moveCursor(grid, 'b', 'left'), 'a');
});

test('should land on the nearest row when moving between columns', () => {
  const tall = [0, 1, 2, 3, 4, 5].map((i) => entry(`t${i}`, 2000, 10 - i));
  const short = [0, 1, 2].map((i) => entry(`s${i}`, 2001, 10 - i));
  const grid = gridOf([...tall, ...short]);
  assert.equal(moveCursor(grid, 't5', 'right'), 's2');
  assert.equal(moveCursor(grid, 't1', 'right'), 's1');
  assert.equal(moveCursor(grid, 's2', 'left'), 't2');
});

test('should never leave the grid when moving past any edge', () => {
  const grid = gridOf([entry('a', 2000, 2), entry('b', 2000, 1), entry('c', 2003, 5)]);
  assert.equal(moveCursor(grid, 'a', 'left'), 'a');
  assert.equal(moveCursor(grid, 'c', 'right'), 'c');
  assert.equal(moveCursor(grid, 'a', 'up'), 'a');
  assert.equal(moveCursor(grid, 'b', 'down'), 'b');
  assert.equal(moveCursor(grid, 'c', 'down'), 'c');
});

test('should stay inside the grid when making many random moves', () => {
  let seed = 12345;
  const next = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed / 2 ** 32;
  };
  const entries = [];
  for (let i = 0; i < 40; i += 1) {
    entries.push(entry(`w${i}`, i % 7 === 0 ? null : 1990 + Math.floor(next() * 12), next()));
  }
  const grid = gridOf(entries);
  const members = new Set(entries.map((e) => e.id));
  const directions = ['left', 'right', 'up', 'down'];
  let cursor = null;
  for (let i = 0; i < 1000; i += 1) {
    cursor = moveCursor(grid, cursor, directions[Math.floor(next() * 4)]);
    assert.notEqual(cursor, null);
    assert.ok(members.has(cursor));
  }
});

test('should start at the most prominent work when the cursor is unset or unknown', () => {
  const grid = gridOf([
    entry('low', 2000, 0.2), entry('tiedLeft', 2001, 0.9, { title: 'a' }), entry('tiedRight', 2002, 0.9, { title: 'a' }),
  ]);
  assert.equal(initialCursor(grid), 'tiedLeft');
  assert.equal(moveCursor(grid, null, 'right'), 'tiedLeft');
  assert.equal(moveCursor(grid, 'foreign', 'down'), 'tiedLeft');
  const pinned = gridOf([entry('a', 2000, 9), entry('p', 2001, 0, { mark: 'pinned' })]);
  assert.equal(initialCursor(pinned), 'p');
});

test('should return null when moving in an empty grid', () => {
  const grid = gridOf([]);
  assert.equal(moveCursor(grid, null, 'down'), null);
  assert.equal(moveCursor(grid, 'x', 'left'), null);
  assert.equal(initialCursor(grid), null);
  assert.equal(reconcileCursor(grid, null), null);
  assert.equal(workAt(grid, { columnIndex: 0, row: 0 }), null);
});

test('should keep focus on the same work when data refreshes and reorders', () => {
  const before = gridOf([entry('a', 2000, 5), entry('b', 2000, 4), entry('c', 2001, 1)]);
  const memory = rememberCursor(before, 'b');
  assert.deepEqual(memory, { workId: 'b', columnKey: 'year:2000', position: { columnIndex: 0, row: 1 } });
  const after = gridOf([entry('n', 1990, 1), entry('a', 2000, 1), entry('b', 2000, 9), entry('c', 2001, 1)]);
  assert.deepEqual(positionOf(after, 'b'), { columnIndex: 1, row: 0 });
  assert.equal(reconcileCursor(after, memory), 'b');
});

test('should fall back to the same column at the clamped row when the focused work disappears', () => {
  const before = gridOf([entry('a', 2000, 5), entry('b', 2000, 4), entry('c', 2000, 3), entry('d', 2001, 1)]);
  const memory = rememberCursor(before, 'c');
  const after = gridOf([entry('a', 2000, 5), entry('b', 2000, 4), entry('d', 2001, 1)]);
  assert.equal(reconcileCursor(after, memory), 'b');
  assert.equal(rememberCursor(before, 'missing'), null);
  assert.equal(rememberCursor(before, null), null);
});

test('should fall back to the neighboring column when the focused column disappears', () => {
  const before = gridOf([entry('a', 2000, 1), entry('b', 2001, 5), entry('b2', 2001, 4), entry('c', 2002, 1), entry('c2', 2002, 0)]);
  const memory = rememberCursor(before, 'b2');
  const middleGone = gridOf([entry('a', 2000, 1), entry('c', 2002, 1), entry('c2', 2002, 0)]);
  assert.equal(reconcileCursor(middleGone, memory), 'c2');
  const lastGone = rememberCursor(before, 'c2');
  const tailGone = gridOf([entry('a', 2000, 1), entry('b', 2001, 5)]);
  assert.equal(reconcileCursor(tailGone, lastGone), 'b');
  assert.equal(reconcileCursor(tailGone, null), initialCursor(tailGone));
});

const tallGrid = () => gridOf([
  entry('a0', 2000, 5), entry('a1', 2000, 4), entry('a2', 2000, 3),
  entry('b0', 2001, 5), entry('b1', 2001, 4),
]);

test('should expand only the focused card when laying out', () => {
  const grid = tallGrid();
  const held = layoutHistogram(grid, 'a1', () => true);
  assert.deepEqual(held.cards.filter((c) => c.expanded).map((c) => [c.id, c.height]), [['a1', G.expandedHeldHeight]]);
  const unheld = layoutHistogram(grid, 'a1', () => false);
  assert.equal(unheld.cards.find((c) => c.id === 'a1').height, G.expandedUnheldHeight);
  assert.equal(layoutHistogram(grid, null, () => true).cards.some((c) => c.expanded), false);
  assert.equal(layoutHistogram(grid, 'foreign', () => true).cards.some((c) => c.expanded), false);
});

test('should move the expansion when focus changes', () => {
  const grid = tallGrid();
  const first = layoutHistogram(grid, 'a0', () => false).cards.filter((c) => c.expanded).map((c) => c.id);
  const second = layoutHistogram(grid, 'b1', () => false).cards.filter((c) => c.expanded).map((c) => c.id);
  assert.deepEqual(first, ['a0']);
  assert.deepEqual(second, ['b1']);
});

test('should shift only cards below the focused card in its column when expanding', () => {
  const grid = tallGrid();
  const flat = layoutHistogram(grid, null, () => false);
  const open = layoutHistogram(grid, 'a1', () => false);
  const delta = G.expandedUnheldHeight - G.collapsedHeight;
  const y = (layout, id) => layout.cards.find((c) => c.id === id).y;
  assert.equal(y(flat, 'a0'), G.cardTop);
  assert.equal(y(flat, 'a1'), G.cardTop + G.collapsedHeight + G.rowGap);
  assert.equal(y(open, 'a0'), y(flat, 'a0'));
  assert.equal(y(open, 'a1'), y(flat, 'a1'));
  assert.equal(y(open, 'a2'), y(flat, 'a2') + delta);
  assert.equal(y(open, 'b0'), y(flat, 'b0'));
  assert.equal(y(open, 'b1'), y(flat, 'b1'));
  assert.equal(open.height, flat.height + delta);
  assert.equal(flat.height, G.cardTop + 3 * G.collapsedHeight + 2 * G.rowGap + G.paddingBottom);
});

test('should size an empty layout to its minimum when there are no entries', () => {
  const layout = layoutHistogram(gridOf([]), null, () => false);
  assert.equal(layout.width, 2 * G.paddingX + G.cardWidth);
  assert.equal(layout.height, G.cardTop + G.paddingBottom);
});

test('should map vim keys and arrows to directions and ignore other keys', () => {
  assert.equal(directionForKey('h'), 'left');
  assert.equal(directionForKey('ArrowLeft'), 'left');
  assert.equal(directionForKey('l'), 'right');
  assert.equal(directionForKey('ArrowRight'), 'right');
  assert.equal(directionForKey('k'), 'up');
  assert.equal(directionForKey('ArrowUp'), 'up');
  assert.equal(directionForKey('j'), 'down');
  assert.equal(directionForKey('ArrowDown'), 'down');
  for (const key of ['H', 'Enter', 'o', 'w', '', 'Escape']) assert.equal(directionForKey(key), null);
  assert.equal(HISTOGRAM_NAV_KEYS.size, 10);
  for (const key of ['h', 'j', 'k', 'l', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter', 'o']) {
    assert.ok(HISTOGRAM_NAV_KEYS.has(key));
  }
});

test('should scroll minimally to reveal a card when it is outside the viewport', () => {
  const viewport = { left: 100, top: 100, width: 400, height: 300 };
  const m = REVEAL_MARGINS;
  assert.deepEqual(scrollToReveal({ x: 200, y: 200, width: 100, height: 50 }, viewport), { left: 100, top: 100 });
  assert.deepEqual(scrollToReveal({ x: 600, y: 200, width: 100, height: 50 }, viewport), { left: 600 + 100 + m.x - 400, top: 100 });
  assert.deepEqual(scrollToReveal({ x: 110, y: 200, width: 100, height: 50 }, viewport), { left: 110 - m.x, top: 100 });
  assert.deepEqual(scrollToReveal({ x: 200, y: 500, width: 100, height: 50 }, viewport), { left: 100, top: 500 + 50 + m.bottom - 300 });
  assert.deepEqual(scrollToReveal({ x: 200, y: 110, width: 100, height: 50 }, viewport), { left: 100, top: 110 - m.top });
  assert.deepEqual(scrollToReveal({ x: 200, y: 300, width: 100, height: 400 }, viewport), { left: 100, top: 300 - m.top });
  assert.deepEqual(scrollToReveal({ x: 20, y: 200, width: 100, height: 50 }, viewport), { left: 0, top: 100 });
  assert.deepEqual(scrollToReveal({ x: 0, y: 0, width: 100, height: 50 }, viewport), { left: 0, top: 0 });
});

test('should clamp the link target into the viewport and drop it when the card is off to the side', () => {
  const box = { x: 0, y: 84, width: 500, height: 400 };
  const card = { x: 300, y: 38, width: 176, height: 46 };
  assert.deepEqual(seedLinkTarget(card, { left: 100, top: 0 }, box), { x: 288, y: 122 });
  assert.deepEqual(seedLinkTarget(card, { left: 100, top: 200 }, box), { x: 288, y: 84 });
  assert.deepEqual(seedLinkTarget({ ...card, y: 1000 }, { left: 100, top: 0 }, box), { x: 288, y: 484 });
  assert.equal(seedLinkTarget(card, { left: 500, top: 0 }, box), null);
  assert.equal(seedLinkTarget({ ...card, x: 600 }, { left: 0, top: 0 }, box), null);
});

test('should build a cubic path from seed to card when drawing the link', () => {
  assert.equal(seedLinkPath({ x: 100, y: 80 }, { x: 300, y: 200 }), 'M 100 80 C 100 140, 300 140, 300 200');
  assert.equal(seedLinkPath({ x: 10.04, y: 0.26 }, { x: 20.06, y: 1 }), 'M 10 0.3 C 10 0.6, 20.1 0.6, 20.1 1');
});

test('should format authors with a limit and return null when none are printable', () => {
  assert.equal(authorLine(null, 3), null);
  assert.equal(authorLine(undefined, 3), null);
  assert.equal(authorLine([], 3), null);
  assert.equal(authorLine(['  ', ''], 3), null);
  assert.equal(authorLine(['A', 'B', 'C', 'D', 'E'], COLLAPSED_AUTHOR_LIMIT), 'A, B, C +2');
  assert.equal(authorLine([' A ', '', 'B'], 3), 'A, B');
  assert.equal(authorLine(['A', 'B', 'C'], 3), 'A, B, C');
});

test('should parse only histogram as the histogram view when reading storage', () => {
  assert.equal(parseLiteratureView('histogram'), 'histogram');
  for (const value of [null, undefined, '', 'Histogram', 'radial', 'graph']) {
    assert.equal(parseLiteratureView(value), 'radial');
  }
});
