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
  await page.locator(".section-tile").first().waitFor();
  const tiles = await page.locator(".section-tile").count();
  assert(tiles >= 5, `expected at least 5 atlas tiles, found ${tiles}`);
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
  await page.locator(".gloss-panel").waitFor();
  await page.keyboard.press("Escape");

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

  console.log(`visual smoke passed: ${tiles} tiles, Markdown, mapped filter, F1/F10, PDF, selection, Gloss, and mobile keys; screenshot ${screenshot}`);
} finally {
  await browser.close();
}
