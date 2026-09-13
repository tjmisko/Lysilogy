import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { checkPdfFits, checkVisualFitPages, pressVisualFit } from './pdf-fit-checks.mjs';

// All source material is synthetic; this suite never opens a reading-library path.
function makePdf() {
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R 10 0 R] /Count 4 >>', '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
  for (let page = 1; page <= 4; page++) {
    const content = `BT /F1 18 Tf 48 738 Td (Source page ${page}) Tj /F1 12 Tf ` + Array.from({length:22}, (_,i)=>`0 -28 Td (Sentence ${i+1} on page ${page} describes the evidence and its limits.) Tj`).join(' ') + ' ET';
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${objects.length+2} 0 R >>`);
    objects.push(`<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`);
  }
  let pdf='%PDF-1.4\n'; const offsets=[0];
  objects.forEach((object,i)=>{ offsets.push(Buffer.byteLength(pdf)); pdf+=`${i+1} 0 obj\n${object}\nendobj\n`; });
  const xref=Buffer.byteLength(pdf);
  pdf+=`xref\n0 ${objects.length+1}\n0000000000 65535 f \n`+offsets.slice(1).map(offset=>`${String(offset).padStart(10,'0')} 00000 n \n`).join('');
  pdf+=`trailer\n<< /Size ${objects.length+1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return Buffer.from(pdf);
}
const pdf=makePdf();
const id='1234567890abcdef';
const now='2026-09-11T18:00:00Z';
const rect=(y)=>({x_min:48,y_min:y,x_max:530,y_max:y+14});
const layout={schema_version:1,pages:Array.from({length:4},(_,i)=>({number:i+1,width:612,height:792,
  tokens:Array.from({length:22},(_,j)=>({index:j,text:`Sentence ${j+1}`,line:j,rects:[rect(75+j*28)]})),
  sentences:Array.from({length:22},(_,j)=>({id:`p${i+1}-s${j}`,page:i+1,start_token:j,end_token:j,text:`Sentence ${j+1} on page ${i+1} describes the evidence and its limits.`,rects:[rect(75+j*28)]}))}))};
const anchor=(page,token)=>({page,start_token:token,end_token:token,sentence_ids:[],rects:[rect(75+token*28)],exact_text:'Source evidence'});
const section=(id,title,start,end)=>({id,title,kind:'theory',family:'theory',pages:{start,end},summary:`${title} explains the mechanism and its limits.`,digest:`This section provides a specific argument for ${title}.`,source_span:{start:anchor(start,0),end:anchor(end,21)},key_quotes:[{text:`Sentence 1 on page ${start} describes the evidence and its limits.`,page:start,significance:'definition',explanation:'States the mechanism precisely.',validation:'exact',anchor:anchor(start,0)}],related_terms:[],tile_width:1,tile_height:1});
const analysis={schema_version:5,provider:'codex',generated_at:now,thesis:'Selecting a noisy proxy also selects its error.',outsider_brief:'THIS MERGED LEGACY BRIEF MUST NOT BE USED AS BEFORE.',author_abstract:'This synthetic paper studies a noisy proxy and carefully qualifies the mechanism.',abstract_extraction:{status:'accepted',start_page:1,end_page:1,checks:[],review:null},
  context_notes:[{kind:'before',text:'Prior research distinguished measurement error from changes in the goal.',source_ids:['prior']},{kind:'after',text:'A subsequent experiment extended the model to correlated observations.',source_ids:['later']}],
  context_sources:[{id:'prior',title:'Earlier measurement study',authors:['A. Researcher'],year:2010,url:'https://example.org/prior',supports:'The prior measurement problem.',verified_at:now,excerpt:'Measurement error changes the observed proxy.',location:'Introduction',relationship:'antecedent'},{id:'later',title:'Later correlated observations',authors:['B. Researcher'],year:2024,url:'https://example.org/later',supports:'The demonstrated extension.',verified_at:now,excerpt:'We extend this model to correlated observations.',location:'Methods',relationship:'extension'}],
  context_assessment:{metrics:{proposed_claims:2,cited_claims:2,proposed_links:2,supported_links:2,fully_supported_claims:2,unassessed_claims:0,accepted_claims:2,published_claims:2},evidence_gaps:[],writer_model:'fixture',assessed_at:now},prerequisites:[],sections:[section('opening','Introduction',1,1),section('regressional','Regressional Goodhart',2,3),section('conclusion','Conclusion',4,4)],claims:[],glossary:[],caveats:[],reading_path:[]};
analysis.sections[1].source_span = {start:anchor(2,5),end:anchor(3,12)};
const paper={id,metadata:{title:'A synthetic paper about noisy proxies',authors:['Test Author', 'Second Author', 'Third Author', 'Fourth Author', 'Fifth Author', 'Sixth Author'],year:2018,page_count:4},relative_path:'synthetic.pdf',status:{state:'ready'},analyzed_at:now,one_line_summary:analysis.thesis};
const root=path.resolve('dist');
const refreshes=[]; let sourceRequests=0; let analyzed=false; const questions=[];
let libraryPapers=[paper]; const paperRequests=[]; const analysisRequests=[];
let paperGate=null;
let pollingTest=false; let pollRequests=0;

const readingIndex={schema_version:2,text:'',pages:[],tokens:[],objects:{word:[],WORD:[],sentence:[],paragraph:[]},figures:[],gaps:[]};
for(let number=1;number<=4;number++) {
  const start=readingIndex.text.length;
  for(let j=0;j<22;j++) {
    const text=`Sentence ${j+1} on page ${number} describes the evidence and its limits.`;
    const paragraphStart=readingIndex.text.length;
    for(const match of text.matchAll(/\S+/g)) {
      const offset=paragraphStart+match.index;
      const token={start:offset,end:offset+match[0].length,text:match[0],page:number,provenance:'native',rects:[{x_min:48+match.index*6,y_min:73+j*28,x_max:48+(match.index+match[0].length)*6,y_max:87+j*28}]};
      readingIndex.tokens.push(token); readingIndex.objects.word.push({start:token.start,end:token.end}); readingIndex.objects.WORD.push({start:token.start,end:token.end});
    }
    readingIndex.text+=text;
    readingIndex.objects.sentence.push({start:paragraphStart,end:readingIndex.text.length});
    readingIndex.objects.paragraph.push({start:paragraphStart,end:readingIndex.text.length,kind:'body'});
    readingIndex.text+='\n\n';
  }
  readingIndex.pages.push({number,width:612,height:792,start,end:readingIndex.text.length,provenance:'native',confidence:null});
}

function selectionFixture() {
  // Courier's fixed advance makes native PDF.js character geometry measurable.
  // ToUnicode maps two font glyphs to an astral character and a combining cluster.
  const runs=[
    {text:'Measurement anti-noise proves useful.',x:48,y:72,paragraph:0},
    {text:'Second sentence follows. Third sentence closes.',x:48,y:100,paragraph:0},
    {text:'Different paragraph starts here.',x:48,y:160,paragraph:1},
    {text:'alpha beta',x:48,y:230,paragraph:2},
    {text:'gamma delta',x:360,y:230,paragraph:2},
    {text:'A😀e\u0301Z',encoded:'A~^Z',x:48,y:290,paragraph:3},
  ];
  const index={schema_version:3,text:'',pages:[],tokens:[],objects:{word:[],WORD:[],sentence:[],paragraph:[]},figures:[],gaps:[]};
  let paragraphStart=0;
  for(let i=0;i<runs.length;i++) {
    const run=runs[i]; const start=index.text.length;
    for(const match of run.text.matchAll(/\S+/gu)) {
      const prefix=run.text.slice(0,match.index);
      const graphemes=text=>Array.from(new Intl.Segmenter('en',{granularity:'grapheme'}).segment(text)).length;
      const x=run.x+graphemes(prefix)*7.2;
      index.tokens.push({start:start+match.index,end:start+match.index+match[0].length,text:match[0],page:1,provenance:'native',
        rects:[{x_min:x,x_max:x+graphemes(match[0])*7.2,y_min:run.y-10,y_max:run.y+2}]});
    }
    index.text+=run.text;
    if(runs[i+1]?.paragraph===run.paragraph)index.text+=' ';
    else {
      index.objects.paragraph.push({start:paragraphStart,end:index.text.length,kind:'body'});
      index.text+='\n\n'; paragraphStart=index.text.length;
    }
  }
  for(const match of index.text.matchAll(/[\p{L}\p{M}\p{N}_]+|[^\s\p{L}\p{M}\p{N}_]+/gu))index.objects.word.push({start:match.index,end:match.index+match[0].length});
  for(const match of index.text.matchAll(/\S+/gu))index.objects.WORD.push({start:match.index,end:match.index+match[0].length});
  for(const paragraph of index.objects.paragraph) {
    const text=index.text.slice(paragraph.start,paragraph.end);
    for(const match of text.matchAll(/[^.!?]+[.!?]|[^.!?]+$/gu)) {
      const leading=match[0].length-match[0].trimStart().length;
      index.objects.sentence.push({start:paragraph.start+match.index+leading,end:paragraph.start+match.index+match[0].trimEnd().length});
    }
  }
  index.pages.push({number:1,width:612,height:792,start:0,end:index.text.length,provenance:'native',confidence:null});
  const mappings=Array.from({length:95},(_,i)=>{
    const code=i+32;const unicode=code===126?'D83DDE00':code===94?'00650301':code.toString(16).padStart(4,'0');
    return `<${code.toString(16).padStart(2,'0')}> <${unicode}>`;
  }).join('\n');
  const cmap=`/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /FixtureUnicode def /CMapType 2 def\n1 begincodespacerange <00> <FF> endcodespacerange\n95 beginbfchar\n${mappings}\nendbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end`;
  const content=runs.map(run=>`BT /F1 12 Tf 1 0 0 1 ${run.x} ${792-run.y} Tm (${run.encoded??run.text}) Tj ET`).join('\n');
  const objects=['<< /Type /Catalog /Pages 2 0 R >>','<< /Type /Pages /Kids [5 0 R] /Count 1 >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Courier /ToUnicode 4 0 R >>',
    `<< /Length ${Buffer.byteLength(cmap)} >>\nstream\n${cmap}\nendstream`,
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 6 0 R >>',
    `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`];
  let source='%PDF-1.4\n';const offsets=[0];
  objects.forEach((object,i)=>{offsets.push(Buffer.byteLength(source));source+=`${i+1} 0 obj\n${object}\nendobj\n`;});
  const xref=Buffer.byteLength(source);
  source+=`xref\n0 ${objects.length+1}\n0000000000 65535 f \n`+offsets.slice(1).map(offset=>`${String(offset).padStart(10,'0')} 00000 n \n`).join('');
  source+=`trailer\n<< /Size ${objects.length+1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return {pdf:Buffer.from(source),index};
}
const preciseSelection=selectionFixture();
let usePreciseSelection=false;
const indexRequests=[];const abortedIndexRequests=[];
const initialIndexStarted=Promise.withResolvers();
const initialIndex=Promise.withResolvers();let indexGate=initialIndex.promise;
let failIndexRequests=0;

const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1280,height:800}});
  await page.addInitScript(()=>{window.copiedSource='';Object.defineProperty(navigator,'clipboard',{value:{writeText:async(text)=>{window.copiedSource=text;}}});});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  page.on('requestfailed',request=>{if(new URL(request.url()).pathname.endsWith('/reading-index'))abortedIndexRequests.push(request.failure()?.errorText);});
  await page.route('http://lysilogy.test/**', async route=>{
    const url=new URL(route.request().url()); const suffix=url.pathname.replace(/^\/api\/papers\/[^/]+/,'');
    if(url.pathname==='/api/library')return route.fulfill({json:{name:'Synthetic library',papers:libraryPapers}});
    if(url.pathname==='/api/queue')return route.fulfill({json:{jobs:[]}});
    if(/^\/api\/papers\/[^/]+$/.test(url.pathname)) {
      const selected=libraryPapers.find(item=>url.pathname.endsWith(item.id));
      paperRequests.push(selected?.id);
      if(pollingTest && selected?.id===id) {
        const number=++pollRequests;
        if(number>1) await new Promise(resolve=>setTimeout(resolve,1200));
        return route.fulfill({json:{paper:{...paper,metadata:{...paper.metadata,title:number===1?'Waiting on analysis':'Refreshed through overlapping polls'},status:{state:number===1?'extracting':'ready'}},analysis:number===1?null:analysis}});
      }
      if(paperGate!==null && selected?.id===id){
        const gate=paperGate; paperGate=null; gate.started.resolve(); await gate.finished.promise;
      }
      return route.fulfill({json:{paper:usePreciseSelection?{...selected,metadata:{...selected.metadata,page_count:1}}:selected,analysis:analyzed&&selected?.status.state==='ready'?analysis:null}});
    }
    if(suffix==='/analyze'){analysisRequests.push(route.request().postDataJSON());return route.fulfill({status:500,json:{message:'Unexpected analysis request'}});}
    if(suffix==='/clarify'){ questions.push(route.request().postDataJSON()); return route.fulfill({json:{answer:'The selected source explains measurement error.',limitation:null}}); }
    if(suffix==='/map')return route.fulfill({json:{layout,highlights:[]}});
    if(suffix==='/reading-index') {
      indexRequests.push({priority:route.request().headers()['x-reading-priority'],etag:route.request().headers()['if-none-match']});
      initialIndexStarted.resolve();
      if(indexGate!==null)await indexGate;
      if(failIndexRequests>0){failIndexRequests--;return route.fulfill({status:503,json:{message:'Index temporarily unavailable'}});}
      const etag=usePreciseSelection?'"precise-index-v3"':'"reading-index-v2"';
      const headers={etag,'cache-control':'private, max-age=2592000, must-revalidate'};
      if(route.request().headers()['if-none-match']===etag)return route.fulfill({status:304,headers});
      return route.fulfill({headers,json:usePreciseSelection?preciseSelection.index:readingIndex});
    }
    if(suffix==='/source'){ sourceRequests++; return route.fulfill({body:usePreciseSelection?preciseSelection.pdf:pdf,contentType:'application/pdf'}); }
    if(suffix==='/abstract/refresh'||suffix==='/context/refresh'||suffix==='/structure/refresh'){ refreshes.push(suffix);return route.fulfill({json:{paper,analysis:analyzed?analysis:null}}); }
    if(suffix==='/reader-tools')return route.fulfill({json:{jobs:[],references:[],supercuts:[]}});
    if(url.pathname.startsWith('/api/'))return route.fulfill({status:404,json:{message:`Unexpected fixture request: ${url.pathname}`}});
    const file=path.join(root,url.pathname==='/'?'index.html':url.pathname.slice(1));
    const contentType=/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.svg')?'image/svg+xml':'text/html';
    return route.fulfill({body:await readFile(file),contentType});
  });

  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await page.locator('.pdf-reader').waitFor();
  await page.waitForFunction(()=>document.querySelector('.pdf-canvas')?.dataset.rendered==='true');
  const expectReaderDefaults=async()=>{
    assert.equal(await page.locator('.app-shell.has-library').count(),0,'PDF entry keeps the library closed');
    assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit'),'height');
    await page.waitForFunction(()=>{
      const canvas=document.querySelector('.pdf-canvas');
      const viewport=document.querySelector('.pdf-viewport');
      return canvas?.dataset.rendered==='true'&&viewport&&canvas.getBoundingClientRect().height<=viewport.clientHeight;
    });
  };
  await expectReaderDefaults();
  await checkPdfFits(page);
  await page.keyboard.press('F1');
  await page.locator('.app-shell.has-library').waitFor();
  await page.keyboard.press('F1');
  await page.setViewportSize({width:900,height:800});
  await page.setViewportSize({width:1280,height:800});
  await expectReaderDefaults();
  await page.keyboard.press('W');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit'),'width','manual fitting still works');
  await page.keyboard.press('+');
  await initialIndexStarted.promise;
  assert.equal(indexRequests.length,1,'opening the PDF warms its index before any search');
  assert.equal(indexRequests[0].priority,'background');
  assert.equal(await page.locator('.pdf-source-tools').count(),0,'warming is quiet');
  await page.keyboard.press('/');
  const search=page.getByRole('textbox',{name:'Search paper with regular expression'});
  await search.fill('evidence');await search.press('Enter');
  await page.getByText('Indexing and searching source…',{exact:true}).waitFor();
  assert.equal(indexRequests.length,1,'search joins an already running warmup');
  await search.press('Escape');
  await page.keyboard.press(':');await page.getByRole('textbox',{name:'Command',exact:true}).fill('home');await page.keyboard.press('Enter');
  await page.locator('.home-page').waitFor();
  await page.locator('.paper-card').click();await page.locator('.pdf-reader').waitFor();
  await page.waitForFunction(()=>document.querySelector('.pdf-canvas')?.dataset.rendered==='true');
  await expectReaderDefaults();
  await page.evaluate(()=>new Promise(resolve=>requestIdleCallback(resolve,{timeout:1500})));
  assert.equal(indexRequests.length,1,'returning to the PDF shares its unfinished index request');
  const warmed=page.waitForResponse(response=>new URL(response.url()).pathname.endsWith('/reading-index'));
  indexGate=null;initialIndex.resolve();await warmed;
  assert.deepEqual(abortedIndexRequests,[],'leaving a reader does not abort its indexing request');
  assert.equal(await page.locator('.pdf-source-tools').count(),0,'completion does not reopen an old search');
  await page.keyboard.press('/');await search.fill('evidence');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('1 / 88'));
  assert.equal(indexRequests.length,1,'the warmed index is reused for search');
  await page.keyboard.press('n');await page.keyboard.press('n');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('3 / 88'));
  await page.keyboard.press('v');await page.keyboard.press('a');await page.keyboard.press('p');
  await page.waitForFunction(offset=>document.querySelector('.pdf-source-mark.is-visual')?.getAttribute('data-source-offset')===String(offset),readingIndex.objects.paragraph[2].start);
  const paragraph=await page.locator('.pdf-source-mark.is-visual').first().getAttribute('data-source-offset');
  assert.equal(Number(paragraph),readingIndex.objects.paragraph[2].start);
  await page.keyboard.press('y');
  await page.waitForFunction(()=>window.copiedSource==='Sentence 3 on page 1 describes the evidence and its limits.\n\n');
  assert.equal(await page.locator('.pdf-mode-note').count(),0);
  // A later visit validates the remembered body cheaply, then searches locally.
  const revalidation=page.waitForResponse(response=>new URL(response.url()).pathname.endsWith('/reading-index')&&response.status()===304);
  await page.keyboard.press(':');await page.getByRole('textbox',{name:'Command',exact:true}).fill('home');await page.keyboard.press('Enter');
  await page.locator('.home-page').waitFor();await page.locator('.paper-card').click();await revalidation;
  assert.equal(indexRequests.length,2);
  assert.equal(indexRequests[1].etag,'"reading-index-v2"');
  assert.equal(indexRequests[1].priority,'background');
  // A match on a different source page becomes visible in paged mode.
  await page.keyboard.press('/');await search.fill('Sentence 20 on page 3');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="3"] canvas')?.dataset.rendered==='true');
  await page.waitForFunction(()=>{const a=document.querySelector('.is-current')?.getBoundingClientRect();const b=document.querySelector('.pdf-viewport')?.getBoundingClientRect();return a&&b&&a.top>=b.top&&a.bottom<=b.bottom;});
  await page.keyboard.press('v');await page.keyboard.press('i');await page.keyboard.press('w');await page.keyboard.press('y');
  await page.waitForFunction(()=>window.copiedSource==='Sentence');
  // Invalid patterns preserve a working reader and report the parse failure.
  await page.keyboard.press('/');await search.fill('[');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-error')?.textContent.includes('regular expression'));
  await search.fill('(.+)+UNLIKELY_END');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-error')?.textContent.includes('1-second search limit'),{},{timeout:4000});
  await search.fill('unfindable-xyz');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('No matches'));
  // Escape clears the prompt, then q clears its status before leaving the reader.
  await search.press('Escape');await page.keyboard.press('q');
  assert.equal(await page.locator('.pdf-reader').count(),1);
  await page.waitForFunction(()=>document.querySelector('.pdf-source-tools')===null,{},{timeout:3000});
  // Continuous and horizontal modes reuse source coordinates and selected offsets.
  await page.keyboard.press('P');await page.keyboard.press('R');await page.keyboard.press('/');
  await search.fill('Sentence 12 on page 4');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="4"] canvas')?.dataset.rendered==='true');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('/Sentence 12 on page 4 · 1 / 1'));
  await page.keyboard.press('v');await page.keyboard.press('a');await page.keyboard.press('p');await page.keyboard.press('y');
  await page.waitForFunction(()=>window.copiedSource==='Sentence 12 on page 4 describes the evidence and its limits.\n\n',{},{timeout:3000});
  await page.screenshot({path:'/tmp/lysilogy-source-search.png'});
  // A native drag ending on another page starts its text object at the selection's first page.
  await page.keyboard.press('R');
  await page.waitForFunction(()=>document.querySelector('.pdf-reader')?.dataset.axis==='vertical');
  await page.evaluate(()=>new Promise(requestAnimationFrame));
  await page.locator('.pdf-viewport').evaluate(node=>{node.scrollTop=0;node.scrollLeft=0;});
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="1"] .pdf-text-layer')?.dataset.textReady==='true'
    || document.querySelector('[data-pdf-page="1"] [data-text-ready="true"]'));
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="2"] [data-text-ready="true"]'));
  await page.evaluate(()=>{
    const first=Array.from(document.querySelectorAll('[data-pdf-page="1"] .pdf-text-layer span')).find(node=>node.textContent.startsWith('Sentence 6 on page 1'));
    const last=Array.from(document.querySelectorAll('[data-pdf-page="2"] .pdf-text-layer span')).find(node=>node.textContent.startsWith('Sentence 3 on page 2'));
    const range=document.createRange();range.setStart(first.firstChild,0);range.setEnd(last.firstChild,last.firstChild.textContent.length);
    const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);
    document.dispatchEvent(new Event('selectionchange'));
    const target=document.querySelector('[data-pdf-page="2"] .pdf-page-surface');
    const box=target.getBoundingClientRect();target.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:box.left+10,clientY:box.top+100}));
  });
  await page.evaluate(()=>new Promise(requestAnimationFrame));
  await page.locator('.pdf-reader').focus();
  await page.keyboard.press('v');await page.keyboard.press('i');await page.keyboard.press('p');await page.keyboard.press('y');
  await page.waitForFunction(()=>window.copiedSource==='Sentence 6 on page 1 describes the evidence and its limits.');
  // A cropped section reports outside matches without leaking that source into its crop.
  analyzed=true;await page.reload();
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await page.locator('.section-boxes').waitFor();
  await page.keyboard.press('p');
  await page.waitForFunction(()=>document.querySelector('.pdf-reader .pdf-canvas')?.dataset.rendered==='true');
  await pressVisualFit(page,'W');
  await page.waitForTimeout(500);
  assert.equal(await page.locator('.glossary-view').count(),0,'gW must cancel the delayed glossary');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit-bounds'),'visual');
  await page.keyboard.press('g');
  await page.locator('.glossary-view').waitFor();
  await page.getByRole('button',{name:'Text',exact:true}).click();
  await page.keyboard.press('H');
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await page.keyboard.press('F1');
  await page.locator('.app-shell.has-library').waitFor();
  await page.locator('.section-boxes button[data-section-id="regressional"]').first().click();
  await page.locator('.section-focus').waitFor();
  await expectReaderDefaults();
  await checkPdfFits(page,{cropped:true});
  await page.keyboard.press('/');await search.fill('Sentence 1 on page 1');await search.press('Enter');
  await page.getByRole('button',{name:'Open match in full paper'}).waitFor();
  assert.equal(await page.locator('.section-focus [data-pdf-page="1"]').count(),0);
  await page.getByRole('button',{name:'Open match in full paper'}).click();
  await page.waitForFunction(()=>document.querySelector('.section-focus')===null);
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="1"] canvas')?.dataset.rendered==='true');
  await expectReaderDefaults();
  assert.match(await page.locator('.pdf-source-status').innerText(),/Sentence 1 on page 1/);
  // Clipboard denial leaves the exact source passage available for manual copy.
  await page.evaluate(()=>{navigator.clipboard.writeText=async()=>{throw Error('Denied');};});
  await page.keyboard.press('v');await page.keyboard.press('i');await page.keyboard.press('p');await page.keyboard.press('y');
  await page.getByRole('textbox',{name:'Text to copy manually'}).waitFor();
  assert.equal(await page.getByRole('textbox',{name:'Text to copy manually'}).inputValue(),'Sentence 1 on page 1 describes the evidence and its limits.');
  await page.keyboard.press('Escape');await page.keyboard.press('q');await page.keyboard.press('q');
  // Root routing from the map opens this PDF search instead of the library menu.
  await page.keyboard.press('T');
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await page.keyboard.press('/');await search.waitFor();

  // A separate real PDF provides character mappings and predictable line geometry.
  // Finish the preceding reader's navigation before injecting a warmup failure.
  // Otherwise its pending request can consume the failure intended for the new reader.
  await page.goto('about:blank');
  usePreciseSelection=true;analyzed=false;failIndexRequests=1;
  const failedWarmup=page.waitForResponse(response=>new URL(response.url()).pathname.endsWith('/reading-index')&&response.status()===503);
  await page.goto(`http://lysilogy.test/#paper=${id}`);await failedWarmup;
  assert.equal(await page.locator('.pdf-source-tools').count(),0,'background indexing errors stay out of the reader');
  const beforeRetry=indexRequests.length;
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="1"] [data-text-ready="true"]'));
  assert.match(await page.locator('.pdf-text-layer').innerText(),/A😀e\u0301Z/,'PDF.js must expose the fixture’s real Unicode mapping');
  const precise=preciseSelection.index;
  const land=async(pattern,offset)=>{
    await page.keyboard.press('/');await search.fill(pattern);await search.press('Enter');
    await page.waitForFunction(({pattern,offset})=>document.querySelector('.pdf-source-status')?.textContent.includes(`/${pattern} · 1 / 1`)
      &&document.querySelector('.pdf-source-mark.is-current')?.getAttribute('data-source-offset')===String(offset),{pattern,offset});
  };
  const visual=async(keys='')=>{
    await page.keyboard.press('v');
    await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('VISUAL'));
    if(keys)await page.keyboard.type(keys);
  };
  const yank=async(expected)=>{
    await page.evaluate(()=>{window.copiedSource=null;});await page.keyboard.press('y');
    await page.waitForFunction(expected=>window.copiedSource===expected,expected,{timeout:3000});
  };
  const marks=async()=>page.locator('.pdf-source-mark.is-visual').evaluateAll(nodes=>nodes.map(node=>({
    start:Number(node.dataset.sourceOffset),left:parseFloat(node.style.left)*612/100,
    width:parseFloat(node.style.width)*612/100,top:parseFloat(node.style.top)*792/100,
  })).sort((a,b)=>a.top-b.top||a.left-b.left));

  await land('sure',3);
  await pressVisualFit(page,'W');
  assert.equal(indexRequests.length,beforeRetry+1,'a real search retries failed warming');
  assert.equal(indexRequests.at(-1).priority,'interactive');
  await visual('ll');
  await page.waitForFunction(()=>{
    const mark=document.querySelector('.pdf-source-mark.is-visual');
    return mark?.getAttribute('data-geometry')==='native'&&mark.getAttribute('data-source-end')==='6';
  });
  const partial=await marks();
  assert.equal(partial.length,1);
  assert.equal(partial[0].start,3,'the mark starts at the selected character, not the word boundary');
  assert.ok(partial[0].width<11*7.2/2,'a three-character selection must not shade all of Measurement');
  const alignment=await page.evaluate(()=>{
    const span=Array.from(document.querySelectorAll('.pdf-text-layer span')).find(node=>node.textContent.startsWith('Measurement'));
    const range=document.createRange();range.setStart(span.firstChild,3);range.setEnd(span.firstChild,6);
    const expected=range.getBoundingClientRect(),actual=document.querySelector('.pdf-source-mark.is-visual').getBoundingClientRect();
    return {left:Math.abs(actual.left-expected.left),width:Math.abs(actual.width-expected.width)};
  });
  assert.ok(alignment.left<1.5&&alignment.width<1.5,`selection must follow the native glyph range: ${JSON.stringify(alignment)}`);
  await yank('sur');

  const compound=precise.text.indexOf('anti-noise');
  await land('anti-noise',compound);await visual('e');await yank('anti');
  await land('anti-noise',compound);await visual('E');
  assert.equal(await page.locator('.notes-panel').count(),0,'Visual E is a word-end motion, not the global Notes shortcut');
  await yank('anti-noise');
  await land('anti-noise',compound);await visual('h');
  await page.waitForFunction(offset=>document.querySelector('.pdf-source-mark.is-cursor')?.getAttribute('data-source-offset')===String(offset),compound-1);
  const spaceWidth=await page.locator('.pdf-source-mark.is-cursor').evaluate(node=>parseFloat(node.style.width)*612/100);
  assert.ok(spaceWidth>0&&spaceWidth<8,'the cursor on an inter-word space must have character-sized geometry');
  await yank(' a');
  await land('Measurement',0);await visual(')');
  await yank(precise.text.slice(0,precise.text.indexOf('Second')+1));
  await land('Third',precise.text.indexOf('Third'));await visual('(');
  await yank(precise.text.slice(precise.text.indexOf('Second'),precise.text.indexOf('Third')+1));

  const firstParagraph=precise.objects.paragraph[0],secondParagraph=precise.objects.paragraph[1];
  await land('Second',precise.text.indexOf('Second'));await visual('ip');
  await yank(precise.text.slice(firstParagraph.start,firstParagraph.end));
  await land('Second',precise.text.indexOf('Second'));await visual('ap');
  await yank(precise.text.slice(firstParagraph.start,firstParagraph.end)+'\n\n');
  await land('Second',precise.text.indexOf('Second'));await visual('apj');
  await page.waitForFunction(({start,end})=>{
    const offset=Number(document.querySelector('.pdf-source-mark.is-cursor')?.getAttribute('data-source-offset'));
    return offset>=start&&offset<end;
  },secondParagraph);
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-source-visual'),'true','j after vap stays in Visual mode');
  await yank(precise.text.slice(firstParagraph.start,secondParagraph.end));
  await land('Measurement',0);await visual('2j');
  await page.waitForFunction(offset=>document.querySelector('.pdf-source-mark.is-cursor')?.getAttribute('data-source-offset')===String(offset),secondParagraph.start);
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-source-visual'),'true','2j advances two lines within Visual mode');
  assert.equal(await page.locator('.pdf-viewport.is-spread').count(),0,'a motion count must not toggle two-page reading');
  await yank(precise.text.slice(0,secondParagraph.start+1));
  await land('Different',secondParagraph.start);await visual('ip');
  await yank(precise.text.slice(secondParagraph.start,secondParagraph.end));
  await land('Different',secondParagraph.start);await visual('ap');
  await yank(precise.text.slice(secondParagraph.start,secondParagraph.end)+'\n\n');

  const unicode=precise.text.indexOf('A😀e\u0301Z');
  await land('A😀e\u0301Z',unicode);await visual('lll');await yank('A😀e\u0301Z');
  await land('Z',unicode+5);await visual('hh');await yank('😀e\u0301Z');
  await land('😀',unicode+1);await visual();await yank('😀');
  await land('e\u0301',unicode+3);await visual('h');await yank('😀e\u0301');

  const columns=precise.objects.paragraph[2];
  await land('alpha beta',columns.start);await visual('ip');
  await page.waitForFunction(()=>{
    const marks=Array.from(document.querySelectorAll('.pdf-source-mark.is-visual'));
    return marks.length===2&&marks.every(mark=>mark.getAttribute('data-geometry')==='native');
  });
  const lines=await marks();
  assert.equal(lines.length,2,'each contiguous line is one highlight, with separate boxes for the two columns');
  assert.ok(lines[0].left<49&&lines[0].left+lines[0].width>119,'left highlight includes both words and their intervening space');
  assert.ok(lines[1].left>359&&lines[1].left+lines[1].width>438,'right highlight includes both words and their intervening space');
  assert.ok(lines.every(box=>box.left>200||box.left+box.width<200),'selection must not paint the column gutter');
  await page.screenshot({path:'/tmp/lysilogy-source-selection.png'});
  await yank(precise.text.slice(columns.start,columns.end));

  usePreciseSelection=false;
  await checkVisualFitPages(page,`http://lysilogy.test/api/papers/${id}/source`);

  assert.deepEqual(errors,[]);
  console.log('PASS gH/gW visual fitting, prefix cancellation, glossary fallback, figures/scans/rotation/blank pages, spread and continuous fitting, quiet background indexing, request survival across visits, shared searches, conditional cache reuse, retry after background failure, source regex search, cross-page navigation, word-end/sentence motions, Unicode character yanks, distinct paragraphs, native partial-word geometry, merged line highlights without column bridges, errors, q/Escape, continuous horizontal reading');
} finally {await browser.close();}
