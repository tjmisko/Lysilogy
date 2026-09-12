import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { pressVisualFit, visualFixture } from './pdf-fit-checks.mjs';

// Synthetic pages and mocked API responses keep this check independent of a library/backend.
const root = path.resolve('dist');
const id = '1234567890abcdef';
const paper = { id, metadata: { title: 'Asymmetric PDF pages', authors: ['Test Author'], year: 2026, page_count: 4, subject: null },
  relative_path: 'fixture.pdf', status: { state: 'discovered' }, analyzed_at: null, one_line_summary: null };
const pdf = visualFixture();
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, reducedMotion: 'reduce' });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('http://lysilogy.test/**', async route => {
    const { pathname } = new URL(route.request().url());
    if (pathname === '/api/library') return route.fulfill({ json: { name: 'Synthetic library', papers: [paper] } });
    if (pathname === '/api/queue') return route.fulfill({ json: { jobs: [] } });
    if (pathname === `/api/papers/${id}`) return route.fulfill({ json: { paper, analysis: null } });
    if (pathname.endsWith('/source')) return route.fulfill({ body: pdf, contentType: 'application/pdf' });
    if (pathname.endsWith('/map')) return route.fulfill({ json: { layout: { schema_version: 1, pages: [] }, highlights: [] } });
    if (pathname.endsWith('/reading-index')) return route.fulfill({ json: {
      schema_version: 3, text: '', pages: [], tokens: [], objects: { word: [], WORD: [], sentence: [], paragraph: [] }, figures: [], gaps: [],
    } });
    if (pathname.startsWith('/api/')) return route.fulfill({ status: 404, json: { message: 'Unused fixture endpoint' } });
    const file = path.join(root, pathname === '/' ? 'index.html' : pathname.slice(1));
    const contentType = /\.m?js$/.test(file) ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html';
    return route.fulfill({ body: await readFile(file), contentType });
  });

  const ready = number => page.waitForFunction(number =>
    document.querySelector(`[data-pdf-page="${number}"] canvas`)?.dataset.rendered === 'true', number);
  const geometry = () => page.locator('[data-pdf-page="1"] .pdf-page-window').evaluate(node => {
    const box = node.getBoundingClientRect(), canvas = node.querySelector('canvas').getBoundingClientRect();
    return { width: box.width, height: box.height, canvasWidth: canvas.width, canvasHeight: canvas.height,
      left: box.left - canvas.left, top: box.top - canvas.top };
  });
  const aligned = async () => {
    const box = await geometry();
    for (const difference of [box.width - box.canvasWidth, box.height - box.canvasHeight, box.left, box.top]) {
      assert.ok(Math.abs(difference) < 1, `page window must match the rendered canvas: ${JSON.stringify(box)}`);
    }
  };

  await page.goto(`http://lysilogy.test/#paper=${id}`);
  await ready(1);
  await page.locator('.pdf-reader').focus();
  await pressVisualFit(page, 'H');
  await aligned();
  const before = await geometry();
  await page.keyboard.press('R');
  await page.waitForFunction(() => document.querySelector('.pdf-reader')?.dataset.axis === 'horizontal');
  await ready(2);
  await page.screenshot({ path: '/tmp/lysilogy-pdf-ink-horizontal.png' });
  await aligned();
  const after = await geometry();
  assert.ok(Math.abs(before.width - after.width) < 1, 'switching to continuous flow preserves the rendered visual crop');

  // Returning to paged mode, changing fit, and resizing must keep the same invariant.
  await page.keyboard.press('P');
  await ready(1);
  await aligned();
  await pressVisualFit(page, 'W');
  await page.keyboard.press('P');
  await ready(2);
  await aligned();
  await page.setViewportSize({ width: 960, height: 720 });
  await page.waitForFunction(() => {
    const host = document.querySelector('.pdf-viewport'), box = host.querySelector('.pdf-page-window');
    return Math.abs(box.getBoundingClientRect().width - (host.clientWidth - 28)) < 1;
  });
  await ready(1);
  await aligned();
  await page.keyboard.press('H');
  await ready(1);
  await aligned();
  await pressVisualFit(page, 'H');
  await page.keyboard.press('R');
  await ready(1);
  await aligned();

  const readerFilter = await page.locator('.pdf-canvas').first().evaluate(node => getComputedStyle(node).filter);
  assert.notEqual(readerFilter, 'none');
  await page.keyboard.press('Escape');
  await page.locator('.home-page').waitFor();
  const thumbnail = page.locator('.paper-preview img');
  await thumbnail.waitFor();
  const sameFilter = filter => page.waitForFunction(filter => {
    const images = [...document.querySelectorAll('.paper-preview img')];
    return images.length > 0 && images.every(node => getComputedStyle(node).filter === filter);
  }, filter);
  await sameFilter(readerFilter);
  const imageBefore = await thumbnail.getAttribute('src');
  await page.screenshot({ path: '/tmp/lysilogy-home-dark.png' });
  await page.keyboard.press('I');
  await sameFilter('none');
  assert.equal(await thumbnail.getAttribute('src'), imageBefore, 'ink changes reuse the thumbnail image');
  await page.screenshot({ path: '/tmp/lysilogy-home-light.png' });
  await page.keyboard.press('i');
  await sameFilter('none');
  await page.getByRole('searchbox', { name: 'Search papers' }).focus();
  await page.keyboard.press('I');
  assert.equal(await page.getByRole('searchbox').inputValue(), 'I', 'I remains ordinary text inside search');
  await page.getByRole('searchbox').fill('');
  await page.keyboard.press('Escape');
  await sameFilter('none');
  await page.keyboard.press('Enter');
  await ready(1);
  assert.equal(await page.locator('.pdf-canvas').evaluate(node => getComputedStyle(node).filter), 'none', 'home ink carries into the reader');
  await page.keyboard.press('I');
  await page.waitForFunction(filter => getComputedStyle(document.querySelector('.pdf-canvas')).filter === filter, readerFilter);
  await page.keyboard.press('Escape');
  await sameFilter(readerFilter);
  assert.equal(await thumbnail.getAttribute('src'), imageBefore, 'cached previews inherit the current ink on returning home');
  assert.deepEqual(errors, []);
  console.log('PDF ink smoke checks passed');
} finally {
  await browser.close();
}
