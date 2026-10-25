import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { KEYMAP, KEYMAP_WAIVERS, findCollisions, helpGroups } from '../src/lib/keymap.ts';

const binding = (keys, scopes) => ({ keys, action: 'x', ...(scopes === undefined ? {} : { scopes }) });
const context = (id, bindings) => ({ id, title: id, help: false, bindings });

test('should find no collisions in the declared keymap', () => {
  assert.deepEqual(findCollisions(), []);
});

test('should report two bindings of one key in the same context', () => {
  const found = findCollisions([context('page', [binding(['x']), binding(['x', 'y'])])], []);
  assert.deepEqual(found.map((entry) => [entry.context, entry.keys, entry.against]), [['page', 'x', 'x']]);
});

test('should report a single key that is the prefix of a sequence', () => {
  const found = findCollisions([context('page', [binding(['g']), binding(['g g'])])], []);
  assert.equal(found.length, 1);
});

test('should allow a sequence and a single key that share only a first letter in different sequences', () => {
  assert.deepEqual(findCollisions([context('page', [binding(['z a']), binding(['z o']), binding(['z'])])], []).length, 2);
  assert.deepEqual(findCollisions([context('page', [binding(['z a']), binding(['z o'])])], []), []);
});

test('should allow one key in two scopes that never overlap', () => {
  assert.deepEqual(findCollisions([context('page', [binding(['a'], ['1']), binding(['a'], ['7'])])], []), []);
});

test('should report one key in overlapping scopes and in an unscoped binding', () => {
  assert.equal(findCollisions([context('page', [binding(['a'], ['1', '7']), binding(['a'], ['7'])])], []).length, 1);
  assert.equal(findCollisions([context('page', [binding(['a']), binding(['a'], ['7'])])], []).length, 1);
});

test('should report a context key that shadows a global binding', () => {
  const found = findCollisions([context('global', [binding(['Y'])]), context('reader', [binding(['Y'])])], []);
  assert.deepEqual(found.map((entry) => [entry.context, entry.other]), [['reader', 'global']]);
});

test('should not report the page that repeats the global key it forwards', () => {
  const page = context('history', [{ keys: ['Y'], action: 'x', global: true }]);
  assert.deepEqual(findCollisions([context('global', [binding(['Y'])]), page], []), []);
});

test('should report a context key that is a prefix of a global sequence unless waived', () => {
  const contexts = [context('global', [binding(['g l'])]), context('graph', [binding(['g'])])];
  assert.equal(findCollisions(contexts, []).length, 1);
  assert.deepEqual(findCollisions(contexts, [{ context: 'graph', keys: 'g', against: 'g l', reason: 'test' }]), []);
});

test('should keep every waiver pointed at a binding that exists', () => {
  for (const waiver of KEYMAP_WAIVERS) {
    const owner = KEYMAP.find((entry) => entry.id === waiver.context);
    assert.ok(owner?.bindings.some((entry) => entry.keys.includes(waiver.keys)), `${waiver.context} ${waiver.keys}`);
    assert.ok(waiver.reason.length > 0);
  }
});

test('should not repeat a key chord twice inside one binding', () => {
  for (const entry of KEYMAP) {
    for (const item of entry.bindings) assert.equal(new Set(item.keys).size, item.keys.length, `${entry.id} ${item.keys}`);
  }
});

test('should show the three list contexts in the key map', () => {
  assert.deepEqual(helpGroups().map((group) => group.title), ['Lists', 'List page', 'History']);
});

// Drift: every key the pages handle is declared. Source scan, not behaviour: it catches a new
// `case "z":` that nobody added to the table.
const KEY_TOKEN = /^(.|Enter|Escape|Arrow\w+)$/u;

const declared = (id) => new Set(KEYMAP.find((entry) => entry.id === id).bindings.flatMap((entry) => entry.keys).flatMap((chord) => chord.split(' ')));
const ctrl = (set) => new Set([...set].flatMap((chord) => (chord.startsWith('Ctrl-') ? [chord.slice(5)] : [chord])));

for (const [file, id, pattern] of [
  ['components/ListPage.tsx', 'list-page', /case "([^"]+)":|\bkey === "([^"]+)"/gu],
  ['components/ListsPage.tsx', 'lists', /\bkey === "([^"]+)"/gu],
  ['components/HistoryPage.tsx', 'history', /\bkey === "([^"]+)"/gu],
]) {
  test(`should declare every key ${file} handles`, async () => {
    const source = await readFile(new URL(`../src/${file}`, import.meta.url), 'utf8');
    const seen = new Set([...source.matchAll(pattern)].flatMap((match) => [match[1], match[2]]).filter((token) => token !== undefined && KEY_TOKEN.test(token)));
    const known = ctrl(declared(id));
    // "d" and "u" under Ctrl are the half-screen keys; ListPage's `case "d"`-style names do not occur.
    const missing = [...seen].filter((token) => !known.has(token));
    assert.deepEqual(missing, [], `${file} handles keys missing from the ${id} table`);
  });
}

test('should declare every sequence the list pages recognise', async () => {
  const source = await readFile(new URL('../src/lib/listKeys.ts', import.meta.url), 'utf8');
  const sequences = [...source.matchAll(/^\s+([a-zA-Z]{2}): "[a-z-]+",$/gmu)].map((match) => match[1].split('').join(' '));
  assert.ok(sequences.length >= 8, sequences.join(','));
  const listPage = new Set(KEYMAP.find((entry) => entry.id === 'list-page').bindings.flatMap((entry) => entry.keys));
  const missing = sequences.filter((chord) => !listPage.has(chord));
  assert.deepEqual(missing, []);
});

test('should declare the view keys one to seven on the list page and one to five in History', () => {
  const keys = (id) => new Set(KEYMAP.find((entry) => entry.id === id).bindings.flatMap((entry) => entry.keys));
  for (const digit of ['1', '2', '3', '4', '5', '6', '7']) assert.ok(keys('list-page').has(digit), digit);
  for (const digit of ['1', '2', '3', '4', '5']) assert.ok(keys('history').has(digit), digit);
});
