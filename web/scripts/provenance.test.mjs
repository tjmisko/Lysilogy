import assert from 'node:assert/strict';
import test from 'node:test';
import {
  PROVENANCE_COLUMN_ORDER, activeOverride, cellValues, formatAuthors, overrideDraft, parseOverrideInput,
} from '../src/lib/provenance.ts';

let nextId = 0;
const claim = (origin, display, agreement, extra = {}) => ({
  id: `a${nextId++}`, origin, extractor: 'x', confidence: 'heuristic', value: display, display,
  review_state: 'unreviewed', retracted_at: null, agreement, excluded_reason: null,
  source_record: null, locator: null, entry: null, created_at: '2026-01-01T00:00:00Z', ...extra,
});
const field = (claims) => ({ field: 'title', selected: {}, claims });

test('should mark cell green when its value agrees with the selected value', () => {
  const cells = cellValues(field([claim('openalex', 'Same', 'agrees')]), 'openalex');
  assert.equal(cells[0].mark, 'agrees');
});

test('should mark cell red when its value disagrees with the selected value', () => {
  const cells = cellValues(field([claim('openalex', 'Other', 'disagrees')]), 'openalex');
  assert.equal(cells[0].mark, 'disagrees');
});

test('should mark the winning cell selected', () => {
  const cells = cellValues(field([
    claim('paper', 'Win', 'selected'), claim('paper', 'Win', 'agrees'),
  ]), 'paper');
  assert.equal(cells.length, 1);
  assert.equal(cells[0].mark, 'selected');
  assert.equal(cells[0].count, 2);
});

test('should return no values when a source has no claim', () => {
  assert.deepEqual(cellValues(field([claim('openalex', 'T', 'selected')]), 'publisher'), []);
  assert.deepEqual(cellValues(field([]), 'user'), []);
});

test('should mark excluded when every claim in a cell is rejected', () => {
  const rejected = (display) => claim('citing_bibliography', display, 'excluded', { review_state: 'rejected', excluded_reason: 'rejected' });
  const cells = cellValues(field([rejected('Gone'), rejected('Gone')]), 'citing_bibliography');
  assert.equal(cells[0].mark, 'excluded');
});

test('should hide retracted overrides from the user column', () => {
  const retracted = claim('user', 'Old', 'excluded', { retracted_at: '2026-02-01T00:00:00Z', excluded_reason: 'retracted' });
  const live = claim('user', 'New', 'selected');
  const subject = field([retracted, live]);
  assert.deepEqual(cellValues(subject, 'user').map((cell) => cell.display), ['New']);
  assert.equal(activeOverride(subject)?.display, 'New');
  assert.equal(activeOverride(field([retracted])), null);
});

test('should group citing claims by value with counts', () => {
  const cells = cellValues(field([
    claim('citing_bibliography', 'B', 'disagrees'),
    claim('citing_bibliography', 'A', 'agrees'),
    claim('citing_bibliography', 'B', 'disagrees'),
    claim('citing_bibliography', 'B', 'disagrees'),
    claim('citing_bibliography', 'C', 'excluded'),
  ]), 'citing_bibliography');
  assert.deepEqual(cells.map((cell) => [cell.display, cell.mark, cell.count]), [
    ['A', 'agrees', 1], ['B', 'disagrees', 3], ['C', 'excluded', 1],
  ]);
});

test('should order columns user publisher paper openalex library citing', () => {
  assert.deepEqual([...PROVENANCE_COLUMN_ORDER], [
    'user', 'publisher', 'paper', 'openalex', 'library_metadata', 'citing_bibliography',
  ]);
});

test('should parse authors one per line or semicolon separated when overriding', () => {
  assert.deepEqual(parseOverrideInput('authors', 'A. One; B. Two\n C. Three ;; \n'), { ok: true, value: ['A. One', 'B. Two', 'C. Three'] });
  assert.equal(parseOverrideInput('authors', ' ; \n').ok, false);
  assert.equal(parseOverrideInput('authors', Array.from({ length: 101 }, (_, i) => `N${i}`).join(';')).ok, false);
  assert.equal(parseOverrideInput('authors', Array.from({ length: 100 }, (_, i) => `N${i}`).join(';')).ok, true);
  assert.equal(overrideDraft('authors', 'A; B'), 'A\nB');
});

test('should reject year outside 1400 to 2200 when overriding', () => {
  assert.equal(parseOverrideInput('year', '1399').ok, false);
  assert.equal(parseOverrideInput('year', '2201').ok, false);
  assert.equal(parseOverrideInput('year', '20x0').ok, false);
  assert.equal(parseOverrideInput('year', '').ok, false);
  assert.deepEqual(parseOverrideInput('year', ' 1400 '), { ok: true, value: 1400 });
  assert.deepEqual(parseOverrideInput('year', '2200'), { ok: true, value: 2200 });
});

test('should strip doi resolver prefix when overriding', () => {
  assert.deepEqual(parseOverrideInput('doi', 'https://doi.org/10.1/abc'), { ok: true, value: '10.1/abc' });
  assert.deepEqual(parseOverrideInput('doi', 'doi: 10.1/abc'), { ok: true, value: '10.1/abc' });
  assert.deepEqual(parseOverrideInput('doi', ' 10.1/abc '), { ok: true, value: '10.1/abc' });
  assert.equal(parseOverrideInput('doi', 'https://example.com/10.1/abc').ok, false);
});

test('should reject empty or overlong title and venue when overriding', () => {
  assert.equal(parseOverrideInput('title', '   ').ok, false);
  assert.equal(parseOverrideInput('venue', 'x'.repeat(501)).ok, false);
  assert.deepEqual(parseOverrideInput('title', ' T '), { ok: true, value: 'T' });
});

test('should truncate author list with remaining count when formatting', () => {
  assert.equal(formatAuthors(['A', 'B', 'C', 'D', 'E', 'F', 'G'], 3), 'A, B, C +4');
  assert.equal(formatAuthors(['A', 'B'], 3), 'A, B');
  assert.equal(formatAuthors(['A', 'B', 'C', 'D', 'E', 'F', 'G']), 'A, B, C, D, E, F +1');
  assert.equal(formatAuthors([]), '');
});
