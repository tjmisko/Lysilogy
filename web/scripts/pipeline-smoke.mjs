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
const paper={id,metadata:{title:'A synthetic paper about noisy proxies',authors:['Test Author'],year:2018,page_count:4},relative_path:'synthetic.pdf',status:{state:'ready'},analyzed_at:now,one_line_summary:analysis.thesis};
const root=path.resolve('dist');
const refreshes=[]; let sourceRequests=0;
const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1600,height:1000}});
  const errors=[]; page.on('pageerror',error=>errors.push(error.message));
  await page.route('http://lysilogy.test/**', async route=>{
    const url=new URL(route.request().url()); const suffix=url.pathname.replace(`/api/papers/${id}`,'');
    if(url.pathname==='/api/library')return route.fulfill({json:{name:'Synthetic library',papers:[paper]}});
    if(url.pathname==='/api/queue')return route.fulfill({json:{jobs:[]}});
    if(url.pathname===`/api/papers/${id}`)return route.fulfill({json:{paper,analysis}});
    if(suffix==='/map')return route.fulfill({json:{layout,highlights:[]}});
    if(suffix==='/source'){ sourceRequests++; return route.fulfill({body:pdf,contentType:'application/pdf'}); }
    if(suffix==='/abstract/refresh'||suffix==='/context/refresh'){ refreshes.push(suffix);return route.fulfill({json:{paper,analysis}}); }
    if(suffix==='/reader-tools')return route.fulfill({json:{jobs:[],references:[],supercuts:[]}});
    if(url.pathname.startsWith('/api/'))return route.fulfill({status:404,json:{message:`Unexpected fixture request: ${url.pathname}`}});
    const file=path.join(root,url.pathname==='/'?'index.html':url.pathname.slice(1));
    const contentType=/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.svg')?'image/svg+xml':'text/html';
    return route.fulfill({body:await readFile(file),contentType});
  });
  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await page.locator('[data-context-kind="before"]').waitFor();
  assert.match(await page.locator('[data-context-kind="before"]').innerText(),/Prior research/);
  assert.doesNotMatch(await page.locator('[data-context-kind="before"]').innerText(),/subsequent experiment|MERGED LEGACY/);
  assert.match(await page.locator('[data-context-kind="after"]').innerText(),/subsequent experiment/);
  assert.equal(await page.locator('[data-context-kind="before"] a').count(),1);
  await page.getByRole('button',{name:'Refresh abstract',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('.authored-abstract button')?.disabled);
  await page.getByRole('button',{name:'Research before and after',exact:true}).click();
  await page.getByRole('button',{name:'Research before and after',exact:true}).waitFor();
  assert.deepEqual(refreshes,['/abstract/refresh','/context/refresh']);
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  const region=page.locator('.section-boxes button[data-section-id="regressional"]').first();
  await region.waitFor();
  await page.waitForFunction(()=>document.querySelector('.source-page[data-page="2"] canvas')?.dataset.rendered === 'true');
  const columns=await page.locator('.page-grid-zoom output').innerText();
  assert.equal(await page.locator('.app-shell.has-library').count(),1);
  await page.screenshot({path:'/tmp/lysilogy-pipeline-overview.png'});
  await region.click();
  await page.locator('.section-focus').waitFor();
  await page.waitForFunction(()=>document.querySelectorAll('.section-focus [data-text-ready="true"]').length>=1);
  assert.deepEqual(await page.locator('.section-focus [data-pdf-page]').evaluateAll(nodes=>nodes.map(n=>Number(n.dataset.pdfPage))),[2,3]);
  assert.equal(sourceRequests,1,'the map and focused reader must share the PDF');
  assert.equal(await page.locator('.paper-position .is-selected').count(),2);
  const source=page.locator('.section-source-scroll'); const digest=page.locator('.section-digest-slot');
  const [left,right]=await Promise.all([source.boundingBox(),digest.boundingBox()]);
  assert.ok(left.x+left.width<=right.x+1,'source and digest must be beside one another');
  const before=await source.evaluate(node=>node.scrollTop);
  await source.focus(); await page.keyboard.press('PageDown');
  assert.ok(await source.evaluate(node=>node.scrollTop)>before,'keyboard must scroll the source pane');
  assert.equal(await page.locator('.digest-scroll').evaluate(node=>node.scrollTop),0);
  await page.getByLabel('PDF page 2, in selected section').click();
  await page.waitForFunction(()=>document.querySelector('.section-source-scroll').scrollTop<150);
  await page.waitForTimeout(500);
  await page.screenshot({path:'/tmp/lysilogy-pipeline-focus.png'});
  await page.evaluate(()=>{
    const span=document.querySelector('.section-focus .pdf-text-layer span');
    const range=document.createRange();range.selectNodeContents(span);
    const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);
    document.dispatchEvent(new Event('selectionchange'));
  });
  await page.getByRole('toolbar',{name:'Selected PDF text'}).waitFor();
  await page.getByRole('button',{name:'Clear selection'}).click();
  await page.getByLabel('Selected section',{exact:true}).selectOption('2');
  await page.waitForFunction(()=>document.querySelector('.section-focus [data-pdf-page]')?.dataset.pdfPage==='4');
  assert.equal(await page.locator('.section-focus [data-pdf-page]').count(),1);
  await page.getByLabel('Selected section',{exact:true}).selectOption('1');
  await page.getByRole('button',{name:'← Whole paper',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.section-focus')===null);
  assert.equal(await page.locator('.page-grid-zoom output').innerText(),columns);
  await page.waitForFunction(()=>document.activeElement?.dataset.sectionId==='regressional');
  assert.equal(await page.locator('.app-shell.has-library').count(),1);
  await region.click();
  await page.getByRole('button',{name:'Abstract',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.app-shell.has-library')!==null);
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await region.click();
  await page.getByLabel('PDF page 3, in selected section').click();
  await page.getByRole('button',{name:'Open full paper ↗',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.text-view [data-pdf-page]')?.dataset.pdfPage==='3');
  await page.getByRole('button',{name:'One page',exact:true}).click();
  await page.waitForFunction(()=>document.querySelectorAll('.text-view [data-text-ready="true"]').length===2);
  assert.deepEqual(await page.locator('.text-view [data-pdf-page]').evaluateAll(nodes=>nodes.map(n=>Number(n.dataset.pdfPage))),[3,4]);
  await page.getByRole('button',{name:'Prev',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.text-view [data-pdf-page]')?.dataset.pdfPage==='1');
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await region.waitFor();
  await page.setViewportSize({width:390,height:844});
  await page.emulateMedia({reducedMotion:'reduce'});
  await region.click();
  await page.getByRole('button',{name:'Section digest',exact:true}).click();
  assert.equal(await source.isVisible(),false);
  assert.equal(await digest.isVisible(),true);
  await page.getByRole('button',{name:'Source pages',exact:true}).click();
  assert.equal(await source.isVisible(),true);
  await page.waitForTimeout(600);
  assert.equal(await page.locator('.page-transition-snapshot').count(),0);
  await page.screenshot({path:'/tmp/lysilogy-pipeline-mobile.png'});
  await source.focus(); await page.keyboard.press('Escape');
  await page.waitForFunction(()=>document.querySelector('.section-focus')===null);
  assert.deepEqual(errors,[]);
  console.log('Pipeline smoke passed: distinct cited context, selective refresh, scoped source pages, shared PDF loading, selection, keyboard scrolling, map restoration, mobile panes, reduced motion.');
} finally { await browser.close(); }
