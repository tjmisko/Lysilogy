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

function motionIndex(text, words, bigWords, sentences = []) {
  const spans = (values) => values.map(value => {
    const start = text.indexOf(value);
    assert.notEqual(start, -1);
    return {start, end:start+value.length};
  });
  return {text,objects:{word:spans(words),WORD:spans(bigWords),sentence:spans(sentences),paragraph:[]}};
}

const { sourceCursor, sourceGrapheme, inclusiveSourceSpan, moveSourceCursor } = await import('../src/lib/sourceMotions.ts');

test('Vim word motions distinguish punctuation groups and WORD, including forward/reverse ends', () => {
  const text='alpha-beta  gamma!\nδelta';
  const index=motionIndex(text,['alpha','-','beta','gamma','!','δelta'],['alpha-beta','gamma!','δelta']);
  const alphaEnd=text.indexOf('-')-1;
  const hyphen=text.indexOf('-');
  const betaStart=text.indexOf('beta');
  const betaEnd=betaStart+3;
  const gammaStart=text.indexOf('gamma');
  const gammaEnd=gammaStart+4;
  const bang=text.indexOf('!');
  assert.equal(moveSourceCursor(index,2,'e'),alphaEnd,'e ends the current word');
  assert.equal(moveSourceCursor(index,alphaEnd,'e'),hyphen,'e on an end advances to the punctuation word');
  assert.equal(moveSourceCursor(index,hyphen,'e'),betaEnd,'e advances to the next word end');
  assert.equal(moveSourceCursor(index,betaStart+1,'ge'),hyphen,'ge from inside a word reaches its preceding end');
  assert.equal(moveSourceCursor(index,betaEnd,'ge'),hyphen,'ge on an end also reaches the preceding end');
  assert.equal(moveSourceCursor(index,hyphen,'ge'),alphaEnd);
  assert.equal(moveSourceCursor(index,2,'E'),betaEnd,'E includes hyphenated WORD');
  assert.equal(moveSourceCursor(index,betaEnd,'E'),bang,'E on an end advances to the next WORD end');
  assert.equal(moveSourceCursor(index,gammaStart+1,'gE'),betaEnd);
  assert.equal(moveSourceCursor(index,bang,'gE'),betaEnd);
  assert.equal(moveSourceCursor(index,gammaStart,'e'),gammaEnd);
  assert.equal(moveSourceCursor(index,gammaEnd,'e'),bang);
  assert.equal(moveSourceCursor(index,2,'w'),hyphen);
  assert.equal(moveSourceCursor(index,2,'W'),gammaStart);
  assert.equal(moveSourceCursor(index,betaStart+1,'b'),betaStart);
  assert.equal(moveSourceCursor(index,betaStart,'b'),hyphen);
  assert.equal(moveSourceCursor(index,betaStart+1,'B'),0);
  assert.equal(moveSourceCursor(index,gammaStart,'B'),0);
  assert.equal(moveSourceCursor(index,2,'e',3),betaEnd);
  assert.equal(moveSourceCursor(index,bang,'ge',2),betaEnd);
  assert.equal(moveSourceCursor(index,0,'b'),0);
  assert.equal(moveSourceCursor(index,text.length-1,'e'),text.length-1);
  assert.equal(moveSourceCursor(index,0,'E',99999),text.length-1);
  assert.equal(moveSourceCursor(index,2,'e',0),2);
  assert.equal(moveSourceCursor(index,2,'not-a-motion'),null);
});

