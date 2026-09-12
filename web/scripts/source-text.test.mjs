import assert from 'node:assert/strict';
import test from 'node:test';
import { findRegexMatches } from '../src/lib/regexSearch.ts';
import { objectAt, tokensInSpan } from '../src/lib/readingIndex.ts';

test('regex search handles smart case, zero-width Unicode, invalid syntax and bounds', () => {
  assert.deepEqual(findRegexMatches('Optimization optimization', 'optimization').matches, [{start:0,end:12},{start:13,end:25}]);
  assert.equal(findRegexMatches('Optimization optimization', 'Optimization').matches.length,1);
  assert.deepEqual(findRegexMatches('😀 x', '(?=.)').matches, []);
  assert.match(findRegexMatches('source','[').error,/regular expression/i);
  assert.equal(findRegexMatches('a a a','a',2).truncated,true);
});

test('source text objects preserve paragraph boundaries and inner/around whitespace', () => {
  const text='First alpha-beta sentence. Next sentence.\n\nSecond paragraph.';
  const index={text,objects:{word:[{start:6,end:11},{start:12,end:16}],WORD:[{start:6,end:16}],sentence:[{start:0,end:26},{start:26,end:40}],paragraph:[{start:0,end:41},{start:43,end:text.length}]},tokens:[{start:0,end:5},{start:6,end:16},{start:17,end:26}]};
  assert.equal(text.slice(...Object.values(objectAt(index,8,'w',false))),'alpha');
  assert.equal(text.slice(...Object.values(objectAt(index,8,'W',true))),'alpha-beta ');
  assert.equal(text.slice(...Object.values(objectAt(index,8,'p',false))),'First alpha-beta sentence. Next sentence.');
  assert.equal(text.slice(...Object.values(objectAt(index,8,'p',true))),'First alpha-beta sentence. Next sentence.\n\n');
  assert.equal(objectAt(index,8,'z',true),null);
  assert.deepEqual(tokensInSpan(index,{start:7,end:18}),[{start:6,end:16},{start:17,end:26}]);
});
