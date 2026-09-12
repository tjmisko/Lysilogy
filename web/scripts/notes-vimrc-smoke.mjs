import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

// Synthetic source and note: this suite never accesses a reading-library path.
function makePdf() {
  const content = 'BT /F1 18 Tf 48 738 Td (Synthetic notes configuration test) Tj ET';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`,
  ];
  let pdf = '%PDF-1.4\n';
  const offsets = [0];
  objects.forEach((object, index) => {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('');
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return Buffer.from(pdf);
}

const id = '1234567890abcdef';
const paper = {
  id, metadata: { title: 'Synthetic Vim configuration', authors: ['Test Author'], year: 2026, page_count: 1 },
  relative_path: 'synthetic.pdf', status: { state: 'unprocessed' }, analyzed_at: null, one_line_summary: null,
};
const readingIndex = { schema_version: 3, text: '', pages: [], tokens: [], objects: { word: [], WORD: [], sentence: [], paragraph: [] }, figures: [], gaps: [] };
const pdf = makePdf();
const root = path.resolve('dist');
const vimrcPath = '/synthetic-settings/.vimrc';
let configuration = { path: vimrcPath, exists: false, text: '' };
let storedNote = { filename: 'synthetic.md', text: 'Original note', revision: 'initial' };
let saveGate = null;
const vimrcRequests = [];
const noteWrites = [];

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('http://lysilogy.test/**', async route => {
    const request = route.request();
    const url = new URL(request.url());
    const suffix = url.pathname.replace(/^\/api\/papers\/[^/]+/, '');
    if (url.pathname === '/api/library') return route.fulfill({ json: { name: 'Synthetic library', papers: [paper] } });
    if (url.pathname === '/api/queue') return route.fulfill({ json: { jobs: [] } });
    if (url.pathname === '/api/notes/vimrc') {
      assert.equal(request.method(), 'GET');
      vimrcRequests.push(configuration.text);
      return route.fulfill({ json: configuration });
    }
    if (url.pathname === `/api/papers/${id}`) return route.fulfill({ json: { paper, analysis: null } });
    if (suffix === '/notes/open') return route.fulfill({ json: storedNote });
    if (suffix === '/notes') {
      if (request.method() === 'PUT') {
        const body = request.postDataJSON();
        noteWrites.push(body);
        if (saveGate !== null) {
          saveGate.started.resolve();
          await saveGate.finished.promise;
        }
        assert.equal(body.revision, storedNote.revision);
        storedNote = { ...storedNote, text: body.text, revision: `revision-${noteWrites.length}` };
      }
      return route.fulfill({ json: storedNote });
    }
    if (suffix === '/source') return route.fulfill({ contentType: 'application/pdf', body: pdf });
    if (suffix === '/reading-index') return route.fulfill({ json: readingIndex });
    if (suffix === '/reader-tools') return route.fulfill({ json: { jobs: [], references: [], supercuts: [] } });
    if (url.pathname.startsWith('/api/')) return route.fulfill({ status: 404, json: { message: `Unexpected fixture request: ${url.pathname}` } });
    const file = path.join(root, url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
    const contentType = /\.m?js$/.test(file) ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html';
    return route.fulfill({ body: await readFile(file), contentType });
  });

  const editor = page.locator('.notes-panel .cm-content');
  const vimInput = page.locator('.notes-panel .cm-vim-panel input');
  const editorText = () => editor.locator('.cm-line').evaluateAll(lines => lines.map(line => line.textContent).join('\n'));
  const mode = value => page.waitForFunction(value => document.querySelector('.notes-panel .cm-vim-panel')?.textContent.includes(value), value);
  const ex = async command => {
    await editor.focus();
    await page.keyboard.press('Escape');
    await page.keyboard.type(':');
    await vimInput.waitFor();
    await page.keyboard.type(command);
    await page.keyboard.press('Enter');
    await vimInput.waitFor({ state: 'hidden' });
  };
  const replaceBuffer = async text => {
    await editor.focus();
    await page.keyboard.press('Escape');
    await page.keyboard.type('ggVGc');
    await mode('INSERT');
    await page.keyboard.insertText(text);
    await page.keyboard.press('Escape');
    await mode('NORMAL');
    assert.equal(await editorText(), text);
  };
  const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const reload = async (text, command = 'source') => {
    configuration = { path: vimrcPath, exists: true, text };
    const response = page.waitForResponse(response => new URL(response.url()).pathname === '/api/notes/vimrc');
    await ex(command);
    await response;
    await settle();
  };
  const search = async query => {
    await page.keyboard.type('/');
    await vimInput.waitFor();
    await page.keyboard.type(query);
    await page.keyboard.press('Enter');
    await vimInput.waitFor({ state: 'hidden' });
  };

  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await page.locator('.pdf-reader').waitFor();
  const initialConfiguration = page.waitForResponse(response => new URL(response.url()).pathname === '/api/notes/vimrc');
  await page.keyboard.press('E');
  await initialConfiguration;
  await editor.waitFor();
  await mode('NORMAL');
  await settle();
  assert.equal(vimrcRequests.length, 1, 'opening notes loads the configured vimrc');

  // U is a default even when the configured file does not yet exist.
  await replaceBuffer('alpha');
  await page.keyboard.type('A extra');
  await page.keyboard.press('Escape');
  await page.keyboard.type('u');
  assert.equal(await editorText(), 'alpha');
  await page.keyboard.type('U');
  assert.equal(await editorText(), 'alpha extra', 'U must redo the last undone change');
  await page.keyboard.type('u');
  await page.keyboard.press('Control+r');
  assert.equal(await editorText(), 'alpha extra', 'Ctrl-r also retains Vim redo');

  const vimrc = [
    'let mapleader = " "',
    'nnoremap U <C-r>',
    'inoremap jk <Esc>',
    'nnoremap ; ,',
    'nnoremap , ;',
    'nnoremap <leader>a ILEADER <Esc>',
    'nnoremap n nzz',
    'vnoremap <leader>u U',
    'let g:prefix = "SCRIPT "',
    'if 1',
    "  execute 'nnoremap <leader>b I' . g:prefix . '<Esc>'",
    'endif',
    'set number tabstop=4 shiftwidth=4 expandtab',
    'command! SaveAndQuit wq',
  ].join('\n');
  await reload(vimrc);
  assert.equal(await editorText(), 'alpha extra', 'sourcing the vimrc preserves an unsaved buffer');
  assert.equal(await page.locator('.notes-vimrc-status').count(), 0, 'the supported vimrc has no diagnostics');
  await page.locator('.notes-panel .cm-lineNumbers').waitFor();

  await replaceBuffer('indented');
  await page.keyboard.type('gg0i');
  await page.keyboard.press('Tab');
  await page.keyboard.press('Escape');
  assert.equal(await editorText(), '    indented', 'Insert Tab respects expandtab and the configured indentation width');

  await replaceBuffer('alpha');
  await page.keyboard.type('A beta');
  await page.keyboard.type('jk');
  await mode('NORMAL');
  assert.equal(await editorText(), 'alpha beta', 'Insert jk exits without inserting the mapped keys');
  assert.equal(await page.locator('.notes-panel').count(), 1, 'mapped Escape stays inside notes');

  await replaceBuffer('a,b,c,d');
  await page.keyboard.type('gg0f,,rX');
  assert.equal(await editorText(), 'a,bXc,d', 'a nonrecursive comma remap repeats the last find forward');
  await replaceBuffer('a,b,c,d');
  await page.keyboard.type('gg0f,,;rX');
  assert.equal(await editorText(), 'aXb,c,d', 'a nonrecursive semicolon remap repeats the last find backward');

  await replaceBuffer('alpha');
  await page.keyboard.type('qa');
  await page.keyboard.type(' a');
  await page.keyboard.type('q');
  assert.equal(await editorText(), 'LEADER alpha', 'mapleader expands within mappings');
  await page.keyboard.type('u');
  assert.equal(await editorText(), 'alpha');
  await page.keyboard.type('@a');
  assert.equal(await editorText(), 'LEADER alpha', 'macros record and replay the actual leader mapping');
  await page.keyboard.type(' b');
  assert.equal(await editorText(), 'SCRIPT LEADER alpha', 'a conditional execute can use scalar variables');
  await replaceBuffer('visual');
  await page.keyboard.type('gg0viw u');
  await mode('NORMAL');
  assert.equal(await editorText(), 'VISUAL', 'the visual map operates without invoking Normal U redo');

  await replaceBuffer('alpha');
  await page.keyboard.type('gg0');
  await settle();
  const beforeSpace = await page.locator('.notes-panel .cm-fat-cursor').first().evaluate(node => node.getBoundingClientRect().x);
  await page.keyboard.press('Space');
  await page.waitForFunction(before => {
    const cursor = document.querySelector('.notes-panel .cm-fat-cursor');
    return cursor !== null && cursor.getBoundingClientRect().x > before + 1;
  }, beforeSpace);
  await page.keyboard.type('rX');
  assert.equal(await editorText(), 'aXpha', 'Space by itself retains native rightward motion after its prefix timeout');
  await replaceBuffer('alpha beta gamma');
  await page.keyboard.type('gg0 wrB');
  assert.equal(await editorText(), 'alpha Beta gamma', 'an unrelated key after Space falls back to native motions');

  await replaceBuffer('alpha first\nsecond alpha\nthird alpha');
  await page.keyboard.type('gg0');
  await search('alpha');
  await page.keyboard.type('nciwFOUND');
  await page.keyboard.press('Escape');
  assert.equal(await editorText(), 'alpha first\nsecond alpha\nthird FOUND', 'n-to-nzz works without recursive search');
  assert.equal(await page.locator('.pdf-source-tools').count(), 0, 'notes search never opens paper search');

  await replaceBuffer('alpha');
  await ex('nmapclear');
  await page.keyboard.type('gg0 rX');
  assert.equal(await editorText(), 'aXpha', ':nmapclear restores native Space motion');
  await page.keyboard.type('u');
  assert.equal(await editorText(), 'alpha', 'clearing mappings preserves native undo');
  await page.keyboard.press('Control+r');
  assert.equal(await editorText(), 'aXpha', 'clearing mappings preserves native redo');
  await reload(vimrc);
  assert.equal(await editorText(), 'aXpha', 'reloading after mapclear preserves text and restores the config');

  // Replacing configuration must remove old remaps without recreating the editor.
  await replaceBuffer('draft');
  await page.keyboard.type('A unsaved');
  await page.keyboard.press('Escape');
  const replacement = 'let mapleader = " "\nnnoremap <leader>a IRELOADED <Esc>\ncommand! SaveAndQuit wq';
  await reload(replacement, `source ${vimrcPath}`);
  assert.equal(await editorText(), 'draft unsaved');
  assert.equal(await page.locator('.notes-panel .cm-lineNumbers').count(), 0, 'removed options return to the editor defaults');
  await page.keyboard.type('u');
  assert.equal(await editorText(), 'draft', 'sourcing preserves the undo history');
  await page.keyboard.type('U');
  assert.equal(await editorText(), 'draft unsaved', 'default U redo survives reloading the config');
  await page.keyboard.type('@a');
  assert.equal(await editorText(), 'RELOADED draft unsaved', 'sourcing preserves macros and replay uses the current mapping');
  await page.keyboard.type('u');
  await page.keyboard.type(' a');
  assert.equal(await editorText(), 'RELOADED draft unsaved', 'new mappings replace their prior definition');
  await page.keyboard.type('Ajk');
  await mode('INSERT');
  await page.keyboard.press('Escape');
  assert.equal(await editorText(), 'RELOADED draft unsavedjk', 'removed Insert mappings return to literal input');
  await replaceBuffer('a,b,c,d');
  await page.keyboard.type('gg0f,;rX');
  assert.equal(await editorText(), 'a,bXc,d', 'removed Normal mappings restore native find-repeat behavior');

  await replaceBuffer('persistent draft');
  saveGate = { started: Promise.withResolvers(), finished: Promise.withResolvers() };
  await ex('SaveAndQuit');
  await saveGate.started.promise;
  assert.equal(await page.locator('.notes-panel').count(), 1, ':wq aliases wait for asynchronous persistence');
  assert.equal(await editorText(), 'persistent draft');
  saveGate.finished.resolve();
  await page.locator('.notes-panel').waitFor({ state: 'hidden' });
  saveGate = null;
  assert.equal(storedNote.text, 'persistent draft');

  const priorOpens = vimrcRequests.length;
  const reopened = page.waitForResponse(response => new URL(response.url()).pathname === '/api/notes/vimrc');
  await page.keyboard.press('E');
  await reopened;
  await editor.waitFor();
  await mode('NORMAL');
  await settle();
  assert.equal(vimrcRequests.length, priorOpens + 1, 'reopening a buffer reads the vimrc again');
  assert.equal(await editorText(), 'persistent draft');
  const invalid = 'let mapleader = " "\nfunction! Unsupported()\nendfunction\nnnoremap <leader>a IVALID <Esc>';
  await reload(invalid);
  const diagnostics = page.locator('.notes-vimrc-status');
  await diagnostics.waitFor();
  assert.match(await diagnostics.innerText(), /unsupported|diagnostic|skipped|warning/i, 'unsupported scripting is reported');
  await diagnostics.locator('summary').click();
  assert.match(await diagnostics.innerText(), /function/i, 'the diagnostic identifies unsupported function scripting');
  await editor.focus();
  await page.keyboard.type(' a');
  assert.equal(await editorText(), 'VALID persistent draft', 'valid mappings still apply when other lines are unsupported');
  await ex('wq');
  await page.locator('.notes-panel').waitFor({ state: 'hidden' });
  assert.equal(storedNote.text, 'VALID persistent draft', 'built-in :wq remains available after a config error');

  // Keep the actual checked-in portable mappings covered by the same engine.
  configuration = { path: vimrcPath, exists: true, text: await readFile('../.vimrc', 'utf8') };
  const projectConfiguration = page.waitForResponse(response => new URL(response.url()).pathname === '/api/notes/vimrc');
  await page.keyboard.press('E');
  await projectConfiguration;
  await editor.waitFor();
  await mode('NORMAL');
  await settle();
  assert.equal(await diagnostics.count(), 0, 'the checked-in vimrc loads without diagnostics');
  await replaceBuffer('redo');
  await page.keyboard.type('A verified');
  await page.keyboard.press('Escape');
  await page.keyboard.type('uU');
  assert.equal(await editorText(), 'redo verified', 'the checked-in U mapping performs redo');
  await replaceBuffer('    alpha beta');
  await page.keyboard.type('gg$');
  await page.keyboard.press('Home');
  await page.keyboard.type('rA');
  assert.equal(await editorText(), '    Alpha beta', 'the checked-in Home mapping goes to the first nonblank character');
  await page.keyboard.press('Control+w');
  await page.keyboard.type('h');
  await page.waitForFunction(() => document.querySelector('.pdf-reader')?.contains(document.activeElement));
  await page.keyboard.press('Control+w');
  await page.keyboard.type('l');
  await page.waitForFunction(() => document.querySelector('.notes-panel .cm-content')?.contains(document.activeElement));
  assert.equal(await editorText(), '    Alpha beta', 'pane switching preserves the dirty buffer');
  await ex('Wq');
  await page.locator('.notes-panel').waitFor({ state: 'hidden' });
  assert.equal(storedNote.text, '    Alpha beta', 'the checked-in Wq alias saves and closes');
  assert.equal(errors.length, 0, errors.join('\n'));
  console.log('Notes vimrc browser checks passed: default redo, mode remaps, nonrecursive find/search, leader scripts, safe reload, diagnostics, asynchronous :wq, and the checked-in portable mappings.');
} finally {
  await browser.close();
}
