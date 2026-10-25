import assert from 'node:assert/strict';
import test from 'node:test';
import { HistoryQueue } from '../src/lib/history.ts';
import { HistoryRecorder, clipText, positionViewOf, restoreTarget } from '../src/lib/historyCapture.ts';

function setup() {
  const clock = { t: Date.UTC(2026, 9, 25, 12, 0, 0) };
  const timers = [];
  const saved = [];
  let sessions = 0;
  const queue = new HistoryQueue({
    now: () => clock.t,
    storage: null,
    newSessionId: () => `s${++sessions}`,
    send: async () => {},
    beacon: () => true,
  });
  const recorder = new HistoryRecorder({
    queue,
    now: () => clock.t,
    timers: {
      setTimeout: (callback, ms) => { const timer = { callback, ms, live: true }; timers.push(timer); return timer; },
      clearTimeout: (timer) => { timer.live = false; },
    },
    savePosition: (paperId, position) => saved.push({ paperId, position }),
  });
  const advance = (ms) => { clock.t += ms; };
  const fire = () => { for (const timer of timers) if (timer.live) { timer.live = false; timer.callback(); } };
  const kinds = () => queue.snapshot().map((event) => event.kind);
  return { clock, queue, recorder, saved, advance, fire, kinds };
}

test('should record paper_opened then paper_left with dwell when a paper is opened and left', () => {
  const { recorder, queue, advance } = setup();
  recorder.openPaper('p1', 'abstract');
  advance(30_000);
  recorder.leavePaper();
  const [opened, left] = queue.snapshot();
  assert.equal(opened.kind, 'paper_opened');
  assert.equal(opened.paper_id, 'p1');
  assert.equal(left.kind, 'paper_left');
  assert.equal(left.dwell_ms, 30_000);
});

test('should not count idle time past a minute when the reader stops interacting', () => {
  const { recorder, queue, advance } = setup();
  recorder.openPaper('p1', 'pdf');
  advance(10 * 60_000);
  recorder.leavePaper();
  assert.equal(queue.snapshot()[1].dwell_ms, 60_000);
});

test('should record page_reached once per page when the same page is reported again', () => {
  const { recorder, kinds } = setup();
  recorder.openPaper('p1', 'pdf');
  recorder.pageReached(3);
  recorder.pageReached(3);
  recorder.pageReached(4);
  recorder.pageReached(3);
  assert.deepEqual(kinds().filter((kind) => kind === 'page_reached').length, 2);
});

test('should record page_reached again when the paper is reopened', () => {
  const { recorder, kinds } = setup();
  recorder.openPaper('p1', 'pdf');
  recorder.pageReached(3);
  recorder.leavePaper();
  recorder.openPaper('p1', 'pdf');
  recorder.pageReached(3);
  assert.equal(kinds().filter((kind) => kind === 'page_reached').length, 2);
});

test('should ignore pages that are not positive integers when reporting a page', () => {
  const { recorder, kinds, saved, fire } = setup();
  recorder.openPaper('p1', 'pdf');
  for (const page of [0, -1, 1.5, Number.NaN]) recorder.pageReached(page);
  fire();
  assert.equal(kinds().includes('page_reached'), false);
  assert.deepEqual(saved, []);
});

test('should report nothing when no paper is open', () => {
  const { recorder, queue, saved, fire } = setup();
  recorder.pageReached(2);
  recorder.setView('pdf');
  recorder.focusSection('a', 'A')();
  recorder.viewFigure('Figure 1', 1)();
  fire();
  assert.equal(queue.pending, 0);
  assert.deepEqual(saved, []);
});

test('should save the position once, 2 s after the last page turn, when pages turn quickly', () => {
  const { recorder, saved, fire } = setup();
  recorder.openPaper('p1', 'pdf');
  recorder.pageReached(1);
  recorder.pageReached(2);
  recorder.pageReached(3);
  assert.deepEqual(saved, []);
  fire();
  assert.equal(saved.length, 1);
  assert.deepEqual(saved[0], { paperId: 'p1', position: { view: 'pdf', page: 3, section_id: null } });
});

test('should save a waiting position at once when the paper is left', () => {
  const { recorder, saved } = setup();
  recorder.openPaper('p1', 'markdown');
  recorder.pageReached(5);
  recorder.leavePaper();
  assert.equal(saved.length, 1);
  assert.equal(saved[0].position.view, 'markdown');
  assert.equal(saved[0].position.page, 5);
});

