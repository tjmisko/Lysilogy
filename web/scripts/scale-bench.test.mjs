import assert from 'node:assert/strict';
import test from 'node:test';
import { searchCases } from './scale-bench.mjs';

test('should generate deterministic independently counted queries when papers share authors and years', () => {
  const papers = [
    { id: 'a', metadata: { title: 'Causal study 00000', authors: ['Ada Researcher'], year: 2020 } },
    { id: 'b', metadata: { title: 'Graph study 00001', authors: ['Ada Researcher'], year: 2020 } },
    { id: 'c', metadata: { title: 'Vision study 00002', authors: ['Ren Yamada'], year: 2021 } },
  ];
  const cases = searchCases(papers, 3);
  assert.deepEqual(searchCases(papers, 3), cases);
  assert.deepEqual(cases, [
    { query: 'Causal study 00000', matches: 1, first: 'a', status: '1 of 3 papers' },
    { query: 'Ren Yamada', matches: 1, first: 'c', status: '1 of 3 papers' },
    { query: '2020', matches: 2, first: 'a', status: '2 of 3 papers' },
  ]);
});
