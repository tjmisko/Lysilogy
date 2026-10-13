import assert from 'node:assert/strict';
import test from 'node:test';
import {
  missingAncestors, nextStatus, pathBreadcrumb, pickCurrentRamp, rampStorageKey, swapOrder,
} from '../src/lib/ramp.ts';

const item = (id) => ({ work: { id } });
const step = (id, title = `Title ${id}`) => ({ work_id: id, title });
const ramp = (id, updated) => ({ id, updated_at: updated });

test('should cycle unread reading read when advancing status', () => {
  assert.equal(nextStatus('unread'), 'reading');
  assert.equal(nextStatus('reading'), 'read');
  assert.equal(nextStatus('read'), 'unread');
  assert.equal(nextStatus('surprise'), 'reading');
});

test('should list ancestors not yet in ramp in path order', () => {
  assert.deepEqual(missingAncestors([item('s')], ['s', 'a', 'b', 'c']), ['a', 'b']);
  assert.deepEqual(missingAncestors([item('s'), item('b')], ['s', 'a', 'b', 'c']), ['a']);
  assert.deepEqual(missingAncestors([], ['s', 'a']), ['s']);
  assert.deepEqual(missingAncestors([item('s')], ['s', 'a', 'a', 'c']), ['a']);
});

test('should return nothing missing when whole path is in ramp', () => {
  assert.deepEqual(missingAncestors([item('s'), item('a'), item('b')], ['s', 'a', 'b']), []);
  assert.deepEqual(missingAncestors([item('s')], ['s']), []);
  assert.deepEqual(missingAncestors([], []), []);
});

test('should swap neighbors and clamp at ends when moving item', () => {
  assert.deepEqual(swapOrder(['a', 'b', 'c'], 0, 1), ['b', 'a', 'c']);
  assert.deepEqual(swapOrder(['a', 'b', 'c'], 2, -1), ['a', 'c', 'b']);
  assert.deepEqual(swapOrder(['a', 'b', 'c'], 0, -1), ['a', 'b', 'c']);
  assert.deepEqual(swapOrder(['a', 'b', 'c'], 2, 1), ['a', 'b', 'c']);
  assert.deepEqual(swapOrder(['a', 'b', 'c'], 5, -1), ['a', 'b', 'c']);
  assert.deepEqual(swapOrder(['a', 'b', 'c'], 1, Number.NaN), ['a', 'b', 'c']);
  assert.deepEqual(swapOrder([], 0, 1), []);
});

test('should not mutate the input when moving item', () => {
  const ids = ['a', 'b'];
  swapOrder(ids, 0, 1);
  assert.deepEqual(ids, ['a', 'b']);
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

test('should prefer remembered ramp then most recent when picking', () => {
  const ramps = [ramp('p1', '2026-10-01T00:00:00Z'), ramp('p2', '2026-10-03T00:00:00Z'), ramp('p3', '2026-10-02T00:00:00Z')];
  assert.equal(pickCurrentRamp(ramps, 'p3').id, 'p3');
  assert.equal(pickCurrentRamp(ramps, null).id, 'p2');
  assert.equal(pickCurrentRamp([], 'p1'), null);
});

test('should ignore remembered ramp that no longer exists', () => {
  const ramps = [ramp('p1', '2026-10-01T00:00:00Z'), ramp('p2', '2026-10-03T00:00:00Z')];
  assert.equal(pickCurrentRamp(ramps, 'gone').id, 'p2');
  assert.equal(pickCurrentRamp(ramps, '').id, 'p2');
});

test('should namespace the storage key by center work', () => {
  assert.equal(rampStorageKey('wabc'), 'lysilogy.ramp.wabc');
  assert.notEqual(rampStorageKey('wa'), rampStorageKey('wb'));
});
