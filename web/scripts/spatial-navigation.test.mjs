import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spatialNeighbor } from '../src/lib/spatialNavigation.ts';

const box = (section, left, right, top, bottom) => ({section,left,right,top,bottom});
// Unequal regions like the Debate overview: the ML experiment sits below the
// debate game, though it is several positions later in document order.
const grid = [box(0,0,425,0,300), box(1,425,603,0,300), box(2,603,915,0,300),
  box(3,915,1240,0,300), box(4,0,135,300,600), box(5,135,319,300,600),
  box(6,319,850,300,600), box(7,850,1240,300,600)];

test('arrows follow uneven screen regions rather than document indices', () => {
  assert.equal(spatialNeighbor(grid,1,'down'),6);
  assert.equal(spatialNeighbor(grid,6,'up'),1);
  assert.equal(spatialNeighbor(grid,1,'left'),0);
  assert.equal(spatialNeighbor(grid,1,'right'),2);
  assert.equal(spatialNeighbor(grid,0,'up'),null);
  assert.equal(spatialNeighbor(grid,7,'down'),null);
});

test('navigation uses current geometry after resizing and scrolling', () => {
  const resized = grid.map(b=>({...b,left:b.left/2+20,right:b.right/2+20,top:b.top/2-80,bottom:b.bottom/2-80}));
  assert.equal(spatialNeighbor(resized,1,'down'),6);
  const narrow = grid.map((b,i)=>box(b.section,(i%2)*200,(i%2+1)*200,Math.floor(i/2)*300,(Math.floor(i/2)+1)*300));
  assert.equal(spatialNeighbor(narrow,1,'down'),3);
});

test('a continued section is not treated as a different destination', () => {
  const fragments = [box(0,0,200,0,300),box(1,200,400,0,300),box(1,0,200,300,600),box(2,200,400,300,600)];
  assert.equal(spatialNeighbor(fragments,1,'down'),3);
  assert.equal(spatialNeighbor(fragments,2,'up'),0);
  assert.equal(spatialNeighbor(fragments,-1,'up'),null);
});
