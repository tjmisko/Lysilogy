import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
import { ApiError, request } from '../src/lib/api.ts';

// Transpile the browser module without changing its production import paths.
const notesSource=await readFile(new URL('../src/lib/notesApi.ts',import.meta.url),'utf8');
const notesJavascript=ts.transpileModule(notesSource,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText
  .replace('from "./api"',`from ${JSON.stringify(new URL('../src/lib/api.ts',import.meta.url).href)}`);
const {notesApi,localCreatedAt}=await import(`data:text/javascript;base64,${Buffer.from(notesJavascript).toString('base64')}`);

test('JSON requests retain useful HTTP errors and accept valid responses',async t=>{
  t.mock.method(globalThis,'fetch',async()=>new Response(JSON.stringify({value:3}),{headers:{'content-type':'application/json'}}));
  assert.deepEqual(await request('/api/example'),{value:3});
  t.mock.method(globalThis,'fetch',async()=>new Response(JSON.stringify({message:'The file changed.'}),{status:409,headers:{'content-type':'application/json'}}));
  await assert.rejects(request('/api/example'),error=>error instanceof ApiError&&error.status===409&&error.kind==='http'&&error.message==='The file changed.');
});

test('SPA HTML and malformed responses produce actionable errors without leaking response bodies',async t=>{
  for(const [body,contentType] of [['<!doctype html><html>PRIVATE APP SHELL</html>','text/html'],['<html>PRIVATE APP SHELL</html>','application/json'],['{"broken":','application/json'],['','application/json']]){
    t.mock.method(globalThis,'fetch',async()=>new Response(body,{headers:{'content-type':contentType}}));
    await assert.rejects(request('/api/papers/example/notes/open'),error=>{
      assert.ok(error instanceof ApiError);assert.equal(error.kind,'invalid_response');assert.equal(error.status,200);
      assert.match(error.message,/Restart the Lysilogy backend/);assert.match(error.message,/\/api proxy/);
      assert.doesNotMatch(error.message,/PRIVATE APP SHELL|Unexpected token|JSON\.parse|<!doctype/i);return true;
    });
  }
});

test('unknown JSON error shapes do not become object-string messages',async t=>{
  t.mock.method(globalThis,'fetch',async()=>new Response(JSON.stringify({message:{detail:'private'}}),{status:502,headers:{'content-type':'application/json'}}));
  await assert.rejects(request('/api/example'),error=>error instanceof ApiError&&error.message==='API request failed (HTTP 502).');
});

test('notes reject old or malformed JSON contracts, but preserve existing empty files',async t=>{
  for(const value of [null,[],{}, {filename:'Paper.md',text:'old'}, {filename:'Paper.md',text:7,revision:'abc'}, {filename:'',text:'',revision:null}]){
    t.mock.method(globalThis,'fetch',async()=>new Response(JSON.stringify(value),{headers:{'content-type':'application/json'}}));
    await assert.rejects(notesApi.open('paper'),error=>error instanceof ApiError&&error.kind==='invalid_response'&&/notes response is incompatible/.test(error.message));
  }
  const empty={filename:'Paper.md',text:'',revision:'existing-empty-revision'};
  t.mock.method(globalThis,'fetch',async()=>new Response(JSON.stringify(empty),{headers:{'content-type':'application/json'}}));
  assert.deepEqual(await notesApi.open('paper'),empty);
});

test('opening notes sends a local-offset timestamp and POST, while comparison only reads',async t=>{
  const calls=[];const note={filename:'Paper.md',text:'template',revision:'created'};
  t.mock.method(globalThis,'fetch',async(path,options)=>{calls.push({path,options});return new Response(JSON.stringify(note),{headers:{'content-type':'application/json'}});});
  const controller=new AbortController();
  assert.deepEqual(await notesApi.open('paper/id',controller.signal),note);
  assert.equal(calls[0].path,'/api/papers/paper%2Fid/notes/open');assert.equal(calls[0].options.method,'POST');
  assert.equal(calls[0].options.signal,controller.signal);
  assert.match(JSON.parse(calls[0].options.body).created_at,/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$/);
  await notesApi.read('paper');assert.equal(calls[1].path,'/api/papers/paper/notes');assert.equal(calls[1].options.method,undefined);
});

test('local creation timestamps keep calendar day and non-hour timezone offsets',()=>{
  const date={getFullYear:()=>2026,getMonth:()=>8,getDate:()=>11,getHours:()=>15,getMinutes:()=>49,getSeconds:()=>7,getTimezoneOffset:()=>420};
  assert.equal(localCreatedAt(date),'2026-09-11T15:49:07-07:00');
  assert.equal(localCreatedAt({...date,getTimezoneOffset:()=>-345}),'2026-09-11T15:49:07+05:45');
  assert.equal(localCreatedAt({...date,getTimezoneOffset:()=>0}),'2026-09-11T15:49:07+00:00');
});
