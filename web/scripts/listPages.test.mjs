import assert from 'node:assert/strict';
import test from 'node:test';
import {
  ARGUMENT_COMMANDS, parseAppCommand, parseAppRoute, resolveListTitle, routeHash, sameRoute,
} from '../src/lib/listRoute.ts';
import {
  describeEdge, manualOrder, moveRelative, moveSelection, nextReadingStatus, orderAfterRestore, sameOrder,
  listErrorMessage, selectionAfterRemoval, stepAmongRows, stepSequence, topicChange,
} from '../src/lib/listKeys.ts';
import {
  GROUP_PREFIX, INITIAL_VIEW_STATE, foldKeyAt, foldKeys, groupCursorId, groupKeyOf, nodeIdOf, reorderBlockReason,
  sequenceModel, viewOrder,
} from '../src/lib/listViewModel.ts';
import { claimFromText, dedupeHits, fuzzyScore, hitToAdd, rankByTitle } from '../src/lib/listSearch.ts';

// ---- routes -------------------------------------------------------------------------------

test('should parse the list routes when the hash names them', () => {
  assert.deepEqual(parseAppRoute('#lists'), { kind: 'lists' });
  assert.deepEqual(parseAppRoute('#history'), { kind: 'history' });
  assert.deepEqual(parseAppRoute('#list=p123&view=3'), { kind: 'list', listId: 'p123', view: 3 });
});

test('should open Sequence when the view is missing or out of range', () => {
  for (const hash of ['#list=p1', '#list=p1&view=0', '#list=p1&view=8', '#list=p1&view=x', '#list=p1&view=2.5']) {
    assert.equal(parseAppRoute(hash).view, 1, hash);
  }
});

test('should return no route when the hash is home, a paper, empty, or malformed', () => {
  for (const hash of ['', '#', '#home', '#paper=abc', '#list=', '#list=%20', '#listsx', '#paper=abc&list=p1']) {
    assert.equal(parseAppRoute(hash), null, hash);
  }
});

test('should round trip a route through its hash when the list id needs escaping', () => {
  const route = { kind: 'list', listId: 'p a&b=c', view: 5 };
  assert.deepEqual(parseAppRoute(routeHash(route)), route);
  assert.equal(routeHash({ kind: 'lists' }), '#lists');
  assert.equal(routeHash({ kind: 'history' }), '#history');
});

test('should compare routes by their hash when either may be null', () => {
  assert.equal(sameRoute(null, null), true);
  assert.equal(sameRoute(null, { kind: 'lists' }), false);
  assert.equal(sameRoute({ kind: 'list', listId: 'p1', view: 1 }, { kind: 'list', listId: 'p1', view: 2 }), false);
  assert.equal(sameRoute({ kind: 'lists' }, { kind: 'lists' }), true);
});

// ---- commands -----------------------------------------------------------------------------

test('should keep the case of the argument when parsing list commands', () => {
  assert.deepEqual(parseAppCommand('list RLHF Basics'), { kind: 'list', title: 'RLHF Basics' });
  assert.deepEqual(parseAppCommand('  LIST-NEW   Café  reading '), { kind: 'list-new', title: 'Café  reading' });
  assert.deepEqual(parseAppCommand('generate learn RLHF reward modelling'), { kind: 'generate', goal: 'learn RLHF reward modelling' });
  assert.deepEqual(parseAppCommand('delete-list RLHF basics'), { kind: 'delete-list', title: 'RLHF basics' });
});

test('should parse the argument-free commands when given no argument', () => {
  assert.deepEqual(parseAppCommand('lists'), { kind: 'lists' });
  assert.deepEqual(parseAppCommand('history'), { kind: 'history' });
  assert.deepEqual(parseAppCommand('verify'), { kind: 'verify' });
  assert.deepEqual(parseAppCommand('read'), { kind: 'read-status', status: 'read' });
  assert.deepEqual(parseAppCommand('Reading'), { kind: 'read-status', status: 'reading' });
  assert.deepEqual(parseAppCommand('unread'), { kind: 'read-status', status: 'unread' });
});

