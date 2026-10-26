import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { visualFixture } from './pdf-fit-checks.mjs';
const titles = ['A Mathematical Theory of Communication', 'AI Control: Improving Safety Despite Intentional Subversion', 'Applied Category Theory — Monads and Haskell', 'Cooperative Inverse Reinforcement Learning', 'The Geometry of Learning', 'Transport Policies for Climate'];
const abstract = 'We study how information moves through a system, and which structures preserve its meaning. Starting with a simple model of communication, we derive bounds on efficient transmission and examine how these results extend to uncertain environments. Our experiments show that a small set of organizing principles can explain a wide range of observed behavior. These findings provide a foundation for further work on reliable communication and learning.';
const papers = titles.map((title, index) => ({ id: `${index + 1}`.padStart(16, '0'), relative_path: `${title}.pdf`, metadata: { title, authors: [['Claude Shannon'], ['Greenblatt', 'Shlegeris', 'Sachan'], ['Duarte Maia'], ['Hadfield-Menell', 'Russell'], ['Ada Lovelace'], ['Axsen', 'Long']][index], year: 1948 + index * 15, page_count: 12 + index, subject: null }, status: { state: index % 2 === 0 ? 'ready' : 'discovered' }, analyzed_at: null, author_abstract: index % 3 === 0 ? abstract : null, one_line_summary: index === 2 ? 'An introduction to the structure and composition of computations.' : null }));
const library = { name: 'Reading room', papers, directories: [] };
let scans = 0, imports = 0;
const errors = [];
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
  page.on('pageerror', error => errors.push(error.message));
  await page.route('http://lysilogy.test/**', async route => {
    const { pathname } = new URL(route.request().url());
    if (pathname === '/api/library') return route.fulfill({ json: library });
    if (pathname === '/api/library/scan') { scans++; return route.fulfill({ json: library }); }
    if (pathname === '/api/library/import') { imports++; return route.fulfill(imports === 1 ? { status: 400, json: { message: 'Fixture import failed' } } : { json: { paper: papers[0], library } }); }
    if (pathname === '/api/queue') return route.fulfill({ json: { jobs: [] } });
    const paper = papers.find(paper => pathname === `/api/papers/${paper.id}`);
    if (paper) return route.fulfill({ json: { paper, analysis: null } });
    if (pathname.endsWith('/source')) return route.fulfill({ body: visualFixture(), contentType: 'application/pdf' });
    if (pathname.endsWith('/map')) return route.fulfill({ json: { layout: { schema_version: 1, pages: [] }, highlights: [] } });
    if (pathname.endsWith('/reading-index')) return route.fulfill({ json: { schema_version: 3, text: '', pages: [], tokens: [], objects: { word: [], WORD: [], sentence: [], paragraph: [] }, figures: [], gaps: [] } });
    if (pathname.startsWith('/api/')) return route.fulfill({ status: 404, json: { message: 'Unused endpoint' } });
    const file = path.join(path.resolve('dist'), pathname === '/' ? 'index.html' : pathname.slice(1));
    return route.fulfill({ body: await readFile(file), contentType: /\.m?js$/.test(file) ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html' });
  });
  await page.goto('http://lysilogy.test/#home');
  await page.locator('.paper-card').first().waitFor();
  assert.equal(await page.locator('.library-rail, .rail-scrim').count(), 0);
  const layoutButton = name => page.getByRole('group', { name: 'Paper layout', exact: true }).getByRole('button', { name, exact: true });
  const columns = () => page.locator('.paper-grid').evaluate(node => getComputedStyle(node).gridTemplateColumns.split(' ').length);
  for (const [name, value, count] of [['Gallery', 'gallery', 4], ['Two columns', 'two-column', 2], ['List', 'list', 1], ['Details', 'details', 1]]) {
    await layoutButton(name).click();
    assert.equal(await page.locator('.paper-grid').getAttribute('data-layout'), value);
    assert.equal(await columns(), count);
    assert.equal(await page.locator('.paper-card').count(), papers.length);
    if (value === 'details') { assert.equal(await page.getByText(abstract, { exact: true }).count(), 2); assert.equal(await page.getByText('Summary', { exact: true }).count(), 1); assert.equal(await page.getByText('No abstract available', { exact: true }).count(), 3); }
    await page.screenshot({ path: `/tmp/lysilogy-home-${value}.png`, fullPage: true });
  }
  await page.reload(); await page.locator('.paper-card').first().waitFor();
  assert.equal(await page.locator('.paper-grid').getAttribute('data-layout'), 'details', 'layout persists');
  await page.getByRole('button', { name: 'Mapped', exact: true }).click();
  assert.equal(await page.locator('.paper-card').count(), 3);
  const search = page.getByRole('searchbox', { name: 'Search papers' });
  await search.fill('category'); assert.equal(await page.locator('.paper-card').count(), 1);
  await search.fill('no such paper'); await page.getByRole('button', { name: 'Show all papers' }).click();
  assert.equal(await page.locator('.paper-card').count(), 6);
  await layoutButton('Two columns').click(); await page.locator('.paper-card').first().focus();
  await page.keyboard.press('ArrowRight'); assert.equal(await page.locator('.paper-card').nth(1).evaluate(node => node === document.activeElement), true);
  await page.keyboard.press('ArrowDown'); assert.equal(await page.locator('.paper-card').nth(3).evaluate(node => node === document.activeElement), true);
  await page.keyboard.press('F1'); await page.getByRole('dialog', { name: 'Switch article' }).waitFor();
  await page.getByRole('button', { name: 'Mapped only', exact: true }).click(); assert.equal(await page.locator('.switcher-results').getByRole('option').count(), 3);
  await page.getByRole('textbox', { name: 'Fuzzy-find an article' }).fill('ctgry'); assert.equal(await page.locator('.switcher-results').getByRole('option').count(), 1);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Rescan', exact: true }).click();
  await page.waitForFunction(() => document.body.textContent.includes('Scanning')); assert.equal(scans, 1);
  const openImport = async () => { await page.getByRole('button', { name: 'Add', exact: true }).click(); await page.getByRole('menuitem', { name: 'Import PDF from URL' }).click(); await page.getByLabel('PDF URL', { exact: true }).fill('https://example.org/paper.pdf'); await page.getByRole('button', { name: 'Import PDF', exact: true }).click(); };
  await openImport(); await page.locator('.import-pdf-dialog [role="alert"]').waitFor();
  assert.equal(await page.locator('.import-pdf-dialog').evaluate(node => node.open), true);
  await page.keyboard.press('Escape'); assert.equal(await page.locator('.import-pdf-dialog').count(), 0); assert.equal(new URL(page.url()).hash, '#home');
  await page.setViewportSize({ width: 390, height: 844 });
  for (const name of ['Gallery', 'Two columns', 'List', 'Details']) { await layoutButton(name).click(); assert.equal(await columns(), 1); assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true); await page.screenshot({ path: `/tmp/lysilogy-home-mobile-${name.replaceAll(' ', '-')}.png`, fullPage: true }); }
  await openImport(); await page.locator('.pdf-reader').waitFor(); assert.equal(imports, 2); assert.equal(await page.locator('.import-pdf-dialog').count(), 0);
  await page.keyboard.press(':');
  await page.locator('.command-menu input').fill('import');
  await page.keyboard.press('Enter');
  await page.getByLabel('PDF URL', { exact: true }).waitFor();
  await page.keyboard.press('Tab');
  assert.equal(await page.locator('.import-pdf-dialog').evaluate(node => node.open && node.contains(document.activeElement)), true, 'Tab stays in the import dialog');
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.pdf-reader').count(), 1, 'closing the import dialog preserves the reader');
  assert.deepEqual(errors, []);
  console.log('Home layouts, navigation, finder, and import checks passed');
} finally { await browser.close(); }
