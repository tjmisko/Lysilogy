import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";

import { chromium } from "playwright";

const paperId = "a72b0e4a80ecd9db";
const screenshot = process.argv[2] ?? "/tmp/lysilogos-atlas.png";
const screenshotVariant = (name) => {
  const parsed = path.parse(screenshot);
  return path.join(parsed.dir, `${parsed.name}-${name}${parsed.ext}`);
};
const root = path.resolve("..");
const analysis = JSON.parse(
  await readFile(path.join(root, ".lysilogos", "papers", paperId, "analysis.json"), "utf8"),
);
analysis.author_abstract = "For a number of years I have been familiar with the observation that the quality of programmers is a decreasing function of the density of go to statements in the programs they produce.";
analysis.outsider_brief = "The letter helped turn structured programming from a design preference into a durable standard for reasoning about control flow. Its title later became shorthand for categorical prohibition, although Dijkstra's argument is more specifically about preserving intelligible coordinates for program execution.";
const extraction = JSON.parse(
  await readFile(path.join(root, ".lysilogos", "papers", paperId, "extraction.json"), "utf8"),
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
        smokeSentence(number, 1, "First grounded sentence at the end of the left column.", 650, 55, 280),
        smokeSentence(number, 2, "Second grounded sentence at the top of the right column.", 110, 330, 555),
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
if (analysis.sections[0]?.key_quotes[0] !== undefined) {
  analysis.sections[0].key_quotes[0].anchor = aiAnchor;
  analysis.sections[0].key_quotes[0].validation = "exact";
  analysis.sections[0].source_span = {
    start: aiAnchor,
    end: {
      page: 1,
      start_token: 1,
      end_token: 1,
      sentence_ids: [layoutPages[0].sentences[1].id],
      rects: layoutPages[0].sentences[1].rects,
      exact_text: layoutPages[0].sentences[1].text,
    },
  };
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
  await page.route("http://lysilogos.test/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname === "/api/library") {
      await route.fulfill({ json: { name: "Articles", papers: [paper, unmappedPaper] } });
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
        path: path.join(root, ".lysilogos", "papers", paperId, "source.md"),
        contentType: "text/markdown; charset=utf-8",
      });
    } else {
      const relative = url.pathname === "/" ? "index.html" : url.pathname.slice(1);
      const filename = path.join(root, "web", "dist", relative);
      await route.fulfill({ path: filename, contentType: mimeType(filename) });
    }
  });

  await page.goto("http://lysilogos.test/", { waitUntil: "networkidle" });
  await page.locator(".abstract-view").waitFor();
  const viewLabels = await page.locator(".view-switch > button").allTextContents();
  assert(
    viewLabels.join("|").replaceAll(/\s+/gu, " ").includes("01 Abstract|02 Overview|03 Glossary|04 Text"),
    `reading levels are out of order: ${viewLabels.join(", ")}`,
  );
  assert((await page.locator(".abstract-tldr").count()) === 1, "one-sentence TL;DR is missing");
  assert((await page.locator(".authored-abstract").count()) === 1, "authored abstract is missing");
  assert((await page.locator(".abstract-supplement").count()) === 1, "AI supplement is missing");
  await page.screenshot({ path: screenshotVariant("abstract"), fullPage: true });
  await page.getByRole("button", { name: /02 Overview/u }).click();
  await page.locator(".section-tile").first().waitFor();
  const tiles = await page.locator(".section-tile").count();
  assert(tiles >= 5, `expected at least 5 atlas tiles, found ${tiles}`);
  await page.locator(".source-page").first().waitFor();
  await page.locator(".source-page").first().scrollIntoViewIfNeeded();
  await page.waitForFunction(() => {
    const canvas = document.querySelector(".source-page-canvas canvas");
    return canvas instanceof HTMLCanvasElement && canvas.width > 0;
  });
  assert((await page.locator(".highlight-rect.origin-ai").count()) > 0, "AI prehighlight was not rendered");
  assert((await page.locator(".section-regions > .is-verified").count()) >= 2, "multi-column source span was not split into aligned blocks");
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

  console.log(`visual smoke passed: ${tiles} tiles, aligned source pages, AI/user highlights, arrows, Markdown, mapped filter, F1/F10, PDF, selection, Gloss, and mobile keys; screenshot ${screenshot}`);
} finally {
  await browser.close();
}
