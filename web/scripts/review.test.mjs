import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  applyGradingKey, completion, draftFromReview, emptyDraft, focusRows, isProduced, nextIssueId,
  progressCounts, progressState, readContext, readLogical, readOutline, setValue, visibleDimensions,
} from '../src/lib/review.ts';

const rubric = JSON.parse(readFileSync(new URL('../../eval/review/rubric.json', import.meta.url), 'utf8'));
const component = (id) => rubric.components.find((entry) => entry.id === id);
const row = (componentId, dimensionId) => ({ componentId, dimension: component(componentId).dimensions.find((d) => d.id === dimensionId) });

const fixture = (name) => JSON.parse(readFileSync(new URL(`../../tests/fixtures/reader/${name}`, import.meta.url), 'utf8'));
const current = fixture('analysis-current.json');
const legacy = fixture('analysis-legacy.json');

test('reads the context node with its source classes from a snapshot', () => {
  const context = readContext(current);
  assert.equal(context.node.lead.length, 2);
  assert.deepEqual(context.node.lead.map((claim) => claim.basis), ['background', 'paper']);
  assert.equal(context.node.read_alongside[1].work_id, 'W200');
  assert.equal(context.legacyNotes.length, 1, 'notes the node wrote are not repeated as legacy notes');
  assert.equal(isProduced('context', current), true);

  const old = readContext(legacy);
  assert.equal(old.node, null);
  assert.equal(old.brief, legacy.outsider_brief);
  assert.equal(old.legacyNotes.length, 1);
  assert.equal(isProduced('context', {}), false);
  assert.equal(isProduced('context', { outsider_brief: '  ', context: { lead: [], before: [], contribution: [], after: [], read_alongside: [], reception_note: null, counts: {} } }), false);
});

test('reads the printed outline with depth and owning unit', () => {
  const rows = readOutline(current);
  assert.equal(rows.length, 13);
  assert.deepEqual(rows.slice(1, 4).map((row) => [row.entry.number, row.depth, row.unit]), [['1', 0, 0], ['2', 0, 0], ['2.1', 1, 0]]);
  assert.equal(rows[0].unit, null, 'the front-matter abstract heading belongs to no unit');
  assert.deepEqual(readOutline(legacy), []);
  assert.equal(isProduced('outline', legacy), false);
});

test('reads logical units with the headings they group, and page-span units without', () => {
  const logical = readLogical(current);
  assert.equal(logical.grouped, true);
  assert.equal(logical.units[0].range, '§ 1–2.1');
  assert.deepEqual(logical.units[0].covers, ['1 Introduction', '2 Background', '2.1 The harbour reopening']);
  assert.equal(logical.units[3].range, '§ 6 Conclusion');
  const old = readLogical(legacy);
  assert.equal(old.grouped, false);
  assert.equal(old.units[0].range, null);
  assert.equal(readLogical({}).units.length, 0);
});

test('maps keys to grades by dimension kind', () => {
  const scale = rubric.scale;
  let draft = emptyDraft();
  draft = applyGradingKey(draft, row('context', 'accuracy'), '3', scale);
  assert.equal(draft.components.context.values.accuracy, 3);
  assert.equal(applyGradingKey(draft, row('context', 'accuracy'), '7', scale), null);
  draft = applyGradingKey(draft, row('abstract', 'exact'), 'n', scale);
  assert.equal(draft.components.abstract.values.exact, false);
  draft = applyGradingKey(draft, row('abstract', 'exact'), ' ', scale);
  assert.equal(draft.components.abstract.values.exact, true);
  draft = applyGradingKey(draft, row('logical', 'granularity'), '3', scale);
  assert.equal(draft.components.logical.values.granularity, 'too_few');
  const count = row('outline', 'missing_headings');
  draft = applyGradingKey(draft, count, '1', scale);
  draft = applyGradingKey(draft, count, '2', scale);
  assert.equal(draft.components.outline.values.missing_headings, 12);
  draft = applyGradingKey(draft, count, '-', scale);
  assert.equal(draft.components.outline.values.missing_headings, 11);
  draft = applyGradingKey(draft, count, 'Backspace', scale);
  assert.equal(draft.components.outline.values.missing_headings, undefined);
  const overall = { componentId: 'overall', dimension: rubric.overall };
  assert.equal(applyGradingKey(draft, overall, '4', scale).overall, 4);
});

