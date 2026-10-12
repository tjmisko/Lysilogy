import assert from 'node:assert/strict';
import test from 'node:test';
import { HOME_PAGE_SIZE, pageOfIndex, pageSummary, pageWindow } from '../src/lib/homePaging.ts';

test('should default to 250 papers per page when no size is given', () => {
  assert.equal(HOME_PAGE_SIZE, 250);
  assert.deepEqual(pageWindow(10951, 0), { page: 0, pageCount: 44, start: 0, end: 250 });
});

test('should clamp the page into range when it is negative or past the end', () => {
  assert.deepEqual(pageWindow(10951, -3), { page: 0, pageCount: 44, start: 0, end: 250 });
  assert.deepEqual(pageWindow(10951, 99), { page: 43, pageCount: 44, start: 10750, end: 10951 });
  assert.deepEqual(pageWindow(10951, 2.9), { page: 2, pageCount: 44, start: 500, end: 750 });
});

test('should keep one empty page when the filtered list is empty', () => {
  assert.deepEqual(pageWindow(0, 5), { page: 0, pageCount: 1, start: 0, end: 0 });
  assert.equal(pageSummary(pageWindow(0, 0), 0), '0 papers');
});

test('should cover every item exactly once when pages are walked in order', () => {
  const total = 1001, size = 250;
  const seen = [];
  for (let page = 0; page < pageWindow(total, 0, size).pageCount; page++) {
    const window = pageWindow(total, page, size);
    for (let at = window.start; at < window.end; at++) seen.push(at);
  }
  assert.deepEqual(seen, Array.from({ length: total }, (_, at) => at));
});

test('should report the page holding a position when an item is restored', () => {
  assert.equal(pageOfIndex(0), 0);
  assert.equal(pageOfIndex(249), 0);
  assert.equal(pageOfIndex(250), 1);
  assert.equal(pageOfIndex(10950), 43);
  assert.equal(pageOfIndex(-1), null);
  assert.equal(pageOfIndex(7, 3), 2);
});

test('should describe the visible slice in one-based terms when summarizing', () => {
  assert.equal(pageSummary(pageWindow(10951, 0), 10951), '1–250 of 10951');
  assert.equal(pageSummary(pageWindow(10951, 43), 10951), '10751–10951 of 10951');
  assert.equal(pageSummary(pageWindow(12, 0), 12), '1–12 of 12');
});
