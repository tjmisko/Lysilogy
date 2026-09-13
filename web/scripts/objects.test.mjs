import assert from 'node:assert/strict';
import test from 'node:test';
import { objectsApi } from '../src/lib/objects.ts';
import { ApiError } from '../src/lib/api.ts';

test('should request the encoded paper route when loading objects', async t => {
  const fixture = {
    schema_version: 1, paper_id: 'paper/id', reading_index_generation: '"generation-1"',
    objects: [{ kind: 'figure', id: 'fig-3', label: 'Figure 3', page: 1, text: 'A caption.',
      anchor: { page: 1, start: 3, end: 13 }, member_anchors: [], region: null,
      confidence: 'candidate', mentions: [] }],
  };
  const controller = new AbortController();
  t.mock.method(globalThis, 'fetch', async (path, init) => {
    assert.equal(path, '/api/papers/paper%2Fid/objects');
    assert.equal(init.signal, controller.signal);
    assert.equal(init.headers.get('Accept'), 'application/json');
    return new Response(JSON.stringify(fixture), { headers: { 'Content-Type': 'application/json' } });
  });
  assert.deepEqual(await objectsApi.get('paper/id', controller.signal), fixture);
});

test('should preserve API failures when an object request fails', async t => {
  t.mock.method(globalThis, 'fetch', async () => new Response(JSON.stringify({ message: 'Paper was not found.' }), {
    status: 404, headers: { 'Content-Type': 'application/json' },
  }));
  await assert.rejects(objectsApi.get('unknown'), error => error instanceof ApiError && error.status === 404);
});
