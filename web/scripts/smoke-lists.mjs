import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

// Every key of the reading-list surfaces, once, against the built bundle (npm run build first).
// Only the HTTP transport is intercepted; the lists and the history live in an in-memory fake.
// The live model call is out of scope here: generation is only started, never awaited.
// localhost is a secure context, which the history session id (crypto.randomUUID) needs.
const root = path.resolve('..');
const now = new Date();
const stamp = (daysAgo, minutes = 0) => new Date(now.getTime() - daysAgo * 86_400_000 - minutes * 60_000).toISOString();
const listId = 'p00000000000000000000000000000001';
const paperId = '1234567890abcdef';
const paper = { id: paperId, metadata: { title: 'Held paper', authors: ['Test Author'], year: 2026, page_count: 2 }, relative_path: 'held.pdf', status: { state: 'discovered' }, analyzed_at: null, one_line_summary: null };
const analysis = {
  schema_version: 5, provider: 'heuristic', generated_at: stamp(0), thesis: 'A synthetic thesis.', author_abstract: 'A synthetic abstract.',
  outsider_brief: '', context_notes: [], context_sources: [], prerequisites: [], sections: [], claims: [], glossary: [], caveats: [], reading_path: [],
};

let counter = 0;
const workRecord = (id, title, year) => ({ id: `w-${id}`, kind: 'article', visibility: 'private', title_state: 'asserted', year_state: 'asserted', created_at: stamp(0), updated_at: stamp(0), display_title: title, display_year: year, display_authors: ['Ann Author'] });
const node = (kind, id, over = {}) => {
  counter += 1;
  return {
    id, list_id: listId, kind, work_id: null, label: null, summary: null, claim: null, claim_state: null,
    candidates: [], position: counter * 10, note: null, rationale: null, context_links: [], source: 'user', review: 'accepted',
    via_work_id: null, path: null, generation_id: null, added_at: stamp(0), updated_at: stamp(0),
    work: null, ordinal: counter, depth: 0, path_steps: [], ...over,
  };
};
const contextLink = { work_id: 'w-a', relation: 'builds_on', note: 'same reward model' };
const workNode = (id, title, year, over = {}) => node('work', id, { work_id: `w-${id}`, work: workRecord(id, title, year), context_links: [contextLink], ...over });
const state = {
  nodes: [
    workNode('a', 'Attention Is All You Need', 2017),
    workNode('b', 'Deep RL from human preferences', 2017),
    workNode('c', 'InstructGPT', 2022),
    workNode('p', 'Proposed paper', 2023, { review: 'proposed', source: 'generated' }),
    node('claim', 'k', { claim: { title: 'a paper on reward hacking', authors: [], year: null, doi: null }, claim_state: 'ambiguous', candidates: [
      { provider_id: 'oa-1', title: 'Scaling laws for reward model overoptimization', year: 2023, authors: ['L Gao'] },
      { provider_id: 'oa-2', title: 'Reward hacking in practice', year: 2022, authors: ['A Pan'] },
    ] }),
  ],
  edges: [],
  statuses: {},
  papers: { 'w-a': [{ paper_id: paperId, title: 'Held paper' }] },
  gone: null,
  archived: false,
  title: 'RLHF basics',
};

function detail() {
  const nodes = [...state.nodes].sort((x, y) => x.position - y.position).map((entry, index) => ({ ...entry, ordinal: index + 1 }));
  const counts = { unread: 0, reading: 0, read: 0 };
  for (const entry of nodes) if (entry.kind === 'work') counts[state.statuses[entry.work_id] ?? 'unread'] += 1;
  return {
    list: {
      id: listId, workspace_id: 'ws', title: state.title, description: null, origin: 'manual', seed_work_id: null, goal: null,
      archived_at: state.archived ? stamp(0) : null, created_at: stamp(3), updated_at: stamp(0), seed_title: null,
      item_count: nodes.filter((n) => n.kind === 'work').length, topic_count: nodes.filter((n) => n.kind === 'topic').length,
      claim_count: 1, proposed_count: 1, status_counts: counts,
    },
    nodes, edges: state.edges, citations: [],
    reading_states: Object.fromEntries(Object.entries(state.statuses).map(([id, status]) => [id, { status, changed_at: stamp(0) }])),
    local_papers: state.papers, acquisitions: {}, expansions: {},
  };
}

