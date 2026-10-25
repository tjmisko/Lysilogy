import { test } from 'node:test';
import assert from 'node:assert/strict';
import { authorSurnames } from '../src/lib/authors.ts';

test('should return the surname when one full name is given', () => {
  assert.equal(authorSurnames(['Marco Locatelli']), 'Locatelli');
});

test('should join two surnames with and when two authors are given', () => {
  assert.equal(authorSurnames(['Geoffrey Irving', 'Paul Christiano']), 'Irving and Christiano');
});

test('should collapse to et al. when three or more authors are given', () => {
  assert.equal(authorSurnames(['Geoffrey Irving', 'Paul Christiano', 'Dario Amodei']), 'Irving et al.');
});

test('should keep a surname-only entry unchanged', () => {
  assert.equal(authorSurnames(['Shannon']), 'Shannon');
});

test('should keep already-short entries when the entry holds several names', () => {
  assert.equal(authorSurnames(['Axsen and Long']), 'Axsen and Long');
  assert.equal(authorSurnames(['Greenblatt et al.']), 'Greenblatt et al.');
});

test('should read Last, First order when a single comma is present', () => {
  assert.equal(authorSurnames(['Shannon, Claude E.']), 'Shannon');
});

test('should keep name particles and drop suffixes when extracting surnames', () => {
  assert.equal(authorSurnames(['Ludwig van Beethoven']), 'van Beethoven');
  assert.equal(authorSurnames(['Martin Luther King Jr.']), 'King');
});

test('should return an empty string when no authors are given', () => {
  assert.equal(authorSurnames([]), '');
  assert.equal(authorSurnames(['  ']), '');
});
