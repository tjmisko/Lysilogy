import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import ts from 'typescript';

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
const refreshes=[]; let sourceRequests=0; let analyzed=true; const questions=[];
let libraryPapers=[paper]; const paperRequests=[]; const analysisRequests=[];
let paperGate=null;
let pollingTest=false; let pollRequests=0;
const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1600,height:1000}});
  const errors=[]; page.on('pageerror',error=>errors.push(error.message));
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
    if(suffix==='/source'){ sourceRequests++; return route.fulfill({body:pdf,contentType:'application/pdf'}); }
    if(suffix==='/abstract/refresh'||suffix==='/context/refresh'||suffix==='/structure/refresh'){ refreshes.push(suffix);return route.fulfill({json:{paper,analysis:analyzed?analysis:null}}); }
    if(suffix==='/reader-tools')return route.fulfill({json:{jobs:[],references:[],supercuts:[]}});
    if(url.pathname.startsWith('/api/'))return route.fulfill({status:404,json:{message:`Unexpected fixture request: ${url.pathname}`}});
    const file=path.join(root,url.pathname==='/'?'index.html':url.pathname.slice(1));
    const contentType=/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.svg')?'image/svg+xml':'text/html';
    return route.fulfill({body:await readFile(file),contentType});
  });
  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await page.locator('[data-context-kind="before"]').waitFor();
  assert.equal(await page.locator('.topbar').getByRole('button',{name:'Analyze',exact:true}).count(),0);
  assert.match(await page.locator('.current-paper-label').innerText(),/Test Author.*2018.*noisy proxies/);
  assert.match(await page.locator('.paper-byline').innerText(),/Sixth Author/);
  assert.equal(await page.locator('.view-introduction').count(),0);
  assert.equal(await page.locator('.paper-kicker').count(),0);
  assert.equal(await page.locator('.authored-abstract .eyebrow').count(),0);
  assert.equal(await page.locator('.authored-abstract > p').evaluate(n=>getComputedStyle(n).textAlign),'justify');
  assert.equal(await page.locator('[data-context-kind="before"] ul > li').count(),1);
  assert.match(await page.locator('[data-context-kind="before"]').innerText(),/Prior research/);
  assert.doesNotMatch(await page.locator('[data-context-kind="before"]').innerText(),/subsequent experiment|MERGED LEGACY/);
  assert.match(await page.locator('[data-context-kind="after"]').innerText(),/subsequent experiment/);
  assert.equal(await page.locator('[data-context-kind="before"] a').count(),1);
  await page.getByRole('button',{name:'Refresh abstract',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('.authored-abstract button')?.disabled);
  await page.getByRole('button',{name:'Research before and after',exact:true}).click();
  await page.getByRole('button',{name:'Research before and after',exact:true}).waitFor();
  assert.deepEqual(refreshes,['/abstract/refresh','/context/refresh']);
  await page.waitForFunction(()=>!document.querySelector('.context-refresh')?.disabled);
  await page.keyboard.press(':');
  await page.getByRole('textbox',{name:'Command',exact:true}).fill('refresh-structure');
  await page.keyboard.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.notice-toast')?.textContent?.includes('Section map refreshed') || document.body.textContent.includes('Section map refreshed.'));
  assert.deepEqual(refreshes,['/abstract/refresh','/context/refresh','/structure/refresh']);
  assert.match(await page.locator('[data-context-kind="before"]').innerText(),/Prior research/);
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  const region=page.locator('.section-boxes button[data-section-id="regressional"]').first();
  await region.waitFor();
  await page.keyboard.press('ArrowRight');
  await page.waitForFunction(()=>document.activeElement?.dataset.sectionId==='regressional');
  assert.equal(await region.getAttribute('class'),'is-verified is-active');
  await page.keyboard.press('ArrowLeft');
  await page.waitForFunction(()=>document.activeElement?.dataset.sectionId==='opening');
  await page.waitForFunction(()=>document.querySelector('.source-page[data-page="2"] canvas')?.dataset.rendered === 'true');
  const columns=await page.locator('.page-grid-zoom output').innerText();
  assert.equal(await page.locator('.app-shell.has-library').count(),1);
  await page.screenshot({path:'/tmp/lysilogy-pipeline-overview.png'});
  const escapeFrom = async (name, prepare) => {
    await region.click();
    await page.locator('.section-focus').waitFor();
    await prepare();
    await page.keyboard.press('Escape');
    await page.waitForFunction(()=>document.querySelector('.section-focus')===null);
    await page.waitForFunction(()=>document.activeElement?.dataset.sectionId==='regressional');
    assert.equal(await page.locator('.app-shell.has-library').count(),1,`${name}: library should be restored`);
  };
  await escapeFrom('source', () => page.locator('.section-source-scroll').focus());
  await escapeFrom('digest', () => page.locator('.digest-fragment').first().focus());
  await escapeFrom('source page selector', () => page.getByLabel('Source page',{exact:true}).focus());
  await escapeFrom('section selector', () => page.getByLabel('Selected section',{exact:true}).focus());
  await escapeFrom('digest selection', async () => {
    await page.locator('.digest-fragment').first().focus();
    await page.keyboard.press('v');
    await page.locator('.is-visual-selected').first().waitFor();
  });
  await escapeFrom('inline question', async () => {
    await page.locator('.digest-fragment').first().focus();
    await page.keyboard.press('c');
    await page.locator('.clarify-composer textarea').waitFor();
    await page.locator('.clarify-composer textarea').fill('Explain the measurement error.');
  });
  await escapeFrom('body after focus loss', () => page.evaluate(()=>document.activeElement?.blur()));
  await region.click();
  await page.locator('.digest-fragment').first().focus();
  await page.keyboard.press('q');
  await page.getByRole('dialog').waitFor();
  await page.keyboard.press('Escape');
  await page.waitForFunction(()=>document.querySelector('[role="dialog"]')===null);
  assert.equal(await page.locator('.section-focus').count(),1,'queue Escape must leave the section open');
  await page.keyboard.press('Escape');
  await page.waitForFunction(()=>document.querySelector('.section-focus')===null);
  await page.waitForFunction(()=>document.activeElement?.dataset.sectionId==='regressional');
  await region.click();
  await page.locator('.section-focus').waitFor();
  await page.waitForFunction(()=>document.querySelectorAll('.section-focus [data-text-ready="true"]').length>=1);
  assert.deepEqual(await page.locator('.section-focus [data-pdf-page]').evaluateAll(nodes=>nodes.map(n=>Number(n.dataset.pdfPage))),[2,3]);
  assert.equal(sourceRequests,1,'the map and focused reader must share the PDF');
  assert.equal(await page.locator('.paper-position, .section-focus-header, .section-text-marks').count(),0);
  assert.equal(await page.locator('.section-focus [data-section-crop="true"]').count(),2);
  const source=page.locator('.section-source-scroll'); const viewport=source.locator('.pdf-viewport'); const digest=page.locator('.section-digest-slot');
  const [left,right]=await Promise.all([source.boundingBox(),digest.boundingBox()]);
  assert.ok(left.x+left.width<=right.x+1,'source and digest must be beside one another');
  assert.ok(right.width>=400 && right.width<=520 && right.width<left.width,'digest should be readable while leaving most width for source');
  const appbar = await page.locator('.topbar').boundingBox();
  assert.equal(right.y,appbar.y+appbar.height,'digest must begin immediately below app bar');
  assert.equal(right.y+right.height,1000,'digest must reach the bottom of the window');
  await page.waitForFunction(()=>document.querySelectorAll('.section-focus [data-text-ready="true"]').length===2);
  const croppedText=await page.locator('.section-focus .pdf-text-layer').evaluateAll(layers=>layers.map(layer=>{
    const range=document.createRange();range.selectNodeContents(layer);
    const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);
    const text=selection.toString();selection.removeAllRanges();return text;
  }));
  assert.match(croppedText[0],/Sentence 6 on page 2/);
  assert.match(croppedText[0],/Sentence 22 on page 2/);
  assert.doesNotMatch(croppedText[0],/Source page|Sentence [1-5] on page 2/);
  assert.match(croppedText[1],/Sentence 1 on page 3/);
  assert.match(croppedText[1],/Sentence 13 on page 3/);
  assert.doesNotMatch(croppedText[1],/Source page|Sentence (1[4-9]|2[0-2]) on page 3/);
  const surface=page.locator('.section-focus .pdf-page-surface').first();
  await surface.evaluate(node=>{node.scrollTop=10000;node.scrollLeft=10000;});
  assert.equal(await surface.evaluate(node=>node.scrollTop+node.scrollLeft),0,'the clipped page itself must not scroll');
  const before=await viewport.evaluate(node=>node.scrollTop);
  await source.focus(); await page.keyboard.press('PageDown');
  assert.ok(await viewport.evaluate(node=>node.scrollTop)>before,'keyboard must scroll the source pane');
  assert.equal(await page.locator('.digest-scroll').evaluate(node=>node.scrollTop),0);
  await page.getByLabel('Source page',{exact:true}).selectOption('3');
  await page.getByLabel('Source page',{exact:true}).selectOption('2');
  await page.waitForFunction(()=>document.querySelector('.section-source-scroll .pdf-viewport').scrollTop<150);
  await page.waitForTimeout(500);
  await page.screenshot({path:'/tmp/lysilogy-pipeline-focus.png'});
  await page.setViewportSize({width:1280,height:650});
  await page.waitForFunction(()=>document.querySelectorAll('.section-focus [data-text-ready="true"]').length===2
    && document.querySelector('.section-focus canvas')?.getBoundingClientRect().width<950);
  await page.screenshot({path:'/tmp/lysilogy-pipeline-focus-compact.png'});
  const compactDigest=await digest.boundingBox();
  assert.equal(compactDigest.y+compactDigest.height,650);
  assert.ok(compactDigest.width>=400 && compactDigest.width<=450);
  assert.ok((await source.boundingBox()).width>=800);
  await page.setViewportSize({width:1600,height:1000});
  await page.waitForFunction(()=>document.querySelectorAll('.section-focus [data-text-ready="true"]').length===2
    && document.querySelector('.section-focus canvas')?.getBoundingClientRect().width>1000);
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
  await page.getByRole('button',{name:'← Map',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.section-focus')===null);
  assert.equal(await page.locator('.page-grid-zoom output').innerText(),columns);
  await page.waitForFunction(()=>document.activeElement?.dataset.sectionId==='regressional');
  assert.equal(await page.locator('.app-shell.has-library').count(),1);
  await region.click();
  await page.getByRole('button',{name:'Abstract',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.app-shell.has-library')!==null);
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await region.click();
  await page.getByLabel('Source page',{exact:true}).selectOption('3');
  await page.getByRole('button',{name:'Open full paper ↗',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.text-view [data-pdf-page]')?.dataset.pdfPage==='3');
  await page.getByRole('button',{name:'One page',exact:true}).click();
  await page.waitForFunction(()=>document.querySelectorAll('.text-view [data-text-ready="true"]').length===2);
  assert.deepEqual(await page.locator('.text-view [data-pdf-page]').evaluateAll(nodes=>nodes.map(n=>Number(n.dataset.pdfPage))),[3,4]);
  await page.getByRole('button',{name:'Prev',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.text-view [data-pdf-page]')?.dataset.pdfPage==='1');
  await page.keyboard.press('H');
  await page.waitForFunction(()=>document.querySelector('.text-view .pdf-reader')?.dataset.fit==='height');
  await page.waitForFunction(()=>document.querySelector('.text-view canvas')?.getBoundingClientRect().height < document.querySelector('.text-view .pdf-viewport').clientHeight);
  await page.keyboard.press('W');
  await page.waitForFunction(()=>document.querySelector('.text-view .pdf-reader')?.dataset.fit==='width');
  await page.keyboard.press('P');
  await page.waitForFunction(()=>document.querySelectorAll('.text-view [data-pdf-page]').length===4);
  const fullViewport=page.locator('.text-view .pdf-viewport');
  assert.ok(await fullViewport.evaluate(node=>node.scrollHeight>node.clientHeight));
  await page.keyboard.press('R');
  await page.waitForFunction(()=>document.querySelector('.text-view .pdf-reader')?.dataset.axis==='horizontal');
  assert.ok(await fullViewport.evaluate(node=>node.scrollWidth>node.clientWidth));
  await fullViewport.hover(); await page.mouse.wheel(0,500);
  await page.waitForFunction(()=>document.querySelector('.text-view .pdf-viewport').scrollLeft>100);
  await page.keyboard.press('l');
  await page.keyboard.press('R');
  await page.waitForFunction(()=>document.querySelector('.text-view .pdf-reader')?.dataset.axis==='vertical');
  await page.keyboard.press('P');
  await page.waitForFunction(()=>document.querySelector('.text-view .pdf-reader')?.dataset.flow==='paged');
  assert.ok(await page.locator('.text-view [data-pdf-page]').count()<=2);
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await region.waitFor();
  await page.setViewportSize({width:390,height:844});
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.waitForTimeout(350);
  await region.click();
  await page.locator('.section-focus').waitFor();
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
  // Large author lists keep the header compact and reveal every full name.
  paper.metadata.authors=Array.from({length:24},(_,i)=>`Researcher ${i+1} Fullname`);
  await page.setViewportSize({width:1280,height:650});
  await page.reload();
  await page.getByRole('button',{name:'Abstract',exact:true}).click();
  await page.locator('.paper-authors summary').waitFor();
  assert.equal(await page.locator('.paper-authors').getAttribute('open'),null);
  await page.locator('.paper-authors summary').click();
  assert.equal(await page.getByRole('list',{name:'Full author list'}).locator('li').count(),24);
  assert.match(await page.locator('.paper-authors').innerText(),/Researcher 24 Fullname/);
  await page.setViewportSize({width:390,height:844});
  assert.ok(await page.locator('.paper-heading').evaluate(n=>n.scrollWidth<=n.clientWidth+1));
  await page.screenshot({path:'/tmp/lysilogy-long-authors-mobile.png'});
  // Unanalyzed papers open straight into source reading, with one analysis action.
  analyzed=false;
  await page.setViewportSize({width:1280,height:650});
  await page.reload();
  await page.waitForFunction(()=>document.querySelector('.text-view [data-text-ready="true"]'));
  assert.equal(await page.locator('.view-switch').getByRole('button',{name:'Abstract',exact:true}).count(),0);
  assert.equal(await page.getByRole('button',{name:'Analyze',exact:true}).count(),1);
  await page.keyboard.press('q');
  await page.getByRole('dialog',{name:'Processing queue'}).waitFor();
  await page.keyboard.press('q');
  await page.getByRole('dialog',{name:'Processing queue'}).waitFor({state:'hidden'});
  await page.evaluate(()=>{
    const span=document.querySelector('.text-view .pdf-text-layer span');
    const range=document.createRange(); range.selectNodeContents(span);
    const selection=window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
    document.dispatchEvent(new Event('selectionchange'));
  });
  await page.getByRole('button',{name:'Ask about this',exact:true}).click();
  const question=page.getByRole('dialog',{name:'Ask about this passage'});
  await question.waitFor();
  await question.getByRole('button',{name:'Ask',exact:true}).click();
  await question.getByText('The selected source explains measurement error.',{exact:true}).waitFor();
  assert.equal(questions.length,1); assert.equal(questions[0].section_id,null);
  await page.keyboard.press('Escape');
  await question.waitFor({state:'hidden'});
  // Home previews source pages lazily, without requesting analysis or paper details.
  analyzed=true;
  libraryPapers=[paper,...Array.from({length:11},(_,i)=>({
    id:`home${String(i).padStart(12,'0')}`,
    metadata:{title:i===0?'An unread paper on collective decisions':`Research into ${['measurement','learning','cooperation','causality'][i%4]} ${i+1}`,
      authors:i===1?Array.from({length:24},(_,j)=>`Researcher ${j+1} Fullname`):[`Researcher ${i+1}`, 'Another Scientist'],year:2000+i,page_count:8+i},
    relative_path:`synthetic-${i}.pdf`,status:{state:i%3===0?'discovered':'ready'},
    analyzed_at:i%3===0?null:now,one_line_summary:i%3===0?null:'A concrete account of the mechanisms, evidence, and limitations behind this result.',
  }))];
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).click();
  await page.locator('.home-page').waitFor();
  assert.equal(new URL(page.url()).hash,'#home');
  const requestsBeforeHome=paperRequests.length;
  await page.reload();
  await page.locator('.paper-card').nth(11).waitFor();
  assert.equal(paperRequests.length,requestsBeforeHome,'home must not load an arbitrary paper');
  assert.equal(await page.locator('.view-switch').count(),0);
  assert.equal(await page.getByRole('button',{name:'Analyze',exact:true}).count(),0);
  assert.equal(await page.locator('.paper-card').count(),12);
  await page.waitForFunction(()=>document.activeElement?.classList.contains('paper-card'));
  await page.locator('.paper-card .paper-preview img').first().waitFor();
  assert.equal(await page.locator('.paper-card-bottom').count(),0);
  assert.equal(await page.locator('.paper-card').getByText(/^(Read paper|Explore paper)$/).count(),0);
  assert.ok(await page.locator('.paper-card h2').first().evaluate(node=>parseFloat(getComputedStyle(node).fontSize))<=18);
  assert.equal(await page.locator('.paper-card[tabindex="0"]').count(),1);
  // Navigation starts on entry and also works after focus moves to the app bar.
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).focus();
  await page.keyboard.press('ArrowRight');
  assert.equal(await page.locator('.paper-card').nth(1).evaluate(node=>node===document.activeElement),true);
  await page.keyboard.press('j');
  const downIndex=await page.locator('.paper-card').evaluateAll(cards=>cards.indexOf(document.activeElement));
  assert.ok(downIndex>1,'Down must move into the next visual row');
  await page.keyboard.press('k');
  assert.equal(await page.locator('.paper-card').nth(1).evaluate(node=>node===document.activeElement),true);
  await page.keyboard.press('h');
  assert.equal(await page.locator('.paper-card').first().evaluate(node=>node===document.activeElement),true);
  await page.keyboard.press('End');
  assert.equal(await page.locator('.paper-card').last().evaluate(node=>node===document.activeElement),true);
  await page.keyboard.press('Home');
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).focus();
  await page.keyboard.press('/');
  assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')),'Search papers');
  await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(()=>document.activeElement?.textContent),'All papers','home uses native Tab traversal');
  await page.getByRole('button',{name:/^Mapped/}).click();
  assert.equal(await page.locator('.paper-card').count(),8);
  await page.getByRole('button',{name:/^Unmapped/}).click();
  assert.equal(await page.locator('.paper-card').count(),4);
  await page.getByRole('button',{name:/^All papers/}).click();
  const homeSearch=page.getByRole('searchbox',{name:'Search papers',exact:true});
  await homeSearch.fill('collective decisions');
  assert.equal(await page.locator('.paper-card').count(),1);
  await page.keyboard.press('ArrowDown');
  assert.equal(await page.locator('.paper-card').evaluate(node=>node===document.activeElement),true);
  await page.keyboard.press('/');
  await homeSearch.fill('there is no such title');
  assert.equal(await page.locator('.paper-card').count(),0);
  await homeSearch.fill('');
  await page.setViewportSize({width:1280,height:850});
  await page.waitForFunction(()=>Array.from(document.querySelectorAll('.paper-card')).filter(node=>{
    const box=node.getBoundingClientRect(); return box.top<innerHeight&&box.bottom>0;
  }).every(node=>node.querySelector('.paper-preview img')!==null));
  await page.screenshot({path:'/tmp/lysilogy-home-desktop.png'});
  const cardColumns=await page.locator('.paper-card').evaluateAll(cards=>new Set(cards.map(card=>Math.round(card.getBoundingClientRect().left))).size);
  assert.ok(cardColumns>=3,'desktop home must present a grid of papers');
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).focus();
  await page.keyboard.press('q');
  await page.getByRole('dialog',{name:'Processing queue'}).waitFor();
  await page.keyboard.press('Escape');
  await page.getByRole('dialog',{name:'Processing queue'}).waitFor({state:'hidden'});
  await page.setViewportSize({width:390,height:844});
  assert.ok(await page.locator('.home-page').evaluate(node=>node.scrollWidth<=node.clientWidth+1),'home should fit mobile width');
  await page.screenshot({path:'/tmp/lysilogy-home-mobile.png'});
  await homeSearch.fill('collective decisions');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.text-view [data-text-ready="true"]'));
  assert.equal(await page.getByRole('button',{name:'Analyze',exact:true}).count(),1);
  await page.goBack();
  await page.locator('.home-page').waitFor();
  await page.waitForFunction(()=>document.activeElement?.dataset.paperId==='home000000000000');
  await page.goForward();
  await page.locator('.text-view').waitFor();
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).click();
  await page.locator('.home-page').waitFor();
  await homeSearch.fill('noisy proxies');
  await page.locator('.paper-card').click();
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await page.setViewportSize({width:1280,height:850});
  await page.waitForTimeout(350);
  await region.click();
  await page.locator('.section-focus').waitFor();
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).click();
  await page.locator('.home-page').waitFor();
  assert.equal(await page.locator('.section-focus').count(),0);
  assert.equal(await page.locator('.app-shell.has-library').count(),0);
  // A slow detail request must not hold the home page hostage or replace it later.
  const gate={started:Promise.withResolvers(),finished:Promise.withResolvers()};
  paperGate=gate;
  await page.goto(`http://lysilogy.test/?slow-load#paper=${id}`,{waitUntil:'domcontentloaded'});
  await gate.started.promise;
  assert.equal(await page.getByRole('button',{name:'Analyze',exact:true}).count(),0,'loading mapped paper must not flash Analyze');
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).click();
  try { await page.locator('.home-page').waitFor({timeout:2000}); }
  finally { gate.finished.resolve(); }
  await page.waitForTimeout(100);
  assert.equal(await page.locator('.home-page').count(),1,'late detail response must not replace home');
  await page.evaluate(()=>{window.location.hash='paper=missing-paper';});
  await page.waitForFunction(()=>window.location.hash==='#home');
  assert.equal(await page.locator('.home-page').count(),1,'invalid links return to a usable library');
  pollingTest=true;
  await page.goto(`http://lysilogy.test/?slow-polls#paper=${id}`);
  await page.waitForFunction(()=>document.querySelector('.current-paper-label')?.textContent?.includes('Refreshed through overlapping polls'),{},{timeout:6000});
  assert.ok(pollRequests>=3,'test must overlap two poll requests before the first completes');
  assert.deepEqual(analysisRequests,[],'browsing home and opening papers must not start analysis');
  assert.deepEqual(errors,[]);
  // Exercise a boundary inside one PDF.js text item. Canvas/CSS clipping alone
  // would leave the whole string available to copy, and trimming must not shift it.
  const textPage=await browser.newPage();
  await textPage.setContent('<div id="layer" style="position:relative;width:900px;height:400px"></div>');
  const textCropCode=ts.transpileModule(await readFile('src/lib/cropTextLayer.ts','utf8'),
    {compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText.replace('export function','function');
  await textPage.addScriptTag({content:textCropCode});
  for (const zoom of [.7,1,1.8]) {
    const result=await textPage.evaluate(zoom=>{
      const layer=document.querySelector('#layer');
      const span=document.createElement('span');span.dataset.textIndex='12';
      span.style.cssText=`position:absolute;left:${20*zoom}px;top:${30*zoom}px;font:${20*zoom}px/1 Courier;transform:scaleX(.85);transform-origin:left top;white-space:pre`;
      const prefix='The introduction. ',chosen='Chosen passage.';
      span.textContent=prefix+chosen+' Next topic.';layer.replaceChildren(span);
      const range=document.createRange();range.setStart(span.firstChild,prefix.length);range.setEnd(span.firstChild,prefix.length+chosen.length);
      const expected=range.getBoundingClientRect(),origin=layer.getBoundingClientRect();
      window.cropTextLayer(layer,[{x_min:(expected.left-origin.left)/zoom,x_max:(expected.right-origin.left)/zoom,
        y_min:(expected.top-origin.top)/zoom,y_max:(expected.bottom-origin.top)/zoom}],zoom,zoom);
      range.selectNodeContents(layer.firstChild);
      const actual=range.getBoundingClientRect();
      const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);
      return {text:selection.toString(),offset:layer.firstChild.dataset.textOffset,item:layer.firstChild.dataset.textIndex,
        shift:Math.abs(actual.left-expected.left)+Math.abs(actual.top-expected.top)};
    },zoom);
    assert.equal(result.text,'Chosen passage.');
    assert.equal(result.offset,'18');assert.equal(result.item,'12');
    assert.ok(result.shift<.1,`cropped selection shifted at zoom ${zoom}: ${result.shift}`);
  }
  await textPage.close();
  console.log('Pipeline smoke passed: cited context, selective refresh, cropped native selection, PDF modes, Escape and map restoration, mobile layout, queue, source questions, home grid, browser history, and slow-request navigation.');
} finally { await browser.close(); }
