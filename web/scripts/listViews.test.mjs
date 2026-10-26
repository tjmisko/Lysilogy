import assert from 'node:assert/strict';
import test from 'node:test';
import {
  SORT_CYCLE, UNFILED_KEY, acquire, filterRows, foldClose, foldCloseAll, foldOpen, foldOpenAll, foldToggle,
  graphModel, graphPlacements, groupByTopic, itemRows, nextSort, nextUp, nextView, parseView, progressStrip, pruneFolds, review, reviewItems,
  sequenceRows, statusGlyph, timeline, titleKey, topicLayers, countsLabel,
} from '../src/lib/listViews.ts';

let counter = 0;
function node(kind, id, over = {}) {
  counter += 1;
  return {
    id, list_id: 'p1', kind, work_id: null, label: null, summary: null, claim: null, claim_state: null,
    candidates: [], position: counter, note: null, rationale: null, context_links: [], source: 'user',
    review: 'accepted', via_work_id: null, path: null, generation_id: null,
    added_at: `2026-01-${String(counter).padStart(2, '0')}T00:00:00Z`, updated_at: '2026-01-01T00:00:00Z',
    work: null, ordinal: counter, depth: 0, path_steps: [], ...over,
  };
}
const work = (id, title, year, over = {}) => node('work', id, {
  work_id: `w-${id}`,
  work: { id: `w-${id}`, display_title: title, display_year: year, display_authors: [`Ann${id}son`] },
  ...over,
});
const topic = (id, label, over = {}) => node('topic', id, { label, ...over });
const claim = (id, title, state, over = {}) => node('claim', id, {
  claim: { title, authors: [], year: null, venue: null, doi: null, arxiv: null, raw: null },
  claim_state: state, ...over,
});
const edge = (from, to, kind = 'prerequisite', over = {}) => ({
  list_id: 'p1', from, to, kind, source: 'user', review: 'accepted', created_at: '2026-01-01T00:00:00Z', ...over,
});

function detail(nodes, { edges = [], citations = [], states = {}, papers = {}, acquisitions = {}, seed = null } = {}) {
  return {
    list: { id: 'p1', title: 'L', seed_work_id: seed },
    nodes, edges, citations,
    reading_states: Object.fromEntries(Object.entries(states).map(([id, status]) => [`w-${id}`, { status, changed_at: null }])),
    local_papers: Object.fromEntries(Object.entries(papers).map(([id, paper]) => [`w-${id}`, [{ paper_id: paper, title: id }]])),
    acquisitions: Object.fromEntries(Object.entries(acquisitions).map(([id, state]) => [`w-${id}`, { state, unavailable_reason: null }])),
    expansions: {},
  };
}
const ids = (rows) => rows.map((row) => row.id);

function sample() {
  counter = 0;
  return detail(
    [
      topic('tA', 'Alignment'), topic('tB', 'Reward models'),
      work('a', 'Attention Is All You Need', 2017), work('b', 'Deep RL from human preferences', 2017),
      work('c', 'Zebra paper', null), work('d', 'InstructGPT', 2022),
    ],
    {
      edges: [edge('tA', 'tB', 'subtopic'), edge('tA', 'a', 'covers', {}), edge('tB', 'b', 'covers'), edge('tB', 'd', 'covers')],
      citations: [['w-d', 'w-b'], ['w-b', 'w-a']],
      states: { a: 'read', b: 'reading' },
      papers: { a: 'paper-a' },
    },
  );
}

test('should number rows from one in manual order when sorting manually', () => {
  const rows = sequenceRows(sample());
  assert.deepEqual(ids(rows), ['a', 'b', 'c', 'd']);
  assert.deepEqual(rows.map((row) => row.ordinal), [1, 2, 3, 4]);
});

test('should step through every sort and wrap when cycling sorts', () => {
  assert.equal(SORT_CYCLE.length, 8);
  let sort = 'manual';
  const seen = [];
  for (let i = 0; i < SORT_CYCLE.length; i += 1) { seen.push(sort); sort = nextSort(sort); }
  assert.deepEqual(seen, [...SORT_CYCLE]);
  assert.equal(sort, 'manual');
  assert.equal(nextSort('bogus'), 'manual');
});

