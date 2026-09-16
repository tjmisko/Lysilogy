import assert from 'node:assert/strict';
import test from 'node:test';
import { gradesApi } from '../src/lib/objects.ts';
import { ApiError } from '../src/lib/api.ts';
import {
  addObject, applyVerdict, clearVerdict, clientToPdf, dragRect, emptyGrades, gradableObjects, gradingProgress,
  normalizeLabel, proposeCaption, removeAddition,
} from '../src/lib/objectGrading.ts';

const object = (id, kind, page, region, extra = {}) => ({
  kind, id, label: `${kind === 'figure' ? 'Figure' : 'Table'} ${id.split('-')[1]}`, page, text: '', anchor: { page, start: 0, end: 1 },
  member_anchors: [], region, confidence: 'candidate', mentions: [], ...extra,
});
const artifact = {
  schema_version: 1, paper_id: 'p1', reading_index_generation: '"abc123"', figure_detector_generation: 'det-9',
  objects: [
    object('tab-1', 'table', 3, { x_min: 60, y_min: 100, x_max: 540, y_max: 300 }),
    object('fig-2', 'figure', 2, null, { anchor: { page: 2, start: 40, end: 60 } }),
    object('fig-1', 'figure', 2, { x_min: 72, y_min: 80, x_max: 300, y_max: 260 }),
    object('eq-1', 'equation', 1, { x_min: 0, y_min: 0, x_max: 10, y_max: 10 }),
    object('fig-0', 'figure', 2, { x_min: 72, y_min: 20, x_max: 300, y_max: 60 }),
  ],
};
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

test('should keep only figures and tables in page then top order when listing gradable objects', () => {
  assert.deepEqual(gradableObjects(artifact).map(item => item.id), ['fig-0', 'fig-1', 'fig-2', 'tab-1']);
});

test('should strip the index generation quotes and copy the detector generation when creating empty grades', () => {
  const grades = emptyGrades(artifact);
  assert.equal(grades.index_sha256, 'abc123');
  assert.equal(grades.objects_generation, 'det-9');
  assert.equal(grades.revision, null);
  assert.equal(grades.complete, false);
  assert.deepEqual(grades.verdicts, {});
  assert.deepEqual(grades.additions, []);
  assert.equal(emptyGrades({ ...artifact, figure_detector_generation: undefined }).objects_generation, '');
});

test('should store the drawn region only for region verdicts when applying verdicts', () => {
  const rect = { x_min: 1, y_min: 2, x_max: 3, y_max: 4 };
  let grades = applyVerdict(emptyGrades(artifact), 'fig-1', 'region', rect);
  assert.deepEqual(grades.verdicts['fig-1'], { verdict: 'region', region: rect, note: '' });
  grades = applyVerdict(grades, 'fig-1', 'correct', rect);
  assert.deepEqual(grades.verdicts['fig-1'], { verdict: 'correct', region: null, note: '' });
  grades = { ...grades, verdicts: { ...grades.verdicts, 'fig-1': { ...grades.verdicts['fig-1'], note: 'keep me' } } };
  assert.equal(applyVerdict(grades, 'fig-1', 'reject').verdicts['fig-1'].note, 'keep me');
});

test('should drop only the named verdict and return the same object when clearing an unknown id', () => {
  const grades = applyVerdict(applyVerdict(emptyGrades(artifact), 'fig-1', 'correct'), 'tab-1', 'reject');
  const cleared = clearVerdict(grades, 'fig-1');
  assert.deepEqual(Object.keys(cleared.verdicts), ['tab-1']);
  assert.equal(clearVerdict(cleared, 'missing'), cleared);
});

test('should allocate unique add ids when adding and removing missed objects', () => {
  const region = { x_min: 10, y_min: 10, x_max: 50, y_max: 50 };
  let grades = addObject(emptyGrades(artifact), { kind: 'figure', printed_label: '4', page: 3, region, caption: { start: 5, end: 9 } });
  grades = addObject(grades, { kind: 'table', printed_label: 'IV', page: 3, region, caption: null });
  assert.deepEqual(grades.additions.map(item => item.id), ['add-1', 'add-2']);
  assert.equal(grades.additions[1].note, '');
  grades = removeAddition(grades, 'add-1');
  grades = addObject(grades, { kind: 'figure', printed_label: '5', page: 1, region, caption: null });
  assert.deepEqual(grades.additions.map(item => item.id), ['add-2', 'add-3'], 'a removed id is not reused while its successor exists');
  assert.equal(removeAddition(grades, 'nope'), grades);
});

