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

test('reads today\'s context fields and the richer shapes the backend may add', () => {
  const legacy = readContext({
    outsider_brief: 'Before this paper, local labor markets were assumed to adjust quickly.',
    context_notes: [{ kind: 'after', text: 'Later work found persistent effects.', source_ids: ['s1'] }],
    context_sources: [{ id: 's1', title: 'The China Shock', url: 'https://example.org', supports: 'persistence' }],
  });
  assert.equal(legacy.paragraphs.length, 2);
  assert.equal(legacy.paragraphs[1].label, 'After');
  assert.equal(legacy.sources[0].supports, 'persistence');
  assert.equal(legacy.readAlongside.length, 0);

  const rich = readContext({ context: {
    why_it_matters: 'It moved trade economics from national to local adjustment.',
    read_alongside: [{ title: 'Topalova 2010', authors: ['Petia Topalova'], year: 2010, why: 'Design predecessor', doi: '10.1257/app.2.4.1' }, 'Pierce and Schott 2016'],
  } });
  assert.equal(rich.paragraphs[0].label, 'Why it matters');
  assert.deepEqual(rich.readAlongside[0], { title: 'Topalova 2010', byline: 'Petia Topalova, 2010', why: 'Design predecessor', link: 'https://doi.org/10.1257/app.2.4.1' });
  assert.equal(rich.readAlongside[1].title, 'Pierce and Schott 2016');
  assert.equal(isProduced('context', {}), false);
  assert.equal(isProduced('context', { outsider_brief: '  ' }), false);
});

test('reads nested and flat outlines with levels and pages', () => {
  const nested = readOutline({ outline: [{ number: 'I', title: 'Introduction', page: 1, children: [{ title: 'Data', pages: { start: 3, end: 4 } }] }] });
  assert.deepEqual(nested, [
    { title: 'I Introduction', level: 1, pages: 'p. 1' },
    { title: 'Data', level: 2, pages: 'pp. 3–4' },
  ]);
  assert.deepEqual(readOutline({ outline: { entries: ['Abstract'] } }), [{ title: 'Abstract', level: 1, pages: null }]);
  assert.deepEqual(readOutline({ sections: [{ title: 'Not an outline' }] }), []);
});

test('prefers a logical segmentation and falls back to sections', () => {
  const logical = readLogical({
    logical_sections: [{ title: 'Why shocks are local', summary: 'Motivation.', covers: ['I', { title: 'II. Theory' }], pages: { start: 1, end: 8 } }],
    sections: [{ title: 'Ignored' }],
  });
  assert.equal(logical.source, 'logical_sections');
  assert.deepEqual(logical.units[0], { title: 'Why shocks are local', summary: 'Motivation.', pages: 'pp. 1–8', covers: ['I', 'II. Theory'] });
  const fallback = readLogical({ sections: [{ title: 'Front matter', digest: 'Everything.' }] });
  assert.equal(fallback.source, 'sections');
  assert.equal(fallback.units[0].summary, 'Everything.');
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
