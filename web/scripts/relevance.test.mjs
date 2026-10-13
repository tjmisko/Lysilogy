import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_TOP_N, TOP_N_MAX, TOP_N_MIN, clusterAriaLabel, markAfterToggle, neighborQuery, nextTopN,
  nodeRadius, signalSummary, toggleInSet,
} from '../src/lib/relevance.ts';

const signals = (overrides = {}) => ({
  score: 0.5,
  signals: {
    citing_works_count: 3, cited_works_count: 0, cited_by_global_count: 1204, display_year: 2016,
    local_available: true, mention_count: 2, method_citation_count: 1, background_citation_count: 0,
    ...overrides,
  },
});

test('should clamp top-N within bounds when stepping', () => {
  assert.equal(nextTopN(DEFAULT_TOP_N, 1), 60);
  assert.equal(nextTopN(DEFAULT_TOP_N, -1), 20);
  assert.equal(nextTopN(20, -1), TOP_N_MIN);
  assert.equal(nextTopN(TOP_N_MIN, -1), TOP_N_MIN);
  assert.equal(nextTopN(190, 1), TOP_N_MAX);
  assert.equal(nextTopN(TOP_N_MAX, 1), TOP_N_MAX);
});

test('should clear mark when toggling the active mark', () => {
  assert.equal(markAfterToggle('pinned', 'pinned'), null);
});

test('should switch mark when toggling a different mark', () => {
  assert.equal(markAfterToggle('pinned', 'hidden'), 'hidden');
  assert.equal(markAfterToggle(null, 'downplayed'), 'downplayed');
});

test('should scale radius monotonically and clamp scores outside 0..1', () => {
  assert.equal(nodeRadius(0), 3.5);
  assert.equal(nodeRadius(1), 7);
  assert.ok(nodeRadius(0.2) < nodeRadius(0.6));
  assert.equal(nodeRadius(-4), 3.5);
  assert.equal(nodeRadius(9), 7);
  assert.equal(nodeRadius(Number.NaN), 3.5);
});

test('should build neighbor query with sorted deduplicated expand and tags', () => {
  const query = new URLSearchParams(neighborQuery({
    direction: 'both', limit: 40, expand: ['decade:2010', 'hidden', 'decade:2010'], tags: new Set(['t2', 't1', 't2']),
  }));
  assert.equal(query.get('direction'), 'both');
  assert.equal(query.get('limit'), '40');
  assert.equal(query.get('expand'), 'decade:2010,hidden');
  assert.equal(query.get('tags'), 't1,t2');
});

test('should omit empty parameters when building neighbor query', () => {
  assert.equal(neighborQuery({}), '');
  assert.equal(neighborQuery({ limit: 20, expand: [], tags: new Set(['']) }), 'limit=20');
  assert.equal(neighborQuery({ limit: Number.NaN }), '');
});

test('should describe signals in words when summarizing prominence', () => {
  assert.equal(signalSummary(signals()), 'cited by 3 here · 1,204 overall · 2 mentions · method citation · held locally · 2016');
  const sparse = signals({
    cited_by_global_count: null, display_year: null, local_available: false, mention_count: 0,
    method_citation_count: 0, background_citation_count: 1,
  });
  assert.equal(signalSummary(sparse), 'cited by 3 here · background citation');
  assert.match(signalSummary(signals({ mention_count: 1 })), /1 mention · /);
});

test('should describe cluster with count when labeling for screen readers', () => {
  assert.equal(clusterAriaLabel({ label: '2010s', count: 65 }), '2010s: 65 more works, press Enter to expand');
  assert.equal(clusterAriaLabel({ label: 'Hidden', count: 1 }), 'Hidden: 1 more work, press Enter to expand');
});

test('should add then remove a value when toggling in a set without mutating it', () => {
  const original = new Set(['a']);
  const added = toggleInSet(original, 'b');
  assert.deepEqual([...added].sort(), ['a', 'b']);
  assert.deepEqual([...original], ['a']);
  assert.deepEqual([...toggleInSet(added, 'a')], ['b']);
});