test('sentence-start motions work from inside/on boundaries and preserve inclusive selection direction', () => {
  const text='First sentence. Second sentence!\n\nThird one?';
  const index=motionIndex(text,[],[],['First sentence.','Second sentence!','Third one?']);
  const second=text.indexOf('Second');
  const third=text.indexOf('Third');
  assert.equal(moveSourceCursor(index,0,')'),second);
  assert.equal(moveSourceCursor(index,5,')'),second);
  assert.equal(moveSourceCursor(index,second,')'),third);
  assert.equal(moveSourceCursor(index,second+6,'('),second);
  assert.equal(moveSourceCursor(index,second,'('),0);
  assert.equal(moveSourceCursor(index,third,'('),second);
  assert.equal(moveSourceCursor(index,0,')',2),third);
  assert.equal(moveSourceCursor(index,third,'(',2),0);
  assert.equal(moveSourceCursor(index,third,')'),third);
  assert.equal(moveSourceCursor(index,0,'('),0);
  const forward=inclusiveSourceSpan(text,0,moveSourceCursor(index,0,')'));
  assert.equal(text.slice(forward.start,forward.end),'First sentence. S');
  const reversed=inclusiveSourceSpan(text,third,moveSourceCursor(index,third,'('));
  assert.equal(text.slice(reversed.start,reversed.end),'Second sentence!\n\nT');
  assert.deepEqual(inclusiveSourceSpan(text,reversed.start,third),reversed);
});

test('character motions and reversed inclusive spans retain combining marks, surrogate pairs, ZWJ and flags', () => {
  const text='Ae\u0301😀👩‍🔬🇺🇸Z';
  const index=motionIndex(text,[text],[text]);
  const starts=[...new Intl.Segmenter(undefined,{granularity:'grapheme'}).segment(text)].map(item=>item.index);
  assert.deepEqual(starts,[0,1,3,5,10,14]);
  assert.deepEqual(sourceGrapheme(text,2),{start:1,end:3});
  assert.deepEqual(sourceGrapheme(text,4),{start:3,end:5});
  assert.deepEqual(sourceGrapheme(text,8),{start:5,end:10});
  assert.deepEqual(sourceGrapheme(text,12),{start:10,end:14});
  assert.equal(sourceCursor(text,12),10);
  for(let at=0;at<starts.length;at++) {
    assert.equal(moveSourceCursor(index,starts[at],'l'),starts[Math.min(starts.length-1,at+1)]);
    assert.equal(moveSourceCursor(index,starts[at],'h'),starts[Math.max(0,at-1)]);
  }
  assert.equal(moveSourceCursor(index,2,'ArrowRight'),3);
  assert.equal(moveSourceCursor(index,8,'ArrowLeft'),3);
  assert.equal(moveSourceCursor(index,0,'l',4),10);
  assert.equal(moveSourceCursor(index,14,'h',3),3);
  const span=inclusiveSourceSpan(text,12,2);
  assert.deepEqual(span,{start:1,end:14});
  assert.equal(text.slice(span.start,span.end),'e\u0301😀👩‍🔬🇺🇸');
  assert.deepEqual(inclusiveSourceSpan(text,2,12),span);
  assert.deepEqual(inclusiveSourceSpan('',0,99),{start:0,end:0});
  assert.deepEqual(sourceGrapheme(text,-1),{start:0,end:1});
  assert.deepEqual(sourceGrapheme(text,Infinity),{start:14,end:15});
});

test('word-end motion lands at the start of the final grapheme without losing its combining characters', () => {
  const text='cafe\u0301 go😀';
  const index=motionIndex(text,['cafe\u0301','go','😀'],['cafe\u0301','go😀']);
  assert.equal(moveSourceCursor(index,0,'e'),3);
  assert.deepEqual(inclusiveSourceSpan(text,0,moveSourceCursor(index,0,'e')),{start:0,end:5});
  assert.equal(moveSourceCursor(index,3,'e'),7);
  assert.equal(moveSourceCursor(index,6,'E'),8);
  assert.deepEqual(inclusiveSourceSpan(text,6,moveSourceCursor(index,6,'E')),{start:6,end:10});
  assert.equal(moveSourceCursor(index,8,'gE'),3);
});

