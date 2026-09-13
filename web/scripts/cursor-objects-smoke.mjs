import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {readFile} from 'node:fs/promises';
import {promisify} from 'node:util';
import path from 'node:path';
import {chromium} from 'playwright';

const root=path.resolve('..');
const {stdout}=await promisify(execFile)('cargo',['test','--test','cursor_objects','--','--nocapture'],{cwd:root,maxBuffer:4*1024*1024});
const fixture=stdout.split('\n').find(line=>line.startsWith('CURSOR_INDEX_JSON '));
assert.ok(fixture,stdout);
const index=JSON.parse(fixture.slice('CURSOR_INDEX_JSON '.length));
const pdf=await readFile(path.join(root,'tests/fixtures/cursor-objects.pdf'));
const id='1234567890abcdef';
const paper={id,metadata:{title:'Cursor figure and gutter fixture',authors:['Test Author'],year:2026,page_count:1},relative_path:'fixture.pdf',status:{state:'discovered'},analyzed_at:null,one_line_summary:null};
const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1280,height:900}});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.addInitScript(()=>{window.copiedSource='';Object.defineProperty(navigator,'clipboard',{value:{writeText:async text=>{window.copiedSource=text;}}});});
  await page.route('**/*',async route=>{
    const url=new URL(route.request().url());
    if(url.pathname==='/api/library')return route.fulfill({json:{name:'Fixture',papers:[paper]}});
    if(url.pathname==='/api/queue')return route.fulfill({json:{jobs:[]}});
    if(url.pathname===`/api/papers/${id}`)return route.fulfill({json:{paper,analysis:null}});
    if(url.pathname.endsWith('/source'))return route.fulfill({body:pdf,contentType:'application/pdf'});
    if(url.pathname.endsWith('/reading-index'))return route.fulfill({json:index});
    if(url.pathname.endsWith('/reader-tools'))return route.fulfill({json:{jobs:[],references:[],supercuts:[]}});
    if(url.pathname.startsWith('/api/'))return route.fulfill({status:404,json:{message:'Unexpected fixture request'}});
    const file=path.join(root,'web/dist',url.pathname==='/'?'index.html':url.pathname.slice(1));
    return route.fulfill({body:await readFile(file),contentType:/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});
  });
  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await page.waitForFunction(()=>document.querySelector('.pdf-canvas')?.dataset.rendered==='true');
  await page.keyboard.press('C');await page.locator('.pdf-source-line-number').first().waitFor();
  const numbered=await page.locator('.pdf-source-line-number').evaluateAll(nodes=>nodes.map(node=>({offset:Number(node.dataset.sourceOffset),right:node.getBoundingClientRect().right,size:Number.parseFloat(getComputedStyle(node).fontSize)})));
  assert.ok(numbered.length>=12);
  for(const line of numbered) {
    assert.ok(line.size<=9,'numbers are smaller');
    assert.ok(!index.figures.some(figure=>figure.spans.some(span=>span.start<=line.offset&&line.offset<span.end)),`a figure/table member received a line number: ${index.text.slice(line.offset,line.offset+50)}`);
  }
  const body=numbered.filter(line=>{const token=index.tokens.find(t=>t.start===line.offset);return token.rects[0].y_min>390&&token.rects[0].y_min<490;});
  const columns=[body.filter(line=>index.tokens.find(t=>t.start===line.offset).rects[0].x_min<300),body.filter(line=>index.tokens.find(t=>t.start===line.offset).rects[0].x_min>300)];
  for(const column of columns) {
    assert.ok(column.length>=4);
    assert.ok(Math.max(...column.map(l=>l.right))-Math.min(...column.map(l=>l.right))<1,'indents and bullets share a flush gutter');
  }
  const search=async query=>{
    await page.keyboard.press('/');const input=page.getByRole('textbox',{name:'Search paper with regular expression'});
    await input.fill(query);await input.press('Enter');
    await page.waitForFunction(query=>document.querySelector('.pdf-source-status')?.textContent.includes(`/${query} · 1 / 1`),query);
  };
  await search('classifier');await page.keyboard.type('viwy');
  await page.waitForFunction(()=>window.copiedSource==='classifier');
  await page.keyboard.type('yy');await page.waitForFunction(()=>window.copiedSource.startsWith('Figure 2.')&&window.copiedSource.includes('[Figure 2 · p. 1]'));
  await search('54[.]70');await page.keyboard.type('viWy');
  await page.waitForFunction(()=>window.copiedSource==='54.70');
  // Normal movement leaves the whole grid in one step; it does not visit its cells.
  await page.keyboard.press('j');
  await page.waitForFunction(text=>{const mark=document.querySelector('.pdf-source-mark.is-cursor');return mark&&text.slice(Number(mark.dataset.sourceOffset)).startsWith('The paragraph after');},index.text);
  await page.keyboard.press('k');await page.keyboard.press('v');await page.locator('.pdf-source-mark.is-block-visual').waitFor();
  await page.keyboard.press('y');await page.waitForFunction(()=>window.copiedSource.startsWith('Table 3.')&&window.copiedSource.includes('[Table 3 · p. 1]'));
  await page.screenshot({path:'/tmp/lysilogy-cursor-objects-fixed.png'});
  assert.deepEqual(errors,[]);
  console.log('PASS real-PDF diagram/table membership, searchable/selectable internal text, atomic motion and caption yanks, no object line numbers, smaller flush two-column gutters');
} finally {await browser.close();}
