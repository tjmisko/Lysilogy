import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { KEYMAP, KEYMAP_WAIVERS, findCollisions, helpGroups } from '../src/lib/keymap.ts';

const binding = (keys, scopes) => ({ keys, action: 'x', ...(scopes === undefined ? {} : { scopes }) });
const context = (id, bindings) => ({ id, title: id, help: false, bindings });

test('should find no collisions when the declared keymap is audited', () => {
  assert.deepEqual(findCollisions(), []);
});

test('should report the collision when two bindings in one context share a key', () => {
  const found = findCollisions([context('page', [binding(['x']), binding(['x', 'y'])])], []);
  assert.deepEqual(found.map((entry) => [entry.context, entry.keys, entry.against]), [['page', 'x', 'x']]);
});

test('should report the collision when a single key is the prefix of a sequence', () => {
  const found = findCollisions([context('page', [binding(['g']), binding(['g g'])])], []);
  assert.equal(found.length, 1);
});

test('should report collisions when a bare z is declared beside z a and z o', () => {
  assert.equal(findCollisions([context('page', [binding(['z a']), binding(['z o']), binding(['z'])])], []).length, 2);
});

test('should allow sequences when they share only a first letter', () => {
  assert.deepEqual(findCollisions([context('page', [binding(['z a']), binding(['z o'])])], []), []);
});

test('should allow one key when the two scopes never overlap', () => {
  assert.deepEqual(findCollisions([context('page', [binding(['a'], ['1']), binding(['a'], ['7'])])], []), []);
});

test('should report one key when the scopes overlap or one binding is unscoped', () => {
  assert.equal(findCollisions([context('page', [binding(['a'], ['1', '7']), binding(['a'], ['7'])])], []).length, 1);
  assert.equal(findCollisions([context('page', [binding(['a']), binding(['a'], ['7'])])], []).length, 1);
});

test('should report a context key when it shadows a global binding', () => {
  const found = findCollisions([context('global', [binding(['Y'])]), context('reader', [binding(['Y'])])], []);
  assert.deepEqual(found.map((entry) => [entry.context, entry.other]), [['reader', 'global']]);
});

test('should not report a page key when it repeats the global key it forwards', () => {
  const page = context('history', [{ keys: ['Y'], action: 'x', global: true }]);
  assert.deepEqual(findCollisions([context('global', [binding(['Y'])]), page], []), []);
});

test('should report a context key when it prefixes a global sequence that is not waived', () => {
  const contexts = [context('global', [binding(['g l'])]), context('graph', [binding(['g'])])];
  assert.equal(findCollisions(contexts, []).length, 1);
  assert.deepEqual(findCollisions(contexts, [{ context: 'graph', keys: 'g', against: 'g l', reason: 'test' }]), []);
});

test('should point every waiver at a binding when the waiver is declared', () => {
  for (const waiver of KEYMAP_WAIVERS) {
    const owner = KEYMAP.find((entry) => entry.id === waiver.context);
    assert.ok(owner?.bindings.some((entry) => entry.keys.includes(waiver.keys)), `${waiver.context} ${waiver.keys}`);
    assert.ok(waiver.reason.length > 0);
  }
});

test('should list each chord once when a binding has alternatives', () => {
  for (const entry of KEYMAP) {
    for (const item of entry.bindings) assert.equal(new Set(item.keys).size, item.keys.length, `${entry.id} ${item.keys}`);
  }
});

test('should show the list, picker, membership and history contexts when the key map renders', () => {
  assert.deepEqual(helpGroups().map((group) => group.title), ['Lists', 'List page', 'Add to a list (Y)', 'Lists this paper is on (M)', 'History']);
});

// Drift: every key the pages handle is declared. Source scan, not behaviour: it catches a new
// `case "z":` that nobody added to the table. A bare key must match a non-Ctrl chord; only a line
// that mentions ctrlKey (or follows one within three lines) may also match a Ctrl- chord.
const KEY_TOKEN = /^(.|Enter|Escape|Arrow\w+|Tab)$/u;

const chords = (id) => KEYMAP.find((entry) => entry.id === id).bindings.flatMap((entry) => entry.keys);
const bare = (id) => new Set(chords(id).filter((chord) => !chord.startsWith('Ctrl-')).flatMap((chord) => chord.split(' ')));
const withCtrl = (id) => new Set([...bare(id), ...chords(id).filter((chord) => chord.startsWith('Ctrl-')).map((chord) => chord.slice(5))]);

for (const [file, id, pattern] of [
  ['components/ListPage.tsx', 'list-page', /case "([^"]+)":|\bkey === "([^"]+)"/gu],
  ['components/ListsPage.tsx', 'lists', /\bkey === "([^"]+)"/gu],
  ['components/HistoryPage.tsx', 'history', /\bkey === "([^"]+)"/gu],
  ['components/ListViews/Acquire.tsx', 'list-page', /\bkey === "([^"]+)"/gu],
  ['components/ListViews/ColumnKeys.ts', 'list-page', /\bkey === "([^"]+)"/gu],
  ['components/ListPicker.tsx', 'picker', /\bkey === "([^"]+)"/gu],
  ['components/ListMembership.tsx', 'membership', /\bkey === "([^"]+)"/gu],
]) {
  test(`should declare every key when ${file} handles it`, async () => {
    const source = await readFile(new URL(`../src/${file}`, import.meta.url), 'utf8');
    const missing = [];
    const lines = source.split('\n');
    for (const [at, line] of lines.entries()) {
      const known = lines.slice(Math.max(0, at - 3), at + 1).some((near) => near.includes('ctrlKey')) ? withCtrl(id) : bare(id);
      for (const match of line.matchAll(pattern)) {
        const token = match[1] ?? match[2];
        if (token !== undefined && KEY_TOKEN.test(token) && !known.has(token)) missing.push(token);
      }
    }
    assert.deepEqual([...new Set(missing)], [], `${file} handles keys missing from the ${id} table`);
  });
}

test('should declare every sequence when the list pages recognise it', async () => {
  const source = await readFile(new URL('../src/lib/listKeys.ts', import.meta.url), 'utf8');
  const sequences = [...source.matchAll(/^\s+([a-zA-Z]{2}): "[a-z-]+",$/gmu)].map((match) => match[1].split('').join(' '));
  assert.ok(sequences.length >= 8, sequences.join(','));
  const listPage = new Set(KEYMAP.find((entry) => entry.id === 'list-page').bindings.flatMap((entry) => entry.keys));
  const missing = sequences.filter((chord) => !listPage.has(chord));
  assert.deepEqual(missing, []);
});

test('should declare view keys one to seven on the list page when History takes one to five', () => {
  const keys = (id) => new Set(KEYMAP.find((entry) => entry.id === id).bindings.flatMap((entry) => entry.keys));
  for (const digit of ['1', '2', '3', '4', '5', '6', '7']) assert.ok(keys('list-page').has(digit), digit);
  for (const digit of ['1', '2', '3', '4', '5']) assert.ok(keys('history').has(digit), digit);
});

test('should bind L to the Graph view on the list page when digit four also selects it', () => {
  const page = KEYMAP.find((entry) => entry.id === 'list-page');
  assert.ok(page.bindings.some((entry) => entry.keys.includes('L')));
  assert.ok(page.bindings.some((entry) => entry.keys.includes('4')));
  assert.deepEqual(findCollisions().filter((entry) => entry.keys === 'L'), []);
});