const { moveSourceLine } = await import('../src/lib/sourceMotions.ts');
const { sourceSpaceAt } = await import('../src/lib/readingIndex.ts');
function geometricIndex(rows) {
  const index={schema_version:2,text:'',tokens:[],pages:[],objects:{word:[],WORD:[],sentence:[],paragraph:[]},figures:[],gaps:[]};
  for(const row of rows) {
    if(index.text.length) index.text+='\n\n';
    for(const [number,word] of row.words.entries()) {
      if(number>0)index.text+=' ';
      const start=index.text.length;index.text+=word.text;
      index.tokens.push({start,end:index.text.length,page:row.page??1,text:word.text,provenance:'native',rects:[{x_min:word.x,y_min:row.y,x_max:word.x+word.width,y_max:row.y+10}]});
    }
  }
  for(const page of [...new Set(rows.map(row=>row.page??1))]) {
    const tokens=index.tokens.filter(token=>token.page===page);
    index.pages.push({number:page,width:612,height:792,start:tokens[0].start,end:tokens.at(-1).end,provenance:'native',confidence:null});
  }
  return index;
}

test('counted vertical motions preserve the original x across short lines and reverse direction', () => {
  const index=geometricIndex([
    {y:10,words:[{text:'abcdefghij',x:40,width:100}]},
    {y:30,words:[{text:'xy',x:40,width:20}]},
    {y:50,words:[{text:'ABCDEFGHIJ',x:40,width:100}]},
  ]);
  const start=5, second=index.tokens[1].start, third=index.tokens[2].start;
  assert.equal(moveSourceLine(index,start,1),second+1,'nearest character on short line');
  assert.equal(moveSourceLine(index,start,1,2),third+5,'count retains x across the short intermediate line');
  assert.equal(moveSourceLine(index,third+5,-1,2),start);
  assert.equal(moveSourceLine(index,start,-1),start,'document boundary does not move');
  assert.equal(moveSourceLine(index,start,1,9999),third+5,'large counts stop on the last available line');
  assert.equal(moveSourceLine(index,start,1,0),start);
});

test('vertical motions recover inter-word spaces and paragraph newline cursors', () => {
  const index=geometricIndex([
    {y:10,words:[{text:'a',x:40,width:10},{text:'b',x:60,width:10}]},
    {y:30,words:[{text:'abc',x:40,width:30}]},
    {y:50,words:[{text:'DEF',x:40,width:30}]},
  ]);
  const second=index.tokens[2].start;
  assert.equal(moveSourceLine(index,1,1),second+1,'space retains its middle x');
  assert.equal(moveSourceLine(index,second+1,-1),1,'can land in a real inter-word gap');
  const newline=index.tokens[1].end;
  assert.equal(moveSourceLine(index,newline,1),second+2,'newline down reaches following visible line');
  assert.equal(moveSourceLine(index,newline,-1),index.tokens[1].start,'newline up reaches preceding visible line without skipping it');
  assert.equal(moveSourceLine(index,newline+1,-1),index.tokens[1].start);
});

test('vertical motion follows reading order across columns/pages without skipping the last line on reverse', () => {
  const index=geometricIndex([
    {page:1,y:10,words:[{text:'abcdef',x:40,width:60}]},
    {page:1,y:30,words:[{text:'ghijkl',x:40,width:60}]},
    {page:1,y:10,words:[{text:'mnopqr',x:350,width:60}]},
    {page:3,y:10,words:[{text:'uvwxyz',x:350,width:60}]},
  ]);
  assert.equal(moveSourceLine(index,2,1),index.tokens[1].start+2,'stays in left column until its next line is exhausted');
  assert.equal(moveSourceLine(index,index.tokens[1].start+2,1),index.tokens[2].start,'continues at top of next column');
  assert.equal(moveSourceLine(index,index.tokens[2].start+3,1),index.tokens[3].start+3,'skips pages without indexed lines');
  assert.equal(moveSourceLine(index,index.tokens[3].start+3,-1),index.tokens[2].start+3,'reverse page transition uses previous page last line');
});

test('vertical motion lands on complete Unicode graphemes and tolerates missing geometry', () => {
  const index=geometricIndex([
    {y:10,words:[{text:'abcd',x:40,width:40}]},
    {y:30,words:[{text:'A😀e\u0301Z',x:40,width:40}]},
  ]);
  assert.equal(moveSourceLine(index,1,1),index.tokens[1].start+1);
  assert.equal(moveSourceLine(index,2,1),index.tokens[1].start+3);
  assert.equal(moveSourceLine({...index,tokens:[]},1,1),null);
  const withoutFirst={...index,tokens:index.tokens.map((token,i)=>i===0?{...token,rects:[]}:token)};
  assert.equal(moveSourceLine(withoutFirst,0,1),index.tokens[1].start,'missing initial geometry falls forward to first usable line');
});

