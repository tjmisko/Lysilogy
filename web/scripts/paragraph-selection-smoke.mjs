import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { chromium } from 'playwright';
import { checkCursorMode } from './cursor-mode-checks.mjs';

// Real Poppler extraction + PDF.js; only the HTTP transport is intercepted.
// The Rust integration test creates its own temporary, synthetic PDF/cache.
const opening = ['We compare our single-view estimator to the', 'off-the-shelf UmeTrack baseline [11], which'];
const continuation = ['was used to provide the original annotations.', 'Our classifier improves the action recognition', 'accuracy of hand poses estimated with UmeTrack.'];
const following = ['Additionally, we present classification confusion', 'matrices and discuss the remaining errors.'];
const root=path.resolve('..');
const {stdout}=await promisify(execFile)('cargo',['test','--test','paragraph_selection','--','--nocapture'],{cwd:root,maxBuffer:4*1024*1024});
const fixture=stdout.split('\n').find(line=>line.startsWith('READING_INDEX_JSON '));
assert.ok(fixture,stdout);
const index=JSON.parse(fixture.slice('READING_INDEX_JSON '.length));
const pdf=await readFile(path.join(root,'tests/fixtures/paragraph-continuation.pdf'));
const id='1234567890abcdef';
const paper={id,metadata:{title:'Synthetic paragraph continuation',authors:['Test Author'],year:2026,page_count:2},relative_path:'synthetic.pdf',status:{state:'discovered'},analyzed_at:null,one_line_summary:null};
const paragraph=index.objects.paragraph.find(p=>index.text.slice(p.start,p.start+20).startsWith('We compare'));
assert.equal(paragraph?.spans?.length,2,JSON.stringify(index.objects.paragraph.map(p=>({...p,text:index.text.slice(p.start,p.end)})),null,2));
const expected=[...opening,...continuation].join(' ');
assert.equal(paragraph.spans.map(span=>index.text.slice(span.start,span.end)).join(' '),expected);
let analyzed=false;
let sourceGate=null;
const layout={schema_version:1,pages:index.pages.map(page=>({number:page.number,width:page.width,height:page.height,
  tokens:index.tokens.filter(token=>token.page===page.number).map((token,i)=>({...token,index:i,line:Math.round(token.rects[0].y_min)})),sentences:[]}))};
const anchor=(offset)=>{
  const token=index.tokens.find(token=>token.start<=offset&&token.end>offset);
  const at=layout.pages.find(page=>page.number===token.page).tokens.findIndex(item=>item.start===token.start);
  return {page:token.page,start_token:at,end_token:at,sentence_ids:[],rects:token.rects,exact_text:token.text};
};
const section={id:'comparison',title:'Action classification',kind:'theory',family:'theory',pages:{start:1,end:2},
  summary:'Compare hand pose estimators.',digest:'The comparison continues across the page break.',
  source_span:{start:anchor(paragraph.start),end:anchor(paragraph.end-1)},key_quotes:[],related_terms:[],tile_width:1,tile_height:1};
const analysis={schema_version:5,provider:'heuristic',generated_at:'2026-09-12T12:00:00Z',thesis:'Compare estimators.',author_abstract:'A synthetic source fixture.',outsider_brief:'',
  context_notes:[],context_sources:[],prerequisites:[],sections:[section],claims:[],glossary:[],caveats:[],reading_path:[]};
