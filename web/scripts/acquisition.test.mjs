import assert from 'node:assert/strict';
import test from 'node:test';
import { acquisitionStatusText, canFetch, isRetry, unavailableReasonText } from '../src/lib/acquisition.ts';

test('should describe each stage in words when a fetch is running', () => {
  assert.equal(acquisitionStatusText(null, 'running'), 'Resolving identifiers…');
  assert.equal(acquisitionStatusText({ state: 'unresolved' }, 'running'), 'Resolving identifiers…');
  assert.equal(acquisitionStatusText({ state: 'identifier_found' }, 'running'), 'Looking for an open-access copy…');
  assert.equal(acquisitionStatusText({ state: 'link_found' }, 'running'), 'Downloading…');
  assert.equal(acquisitionStatusText({ state: 'downloaded' }, 'running'), 'Saving to the library…');
  assert.equal(acquisitionStatusText({ state: 'mapped' }, 'running'), 'Fetched.');
});

test('should say not fetched yet when there is no acquisition', () => {
  assert.equal(acquisitionStatusText(null, 'none'), 'Not fetched yet.');
  assert.equal(acquisitionStatusText(undefined, 'idle'), 'Not fetched yet.');
  assert.equal(acquisitionStatusText({ state: 'unresolved' }, 'idle'), 'Not fetched yet.');
});

test('should say fetched when the acquisition is mapped and idle', () => {
  assert.equal(acquisitionStatusText({ state: 'mapped' }, 'succeeded'), 'Fetched.');
});

test('should suggest trying again when a fetch stopped midway', () => {
  for (const state of ['identifier_found', 'link_found', 'downloaded']) {
    assert.match(acquisitionStatusText({ state }, 'idle'), /did not finish/u);
  }
});

test('should report failure when the job failed without a record', () => {
  assert.equal(acquisitionStatusText(null, 'failed'), 'The fetch failed.');
});

test('should name the reason and check date when unavailable', () => {
  const text = acquisitionStatusText(
    { state: 'unavailable', unavailable_reason: 'no_oa_location', checked_at: '2026-10-13T08:00:00Z' },
    'idle',
  );
  assert.equal(text, 'Unavailable: no open-access copy was found (checked 2026-10-13).');
});

test('should omit the check date when it is missing or malformed', () => {
  for (const checked_at of [null, undefined, '', 'yesterday']) {
    const text = acquisitionStatusText({ state: 'unavailable', unavailable_reason: 'no_identifier', checked_at }, 'idle');
    assert.equal(text, 'Unavailable: the work has no DOI, arXiv ID, or OpenAlex ID to look up.');
  }
});

test('should describe every known unavailable reason in words', () => {
  assert.match(unavailableReasonText('no_identifier'), /no DOI/u);
  assert.match(unavailableReasonText('no_oa_location'), /no open-access copy/u);
  assert.match(unavailableReasonText('all_links_failed'), /none could be downloaded/u);
});

test('should fall back to a generic reason when the reason is unknown', () => {
  const generic = 'no open-access copy could be fetched';
  for (const reason of ['something_new', '', null, undefined, '__proto__', 'constructor', 'toString']) {
    assert.equal(unavailableReasonText(reason), generic, String(reason));
  }
});

test('should give generic text when the state string is unknown', () => {
  assert.equal(acquisitionStatusText({ state: 'quantum' }, 'idle'), 'The fetch status is unknown.');
  assert.equal(acquisitionStatusText({ state: 'quantum' }, 'running'), 'Resolving identifiers…');
});

test('should refuse to fetch when a local paper exists', () => {
  assert.equal(canFetch('w1', { w1: [{ paper_id: 'p' }] }, {}, 'none'), false);
});

test('should allow fetch when the local paper list is empty or absent', () => {
  assert.equal(canFetch('w1', { w1: [] }, {}, 'none'), true);
  assert.equal(canFetch('w1', {}, {}, 'idle'), true);
  assert.equal(canFetch('w1', { other: [{}] }, {}, 'none'), true);
});

test('should refuse to fetch when a job is running', () => {
  assert.equal(canFetch('w1', {}, {}, 'running'), false);
});

test('should allow fetch as a retry when the work is unavailable', () => {
  const acquisitions = { w1: { state: 'unavailable', unavailable_reason: 'all_links_failed' } };
  assert.equal(canFetch('w1', {}, acquisitions, 'idle'), true);
  assert.equal(isRetry(acquisitions.w1), true);
  assert.equal(isRetry({ state: 'link_found' }), false);
  assert.equal(isRetry(null), false);
});
