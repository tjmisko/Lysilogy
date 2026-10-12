import assert from 'node:assert/strict';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { chromium } from 'playwright';

// Synthetic PDF, reading index and detector objects only; no library or backend access.
const blocks = [
  ['Introduction to the mechanism.', 'body', 1],
  ['Figure 1: First mechanism.', 'caption', 2],
  ['Figure 2: Second mechanism, uncropped.', 'caption', 2],
  ['Table IV: Main results.', 'caption', 3],
  ['Figure 4. Fourth mechanism, missed.', 'caption', 3],
  ['Closing remarks.', 'body', 4],
];
const index = { schema_version: 4, text: '', tokens: [], pages: [], objects: {word: [], WORD: [], sentence: [], paragraph: []}, figures: [], gaps: [] };
const runs = [];
for (const [text, kind, page] of blocks) {
  const start = index.text.length, y = 90 + runs.filter(run => run.page === page).length * 52;
  const run = {text, kind, page, y, start}; runs.push(run);
  for (const match of text.matchAll(/\S+/gu)) {
    const token = {start: start + match.index, end: start + match.index + match[0].length, text: match[0], page, provenance: 'native',
      rects: [{x_min: 48 + match.index * 6, x_max: 48 + (match.index + match[0].length) * 6, y_min: y - 8, y_max: y + 2}]};
    index.tokens.push(token); index.objects.word.push({start:token.start,end:token.end}); index.objects.WORD.push({start:token.start,end:token.end});
  }
  index.text += text;
  index.objects.paragraph.push({start,end:index.text.length,kind}); index.objects.sentence.push({start,end:index.text.length});
  index.text += '\n\n';
}
for (let number = 1; number <= 4; number++) {
  const tokens = index.tokens.filter(token => token.page === number);
  index.pages.push({number,start:tokens[0].start,end:tokens.at(-1).end,width:612,height:792,provenance:'native',confidence:null});
}
const paragraph = text => index.objects.paragraph[blocks.findIndex(block => block[0] === text)];
const escapePdf = text => text.replaceAll('\\', '\\\\').replaceAll('(', '\\(').replaceAll(')', '\\)');
const pdfObjects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R 10 0 R] /Count 4 >>', '<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>'];
for (let number = 1; number <= 4; number++) {
  const content = runs.filter(run => run.page === number).map(run => `BT /F1 10 Tf 1 0 0 1 48 ${792-run.y} Tm (${escapePdf(run.text)}) Tj ET`).join('\n');
  pdfObjects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${pdfObjects.length+2} 0 R >>`);
  pdfObjects.push(`<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`);
}
let pdf = '%PDF-1.4\n'; const offsets = [0];
pdfObjects.forEach((object, at) => {offsets.push(Buffer.byteLength(pdf));pdf += `${at+1} 0 obj\n${object}\nendobj\n`;});
const xref = Buffer.byteLength(pdf);
pdf += `xref\n0 ${pdfObjects.length+1}\n0000000000 65535 f \n` + offsets.slice(1).map(offset => `${String(offset).padStart(10,'0')} 00000 n \n`).join('');
pdf += `trailer\n<< /Size ${pdfObjects.length+1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;

const ids = ['1111222233334444', '5555666677778888'];
const papers = ids.map((id, at) => ({id,metadata:{title:at === 0 ? 'Graded objects fixture' : 'Second queued paper',authors:['Test Author'],year:2026,page_count:4},relative_path:`synthetic-${at}.pdf`,status:{state:'extracted'},analyzed_at:null,one_line_summary:null}));
const anchorFor = text => { const run = runs.find(run => run.text === text); return {page: run.page, start: run.start, end: run.start + run.text.length}; };
const detector = id => ({
  schema_version: 1, paper_id: id, reading_index_generation: '"index-sha-1"', figure_detector_generation: 'detector-gen-1',
  objects: [
    {kind:'figure', id:'fig-1', label:'Figure 1', page:2, text:'First mechanism.', anchor: anchorFor('Figure 1: First mechanism.'), member_anchors:[], region:{x_min:72,y_min:200,x_max:300,y_max:380}, confidence:'candidate', mentions:[]},
    {kind:'figure', id:'fig-2', label:'Figure 2', page:2, text:'Second mechanism, uncropped.', anchor: anchorFor('Figure 2: Second mechanism, uncropped.'), member_anchors:[], region:null, confidence:'candidate', mentions:[]},
    {kind:'table', id:'tab-1', label:'Table IV', page:3, text:'Main results.', anchor: anchorFor('Table IV: Main results.'), member_anchors:[], region:{x_min:60,y_min:400,x_max:540,y_max:600}, confidence:'candidate', mentions:[]},
    {kind:'equation', id:'eq-1', label:'(1)', page:1, text:'', anchor: anchorFor('Introduction to the mechanism.'), member_anchors:[], region:{x_min:1,y_min:1,x_max:2,y_max:2}, confidence:'candidate', mentions:[]},
  ],
});
const stored = new Map();
const saves = [];
let conflictOnce = false;
const root = path.resolve('dist');
const errors = [];
const browser = await chromium.launch({headless:true});
try {
  const page = await browser.newPage({viewport:{width:1280,height:800}});
  page.setDefaultTimeout(10000);
  page.on('pageerror', error => errors.push(error.message));
  await page.route('http://lysilogy.test/**', async route => {
    const url = new URL(route.request().url());
    const grades = url.pathname.match(/^\/api\/papers\/([^/]+)\/objects\/grades$/u);
    if (url.pathname === '/api/library') return route.fulfill({json:{name:'Synthetic library',papers}});
    if (url.pathname === '/api/queue') return route.fulfill({json:{jobs:[]}});
    if (url.pathname === '/api/grading/queue') return route.fulfill({json:{summary:{complete:0,partial:0,ungraded:2},papers:papers.map((paper, position) => ({paper_id:paper.id,title:paper.metadata.title,stratum:null,status:'ungraded',position}))}});
    if (url.pathname === '/api/grading/next') {
      const after = url.searchParams.get('after');
      const next = papers[(after === null ? -1 : ids.indexOf(after)) + 1];
      return next === undefined ? route.fulfill({status:404,json:{error:'grading_queue_exhausted',message:'Every paper in the grading queue is complete.'}}) : route.fulfill({json:{paper_id:next.id,title:next.metadata.title}});
    }
    if (grades !== null) {
      const id = grades[1];
      if (route.request().method() === 'PUT') {
        const body = route.request().postDataJSON();
        const record = {id, body, revision: null};
        saves.push(record);
        const current = stored.get(id) ?? null;
        if (conflictOnce) { conflictOnce = false; return route.fulfill({status:409,json:{error:'grades_conflict',message:'These grades were saved elsewhere since you loaded them.',current}}); }
        if ((current?.revision ?? null) !== body.revision) return route.fulfill({status:409,json:{error:'grades_conflict',message:'These grades were saved elsewhere since you loaded them.',current}});
        const saved = {...body, grader: body.grader || 'smoke', updated_at: '2026-09-15T20:11:00-07:00', revision: null};
        saved.revision = createHash('sha256').update(JSON.stringify(saved)).digest('hex');
        stored.set(id, saved); record.revision = saved.revision;
        return route.fulfill({json:saved});
      }
      const current = stored.get(id);
      return current === undefined ? route.fulfill({status:404,json:{error:'grades_not_found',message:'This paper has not been graded yet.'}}) : route.fulfill({json:current});
    }
    const paper = papers.find(item => url.pathname.startsWith(`/api/papers/${item.id}`));
    if (paper !== undefined && url.pathname === `/api/papers/${paper.id}`) return route.fulfill({json:{paper,analysis:null}});
    if (paper !== undefined && url.pathname.endsWith('/objects')) return route.fulfill({json:detector(paper.id)});
    if (url.pathname.endsWith('/source')) return route.fulfill({body:Buffer.from(pdf),contentType:'application/pdf'});
    if (url.pathname.endsWith('/reading-index')) return route.fulfill({json:index});
    if (url.pathname.endsWith('/reader-tools')) return route.fulfill({json:{references:[],supercuts:[],jobs:[]}});
    if (url.pathname.startsWith('/api/')) return route.fulfill({status:404,json:{message:'Unexpected fixture request'}});
    const file = path.join(root,url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
    return route.fulfill({body:await readFile(file),contentType:/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.svg')?'image/svg+xml':'text/html'});
  });
  const ready = async number => page.waitForFunction(number => document.querySelector(`[data-pdf-page="${number}"] .pdf-canvas`)?.dataset.rendered === 'true', number);
  const status = page.locator('.pdf-object-grading-status');
  const focusText = () => status.locator('[data-object-focus]').textContent();
  const saved = async count => { await page.waitForFunction(count => document.querySelector('.pdf-object-grading-status em')?.textContent === 'saved' && document.querySelector('.pdf-object-grading-status em')?.dataset.saveState === 'saved', count); assert.equal(saves.length, count, `expected ${count} saves`); return saves.at(-1).body; };
  const box = id => page.locator(`.pdf-object-box[data-object-id="${id}"]`);
  const drag = async (number, from, to) => {
    const host = await page.locator(`.pdf-text-layer-host[data-page="${number}"]`).boundingBox();
    await page.mouse.move(host.x + host.width * from[0], host.y + host.height * from[1]);
    await page.mouse.down();
    await page.mouse.move(host.x + host.width * (from[0] + to[0]) / 2, host.y + host.height * (from[1] + to[1]) / 2, {steps: 3});
    await page.locator('.pdf-object-drag').waitFor();
    await page.mouse.move(host.x + host.width * to[0], host.y + host.height * to[1], {steps: 3});
    await page.mouse.up();
  };
  const near = (actual, expected, tolerance = 3) => assert.ok(Math.abs(actual - expected) <= tolerance, `${actual} is not within ${tolerance} of ${expected}`);

  await page.goto(`http://lysilogy.test/#paper=${ids[0]}`); await ready(1);
  await page.keyboard.press('Shift+G');
  await status.waitFor();
  await ready(2);
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-object-grading'), 'true');
  await page.waitForFunction(() => document.querySelector('.pdf-object-grading-status strong')?.textContent?.includes('0/3 graded'));
  assert.match(await focusText(), /^fig-1 Figure 1 p\.2/, 'the first ungraded object is focused and its page opened');
  await box('fig-1').waitFor();
  assert.equal(await box('fig-1').getAttribute('data-state'), 'ungraded');
  assert.equal(await box('fig-1').getAttribute('data-focused'), 'true');
  assert.match(await box('fig-1').locator('.pdf-object-badge').textContent(), /fig-1 · Figure 1 · p\.2/);
  const anchor = page.locator('.pdf-object-anchor[data-object-id="fig-2"]');
  await anchor.waitFor();
  assert.match(await anchor.textContent(), /no region/);
  assert.equal(await page.locator('.pdf-object-box[data-object-id="eq-1"]').count(), 0, 'equations are not graded');
  // Boxes project the detector region onto the rendered page.
  const host2 = await page.locator('.pdf-text-layer-host[data-page="2"]').boundingBox();
  const projected = await box('fig-1').boundingBox();
  near(projected.x, host2.x + 72 / 612 * host2.width); near(projected.y, host2.y + 200 / 792 * host2.height);
  near(projected.width, 228 / 612 * host2.width); near(projected.height, 180 / 792 * host2.height);
  await mkdir('/tmp/lysilogy-object-grading-artifacts', {recursive:true});
  await page.screenshot({path:'/tmp/lysilogy-object-grading-artifacts/overlay.png'});

  // Verdicts autosave with the loaded revision (null for a new file) and adopt the server's.
  await page.keyboard.press('y');
  await page.waitForFunction(() => document.querySelector('.pdf-object-box[data-object-id="fig-1"]')?.dataset.state === 'correct');
  let body = await saved(1);
  assert.equal(body.paper_id, ids[0]);
  assert.equal(body.index_sha256, 'index-sha-1');
  assert.equal(body.objects_generation, 'detector-gen-1');
  assert.equal(body.revision, null);
  assert.equal(body.grader, '');
  assert.equal(body.schema_version, 1);
  assert.deepEqual(body.verdicts, {'fig-1': {verdict:'correct', region:null, note:''}});
  assert.deepEqual(body.additions, []);
  assert.equal(body.complete, false);
  assert.match(await status.locator('strong').textContent(), /1\/3 graded/);

  // The reader-side key chain: j moves within the page, y refuses without a region.
  await page.keyboard.press('j');
  assert.match(await focusText(), /^fig-2 Figure 2 p\.2 · no region/);
  await page.keyboard.press('y');
  assert.match(await status.locator('.pdf-object-message').textContent(), /no detector region/);
  assert.equal(saves.length, 1);
  await page.keyboard.press('n');
  await page.waitForFunction(() => document.querySelector('.pdf-object-anchor[data-object-id="fig-2"]')?.dataset.state === 'reject');
  body = await saved(2);
  assert.equal(body.revision, saves[0].revision, 'the second save carries the revision the first save returned');
  assert.equal(body.verdicts['fig-2'].verdict, 'reject');

  // Moving to a table on another page turns the page; drawing a region records the drag in PDF units.
  await page.keyboard.press('j');
  await ready(3);
  assert.match(await focusText(), /^tab-1 Table IV p\.3/);
  assert.equal(await page.locator('.pdf-object-box[data-object-id="fig-1"]').count(), 0, 'page 2 boxes leave with the page');
  await box('tab-1').waitFor();
  await page.keyboard.press('e');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-object-drawing'), '');
  await drag(3, [0.1, 0.5], [0.9, 0.75]);
  await page.waitForFunction(() => document.querySelector('.pdf-object-box[data-object-id="tab-1"]')?.dataset.state === 'region');
  body = await saved(3);
  const region = body.verdicts['tab-1'].region;
  near(region.x_min, 61.2, 2); near(region.x_max, 550.8, 2); near(region.y_min, 396, 2); near(region.y_max, 594, 2);
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-object-drawing'), null);
  assert.equal(await page.locator('.pdf-object-drag').count(), 0);

  // Escape cancels a draw without leaving the mode.
  await page.keyboard.press('e');
  await page.keyboard.press('Escape');
  assert.equal(await status.count(), 1);
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-object-drawing'), null);

  // Adding a missed figure: draw, choose the kind, type the label, confirm the proposed caption.
  await page.keyboard.press('a');
  await drag(3, [0.2, 0.1], [0.6, 0.3]);
  await page.waitForFunction(() => /Add: kind\?/.test(document.querySelector('.pdf-object-grading-status [data-object-focus]')?.textContent ?? ''));
  await page.keyboard.press('f');
  await page.keyboard.type('4');
  assert.match(await focusText(), /printed label: 4/);
  await page.keyboard.press('Enter');
  assert.match(await focusText(), /caption: Figure 4\. Fourth mechanism, missed\./);
  await page.keyboard.press('Enter');
  await page.locator('.pdf-object-box[data-object-id="add-1"][data-state="addition"][data-focused="true"]').waitFor();
  body = await saved(4);
  assert.equal(body.additions.length, 1);
  const addition = body.additions[0];
  assert.equal(addition.id, 'add-1'); assert.equal(addition.kind, 'figure'); assert.equal(addition.printed_label, '4'); assert.equal(addition.page, 3); assert.equal(addition.note, '');
  const caption = paragraph('Figure 4. Fourth mechanism, missed.');
  assert.deepEqual(addition.caption, {start: caption.start, end: caption.end});
  near(addition.region.x_min, 122.4, 2); near(addition.region.x_max, 367.2, 2); near(addition.region.y_min, 79.2, 2); near(addition.region.y_max, 237.6, 2);
  assert.match(await status.locator('strong').textContent(), /3\/3 graded · 1 added/);
  await page.screenshot({path:'/tmp/lysilogy-object-grading-artifacts/addition.png'});

  // Additions cannot take verdicts; x removes them and focus moves to a neighbour.
  await page.keyboard.press('y');
  assert.match(await status.locator('.pdf-object-message').textContent(), /Additions are truth already/);
  await page.keyboard.press('x');
  await page.waitForFunction(() => document.querySelector('.pdf-object-box[data-object-id="add-1"]') === null);
  body = await saved(5);
  assert.deepEqual(body.additions, []);
  assert.match(await focusText(), /^tab-1/);

  // A cleared verdict blocks completion; c reports the first ungraded object, then completes.
  await page.keyboard.press('u');
  await page.waitForFunction(() => document.querySelector('.pdf-object-box[data-object-id="tab-1"]')?.dataset.state === 'ungraded');
  await saved(6);
  await page.keyboard.press('c');
  assert.match(await status.locator('.pdf-object-message').textContent(), /1 object still ungraded \(first: tab-1\)/);
  await page.keyboard.press('y');
  body = await saved(7);
  assert.equal(body.verdicts['tab-1'].verdict, 'correct');
  await page.keyboard.press('c');
  body = await saved(8);
  assert.equal(body.complete, true);
  assert.match(await status.locator('strong').textContent(), /complete/);

  // A 409 reloads the server copy and reports the conflict.
  conflictOnce = true;
  await page.keyboard.press('c');
  await page.waitForFunction(() => document.querySelector('.pdf-object-grading-status em')?.dataset.saveState === 'conflict');
  assert.match(await status.locator('.pdf-object-message').textContent(), /conflict: reloaded/);
  assert.match(await status.locator('strong').textContent(), /complete/, 'the reloaded server copy is still complete');
  assert.equal(saves.length, 9);

  // Keys stay inside the mode: E must not open notes, ? shows the key list.
  await page.keyboard.press('Shift+E');
  assert.equal(await page.locator('.notes-panel').count(), 0);
  await page.keyboard.press('?');
  assert.match(await status.textContent(), /\[N\] save \+ next paper/);

  // N saves and opens the next queued paper, which enters grading on arrival.
  await page.keyboard.press('Shift+N');
  await page.waitForURL(url => url.hash === `#paper=${ids[1]}`);
  await ready(2);
  await page.waitForFunction(id => document.querySelector('.pdf-object-grading-status strong')?.textContent?.includes('0/3 graded') && location.hash === `#paper=${id}`, ids[1]);
  assert.match(await focusText(), /^fig-1 Figure 1 p\.2/);
  assert.equal(stored.has(ids[1]), false, 'nothing is saved before a verdict');

  // Escape leaves the mode and the reader stays.
  await page.keyboard.press('Escape');
  await status.waitFor({state:'detached'});
  assert.equal(await page.locator('.pdf-reader').count(), 1);
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-object-grading'), null);
  assert.equal(await page.locator('.pdf-object-box').count(), 0);

  // The :grade command finds the paper after the current one; q exits too.
  await page.keyboard.press(':');
  await page.locator('.command-menu input').fill('grade');
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => /queue exhausted/i.test(document.body.textContent ?? ''));
  await page.goto(`http://lysilogy.test/#paper=${ids[0]}`); await page.reload(); await ready(1);
  await page.keyboard.press('Shift+G');
  await status.waitFor();
  await page.waitForFunction(() => document.querySelector('.pdf-object-grading-status strong')?.textContent?.includes('3/3 graded · complete'));
  await ready(2);
  assert.equal(await box('fig-1').getAttribute('data-state'), 'correct', 'stored verdicts colour the overlay on reopen');
  assert.equal(await page.locator('.pdf-object-anchor[data-object-id="fig-2"]').getAttribute('data-state'), 'reject');
  await page.keyboard.press('q');
  await status.waitFor({state:'detached'});

  // Cursor and Visual modes keep G as "document end": grading must not open and the cursor must move.
  await page.keyboard.press('C');
  await page.locator('.pdf-cursor-badge').waitFor();
  const cursor = () => page.waitForFunction(() => document.querySelector('.pdf-source-mark.is-cursor')?.dataset.sourceOffset !== undefined)
    .then(() => page.locator('.pdf-source-mark.is-cursor').first().getAttribute('data-source-offset')).then(Number);
  const before = await cursor();
  await page.keyboard.press('Shift+G');
  await page.waitForFunction(before => Number(document.querySelector('.pdf-source-mark.is-cursor')?.dataset.sourceOffset) > before, before);
  assert.equal(await status.count(), 0, 'cursor mode owns G');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-object-grading'), null);
  await page.keyboard.press('v');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-source-visual'), 'true');
  await page.keyboard.press('Shift+G');
  assert.equal(await status.count(), 0, 'visual mode owns G');
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.pdf-cursor-badge').count(), 0);
  await page.keyboard.press('Shift+G');
  await status.waitFor();
  await page.keyboard.press('q');
  await status.waitFor({state:'detached'});
  assert.deepEqual(errors, []);
  console.log(`Object grading smoke passed: overlay projection, verdict keys, autosave bodies, region drawing, add flow, completion, conflict reload, cursor/visual-mode G guard and queue navigation (${saves.length} saves).`);
} finally {await browser.close();}
