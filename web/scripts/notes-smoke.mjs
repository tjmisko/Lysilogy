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

let storedNote={filename:'synthetic.md',text:'',revision:null};
const noteWrites=[]; const noteOpens=[]; let noteCreations=0; let notesLegacyResponse=true; let saveFailure=false;
const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1280,height:800},timezoneId:'America/Los_Angeles'});
  await page.clock.setFixedTime(new Date('2026-09-11T22:49:00Z'));
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
    if(suffix==='/notes/open') {
      assert.equal(route.request().method(),'POST');
      const body=route.request().postDataJSON(); noteOpens.push(body);
      if(notesLegacyResponse)return route.fulfill({status:200,contentType:'text/html',body:'<!doctype html><html><body>PRIVATE SPA CONTENT</body></html>'});
      if(storedNote.revision===null) {
        noteCreations++;
        storedNote={...storedNote,text:`---\ndate: ${body.created_at.slice(0,10)}\ntime: ${body.created_at.slice(11,16)}\ntags:\n  - paper\n---\n\n## Sources\n- [synthetic](<file:///synthetic-library/synthetic.pdf>)\n`,revision:'created-template'};
      }
      return route.fulfill({json:storedNote});
    }
    if(suffix==='/notes') {
      if(route.request().method()==='PUT') {
        const body=route.request().postDataJSON(); noteWrites.push(body);
        if(saveFailure)return route.fulfill({status:503,json:{message:'Notes storage is temporarily unavailable.'}});
        if(body.revision!==storedNote.revision) return route.fulfill({status:409,json:{message:'The notes file changed outside this editor. Reload it before saving again.'}});
        storedNote={...storedNote,text:body.text,revision:`revision-${noteWrites.length}`};
      }
      return route.fulfill({json:storedNote});
    }
    if(suffix==='/reading-index')return route.fulfill({json:readingIndex});
    if(suffix==='/source'){ sourceRequests++; return route.fulfill({body:pdf,contentType:'application/pdf'}); }
    if(suffix==='/abstract/refresh'||suffix==='/context/refresh'||suffix==='/structure/refresh'){ refreshes.push(suffix);return route.fulfill({json:{paper,analysis:analyzed?analysis:null}}); }
    if(suffix==='/reader-tools')return route.fulfill({json:{jobs:[],references:[],supercuts:[]}});
    if(url.pathname.startsWith('/api/'))return route.fulfill({status:404,json:{message:`Unexpected fixture request: ${url.pathname}`}});
    const file=path.join(root,url.pathname==='/'?'index.html':url.pathname.slice(1));
    const contentType=/\.m?js$/.test(file)?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.svg')?'image/svg+xml':'text/html';
    return route.fulfill({body:await readFile(file),contentType});
  });

  const homeCommand=async(click=false)=>{
    await page.keyboard.press(':');
    const menu=page.getByRole('dialog',{name:'Command menu',exact:true});await menu.waitFor();
    const option=menu.getByRole('option',{name:':home Open the paper library home',exact:true});
    await option.waitFor();
    const command=menu.getByRole('textbox',{name:'Command',exact:true});
    await command.fill('hom');assert.equal(await menu.getByRole('option').count(),1);
    await command.press('Tab');assert.equal(await command.inputValue(),'home');
    if(click)await option.click();else await command.press('Enter');
    await menu.waitFor({state:'hidden'});
  };

  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await page.locator('.pdf-reader').waitFor();
  await page.locator('.pdf-reader').focus();await homeCommand();
  await page.locator('.home-page').waitFor();
  assert.equal(new URL(page.url()).hash,'#home',':home navigates from a paper to the library grid');
  await page.locator('.paper-card').click();await page.locator('.pdf-reader').waitFor();
  await page.keyboard.press('E');
  const editor=page.locator('.notes-panel .cm-content');
  const editorText=()=>editor.locator('.cm-line').evaluateAll(lines=>lines.map(line=>line.textContent).join('\n'));
  const vimInput=page.locator('.notes-panel .cm-vim-panel input');
  const mode=async(value)=>page.waitForFunction(value=>document.querySelector('.notes-panel .cm-vim-panel')?.textContent.includes(value),value);
  const ex=async(command)=>{
    await editor.focus(); await page.keyboard.press('Escape'); await page.keyboard.type(':');
    await vimInput.waitFor(); await page.keyboard.type(command); await page.keyboard.press('Enter');
    await vimInput.waitFor({state:'hidden'});
  };
  const replaceBuffer=async(text)=>{
    await editor.focus(); await page.keyboard.press('Escape'); await page.keyboard.type('ggVGc');
    await mode('INSERT'); await page.keyboard.insertText(text); await page.keyboard.press('Escape');
    await mode('NORMAL'); assert.equal(await editorText(),text);
  };
  const search=async(direction,query)=>{
    await editor.focus(); await page.keyboard.type(direction); await vimInput.waitFor();
    await page.keyboard.type(query); await page.keyboard.press('Enter'); await vimInput.waitFor({state:'hidden'});
  };
  const focusWithin=async(selector)=>page.waitForFunction(selector=>document.querySelector(selector)?.contains(document.activeElement),selector,{timeout:3000});
  const closed=()=>page.locator('.notes-panel').waitFor({state:'hidden'});
  const opened=async()=>{await page.keyboard.press('E'); await editor.waitFor(); await mode('NORMAL');};
  const saved=()=>page.waitForFunction(()=>document.querySelector('.notes-panel')?.dataset.dirty==='false');
  await page.getByText('Notes unavailable',{exact:true}).waitFor();
  assert.match(await page.locator('.notes-error').innerText(),/Restart the Lysilogy backend/);
  assert.match(await page.locator('.notes-error').innerText(),/\/api proxy/);
  assert.doesNotMatch(await page.locator('.notes-panel').innerText(),/JSON\.parse|PRIVATE SPA CONTENT|Opening Markdown|Saved to Markdown/);
  assert.equal(await editor.count(),0);
  assert.equal(noteCreations,0);
  await page.screenshot({path:'/tmp/lysilogy-notes-unavailable.png'});
  notesLegacyResponse=false;
  await page.getByRole('button',{name:'Retry',exact:true}).click();
  await editor.waitFor();
  assert.equal(noteCreations,1,'opening a missing note creates its template immediately');
  assert.equal(noteWrites.length,0,'creating the initial template requires no Ctrl-S');
  assert.equal(noteOpens.at(-1).created_at,'2026-09-11T15:49:00-07:00');
  const template='---\ndate: 2026-09-11\ntime: 15:49\ntags:\n  - paper\n---\n\n## Sources\n- [synthetic](<file:///synthetic-library/synthetic.pdf>)\n';
  assert.equal(storedNote.text,template);
  assert.equal(await editorText(),template,'the created template must appear in the Markdown editor');
  await ex('q'); await closed();
  await opened();
  assert.equal(storedNote.text,template,'reopening an existing note does not change its template');
  assert.equal(noteCreations,1);
  await ex('q'); await closed();
  storedNote={...storedNote,text:'',revision:'existing-empty'};
  await opened();
  assert.equal(storedNote.text,'','an existing empty note stays empty');
  assert.equal(noteCreations,1);
  assert.equal(noteWrites.length,0);
  await mode('NORMAL');
  await editor.press('Escape'); await editor.press('Escape');
  assert.equal(await page.locator('.notes-panel').count(),1,'Normal Escape never closes the notes panel');
  await editor.press('i'); await mode('INSERT');
  await page.keyboard.insertText('# Reading notes\n\nq / notes **strong**');
  assert.equal(await page.locator('.notes-md-heading').count(),1);
  assert.equal(await page.locator('.notes-md-strong').count(),1);
  await page.keyboard.type(' q /');
  assert.equal(await page.getByRole('dialog',{name:'Processing queue'}).count(),0);
  assert.equal(await page.locator('.pdf-source-tools').count(),0,'Insert-mode / must not open paper search');
  await editor.press('Control+s'); await saved();
  assert.equal(storedNote.text,'# Reading notes\n\nq / notes **strong** q /');
  assert.equal(noteWrites.length,1);
  await editor.press('Escape'); await mode('NORMAL');
  await page.keyboard.type('A'); await page.keyboard.type(' additional text'); await editor.press('Escape');
  await page.keyboard.type('u');
  assert.equal(await editorText(),storedNote.text,'Vim undo restores the last saved text');
  await editor.press('Control+r');
  assert.match(await editorText(),/additional text/);
  await ex('q');
  await page.getByText('Save your notes before leaving?',{exact:true}).waitFor();
  await page.keyboard.press('Escape');
  await page.locator('.notes-close-prompt').waitFor({state:'hidden'});
  assert.equal(await editor.evaluate(node=>node.contains(document.activeElement)),true,'Escape cancels the close prompt and returns focus');
  assert.equal(await page.locator('.notes-panel').getAttribute('data-dirty'),'true');
  await ex('q');
  await page.getByRole('button',{name:'Keep editing',exact:true}).click();
  assert.equal(await page.locator('.notes-panel').getAttribute('data-dirty'),'true');
  await ex('q');
  await page.getByRole('button',{name:'Discard changes',exact:true}).click(); await closed();
  await opened();
  assert.equal(await editorText(),storedNote.text);

  // Exercise native Vim editing, rather than simulating changes through the DOM.
  await replaceBuffer('alpha beta gamma\nsecond alpha line\nthird alpha item');
  await page.keyboard.type('ggwciwBETA'); await editor.press('Escape');
  assert.equal(await editorText(),'alpha BETA gamma\nsecond alpha line\nthird alpha item','ciw changes only the selected word');
  await page.keyboard.type('gg0v4l'); await mode('VISUAL');
  await page.keyboard.type('yG$p');
  assert.match(await editorText(),/itemalpha$/,'a visual yank can be pasted elsewhere');
  await page.keyboard.type('u');
  await page.keyboard.type('gg0"ayiwG$"ap');
  assert.match(await editorText(),/itemalpha$/,'named registers retain yanked text');
  await page.keyboard.type('u');
  await page.keyboard.type('gg0d'); await editor.press('Escape'); await page.keyboard.type('rA');
  assert.match(await editorText(),/^Alpha BETA gamma/,'Escape cancels an unfinished operator');
  await page.keyboard.type('v'); await mode('VISUAL'); await editor.press('Escape'); await mode('NORMAL');
  assert.equal(await page.locator('.notes-panel').count(),1,'Visual Escape returns to Normal');
  await replaceBuffer('keep remove');
  await page.keyboard.type('A'); await editor.press('Control+w'); await mode('INSERT'); await editor.press('Escape');
  assert.equal(await editorText(),'keep ','Insert Ctrl-w still deletes a word');

  await replaceBuffer('one\ntwo\nthree');
  await page.keyboard.type('ggqaI- '); await editor.press('Escape'); await page.keyboard.type('jq');
  await page.keyboard.type('@a@@');
  assert.equal(await editorText(),'- one\n- two\n- three','q records and @ replays a real Vim macro');
  assert.equal(await page.getByRole('dialog',{name:'Processing queue'}).count(),0,'macro q stays local');

  await replaceBuffer('alpha first\nsecond alpha\nthird alpha\nlast beta');
  await page.keyboard.type('gg0'); await search('/','alpha'); await page.keyboard.type('nNciwFORWARD'); await editor.press('Escape');
  assert.equal(await editorText(),'alpha first\nsecond FORWARD\nthird alpha\nlast beta','/ with n and N navigates notes matches');
  await page.keyboard.type('G$'); await search('?','alpha'); await page.keyboard.type('ciwBACKWARD'); await editor.press('Escape');
  assert.equal(await editorText(),'alpha first\nsecond FORWARD\nthird BACKWARD\nlast beta','? searches backward within notes');
  await replaceBuffer('alpha first\nsecond alpha\nthird alpha');
  await page.keyboard.type('gg0*ciwSTAR'); await editor.press('Escape');
  await page.keyboard.type('G0w#ciwHASH'); await editor.press('Escape');
  assert.equal(await editorText(),'HASH first\nsecond STAR\nthird alpha','* and # find the word under the cursor');
  assert.equal(await page.locator('.pdf-source-tools').count(),0,'Vim searches never open PDF search');

  await replaceBuffer('keep this text');
  await page.keyboard.type('gg0/this'); await vimInput.waitFor(); await page.keyboard.press('Escape');
  await vimInput.waitFor({state:'hidden'}); await page.keyboard.type('rK');
  assert.equal(await editorText(),'Keep this text','canceling a search restores the original cursor');
  await page.keyboard.type(':'); await vimInput.waitFor(); await page.keyboard.type('%s/Keep/Delete/');
  await page.keyboard.press('Escape'); await vimInput.waitFor({state:'hidden'});
  assert.equal(await editorText(),'Keep this text','Escape cancels Ex without running its command');
  assert.equal(await page.getByRole('textbox',{name:'Command',exact:true}).count(),0,'Ex never opens the app command bar');
  await replaceBuffer('alpha alpha\nalpha beta');
  await ex('%s/alpha/omega/g');
  assert.equal(await editorText(),'omega omega\nomega beta','substitution applies across the buffer');
  await ex('w'); await saved();
  assert.equal(storedNote.text,'omega omega\nomega beta',':w writes the exact Vim buffer');

  // Switch panes without closing notes or leaking normal commands into the PDF.
  await ex('reader'); await focusWithin('.pdf-reader');
  await page.keyboard.press('Control+w'); await page.keyboard.type('l');
  await focusWithin('.notes-panel .cm-content');
  await editor.press('Control+w'); await page.keyboard.type('h');
  await focusWithin('.pdf-reader');
  await page.keyboard.press('Control+w'); await page.keyboard.type('l');
  await focusWithin('.notes-panel .cm-content');
  await editor.press('Control+w'); await page.keyboard.type('w');
  await focusWithin('.pdf-reader');
  await page.keyboard.press('Control+w'); await page.keyboard.type('w');
  await focusWithin('.notes-panel .cm-content');
  assert.equal(await page.locator('.notes-panel').count(),1);

  await replaceBuffer('# Discard this draft');
  const beforeForceQuit=noteWrites.length;
  await ex('q!'); await closed();
  assert.equal(noteWrites.length,beforeForceQuit,':q! discards without writing');
  await opened(); assert.equal(await editorText(),storedNote.text);
  await replaceBuffer('# Written with wq');
  await ex('wq'); await closed();
  assert.equal(storedNote.text,'# Written with wq');
  await opened();
  const beforeCleanExit=noteWrites.length;
  await ex('x'); await closed();
  assert.equal(noteWrites.length,beforeCleanExit,':x closes a clean buffer without writing');
  await opened(); await replaceBuffer('# Written with x');
  await ex('x'); await closed(); assert.equal(storedNote.text,'# Written with x');
  await opened(); await replaceBuffer('# Failed save must stay open');
  saveFailure=true; await ex('wq');
  await page.getByText('Notes storage is temporarily unavailable.',{exact:true}).waitFor();
  assert.equal(await editorText(),'# Failed save must stay open');
  assert.equal(await page.locator('.notes-panel').getAttribute('data-dirty'),'true');
  assert.equal(storedNote.text,'# Written with x','a failed :wq never replaces the saved note');
  saveFailure=false; await ex('wq'); await closed();
  assert.equal(storedNote.text,'# Failed save must stay open','retrying :wq saves and closes only on success');
  await opened();
  storedNote={...storedNote,text:'# External edit\n\nWritten in another editor.',revision:'external-revision'};
  await replaceBuffer('# My unsaved draft'); await ex('wq');
  await page.getByRole('button',{name:'Compare with disk',exact:true}).waitFor();
  assert.equal(await editorText(),'# My unsaved draft');
  await page.getByRole('button',{name:'Compare with disk',exact:true}).click();
  await page.locator('.notes-disk-version pre').waitFor();
  assert.match(await page.locator('.notes-disk-version pre').innerText(),/External edit/);
  assert.equal(await editorText(),'# My unsaved draft','conflict comparison preserves the draft');
  await page.getByRole('button',{name:'Discard draft and use disk version',exact:true}).click();
  assert.equal(await editorText(),storedNote.text);
  await replaceBuffer('# Explicit overwrite');
  storedNote={...storedNote,text:'# Another external edit',revision:'force-revision'};
  await ex('wq');
  await page.getByRole('button',{name:'Compare with disk',exact:true}).waitFor();
  assert.equal(storedNote.text,'# Another external edit');
  await ex('w!'); await saved();
  assert.equal(storedNote.text,'# Explicit overwrite',':w! explicitly replaces an external edit');
  assert.equal(noteWrites.at(-1).revision,'force-revision','forced writes still use the latest content revision');
  assert.equal(await page.locator('.notes-panel').count(),1,':w! saves without closing');
  await replaceBuffer('# Explicit overwrite and quit');
  storedNote={...storedNote,text:'# Newer external edit',revision:'force-quit-revision'};
  await ex('wq');
  await page.getByRole('button',{name:'Compare with disk',exact:true}).waitFor();
  await ex('wq!'); await closed();
  assert.equal(storedNote.text,'# Explicit overwrite and quit',':wq! explicitly overwrites and closes after success');
  assert.equal(noteWrites.at(-1).revision,'force-quit-revision');
  await opened();
  await replaceBuffer('# Saved before going home');
  await page.mouse.move(20,2);
  await page.getByRole('button',{name:'Lysilogy home',exact:true}).click();
  await page.getByRole('button',{name:'Save and close',exact:true}).click();
  await page.locator('.home-page').waitFor();
  assert.equal(await page.locator('.notes-panel').count(),0);
  assert.equal(storedNote.text,'# Saved before going home');
  await page.locator('.paper-card').click(); await page.locator('.pdf-reader').waitFor();
  await opened();
  await replaceBuffer('# Discard before going home');
  await editor.press('Control+w');await page.keyboard.type('h');await focusWithin('.pdf-reader');
  await homeCommand();
  await page.getByText('Save your notes before leaving?',{exact:true}).waitFor();
  assert.equal(new URL(page.url()).hash,`#paper=${id}`,':home defers navigation while notes are dirty');
  await page.getByRole('button',{name:'Keep editing',exact:true}).click();
  assert.equal(await editorText(),'# Discard before going home',':home cancellation preserves the draft');
  await editor.press('Control+w');await page.keyboard.type('h');await focusWithin('.pdf-reader');
  await homeCommand();
  await page.getByRole('button',{name:'Discard changes',exact:true}).click();
  await page.locator('.home-page').waitFor();
  assert.equal(storedNote.text,'# Saved before going home');
  await page.locator('.paper-card').click(); await page.locator('.pdf-reader').waitFor();
  await opened();
  await replaceBuffer('# Back navigation draft');
  await page.goBack();
  await page.getByText('Save your notes before leaving?',{exact:true}).waitFor();
  assert.equal(new URL(page.url()).hash,`#paper=${id}`,'dirty browser back must retain current paper URL');
  await page.getByRole('button',{name:'Discard changes',exact:true}).click();
  await page.locator('.home-page').waitFor();
  await page.locator('.paper-card').click(); await page.locator('.pdf-reader').waitFor();
  await opened();
  await replaceBuffer('# Questions while reading\n\n- What assumptions does the result need?\n- How does this relate to earlier evidence?\n\nA **working note**, with `inline code` and a [source link](https://example.org).\n\n> Keep the important qualification alongside the claim.');
  await page.screenshot({path:'/tmp/lysilogy-notes-editor.png'});
  await ex('wq'); await closed();
  // Notes preserve the same local modes and pane focus inside a selected section.
  analyzed=true; await page.reload();
  await page.getByRole('button',{name:'Overview',exact:true}).click();
  await page.locator('.section-boxes button[data-section-id="regressional"]').first().click();
  await page.locator('.section-focus').waitFor(); await opened();
  await editor.press('Escape'); await editor.press('Escape');
  assert.equal(await page.locator('.section-focus').count(),1);
  assert.equal(await page.locator('.notes-panel').count(),1);
  await editor.press('Control+w'); await page.keyboard.type('h');
  await focusWithin('.section-source-scroll');
  await page.keyboard.press('Control+w'); await page.keyboard.type('l');
  await focusWithin('.notes-panel .cm-content');
  await ex('q'); await closed();
  assert.equal(await page.locator('.section-focus').count(),1,':q closes only notes inside a section');
  await focusWithin('.section-source-scroll');await homeCommand(true);
  await page.locator('.home-page').waitFor();
  assert.equal(new URL(page.url()).hash,'#home');
  assert.equal(await page.locator('.section-focus').count(),0,':home leaves a focused section for the library grid');
  assert.deepEqual(errors,[]);
  console.log('PASS notes: HTML fallback diagnosis, retry, create-on-open template, local date, existing-note preservation, CodeMirror, Markdown styling, Vim modes, text objects, macros, registers, native search, substitution, undo/redo, pane focus, Ex save/quit, failed saves, conflicts, dirty-close choices, and :home command navigation from paper/section with dirty-note safeguards.');
} finally {await browser.close();}
