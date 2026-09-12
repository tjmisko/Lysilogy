import assert from 'node:assert/strict';

export async function pressVisualFit(page, key) {
  await page.keyboard.press('g');
  // Real typing includes a Shift keydown between the prefix and its suffix.
  await page.keyboard.down('Shift');
  await page.keyboard.press(`Key${key}`);
  await page.keyboard.up('Shift');
  await page.waitForFunction(fit => document.querySelector('.pdf-reader')?.dataset.fitBounds === 'visual'
    && document.querySelector('.pdf-reader')?.dataset.fit === fit
    && document.querySelector('.pdf-canvas')?.dataset.rendered === 'true', key === 'H' ? 'height' : 'width');
}

export async function checkPdfFits(page, { cropped = false } = {}) {
  const ready = () => page.waitForFunction(() => document.querySelector('.pdf-canvas')?.dataset.rendered === 'true');
  const scale = () => page.locator('.pdf-page-surface').first().evaluate(node => Number(node.style.getPropertyValue('--total-scale-factor')));
  const fitInside = async key => {
    const geometry = await page.locator('.pdf-page-window').first().evaluate(node => {
      const box = node.getBoundingClientRect(), viewport = node.closest('.pdf-viewport');
      const text = [...node.querySelectorAll('.pdf-text-layer span')].filter(span => span.textContent.trim()).map(span => span.getBoundingClientRect());
      return { width: box.width, height: box.height, viewportWidth: viewport.clientWidth, viewportHeight: viewport.clientHeight,
        textInside: text.every(r => r.left >= box.left - 1 && r.right <= box.right + 1 && r.top >= box.top - 1 && r.bottom <= box.bottom + 1) };
    });
    assert.ok(geometry[key === 'H' ? 'height' : 'width'] <= geometry[key === 'H' ? 'viewportHeight' : 'viewportWidth'], 'the fitted content fits its viewport');
    // Section text may contain masked spans retained for source offsets.
    if (!cropped) assert.ok(geometry.textInside, 'the visual margin contains all selectable text');
  };

  await page.locator('.pdf-reader').focus();
  await page.keyboard.press('H'); await ready();
  const fullHeight = await scale();
  await pressVisualFit(page, 'H');
  const visualHeight = await scale();
  if (!cropped) assert.ok(visualHeight > fullHeight * 1.03, 'gH enlarges content beyond page-height fitting');
  await fitInside('H');
  await page.keyboard.press('+'); await ready();
  assert.ok(await scale() > visualHeight, 'manual zoom works from visual fitting');
  await pressVisualFit(page, 'H');
  assert.ok(Math.abs(await scale() - visualHeight) < .01, 'gH resets manual zoom');
  await page.keyboard.press('W'); await ready();
  const fullWidth = await scale();
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit-bounds'), 'page');
  await pressVisualFit(page, 'W');
  if (!cropped) assert.ok(await scale() > fullWidth * 1.03, 'gW enlarges content beyond page-width fitting');
  await fitInside('W');

  await page.keyboard.press('H'); await ready();
  await page.keyboard.press('g'); await page.keyboard.press('Escape'); await page.keyboard.press('W'); await ready();
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit-bounds'), 'page', 'Escape cancels only the pending prefix');
  await page.keyboard.press('g'); await page.keyboard.press('z'); await page.keyboard.press('H'); await ready();
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit-bounds'), 'page', 'unrelated keys cancel the prefix');
  await page.keyboard.press('g'); await page.locator('.pdf-viewport').focus(); await page.keyboard.press('W'); await ready();
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit-bounds'), 'page', 'focus changes cancel the prefix');
  await page.keyboard.press('g'); await page.keyboard.press('g'); await page.keyboard.press('H'); await ready();
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit-bounds'), 'page', 'gg consumes the prefix');
  await page.keyboard.press('g');
  await page.waitForTimeout(500);
  await page.keyboard.press('W'); await ready();
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-fit-bounds'), 'page', 'an expired g does not modify a later W');
  await page.keyboard.press('H'); await ready();
}

function visualFixture() {
  const content = 'BT /F1 16 Tf 180 650 Td (Asymmetric text) Tj ET 1 0 1 rg 80 260 380 220 re f';
  const pages = [content, content,
    'q 340 0 0 410 70 210 cm BI /W 2 /H 2 /BPC 8 /CS /G /F /AHx ID 00888800> EI Q', ''];
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R 10 0 R] /Count 4 >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
  pages.forEach((stream, i) => {
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] ${i === 1 ? '/Rotate 90' : ''} /Resources << /Font << /F1 3 0 R >> >> /Contents ${objects.length + 2} 0 R >>`);
    objects.push(`<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`);
  });
  let pdf = '%PDF-1.4\n'; const offsets = [0];
  objects.forEach((object, i) => { offsets.push(Buffer.byteLength(pdf)); pdf += `${i + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n` + offsets.slice(1).map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('');
  return Buffer.from(pdf + `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`);
}

export async function checkVisualFitPages(page, sourceUrl) {
  await page.route(sourceUrl, route => route.fulfill({ body: visualFixture(), contentType: 'application/pdf' }));
  await page.reload();
  const ready = number => page.waitForFunction(number => document.querySelector(`[data-pdf-page="${number}"] canvas`)?.dataset.rendered === 'true', number);
  const size = () => page.locator('.pdf-page-window').first().boundingBox();
  for (let number = 1; number <= 4; number++) {
    if (number > 1) await page.keyboard.press('l');
    await ready(number);
    await page.keyboard.press('H'); await ready(number);
    const normal = await page.locator('.pdf-page-surface').first().boundingBox();
    await pressVisualFit(page, 'H'); await ready(number);
    const surface = await page.locator('.pdf-page-surface').first().boundingBox();
    if (number === 4) assert.ok(Math.abs(surface.height - normal.height) < 1, 'a blank page falls back to the paper bounds');
    else assert.ok(surface.height > normal.height * 1.25, `visual fitting enlarges page ${number}, including rotated pages and scans`);
    if (number === 1) {
      const window = await size();
      // Figure extends farther left/right than the text. Both must remain visible.
      const scale = surface.width / 612;
      assert.ok(surface.x + 80 * scale > window.x + 5);
      assert.ok(surface.x + 460 * scale < window.x + window.width - 5);
      await page.screenshot({path:'/tmp/lysilogy-visual-fit.png'});
    }
  }
  await page.keyboard.press('h'); await page.keyboard.press('h'); await page.keyboard.press('h'); await ready(1);
  await pressVisualFit(page, 'W');
  await page.keyboard.press('2'); await ready(2);
  const widths = await page.locator('.pdf-page-window').evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect().width));
  const viewportWidth = await page.locator('.pdf-viewport').evaluate(node => node.clientWidth);
  assert.equal(widths.length, 2);
  assert.ok(widths.reduce((a, b) => a + b, 0) + 18 < viewportWidth, 'visual widths fit a two-page spread');
  await page.keyboard.press('2');
  await page.keyboard.press('P'); await pressVisualFit(page, 'W');
  await page.setViewportSize({width:900,height:800}); await ready(1);
  await page.waitForFunction(() => {
    const host = document.querySelector('.pdf-viewport'), window = host.querySelector('.pdf-page-window');
    return Math.abs(window.getBoundingClientRect().width - (host.clientWidth - 28)) < 2;
  });
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-flow'), 'continuous');
  await page.keyboard.press('R'); await pressVisualFit(page, 'H');
  assert.equal(await page.locator('.pdf-reader').getAttribute('data-axis'), 'horizontal');
  await page.setViewportSize({width:1280,height:800});
}