test('should wrap views and fall back to sequence when the view is out of range', () => {
  assert.equal(nextView(1), 2);
  assert.equal(nextView(7), 1);
  assert.equal(parseView('5'), 5);
  for (const bad of ['0', '8', 'x', '', null, undefined, 2.5]) assert.equal(parseView(bad), 1);
});

test('should put unknown years last in both year orders', () => {
  const d = sample();
  assert.deepEqual(ids(sequenceRows(d, { sort: 'year_asc' })), ['a', 'b', 'd', 'c']);
  assert.deepEqual(ids(sequenceRows(d, { sort: 'year_desc' })), ['d', 'a', 'b', 'c']);
});

test('should reverse the sorted order when reverse is set', () => {
  assert.deepEqual(ids(sequenceRows(sample(), { reverse: true })), ['d', 'c', 'b', 'a']);
  assert.deepEqual(ids(sequenceRows(sample(), { sort: 'year_asc', reverse: true })), ['c', 'd', 'b', 'a']);
});

test('should sort by title key ignoring case and punctuation', () => {
  assert.deepEqual(ids(sequenceRows(sample(), { sort: 'title' })), ['a', 'b', 'd', 'c']);
  assert.equal(titleKey('  Dé-ep   RL! '), 'dé ep rl');
  assert.equal(titleKey('日本語 タイトル'), '日本語 タイトル');
});

test('should sort active work first when sorting by status', () => {
  assert.deepEqual(ids(sequenceRows(sample(), { sort: 'status' })), ['b', 'c', 'd', 'a']);
});

test('should sort newest added first when sorting by added', () => {
  assert.deepEqual(ids(sequenceRows(sample(), { sort: 'added' })), ['d', 'c', 'b', 'a']);
});

test('should sort by citations received when sorting by prominence', () => {
  const rows = sequenceRows(sample(), { sort: 'prominence' });
  assert.deepEqual(ids(rows).slice(0, 2), ['a', 'b']);
  const given = sequenceRows(sample(), { sort: 'prominence', prominence: { 'w-d': 9 } });
  assert.equal(given[0].id, 'd');
});

test('should place cited work before citing work when sorting topologically', () => {
  const order = ids(sequenceRows(sample(), { sort: 'topological' }));
  assert.ok(order.indexOf('a') < order.indexOf('b'));
  assert.ok(order.indexOf('b') < order.indexOf('d'));
});

test('should honor a prerequisite edge over a citation when sorting topologically', () => {
  counter = 0;
  const d = detail([work('x', 'X', 2000), work('y', 'Y', 2001)], { edges: [edge('y', 'x')], citations: [['w-y', 'w-x']] });
  assert.deepEqual(ids(sequenceRows(d, { sort: 'topological' })), ['y', 'x']);
});

test('should keep the seed last when sorting topologically', () => {
  counter = 0;
  const d = detail([work('s', 'Seed', 1990, { depth: 0 }), work('x', 'X', 2000)], { seed: 'w-s' });
  assert.deepEqual(ids(sequenceRows(d, { sort: 'topological' })), ['x', 's']);
});

test('should match every term across title, author, topic, note, year and status when filtering', () => {
  const d = sample();
  assert.deepEqual(ids(sequenceRows(d, { filter: 'instruct' })), ['d']);
  assert.deepEqual(ids(sequenceRows(d, { filter: 'annbson' })), ['b']);
  assert.deepEqual(ids(sequenceRows(d, { filter: 'reward 2017' })), ['b']);
  assert.deepEqual(ids(sequenceRows(d, { filter: 'READ' })).sort(), ['a', 'b', 'c', 'd']);
  assert.deepEqual(ids(sequenceRows(d, { filter: 'reading' })), ['b']);
  assert.deepEqual(ids(sequenceRows(d, { filter: 'nothing like this' })), []);
});

test('should keep every row and renumber filtered rows when the filter is blank', () => {
  assert.equal(sequenceRows(sample(), { filter: '   ' }).length, 4);
  assert.deepEqual(sequenceRows(sample(), { filter: 'zebra' }).map((row) => row.ordinal), [1]);
  assert.deepEqual(filterRows([], 'x'), []);
});