test('should report an error when a command misses or gets an argument', () => {
  for (const raw of ['list', 'list-new   ', 'generate', 'delete-list', 'lists now', 'read it', 'order', 'order sideways']) {
    assert.equal(parseAppCommand(raw)?.kind, 'error', raw);
  }
});

test('should reject over-long titles and goals when parsing', () => {
  assert.equal(parseAppCommand(`list-new ${'x'.repeat(121)}`).kind, 'error');
  assert.equal(parseAppCommand(`list-new ${'x'.repeat(120)}`).kind, 'list-new');
  assert.equal(parseAppCommand(`generate ${'g'.repeat(2001)}`).kind, 'error');
  assert.equal(parseAppCommand(`generate ${'g'.repeat(2000)}`).kind, 'generate');
});

test('should count characters not code units when limiting a title', () => {
  assert.equal(parseAppCommand(`list-new ${'📚'.repeat(120)}`).kind, 'list-new');
});

test('should return null when the command is not a list command', () => {
  for (const raw of ['', '   ', 'analyze codex', 'home', 'help', 'listing']) assert.equal(parseAppCommand(raw), null, raw);
});

test('should normalise the order strategy when parsing :order', () => {
  assert.deepEqual(parseAppCommand('order topological'), { kind: 'order', strategy: 'topological' });
  assert.deepEqual(parseAppCommand('order Foundational-First'), { kind: 'order', strategy: 'foundational_first' });
  assert.deepEqual(parseAppCommand('order year'), { kind: 'order', strategy: 'year' });
  assert.equal(parseAppCommand('order __proto__').kind, 'error');
});

test('should name every argument command so the menu runs them as typed', () => {
  for (const name of ARGUMENT_COMMANDS) assert.notEqual(parseAppCommand(`${name} x`), null, name);
});

test('should resolve a title by id, exact title, or unique prefix when looking up a list', () => {
  const lists = [
    { id: 'p1', title: 'RLHF basics' },
    { id: 'p2', title: 'RLHF basics, advanced' },
    { id: 'p3', title: 'Optimal transport' },
  ];
  assert.equal(resolveListTitle(lists, 'p3').list.title, 'Optimal transport');
  assert.equal(resolveListTitle(lists, 'rlhf BASICS').list.id, 'p1');
  assert.equal(resolveListTitle(lists, 'opt').list.id, 'p3');
  assert.equal(resolveListTitle(lists, 'rlhf').kind, 'ambiguous');
  assert.equal(resolveListTitle(lists, 'rlhf').matches.length, 2);
  assert.equal(resolveListTitle(lists, 'quantum').kind, 'missing');
  assert.equal(resolveListTitle(lists, '   ').kind, 'missing');
  assert.equal(resolveListTitle([], 'a').kind, 'missing');
});

// ---- key sequences ------------------------------------------------------------------------

test('should complete a sequence when the second key matches', () => {
  assert.deepEqual(stepSequence('g', 'g'), { action: 'top', pending: '', consumed: true });
  assert.deepEqual(stepSequence('d', 'd'), { action: 'cut', pending: '', consumed: true });
  assert.equal(stepSequence('z', 'o').action, 'fold-open');
  assert.equal(stepSequence('z', 'c').action, 'fold-close');
  assert.equal(stepSequence('z', 'a').action, 'fold-toggle');
  assert.equal(stepSequence('z', 'R').action, 'fold-open-all');
  assert.equal(stepSequence('z', 'M').action, 'fold-close-all');
  assert.equal(stepSequence('z', 'g').action, 'group-toggle');
});

test('should wait for a second key when a prefix is typed', () => {
  for (const key of ['g', 'd', 'z']) assert.deepEqual(stepSequence('', key), { action: null, pending: key, consumed: true });
});

test('should cancel and swallow the key when the second key completes nothing', () => {
  assert.deepEqual(stepSequence('d', 'x'), { action: null, pending: '', consumed: true });
  assert.deepEqual(stepSequence('z', 'z'), { action: null, pending: '', consumed: true });
  assert.deepEqual(stepSequence('g', 'j'), { action: null, pending: '', consumed: true });
});