test('should report ungraded ids and export blockers when computing progress', () => {
  let grades = applyVerdict(emptyGrades(artifact), 'fig-2', 'correct');
  grades = addObject(grades, { kind: 'figure', printed_label: '4', page: 3, region: { x_min: 1, y_min: 1, x_max: 2, y_max: 2 }, caption: null });
  const progress = gradingProgress(artifact, grades);
  assert.equal(progress.total, 4);
  assert.equal(progress.graded, 1);
  assert.deepEqual(progress.ungraded, ['fig-0', 'fig-1', 'tab-1']);
  assert.deepEqual(progress.blockers.map(item => item.id), ['fig-2', 'add-1']);
  assert.match(progress.blockers[0].reason, /without a region/);
  assert.match(progress.blockers[1].reason, /without a caption/);
  grades = applyVerdict(grades, 'fig-2', 'reject');
  for (const id of ['fig-0', 'fig-1', 'tab-1']) grades = applyVerdict(grades, id, 'correct');
  grades = { ...grades, additions: grades.additions.map(item => ({ ...item, caption: { start: 0, end: 4 } })) };
  assert.deepEqual(gradingProgress(artifact, grades), { graded: 4, total: 4, ungraded: [], blockers: [] });
});

test('should mirror the collector label normalization when normalizing printed labels', () => {
  assert.equal(normalizeLabel('Figure 3:', 'figure'), '3');
  assert.equal(normalizeLabel('Fig. S1.', 'figure'), 's1');
  assert.equal(normalizeLabel('FIG 2.3', 'figure'), '2.3');
  assert.equal(normalizeLabel('Table IV:', 'table'), 'iv');
  assert.equal(normalizeLabel('(4)', 'figure'), '4');
  assert.equal(normalizeLabel('12a', 'table'), '12a');
  assert.equal(normalizeLabel('Table 3', 'figure'), null, 'a table prefix is not a figure label');
  assert.equal(normalizeLabel('Figure', 'figure'), null);
  assert.equal(normalizeLabel('3-4', 'figure'), null);
  assert.equal(normalizeLabel('ab1', 'figure'), null);
});

test('should choose the caption paragraph on the page whose label matches when proposing a caption', () => {
  const lines = [
    ['Figure 3: Third mechanism.', 'caption', 1],
    ['See Fig. 4 below.', 'body', 2],
    ['Figure 4. Fourth mechanism.', 'caption', 2],
    ['Table IV: Main results.', 'caption', 2],
    ['Figure 4: A repeat on another page.', 'caption', 3],
  ];
  const index = { text: '', pages: [], objects: { paragraph: [] } };
  const starts = new Map();
  for (const [text, kind, page] of lines) {
    const start = index.text.length;
    index.text += text + '\n\n';
    index.objects.paragraph.push({ start, end: start + text.length, kind });
    if (!starts.has(page)) starts.set(page, start);
  }
  for (const [page, start] of starts) index.pages.push({ number: page, start, end: index.text.length, width: 612, height: 792 });
  index.pages.forEach((page, at) => { page.end = index.pages[at + 1]?.start ?? index.text.length; });
  assert.deepEqual(proposeCaption(index, 2, 'figure', '4'), { start: index.objects.paragraph[2].start, end: index.objects.paragraph[2].end });
  assert.deepEqual(proposeCaption(index, 2, 'table', 'iv'), { start: index.objects.paragraph[3].start, end: index.objects.paragraph[3].end });
  assert.deepEqual(proposeCaption(index, 2, 'table', 'Table IV'), proposeCaption(index, 2, 'table', 'IV'));
  assert.equal(proposeCaption(index, 2, 'figure', '3'), null, 'captions on other pages are not proposed');
  assert.equal(proposeCaption(index, 2, 'table', '4'), null, 'a figure caption is not a table caption');
  assert.equal(proposeCaption(index, 9, 'figure', '4'), null, 'unknown pages propose nothing');
  assert.equal(proposeCaption(index, 2, 'figure', '??'), null, 'invalid labels propose nothing');
});