test('should match filters case-insensitively across unicode forms', () => {
  counter = 0;
  const d = detail([work('u', 'Über-Modelle für Ärzte', 2020)]);
  assert.equal(sequenceRows(d, { filter: 'ÜBER' }).length, 1);
  assert.equal(sequenceRows(d, { filter: 'ärzte' }).length, 1);
});

test('should show claims with their state and exclude topics and proposed nodes from the sequence', () => {
  counter = 0;
  const d = detail([
    topic('t', 'T'), claim('c', 'A claimed paper', 'ambiguous'),
    work('p', 'Proposed', 2020, { review: 'proposed' }), work('a', 'Accepted', 2021),
  ]);
  const rows = sequenceRows(d);
  assert.deepEqual(ids(rows), ['c', 'a']);
  assert.equal(rows[0].kind, 'claim');
  assert.equal(rows[0].claimState, 'ambiguous');
  assert.equal(rows[0].title, 'A claimed paper');
  assert.equal(rows[0].status, 'unread');
});

test('should fall back to Untitled when a work has no title', () => {
  counter = 0;
  const d = detail([work('n', null, null)]);
  assert.equal(itemRows(d)[0].title, 'Untitled');
});

test('should report held and fetch state per row', () => {
  const d = sample();
  d.acquisitions['w-b'] = { state: 'link_found', unavailable_reason: null };
  const rows = itemRows(d);
  assert.equal(rows[0].held, true);
  assert.equal(rows[0].paperId, 'paper-a');
  assert.equal(rows[1].held, false);
  assert.equal(rows[1].fetch, 'link_found');
  assert.deepEqual(rows[1].topics, ['Reward models']);
});

test('should map status to glyphs', () => {
  assert.deepEqual(['unread', 'reading', 'read'].map(statusGlyph), ['○', '◐', '●']);
});

test('should group rows under their first topic by position and leave the rest unfiled', () => {
  const d = sample();
  const groups = groupByTopic(d, sequenceRows(d), foldOpenAll());
  assert.deepEqual(groups.map((g) => [g.key, ids(g.rows)]), [['tA', ['a']], ['tB', ['b', 'd']], [UNFILED_KEY, ['c']]]);
});

test('should file a work once when it sits under several topics', () => {
  const d = sample();
  d.edges.push(edge('tA', 'b', 'covers'));
  const groups = groupByTopic(d, sequenceRows(d), foldOpenAll());
  assert.deepEqual(groups.map((g) => ids(g.rows)), [['a', 'b'], ['d'], ['c']]);
});

test('should hide rows of folded groups and empty groups on request', () => {
  const d = sample();
  const folded = foldClose(foldOpenAll(), 'tB');
  const groups = groupByTopic(d, sequenceRows(d), folded);
  assert.deepEqual(groups.find((g) => g.key === 'tB').visible, []);
  assert.equal(groups.find((g) => g.key === 'tB').rows.length, 2);
  const filtered = groupByTopic(d, sequenceRows(d, { filter: 'zebra' }), foldOpenAll(), true);
  assert.deepEqual(filtered.map((g) => g.key), [UNFILED_KEY]);
  assert.equal(groupByTopic(d, sequenceRows(d, { filter: 'zebra' }), foldOpenAll()).length, 3);
});

test('should offer an unfiled group when the list has no topics at all', () => {
  counter = 0;
  const d = detail([]);
  assert.deepEqual(groupByTopic(d, [], foldOpenAll()).map((g) => g.key), [UNFILED_KEY]);
});

test('should open, close, toggle and reset folds without mutating the old state', () => {
  const none = foldOpenAll();
  const one = foldClose(none, 'tA');
  assert.equal(none.size, 0);
  assert.deepEqual([...one], ['tA']);
  assert.deepEqual([...foldClose(one, 'tA')], ['tA']);
  assert.equal(foldToggle(one, 'tA').size, 0);
  assert.deepEqual([...foldToggle(none, 'tB')], ['tB']);
  assert.equal(foldOpen(one, 'tA').size, 0);
  assert.equal(foldOpen(one, 'other').size, 1);
  assert.deepEqual([...foldCloseAll(['x', 'y'])].sort(), ['x', 'y']);
});

