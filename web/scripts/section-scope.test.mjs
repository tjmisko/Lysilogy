import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sectionPages } from '../src/lib/sectionScope.ts';

test('scopes whole boundary pages from verified anchors', () => {
  const section = {pages:{start:1,end:8},source_span:{start:{page:2,start_token:5},end:{page:3,end_token:9}}};
  assert.deepEqual(sectionPages(section,8),[2,3]);
  section.source_span.end={page:2,end_token:7};
  assert.deepEqual(sectionPages(section,8),[2]);
});
test('legacy and malformed anchors use only valid document pages', () => {
  assert.deepEqual(sectionPages({pages:{start:3,end:9}},4),[3,4]);
  assert.deepEqual(sectionPages({pages:{start:2,end:2},source_span:{start:{page:2,start_token:9},end:{page:2,end_token:2}}},4),[2]);
  assert.deepEqual(sectionPages({pages:{start:7,end:9}},4),[]);
  assert.deepEqual(sectionPages({pages:{start:3,end:1}},4),[]);
  assert.deepEqual(sectionPages({pages:{start:NaN,end:1}},4),[]);
  assert.deepEqual(sectionPages({pages:{start:1,end:1}},0),[]);
});
