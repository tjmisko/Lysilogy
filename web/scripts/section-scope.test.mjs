import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sectionPages } from '../src/lib/sectionScope.ts';
import { sectionPageCrop } from '../src/lib/sectionCrop.ts';

test('scopes whole boundary pages from verified anchors', () => {
  const section = {pages:{start:1,end:8},source_span:{start:{page:2,start_token:5},end:{page:3,end_token:9}}};
  assert.deepEqual(sectionPages(section,8),[2,3]);
  section.source_span.end={page:2,end_token:7};
  assert.deepEqual(sectionPages(section,8),[2]);
});

const token = (index, x, y, text = `word${index}`) => ({ index, text, line:index,
  rects:[{x_min:x,y_min:y,x_max:x+180,y_max:y+12}] });
const page = (tokens) => ({number:2,width:600,height:800,tokens});
const span = (start,end) => ({pages:{start:2,end:2},source_span:{start:{page:2,start_token:start},end:{page:2,end_token:end}}});
const inside = (crop,x,y) => crop.regions.some(r => x >= r.x_min && x <= r.x_max && y >= r.y_min && y <= r.y_max);

test('crops both boundaries and retains space for figures within the section', () => {
  const crop = sectionPageCrop(span(1,3),page([token(0,40,50),token(1,40,100),token(2,40,130),token(3,40,400),token(4,40,440)]),3);
  assert.equal(crop.bounds.y_min,97);
  assert.equal(crop.bounds.y_max,415);
  assert.ok(inside(crop,80,250),'space for a figure between source lines must survive');
  assert.ok(!inside(crop,80,55));
  assert.ok(!inside(crop,80,445));
});

test('does not expose the other column on either side of a section boundary', () => {
  const crop = sectionPageCrop(span(2,3),page([token(0,40,50),token(1,40,100),token(2,40,400),token(3,340,50),token(4,340,100)]),3);
  assert.ok(inside(crop,80,406));
  assert.ok(inside(crop,380,56));
  assert.ok(!inside(crop,80,56),'the preceding section in the left column must be absent');
  assert.ok(!inside(crop,380,106),'the following section in the right column must be absent');
});

test('a figure keeps the content width even when adjacent text lines are short', () => {
  const tokens=[token(0,40,100),token(1,90,130),token(2,90,400),token(3,40,440)];
  tokens[1].rects[0].x_max=170;tokens[2].rects[0].x_max=170;
  const crop=sectionPageCrop(span(0,3),page(tokens),3);
  assert.ok(inside(crop,50,250));
  assert.ok(inside(crop,210,250));
});

test('clips partial lines and omits isolated page numbers', () => {
  const tokens = [token(0,40,100),token(1,240,100),{...token(2,295,720,'2'),rects:[{x_min:295,y_min:720,x_max:305,y_max:732}]}];
  tokens[1].line=0;
  const crop=sectionPageCrop(span(1,2),page(tokens),3);
  assert.ok(inside(crop,260,106));
  assert.ok(!inside(crop,100,106));
  assert.ok(crop.bounds.y_max<720);
});

test('missing or invalid geometry cannot silently open a whole boundary page', () => {
  assert.equal(sectionPageCrop({pages:{start:2,end:2}},page([token(0,40,100)]),3),null);
  assert.equal(sectionPageCrop(span(0,7),page([token(0,40,100)]),3),null);
  assert.equal(sectionPageCrop(span(0,0),page([{...token(0,40,100),rects:[]}]),3),null);
  assert.equal(sectionPageCrop(span(0,0),page([token(0,NaN,100)]),3),null);
});
test('legacy and malformed anchors use only valid document pages', () => {
  assert.deepEqual(sectionPages({pages:{start:3,end:9}},4),[3,4]);
  assert.deepEqual(sectionPages({pages:{start:2,end:2},source_span:{start:{page:2,start_token:9},end:{page:2,end_token:2}}},4),[2]);
  assert.deepEqual(sectionPages({pages:{start:7,end:9}},4),[]);
  assert.deepEqual(sectionPages({pages:{start:3,end:1}},4),[]);
  assert.deepEqual(sectionPages({pages:{start:NaN,end:1}},4),[]);
  assert.deepEqual(sectionPages({pages:{start:1,end:1}},0),[]);
});