test('should not save a position when the view changes before any page is known', () => {
  const { recorder, saved, fire } = setup();
  recorder.openPaper('p1', 'abstract');
  recorder.setView('overview');
  fire();
  assert.deepEqual(saved, []);
});

test('should record view_changed once per change and not for the same view', () => {
  const { recorder, queue } = setup();
  recorder.openPaper('p1', 'abstract');
  recorder.setView('abstract');
  recorder.setView('pdf');
  recorder.setView('pdf');
  recorder.setView('sources');
  const changes = queue.snapshot().filter((event) => event.kind === 'view_changed');
  assert.deepEqual(changes.map((event) => event.detail), [{ view: 'pdf', from: 'abstract' }, { view: 'sources', from: 'pdf' }]);
});

test('should record paper_left then paper_opened when another paper opens', () => {
  const { recorder, kinds } = setup();
  recorder.openPaper('p1', 'pdf');
  recorder.openPaper('p2', 'pdf');
  assert.deepEqual(kinds(), ['paper_opened', 'paper_left', 'paper_opened']);
});

test('should not open a visit when the paper id is null or blank', () => {
  const { recorder, queue } = setup();
  recorder.openPaper(null, 'pdf');
  recorder.openPaper('  ', 'pdf');
  assert.equal(queue.pending, 0);
});

test('should record section_focused with its own dwell when a section closes', () => {
  const { recorder, queue, advance } = setup();
  recorder.openPaper('p1', 'overview');
  advance(5_000);
  const close = recorder.focusSection('sec-1', 'Methods');
  advance(12_000);
  close();
  close();
  const focused = queue.snapshot().filter((event) => event.kind === 'section_focused');
  assert.equal(focused.length, 1);
  assert.equal(focused[0].dwell_ms, 12_000);
  assert.deepEqual(focused[0].detail, { section_id: 'sec-1', title: 'Methods' });
});

test('should name the section in the saved position while it is open', () => {
  const { recorder, saved, fire } = setup();
  recorder.openPaper('p1', 'overview');
  recorder.pageReached(2);
  fire();
  const close = recorder.focusSection('sec-1', 'Methods');
  fire();
  close();
  assert.deepEqual(saved.map((entry) => entry.position.view), ['overview', 'section']);
  assert.equal(saved[1].position.section_id, 'sec-1');
});

test('should close open timed events before paper_left when the paper is left', () => {
  const { recorder, kinds } = setup();
  recorder.openPaper('p1', 'overview');
  recorder.focusSection('sec-1', 'Methods');
  recorder.viewFigure('Figure 2', 4);
  recorder.leavePaper();
  assert.deepEqual(kinds(), ['paper_opened', 'section_focused', 'figure_viewed', 'paper_left']);
});

test('should keep the paper id of a figure when it closes after the paper changed', () => {
  const { recorder, queue } = setup();
  recorder.openPaper('p1', 'overview');
  const close = recorder.viewFigure('Figure 2', 4);
  recorder.openPaper('p2', 'overview');
  close();
  const figure = queue.snapshot().find((event) => event.kind === 'figure_viewed');
  assert.equal(figure.paper_id, 'p1');
  assert.equal(figure.detail.page, 4);
});

test('should record work_inspected only when the panel stayed open two seconds', () => {
  const { recorder, queue, advance } = setup();
  const quick = recorder.beginInspection('w1');
  advance(1_999);
  assert.equal(quick(), false);
  const slow = recorder.beginInspection('w2');
  advance(2_000);
  assert.equal(slow(), true);
  assert.equal(slow(), false);
  const inspected = queue.snapshot().filter((event) => event.kind === 'work_inspected');
  assert.equal(inspected.length, 1);
  assert.equal(inspected[0].work_id, 'w2');
  assert.equal(inspected[0].dwell_ms, 2_000);
});

test('should fill in the open paper when recording a generic event, unless the draft names one', () => {
  const { recorder, queue } = setup();
  recorder.openPaper('p1', 'pdf');
  recorder.record({ kind: 'search_run', detail: { query: 'x' } });
  recorder.record({ kind: 'question_asked', paper_id: 'other' });
  const [, search, question] = queue.snapshot();
  assert.equal(search.paper_id, 'p1');
  assert.equal(question.paper_id, 'other');
});

test('should record events without a paper id when no paper is open', () => {
  const { recorder, queue } = setup();
  recorder.record({ kind: 'list_opened', list_id: 'l1' });
  assert.equal(queue.snapshot()[0].paper_id, undefined);
});