test('should leave single keys alone when no sequence is pending', () => {
  for (const key of ['j', 'x', 'p', 'P', 'J', 'u', 'G', 'D', '1']) {
    assert.deepEqual(stepSequence('', key), { action: null, pending: '', consumed: false }, key);
  }
});

test('should not match inherited object keys when stepping a sequence', () => {
  assert.equal(stepSequence('_', '_proto__').action, null);
  assert.equal(stepSequence('constructor', 'x').action, null);
});

// ---- status -------------------------------------------------------------------------------

test('should cycle unread to reading to read and back when pressing r', () => {
  assert.equal(nextReadingStatus('unread'), 'reading');
  assert.equal(nextReadingStatus('reading'), 'read');
  assert.equal(nextReadingStatus('read'), 'unread');
  assert.equal(nextReadingStatus('bogus'), 'reading');
});

// ---- reorder ------------------------------------------------------------------------------

test('should move a row before or after its target when putting', () => {
  const order = ['a', 'b', 'c', 'd'];
  assert.deepEqual(moveRelative(order, 'a', 'c', 'after'), ['b', 'c', 'a', 'd']);
  assert.deepEqual(moveRelative(order, 'a', 'c', 'before'), ['b', 'a', 'c', 'd']);
  assert.deepEqual(moveRelative(order, 'd', 'a', 'before'), ['d', 'a', 'b', 'c']);
  assert.deepEqual(moveRelative(order, 'a', 'd', 'after'), ['b', 'c', 'd', 'a']);
});

test('should change nothing when the put is degenerate', () => {
  const order = ['a', 'b', 'c'];
  assert.deepEqual(moveRelative(order, 'a', 'a', 'after'), order);
  assert.deepEqual(moveRelative(order, 'z', 'a', 'after'), order);
  assert.deepEqual(moveRelative(order, 'a', 'z', 'before'), order);
  assert.deepEqual(moveRelative([], 'a', 'b', 'before'), []);
});

test('should not mutate the given order when moving', () => {
  const order = Object.freeze(['a', 'b', 'c']);
  assert.deepEqual(moveRelative(order, 'c', 'a', 'before'), ['c', 'a', 'b']);
  assert.deepEqual(order, ['a', 'b', 'c']);
});

test('should step among the visible rows when topics sit between them', () => {
  const order = ['t1', 'a', 't2', 'b', 'c'];
  const rows = ['a', 'b', 'c'];
  assert.deepEqual(stepAmongRows(order, rows, 'a', 1), ['t1', 't2', 'b', 'a', 'c']);
  assert.deepEqual(stepAmongRows(order, rows, 'c', -1), ['t1', 'a', 't2', 'c', 'b']);
});

test('should change nothing when J or K steps past either end', () => {
  const order = ['a', 'b'];
  assert.deepEqual(stepAmongRows(order, ['a', 'b'], 'a', -1), order);
  assert.deepEqual(stepAmongRows(order, ['a', 'b'], 'b', 1), order);
  assert.deepEqual(stepAmongRows(order, ['a', 'b'], 'zz', 1), order);
  assert.deepEqual(stepAmongRows(order, [], 'a', 1), order);
});

test('should compare orders by value when checking whether a move did anything', () => {
  assert.equal(sameOrder(['a', 'b'], ['a', 'b']), true);
  assert.equal(sameOrder(['a', 'b'], ['b', 'a']), false);
  assert.equal(sameOrder(['a'], ['a', 'b']), false);
  assert.equal(sameOrder([], []), true);
});

test('should sort nodes by position then id when building the manual order', () => {
  const nodes = [
    { id: 'c', position: 20 }, { id: 'b', position: 10 }, { id: 'a', position: 10 }, { id: 'd', position: 0 },
  ];
  assert.deepEqual(manualOrder(nodes), ['d', 'a', 'b', 'c']);
  assert.deepEqual(manualOrder([]), []);
});

test('should put a restored node back at its old position when undoing a removal', () => {
  const others = [{ id: 'a', position: 0 }, { id: 'c', position: 20 }, { id: 'd', position: 30 }];
  assert.deepEqual(orderAfterRestore(others, 10, 'n'), ['a', 'n', 'c', 'd']);
  assert.deepEqual(orderAfterRestore(others, 99, 'n'), ['a', 'c', 'd', 'n']);
  assert.deepEqual(orderAfterRestore(others, -1, 'n'), ['n', 'a', 'c', 'd']);
  assert.deepEqual(orderAfterRestore([], 5, 'n'), ['n']);
});

