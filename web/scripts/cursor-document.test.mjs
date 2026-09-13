import assert from 'node:assert/strict';
import test from 'node:test';
import { cursorDocument, cursorLine, cursorBlock, cursorSelection, cursorGrapheme, cursorCopyText, virtualCursor, originalCursor, moveCursorScreen } from '../src/lib/cursorDocument.ts';

function fixture() {
  const index={schema_version:5,text:'',tokens:[],pages:[],objects:{word:[],WORD:[],sentence:[],paragraph:[]},figures:[],gaps:[]};
  const add=(text,y,page=1,kind='body')=>{
    const start=index.text.length;
    for(const m of text.matchAll(/\S+/gu))index.tokens.push({start:start+m.index,end:start+m.index+m[0].length,text:m[0],page,provenance:'native',rects:[{x_min:50+m.index*6,x_max:50+(m.index+m[0].length)*6,y_min:y,y_max:y+12}]});
    index.text+=text+' ';
    index.objects.paragraph.push({start,end:start+text.length,kind});
    return {start,end:start+text.length};
  };
  add('Alpha beta.',50);add('Second physical line.',70);
  const cells=add('Method Score 32.91 50.3',130,1,'float');
  const table=add('Table 5. Evaluation results.',160,1,'caption');
  add('The logical paragraph resumes.',200);
  const caption=add('Figure 6. Confusion matrices.',270,1,'caption');
  add('Final line before page break.',730);
  const pageEnd=index.text.length;
  add('Continuation on the next page.',50,2);
  index.figures.push({...table,id:'table-5',kind:'table',label:'Table 5',page:1,caption:index.text.slice(table.start,table.end),rect:{x_min:42,x_max:400,y_min:115,y_max:176},confidence:'candidate',references:[]});
  index.figures.push({...caption,id:'figure-6',kind:'figure',label:'Figure 6',page:1,caption:index.text.slice(caption.start,caption.end),rect:{x_min:42,x_max:400,y_min:225,y_max:286},confidence:'candidate',references:[]});
  index.pages=[{number:1,start:0,end:pageEnd,width:612,height:792,provenance:'native'}, {number:2,start:pageEnd,end:index.text.length,width:612,height:792,provenance:'native'}];
  return {index,cells,table,caption};
}

test('physical lines remain distinct even when source paragraphs join them with spaces',()=>{
  const {index}=fixture();const before=structuredClone(index);const doc=cursorDocument(index);
  assert.deepEqual(index,before,'projection must not mutate source');
  assert.equal(cursorDocument(index),doc,'projection cached');
  assert.equal(doc.lines.length,7,'both tables and figures count as one line');
  assert.equal(doc.index.text.split('\n')[0],'Alpha beta.');
  assert.equal(doc.index.text.split('\n')[1],'Second physical line.');
  assert.deepEqual(doc.lines.map(l=>l.number),[1,2,3,4,5,6,7]);
  const space=doc.index.text.indexOf(' ');
  const span=cursorGrapheme(doc,space);
  assert.equal(index.text.slice(span.start,span.end),' ','spaces are cursor positions');
  assert.equal(virtualCursor(doc,originalCursor(doc,space)),space);
});

test('table cells and captions collapse to one atom with caption plus source when copied',()=>{
  const {index,cells,table,caption}=fixture();const doc=cursorDocument(index);
  const at=virtualCursor(doc,cells.start);
  assert.equal(virtualCursor(doc,table.start),at);
  assert.equal(doc.index.text[at],'\uFFFC');
  assert.equal(cursorBlock(doc,at)?.id,'table-5');
  assert.equal(cursorLine(doc,at).end-at,1);
  assert.equal(cursorCopyText(doc,cursorGrapheme(doc,at),'https://example.test/source'),
    'Table 5. Evaluation results.\n\n[Table 5 · p. 1](https://example.test/source#page=1)');
  const figure=virtualCursor(doc,caption.start);
  assert.equal(cursorBlock(doc,figure)?.id,'figure-6');
  const selection=cursorSelection(doc,{start:0,end:figure+1});
  const copy=cursorCopyText(doc,selection,'https://example.test/source');
  assert.match(copy,/Alpha beta/);assert.match(copy,/The logical paragraph resumes/);
  assert.match(copy,/\[Figure 6 · p. 1\]/);assert.doesNotMatch(copy,/32.91/);
});

test('half-screen movement uses geometry and crosses pages without visiting floating text',()=>{
  const doc=cursorDocument(fixture().index);
  const first=doc.lines[0];
  const moved=moveCursorScreen(doc,first.start,1,200);
  assert.equal(cursorLine(doc,moved).number,5);
  const final=doc.lines[5];
  assert.equal(cursorLine(doc,moveCursorScreen(doc,final.start,1,200)).page,2);
  assert.equal(cursorLine(doc,moveCursorScreen(doc,doc.lines[6].start,-1,200)).number,6);
});

test('figure membership overrides prose labels while an inspection projection preserves their text',async()=>{
  const {cursorTextDocument}=await import('../src/lib/cursorDocument.ts');
  const {index,cells,table}=fixture();
  index.figures[0].spans=[{start:cells.start,end:table.end}];
  index.objects.paragraph.find(p=>p.start===cells.start).kind='body';
  const doc=cursorDocument(index);
  assert.equal(cursorBlock(doc,virtualCursor(doc,cells.start)).id,'table-5');
  const inspection=cursorTextDocument(doc,'table-5');
  const at=virtualCursor(inspection,index.text.indexOf('32.91'));
  assert.equal(inspection.index.text.slice(at,at+5),'32.91');
  assert.equal(doc.lines.length,7,'inspection must not change the original navigation lines');
});

test('line gutters stay flush across indents and bullets, with independent page columns',async()=>{
  const {alignLineGutters}=await import('../src/lib/lineGutters.ts');
  const lines=[52,48,62,74,48,330,326,340,352,326].map((x,i)=>({page:1,rect:{x_min:x,x_max:x+200,y_min:i*14,y_max:i*14+10}}));
  const figure={page:1,rect:{x_min:80,x_max:550,y_min:200,y_max:350},block:{}};
  const second={page:2,rect:{x_min:66,x_max:250,y_min:100,y_max:110}};
  alignLineGutters([...lines,figure,second]);
  assert.deepEqual(lines.map(l=>l.gutter),[48,48,48,48,48,326,326,326,326,326]);
  assert.equal(figure.gutter,undefined);
  assert.equal(second.gutter,66);
});