test('should stop counting dwell while the tab is hidden', () => {
  const { recorder, queue, advance } = setup();
  recorder.openPaper('p1', 'pdf');
  advance(10_000);
  recorder.setVisible(false);
  advance(30_000);
  recorder.setVisible(true);
  recorder.input();
  advance(5_000);
  recorder.leavePaper();
  assert.equal(queue.snapshot().at(-1).dwell_ms, 15_000);
});

test('should clip long text by code points without splitting a character', () => {
  assert.equal(clipText('abc', 5), 'abc');
  assert.equal(clipText('😀😀😀', 2), '😀😀…');
  assert.equal(Array.from(clipText('x'.repeat(2000))).length, 501);
  assert.equal(clipText(''), '');
});

test('should map reader views to the position views the server accepts', () => {
  assert.equal(positionViewOf('pdf'), 'pdf');
  assert.equal(positionViewOf('markdown'), 'markdown');
  for (const view of ['abstract', 'overview', 'glossary', 'sources']) assert.equal(positionViewOf(view), 'overview');
});

test('should open the PDF at the saved page when the position is pdf', () => {
  assert.deepEqual(restoreTarget({ view: 'pdf', page: 7 }), { view: 'text', textMode: 'pdf', page: 7 });
});

test('should open markdown without a page when the position is markdown', () => {
  assert.deepEqual(restoreTarget({ view: 'markdown', page: 3 }), { view: 'text', textMode: 'markdown', page: null });
});

test('should open the PDF at the section page when the position is a section', () => {
  assert.deepEqual(restoreTarget({ view: 'section', page: 12 }), { view: 'text', textMode: 'pdf', page: 12 });
});

test('should open the overview when the position is overview with no page', () => {
  assert.deepEqual(restoreTarget({ view: 'overview', page: null }), { view: 'overview', textMode: 'pdf', page: null });
});

test('should open the PDF at the page when the position is overview with a page', () => {
  assert.deepEqual(restoreTarget({ view: 'overview', page: 4 }), { view: 'text', textMode: 'pdf', page: 4 });
});

test('should return null when there is no saved position', () => {
  assert.equal(restoreTarget(null), null);
});

test('should clamp the page to 1 when the saved page is 0 or negative', () => {
  assert.equal(restoreTarget({ view: 'pdf', page: 0 }).page, 1);
  assert.equal(restoreTarget({ view: 'pdf', page: -5 }).page, 1);
});

test('should carry work_id on paper_opened, page_reached and paper_left when the Work is known at open', () => {
  const { recorder, queue } = setup();
  recorder.openPaper('p1', 'pdf', 'w1');
  recorder.pageReached(2);
  recorder.leavePaper();
  const events = queue.snapshot();
  assert.deepEqual(events.map((event) => event.kind), ['paper_opened', 'page_reached', 'paper_left']);
  for (const event of events) assert.equal(event.work_id, 'w1');
});

test('should attach work_id to later events and paper_left when the Work resolves after open', () => {
  const { recorder, queue } = setup();
  recorder.openPaper('p1', 'pdf');
  recorder.pageReached(1);
  recorder.setWork('p1', 'w9');
  recorder.pageReached(2);
  recorder.setView('markdown');
  recorder.leavePaper();
  const byKind = (kind) => queue.snapshot().filter((event) => event.kind === kind);
  assert.equal(byKind('paper_opened')[0].work_id, undefined);
  assert.equal(byKind('page_reached')[0].work_id, undefined);
  assert.equal(byKind('page_reached')[1].work_id, 'w9');
  assert.equal(byKind('view_changed')[0].work_id, 'w9');
  assert.equal(byKind('paper_left')[0].work_id, 'w9');
});

test('should carry work_id on section_focused and a reopened paper_opened when the Work was learned earlier', () => {
  const { recorder, queue } = setup();
  recorder.openPaper('p1', 'pdf', 'w1');
  recorder.focusSection('s1', 'Intro')();
  recorder.leavePaper();
  recorder.openPaper('p1', 'pdf');
  const events = queue.snapshot();
  assert.equal(events.find((event) => event.kind === 'section_focused').work_id, 'w1');
  assert.equal(events.filter((event) => event.kind === 'paper_opened')[1].work_id, 'w1');
});

test('should not leak a Work onto another paper when setWork names a paper that is not open', () => {
  const { recorder, queue } = setup();
  recorder.openPaper('p1', 'pdf');
  recorder.setWork('p2', 'w2');
  recorder.leavePaper();
  assert.equal(queue.snapshot()[1].work_id, undefined);
});