test('shows the abstract error type only after a fail and counts completion', () => {
  const abstract = component('abstract');
  let draft = emptyDraft();
  assert.deepEqual(visibleDimensions(abstract, { not_produced: false, hard_reject: false, values: {}, notes: '' }).map((d) => d.id), ['exact']);
  draft = setValue(draft, 'abstract', 'exact', false);
  assert.deepEqual(visibleDimensions(abstract, draft.components.abstract).map((d) => d.id), ['exact', 'error_type']);
  const rows = focusRows(rubric, draft);
  assert.equal(rows.at(-1).componentId, 'overall');
  const before = completion(rubric, draft);
  assert.equal(before.graded, 1);
  assert.equal(before.components.abstract, false);
  draft = setValue(draft, 'abstract', 'error_type', 'truncated');
  assert.equal(completion(rubric, draft).components.abstract, true);
  assert.equal(completion(rubric, draft).total, before.total);
});

test('restores drafts, numbers issues and summarises progress', () => {
  const draft = draftFromReview({ components: { thesis: { values: { faithful_specific: 2 } } }, overall: 3, notes: 'ok', issues: [{ id: 'issue-2' }] });
  assert.equal(draft.components.thesis.not_produced, false);
  assert.equal(draft.components.thesis.values.faithful_specific, 2);
  assert.equal(nextIssueId(draft.issues), 'issue-3');
  assert.equal(nextIssueId([{ id: 'issue-1' }, { id: 'issue-3' }]), 'issue-4');
  const rows = [
    { manifest_id: 'a', paper_id: null, analysed: false, reviewed: false, reviewed_earlier: false },
    { manifest_id: 'b', paper_id: 'p1', analysed: true, reviewed: true, reviewed_earlier: false },
    { manifest_id: 'c', paper_id: 'p2', analysed: true, reviewed: false, reviewed_earlier: true },
  ];
  assert.deepEqual(progressCounts(rows), { total: 3, inLibrary: 2, analysed: 2, reviewed: 1 });
  assert.deepEqual(rows.map(progressState), ['missing', 'reviewed', 'stale']);
});

test('reads the pinned citation labels and keeps a saved review on the labels it graded', async () => {
  const { readEdgeLabels, shownFields } = await import('../src/lib/review.ts');
  const label = (claim_id, relation, engagement) => ({
    claim_id, relation, reason: `Reason ${claim_id}.`, quote: 'their index of heat stress', path: 'edge_labeler',
    engagement, review_state: 'unreviewed', model: null, prompt_version: 2, passage: 'We use their index of heat stress [3].', page: 4,
  });
  const pinned = (reason) => ({
    generated_at: '2026-11-18T00:00:00Z', status: 'completed', detail: null, labeler: 'openrouter:m', prompt_version: 2,
    label_version: 'openrouter:m @ prompt v2',
    counts: { edges: 5, labelled: 3, unlabelled_with_passages: 1, perfunctory: 1, not_shown: 0 },
    unresolved_keys: 2, left_for_later: 0, rejected_for_grounding: 1,
    sample: [
      { work_id: 'w1', title: 'Reef survey', year: 2001, first_author: null, mention_count: 2, label: { ...label('r1', 'uses_method', 'substantive'), reason } },
      { work_id: 'w2', title: 'No label', year: null, first_author: null, mention_count: 1, label: null },
    ],
  });
  const output = readEdgeLabels({ edge_labels: pinned('Uses it.') });
  assert.equal(output.sample.length, 1);
  assert.equal(output.countsLine, '3 of 5 edges labelled · 1 with passages but no label · 1 perfunctory · 2 core reference keys unresolved · 1 answers failed grounding');
  assert.equal(readEdgeLabels({}).pinned, null);
  assert.equal(isProduced('edge_labels', { edge_labels: pinned('x') }), true);
  assert.equal(isProduced('edge_labels', { ...current }), false, 'an analysis alone has no labels');

  const snapshot = { snapshot_id: 's-1', generated_at: null, pipeline_version: null, provider: null, fields: { thesis: 'T.', edge_labels: pinned('Current run.') } };
  const review = { snapshot: { ...snapshot, fields: { thesis: 'T.', edge_labels: pinned('Graded run.') } } };
  assert.equal(shownFields(snapshot, review).edge_labels.sample[0].label.reason, 'Graded run.');
  assert.equal(shownFields(snapshot, null).edge_labels.sample[0].label.reason, 'Current run.');
  assert.equal(shownFields(null, null), null);
  const edgeComponent = component('edge_labels');
  assert.deepEqual(edgeComponent.reads, ['edge_labels']);
  assert.ok(edgeComponent.dimensions.some((dimension) => dimension.id === 'wrong_relation' && dimension.kind === 'count'));
});
