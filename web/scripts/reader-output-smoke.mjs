// Renders the built app against the shared reader fixtures
// (tests/fixtures/reader/) with a mocked API, checks that the Abstract view,
// the Overview's two segmentations and the review panel show every new field,
// that an analysis written before those fields still renders, and saves
// screenshots. Run after `npm run build`:
//   node scripts/reader-output-smoke.mjs [screenshot-dir]
import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

import { chromium } from "playwright";

const root = path.resolve("..");
const shots = path.resolve(process.argv[2] ?? "/tmp/lysilogy-reader-output");
await mkdir(shots, { recursive: true });
const readJson = async (relative) => JSON.parse(await readFile(path.join(root, relative), "utf8"));

const current = await readJson("tests/fixtures/reader/analysis-current.json");
const legacy = await readJson("tests/fixtures/reader/analysis-legacy.json");
const rubric = await readJson("eval/review/rubric.json");
const manifest = await readJson("eval/review/papers.json");
const PAGES = 6;

// The fixture's anchors point at a real paper's text layer; the smoke PDF is
// synthetic, so the page map uses page spans (dashed, inferred tiles).
const forMap = structuredClone(current);
for (const section of forMap.sections) {
  section.source_span = null;
  for (const quote of section.key_quotes) quote.anchor = null;
}

const paperFor = (id, analysis, title) => ({
  id,
  metadata: { title, authors: ["Fixture Author"], year: 1990, page_count: PAGES, subject: null },
  relative_path: `${title}.pdf`,
  status: { state: "ready" },
  analyzed_at: analysis.generated_at,
  one_line_summary: analysis.thesis,
  author_abstract: analysis.author_abstract,
});
const papers = {
  "00000000000000c1": { paper: paperFor("00000000000000c1", current, "A Sudden Arrival: Local Labour Markets After an Unexpected Influx"), analysis: forMap },
  "00000000000000a1": { paper: paperFor("00000000000000a1", legacy, "A Sudden Arrival (analysed before the outline existed)"), analysis: legacy },
};

