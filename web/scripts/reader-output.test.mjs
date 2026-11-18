// Exercises the reader helpers against the shared fixtures in
// tests/fixtures/reader/, which the Rust tests in src/domain/fixture_tests.rs
// also load: a field renamed on either side breaks one of the two.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  CONTEXT_PARTS, basisCounts, basisLabel, downgradeText, groupedHeadings, groupedRangeLabel, hasOutline,
  outlineLabel, outlineRows, paperContext, pipelineLabel, readerWarnings, relationLabel, roleLabel,
} from '../src/lib/readerOutput.ts';

const fixture = (name) => JSON.parse(readFileSync(new URL(`../../tests/fixtures/reader/${name}`, import.meta.url), 'utf8'));
const current = fixture('analysis-current.json');
const legacy = fixture('analysis-legacy.json');

test('labels every source class and falls back to background', () => {
  assert.equal(basisLabel('paper').short, 'From the paper');
  assert.equal(basisLabel('dossier').short, 'Citation graph');
  assert.match(basisLabel('background').short, /unverified/u);
  assert.equal(basisLabel(undefined).short, basisLabel('background').short);
  assert.equal(basisLabel('something-new').short, basisLabel('background').short);
});

test('reads the context node and counts claims by source class', () => {
  const context = paperContext(current);
  assert.notEqual(context, null);
  for (const { part } of CONTEXT_PARTS) assert.ok(context[part].length > 0, `${part} is exercised by the fixture`);
  assert.deepEqual(basisCounts(context), { paper: 2, dossier: 2, background: 3 });
  assert.equal(downgradeText(context.before[1]), 'The work it cited is not in the citation graph.');
  assert.equal(downgradeText(context.contribution[1]), 'Its quote could not be found in the paper.');
  assert.equal(downgradeText(context.lead[0]), null);
  assert.equal(relationLabel(context.read_alongside[1].relation), 'Critique');
  assert.equal(relationLabel('unheard-of'), 'Related');
});

test('treats a missing or empty context as absent', () => {
  assert.equal(paperContext(legacy), null);
  assert.equal(paperContext({ context: null }), null);
  assert.equal(paperContext({ context: { lead: [], before: [], contribution: [], after: [], read_alongside: [], reception_note: null, counts: {} } }), null);
});

test('builds outline rows with relative depth and owning unit', () => {
  const rows = outlineRows(current.outline, current.sections);
  assert.equal(rows.length, current.outline.length);
  assert.equal(rows[3].depth, 1);
  assert.equal(rows[3].unit, 0);
  assert.equal(rows[12].unit, 4);
  assert.equal(outlineLabel(rows[3].entry), '2.1 The harbour reopening');
  assert.equal(outlineLabel(rows[0].entry), 'Abstract');
  assert.equal(outlineLabel({ number: '2', title: '2 Data' }), '2 Data');
  assert.equal(roleLabel(rows[11].entry.role), 'References');
  assert.equal(roleLabel(undefined), 'Main text');
  // Deeper-only outlines start at depth 0.
  assert.deepEqual(outlineRows([{ title: 'A', level: 2, page: 1 }, { title: 'B', level: 3, page: 1 }], []).map((row) => row.depth), [0, 1]);
});

test('labels the headings a logical unit groups', () => {
  const [first, , , fourth, last] = current.sections;
  assert.equal(groupedRangeLabel(first, current.outline), '§ 1–2.1');
  assert.equal(groupedRangeLabel(fourth, current.outline), '§ 6 Conclusion');
  assert.equal(groupedRangeLabel(last, current.outline), '§ References–A');
  assert.equal(groupedHeadings({ outline_entries: [99, 1] }, current.outline).length, 1, 'out-of-range indices are skipped');
  assert.equal(groupedRangeLabel(legacy.sections[0], []), null);
  assert.equal(hasOutline(current), true);
  assert.equal(hasOutline(legacy), false);
});

test('shows pipeline warnings and a short pipeline label', () => {
  assert.equal(readerWarnings(current).length, 2);
  assert.equal(readerWarnings({ quality_warnings: [current.quality_warnings[0], current.quality_warnings[0]] }).length, 1);
  assert.deepEqual(readerWarnings(legacy), []);
  assert.equal(pipelineLabel(current.pipeline_version), 'example/core-model+example/judge-model@deepinfra');
  assert.equal(pipelineLabel(undefined), null);
});