test('should forget folds of topics that were deleted', () => {
  const d = sample();
  const pruned = pruneFolds(new Set(['tA', 'gone', UNFILED_KEY]), d);
  assert.deepEqual([...pruned].sort(), ['tA', UNFILED_KEY].sort());
});

test('should layer topics by longest path and list their own works', () => {
  const d = sample();
  const { layers, unfiled } = topicLayers(d);
  assert.deepEqual(layers.map((layer) => layer.map((e) => e.topic.id)), [['tA'], ['tB']]);
  assert.deepEqual(ids(layers[1][0].works), ['b', 'd']);
  assert.deepEqual(layers[1][0].progress, { unread: 1, reading: 1, read: 0 });
  assert.deepEqual(ids(unfiled.rows), ['c']);
});

test('should push a topic below every topic that must come before it', () => {
  counter = 0;
  const d = detail([topic('a', 'A'), topic('b', 'B'), topic('c', 'C')], {
    edges: [edge('a', 'b', 'subtopic'), edge('b', 'c'), edge('a', 'c')],
  });
  assert.deepEqual(topicLayers(d).layers.map((layer) => layer.map((e) => e.topic.id)), [['a'], ['b'], ['c']]);
});

test('should not loop forever when stored topic edges cycle', () => {
  counter = 0;
  const d = detail([topic('a', 'A'), topic('b', 'B')], { edges: [edge('a', 'b'), edge('b', 'a')] });
  const flat = topicLayers(d).layers.flat();
  assert.equal(flat.length, 2);
});

test('should mark folded topics and omit unfiled when every work is filed', () => {
  const d = sample();
  d.edges.push(edge('tA', 'c', 'covers'));
  const result = topicLayers(d, foldClose(foldOpenAll(), 'tA'));
  assert.equal(result.layers[0][0].folded, true);
  assert.equal(result.unfiled, null);
});

test('should bucket by year oldest first and keep undated rows apart when building the timeline', () => {
  const t = timeline(sample());
  assert.deepEqual(t.buckets.map((b) => [b.year, ids(b.rows)]), [[2017, ['a', 'b']], [2022, ['d']]]);
  assert.deepEqual(ids(t.undated), ['c']);
});

test('should link citing nodes to cited nodes once in the timeline', () => {
  const d = sample();
  d.citations.push(['w-d', 'w-b'], ['w-b', 'w-b'], ['w-zz', 'w-b']);
  assert.deepEqual(timeline(d).links, [{ from: 'd', to: 'b' }, { from: 'b', to: 'a' }]);
});

test('should return an empty timeline when the list is empty', () => {
  counter = 0;
  assert.deepEqual(timeline(detail([])), { buckets: [], undated: [], links: [] });
});

test('should list unread works whose prerequisites are read in reading order when computing next up', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001), work('c', 'C', 2002)], {
    edges: [edge('a', 'b'), edge('b', 'c')], states: { a: 'read' },
  });
  const result = nextUp(d);
  assert.deepEqual(ids(result.items.map((i) => i.row)), ['b']);
  assert.equal(result.items[0].unlocks, 1);
  assert.equal(result.empty, null);
});

test('should explain what blocks the first unread work when nothing is ready', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001)], { edges: [edge('a', 'b'), edge('b', 'a')], states: {} });
  const result = nextUp(d);
  assert.equal(result.empty, 'blocked');
  assert.ok(result.blocked.blockedBy.length >= 1);
});

test('should explain a blocked topic prerequisite when the first work waits on a reading work', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001)], { edges: [edge('a', 'b')], states: { a: 'reading' } });
  const result = nextUp(d);
  assert.equal(result.empty, 'blocked');
  assert.equal(result.blocked.row.id, 'b');
  assert.deepEqual(ids(result.blocked.blockedBy), ['a']);
});

