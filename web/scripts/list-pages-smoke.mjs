import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

// Reading-list pages against the built bundle (npm run build first). Only the HTTP transport is
// intercepted: the lists live in an in-memory fake that mimics the server's answers.
const root = path.resolve('..');
const now = '2026-10-24T12:00:00Z';
const listId = 'p00000000000000000000000000000001';
const paperId = '1234567890abcdef';
const paper = { id: paperId, metadata: { title: 'Held paper', authors: ['Test Author'], year: 2026, page_count: 2 }, relative_path: 'held.pdf', status: { state: 'discovered' }, analyzed_at: null, one_line_summary: null };

const analysis = {
  schema_version: 5, provider: 'heuristic', generated_at: now, thesis: 'A synthetic thesis.', author_abstract: 'A synthetic abstract.',
  outsider_brief: '', context_notes: [], context_sources: [], prerequisites: [], sections: [], claims: [], glossary: [], caveats: [], reading_path: [],
};

let counter = 0;
const workNode = (id, title, year, over = {}) => {
  counter += 1;
  return {
    id, list_id: listId, kind: 'work', work_id: `w-${id}`, label: null, summary: null, claim: null, claim_state: null,
    candidates: [], position: counter * 10, note: null, rationale: null, context_links: [], source: 'user', review: 'accepted',
    via_work_id: null, path: null, generation_id: null, added_at: now, updated_at: now,
    work: { id: `w-${id}`, kind: 'article', visibility: 'private', display_title: title, display_year: year, display_authors: ['Ann Author'] },
    ordinal: counter, depth: 0, path_steps: [], ...over,
  };
};
const state = {
  nodes: [workNode('a', 'Attention Is All You Need', 2017), workNode('b', 'Deep RL from human preferences', 2017), workNode('c', 'InstructGPT', 2022)],
  edges: [],
  statuses: {},
  papers: { 'w-a': [{ paper_id: paperId, title: 'Held paper' }] },
  gone: null,
};

function detail() {
  const nodes = [...state.nodes].sort((x, y) => x.position - y.position).map((node, index) => ({ ...node, ordinal: index + 1 }));
  const counts = { unread: 0, reading: 0, read: 0 };
  for (const node of nodes) if (node.kind === 'work') counts[state.statuses[node.work_id] ?? 'unread'] += 1;
  return {
    list: {
      id: listId, workspace_id: 'ws', title: 'RLHF basics', description: null, origin: 'manual', seed_work_id: null, goal: null,
      archived_at: null, created_at: now, updated_at: now, seed_title: null, item_count: nodes.filter((n) => n.kind !== 'topic').length,
      topic_count: nodes.filter((n) => n.kind === 'topic').length, claim_count: 0, proposed_count: 0, status_counts: counts,
    },
    nodes, edges: state.edges, citations: [],
    reading_states: Object.fromEntries(Object.entries(state.statuses).map(([id, status]) => [id, { status, changed_at: now }])),
    local_papers: state.papers, acquisitions: {}, expansions: {},
  };
}
const summary = () => detail().list;
const titles = (page) => page.$$eval('.list-sequence .list-row .list-row-title', (nodes) => nodes.map((node) => node.textContent));