test('should ignore the restored node when it already sits among the others', () => {
  const others = [{ id: 'a', position: 0 }, { id: 'n', position: 40 }, { id: 'c', position: 20 }];
  assert.deepEqual(orderAfterRestore(others, 10, 'n'), ['a', 'n', 'c']);
});

// ---- selection ----------------------------------------------------------------------------

test('should clamp the selection to the ends when moving', () => {
  const ids = ['a', 'b', 'c'];
  assert.equal(moveSelection(ids, 'a', -1), 'a');
  assert.equal(moveSelection(ids, 'c', 1), 'c');
  assert.equal(moveSelection(ids, 'a', 1), 'b');
  assert.equal(moveSelection(ids, 'a', 99), 'c');
  assert.equal(moveSelection(ids, 'b', -99), 'a');
});

test('should start at the matching end when nothing valid is selected', () => {
  assert.equal(moveSelection(['a', 'b'], null, 1), 'a');
  assert.equal(moveSelection(['a', 'b'], 'gone', 1), 'a');
  assert.equal(moveSelection(['a', 'b'], null, -1), 'b');
  assert.equal(moveSelection([], 'a', 1), null);
});

test('should pick the neighbor when the selected row is removed', () => {
  assert.equal(selectionAfterRemoval(['a', 'b', 'c'], 'b'), 'c');
  assert.equal(selectionAfterRemoval(['a', 'b', 'c'], 'c'), 'b');
  assert.equal(selectionAfterRemoval(['a'], 'a'), null);
  assert.equal(selectionAfterRemoval(['a', 'b'], 'zz'), 'a');
  assert.equal(selectionAfterRemoval([], 'a'), null);
});

// ---- edges --------------------------------------------------------------------------------

test('should diff topic sets when assigning topics', () => {
  assert.deepEqual(topicChange(['a', 'b'], ['b', 'c']), { add: ['c'], remove: ['a'] });
  assert.deepEqual(topicChange([], ['a', 'a']), { add: ['a'], remove: [] });
  assert.deepEqual(topicChange(['a'], []), { add: [], remove: ['a'] });
  assert.deepEqual(topicChange(['a'], ['a']), { add: [], remove: [] });
});

test('should word each edge from the selected node when listing its edges', () => {
  const title = (id) => ({ x: 'Xavier', y: 'Yolo' }[id] ?? id);
  assert.equal(describeEdge({ from: 's', to: 'x', kind: 'prerequisite' }, 's', title), 'prerequisite of Xavier');
  assert.equal(describeEdge({ from: 'y', to: 's', kind: 'prerequisite' }, 's', title), 'needs Yolo first');
  assert.equal(describeEdge({ from: 'y', to: 's', kind: 'covers' }, 's', title), 'under topic Yolo');
  assert.equal(describeEdge({ from: 's', to: 'x', kind: 'covers' }, 's', title), 'covers Xavier');
  assert.equal(describeEdge({ from: 's', to: 'x', kind: 'subtopic' }, 's', title), 'has subtopic Xavier');
  assert.equal(describeEdge({ from: 'y', to: 's', kind: 'subtopic' }, 's', title), 'subtopic of Yolo');
});

// ---- search -------------------------------------------------------------------------------

test('should match letters in order when fuzzy scoring', () => {
  assert.notEqual(fuzzyScore('rlhf', 'RLHF basics'), null);
  assert.equal(fuzzyScore('zq', 'RLHF basics'), null);
  assert.equal(fuzzyScore('', 'anything'), 0);
  assert.equal(fuzzyScore('  ', 'anything'), 0);
  assert.ok(fuzzyScore('rl', 'RL basics') > fuzzyScore('rl', 'Optimal transport and ranking learning'));
});

