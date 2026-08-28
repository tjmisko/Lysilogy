import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";

import { chromium } from "playwright";

const paperId = "a72b0e4a80ecd9db";
const screenshot = process.argv[2] ?? "/tmp/lysilogy-atlas.png";
const screenshotVariant = (name) => {
  const parsed = path.parse(screenshot);
  return path.join(parsed.dir, `${parsed.name}-${name}${parsed.ext}`);
};
const root = path.resolve("..");
const analysis = JSON.parse(
  await readFile(path.join(root, ".lysilogy", "papers", paperId, "analysis.json"), "utf8"),
);
analysis.author_abstract = "For a number of years I have been familiar with the observation that the quality of programmers is a decreasing function of the density of go to statements in the programs they produce.";
analysis.schema_version = 4;
analysis.provider = "codex";
analysis.outsider_brief = "Dijkstra later wrote that the letter was often known only by its editor-supplied title, which became a reusable computing trope. Knuth's later response treated the dispute as a question of disciplined control structures rather than a literal ban.";
analysis.context_notes = [
  {
    text: "Dijkstra later wrote that the letter was often known only by its editor-supplied title, which became a reusable computing trope.",
    source_ids: ["dijkstra-2001"],
  },
  {
    text: "Knuth's later response treated the dispute as a question of disciplined control structures rather than a literal ban.",
    source_ids: ["knuth-1974"],
  },
];
analysis.context_sources = [
  {
    id: "dijkstra-2001",
    title: "What led to ‘Notes on Structured Programming’",
    authors: ["Edsger W. Dijkstra"],
    year: 2001,
    url: "https://www.cs.utexas.edu/~EWD/transcriptions/EWD13xx/EWD1308.html",
    supports: "Dijkstra's own later account of the title change, shallow title-only readings, and the phrase's afterlife.",
    verified_at: "2026-08-26T12:00:00Z",
  },
  {
    id: "knuth-1974",
    title: "Structured Programming with go to Statements",
    authors: ["Donald E. Knuth"],
    year: 1974,
    url: "https://homepages.cwi.nl/~storm/teaching/reader/Knuth74.pdf",
    supports: "A prominent later interpretation arguing for structured, deliberate uses of go to rather than a categorical prohibition.",
    verified_at: "2026-08-26T12:00:00Z",
  },
];
const extraction = JSON.parse(
  await readFile(path.join(root, ".lysilogy", "papers", paperId, "extraction.json"), "utf8"),
);
const metadata = extraction.metadata;
const paper = {
  id: paperId,
  metadata,
  relative_path: "Dijkstra - 1968 - GOTO Statements Considered Harmful.pdf",
  status: { state: "ready" },
  analyzed_at: analysis.generated_at,
  one_line_summary: analysis.thesis,
};
const unmappedPaper = {
  id: "1111111111111111",
  metadata: {
    title: "An unmapped control paper",
    authors: ["Test Author"],
    year: 2026,
    page_count: 2,
    subject: null,
  },
  relative_path: "Test Author - 2026 - An unmapped control paper.pdf",
  status: { state: "discovered" },
  analyzed_at: null,
  one_line_summary: null,
};
const smokeSentence = (page, index, text, y, xMin = 55, xMax = 555) => ({
  id: `p${String(page).padStart(4, "0")}-s${String(index).padStart(5, "0")}`,
  page,
  start_token: index - 1,
  end_token: index - 1,
  text,
  rects: [{ x_min: xMin, y_min: y, x_max: xMax, y_max: y + 13 }],
});
const layoutPages = Array.from({ length: 4 }, (_, pageIndex) => {
  const number = pageIndex + 1;
  const sentences = number === 1
    ? [
        smokeSentence(number, 1, "First grounded sentence on the page.", 110),
        smokeSentence(number, 2, "Second grounded sentence on the page.", 290),
        smokeSentence(number, 3, "Third grounded sentence marks three quarters progress.", 470),
        smokeSentence(number, 4, "Fourth grounded sentence begins the next section.", 650),
      ]
    : [
        smokeSentence(number, 1, `First grounded sentence on PDF page ${number}.`, 130),
        smokeSentence(number, 2, `Second grounded sentence on PDF page ${number}.`, 154),
      ];
  return {
    number,
    width: 612,
    height: 792,
    tokens: sentences.map((sentence, index) => ({
      index,
      text: sentence.text,
      line: index,
      rects: sentence.rects,
    })),
    sentences,
  };
});
const aiAnchor = {
  page: 1,
  start_token: 0,
  end_token: 0,
  sentence_ids: [layoutPages[0].sentences[0].id],
  rects: layoutPages[0].sentences[0].rects,
  exact_text: layoutPages[0].sentences[0].text,
};
const firstSection = analysis.sections[0];
if (firstSection !== undefined) {
  if (firstSection.key_quotes[0] !== undefined) {
    firstSection.key_quotes[0].anchor = aiAnchor;
    firstSection.key_quotes[0].validation = "exact";
  }
  firstSection.source_span = {
    start: aiAnchor,
    end: {
      page: 1,
      start_token: 2,
      end_token: 2,
      sentence_ids: [layoutPages[0].sentences[2].id],
      rects: layoutPages[0].sentences[2].rects,
      exact_text: layoutPages[0].sentences[2].text,
    },
  };
  firstSection.pages = { start: 1, end: 1 };
}
if (analysis.sections[1] !== undefined) {
  const nextAnchor = {
    page: 1,
    start_token: 3,
    end_token: 3,
    sentence_ids: [layoutPages[0].sentences[3].id],
    rects: layoutPages[0].sentences[3].rects,
    exact_text: layoutPages[0].sentences[3].text,
  };
  analysis.sections[1].pages = { start: 1, end: 1 };
  analysis.sections[1].source_span = { start: nextAnchor, end: nextAnchor };
}
for (const section of analysis.sections.slice(2)) {
  if (section.pages.start === 1) {
    section.pages.start = Math.min(4, Math.max(2, section.pages.end));
    section.pages.end = Math.max(section.pages.start, section.pages.end);
  }
}
let highlights = [{
  id: "ai-smoke-0",
  origin: { type: "ai", provider: "heuristic", section_id: analysis.sections[0].id, quote_index: 0 },
  kind: "evidence",
  anchor: aiAnchor,
  text: aiAnchor.exact_text,
  note: "Smoke-test evidence anchor",
  created_at: analysis.generated_at,
}];
const paperMap = {
  layout: { schema_version: 1, pages: layoutPages },
  highlights,
};
let analyzeRequests = 0;
let feedbackRequests = 0;
let queueJobs = [{
  paper_id: paperId,
  paper_title: metadata.title,
  provider: "codex",
  kind: "initial",
  status: { state: "completed" },
  progress: 100,
  tasks: [
    { id: "extract", label: "Extract text and exact PDF page coordinates", status: "completed", detail: null },
    { id: "read", label: "Read the complete target paper", status: "completed", detail: "4 pages read" },
  ],
  resumable: true,
  feedback: null,
  created_at: analysis.generated_at,
  updated_at: analysis.generated_at,
}];

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function mimeType(filename) {
  if (filename.endsWith(".js")) return "text/javascript; charset=utf-8";
  if (filename.endsWith(".css")) return "text/css; charset=utf-8";
  if (filename.endsWith(".mjs")) return "text/javascript; charset=utf-8";
  return "text/html; charset=utf-8";
}

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.route("http://lysilogy.test/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname === "/api/library") {
      await route.fulfill({ json: { name: "Articles", papers: [paper, unmappedPaper] } });
    } else if (url.pathname === "/api/queue") {
      await route.fulfill({ json: { jobs: queueJobs } });
    } else if (url.pathname === `/api/papers/${paperId}`) {
      await route.fulfill({ json: { paper, analysis } });
    } else if (url.pathname === `/api/papers/${paperId}/map`) {
      await route.fulfill({ json: paperMap });
    } else if (url.pathname === `/api/papers/${paperId}/highlights` && request.method() === "POST") {
      const payload = request.postDataJSON();
      const start = layoutPages.flatMap((entry) => entry.sentences)
        .find((sentence) => sentence.id === payload.start_sentence_id);
      const end = layoutPages.flatMap((entry) => entry.sentences)
        .find((sentence) => sentence.id === (payload.end_sentence_id ?? payload.start_sentence_id));
      const highlight = {
        id: "user-smoke-0",
        origin: { type: "user" },
        kind: payload.kind,
        anchor: {
          page: start.page,
          start_token: Math.min(start.start_token, end.start_token),
          end_token: Math.max(start.end_token, end.end_token),
          sentence_ids: [start.id, end.id],
          rects: [...start.rects, ...end.rects],
          exact_text: `${start.text} ${end.text}`,
        },
        text: `${start.text} ${end.text}`,
        note: payload.note,
        created_at: new Date().toISOString(),
      };
      highlights = [...highlights, highlight];
      paperMap.highlights = highlights;
      await route.fulfill({ status: 201, json: highlight });
    } else if (url.pathname.startsWith(`/api/papers/${paperId}/highlights/`) && request.method() === "DELETE") {
      const id = decodeURIComponent(url.pathname.split("/").at(-1));
      highlights = highlights.filter((highlight) => highlight.id !== id);
      paperMap.highlights = highlights;
      await route.fulfill({ status: 204, body: "" });
    } else if (url.pathname === `/api/papers/${paperId}/clarify`) {
      await route.fulfill({
        json: {
          selection: "selected passage",
          answer: "The passage links readable program structure to the ability to describe execution with stable coordinates.",
          concepts: analysis.glossary.slice(0, 1),
          connections: [analysis.thesis],
          limitation: "This smoke response only verifies the interaction surface.",
          provider: "heuristic",
        },
      });
    } else if (url.pathname === `/api/papers/${paperId}/analyze` && request.method() === "POST") {
      analyzeRequests += 1;
      await route.fulfill({ status: 202, json: paper });
    } else if (url.pathname === `/api/papers/${paperId}/feedback` && request.method() === "POST") {
      feedbackRequests += 1;
      const payload = request.postDataJSON();
      const job = {
        ...queueJobs[0],
        kind: "revision",
        provider: payload.provider,
        status: { state: "queued" },
        progress: 0,
        feedback: payload.feedback,
        resumable: true,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        tasks: [{ id: "feedback", label: "Interpret the reader's feedback", status: "active", detail: "resuming" }],
      };
      queueJobs = [job];
      await route.fulfill({ status: 202, json: job });
    } else if (url.pathname === `/api/papers/${paperId}/source`) {
      await route.fulfill({
        path: path.join(
          root,
          "local-articles",
          "Articles",
          "Dijkstra - 1968 - GOTO Statements Considered Harmful.pdf",
        ),
        contentType: "application/pdf",
      });
    } else if (url.pathname === `/api/papers/${paperId}/markdown`) {
      await route.fulfill({
        path: path.join(root, ".lysilogy", "papers", paperId, "source.md"),
        contentType: "text/markdown; charset=utf-8",
      });
    } else {
      const relative = url.pathname === "/" ? "index.html" : url.pathname.slice(1);
      const filename = path.join(root, "web", "dist", relative);
      await route.fulfill({ path: filename, contentType: mimeType(filename) });
    }
  });

  await page.goto("http://lysilogy.test/", { waitUntil: "networkidle" });
  await page.locator(".abstract-view").waitFor();
  const viewLabels = await page.locator(".view-switch > button").allTextContents();
  assert(
    viewLabels.join("|").replaceAll(/\s+/gu, " ").includes("01 Abstract|02 Overview|03 Glossary|04 Text"),
    `reading levels are out of order: ${viewLabels.join(", ")}`,
  );
  assert((await page.locator(".abstract-tldr").count()) === 1, "one-sentence TL;DR is missing");
  assert((await page.locator(".authored-abstract").count()) === 1, "authored abstract is missing");
  assert((await page.locator(".abstract-supplement").count()) === 1, "AI supplement is missing");
  assert((await page.locator(".context-source").count()) === 2, "exact context sources are missing");
  assert((await page.locator(".context-source-check").count()) === 2, "link-check timestamps are missing");
  assert(
    (await page.locator(".context-verification-scope").textContent())?.includes("not that the source semantically proves"),
    "link verification is not distinguished from semantic support",
  );
  await page.locator(".context-sources").scrollIntoViewIfNeeded();
  await page.screenshot({ path: screenshotVariant("abstract"), fullPage: true });
  await page.getByRole("button", { name: /02 Overview/u }).click();
  await page.locator(".source-page").first().waitFor();
  assert((await page.locator(".source-page").count()) === layoutPages.length, "the page map did not include every PDF page");
  assert(await page.locator(".source-map").evaluate((map) => {
    const chart = document.querySelector(".conceptual-atlas");
    return chart !== null && Boolean(map.compareDocumentPosition(chart) & Node.DOCUMENT_POSITION_FOLLOWING);
  }), "the page map does not lead the conceptual-weight chart");
  const columnCount = async () => Number(await page.locator(".source-pages").evaluate((grid) =>
    getComputedStyle(grid).getPropertyValue("--page-columns"),
  ));
  assert(await columnCount() === 4, "the page grid did not choose an integer default column count");
  await page.keyboard.press("Shift+=");
  assert(await columnCount() === 3, "plus did not zoom in by removing one page column");
  await page.keyboard.press("-");
  assert(await columnCount() === 4, "minus did not zoom out by adding one page column");

  await page.locator(".section-tile").first().waitFor();
  const tiles = await page.locator(".section-tile").count();
  assert(tiles >= 5, `expected at least 5 atlas tiles, found ${tiles}`);
  await page.waitForFunction(() => {
    const canvas = document.querySelector(".source-page-canvas canvas");
    return canvas instanceof HTMLCanvasElement && canvas.width > 0;
  });
  assert((await page.locator(".highlight-rect.origin-ai").count()) > 0, "AI prehighlight was not rendered");
  const firstPageRegions = page.locator(".source-page").first().locator(".section-regions > button");
  assert((await firstPageRegions.count()) === 2, "the shared page was not split into two section segments");
  const firstRegion = await firstPageRegions.first().evaluate((region) => ({
    left: Number.parseFloat(region.style.left),
    width: Number.parseFloat(region.style.width),
    top: getComputedStyle(region).top,
    height: getComputedStyle(region).height,
  }));
  assert(Math.abs(firstRegion.left) < 0.01, "the first section segment did not begin at the page edge");
  assert(Math.abs(firstRegion.width - 75) < 0.01, `three-quarter source progress rendered at ${firstRegion.width}% instead of 75%`);
  assert(firstRegion.top === "0px", "section progress was incorrectly projected down the PDF page");
  await page.keyboard.press("Shift+H");
  assert((await page.locator(".highlight-rect.origin-ai").count()) === 0, "AI prehighlight toggle did not hide evidence");
  await page.keyboard.press("Shift+H");
  await page.keyboard.press("Shift+I");
  assert(!(await page.locator(".source-page-canvas canvas").first().evaluate((canvas) => canvas.classList.contains("dark-ink"))), "capital I did not toggle atlas PDF inversion");
  await page.keyboard.press("Shift+I");
  await page.keyboard.press("v");
  await page.locator(".sentence-layer").first().waitFor();
  await page.waitForFunction(() => document.activeElement?.matches(".sentence-layer > button") === true);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Space");
  await page.locator(".highlight-rect.origin-user").first().waitFor();
  await page.keyboard.press("Shift+U");
  assert((await page.locator(".highlight-rect.origin-user").count()) === 0, "reader highlight toggle did not hide marks");
  await page.keyboard.press("Shift+U");
  await page.keyboard.press("Escape");
  await page.screenshot({ path: screenshot, fullPage: true });

  assert((await page.locator(".paper-list-item").count()) === 2, "library fixture did not load");
  await page.locator(".library-counts button").click();
  assert((await page.locator(".paper-list-item").count()) === 1, "mapped-only filter did not narrow the library");
  await page.locator(".library-counts button").click();
  await page.keyboard.press("f");
  assert((await page.locator(".paper-list-item").count()) === 1, "mapped-only keyboard filter failed");
  await page.keyboard.press("f");

  await page.keyboard.press("F1");
  await page.locator(".library-rail:not(.is-open)").waitFor();
  await page.keyboard.press("F1");
  await page.locator(".library-rail.is-open").waitFor();

  await page.keyboard.press("F10");
  await page.locator(".paper-switcher").waitFor();
  await page.locator(".switcher-search input").fill("goto");
  assert((await page.locator(".switcher-results > button").count()) === 1, "fuzzy switcher did not filter");
  await page.screenshot({ path: screenshotVariant("switcher"), fullPage: true });
  await page.keyboard.press("Enter");
  await page.locator(".paper-switcher").waitFor({ state: "detached" });

  await page.keyboard.press("m");
  await page.locator(".markdown-document").waitFor();
  assert((await page.locator(".text-view-header").count()) === 1, "Text format chooser is missing");
  assert((await page.locator(".markdown-page-marker").count()) === 4, "Markdown page provenance is incomplete");
  await page.screenshot({ path: screenshotVariant("markdown"), fullPage: true });
  await page.keyboard.press("m");
  await page.locator(".section-atlas").waitFor();

  await page.keyboard.press("Enter");
  await page.locator(".digest-panel").waitFor();
  await page.keyboard.press("v");
  await page.keyboard.press("j");
  await page.keyboard.press("c");
  await page.locator(".clarify-composer").waitFor();
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  await page.keyboard.press("g");
  await delay(500);
  await page.locator(".glossary-view").waitFor();
  assert((await page.locator(".glossary-view .gloss-entry").count()) > 0, "full glossary view is empty");
  await page.screenshot({ path: screenshotVariant("glossary"), fullPage: true });
  await page.keyboard.press("Escape");
  await page.locator(".section-atlas").waitFor();

  // Tab belongs to the reader: it steps through the phases of the current
  // paper and never falls through to the browser's focus traversal.
  const activePhase = async () =>
    (await page.locator('.view-switch > button[aria-current="page"]').textContent())
      ?.replaceAll(/\s+/gu, " ").trim();
  const rememberFocus = async () => page.evaluate(() => {
    window.__focusProbe = document.activeElement;
  });
  const focusHeldStill = async () => page.evaluate(() => document.activeElement === window.__focusProbe);

  await page.getByRole("button", { name: /01 Abstract/u }).click();
  await page.locator(".abstract-view").waitFor();
  await rememberFocus();
  await page.keyboard.press("Tab");
  await page.locator(".section-atlas").waitFor();
  assert(await activePhase() === "Overview", "Tab did not advance the abstract to the overview phase");
  assert(await focusHeldStill(), "Tab moved browser focus instead of only changing the reading phase");
  await page.keyboard.press("Tab");
  await page.locator(".glossary-view").waitFor();
  assert(await activePhase() === "Glossary", "Tab did not advance the overview to the glossary phase");
  await page.keyboard.press("Tab");
  await page.locator(".text-view").waitFor();
  assert(await activePhase() === "Text", "Tab did not advance the glossary to the text phase");
  await page.keyboard.press("Tab");
  await page.locator(".abstract-view").waitFor();
  assert(await activePhase() === "Abstract", "Tab did not wrap from the text phase back to the abstract");
  await page.keyboard.press("Shift+Tab");
  await page.locator(".text-view").waitFor();
  assert(await activePhase() === "Text", "Shift+Tab did not step back to the previous phase");

  // Panels and text fields are the states where a stray Tab used to escape.
  await page.keyboard.press("?");
  await page.locator(".help-card").waitFor();
  await page.keyboard.press("Tab");
  await page.locator(".help-card").waitFor({ state: "detached" });
  assert(await activePhase() === "Abstract", "Tab did not change phase while the help overlay was open");
  await page.keyboard.press("/");
  await page.waitForFunction(() => document.activeElement?.matches(".library-rail .search-box input") === true);
  await page.keyboard.press("Tab");
  await page.locator(".section-atlas").waitFor();
  assert(await activePhase() === "Overview", "Tab did not change phase from inside the library filter");
  assert(
    await page.evaluate(() => document.activeElement?.matches(".library-rail .search-box input") === true),
    "Tab tabbed out of the library filter instead of changing the reading phase",
  );
  await page.keyboard.press("Escape");

  // The switcher defines its own Tab, and still keeps it from the browser.
  await page.keyboard.press("F10");
  await page.locator(".paper-switcher").waitFor();
  await rememberFocus();
  await page.keyboard.press("Tab");
  await delay(40);
  assert((await page.locator(".paper-switcher").count()) === 1, "Tab closed the switcher instead of moving its selection");
  assert(await activePhase() === "Overview", "Tab changed the reading phase while the switcher owned the key");
  assert(await focusHeldStill(), "Tab moved browser focus out of the switcher");
  assert(
    await page.locator(".switcher-results > button").nth(1).evaluate((item) => item.classList.contains("is-active")),
    "Tab did not move the switcher selection to the next paper",
  );
  await page.keyboard.press("Escape");
  await page.locator(".paper-switcher").waitFor({ state: "detached" });

  await page.keyboard.press(":");
  await page.locator(".command-menu").waitFor();
  await page.locator(".command-input input").fill("gl");
  const expectedCompletion = (await page.locator(".command-results > button.is-active strong").textContent())
    ?.replace(":", "");
  await rememberFocus();
  await page.keyboard.press("Tab");
  await delay(60);
  assert((await page.locator(".command-menu").count()) === 1, "Tab closed the command menu instead of completing");
  assert(
    await page.locator(".command-input input").inputValue() === expectedCompletion,
    "Tab did not complete the typed command",
  );
  assert(await activePhase() === "Overview", "Tab changed the reading phase while the command menu owned the key");
  assert(await focusHeldStill(), "Tab moved browser focus out of the command menu");
  await page.keyboard.press("Escape");
  await page.locator(".command-menu").waitFor({ state: "detached" });
  await page.locator(".section-atlas").waitFor();

  await page.keyboard.press("p");
  await page.locator(".pdf-canvas").waitFor();
  await page.waitForFunction(() => {
    const canvas = document.querySelector(".pdf-canvas");
    return canvas instanceof HTMLCanvasElement && canvas.width > 0;
  });
  await page.keyboard.press("i");
  assert(await page.locator(".pdf-canvas").evaluate((canvas) => canvas.classList.contains("dark-ink")), "lowercase i should not invert the PDF");
  await page.keyboard.press("Shift+I");
  assert(!(await page.locator(".pdf-canvas").evaluate((canvas) => canvas.classList.contains("dark-ink"))), "capital I did not reveal true PDF colours");
  await page.keyboard.press("2");
  await page.waitForFunction(() => document.querySelectorAll(".pdf-canvas").length === 2);
  assert((await page.locator(".pdf-canvas").count()) === 2, "two-page PDF view did not render a spread");
  await page.screenshot({ path: screenshotVariant("pdf-spread"), fullPage: true });
  await page.keyboard.press("PageDown");
  await page.locator('.pdf-canvas[aria-label^="Page 3 "]').waitFor();
  await page.keyboard.press("PageUp");
  await page.locator('.pdf-canvas[aria-label^="Page 1 "]').waitFor();
  await page.keyboard.press("Control+d");
  await page.locator('.pdf-canvas[aria-label^="Page 3 "]').waitFor();
  await page.keyboard.press("Control+u");
  await page.locator('.pdf-canvas[aria-label^="Page 1 "]').waitFor();
  await page.keyboard.press("ArrowRight");
  await page.locator('.pdf-canvas[aria-label^="Page 3 "]').waitFor();
  await page.keyboard.press("ArrowLeft");
  await page.locator('.pdf-canvas[aria-label^="Page 1 "]').waitFor();

  await page.keyboard.press("q");
  await page.locator(".queue-panel").waitFor();
  assert((await page.locator(".queue-progress").count()) === 1, "queue did not render tasklist progress");
  await delay(220);
  await page.screenshot({ path: screenshotVariant("queue"), fullPage: true });
  await page.locator(".feedback-form textarea").fill("Explain the central objection more plainly.");
  await page.locator(".feedback-form button[type=submit]").click();
  await page.waitForFunction(() => document.querySelector(".feedback-form textarea")?.value === "");
  assert(feedbackRequests === 1, "feedback retry was not sent to the backend");
  await page.keyboard.press("q");
  await page.locator(".queue-panel").waitFor({ state: "detached" });
  queueJobs = queueJobs.map((job) => ({
    ...job,
    status: { state: "completed" },
    progress: 100,
    tasks: job.tasks.map((task) => ({ ...task, status: "completed" })),
  }));
  await delay(950);

  await page.keyboard.press("a");
  await delay(40);
  assert(analyzeRequests === 0, "lowercase a still triggered analysis");
  await page.keyboard.press(":");
  await page.locator(".command-menu").waitFor();
  await page.locator(".command-input input").fill("analyze heuristic");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => !document.querySelector(".command-menu"));
  assert(analyzeRequests === 1, ":analyze did not start analysis");

  await page.setViewportSize({ width: 390, height: 844 });
  await delay(100);
  await page.getByRole("button", { name: /01 Abstract/u }).click();
  await page.locator(".abstract-view").waitFor();
  await page.screenshot({ path: screenshotVariant("abstract-mobile"), fullPage: true });
  await page.keyboard.press("b");
  await page.locator(".library-rail.is-open").waitFor();
  await page.keyboard.press("j");
  assert(
    await page.locator(".paper-list-item").nth(1).evaluate((item) => item === document.activeElement),
    "mobile library did not enter keyboard navigation mode",
  );
  await delay(30);
  await page.keyboard.press("Enter");
  await page.locator(".library-rail:not(.is-open)").waitFor();

  await page.keyboard.press("?");
  await page.locator(".help-card").waitFor();
  await page.keyboard.press("Escape");
  await page.locator(".help-card").waitFor({ state: "detached" });

  console.log(`visual smoke passed: cited context, ${tiles} tiles, aligned source pages, AI/user highlights, arrows and paging, two-page PDF, reconstructed text, mapped filter, Tab phase cycling, F1/F10, :analyze, live queue, feedback retry, selection, glossary, and mobile keys; screenshot ${screenshot}`);
} finally {
  await browser.close();
}