test('should report no works or all started in the next up empty state', () => {
  counter = 0;
  assert.equal(nextUp(detail([])).empty, 'no_works');
  assert.equal(nextUp(detail([topic('t', 'T'), claim('c', 'C', 'pending')])).empty, 'no_works');
  const d = detail([work('a', 'A', 2000)], { states: { a: 'read' } });
  assert.equal(nextUp(d).empty, 'all_started');
});

test('should ignore proposed prerequisite edges when computing next up', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001)], { edges: [edge('a', 'b', 'prerequisite', { review: 'proposed' })] });
  assert.equal(nextUp(d).items.length, 2);
});

test('should list only works without a local paper and mark fetchable ones when acquiring', () => {
  const d = sample();
  d.acquisitions['w-b'] = { state: 'downloaded', unavailable_reason: null };
  d.acquisitions['w-c'] = { state: 'unavailable', unavailable_reason: 'paywall' };
  d.acquisitions['w-d'] = { state: 'link_found', unavailable_reason: null };
  const result = acquire(d);
  assert.deepEqual(ids(result.rows.map((r) => r.row)), ['b', 'c', 'd']);
  assert.deepEqual(result.rows.map((r) => r.fetchable), [false, false, true]);
  assert.deepEqual(result.fetchable, ['w-d']);
});

test('should leave claims out of acquire and treat a never-fetched work as fetchable', () => {
  counter = 0;
  const d = detail([claim('c', 'C', 'pending'), work('w', 'W', 2000)]);
  const result = acquire(d);
  assert.deepEqual(result.fetchable, ['w-w']);
  assert.equal(result.rows.length, 1);
});

test('should gather proposed items, proposed links and claims by state when reviewing', () => {
  counter = 0;
  const d = detail(
    [
      work('p', 'Proposed work', 2020, { review: 'proposed' }), topic('t', 'Proposed topic', { review: 'proposed' }),
      claim('c1', 'Pending claim', 'pending'), claim('c2', 'Contradicted', 'contradicted', { review: 'proposed' }),
      claim('c3', 'Another pending', 'pending'), work('a', 'Accepted', 2021),
    ],
    { edges: [edge('p', 'a', 'prerequisite', { review: 'proposed', source: 'agent' }), edge('a', 'p')] },
  );
  const sections = review(d);
  assert.deepEqual(sections.map((s) => s.key), ['nodes', 'edges', 'claim:pending', 'claim:contradicted']);
  assert.deepEqual(sections[0].items.map((i) => i.node.id), ['p', 't']);
  assert.equal(sections[1].items[0].fromTitle, 'Proposed work');
  assert.deepEqual(sections[2].items.map((i) => i.node.id), ['c1', 'c3']);
  assert.equal(sections[3].items[0].proposed, true);
  assert.equal(reviewItems(sections).length, 6);
});

test('should return no review sections when nothing awaits review', () => {
  assert.deepEqual(review(sample()), []);
});

test('should count statuses overall and per topic including subtopics when building the progress strip', () => {
  const strip = progressStrip(sample());
  assert.deepEqual(strip.total, { unread: 2, reading: 1, read: 1 });
  assert.deepEqual(strip.topics.map((t) => [t.id, t.counts]), [
    ['tA', { unread: 1, reading: 1, read: 1 }],
    ['tB', { unread: 1, reading: 1, read: 0 }],
  ]);
  assert.deepEqual(strip.unfiled, { unread: 1, reading: 0, read: 0 });
  assert.equal(countsLabel(strip.total), '1 read · 1 reading · 2 unread');
});

test('should not count claims or proposed works when building the progress strip', () => {
  counter = 0;
  const d = detail([claim('c', 'C', 'pending'), work('p', 'P', 2000, { review: 'proposed' })]);
  assert.deepEqual(progressStrip(d).total, { unread: 0, reading: 0, read: 0 });
});

test('should survive subtopic cycles when counting progress', () => {
  counter = 0;
  const d = detail([topic('a', 'A'), topic('b', 'B'), work('w', 'W', 2000)], {
    edges: [edge('a', 'b', 'subtopic'), edge('b', 'a', 'subtopic'), edge('b', 'w', 'covers')],
  });
  assert.deepEqual(progressStrip(d).topics.map((t) => t.counts.unread), [1, 1]);
});

const layerIds = (model) => model.layers.map((layer) => ids(layer));