const event = (id, session, minutes, kind, over = {}) => ({
  id, occurred_at: stamp(0, minutes), session_id: session, kind, paper_id: null, work_id: null, list_id: null, dwell_ms: null,
  detail: {}, client_seq: id, work_title: null, ...over,
});
const sessions = [
  { session_id: 's-today', started_at: stamp(0, 30), ended_at: stamp(0, 5), events: [
    event(3, 's-today', 5, 'paper_opened', { paper_id: paperId }),
    event(2, 's-today', 15, 'search_run', { detail: { query: 'reward' } }),
    event(1, 's-today', 25, 'graph_opened', { paper_id: paperId, work_id: 'w-a', work_title: 'Attention Is All You Need' }),
  ] },
  { session_id: 's-old', started_at: stamp(2, 30), ended_at: stamp(2, 5), events: [
    { ...event(5, 's-old', 5, 'list_opened', { list_id: listId }), occurred_at: stamp(2, 5) },
    { ...event(4, 's-old', 20, 'section_focused', { paper_id: paperId }), occurred_at: stamp(2, 20) },
  ] },
];

const citations = {
  work: workRecord('a', 'Attention Is All You Need', 2017),
  outgoing: [workRecord('r1', 'Neural machine translation by jointly learning to align', 2014), workRecord('r2', 'Sequence to sequence learning', 2014)],
  incoming: [workRecord('i1', 'BERT', 2018)],
  edges: [], local_papers: {}, acquisitions: {},
};

