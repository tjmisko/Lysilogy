import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

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

const readingIndex={schema_version:1,text:'',pages:[],tokens:[],objects:{word:[],WORD:[],sentence:[],paragraph:[]},figures:[],gaps:[]};
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

const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1280,height:800}});
  await page.addInitScript(()=>{window.copiedSource='';Object.defineProperty(navigator,'clipboard',{value:{writeText:async(text)=>{window.copiedSource=text;}}});});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
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
      return route.fulfill({json:{paper:selected,analysis:analyzed&&selected?.status.state==='ready'?analysis:null}});
    }
    if(suffix==='/analyze'){analysisRequests.push(route.request().postDataJSON());return route.fulfill({status:500,json:{message:'Unexpected analysis request'}});}
    if(suffix==='/clarify'){ questions.push(route.request().postDataJSON()); return route.fulfill({json:{answer:'The selected source explains measurement error.',limitation:null}}); }
    if(suffix==='/map')return route.fulfill({json:{layout,highlights:[]}});
    if(suffix==='/reading-index')return route.fulfill({json:readingIndex});
    if(suffix==='/source'){ sourceRequests++; return route.fulfill({body:pdf,contentType:'application/pdf'}); }
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
  await page.keyboard.press('/');
  const search=page.getByRole('textbox',{name:'Search paper with regular expression'});
  await search.fill('evidence');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('1 / 88'));
  await page.keyboard.press('n');await page.keyboard.press('n');
  await page.waitForFunction(()=>document.querySelector('.pdf-source-status')?.textContent.includes('3 / 88'));
  await page.keyboard.press('v');await page.keyboard.press('a');await page.keyboard.press('p');
  await page.locator('.pdf-source-mark.is-visual').first().waitFor();
  const paragraph=await page.locator('.pdf-source-mark.is-visual').first().getAttribute('data-source-offset');
  assert.equal(Number(paragraph),readingIndex.objects.paragraph[2].start);
  await page.keyboard.press('y');
  await page.waitForFunction(()=>window.copiedSource==='Sentence 3 on page 1 describes the evidence and its limits.\n\n');
  assert.equal(await page.locator('.pdf-mode-note').count(),0);
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
  await page.locator('.section-boxes button[data-section-id="regressional"]').first().click();
  await page.locator('.section-focus').waitFor();
  await page.keyboard.press('/');await search.fill('Sentence 1 on page 1');await search.press('Enter');
  await page.getByRole('button',{name:'Open match in full paper'}).waitFor();
  assert.equal(await page.locator('.section-focus [data-pdf-page="1"]').count(),0);
  await page.getByRole('button',{name:'Open match in full paper'}).click();
  await page.waitForFunction(()=>document.querySelector('.section-focus')===null);
  await page.waitForFunction(()=>document.querySelector('[data-pdf-page="1"] canvas')?.dataset.rendered==='true');
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

  assert.deepEqual(errors,[]);
  console.log('PASS source regex search, cross-page navigation, visual text objects, exact paragraph yank, errors, q/Escape, continuous horizontal reading');
} finally {await browser.close();}