test('should rank by score and keep the given order when the query is blank', () => {
  const items = [{ title: 'Optimal transport' }, { title: 'RLHF basics' }, { title: 'Reward learning' }];
  assert.deepEqual(rankByTitle(items, '  ').map((item) => item.title), items.map((item) => item.title));
  assert.equal(rankByTitle(items, 'rlhf')[0].title, 'RLHF basics');
  assert.deepEqual(rankByTitle(items, 'zzz'), []);
  assert.deepEqual(rankByTitle([], 'a'), []);
});

test('should match unicode titles when fuzzy scoring', () => {
  assert.notEqual(fuzzyScore('café', 'Café au lait'), null);
  assert.notEqual(fuzzyScore('日本', '日本語の本'), null);
});

test('should build a claim from typed text when no search hit matches', () => {
  assert.deepEqual(claimFromText('  Attention is all you need  '), { claim: { title: 'Attention is all you need', raw: 'Attention is all you need' } });
  assert.deepEqual(claimFromText('Attention is all you need (2017)').claim.year, 2017);
  assert.equal(claimFromText('Attention is all you need (2017)').claim.title, 'Attention is all you need');
  assert.equal(claimFromText('   '), null);
  assert.equal(claimFromText(''), null);
});

test('should read identifiers when the typed text is a DOI or arXiv id', () => {
  assert.equal(claimFromText('10.1145/3292500.3330701').claim.doi, '10.1145/3292500.3330701');
  assert.equal(claimFromText('https://doi.org/10.1145/3292500.3330701').claim.doi, '10.1145/3292500.3330701');
  assert.equal(claimFromText('arXiv:1706.03762').claim.arxiv, '1706.03762');
  assert.equal(claimFromText('1706.03762v5').claim.arxiv, '1706.03762');
});

test('should cap the title length when the typed text is huge', () => {
  assert.equal([...claimFromText('x'.repeat(2000)).claim.title].length, 500);
});

test('should add a Work by id, a held paper by paper id, and a provider hit as a claim when turning hits into adds', () => {
  const base = { title: 'T', year: 2020, authors: ['A B'], source: 'graph', in_graph: true, openalex_id: null, doi: null };
  assert.deepEqual(hitToAdd({ ...base, work_id: 'w1', paper_id: 'x' }), { work_id: 'w1' });
  assert.deepEqual(hitToAdd({ ...base, work_id: null, paper_id: 'x' }), { paper_id: 'x' });
  assert.deepEqual(hitToAdd({ ...base, work_id: null, paper_id: null, source: 'provider', in_graph: false, doi: '10.1/x' }),
    { claim: { title: 'T', authors: ['A B'], year: 2020, doi: '10.1/x' } });
});

test('should show a Work once when library and graph search both find it', () => {
  const hit = (over) => ({ work_id: null, paper_id: null, title: 'T', year: null, authors: [], source: 'graph', in_graph: true, openalex_id: null, doi: null, ...over });
  const kept = dedupeHits([hit({ work_id: 'w1' }), hit({ work_id: 'w1', source: 'library' }), hit({ work_id: 'w2' }), hit({ title: 'Same', year: 1999 }), hit({ title: 'same', year: 1999 }), hit({ title: 'Same', year: 2000 })]);
  assert.equal(kept.length, 4);
  assert.deepEqual(dedupeHits([]), []);
});

// ---- errors -------------------------------------------------------------------------------

test('should word the list error codes when the server rejects an add', () => {
  assert.equal(listErrorMessage({ code: 'item_exists', message: 'x' }), 'Already on this list.');
  assert.match(listErrorMessage({ code: 'list_full', message: 'x' }), /full/u);
  assert.match(listErrorMessage({ code: 'paper_not_linked', message: 'x' }), /not linked/u);
  assert.match(listErrorMessage({ code: 'no_citation_edge', message: 'x' }), /citation/u);
  assert.match(listErrorMessage({ code: 'via_not_in_list', message: 'x' }), /not on this list/u);
});