test('sourceSpaceAt supplies ordinary source gaps but never fabricates newline or column geometry', () => {
  const index=geometricIndex([
    {y:10,words:[{text:'a',x:40,width:10},{text:'b',x:60,width:10}]},
    {y:30,words:[{text:'c',x:40,width:10}]},
  ]);
  const space=sourceSpaceAt(index,1);
  assert.deepEqual({start:space.start,end:space.end,text:space.text,page:space.page},{start:1,end:2,text:' ',page:1});
  assert.deepEqual(space.rects,[{x_min:50,x_max:60,y_min:10,y_max:20}]);
  assert.equal(sourceSpaceAt(index,0),null);
  assert.equal(sourceSpaceAt(index,index.tokens[1].end),null);
  const column=geometricIndex([{y:10,words:[{text:'left',x:40,width:40},{text:'right',x:350,width:50}]}]);
  assert.equal(sourceSpaceAt(column,column.tokens[0].end),null);
  const pages=geometricIndex([{page:1,y:10,words:[{text:'left',x:40,width:40}]},{page:2,y:10,words:[{text:'right',x:90,width:50}]}]);
  assert.equal(sourceSpaceAt(pages,pages.tokens[0].end),null);
});

const { selectionText, selectionWithinSpan, skipSelectionGap, selectionSpans } = await import('../src/lib/readingIndex.ts');

test('logical paragraph selection skips floats from either page and keeps captions independently selectable', () => {
  const chunks = ['We compare the estimator, which', 'Table 5. Accuracy 50.3', 'Figure 6. Confusion matrices.', 'was used to provide annotations with UmeTrack.', 'Additionally, another paragraph.'];
  const text = chunks.join('\n\n');
  const parts = chunks.map(chunk=>({start:text.indexOf(chunk),end:text.indexOf(chunk)+chunk.length}));
  const paragraph = {...parts[0],end:parts[3].end,spans:[parts[0],parts[3]],kind:'body'};
  const index = {text,objects:{word:[],WORD:[],sentence:[],paragraph:[paragraph,{...parts[1],kind:'float'},{...parts[2],kind:'caption'},{...parts[4],kind:'body'}]},tokens:parts.map((part,i)=>({...part,text:chunks[i],page:i===0?1:2}))};
  const inner = objectAt(index, text.indexOf('compare'), 'p', false);
  assert.equal(selectionText(index,inner),chunks[0]+' '+chunks[3]);
  assert.deepEqual(objectAt(index,text.indexOf('provide'),'p',false),inner,'the continuation resolves to the same whole paragraph');
  assert.equal(selectionText(index,objectAt(index,text.indexOf('Confusion'),'p',false)),chunks[2]);
  const around = objectAt(index,text.indexOf('compare'),'p',true);
  assert.equal(selectionText(index,around),chunks[0]+' '+chunks[3]+'\n\n');
  assert.deepEqual(tokensInSpan(index,around).map(token=>token.text),[chunks[0],chunks[3]]);
  assert.equal(selectionText(index,selectionWithinSpan(around,{start:around.start,end:parts[3].end-1})),chunks[0]+' '+chunks[3].slice(0,-1),'shrinking the endpoint does not include floats');
  assert.equal(selectionText(index,selectionWithinSpan(around,{start:parts[0].start,end:parts[0].end})),chunks[0]);
  assert.equal(skipSelectionGap(around,parts[0].end,true),parts[3].start);
  assert.equal(skipSelectionGap(around,parts[3].start-1,false),parts[0].end-1);
  assert.equal(skipSelectionGap(around,parts[4].start,true),parts[4].start,'explicit motions may extend beyond the object');
  const extended=selectionWithinSpan(around,{start:parts[0].start,end:text.length});
  assert.deepEqual(selectionSpans(extended),[parts[0],{start:parts[3].start,end:text.length}]);
});
