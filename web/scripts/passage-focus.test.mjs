import assert from 'node:assert/strict';
import test from 'node:test';
import { locatePassage } from '../src/lib/passageFocus.ts';

const sentence = 'We build on prior work [1] for reward learning.';
const text = `Intro text here.\n\nWe build on prior work [1]\nfor   reward learning. Later text follows.`;
const collapsedSentence = sentence;

test('should return the stored offsets when they still select the passage', () => {
  const start = text.indexOf('We build');
  const end = text.indexOf(' Later');
  assert.deepEqual(locatePassage(text, start, end, collapsedSentence), { start, end });
});

test('should find the passage when the stored offsets have drifted', () => {
  const start = text.indexOf('We build');
  const end = text.indexOf(' Later');
  const found = locatePassage(text, start + 7, end + 9, collapsedSentence);
  assert.deepEqual(found, { start, end });
});

test('should keep exact offsets when an astral character precedes the span', () => {
  const body = `Emoji \u{1F600}\u{1F600} then. ${sentence} After.`;
  const start = body.indexOf('We build');
  const end = start + sentence.length;
  assert.deepEqual(locatePassage(body, start, end, sentence), { start, end });
  // Offsets counted in code points would be off by two; the search recovers them.
  assert.deepEqual(locatePassage(body, start - 2, end - 2, sentence), { start, end });
});

test('should fall back to the passage prefix when the tail differs', () => {
  const head = 'Following Christiano et al. (2017), we ask human labelers to pick the better summary';
  const body = `Intro.\n\n${head} from two options shown side by side. Next sentence.`;
  const stored = `${head} but this stored tail differs from the paper text.`;
  const found = locatePassage(body, 0, 5, stored);
  assert.ok(found !== null);
  assert.equal(found.start, body.indexOf('Following'));
  assert.ok(found.end > found.start + head.length - 1 && found.end <= body.length);
});

test('should return null when the passage is absent', () => {
  assert.equal(locatePassage(text, 0, 10, 'A sentence the paper never says anywhere at all.'), null);
});

test('should return null when the passage is empty', () => {
  assert.equal(locatePassage(text, 0, 10, ''), null);
  assert.equal(locatePassage(text, 0, 10, '   \n  '), null);
});

test('should prefer the occurrence nearest the stored offset when the passage repeats', () => {
  const repeated = `${sentence} Middle part. ${sentence} End.`;
  const second = repeated.lastIndexOf(sentence);
  const found = locatePassage(repeated, second + 3, second + 9, sentence);
  assert.deepEqual(found, { start: second, end: second + sentence.length });
});

test('should not throw when offsets are out of range or not integers', () => {
  assert.deepEqual(locatePassage(text, -5, 99999, collapsedSentence), { start: text.indexOf('We build'), end: text.indexOf(' Later') });
  assert.ok(locatePassage(text, Number.NaN, 4.5, collapsedSentence) !== null);
});