const browser = await chromium.launch({ headless: true, ...(process.env.LYSILOGY_CHROME === undefined ? {} : { executablePath: process.env.LYSILOGY_CHROME }) });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 850 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  const requests = [];
  await page.route('**/*', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const method = request.method();
    if (url.pathname.startsWith('/api/')) requests.push(`${method} ${url.pathname}${url.search}`);
    const json = (body, status = 200) => route.fulfill({ status, json: body });
    const body = () => (request.postData() === null ? {} : JSON.parse(request.postData()));
    const segments = url.pathname.split('/');
    if (url.pathname === '/api/library') return json({ name: 'Synthetic library', papers: [paper] });
    if (url.pathname === '/api/queue') return json({ jobs: [] });
    if (url.pathname === '/api/tags') return json({ tags: [] });
    if (url.pathname === `/api/papers/${paperId}`) return json({ paper, analysis });
    if (url.pathname.endsWith('/map')) return json({ layout: { schema_version: 1, pages: [] }, highlights: [] });
    if (url.pathname === `/api/papers/${paperId}/lists`) {
      const here = state.nodes.find((node) => node.id === 'a');
      return json({ paper_id: paperId, work_id: 'w-a', memberships: here === undefined ? [] : [{ list_id: listId, title: 'RLHF basics', archived: false, node_id: 'a', topics: ['Rewards'], note: null }] });
    }
    if (url.pathname === `/api/papers/${paperId}/work`) return json({ paper_id: paperId, work: { id: 'w-a' }, link: null });
    if (url.pathname === '/api/lists' && method === 'GET') return json({ lists: [summary()] });
    if (url.pathname === '/api/lists' && method === 'POST') return json(detail(), 201);
    if (url.pathname === `/api/lists/${listId}` && method === 'GET') return json(detail());
    if (url.pathname === `/api/lists/${listId}/order`) {
      body().node_ids.forEach((id, index) => { state.nodes.find((node) => node.id === id).position = (index + 1) * 10; });
      return json(detail());
    }
    if (url.pathname === `/api/lists/${listId}/nodes` && method === 'POST') {
      const add = body();
      counter += 1;
      const restored = state.gone !== null && state.gone.work_id === add.work_id ? state.gone : null;
      const node = restored === null
        ? workNode(`n${counter}`, add.work_id ?? add.claim?.title ?? 'New', 2020, { work_id: add.work_id ?? null, position: counter * 10 })
        : { ...restored, id: `n${counter}`, position: counter * 10 };
      state.nodes.push(node);
      return json(node, 201);
    }
    if (segments[3] === listId.slice(0) && segments[4] === 'nodes' && segments[5] !== undefined) {
      const index = state.nodes.findIndex((node) => node.id === segments[5]);
      if (method === 'DELETE') {
        const [node] = state.nodes.splice(index, 1);
        state.gone = node;
        return json({ node, edges: [], was_seed: false });
      }
      if (method === 'PATCH') {
        state.nodes[index] = { ...state.nodes[index], note: body().note === '' ? null : body().note };
        return json(state.nodes[index]);
      }
    }
    if (url.pathname === `/api/lists/${listId}/edges` && method === 'POST') {
      const edge = { list_id: listId, ...body(), source: 'user', review: 'accepted', created_at: now };
      state.edges.push(edge);
      return json(edge, 201);
    }
    if (segments[2] === 'works' && segments[4] === 'reading-state') {
      state.statuses[segments[3]] = body().status;
      return json({ work_id: segments[3], status: body().status, changed_at: now });
    }
    if (url.pathname === '/api/lists/generate') return json({ generation_id: 'g00000000000000000000000000000001' }, 202);
    if (segments[2] === 'generations') return json({ state: 'succeeded', report: {}, spend_usd: 0.0123, list_id: listId });
    if (url.pathname === '/api/works/search') {
      return json({ results: [{ work_id: 'w-new', paper_id: null, title: 'Learning to summarize', year: 2020, authors: ['N Stiennon'], source: 'graph', in_graph: true, openalex_id: null, doi: null }] });
    }
    if (url.pathname.startsWith('/api/')) return route.fulfill({ status: 404, json: { message: `Unexpected fixture request ${method} ${url.pathname}` } });
    const file = path.join(root, 'web/dist', url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
    return route.fulfill({ body: await readFile(file), contentType: /\.m?js$/.test(file) ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html' });
  });

  // Routes: the index, then a list.
  await page.goto('http://lysilogy.test/#lists');
  await page.waitForSelector('.lists-row');
  assert.equal(await page.locator('.lists-row-title').innerText(), 'RLHF basics');
  await page.keyboard.press('Enter');
  await page.waitForSelector('.list-sequence .list-row');
  assert.match(page.url(), /#list=/);
  assert.deepEqual(await titles(page), ['Attention Is All You Need', 'Deep RL from human preferences', 'InstructGPT']);
  assert.equal(await page.locator('.list-row.is-selected').count(), 1);

  // Movement and status.
  await page.keyboard.press('j');
  assert.match(await page.locator('.list-row.is-selected .list-row-title').innerText(), /Deep RL/u);
  await page.keyboard.press('r');
  await page.waitForSelector('.list-row[data-status="reading"]');
  await page.keyboard.press('r');
  await page.waitForSelector('.list-row[data-status="read"]');
  await page.keyboard.press('r');
  await page.waitForSelector('.list-row.is-selected[data-status="unread"]');

  // Reorder: dd then p, J, K.
  await page.keyboard.press('g'); await page.keyboard.press('g');
  await page.keyboard.press('d'); await page.keyboard.press('d');
  await page.waitForSelector('.list-row.is-cut');
  await page.keyboard.press('G');
  await page.keyboard.press('p');
  await page.waitForFunction(() => document.querySelector('.list-sequence .list-row:last-child .list-row-title')?.textContent === 'Attention Is All You Need');
  assert.deepEqual(await titles(page), ['Deep RL from human preferences', 'InstructGPT', 'Attention Is All You Need']);
  await page.keyboard.press('K');
  await page.waitForFunction(() => document.querySelector('.list-sequence .list-row .list-row-title')?.textContent === 'InstructGPT');
  assert.deepEqual(await titles(page), ['InstructGPT', 'Deep RL from human preferences', 'Attention Is All You Need']);

  // Remove and undo.
  await page.keyboard.press('x');
  await page.waitForFunction(() => document.querySelectorAll('.list-sequence .list-row').length === 2);
  await page.keyboard.press('u');
  await page.waitForFunction(() => document.querySelectorAll('.list-sequence .list-row').length === 3);
  assert.deepEqual(await titles(page), ['InstructGPT', 'Deep RL from human preferences', 'Attention Is All You Need']);

  // Note, edge mode, add by name.
  await page.keyboard.press('e');
  await page.locator('.list-row-input').fill('start here');
  await page.keyboard.press('Enter');
  await page.waitForSelector('.list-row-note');
  await page.keyboard.press('E');
  await page.keyboard.press('j');
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => document.querySelector('.list-message')?.textContent?.includes('prerequisite'));
  await page.keyboard.press('a');
  await page.getByRole('textbox', { name: 'Add a paper by name' }).fill('summarize');
  await page.getByRole('option', { name: /Learning to summarize/u }).waitFor();
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => document.querySelectorAll('.list-sequence .list-row').length === 4);

  // Sort and filter are view-only.
  await page.keyboard.press('s');
  await page.keyboard.press('s');
  assert.match(await page.locator('.list-toolbar').innerText(), /year/u);
  await page.keyboard.press('/');
  await page.locator('.list-filter').fill('instruct');
  await page.keyboard.press('Enter');
  assert.deepEqual(await titles(page), ['InstructGPT']);
  await page.keyboard.press('Escape');
  assert.equal((await titles(page)).length, 4);

  // Views.
  await page.keyboard.press('4');
  await page.waitForSelector('[data-empty="graph"]');
  assert.match(page.url(), /view=4/u);
  await page.keyboard.press('v');
  await page.waitForSelector('.list-views button[aria-pressed="true"]');
  await page.keyboard.press('1');

  // Picker and membership over the list page.
  await page.keyboard.press('Y');
  await page.waitForSelector('[data-list-overlay]');
  await page.keyboard.press('Escape');
  await page.waitForSelector('[data-list-overlay]', { state: 'detached' });

  // Reader: chips and the membership popover.
  await page.goto(`http://lysilogy.test/#paper=${paperId}`);
  await page.waitForSelector('.paper-lists');
  assert.match(await page.locator('.paper-list-chip').first().innerText(), /RLHF basics · Rewards/u);
  await page.keyboard.press('M');
  await page.waitForSelector('[data-list-overlay]');
  await page.getByRole('option', { name: /RLHF basics/u }).waitFor();
  await page.keyboard.press('x');
  await page.waitForFunction(() => document.body.textContent.includes('Press u to undo'));
  assert.ok(requests.some((entry) => entry.startsWith('DELETE /api/lists/') && entry.endsWith('/nodes/a')), requests.join('\n'));
  await page.keyboard.press('u');
  await page.waitForFunction(() => document.body.textContent.includes('Put back'));
  await page.keyboard.press('Escape');
  await page.keyboard.press('Y');
  await page.getByRole('option', { name: /RLHF basics/u }).waitFor();
  await page.keyboard.press('Enter');
  await page.waitForSelector('[data-list-overlay]', { state: 'detached' });
  assert.ok(requests.filter((entry) => entry.startsWith('POST /api/lists/') && entry.endsWith('/nodes')).length >= 3, requests.join('\n'));
  // Colon commands keep the argument's case and start a generation.
  await page.keyboard.type(':');
  await page.getByRole('textbox', { name: 'Command' }).fill('generate learn RLHF reward modelling');
  await page.keyboard.press('Enter');
  await page.waitForSelector('.generation-strip');
  assert.match(await page.locator('.generation-strip').innerText(), /learn RLHF reward modelling/u);
  await page.keyboard.press('g'); await page.keyboard.press('l');
  await page.waitForSelector('.lists-row');
  assert.match(page.url(), /#lists/u);
  await page.keyboard.press('g'); await page.keyboard.press('r');
  await page.waitForSelector('[aria-label="Reading history"]');

  assert.deepEqual(errors, []);
  console.log('list pages smoke passed');
} finally {
  await browser.close();
}