const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1280,height:850}});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.addInitScript(()=>{window.copiedSource='';Object.defineProperty(navigator,'clipboard',{value:{writeText:async text=>{window.copiedSource=text;}}});});
  await page.route('**/*',async route=>{
    const url=new URL(route.request().url());
    if(url.pathname==='/api/library')return route.fulfill({json:{name:'Synthetic library',papers:[paper]}});
    if(url.pathname==='/api/queue')return route.fulfill({json:{jobs:[]}});
    if(url.pathname===`/api/papers/${id}`)return route.fulfill({json:{paper,analysis:analyzed?analysis:null}});
    if(url.pathname.endsWith('/map'))return route.fulfill({json:{layout,highlights:[]}});
    if(url.pathname.endsWith('/source')){
      if(sourceGate!==null)await sourceGate.promise;
      return route.fulfill({body:pdf,contentType:'application/pdf'});
    }
    if(url.pathname.endsWith('/reading-index'))return route.fulfill({json:index});
    if(url.pathname.endsWith('/reader-tools'))return route.fulfill({json:{jobs:[],references:[],supercuts:[]}});
    if(url.pathname.startsWith('/api/'))return route.fulfill({status:404,json:{message:'Unexpected fixture request'}});
    const file=path.join(root,'web/dist',url.pathname==='/'?'index.html':url.pathname.slice(1));
    return route.fulfill({body:await readFile(file),contentType:/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.svg')?'image/svg+xml':'text/html'});
  });
  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await page.waitForFunction(()=>document.querySelector('.pdf-canvas')?.dataset.rendered==='true');
  const select=async(pattern,object='ap')=>{
    await page.keyboard.press('/');const input=page.getByRole('textbox',{name:'Search paper with regular expression'});
    await input.fill(pattern);await input.press('Enter');
    await page.waitForFunction(pattern=>document.querySelector('.pdf-source-status')?.textContent.includes(`/${pattern} · 1 / 1`),pattern,{timeout:5000}).catch(async error=>{ throw Error(`${error.message}\n${await page.locator('.pdf-source-tools').innerText()}\n${JSON.stringify(errors)}`); });
    await page.keyboard.type('v'+object);
    await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('VISUAL'),null,{timeout:5000}).catch(async error=>{await page.screenshot({path:'/tmp/lysilogy-cursor-error.png'});throw Error(error.message+' '+await page.locator('.pdf-source-tools').innerText()+' '+JSON.stringify(errors));});
  };
  await select('compare');
  await page.keyboard.press('o');
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="1"] canvas')?.dataset.rendered==='true');
  await page.keyboard.press('o');
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="2"] canvas')?.dataset.rendered==='true');
  await page.keyboard.press('y');
  await page.waitForFunction(text=>window.copiedSource===text,expected+'\n\n');
  // Selection from the continuation is the same paragraph; render both pages.
  await page.keyboard.press('P');await select('provide','ip');
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="2"] [data-text-ready="true"]'));
  const onlyProse=async()=>{
    await page.locator('.pdf-source-mark.is-visual').first().waitFor({state:'attached'});
    const marks=await page.locator('.pdf-source-mark.is-visual').evaluateAll(nodes=>nodes.map(node=>({start:Number(node.dataset.sourceOffset),end:Number(node.dataset.sourceEnd)})));
    assert.ok(marks.length>0);
    for(const mark of marks)assert.ok(paragraph.spans.some(span=>mark.start>=span.start&&mark.end<=span.end),`intervening text selected: ${JSON.stringify(mark)}`);
  };
  await onlyProse();
  // Editing either visual endpoint must retain the hole for table/figure text.
  await page.keyboard.press('h');await onlyProse();await page.keyboard.press('l');
  await page.keyboard.press('y');await page.waitForFunction(text=>window.copiedSource===text,expected);
  await select('Confusion','ip');await page.keyboard.press('y');
  await page.waitForFunction(()=>window.copiedSource==='Figure 6. Confusion matrices of verb classification.');
  await select('Additionally','ip');await page.keyboard.press('y');
  await page.waitForFunction(text=>window.copiedSource===text,following.join(' '));
  await checkCursorMode(page,index);
  // The same logical object is allowed inside a section crop: the excluded
  // floats are not treated as required selection members by the bounds check.
  analyzed=true;await page.reload();
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  sourceGate=Promise.withResolvers();
  await page.locator('.section-boxes button[data-section-id="comparison"]').first().click();
  await page.locator('.section-focus').waitFor();
  // The retained text index can answer before PDF.js knows the page count.
  await select('compare','ip');
  assert.equal(await page.getByRole('button',{name:'Open match in full paper'}).count(),0);
  sourceGate.resolve();sourceGate=null;
  await onlyProse();
  await page.keyboard.press('y');await page.waitForFunction(text=>window.copiedSource===text,expected);
  await page.keyboard.press('C');await page.keyboard.type('gg');
  await page.locator('.pdf-source-line-number.is-active').waitFor();
  await page.screenshot({path:'/tmp/lysilogy-cursor-crop.png'});
  const start=Number(await page.locator('.pdf-source-mark.is-cursor').first().getAttribute('data-source-offset'));
  assert.equal(start,paragraph.start,'gg stops at the section boundary');
  await page.keyboard.press('k');
  assert.equal(Number(await page.locator('.pdf-source-mark.is-cursor').first().getAttribute('data-source-offset')),paragraph.start);
  await page.keyboard.press('C');
  assert.deepEqual(errors,[]);
  console.log('PASS real PDF indexing, cross-page vap/vip from either fragment, excluded table/figure geometry, endpoint motions, independent captions/next paragraph, and cropped section reader');
} finally { await browser.close(); }
