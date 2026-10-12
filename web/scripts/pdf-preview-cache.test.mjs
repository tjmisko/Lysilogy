import assert from 'node:assert/strict';
import test from 'node:test';
import { createPdfPreviewCache } from '../src/lib/pdfPreviewCache.ts';

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const tick = () => new Promise(resolve => setImmediate(resolve));

test('shared renders survive a consumer leaving and are cached for remounts', async () => {
  const gate = deferred(); let renders = 0;
  const cache = createPdfPreviewCache({ render: () => { renders++; return gate.promise; } });
  let disposed = false;
  const first = cache.load('paper', () => disposed ? Infinity : 0);
  disposed = true;
  const remounted = cache.load('paper');
  assert.equal(first, remounted);
  await tick();
  assert.equal(renders, 1);
  gate.resolve('thumbnail');
  assert.equal(await remounted, 'thumbnail');
  assert.equal(await cache.load('paper'), 'thumbnail');
  assert.equal(cache.peek('paper'), 'thumbnail');
  assert.equal(renders, 1);
});

test('bounded render queue reprioritizes the current viewport after a fast jump', async () => {
  const gates = new Map(); const started = [];
  const cache = createPdfPreviewCache({ concurrency: 2, render: url => {
    started.push(url); const gate = deferred(); gates.set(url, gate); return gate.promise;
  } });
  const first = cache.load('first'); const second = cache.load('second');
  await tick();
  let jumped = false;
  const passed = cache.load('passed', () => jumped ? 2000 : 0);
  const destination = cache.load('destination', () => jumped ? 0 : 2000);
  await tick();
  assert.deepEqual(started, ['first', 'second']);
  jumped = true;
  gates.get('first').resolve('first image');
  await tick();
  assert.deepEqual(started, ['first', 'second', 'destination']);
  gates.get('second').resolve('second image');
  await tick();
  gates.get('destination').resolve('destination image');
  gates.get('passed').resolve('passed image');
  await Promise.all([first, second, passed, destination]);
});

test('persistent hits bypass occupied render slots and survive a fresh memory cache', async () => {
  const saved = new Map([['cached', 'saved image']]); const gate = deferred(); let renders = 0;
  const options = { concurrency: 1, storage: {
    read: async url => saved.get(url), write: async (url, image) => { saved.set(url, image); },
  }, render: () => { renders++; return gate.promise; } };
  const cache = createPdfPreviewCache(options);
  const slow = cache.load('slow');
  await tick();
  assert.equal(await cache.load('cached'), 'saved image');
  assert.equal(renders, 1);
  gate.resolve('rendered image');
  await slow;
  const reloaded = createPdfPreviewCache(options);
  assert.equal(await reloaded.load('slow'), 'rendered image');
  assert.equal(renders, 1);
});

test('storage failures do not block rendering or poison the memory cache', async () => {
  let renders = 0;
  const cache = createPdfPreviewCache({ storage: {
    read: async () => { throw new Error('Storage disabled'); },
    write: async () => { throw new Error('Quota exceeded'); },
  }, render: async () => { renders++; return 'image'; } });
  assert.equal(await cache.load('paper'), 'image');
  assert.equal(await cache.load('paper'), 'image');
  assert.equal(renders, 1);
});

test('failed renders release queue slots and can be retried', async () => {
  let failed = false;
  const cache = createPdfPreviewCache({ concurrency: 1, render: async url => {
    if (url === 'broken' && !failed) { failed = true; throw new Error('Invalid PDF'); }
    return url;
  } });
  const broken = cache.load('broken'); const next = cache.load('next');
  await assert.rejects(broken, /Invalid PDF/);
  assert.equal(await next, 'next');
  assert.equal(await cache.load('broken'), 'broken');
});

test('memory retains recently read images within entry and byte limits', async () => {
  const cache = createPdfPreviewCache({ maxEntries: 2, render: async url => url });
  await cache.load('a'); await cache.load('b'); cache.peek('a'); await cache.load('c');
  assert.equal(cache.peek('b'), undefined);
  assert.equal(cache.peek('a'), 'a');
  const small = createPdfPreviewCache({ maxBytes: 8, render: async url => url });
  await small.load('aa'); await small.load('bb'); await small.load('cc');
  assert.equal(small.peek('aa'), undefined);
  assert.equal(small.peek('bb'), 'bb');
  assert.equal(await small.load('oversized'), 'oversized');
  assert.equal(small.peek('oversized'), undefined);
  assert.equal(small.peek('bb'), 'bb');
});
