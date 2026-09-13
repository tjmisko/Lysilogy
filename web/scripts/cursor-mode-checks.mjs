import assert from 'node:assert/strict';

export async function checkCursorMode(page,index) {
  const reader=page.locator('.pdf-reader');
  const mark=()=>page.locator('.pdf-source-mark.is-cursor, .pdf-source-mark.is-block-cursor').first();
  const offset=async()=>Number(await mark().getAttribute('data-source-offset'));
  const search=async query=>{
    await page.keyboard.press('/');const input=page.getByRole('textbox',{name:'Search paper with regular expression'});
    await input.fill(query);await input.press('Enter');
    await page.waitForFunction(query=>document.querySelector('.pdf-source-status')?.textContent.includes(`/${query} · 1 / 1`),query);
  };
  await search('compare');await page.keyboard.press('C');
  await mark().waitFor();
  assert.equal(await reader.getAttribute('data-source-cursor'),'true');
  assert.equal(await reader.getAttribute('data-line-numbers'),'relative');
  await page.locator('.pdf-source-line-number.is-active').waitFor();
  const first=await offset();
  const active=Number(await page.locator('.pdf-source-line-number.is-active').first().getAttribute('data-line-number'));
  await page.keyboard.press('j');
  await page.waitForFunction(first=>Number(document.querySelector('.pdf-source-mark.is-cursor')?.dataset.sourceOffset)>first,first);
  const second=await offset();
  assert.match(index.text.slice(second,second+30),/the.shelf|shelf UmeTrack/);
  assert.equal(Number(await page.locator('.pdf-source-line-number.is-active').first().getAttribute('data-line-number')),active+1);
  assert.equal(await page.locator(`.pdf-source-line-number[data-line-number="${active}"]`).first().textContent(),'1');
  await page.keyboard.press('k');await page.waitForFunction(first=>Number(document.querySelector('.pdf-source-mark.is-cursor')?.dataset.sourceOffset)===first,first);
  await page.keyboard.press('L');assert.equal(await reader.getAttribute('data-line-numbers'),'absolute');
  await page.waitForFunction(()=>Array.from(document.querySelectorAll('.pdf-source-line-number')).every(node=>node.textContent===node.dataset.lineNumber));
  await page.keyboard.press('L');await page.waitForFunction(()=>document.querySelectorAll('.pdf-source-line-number').length===0);
  await page.keyboard.press('L');assert.equal(await reader.getAttribute('data-line-numbers'),'relative');
  await page.keyboard.press('0');await page.keyboard.press('l');
  await page.waitForFunction(start=>Number(document.querySelector('.pdf-source-mark.is-cursor')?.dataset.sourceOffset)===start+1,index.text.indexOf('We compare'));
  await page.keyboard.press('l');
  await page.waitForFunction(start=>Number(document.querySelector('.pdf-source-mark.is-cursor')?.dataset.sourceOffset)===start+2,index.text.indexOf('We compare'));
  // The space after "We" is a real block-cursor position.
  await page.keyboard.press('v');await page.keyboard.press('y');await page.waitForFunction(()=>window.copiedSource===' ');
  await page.keyboard.press('0');await page.keyboard.press('y');
  assert.equal(await page.evaluate(()=>window.copiedSource),' ','y alone must remain operator-pending');
  await page.keyboard.press('w');await page.waitForFunction(()=>window.copiedSource==='We ');
  await page.keyboard.type('yy');await page.waitForFunction(()=>window.copiedSource==='We compare our single-view estimator to the\n');
  await page.keyboard.type('yap');
  await page.waitForFunction(()=>window.copiedSource.startsWith('We compare')&&window.copiedSource.includes('with UmeTrack.')&&!window.copiedSource.includes('Table 5'));
  await page.keyboard.press('E');
  await page.waitForFunction(()=>document.querySelector('.pdf-reader')?.dataset.sourceCursor==='true');
  assert.equal(await page.locator('.notes-panel').count(),0,'E belongs to WORD-end motion in Cursor mode');
  await page.keyboard.press('i');assert.equal(await reader.getAttribute('data-source-cursor'),'true');
  assert.equal(await page.locator('.notes-panel').count(),0);
  // A block is one stop; searching inside its caption snaps to its outline.
  await search('Confusion');
  await page.locator('.pdf-source-mark.is-block-cursor').waitFor();
  const figure=index.figures.find(f=>f.kind==='figure');
  assert.ok(figure);
  await page.screenshot({path:'/tmp/lysilogy-cursor-figure.png'});
  await page.keyboard.press('y');const before=await page.evaluate(()=>window.copiedSource);
  assert.ok(before.startsWith('We compare'));
  await page.keyboard.press('y');
  await page.waitForFunction(caption=>window.copiedSource.startsWith(caption)&&window.copiedSource.includes('[Figure 6 · p. 2](http://lysilogy.test/api/papers/1234567890abcdef/source#page=2)'),figure.caption);
  await page.keyboard.press('v');await page.locator('.pdf-source-mark.is-block-visual').waitFor();
  assert.equal(await page.locator('.pdf-source-mark.is-visual').count(),0,'atomic visual selection uses an outline, not individual cells');
  await page.keyboard.press('y');await page.locator('.pdf-source-mark.is-block-cursor').waitFor();
  await page.keyboard.press('j');await page.locator('.pdf-source-mark.is-cursor').waitFor();
  assert.ok(index.text.slice(await offset()).startsWith('was used'),'one j passes the whole figure');
  await search('Evaluation');await page.locator('.pdf-source-mark.is-block-cursor').waitFor();
  await page.keyboard.type('yy');await page.waitForFunction(()=>window.copiedSource.startsWith('Table 5.')&&window.copiedSource.includes('[Table 5 · p. 2]')&&!window.copiedSource.includes('32.91'));
  await page.keyboard.press('C');await page.waitForFunction(()=>!document.querySelector('.pdf-reader')?.hasAttribute('data-source-cursor'));
  await page.waitForFunction(()=>document.querySelectorAll('.pdf-source-line-number').length===0);
  // Re-entering has no edit state; C always leaves even from Visual mode.
  await page.keyboard.press('C');await page.keyboard.press('v');await page.keyboard.press('C');
  await page.waitForFunction(()=>!document.querySelector('.pdf-reader')?.hasAttribute('data-source-visual'));
  console.log('PASS Cursor mode physical lines, relative/absolute/off numbers, space cursor, y operators, atomic figure/table selection, caption references, read-only mode, and C toggle');
}

