import { test } from 'node:test';
import assert from 'node:assert/strict';
import { inkBounds } from '../src/lib/pdfVisualBounds.ts';

const raster = () => ({ width: 10, height: 12, data: new Uint8ClampedArray(10 * 12 * 4).fill(255) });
const dot = (image, x, y, rgba) => image.data.set(rgba, (y * image.width + x) * 4);

test('blank paper and transparent dark pixels have no visual bounds', () => {
  const image = raster();
  assert.equal(inkBounds(image), null);
  dot(image, 0, 0, [0, 0, 0, 0]);
  dot(image, 9, 11, [250, 250, 250, 255]);
  assert.equal(inkBounds(image), null);
});

test('bounds include colored figures and disconnected text at asymmetric positions', () => {
  const image = raster();
  dot(image, 2, 3, [0, 0, 0, 255]);
  dot(image, 7, 9, [255, 0, 255, 255]);
  dot(image, 5, 5, [0, 0, 0, 100]);
  assert.deepEqual(inkBounds(image), { x_min: 2, y_min: 3, x_max: 8, y_max: 10 });
});

test('content at the paper edges retains the whole page', () => {
  const image = raster();
  dot(image, 0, 0, [0, 0, 0, 255]);
  dot(image, 9, 11, [0, 0, 0, 255]);
  assert.deepEqual(inkBounds(image), { x_min: 0, y_min: 0, x_max: 10, y_max: 12 });
});
