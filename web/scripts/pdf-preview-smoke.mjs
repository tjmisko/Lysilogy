import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

// Serve only generated papers and built assets; no reading-library files are used.
function makePdf() {
  const text = 'BT /F1 24 Tf 48 730 Td (A cached paper preview) Tj ET';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    `<< /Length ${text.length} >>\nstream\n${text}\nendstream`,
  ];
  let pdf = '%PDF-1.4\n'; const offsets = [];
  objects.forEach((object, index) => { offsets.push(pdf.length); pdf += `${index + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = pdf.length;
  pdf += `xref\n0 6\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}`;
  return Buffer.from(`${pdf}trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`);
}
const pdf = makePdf();
const papers = Array.from({ length: 100 }, (_, index) => ({
  id: index.toString(16).padStart(16, '0'),
  metadata: { title: `Paper ${String(index).padStart(3, '0')}`, authors: ['Synthetic Author'], year: 2026, page_count: 1, subject: null },
  relative_path: `synthetic-${index}.pdf`, status: { state: 'discovered' }, analyzed_at: null, one_line_summary: null,
}));
const root = path.resolve('dist');
async function waitUntil(predicate) {
  const deadline = Date.now() + 30_000;
  while (!await predicate()) {
    assert.ok(Date.now() < deadline, 'Preview condition timed out');
    await new Promise(resolve => setTimeout(resolve, 25));
  }
}
const browser = await chromium.launch({ headless: true });
try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const requests = new Map(); const errors = [];
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let hold = true;
  await context.route('http://lysilogy.test/**', async route => {
    const url = new URL(route.request().url());
    if (url.pathname === '/api/library') return route.fulfill({ json: { name: 'Preview fixtures', papers } });
    if (url.pathname === '/api/queue') return route.fulfill({ json: { jobs: [] } });
    const source = url.pathname.match(/^\/api\/papers\/([^/]+)\/source$/);
    if (source) {
      requests.set(source[1], (requests.get(source[1]) ?? 0) + 1);
      if (hold) await held;
      return route.fulfill({ body: pdf, contentType: 'application/pdf' });
    }
    if (url.pathname.startsWith('/api/')) throw new Error(`Unexpected request: ${url.pathname}`);
    const file = path.join(root, url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
    const contentType = /\.m?js$/.test(file) ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html';
    return route.fulfill({ body: await readFile(file), contentType });
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  const waitForNearbyImages = () => page.waitForFunction(() => {
    const viewport = document.querySelector('.main-stage').getBoundingClientRect();
    return Array.from(document.querySelectorAll('.paper-preview')).filter(node => {
      const { top, bottom } = node.getBoundingClientRect();
      return top < viewport.bottom + 1200 && bottom > viewport.top - 1200;
    }).every(node => node.querySelector('img')?.complete);
  });
  await page.goto('http://lysilogy.test/#home');
  await page.waitForFunction(() => document.activeElement?.classList.contains('paper-card'));
  // Wait for all render slots to be occupied before scrolling out of their range.
  await waitUntil(() => requests.size === 3);
  assert.equal(requests.size, 3);
  await page.keyboard.press('End');
  await page.waitForFunction(() => document.activeElement?.getAttribute('data-paper-id') === '0000000000000063');
  hold = false; release();
  await page.locator('.paper-card').last().locator('img').waitFor();
  await waitForNearbyImages();
  await page.keyboard.press('Home');
  await page.locator('.paper-card').first().locator('img').waitFor();
  assert.equal(requests.get(papers[0].id), 1, 'scrolling away must not discard an active render');
  // A card more than 160px below the viewport is already rendered before scrolling.
  const ahead = page.locator('.paper-card').nth(12);
  assert.ok(await ahead.evaluate(node => node.getBoundingClientRect().top > innerHeight + 160));
  await ahead.locator('img').waitFor();
  await waitForNearbyImages();
  const search = page.getByRole('searchbox', { name: 'Search papers', exact: true });
  await search.fill('Paper 000');
  await page.locator('.paper-card img').waitFor();
  await search.fill('');
  await page.locator('.paper-card').nth(12).locator('img').waitFor();
  assert.ok([...requests.values()].every(count => count === 1), 'filter remounts share and reuse previews');
  // Wait for the cache's background writes, then destroy the whole page/module cache.
  await waitUntil(() => page.evaluate(async expected => {
    const db = await new Promise((resolve, reject) => {
      const request = indexedDB.open('lysilogy-pdf-previews-v1', 1);
      request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    });
    try {
      return await new Promise(resolve => {
        const request = db.transaction('previews').objectStore('previews').count();
        request.onsuccess = () => resolve(request.result >= expected);
      });
    } finally { db.close(); }
  }, requests.size));
  const beforeReload = new Map(requests);
  await page.reload();
  await page.locator('.paper-card').nth(12).locator('img').waitFor();
  await page.keyboard.press('End');
  await page.locator('.paper-card').last().locator('img').waitFor();
  assert.deepEqual(requests, beforeReload, 'reloads restore thumbnails without downloading PDFs');
  await page.keyboard.press('Home');
  await page.locator('.paper-card').first().locator('img').waitFor();
  await page.screenshot({ path: '/tmp/lysilogy-preview-cache.png' });

  // An expired entry is refreshed; a blocked storage API still renders normally.
  await page.evaluate(async id => {
    const db = await new Promise(resolve => {
      const request = indexedDB.open('lysilogy-pdf-previews-v1', 1);
      request.onsuccess = () => resolve(request.result);
    });
    await new Promise((resolve, reject) => {
      const transaction = db.transaction('previews', 'readwrite');
      const store = transaction.objectStore('previews');
      const request = store.get(`/api/papers/${id}/source`);
      request.onsuccess = () => store.put({ ...request.result, savedAt: 0 });
      transaction.oncomplete = resolve; transaction.onabort = () => reject(transaction.error);
    });
    db.close();
  }, papers[0].id);
  await page.reload();
  await page.locator('.paper-card').first().locator('img').waitFor();
  assert.equal(requests.get(papers[0].id), 2, 'expired previews are rendered again');
  await page.addInitScript(() => Object.defineProperty(window, 'indexedDB', { get() { throw new Error('Storage disabled'); } }));
  await page.reload();
  await page.locator('.paper-card').nth(12).locator('img').waitFor();
  assert.equal(requests.get(papers[0].id), 3, 'memory/render fallback works without IndexedDB');
  assert.deepEqual(errors, []);
  console.log('Preview smoke passed: eager loading, fast scrolling, filter reuse, persistent cache, expiry and storage fallback.');
} finally {
  await browser.close();
}