export async function checkCursorPaging(page,index) {
  const active=async()=>Number(await page.locator('.pdf-source-line-number.is-active').first().getAttribute('data-line-number'));
  const waitLine=async number=>page.waitForFunction(number=>Number(document.querySelector('.pdf-source-line-number.is-active')?.dataset.lineNumber)===number,number);
  await page.keyboard.press('H');await page.keyboard.press('C');
  await page.locator('.pdf-source-line-number.is-active').waitFor();
  await page.keyboard.type('gg');await waitLine(1);
  await page.keyboard.press('PageDown');
  await page.waitForFunction(()=>Number(document.querySelector('.pdf-source-line-number.is-active')?.dataset.lineNumber)>1);
  const half=await active();
  assert.ok(half>=12&&half<=16,`half-height movement lands on line ${half}`);
  assert.equal(await page.locator('.pdf-source-mark.is-cursor').first().evaluate(node=>node.closest('[data-pdf-page]').dataset.pdfPage),'1','half-page movement does not skip a whole PDF page');
  await page.keyboard.press('PageUp');await waitLine(1);
  await page.keyboard.press('Control+d');await waitLine(half);
  await page.keyboard.press('Control+u');await waitLine(1);
  await page.keyboard.type('3j');await waitLine(4);
  await page.keyboard.press('ArrowDown');await waitLine(5);
  await page.keyboard.press('ArrowUp');await waitLine(4);
  await page.keyboard.press('v');await page.keyboard.press('j');await waitLine(5);
  await page.keyboard.press('y');await page.waitForFunction(()=>window.copiedSource.includes('Sentence 4 on page 1')&&window.copiedSource.endsWith('S'));
  // Continuous vertical and horizontal modes retain the physical-line cursor.
  await page.keyboard.press('P');await page.keyboard.press('R');
  await page.keyboard.type('gg');await waitLine(1);
  await page.keyboard.press('Control+d');await waitLine(half);
  await page.keyboard.press('G');await waitLine(88);
  const end=Number(await page.locator('.pdf-source-mark.is-cursor').first().getAttribute('data-source-offset'));
  assert.equal(end,index.objects.paragraph.at(-1).start);
  await page.keyboard.type('gg');await waitLine(1);
  await page.screenshot({path:'/tmp/lysilogy-cursor-mode.png'});
  await page.keyboard.press('R');await page.keyboard.press('P');await page.keyboard.press('C');
  await page.keyboard.press('q');
  console.log('PASS physical cursor lines with counts/arrows, half-page PageUp/PageDown/Ctrl-u/Ctrl-d, visual line movement, and vertical/horizontal continuous PDF modes');
}