test('should layer by longest prerequisite path when building the graph', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001), work('c', 'C', 2002), work('d', 'D', 2003)],
    { edges: [edge('a', 'b'), edge('b', 'c'), edge('a', 'c')] });
  assert.deepEqual(layerIds(graphModel(d)), [['a', 'd'], ['b'], ['c']]);
});

test('should put cited works before citing works when building the graph', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001), work('c', 'C', 2002)], { citations: [['w-c', 'w-b'], ['w-b', 'w-a']] });
  const model = graphModel(d);
  assert.deepEqual(layerIds(model), [['a'], ['b'], ['c']]);
  assert.deepEqual(model.links, [{ from: 'a', to: 'b', kind: 'citation' }, { from: 'b', to: 'c', kind: 'citation' }]);
});

test('should keep one link when a prerequisite and a citation join the same pair', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001)], { edges: [edge('a', 'b')], citations: [['w-b', 'w-a']] });
  assert.deepEqual(graphModel(d).links, [{ from: 'a', to: 'b', kind: 'prerequisite' }]);
});

test('should break a citation cycle deterministically when building the graph', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001)], { citations: [['w-a', 'w-b'], ['w-b', 'w-a']] });
  const model = graphModel(d);
  assert.deepEqual(layerIds(model), [['a'], ['b']]);
  assert.equal(model.links.length, 2);
  assert.deepEqual(graphModel(d), model);
});

test('should let a prerequisite win when it conflicts with a citation in the graph', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001)], { edges: [edge('b', 'a')], citations: [['w-b', 'w-a']] });
  assert.deepEqual(layerIds(graphModel(d)), [['b'], ['a']]);
});

test('should leave no layer empty when a topic-expanded prerequisite closes a cycle', () => {
  counter = 0;
  const d = detail([topic('T', 'T'), topic('U', 'U'), work('a', 'A', 2000), work('b', 'B', 2001)],
    { edges: [edge('a', 'T'), edge('b', 'U'), edge('T', 'b', 'covers'), edge('U', 'a', 'covers')] });
  const model = graphModel(d);
  assert.ok(model.layers.every((layer) => layer.length > 0));
  assert.deepEqual(layerIds(model), [['a'], ['b']]);
  assert.equal(model.links.length, 2);
  assert.deepEqual(graphModel(d), model);
});

test('should expand a topic prerequisite to its works and list topics when building the graph', () => {
  counter = 0;
  const d = detail([topic('t', 'Basics'), work('a', 'A', 2000), work('b', 'B', 2001)],
    { edges: [edge('t', 'a', 'covers'), edge('t', 'b')] });
  const model = graphModel(d);
  assert.deepEqual(model.links, [{ from: 'a', to: 'b', kind: 'prerequisite' }]);
  assert.deepEqual(model.topics, [{ id: 't', label: 'Basics', rowIds: ['a'] }]);
});

test('should place claims in the first layer and ignore proposed edges when building the graph', () => {
  counter = 0;
  const d = detail([claim('q', 'A claim', 'unverified'), work('a', 'A', 2000), work('b', 'B', 2001)],
    { edges: [edge('a', 'b', 'prerequisite', { review: 'proposed' })] });
  assert.deepEqual(layerIds(graphModel(d)), [['q', 'a', 'b']]);
});

test('should return no layers when the list is empty in the graph', () => {
  assert.deepEqual(graphModel(detail([])), { layers: [], links: [], topics: [] });
  assert.deepEqual(graphPlacements(graphModel(detail([]))).placements, []);
});

test('should place layers left to right and rows top down when laying out the graph', () => {
  counter = 0;
  const d = detail([work('a', 'A', 2000), work('b', 'B', 2001), work('c', 'C', 2002)], { edges: [edge('a', 'b')] });
  const { placements, width, height } = graphPlacements(graphModel(d));
  const at = Object.fromEntries(placements.map((place) => [place.id, place]));
  assert.ok(at.b.x > at.a.x);
  assert.equal(at.a.x, at.c.x);
  assert.ok(at.c.y > at.a.y);
  assert.ok(width > at.b.x && height > at.c.y);
});
