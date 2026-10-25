import assert from 'node:assert/strict';
import test from 'node:test';
import {
  listStorageKey, missingAncestors, pathBreadcrumb, pickCurrentList, walkListTitle,
} from '../src/lib/listSidebar.ts';
import { nextReadingStatus, stepAmongRows } from '../src/lib/listKeys.ts';

const node = (id) => ({ work_id: id });
const step = (id, title = `Title ${id}`) => ({ work_id: id, title });
const list = (id, updated, seed = 'c') => ({ id, updated_at: updated, seed_work_id: seed });

test('should cycle unread reading read when advancing status', () => {
  assert.equal(nextReadingStatus('unread'), 'reading');
  assert.equal(nextReadingStatus('reading'), 'read');
  assert.equal(nextReadingStatus('read'), 'unread');
  assert.equal(nextReadingStatus('surprise'), 'reading');
});

test('should list ancestors not yet in list in path order', () => {
  assert.deepEqual(missingAncestors([node('s')], ['s', 'a', 'b', 'c']), ['a', 'b']);
  assert.deepEqual(missingAncestors([node('s'), node('b')], ['s', 'a', 'b', 'c']), ['a']);
  assert.deepEqual(missingAncestors([], ['s', 'a']), ['s']);
  assert.deepEqual(missingAncestors([node('s')], ['s', 'a', 'a', 'c']), ['a']);
});

test('should return nothing missing when whole path is in list', () => {
  assert.deepEqual(missingAncestors([node('s'), node('a'), node('b')], ['s', 'a', 'b']), []);
  assert.deepEqual(missingAncestors([node('s')], ['s']), []);
  assert.deepEqual(missingAncestors([], []), []);
});

test('should not count topic or claim nodes as held when finding ancestors', () => {
  assert.deepEqual(missingAncestors([node(null), node('s')], ['s', 'a', 'x']), ['a']);
  assert.deepEqual(missingAncestors([node(null)], ['s', 'x']), ['s']);
});

test('should move a row among visible rows and leave topics in place when stepping', () => {
  const order = ['t', 'a', 'b', 'c'];
  assert.deepEqual(stepAmongRows(order, ['a', 'b', 'c'], 'a', 1), ['t', 'b', 'a', 'c']);
  assert.deepEqual(stepAmongRows(order, ['a', 'b', 'c'], 'a', -1), ['t', 'a', 'b', 'c']);
  assert.deepEqual(stepAmongRows(order, ['a', 'b', 'c'], 'c', 1), ['t', 'a', 'b', 'c']);
  assert.deepEqual(stepAmongRows(order, ['a', 'b', 'c'], 'zz', 1), ['t', 'a', 'b', 'c']);
});

test('should middle truncate long breadcrumbs', () => {
  const path = [step('s'), step('a'), step('b'), step('c'), step('d'), step('self')];
  assert.deepEqual(pathBreadcrumb(path, 3), ['Title s', '…', 'Title d']);
  assert.deepEqual(pathBreadcrumb(path, 4), ['Title s', 'Title a', '…', 'Title d']);
  assert.deepEqual(pathBreadcrumb(path, 1), ['…']);
  assert.deepEqual(pathBreadcrumb(path, 0), ['…']);
});

test('should keep short breadcrumbs whole and name untitled works', () => {
  assert.deepEqual(pathBreadcrumb([step('s'), step('a'), step('self')], 3), ['Title s', 'Title a']);
  assert.deepEqual(pathBreadcrumb([step('s', null), step('a', '  '), step('self')], 3), ['Untitled', 'Untitled']);
  assert.deepEqual(pathBreadcrumb([step('s')], 3), []);
  assert.deepEqual(pathBreadcrumb([], 3), []);
});

test('should prefer remembered list then most recent seeded at the center when picking', () => {
  const lists = [list('p1', '2026-10-01T00:00:00Z'), list('p2', '2026-10-03T00:00:00Z'), list('p3', '2026-10-02T00:00:00Z')];
  assert.equal(pickCurrentList(lists, 'p3', 'c').id, 'p3');
  assert.equal(pickCurrentList(lists, null, 'c').id, 'p2');
  assert.equal(pickCurrentList([], 'p1', 'c'), null);
});

test('should ignore remembered list that no longer exists', () => {
  const lists = [list('p1', '2026-10-01T00:00:00Z'), list('p2', '2026-10-03T00:00:00Z')];
  assert.equal(pickCurrentList(lists, 'gone', 'c').id, 'p2');
  assert.equal(pickCurrentList(lists, '', 'c').id, 'p2');
});

test('should not pick a list seeded elsewhere or unseeded when nothing is remembered', () => {
  const lists = [list('p1', '2026-10-05T00:00:00Z', 'other'), list('p2', '2026-10-04T00:00:00Z', null)];
  assert.equal(pickCurrentList(lists, null, 'c'), null);
  assert.equal(pickCurrentList(lists, 'p1', 'c').id, 'p1');
});

test('should namespace the storage key by center work and keep the ramp prefix', () => {
  assert.equal(listStorageKey('wabc'), 'lysilogy.ramp.wabc');
  assert.notEqual(listStorageKey('wa'), listStorageKey('wb'));
});

test('should build a bounded list title from the paper title', () => {
  assert.equal(walkListTitle('Attention  Is\nAll'), 'Reading list: Attention Is All');
  assert.equal(walkListTitle(null), 'Reading list');
  assert.equal(walkListTitle('   '), 'Reading list');
  const long = walkListTitle('x'.repeat(300));
  assert.equal(Array.from(long).length, 100);
  assert.ok(long.endsWith('…'));
  const wide = walkListTitle('論'.repeat(300));
  assert.equal(Array.from(wide).length, 100);
});