/** A small PDF whose pages print the fixture's headings, so the page map has something to show. */
function fixturePdf() {
  const escape = (text) => text.replace(/[\\()]/gu, (match) => `\\${match}`);
  const pages = Array.from({ length: PAGES }, (_, index) => {
    const number = index + 1;
    const headings = current.outline.filter((entry) => entry.page === number);
    const lines = [];
    let y = 720;
    if (number === 1) { lines.push(`BT /F1 17 Tf 72 ${y} Td (${escape(papers["00000000000000c1"].paper.metadata.title)}) Tj ET`); y -= 40; }
    for (const heading of headings) {
      lines.push(`BT /F1 13 Tf 72 ${y} Td (${escape(`${heading.number ?? ""} ${heading.title}`.trim())}) Tj ET`);
      y -= 24;
      for (let line = 0; line < 9; line++, y -= 15) lines.push(`0.55 g 72 ${y} 468 7 re f 0 g`);
      y -= 18;
    }
    lines.push(`BT /F1 9 Tf 300 40 Td (${number}) Tj ET`);
    return lines.join("\n");
  });
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${pages.map((_, i) => `${4 + i * 2} 0 R`).join(" ")}] /Count ${PAGES} >>`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>"];
  pages.forEach((stream) => {
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${objects.length + 2} 0 R >>`);
    objects.push(`<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`);
  });
  let pdf = "%PDF-1.4\n";
  const offsets = [];
  objects.forEach((object, i) => { offsets.push(Buffer.byteLength(pdf)); pdf += `${i + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}`;
  return Buffer.from(`${pdf}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`);
}

const layout = {
  schema_version: 1,
  pages: Array.from({ length: PAGES }, (_, index) => {
    const number = index + 1;
    const sentences = Array.from({ length: 6 }, (_, at) => ({
      id: `p${String(number).padStart(4, "0")}-s${String(at + 1).padStart(5, "0")}`,
      page: number, start_token: at, end_token: at,
      text: `Fixture sentence ${at + 1} on page ${number}.`,
      rects: [{ x_min: 72, y_min: 90 + at * 110, x_max: 540, y_max: 104 + at * 110 }],
    }));
    return {
      number, width: 612, height: 792, sentences,
      tokens: sentences.map((sentence, at) => ({ index: at, text: sentence.text, line: at, rects: sentence.rects })),
    };
  }),
};

const snapshotFields = (analysis) => Object.fromEntries(Object.entries(analysis)
  .filter(([key]) => ["thesis", "author_abstract", "abstract", "outsider_brief", "context", "outline", "section", "quality_warnings"].some((prefix) => key.startsWith(prefix))));
const savedReviews = [];

const errors = [];
// LYSILOGY_CHROMIUM points at a browser when Playwright's bundled one is not installed.
const browser = await chromium.launch({ headless: true, executablePath: process.env.LYSILOGY_CHROMIUM || undefined });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("http://lysilogy.test/**", async (route) => {
    const request = route.request();
    const { pathname } = new URL(request.url());
    const match = /^\/api\/papers\/([0-9a-f]{16})(\/.*)?$/u.exec(pathname);
    const entry = match === null ? undefined : papers[match[1]];
    if (pathname === "/api/library") return route.fulfill({ json: { name: "Reading room", papers: Object.values(papers).map((item) => item.paper), directories: [] } });
    if (pathname === "/api/queue") return route.fulfill({ json: { jobs: [] } });
    if (pathname === "/api/review/rubric") return route.fulfill({ json: rubric });
    if (pathname === "/api/review/manifest") return route.fulfill({ json: manifest });
    if (pathname === "/api/review/progress") return route.fulfill({ json: [] });
    if (entry !== undefined) {
      const rest = match[2] ?? "";
      if (rest === "") return route.fulfill({ json: { paper: entry.paper, analysis: entry.analysis } });
      if (rest === "/map") return route.fulfill({ json: { layout, highlights: [] } });
      if (rest === "/source") return route.fulfill({ body: fixturePdf(), contentType: "application/pdf" });
      if (rest === "/review" && request.method() === "PUT") {
        const input = request.postDataJSON();
        savedReviews.push(input);
        const now = new Date().toISOString();
        return route.fulfill({ json: { ...input, paper_id: entry.paper.id, paper_title: entry.paper.metadata.title, manifest_id: null,
          snapshot: { snapshot_id: input.snapshot_id, generated_at: entry.analysis.generated_at, pipeline_version: entry.analysis.pipeline_version ?? null, provider: entry.analysis.provider, fields: snapshotFields(entry.analysis) },
          created_at: now, updated_at: now } });
      }
      if (rest === "/review") {
        return route.fulfill({ json: { paper_id: entry.paper.id, paper_title: entry.paper.metadata.title, manifest_id: null,
          snapshot: { snapshot_id: `s-${entry.paper.id}`, generated_at: entry.analysis.generated_at, pipeline_version: entry.analysis.pipeline_version ?? null, provider: entry.analysis.provider, fields: snapshotFields(current === entry.analysis ? current : entry.analysis) },
          review: null, earlier: [] } });
      }
    }
    if (pathname.startsWith("/api/")) return route.fulfill({ status: 404, json: { message: "Unused endpoint" } });
    const file = path.join(path.resolve("dist"), pathname === "/" ? "index.html" : pathname.slice(1));
    const type = /\.m?js$/u.test(file) ? "text/javascript" : file.endsWith(".css") ? "text/css" : file.endsWith(".svg") ? "image/svg+xml"
      : file.endsWith(".wasm") ? "application/wasm" : "text/html";
    return route.fulfill({ body: await readFile(file), contentType: type });
  });

  // Abstract view: why it matters, the three-part context with source classes, read-alongside, warnings.
  await page.goto("http://lysilogy.test/#paper=00000000000000c1");
  await page.locator(".abstract-view").waitFor();
  const why = page.locator(".why-it-matters");
  await why.waitFor();
  assert.equal(await why.locator(".context-claim").count(), current.context.lead.length);
  assert.equal(await page.locator(".reading-context.is-node .reading-context-card").count(), 3);
  for (const basis of ["paper", "dossier", "background"]) {
    assert.ok(await page.locator(`.abstract-view .context-basis.basis-${basis}`).count() > 0, `a ${basis} marker is shown`);
  }
  assert.ok(await page.locator(".context-claim-quote").count() >= 2, "paper claims show their quotes");
  assert.equal(await page.locator(".read-alongside-item").count(), current.context.read_alongside.length);
  assert.equal(await page.locator(".read-alongside-item").nth(0).getByRole("button", { name: "Open in reader" }).count(), 1);
  assert.equal(await page.locator(".read-alongside-item").nth(1).getByRole("button", { name: "Fetch PDF" }).count(), 1);
  assert.equal(await page.locator(".read-alongside-item").nth(2).getByText("Not in the citation graph").count(), 1);
  assert.match(await page.locator(".pipeline-checks summary").innerText(), /2 pipeline checks flagged/u);
  await page.setViewportSize({ width: 1440, height: 2300 });
  await page.screenshot({ path: path.join(shots, "abstract-view.png") });
  await page.setViewportSize({ width: 1440, height: 1000 });

  // Overview: logical units on the page map, with the headings each groups.
  await page.getByRole("button", { name: /Continue to overview/u }).click();
  await page.locator(".segmentation-toggle").waitFor();
  await page.locator(".section-boxes button").first().waitFor();
  assert.equal(await page.locator(".unit-index > li").count(), current.sections.length);
  assert.match(await page.locator(".unit-index > li").first().innerText(), /§ 1–2\.1/u);
  assert.ok(await page.locator(".section-box-range").count() > 0, "tiles name the headings they group");
  await page.waitForFunction(() => document.querySelectorAll(".source-page canvas[data-rendered='true']").length > 0).catch(() => {});
  await page.screenshot({ path: path.join(shots, "overview-units.png") });

  // O switches to the printed outline; j/k move; Enter opens the unit holding the heading.
  await page.locator(".main-stage").focus();
  await page.keyboard.press("O");
  await page.locator(".outline-view").waitFor();
  assert.equal(await page.locator(".outline-list > li").count(), current.outline.length);
  await page.keyboard.press("j");
  await page.keyboard.press("j");
  await page.keyboard.press("j");
  assert.equal(await page.locator(".outline-list > li.is-active").getAttribute("data-outline-index"), "3");
  assert.match(await page.locator(".outline-list > li").nth(1).innerText(), /Why a sudden arrival/u);
  await page.screenshot({ path: path.join(shots, "overview-outline.png") });
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.querySelector(".app-shell")?.classList.contains("has-section-focus")
    || document.querySelector(".context-panel, .section-focus") !== null);
  await page.keyboard.press("Escape");
  await page.keyboard.press("O");
  await page.locator(".section-boxes").waitFor();

  // Review panel: the outline, the logical grouping and the context with source classes.
  await page.keyboard.press("F2");
  await page.locator(".review-panel").waitFor();
  await page.locator(".review-outline").waitFor();
  assert.equal(await page.locator(".review-outline > li").count(), current.outline.length);
  assert.equal(await page.locator(".review-units > li").count(), current.sections.length);
  assert.match(await page.locator(".review-units > li").first().innerText(), /Groups § 1–2\.1/u);
  assert.ok(await page.locator(".review-panel .review-basis.basis-paper").count() > 0);
  assert.ok(await page.locator(".review-panel .review-warnings").count() > 0, "component warnings are shown");
  await page.setViewportSize({ width: 1440, height: 1400 });
  await page.screenshot({ path: path.join(shots, "review-panel.png") });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.keyboard.press("Escape");

  // An analysis written before the new fields still renders, with no outline to switch to.
  await page.goto("http://lysilogy.test/#paper=00000000000000a1");
  await page.locator(".abstract-view").waitFor();
  assert.equal(await page.locator(".why-it-matters").count(), 0);
  assert.equal(await page.locator(".read-alongside").count(), 0);
  await page.getByRole("button", { name: /Continue to overview/u }).click();
  await page.locator(".segmentation-toggle").waitFor();
  assert.equal(await page.getByRole("button", { name: /^Outline/u }).isDisabled(), true);
  assert.equal(await page.locator(".unit-index").count(), 0);
  await page.locator(".main-stage").focus();
  await page.keyboard.press("O");
  assert.equal(await page.locator(".outline-view").count(), 0);

  assert.deepEqual(errors, []);
  console.log(`Reader output checks passed; screenshots in ${shots}`);
} finally {
  await browser.close();
}