const browser = await chromium.launch({ headless: true, ...(process.env.LYSILOGY_CHROME === undefined ? {} : { executablePath: process.env.LYSILOGY_CHROME }) });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 850 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  const requests = [];
  const bodies = [];
  const unexpected = [];
  await page.route('**/*', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const method = request.method();
    if (url.pathname.startsWith('/api/')) {
      requests.push(`${method} ${url.pathname}${url.search}`);
      bodies.push(request.postData());
    }
    const json = (body, status = 200) => route.fulfill({ status, json: body });
    const body = () => (request.postData() === null ? {} : JSON.parse(request.postData()));
    const segments = url.pathname.split('/');
    if (url.pathname === '/api/library') return json({ name: 'Synthetic library', papers: [paper] });
    if (url.pathname === '/api/queue') return json({ jobs: [] });
    if (url.pathname === '/api/tags') return json({ tags: [] });
    if (url.pathname === `/api/papers/${paperId}`) return json({ paper, analysis });
    if (url.pathname.endsWith('/map')) return json({ layout: { schema_version: 1, pages: [] }, highlights: [] });
    if (url.pathname === `/api/papers/${paperId}/lists`) {
      const here = state.nodes.find((entry) => entry.work_id === 'w-a');
      return json({ paper_id: paperId, work_id: 'w-a', memberships: here === undefined ? [] : [{ list_id: listId, title: state.title, archived: false, node_id: here.id, topics: [], note: null }] });
    }
    if (url.pathname === `/api/papers/${paperId}/work`) return json({ paper_id: paperId, work: { id: 'w-a' }, link: null });
    if (url.pathname === '/api/history/events') return json({ accepted: 1, duplicates: 0, nudged: [] });
    if (url.pathname === '/api/history') return json({ sessions, next_before: null });
    if (url.pathname === '/api/history/inspected') return json({ works: [{ work_id: 'w-i', title: 'Looked at once', inspects: 1, last_inspected: stamp(0, 3) }] });
    if (url.pathname === `/api/history/papers/${paperId}`) return json({ position: null });
    if (url.pathname.startsWith('/api/papers/') && url.pathname.endsWith('/position')) return json({});
    if (url.pathname === '/api/lists' && method === 'GET') return json({ lists: [detail().list] });
    if (url.pathname === '/api/lists' && method === 'POST') {
            return json(detail(), 201);
    }
    if (url.pathname === `/api/lists/${listId}` && method === 'GET') return json(detail());
    if (url.pathname === `/api/lists/${listId}` && method === 'PATCH') {
      const patch = body();
      if (typeof patch.title === 'string') state.title = patch.title;
      if (patch.archived !== undefined) state.archived = patch.archived === true;
      return json(detail().list);
    }
    if (url.pathname === `/api/lists/${listId}/order`) {
      body().node_ids.forEach((id, index) => { state.nodes.find((entry) => entry.id === id).position = (index + 1) * 10; });
      return json(detail());
    }
    if ((url.pathname === `/api/lists/${listId}/nodes` || url.pathname === `/api/lists/${listId}/nodes:batch`) && method === 'POST') {
      const add = body();
      const adds = add.nodes ?? [add];
      const made = adds.map((one) => {
        counter += 1;
        if (one.topic !== undefined) return node('topic', `n${counter}`, { label: one.topic.label });
        if (one.claim !== undefined && one.work_id === undefined) return node('claim', `n${counter}`, { claim: one.claim.title ?? 'claim', claim_state: 'pending' });
        const restored = state.gone !== null && state.gone.work_id === one.work_id ? state.gone : null;
        return restored === null
          ? workNode(`n${counter}`, one.work_id ?? 'New', 2020, { work_id: one.work_id ?? null })
          : { ...restored, id: `n${counter}`, position: counter * 10 };
      });
      state.nodes.push(...made);
      return json(url.pathname.endsWith(':batch') ? { nodes: made } : made[0], 201);
    }
    if (segments[3] === listId && segments[4] === 'nodes' && segments[5] !== undefined) {
      const index = state.nodes.findIndex((entry) => entry.id === segments[5]);
      if (method === 'DELETE') {
        const [removed] = state.nodes.splice(index, 1);
        state.gone = removed;
        return json({ node: removed, edges: [], was_seed: false });
      }
      if (method === 'PATCH') {
        const patch = body();
        state.nodes[index] = { ...state.nodes[index], ...(patch.note === undefined ? {} : { note: patch.note === '' ? null : patch.note }), ...(patch.review === undefined ? {} : { review: patch.review }) };
        return json(state.nodes[index]);
      }
    }
    if (url.pathname === `/api/lists/${listId}/edges`) {
      if (method === 'POST') {
        const edge = { list_id: listId, ...body(), source: 'user', review: 'accepted', created_at: stamp(0) };
        state.edges.push(edge);
        return json(edge, 201);
      }
      return route.fulfill({ status: 204 });
    }
    if (segments[2] === 'works' && segments[4] === 'reading-state') {
      state.statuses[segments[3]] = body().status;
      return json({ work_id: segments[3], status: body().status, changed_at: stamp(0) });
    }
    if (segments[2] === 'works' && segments[4] === 'lists') return json({ work_id: segments[3], memberships: [] });
    if (segments[2] === 'works' && segments[4] === 'citations') return json(citations);
    if (segments[2] === 'works' && segments[4] === 'acquisition') return json({ work_id: segments[3], job: { state: 'idle' }, acquisition: null, paper_id: null });
    if (segments[2] === 'works' && segments[4] === 'fetch') return json({ work_id: segments[3], job: { state: 'running' }, acquisition: null, paper_id: null }, 202);
    if (url.pathname === '/api/lists/generate') return json({ generation_id: 'g00000000000000000000000000000001' }, 202);
    if (segments[2] === 'generations') return json({ state: 'running', report: {}, spend_usd: 0, list_id: listId });
    if (url.pathname === '/api/works/search') {
      return json({ results: [{ work_id: 'w-new', paper_id: null, title: 'Learning to summarize', year: 2020, authors: ['N Stiennon'], source: 'graph', in_graph: true, openalex_id: null, doi: null }] });
    }
    if (url.pathname.startsWith('/api/')) {
      unexpected.push(`${method} ${url.pathname}`);
      return route.fulfill({ status: 404, json: { message: `Unexpected fixture request ${method} ${url.pathname}` } });
    }
    const file = path.join(root, 'web/dist', url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
    return route.fulfill({ body: await readFile(file), contentType: /\.m?js$/.test(file) ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html' });
  });

  // Calls made since `mark`, with their JSON bodies: the observable effect of a key.
  const since = (mark) => requests.slice(mark).map((call, index) => ({ call, body: bodies[mark + index] === null ? null : JSON.parse(bodies[mark + index] ?? 'null') }));
  const sent = (mark, call, test = () => true) => since(mark).some((entry) => entry.call === call && test(entry.body));
  const press = async (...keys) => { for (const key of keys) await page.keyboard.press(key); };
  const titles = () => page.$$eval('.list-sequence .list-row .list-row-title', (nodes) => nodes.map((entry) => entry.textContent));
  const rowCount = (count) => page.waitForFunction((n) => document.querySelectorAll('.list-sequence .list-row').length === n, count);
  const selected = () => page.locator('.list-row.is-selected .list-row-title').first().innerText();
  // Esc steps back out of a nested stage (topic choice) before it cancels the dialog.
  const closeOverlay = async () => {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      await page.waitForTimeout(300);
      if (await page.locator('[data-list-overlay]').count() === 0) return;
      await page.keyboard.press('Escape');
    }
    await page.waitForSelector('[data-list-overlay]', { state: 'detached' });
  };
  const overlayGone = () => page.waitForSelector('[data-list-overlay]', { state: 'detached' });

  // ---- Lists index: j k gg G Ctrl-d Ctrl-u / n R A za Enter o q -----------------------------
  await page.goto('http://localhost/#lists');
  await page.waitForSelector('.lists-row');
  await press('j', 'ArrowDown', 'k', 'ArrowUp', 'G', 'g', 'g', 'Control+d', 'Control+u');
  await press('/');
  await page.locator('.lists-filter, input[aria-label*="ilter"]').first().fill('rlhf');
  await press('Enter');
  assert.equal(await page.locator('.lists-row').count(), 1);
  await press('Escape');
  await press('R');
  await page.getByRole('textbox', { name: 'List title' }).fill('RLHF fundamentals');
  await press('Enter');
  await page.waitForFunction(() => document.body.textContent.includes('RLHF fundamentals'));
  const archiveMark = requests.length;
  await press('A');
  await page.waitForFunction(() => document.body.textContent.includes('rchived'));
  assert.ok(sent(archiveMark, `PATCH /api/lists/${listId}`, (body) => body.archived === true), requests.slice(archiveMark).join('\n'));
  await press('z', 'a');
  state.archived = false;
  await press('q'); // from the index, back to the home grid
  await page.goto('http://localhost/#lists');
  await page.waitForSelector('.lists-row');
  await press('o'); await page.waitForSelector('.list-sequence .list-row');
  await press('q'); await page.waitForSelector('.lists-row');
  await press('n');
  await page.getByRole('textbox', { name: 'New list title' }).fill('Another list');
  await press('Enter');
  await page.waitForSelector('.list-sequence .list-row');
  assert.ok(requests.some((entry) => entry === 'POST /api/lists'), requests.join('\n'));

  // ---- List page: views 1-7 and v ---------------------------------------------------------
  assert.deepEqual(await titles(), ['Attention Is All You Need', 'Deep RL from human preferences', 'InstructGPT', 'a paper on reward hacking']);
  await press('v'); assert.match(page.url(), /view=2/u);
  await press('v'); assert.match(page.url(), /view=3/u);
  await press('4'); await page.waitForSelector('[data-empty="graph"]');
  assert.match(await page.locator('[data-empty="graph"]').innerText(), /not in this build/iu);
  await press('2'); await page.waitForSelector('.list-col');
  await press('h', 'l', 'ArrowLeft', 'ArrowRight');
  await press('z', 'o', 'z', 'c', 'z', 'a', 'z', 'R', 'z', 'M');
  await press('3'); await page.waitForSelector('.list-col');
  await press('h', 'l');
  await press('5'); await page.waitForSelector('.list-views button[aria-pressed="true"]');
  await press('6'); await page.waitForSelector('[aria-label="Papers to acquire"], .list-empty');
  const fetchMark = requests.length;
  await press('F');
  await press('f');
  await page.waitForTimeout(300);
  const fetched = since(fetchMark).filter((entry) => /^POST \/api\/works\/[^/]+\/fetch/u.test(entry.call));
  assert.ok(fetched.length > 0 || await page.locator('.list-empty').count() > 0, `Acquire f / F sent no fetch: ${requests.slice(fetchMark).join('\n')}`);
  await press('7'); await page.waitForSelector('[data-node-id], .list-row');
  await press('1'); await page.waitForSelector('.list-sequence .list-row');

  // ---- Sequence: movement, status, sort, filter, group, fold -------------------------------
  await press('j', 'ArrowDown', 'k', 'ArrowUp', 'G', 'g', 'g', 'Control+d', 'Control+u', 'g', 'g');
  assert.match(await selected(), /Attention/u);
  await press('j');
  assert.match(await selected(), /Deep RL/u);
  await press('r'); await page.waitForSelector('.list-row[data-status="reading"]');
  await press('r'); await page.waitForSelector('.list-row[data-status="read"]');
  await press('r'); await page.waitForSelector('.list-row.is-selected[data-status="unread"]');
  await press('d', 'd'); await page.waitForSelector('.list-row.is-cut');
  await press('G', 'p');
  await page.waitForFunction(() => document.querySelector('.list-sequence .list-row:last-child .list-row-title')?.textContent === 'Deep RL from human preferences');
  await press('g', 'g', 'd', 'd', 'G', 'P');
  await page.waitForFunction(() => document.querySelector('.list-sequence .list-row:nth-last-child(2) .list-row-title')?.textContent === 'Attention Is All You Need');
  await press('K'); await press('J');
  await press('x'); await rowCount(3);
  await press('u'); await rowCount(4);
  await press('e');
  await page.locator('.list-row-input').fill('start here');
  await press('Enter');
  await page.waitForSelector('.list-row-note');
  await press('s', 's');
  assert.match(await page.locator('.list-toolbar').innerText(), /year/u);
  await press('S');
  await press('/');
  await page.locator('.list-filter').fill('instruct');
  await press('Enter');
  assert.deepEqual(await titles(), ['InstructGPT']);
  await press('Escape');
  assert.equal((await titles()).length, 4);

  // ---- Topics, edges, context, references, claims ------------------------------------------
  await press('O');
  await page.locator('.list-row-input').fill('Rewards');
  await press('Enter');
  await page.waitForFunction(() => document.querySelector('.list-row-input') === null);
  assert.ok(state.nodes.some((entry) => entry.kind === 'topic' && entry.label === 'Rewards'), 'topic created');
  await press('z', 'g'); await press('z', 'g');
  await press('g', 'g', 'j'); // the sort is reversed by year: first row is the claim
  await press('T'); await page.waitForSelector('[data-list-overlay]');
  await press('Escape'); await overlayGone();
  await press('E'); await press('j'); await press('Enter');
  await page.waitForFunction(() => document.querySelector('.list-message')?.textContent?.includes('prerequisite'));
  await press('X'); await page.waitForSelector('[data-list-overlay]');
  await press('Escape'); await overlayGone();
  // C needs a Work with context links; step down until one opens the chooser.
  for (let attempt = 0; attempt < 6 && await page.locator('[data-list-overlay]').count() === 0; attempt += 1) {
    await press('C');
    await page.waitForTimeout(200);
    if (await page.locator('[data-list-overlay]').count() === 0) await press('j');
  }
  await page.getByText('Connects to papers you read').waitFor();
  const contextMark = requests.length;
  await press('Enter'); // opens the linked paper, or says it is not in the library
  await overlayGone();
  await page.waitForTimeout(300);
  assert.ok(sent(contextMark, 'GET /api/works/w-a/citations'), requests.slice(contextMark).join('\n'));
  await press('g', 'g', 'j');
  await press('A'); await page.getByRole('dialog').waitFor();
  await page.getByText('Neural machine translation').waitFor();
  await press('Space'); await press('Tab'); await press('Tab');
  await press('Escape'); await overlayGone();
  await press('g', 'g', 'j');
  await press('A'); await page.getByRole('dialog').waitFor();
  await page.getByText('Neural machine translation').waitFor();
  const referencesMark = requests.length;
  await press('Space'); await press('Enter');
  await overlayGone();
  await page.waitForTimeout(300);
  assert.ok(sent(referencesMark, `POST /api/lists/${listId}/nodes:batch`, (body) => body.nodes.length > 0 && body.nodes.every((one) => typeof one.via_work_id === 'string')), requests.slice(referencesMark).join('\n'));
  await press('a');
  await page.getByRole('textbox', { name: 'Add a paper by name' }).fill('summarize');
  await press('Tab');
  await page.getByText(/provider search on/u).waitFor();
  await press('Tab');
  await page.getByText(/provider search off/u).waitFor();
  await page.getByRole('option', { name: /Learning to summarize/u }).waitFor();
  await press('Enter');
  await page.waitForFunction(() => document.querySelectorAll('.list-sequence .list-row').length >= 5);
  await press('Enter'); // o / Enter open the Work panel
  await press('f');
  await press('q'); // closes the panel before leaving the page

  // ---- Review: a x c Enter Escape ---------------------------------------------------------
  await press('7');
  await page.waitForSelector('[data-node-id="p"]');
  await page.locator('[data-node-id="p"]').click();
  await press('a');
  await page.waitForSelector('[data-node-id="p"]', { state: 'detached' });
  assert.ok(requests.some((entry) => entry === `PATCH /api/lists/${listId}/nodes/p`), requests.join('\n'));
  await page.locator('[data-node-id="k"]').click();
  await press('c'); await press('c'); await press('Escape');
  await press('c'); await press('Enter');
  await press('x');
  await press('1');

  // ---- Reader: M, Y, g l, g r, and the generation command ---------------------------------
  await page.goto(`http://localhost/#paper=${paperId}`);
  await page.waitForSelector('.paper-lists');
  // Picker: j k are typed into the search, so movement is Ctrl-n / Ctrl-p; Enter adds.
  await press('Y'); await page.waitForSelector('[data-list-overlay]');
  await page.getByRole('option').first().waitFor();
  await press('ArrowDown', 'ArrowUp', 'Control+n', 'Control+p');
  let mark = requests.length;
  await press('Enter');
  await closeOverlay();
  assert.ok(sent(mark, `POST /api/lists/${listId}/nodes`), requests.slice(mark).join('\n'));
  // Ctrl-Enter creates a list from the typed title, then adds.
  await press('Y'); await page.waitForSelector('[data-list-overlay]');
  await page.getByRole('textbox', { name: 'Find a list' }).fill('Brand new list');
  mark = requests.length;
  await press('Control+Enter');
  await closeOverlay();
  const created = since(mark);
  assert.deepEqual(created.map((entry) => entry.call).slice(0, 2), ['POST /api/lists', `POST /api/lists/${listId}/nodes`], requests.slice(mark).join('\n'));
  assert.equal(created[0].body.title, 'Brand new list');
  // Tab opens the topic choice: j moves to the first topic, Enter adds under it.
  await press('Y'); await page.waitForSelector('[data-list-overlay]');
  await page.getByRole('option').first().waitFor();
  await press('Tab');
  await page.getByText('Choose a topic').waitFor();
  await press('j');
  mark = requests.length;
  await press('Enter');
  await closeOverlay();
  assert.ok(sent(mark, `POST /api/lists/${listId}/nodes`, (body) => Array.isArray(body.topic_ids) && body.topic_ids.length === 1), requests.slice(mark).join('\n'));
  // Membership: j k move, x removes, u undoes, Enter opens the list.
  await press('M'); await page.waitForSelector('[data-list-overlay]');
  await page.getByRole('option').first().waitFor();
  await press('j', 'k');
  mark = requests.length;
  await press('x');
  await page.getByText(/Press u to undo/u).waitFor();
  assert.ok(requests.slice(mark).some((entry) => entry.startsWith(`DELETE /api/lists/${listId}/nodes/`)), requests.slice(mark).join('\n'));
  mark = requests.length;
  await press('u');
  await page.getByText(/Put back on/u).waitFor();
  assert.ok(sent(mark, `POST /api/lists/${listId}/nodes`, (body) => body.work_id === 'w-a'), requests.slice(mark).join('\n'));
  await page.getByRole('option').first().waitFor();
  await press('Enter');
  await page.waitForFunction(() => window.location.hash.startsWith('#list='));
  await page.goto(`http://localhost/#paper=${paperId}`);
  await page.waitForSelector('.paper-lists');
  await press('M'); await page.waitForSelector('[data-list-overlay]');
  await press('Escape'); await overlayGone();
  await page.keyboard.type(':');
  await page.getByRole('textbox', { name: 'Command' }).fill('generate learn RLHF reward modelling');
  await press('Enter');
  await page.waitForSelector('.generation-strip');
  await press('g', 'l'); await page.waitForSelector('.lists-row');
  assert.match(page.url(), /#lists/u);

  // ---- History: j k gg G Ctrl-d [ ] Enter Y / 1-5 zo zc za zR zM I Escape ------------------
  await press('g', 'r'); await page.waitForSelector('[aria-label="Reading history"] .history-row');
  await press('j', 'ArrowDown', 'k', 'ArrowUp', 'G', 'g', 'g', 'Control+d', 'Control+u');
  await press('z', 'o'); await page.waitForSelector('.history-row:not(.history-session)');
  await press('z', 'c', 'z', 'a', 'z', 'R', 'z', 'M');
  await press('g', 'g');
  const dayAt = await page.locator('.history-row.is-selected').first().innerText();
  await press(']');
  const dayNext = await page.locator('.history-row.is-selected').first().innerText();
  await press('[');
  assert.equal(await page.locator('.history-row.is-selected').first().innerText(), dayAt);
  assert.notEqual(dayNext, dayAt, 'History ] moves to the next day');
  await press('/');
  await page.getByLabel('Filter history').fill('reward');
  await press('Enter');
  await press('Escape');
  for (const kind of ['1', '2', '3', '4', '5']) { await press(kind); await press(kind); }
  await press('Y'); await page.waitForSelector('[data-list-overlay]');
  await press('Escape'); await overlayGone();
  await press('I'); await page.waitForSelector('[aria-label="Inspected works"]');
  await press('Y'); await page.waitForSelector('[data-list-overlay]');
  await press('Escape'); await overlayGone();
  await press('Escape');
  await press('g', 'g');
  await press('o'); // reopen the first session row: toggles it, nothing to open
  await press('Enter');

  assert.deepEqual(unexpected, []);
  assert.deepEqual(errors, []);
  console.log('lists smoke passed');
} finally {
  await browser.close();
}