test('should name the cycle by titles when an edge would close one', () => {
  const titles = { a: 'Alpha', b: 'Beta' };
  assert.equal(
    listErrorMessage({ code: 'edge_cycle', message: 'x', cycle: ['a', 'b'] }, (id) => titles[id] ?? id),
    'That would close a cycle: Alpha → Beta → Alpha.',
  );
  assert.equal(listErrorMessage({ code: 'edge_cycle', message: 'x', cycle: null }), 'That would close a cycle.');
  assert.equal(listErrorMessage({ code: 'edge_cycle', message: 'x', cycle: [] }), 'That would close a cycle.');
});

test('should pass the server message through when the code is unknown or missing', () => {
  assert.equal(listErrorMessage({ code: 'weird', message: 'Boom.' }), 'Boom.');
  assert.equal(listErrorMessage({ code: null, message: 'Boom.' }), 'Boom.');
  assert.equal(listErrorMessage({ message: 'Boom.' }), 'Boom.');
});

// ---- view model ---------------------------------------------------------------------------

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
const work = (id, title, year) => node('work', id, {
  work_id: `w-${id}`, work: { id: `w-${id}`, display_title: title, display_year: year, display_authors: [] },
});
const topic = (id, label) => node('topic', id, { label });
const edge = (from, to, kind = 'covers') => ({
  list_id: 'p1', from, to, kind, source: 'user', review: 'accepted', created_at: '2026-01-01T00:00:00Z',
});
function detail(nodes, edges = []) {
  return {
    list: { id: 'p1', title: 'L', seed_work_id: null }, nodes, edges, citations: [],
    reading_states: {}, local_papers: {}, acquisitions: {}, expansions: {},
  };
}
const sample = () => {
  counter = 0;
  return detail(
    [topic('t1', 'Rewards'), work('a', 'Alpha', 2019), work('b', 'Beta', 2017), work('c', 'Gamma', 2021)],
    [edge('t1', 'a'), edge('t1', 'b')],
  );
};

test('should list rows in manual order when the Sequence is flat', () => {
  const model = sequenceModel(sample(), INITIAL_VIEW_STATE);
  assert.deepEqual(model.ids, ['a', 'b', 'c']);
  assert.equal(model.groups, null);
});

test('should put group headers among the cursor ids when the Sequence is grouped', () => {
  const model = sequenceModel(sample(), { ...INITIAL_VIEW_STATE, grouped: true });
  assert.deepEqual(model.ids, [groupCursorId('t1'), 'a', 'b', groupCursorId('unfiled'), 'c']);
});

test('should hide the rows of a folded group when the Sequence is grouped', () => {
  const model = sequenceModel(sample(), { ...INITIAL_VIEW_STATE, grouped: true, folded: new Set(['t1']) });
  assert.deepEqual(model.ids, [groupCursorId('t1'), groupCursorId('unfiled'), 'c']);
});

test('should apply the filter and sort when listing Sequence ids', () => {
  const byYear = sequenceModel(sample(), { ...INITIAL_VIEW_STATE, sort: 'year_asc' });
  assert.deepEqual(byYear.ids, ['b', 'a', 'c']);
  const filtered = sequenceModel(sample(), { ...INITIAL_VIEW_STATE, filter: 'gam' });
  assert.deepEqual(filtered.ids, ['c']);
  assert.deepEqual(sequenceModel(sample(), { ...INITIAL_VIEW_STATE, filter: 'zzz' }).ids, []);
});

test('should explain why reordering is blocked when the order is not manual', () => {
  assert.equal(reorderBlockReason(INITIAL_VIEW_STATE), null);
  assert.match(reorderBlockReason({ ...INITIAL_VIEW_STATE, sort: 'title' }), /manual/u);
  assert.match(reorderBlockReason({ ...INITIAL_VIEW_STATE, reverse: true }), /manual/u);
  assert.match(reorderBlockReason({ ...INITIAL_VIEW_STATE, filter: ' x ' }), /filter/u);
  assert.match(reorderBlockReason({ ...INITIAL_VIEW_STATE, grouped: true }), /zg/u);
  assert.equal(reorderBlockReason({ ...INITIAL_VIEW_STATE, filter: '   ' }), null);
});

