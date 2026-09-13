import assert from 'node:assert/strict';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

// Synthetic PDFs and source indices only; no library or backend access.
const blocks = [
  ['Compare [1, 2] with Smith (2020).', 'body', 1],
  ['Figs. 1-3 and Table IV summarize the results.', 'body', 1],
  ['Supplementary Fig. 1 provides detail.', 'body', 1],
  ['Fig. 9 is unresolved. Eq. (2) is unrelated.', 'body', 1],
  ['More evidence [1] and [2] and [1] supports this.', 'body', 1],
  ['Publisher link', 'body', 1],
  ['Online appendix', 'body', 1],
  ['Figure 1: First mechanism.', 'caption', 2],
  ['Figure 2: Second mechanism.', 'caption', 2],
  ['Figure 3: Third mechanism.', 'caption', 2],
  ['See Fig. 2 for detail on this same page.', 'body', 2],
  ['Table IV: Main results.', 'body', 3],
  ['Figure S1. Supplementary mechanism.', 'caption', 3],
  ['References', 'heading', 4],
  ['[1] Adams, A. (2019). First paper.', 'body', 4],
  ['[2] Brown, B. (2021). Second paper.', 'body', 4],
  ['Smith, C. (2020). Author-year paper.', 'body', 4],
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
const escapePdf = text => text.replaceAll('\\', '\\\\').replaceAll('(', '\\(').replaceAll(')', '\\)');
const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R 10 0 R] /Count 4 >>', '<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>'];
for (let number = 1; number <= 4; number++) {
  const content = runs.filter(run => run.page === number).map(run => `BT /F1 10 Tf 1 0 0 1 48 ${792-run.y} Tm (${escapePdf(run.text)}) Tj ET`).join('\n');
  objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${objects.length+2} 0 R ${number === 1 ? '/Annots [12 0 R 13 0 R]' : ''} >>`);
  objects.push(`<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`);
}
const nativeRun = runs.find(run => run.text === 'Publisher link'), externalRun = runs.find(run => run.text === 'Online appendix');
objects.push(`<< /Type /Annot /Subtype /Link /Rect [48 ${792-nativeRun.y-2} 132 ${792-nativeRun.y+8}] /Dest [10 0 R /XYZ 48 600 null] /Border [0 0 0] >>`);
objects.push(`<< /Type /Annot /Subtype /Link /Rect [48 ${792-externalRun.y-2} 138 ${792-externalRun.y+8}] /A << /S /URI /URI (https://example.org/appendix) >> /Border [0 0 0] >>`);
let pdf = '%PDF-1.4\n'; const offsets = [0];
objects.forEach((object, at) => {offsets.push(Buffer.byteLength(pdf));pdf += `${at+1} 0 obj\n${object}\nendobj\n`;});
const xref = Buffer.byteLength(pdf);
pdf += `xref\n0 ${objects.length+1}\n0000000000 65535 f \n` + offsets.slice(1).map(offset => `${String(offset).padStart(10,'0')} 00000 n \n`).join('');
pdf += `trailer\n<< /Size ${objects.length+1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
const id = '1111222233334444';
const paper = {id,metadata:{title:'Paper link conventions',authors:['Test Author'],year:2026,page_count:4},relative_path:'synthetic.pdf',status:{state:'extracted'},analyzed_at:null,one_line_summary:null};
const layout = {schema_version:1,pages:index.pages.map(page => ({number:page.number,width:612,height:792,
  tokens:index.tokens.filter(token=>token.page===page.number).map((token,at)=>({index:at,text:token.text,line:runs.findIndex(run=>run.page===page.number&&run.start<=token.start&&run.start+run.text.length>=token.end),rects:token.rects})),
  sentences:runs.filter(run=>run.page===page.number).map((run,at)=>({id:`p${page.number}-s${at}`,page:page.number,text:run.text,start_token:index.tokens.filter(token=>token.page===page.number).findIndex(token=>token.start===run.start),end_token:index.tokens.filter(token=>token.page===page.number&&token.start<run.start+run.text.length).length-1,rects:[{x_min:48,x_max:48+run.text.length*6,y_min:run.y-8,y_max:run.y+2}]}))}))};
const anchor = sentence => ({page:1,start_token:sentence.start_token,end_token:sentence.end_token,sentence_ids:[sentence.id],rects:sentence.rects,exact_text:sentence.text});
const section = {id:'mechanism',title:'Mechanism and supporting displays',kind:'methods',family:'method',pages:{start:1,end:1},summary:'This section cites the displays.',digest:'Compare the mechanism and its evidence.',
  source_span:{start:anchor(layout.pages[0].sentences[1]),end:anchor(layout.pages[0].sentences[2])},key_quotes:[],related_terms:[],tile_width:1,tile_height:1};
const analysis = {schema_version:5,provider:'heuristic',generated_at:'2026-09-12T12:00:00Z',thesis:'A synthetic paper.',outsider_brief:'A synthetic fixture.',author_abstract:null,context_notes:[],context_sources:[],prerequisites:[],sections:[section],claims:[],glossary:[],caveats:[],reading_path:['mechanism']};
const root = path.resolve('dist');
let failIndex = false, gate = null, indexRequests = 0, analyzed = false;
const errors = [];
const browser = await chromium.launch({headless:true});
try {
  const page = await browser.newPage({viewport:{width:1280,height:800}});
  page.setDefaultTimeout(10000);
  page.on('pageerror', error => errors.push(error.message));
  await page.route('http://lysilogy.test/**', async route => {
    const url = new URL(route.request().url());
    if (url.pathname === '/api/library') return route.fulfill({json:{name:'Synthetic library',papers:[paper]}});
    if (url.pathname === '/api/queue') return route.fulfill({json:{jobs:[]}});
    if (url.pathname === `/api/papers/${id}`) return route.fulfill({json:{paper:analyzed?{...paper,status:{state:'ready'}}:paper,analysis:analyzed?analysis:null}});
    if (url.pathname.endsWith('/map')) return route.fulfill({json:{layout,highlights:[]}});
    if (url.pathname.endsWith('/source')) return route.fulfill({body:Buffer.from(pdf),contentType:'application/pdf'});
    if (url.pathname.endsWith('/reading-index')) {
      indexRequests++;
      if (gate !== null) await gate.promise;
      if (failIndex) return route.fulfill({status:503,json:{message:'Synthetic extraction unavailable'}});
      return route.fulfill({json:index});
    }
    if (url.pathname.endsWith('/reader-tools')) return route.fulfill({json:{references:[],supercuts:[],jobs:[]}});
    if (url.pathname.startsWith('/api/')) return route.fulfill({status:404,json:{message:'Unexpected fixture request'}});
    const file = path.join(root,url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
    return route.fulfill({body:await readFile(file),contentType:/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.svg')?'image/svg+xml':'text/html'});
  });
  const ready = async number => page.waitForFunction(number => document.querySelector(`[data-pdf-page="${number}"] .pdf-canvas`)?.dataset.rendered === 'true', number);
  const open = async () => {await page.keyboard.press('f');await page.waitForFunction(() => {const status=document.querySelector('.pdf-link-hint-status strong');return status&&status.textContent !== 'Finding links…';});};
  const follow = async locator => {const code = await locator.getAttribute('data-hint-code');assert.ok(code);await page.keyboard.type(code);};
  const goBack = async () => {await page.keyboard.press('Control+o');await ready(1);await page.locator('.pdf-link-destination').waitFor();};
  await page.goto(`http://lysilogy.test/#paper=${id}`); await ready(1);
  await open();
  assert.equal(await page.locator('.pdf-link-hint').count(), 13);
  assert.equal(await page.locator('.pdf-link-hint[data-link-kind="reference"]').count(), 6);
  assert.equal(await page.locator('.pdf-link-hint[data-link-kind="figure"]').count(), 4);
  assert.equal(await page.locator('.pdf-link-hint[data-link-kind="table"]').count(), 1);
  const boxes = await page.locator('.pdf-link-hint').evaluateAll(nodes => nodes.map(node => {const {left,top,right,bottom} = node.getBoundingClientRect();return {left,top,right,bottom};}));
  for (let i=0;i<boxes.length;i++) for(let j=i+1;j<boxes.length;j++) assert.ok(!(boxes[i].left<boxes[j].right&&boxes[j].left<boxes[i].right&&boxes[i].top<boxes[j].bottom&&boxes[j].top<boxes[i].bottom), 'Hint badges must not overlap');
  await mkdir('/tmp/lysilogy-link-hints-artifacts', {recursive:true});
  await page.screenshot({path:'/tmp/lysilogy-link-hints-artifacts/hints.png'});

  const first = page.locator('.pdf-link-hint[data-link-kind="reference"]').first();
  const code = await first.getAttribute('data-hint-code');
  assert.equal(code.length,2);
  await page.keyboard.press(code[0]);
  assert.ok(await page.locator('.pdf-link-hint').count() < 13);
  await page.keyboard.press('z');
  assert.equal(await page.locator('.pdf-link-hint').count(),0);
  await page.keyboard.press('Backspace');
  await page.keyboard.press('Backspace');
  assert.equal(await page.locator('.pdf-link-hint').count(),13);
  await follow(first); await ready(4);
  await page.locator('.pdf-link-destination').waitFor();
  assert.match(await page.locator('.pdf-link-destination').getAttribute('aria-label'), /Adams/);
  await goBack();

  await open();
  await follow(page.locator('.pdf-link-hint[data-link-kind="table"]')); await ready(3);
  assert.equal(await page.locator('.pdf-link-hints').count(),0);
  await goBack();
  await open(); await follow(page.locator('.pdf-link-hint[data-link-kind="figure"]').first()); await ready(2);
  await open(); assert.equal(await page.locator('.pdf-link-hint').count(),1);
  await follow(page.locator('.pdf-link-hint')); await page.locator('.pdf-link-destination').waitFor();
  assert.match(await page.locator('.pdf-link-destination').getAttribute('aria-label'), /Figure 2/);
  await page.keyboard.press('Control+o'); await page.keyboard.press('Control+o'); await ready(1);

  await open(); await follow(page.locator('.pdf-link-hint[title^="Page 4"]')); await ready(4); await goBack();
  await open();
  const popup = page.waitForEvent('popup');
  await follow(page.locator('.pdf-link-hint[title^="https://example.org"]'));
  const external = await popup; await external.close();

  await open(); await page.keyboard.press('Escape');
  assert.equal(await page.locator('.pdf-link-hints').count(),0);
  assert.equal(await page.locator('.pdf-reader').count(),1);
  await open(); await page.keyboard.press('q');
  assert.equal(await page.locator('.pdf-reader').count(),1);
  await open(); await page.keyboard.press('E');
  assert.equal(await page.locator('.notes-panel').count(),0, 'Hints own typed keys before application shortcuts');
  await page.keyboard.press('Escape');
  await page.keyboard.press('/');
  const search = page.getByRole('textbox',{name:'Search paper with regular expression'});
  await search.fill('f');
  assert.equal(await page.locator('.pdf-link-hints').count(),0, 'Editable fields keep f');
  await page.keyboard.press('Escape');

  // Link hints must own their keys without exiting the newer Cursor/Visual modes.
  await page.keyboard.press('C');
  await page.locator('.pdf-cursor-badge').waitFor();
  await page.keyboard.press('v');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-source-visual'), 'true');
  await open(); await page.keyboard.press('Escape');
  assert.equal(await page.locator('.pdf-cursor-badge').count(), 1);
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-source-visual'), 'true');
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-source-visual'), null);
  await open(); await page.keyboard.press('q');
  assert.equal(await page.locator('.pdf-cursor-badge').count(), 1);
  await page.keyboard.press('C');
  assert.equal(await page.locator('.pdf-cursor-badge').count(), 0);

  await page.keyboard.press('P'); await page.keyboard.press('W'); await ready(1);
  await open();
  assert.ok(await page.locator('.pdf-link-hint').count() > 0);
  await page.locator('.pdf-viewport').evaluate(node => node.scrollTop += 100);
  await page.waitForFunction(() => !document.querySelector('.pdf-link-hints'));
  await page.locator('.pdf-viewport').evaluate(node => {node.scrollTop=0;});
  await open(); await follow(page.locator('.pdf-link-hint[data-link-kind="table"]')); await ready(3);
  await page.waitForFunction(() => {const node=document.querySelector('.pdf-link-destination');if(!node)return false;const a=node.getBoundingClientRect(),b=document.querySelector('.pdf-viewport').getBoundingClientRect();return a.top>=b.top&&a.bottom<=b.bottom;});
  assert.ok(await page.evaluate(()=>document.querySelector('.pdf-link-destination').getBoundingClientRect().top-document.querySelector('.pdf-viewport').getBoundingClientRect().top<=30), 'Continuous links reveal the destination, not just its page');
  await page.keyboard.press('Control+o');
  await page.waitForFunction(() => document.querySelector('.pdf-viewport').scrollTop < 100);
  await page.keyboard.press('P');
  await page.keyboard.press('H'); await ready(1);
  await open(); await page.setViewportSize({width:1100,height:750});
  await page.locator('.pdf-link-hints').waitFor({state:'detached'});

  // A failed index still leaves embedded PDF links usable.
  failIndex = true;
  await page.reload(); await ready(1); await open();
  assert.equal(await page.locator('.pdf-link-hint').count(),2);
  assert.match(await page.locator('.pdf-link-hint-status').textContent(), /Text links unavailable/);
  await page.keyboard.press('Escape');
  failIndex = false;
  await open(); assert.equal(await page.locator('.pdf-link-hint').count(),13);
  await page.keyboard.press('Escape');

  gate = Promise.withResolvers();
  await page.reload(); await ready(1);
  await page.keyboard.press('f');
  assert.match(await page.locator('.pdf-link-hint-status').textContent(), /Finding links/);
  await page.keyboard.press('Escape');
  gate.resolve(); gate = null;
  await page.waitForTimeout(100);
  assert.equal(await page.locator('.pdf-link-hints').count(),0, 'Cancelled extraction cannot reopen hints');
  await open(); assert.equal(await page.locator('.pdf-link-hint').count(),13);
  await page.keyboard.press('Escape');

  // Only references inside the selected section crop get hints. Following a table
  // outside that crop must survive the reader being replaced by the full paper.
  analyzed = true;
  await page.reload();
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await page.locator('.section-boxes button[data-section-id="mechanism"]').first().click();
  await page.locator('.pdf-reader').waitFor(); await ready(1);
  await open();
  assert.equal(await page.locator('.pdf-link-hint[data-link-kind="table"]').count(),0, 'Horizontally clipped citations get no hints');
  await page.keyboard.press('Escape'); await page.keyboard.press('W'); await ready(1);
  await open();
  assert.equal(await page.locator('.pdf-link-hint').count(),5);
  assert.equal(await page.locator('.pdf-link-hint[data-link-kind="reference"]').count(),0);
  await page.screenshot({path:'/tmp/lysilogy-link-hints-artifacts/section-hints.png'});
  await follow(page.locator('.pdf-link-hint[data-link-kind="table"]'));
  await ready(3); await page.locator('.pdf-link-destination').waitFor();
  assert.match(await page.locator('.pdf-link-destination').getAttribute('aria-label'), /Table IV/);
  assert.equal(await page.locator('[data-section-crop="true"]').count(),0);
  await goBack();
  await open(); assert.equal(await page.locator('.pdf-link-hint').count(),13);
  assert.deepEqual(errors,[]);
  console.log(`Link hints smoke passed: paper conventions, all destination types, hint filtering, crops, history, keyboard isolation and extraction recovery (${indexRequests} index requests).`);
} finally {await browser.close();}
