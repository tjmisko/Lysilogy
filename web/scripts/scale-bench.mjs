// Live benchmark of a generated library served by the real isolated backend.
// No mocked API timings and no analysis, import, or note-writing requests.
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { chromium } from 'playwright';

export function searchCases(papers, count = 20) {
  assert(papers.length > 0);
  const collator = new Intl.Collator('en-US', { sensitivity: 'base' });
  return Array.from({ length: count }, (_, index) => {
    const paper = papers[(index * 491) % papers.length];
    const query = index % 3 === 0 ? paper.metadata.title
      : index % 3 === 1 ? paper.metadata.authors[0] : String(paper.metadata.year);
    const words = query.toLocaleLowerCase('en-US').split(/\s+/u).filter(Boolean);
    const matching = papers.filter(candidate => {
      const text = [candidate.metadata.title, ...candidate.metadata.authors,
        candidate.metadata.year ?? '', candidate.metadata.subject ?? ''].join(' ').toLocaleLowerCase('en-US');
      return words.every(word => text.includes(word));
    }).sort((left, right) => collator.compare(left.metadata.title, right.metadata.title));
    return { query, matches: matching.length, first: matching[0]?.id,
      status: matching.length === papers.length ? 'On the shelves' : `${matching.length} of ${papers.length} papers` };
  });
}

export async function measureBrowser(browser, baseUrl, count, repeats = 3, screenshot = null) {
  const address = new URL(baseUrl);
  assert(address.protocol === 'http:' && address.hostname === '127.0.0.1', 'isolated loopback server required');
  const home = [];
  const search = [];
  const errors = [];
  let domCards = 0;
  for (let repeat = 0; repeat < repeats; repeat++) {
    // A fresh context gives cold browser caches without deleting user storage.
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, locale: 'en-US', serviceWorkers: 'block' });
    try {
      const page = await context.newPage();
      page.setDefaultTimeout(120_000);
      page.on('pageerror', error => errors.push(error.message));
      await page.route('**/*', route => {
        const request = route.request();
        const url = new URL(request.url());
        if (url.origin !== address.origin || !['GET', 'HEAD'].includes(request.method())) {
          errors.push(`forbidden browser request: ${request.method()} ${url.pathname}`);
          return route.abort();
        }
        return route.continue();
      });
      await page.addInitScript(expected => {
        let scheduled = false;
        const observer = new MutationObserver(() => {
          const countLabel = document.querySelector('.home-counts strong');
          const card = document.querySelector('.paper-card');
          if (scheduled || Number(countLabel?.textContent) !== expected || !card) return;
          scheduled = true;
          // The second frame observes a paint opportunity after the populated
          // home committed. Thumbnails may still be loading; this is first render.
          requestAnimationFrame(() => requestAnimationFrame(() => {
            if (card.getBoundingClientRect().height > 0) window.scaleHomeMs = performance.now();
            observer.disconnect();
          }));
        });
        observer.observe(document, { childList: true, subtree: true });
      }, count);
      await page.goto(baseUrl, { waitUntil: 'commit', timeout: 120_000 });
      await page.waitForFunction(() => Number.isFinite(window.scaleHomeMs));
      home.push(await page.evaluate(() => window.scaleHomeMs));
      domCards = await page.locator('.paper-card').count();
      const papers = await page.evaluate(async () => (await (await fetch('/api/library')).json()).papers);
      assert.equal(papers.length, count, 'running app must contain the entire generated manifest');
      if (repeat === 0 && screenshot) await page.screenshot({ path: screenshot });
      for (const sample of searchCases(papers)) {
        const input = page.getByRole('searchbox', { name: 'Search papers' });
        // Begin every search with the full populated home, so removing old cards
        // remains inside the measurement. Resetting to an empty result first
        // would hide the current nonvirtualized grid's removal cost.
        await input.fill('');
        await page.waitForFunction(() => document.querySelector('.home-results-line [role="status"]')?.textContent === 'On the shelves');
        await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
        await page.evaluate(expected => {
          delete window.scaleSearchMs;
          const input = document.querySelector('input[aria-label="Search papers"]');
          input.addEventListener('input', () => {
            const start = performance.now();
            const check = () => {
              const status = document.querySelector('.home-results-line [role="status"]')?.textContent;
              const first = document.querySelector('.paper-card')?.dataset.paperId;
              if (status === expected.status && first === expected.first) {
                requestAnimationFrame(() => { window.scaleSearchMs = performance.now() - start; });
              } else requestAnimationFrame(check);
            };
            requestAnimationFrame(check);
          }, { once: true, capture: true });
        }, sample);
        await input.fill(sample.query);
        await page.waitForFunction(() => Number.isFinite(window.scaleSearchMs));
        search.push({ ...sample, milliseconds: await page.evaluate(() => window.scaleSearchMs) });
      }
    } finally {
      await context.close();
    }
  }
  assert.deepEqual(errors, [], 'browser must finish without errors or forbidden requests');
  return { schema_version: 1, papers: count, home_first_render_ms: home,
    search, dom_cards_at_home: domCards, browser_version: browser.version(),
    viewport: { width: 1280, height: 800 }, locale: 'en-US' };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [baseUrl, count, repeats, output, screenshot] = process.argv.slice(2);
  assert(Number.isInteger(Number(count)) && Number(count) >= 1 && Number(count) <= 10000);
  const browser = await chromium.launch({ headless: true });
  try {
    const observation = await measureBrowser(browser, baseUrl, Number(count), Number(repeats), screenshot);
    await writeFile(output, `${JSON.stringify(observation, null, 2)}\n`);
  } finally {
    await browser.close();
  }
}