test('should invert projectRect scaling and clamp to the page when converting client points', () => {
  const box = { left: 100, top: 50, width: 306, height: 396 };
  assert.deepEqual(clientToPdf(box, 612, 792, 100, 50), { x: 0, y: 0 });
  assert.deepEqual(clientToPdf(box, 612, 792, 253, 248), { x: 306, y: 396 });
  assert.deepEqual(clientToPdf(box, 612, 792, 20, 900), { x: 0, y: 792 });
  assert.equal(clientToPdf({ ...box, width: 0 }, 612, 792, 120, 60), null);
  assert.equal(clientToPdf(box, 0, 792, 120, 60), null);
});

test('should normalise corner order and reject tiny drags when building a drawn rectangle', () => {
  assert.deepEqual(dragRect({ x: 30, y: 40 }, { x: 10, y: 20 }), { x_min: 10, y_min: 20, x_max: 30, y_max: 40 });
  assert.equal(dragRect({ x: 10, y: 10 }, { x: 11, y: 40 }), null);
  assert.equal(dragRect({ x: 10, y: 10 }, { x: 40, y: 11 }), null);
});

test('should resolve null when the grades route reports grades_not_found', async t => {
  t.mock.method(globalThis, 'fetch', async path => {
    assert.equal(path, '/api/papers/paper%2Fid/objects/grades');
    return json({ error: 'grades_not_found', message: 'This paper has not been graded yet.' }, 404);
  });
  assert.equal(await gradesApi.get('paper/id'), null);
});

test('should preserve other failures when loading grades', async t => {
  t.mock.method(globalThis, 'fetch', async () => json({ message: 'Boom' }, 500));
  await assert.rejects(gradesApi.get('p1'), error => error instanceof ApiError && error.status === 500);
});

test('should PUT the document with its loaded revision when saving grades', async t => {
  const grades = { ...emptyGrades(artifact), revision: 'rev-1' };
  t.mock.method(globalThis, 'fetch', async (path, init) => {
    assert.equal(path, '/api/papers/p1/objects/grades');
    assert.equal(init.method, 'PUT');
    assert.equal(init.headers.get('Content-Type'), 'application/json');
    const body = JSON.parse(init.body);
    assert.equal(body.revision, 'rev-1');
    assert.equal(body.index_sha256, 'abc123');
    return json({ ...body, revision: 'rev-2', updated_at: '2026-09-15T20:11:00-07:00', grader: 'tjmisko' });
  });
  const saved = await gradesApi.save('p1', grades);
  assert.equal(saved.revision, 'rev-2');
  assert.equal(saved.grader, 'tjmisko');
});

test('should surface a 409 as an ApiError when the revision is stale', async t => {
  t.mock.method(globalThis, 'fetch', async () => json({ error: 'grades_conflict', message: 'These grades were saved elsewhere since you loaded them.', current: {} }, 409));
  await assert.rejects(gradesApi.save('p1', emptyGrades(artifact)), error => error instanceof ApiError && error.status === 409 && /saved elsewhere/.test(error.message));
});

test('should encode the limit and after parameters when reading the grading queue', async t => {
  const calls = [];
  t.mock.method(globalThis, 'fetch', async path => {
    calls.push(path);
    if (path.startsWith('/api/grading/queue')) return json({ summary: { complete: 1, partial: 0, ungraded: 2 }, papers: [] });
    if (path === '/api/grading/next?after=a%2Fb') return json({ paper_id: 'next-1', title: 'Next paper' });
    return json({ error: 'grading_queue_exhausted', message: 'Every paper in the grading queue is complete.' }, 404);
  });
  assert.deepEqual((await gradesApi.queue(25)).summary, { complete: 1, partial: 0, ungraded: 2 });
  await gradesApi.queue();
  assert.deepEqual(await gradesApi.next('a/b'), { paper_id: 'next-1', title: 'Next paper' });
  assert.equal(await gradesApi.next(), null);
  assert.deepEqual(calls, ['/api/grading/queue?limit=25', '/api/grading/queue', '/api/grading/next?after=a%2Fb', '/api/grading/next']);
});