test('should tell node ids from group cursors when resolving the selection', () => {
  assert.equal(nodeIdOf(null), null);
  assert.equal(nodeIdOf('a'), 'a');
  assert.equal(nodeIdOf(groupCursorId('t1')), 't1');
  assert.equal(nodeIdOf(groupCursorId('unfiled')), null);
  assert.equal(groupKeyOf('a'), null);
  assert.equal(groupKeyOf(`${GROUP_PREFIX}t1`), 't1');
  assert.equal(groupKeyOf(null), null);
});

test('should find the fold key of the group the cursor is in when folding', () => {
  const d = sample();
  const state = { ...INITIAL_VIEW_STATE, grouped: true };
  assert.equal(foldKeyAt(d, state, 1, 'a'), 't1');
  assert.equal(foldKeyAt(d, state, 1, 'c'), 'unfiled');
  assert.equal(foldKeyAt(d, state, 1, groupCursorId('t1')), 't1');
  assert.equal(foldKeyAt(d, INITIAL_VIEW_STATE, 1, 'a'), null);
  assert.equal(foldKeyAt(d, state, 1, null), null);
  assert.equal(foldKeyAt(d, state, 2, 't1'), 't1');
  assert.equal(foldKeyAt(d, state, 2, 'b'), 't1');
  assert.equal(foldKeyAt(d, state, 3, 'a'), null);
});

test('should list every fold key when closing all folds', () => {
  const d = sample();
  assert.deepEqual(foldKeys(d, { ...INITIAL_VIEW_STATE, folded: new Set(['t1']) }, 1), ['t1', 'unfiled']);
  assert.deepEqual(foldKeys(d, INITIAL_VIEW_STATE, 2), ['t1', 'unfiled']);
  assert.deepEqual(foldKeys(d, INITIAL_VIEW_STATE, 5), []);
});

test('should walk topics then their works when listing the Topics view', () => {
  assert.deepEqual(viewOrder(2, sample(), INITIAL_VIEW_STATE), ['t1', 'a', 'b', groupCursorId('unfiled'), 'c']);
  assert.deepEqual(viewOrder(2, sample(), { ...INITIAL_VIEW_STATE, folded: new Set(['t1']) }), ['t1', groupCursorId('unfiled'), 'c']);
});

test('should keep a work under two topics once when listing the Topics view', () => {
  counter = 0;
  const d = detail(
    [topic('t1', 'One'), topic('t2', 'Two'), work('a', 'Alpha', 2019)],
    [edge('t1', 'a'), edge('t2', 'a')],
  );
  assert.deepEqual(viewOrder(2, d, INITIAL_VIEW_STATE), ['t1', 'a', 't2']);
});

test('should order the other views by year, readiness and fetch need when listing ids', () => {
  assert.deepEqual(viewOrder(3, sample(), INITIAL_VIEW_STATE), ['b', 'a', 'c']);
  assert.deepEqual(viewOrder(4, sample(), INITIAL_VIEW_STATE), ['a', 'b', 'c']);
  assert.equal(viewOrder(5, sample(), INITIAL_VIEW_STATE).length, 3);
  assert.equal(viewOrder(6, sample(), INITIAL_VIEW_STATE).length, 3);
  assert.deepEqual(viewOrder(7, sample(), INITIAL_VIEW_STATE), []);
});

test('should list nothing when the list is empty', () => {
  const empty = detail([]);
  for (const view of [1, 2, 3, 4, 5, 6, 7]) assert.deepEqual(viewOrder(view, empty, INITIAL_VIEW_STATE), [], String(view));
  assert.deepEqual(sequenceModel(empty, { ...INITIAL_VIEW_STATE, grouped: true }).ids, [groupCursorId('unfiled')]);
});

test('should step the cursor to proposed items and claims when listing Review ids', () => {
  counter = 0;
  const proposed = work('p', 'Proposed', 2020);
  proposed.review = 'proposed';
  const claim = node('claim', 'k', { claim: { title: 'Ghost', authors: [], year: null, venue: null, doi: null, arxiv: null, raw: null }, claim_state: 'unverified' });
  const d = detail([work('a', 'Alpha', 2019), proposed, claim], [edge('a', 'p', 'prerequisite')]);
  d.edges[0].review = 'proposed';
  assert.deepEqual(viewOrder(7, d, INITIAL_VIEW_STATE), ['p', 'k']);
});
