import assert from 'node:assert/strict';
import test from 'node:test';
import { objectsApi, objectsMatchGeneration, sourcePaperId } from '../src/lib/objects.ts';
import { ApiError } from '../src/lib/api.ts';

test('should request the encoded paper route when loading objects', async t => {
  const fixture = {
    schema_version: 2, paper_id: 'paper/id', reading_index_generation: '"generation-1"', unresolved_citations: [],
    objects: [{ kind: 'figure', id: 'fig-3', label: 'Figure 3', page: 1, text: 'A caption.',
      anchor: { page: 1, start: 3, end: 13 }, member_anchors: [], region: null,
      confidence: 'candidate', mentions: [] }],
  };
  const controller = new AbortController();
  t.mock.method(globalThis, 'fetch', async (path, init) => {
    assert.equal(path, '/api/papers/paper%2Fid/objects');
    assert.equal(init.signal, controller.signal);
    assert.equal(init.cache, 'no-cache');
    assert.equal(init.headers.get('Accept'), 'application/json');
    return new Response(JSON.stringify(fixture), { headers: { 'Content-Type': 'application/json' } });
  });
  assert.deepEqual(await objectsApi.get('paper/id', controller.signal), fixture);
});

test('should reject stale malformed or cross-paper artifacts when checking source provenance', () => {
  const artifact = { schema_version:2, paper_id:'1111222233334444', reading_index_generation:'"current"', objects:[], unresolved_citations:[] };
  assert.equal(objectsMatchGeneration(artifact,'"current"',artifact.paper_id),true);
  for (const broken of [null, undefined, [], {}, {...artifact,schema_version:1}, {...artifact,objects:[null]}, {...artifact,objects:[{kind:'bib_entry',anchor:{}}]}]) {
    assert.equal(objectsMatchGeneration(broken,'"current"',artifact.paper_id),false);
  }
  assert.equal(objectsMatchGeneration(artifact,null),false);
  assert.equal(objectsMatchGeneration(artifact,'"other"'),false);
  assert.equal(objectsMatchGeneration(artifact,'"current"','9999000011112222'),false);
  assert.equal(sourcePaperId('/api/papers/1111222233334444/source'),artifact.paper_id);
  assert.equal(sourcePaperId('https://other.example/api/papers/1111222233334444/source'),null);
  assert.equal(sourcePaperId('/api/papers/../../source'),null);
});

test('should preserve API failures when an object request fails', async t => {
  t.mock.method(globalThis, 'fetch', async () => new Response(JSON.stringify({ message: 'Paper was not found.' }), {
    status: 404, headers: { 'Content-Type': 'application/json' },
  }));
  await assert.rejects(objectsApi.get('unknown'), error => error instanceof ApiError && error.status === 404);
});
