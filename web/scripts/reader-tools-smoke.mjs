import assert from "node:assert/strict";
import { readFile, mkdir } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";

const output = process.argv[2] ?? "/tmp/lysilogy-reader-tools-smoke";
await mkdir(output, { recursive: true });
const root = path.resolve("..");
const paperId = "aaaaaaaaaaaaaaaa";
const citedId = "bbbbbbbbbbbbbbbb";
const overview = (id, title) => ({
  id, metadata: { title, authors: ["Ada Researcher"], year: 2026, page_count: 10, subject: null },
  relative_path: title + ".pdf", status: { state: "discovered" }, analyzed_at: null, one_line_summary: null,
});
const papers = [overview(paperId, "Evidence and mechanisms"), overview(citedId, "A related result")];
const state = { supercuts: [], references: [], jobs: [] };
const calls = [];
let pendingPolls = 0;
let nextFailure = false;
const now = "2026-09-10T12:00:00Z";
const finish = () => {
  const job = state.jobs.at(-1);
  if (!job || job.status !== "running") return;
  if (nextFailure) {
    job.status = "failed"; job.error = "The source passage could not be verified."; nextFailure = false; return;
  }
  job.status = "completed";
  if (job.action.kind === "supercut") {
    const six = job.action.format === "six_pages";
    const count = six ? 6 : 10;
    state.supercuts.push({
      id: job.id, format: job.action.format, agent: "Lysilogos", provider: "codex", created_at: now, source_words: count * 100, total_words: count * 110,
      paragraphs: Array.from({ length: count }, (_, i) => ({
        cut_page: six ? i + 1 : 1,
        segments: [
          { kind: "source", text: Array.from({ length: six ? 34 : 1 }, () => `Passage ${i + 1} describes the evidence and the limits of this mechanism.`).join(" "), source_page: i + 1 },
          { kind: "connector", text: "This qualification matters for the next inference.", source_page: null },
        ],
      })),
    });
  } else {
    const reference = state.references.find((item) => item.id === job.action.reference_id);
    if (job.action.kind === "find_reference") {
      reference.candidate = { title: "A related result", landing_url: "https://example.com/paper", pdf_url: "https://example.com/paper.pdf", explanation: "Matched title and author." };
      reference.linked_paper_id = citedId;
    } else {
      reference.question = job.action.question;
      reference.connection = {
        verdict: "qualifies", connector: "The cited result restricts the mechanism to a narrower setting.",
        limitation: "The texts establish a relationship, not independent replication.",
        evidence: [
          { paper_id: paperId, source_page: 2, quote: "The mechanism depends on this assumption." },
          { paper_id: citedId, source_page: 4, quote: "The assumption holds only in this setting." },
        ],
      };
    }
  }
};
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 }, acceptDownloads: true });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.route("http://lysilogy.test/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const method = request.method();
    const suffix = url.pathname.replace(`/api/papers/${paperId}`, "");
    if (url.pathname === "/api/library") return route.fulfill({ json: { name: "Test library", papers } });
    if (url.pathname === "/api/queue") return route.fulfill({ json: { jobs: [] } });
    if (url.pathname === `/api/papers/${paperId}` || url.pathname === `/api/papers/${citedId}`) {
      return route.fulfill({ json: { paper: papers.find((item) => url.pathname.endsWith(item.id)), analysis: null } });
    }
    if (suffix === "/reader-tools") {
      if (pendingPolls > 0 && --pendingPolls === 0) finish();
      return route.fulfill({ json: state });
    }
    if (suffix === "/reader-tools/jobs") {
      const payload = request.postDataJSON();
      calls.push(payload);
      const job = { id: `job-${calls.length}`, ...payload, status: "running", created_at: now, error: null };
      state.jobs.push(job); pendingPolls = 2;
      return route.fulfill({ status: 202, json: job });
    }
    if (suffix === "/references" && method === "POST") {
      const saved = { id: "reference-1", ...request.postDataJSON(), created_at: now, linked_paper_id: null, candidate: null, connection: null, question: null };
      state.references.push(saved);
      return route.fulfill({ status: 201, json: saved });
    }
    if (suffix.startsWith("/references/") && method === "PATCH") {
      state.references[0].linked_paper_id = request.postDataJSON().linked_paper_id;
      return route.fulfill({ json: state });
    }
    if (suffix.startsWith("/references/") && method === "DELETE") {
      state.references.length = 0;
      return route.fulfill({ status: 204 });
    }
    if (url.pathname.startsWith("/api/")) return route.fulfill({ status: 404, json: { message: "Unexpected fixture request: " + url.pathname } });
    const file = path.join(root, "web", "dist", url.pathname === "/" ? "index.html" : url.pathname.slice(1));
    const mime = file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : file.endsWith(".svg") ? "image/svg+xml" : "text/html";
    return route.fulfill({ body: await readFile(file), contentType: mime });
  });
  await page.goto(`http://lysilogy.test/#paper=${paperId}`);
  await page.locator(".topbar").waitFor();
  await page.keyboard.press(":");
  await page.getByRole("textbox", { name: "Command", exact: true }).fill("supercut");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Lysilogos reader tools" });
  await dialog.waitFor();
  await page.keyboard.press("Tab");
  assert.equal(await page.evaluate(() => document.activeElement?.textContent), "Supercut", "Tab should move inside the dialog");
  await page.getByLabel("Cut length").selectOption("ten_paragraphs");
  await page.getByRole("button", { name: "Create Supercut", exact: true }).click();
  await page.locator(".supercut-result").waitFor();
  assert.equal(await page.locator(".cut-sheet > p").count(), 10);
  assert.equal(await page.locator(".cut-connector").count(), 10);
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download Markdown" }).click();
  const download = await downloadPromise;
  await download.saveAs(path.join(output, "supercut.md"));
  assert.match(await readFile(path.join(output, "supercut.md"), "utf8"), /Lysilogos connector:/u);

  await page.getByLabel("Cut length").selectOption("six_pages");
  await page.getByRole("button", { name: "Create Supercut", exact: true }).click();
  await page.locator(".six-page-cut .cut-sheet").nth(5).waitFor();
  assert.equal(await page.locator(".six-page-cut .cut-sheet").count(), 6);
  assert.equal(await page.locator(".cut-sheet").evaluateAll((sheets) => sheets.some((sheet) => sheet.scrollHeight > sheet.clientHeight + 1)), false, "six-page sheets must fit");
  await page.screenshot({ path: path.join(output, "supercut.png") });
  await page.pdf({ path: path.join(output, "six-page-supercut.pdf"), preferCSSPageSize: true, printBackground: true });

  await page.getByRole("button", { name: /^References/u }).click();
  await page.getByLabel("Citation", { exact: true }).fill("Researcher — A related result");
  await page.getByLabel("PDF page (optional)").fill("3");
  await page.getByLabel("Why save it?").fill("Check the scope assumption");
  await page.getByRole("button", { name: "Save reference", exact: true }).click();
  await page.locator(".saved-reference").waitFor();
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "detached" });
  await page.locator(".topbar").waitFor();
  await page.keyboard.press(":");
  await page.getByRole("textbox", { name: "Command", exact: true }).fill("references");
  await page.keyboard.press("Enter");
  await page.locator(".saved-reference").waitFor();
  assert.match(await page.locator(".saved-reference").innerText(), /Check the scope assumption/u);
  await page.getByLabel("Linked paper").selectOption(citedId);
  await page.getByLabel("Claim or question").fill("Does the cited paper support this mechanism?");
  await page.getByRole("button", { name: "Check claim", exact: true }).click();
  await page.locator(".reference-connection").waitFor();
  assert.equal(await page.locator(".reference-connection blockquote").count(), 2);
  assert.equal(calls.at(-1).action.kind, "connect_reference");
  await page.getByRole("button", { name: "Find & fetch paper", exact: true }).click();
  await page.locator(".reference-candidate").waitFor();
  assert.equal(calls.at(-1).action.kind, "find_reference");
  nextFailure = true;
  await page.getByRole("button", { name: "Check claim", exact: true }).click();
  await page.getByText("The source passage could not be verified.", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Retry task", exact: true }).click();
  await page.getByRole("status").filter({ hasText: "Lysilogos is working" }).waitFor({ state: "detached" });

  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: path.join(output, "references-mobile.png"), fullPage: true });
  assert.equal(await dialog.evaluate((element) => element.scrollWidth <= element.clientWidth + 1), true, "reference panel must fit mobile");
  await page.reload();
  await page.locator(".topbar").waitFor();
  await page.keyboard.press(":");
  await page.getByRole("textbox", { name: "Command", exact: true }).fill("supercut");
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: /^References/u }).click();
  await page.locator(".reference-connection").waitFor();
  await page.getByRole("button", { name: /^Remove reference:/u }).click();
  await page.locator(".saved-reference").waitFor({ state: "detached" });
  assert.deepEqual(pageErrors, []);
  console.log(`reader tools smoke passed: two cut formats, print layout, export, citation persistence, linking, search, comparison, retry, keyboard, and mobile; artifacts ${output}`);
} finally {
  await browser.close();
}
