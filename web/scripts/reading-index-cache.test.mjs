import assert from 'node:assert/strict';
import test from 'node:test';
import { createReadingIndexCache } from '../src/lib/readingIndexCache.ts';

const index = (text='Source text') => ({schema_version:3,text,pages:[],tokens:[],objects:{word:[],WORD:[],sentence:[],paragraph:[]},figures:[],gaps:[]});
const reply = (value=index(),tag='"source-1"') => new Response(JSON.stringify(value),{status:200,headers:{'content-type':'application/json',etag:tag}});
const url = (id='a') => `https://lysilogy.test/api/papers/${id}/reading-index`;
function deferred() { let resolve,reject; const promise=new Promise((yes,no)=>{resolve=yes;reject=no;}); return {promise,resolve,reject}; }

test('one background request survives consumer release and is joined by interactive demand', async()=>{
  const gate=deferred();const calls=[];
  const cache=createReadingIndexCache({fetcher:(target,options)=>{calls.push({target,options});return gate.promise;}});
  const original=cache.load(url()+'?priority=background',{priority:'background',revalidate:true});
  let released=false;
  const departedConsumer=original.then(value=>released?null:value);
  released=true; // Mirrors a hook ignoring completion after unmount; it cannot abort shared work.
  const nextConsumer=cache.load(url(),{priority:'interactive',revalidate:true});
  assert.equal(nextConsumer,original);
  assert.equal(calls.length,1);
  assert.equal(calls[0].target,url(),'priority must not create another HTTP-cache URL');
  assert.equal(calls[0].options.cache,'no-cache');
  assert.equal(calls[0].options.priority,'low');
  assert.equal(calls[0].options.headers.get('X-Reading-Priority'),'background');
  assert.equal(calls[0].options.signal.aborted,false,'consumer release cannot abort the cache-owned request');
  gate.resolve(reply());
  const result=await nextConsumer;
  assert.equal(await departedConsumer,null);
  assert.equal(cache.peek(url()),result);
  assert.equal(await cache.load(url()),result);
  assert.equal(calls.length,1);
});

test('failed requests are removed and failed revalidation is retried by interactive access',async()=>{
  let calls=0;
  const cache=createReadingIndexCache({fetcher:async()=>{
    calls++;
    if(calls===1)throw new Error('offline');
    if(calls===3)return new Response('temporarily unavailable',{status:503});
    return reply(index(calls===2?'Old source':'New source'),`"source-${calls}"`);
  }});
  await assert.rejects(cache.load(url(),{priority:'background'}),/offline/);
  const old=await cache.load(url());
  await assert.rejects(cache.load(url(),{priority:'background',revalidate:true}),/503/);
  assert.equal(cache.peek(url()),old,'old source remains available for already-mounted display');
  const fresh=await cache.load(url());
  assert.equal(fresh.text,'New source','interactive access retries rather than accepting failed validation');
  assert.equal(calls,4);
});

test('ETag validation accepts a bodyless 304 and replaces changed source data',async()=>{
  const calls=[];
  const cache=createReadingIndexCache({fetcher:async(target,options)=>{
    calls.push({target,options});
    if(calls.length===2){
      const response=new Response(null,{status:304,headers:{etag:'"source-1"'}});
      response.json=()=>{throw new Error('304 has no body');};
      return response;
    }
    return reply(index(calls.length===1?'Original':'Changed'),calls.length===1?'"source-1"':'"source-2"');
  }});
  const initial=await cache.load(url());
  assert.equal(await cache.load(url(),{revalidate:true}),initial);
  assert.equal(calls[1].options.headers.get('If-None-Match'),'"source-1"');
  assert.equal(calls[1].options.cache,'no-cache');
  const changed=await cache.load(url(),{revalidate:true});
  assert.equal(changed.text,'Changed');
  assert.notEqual(changed,initial);
  assert.equal(cache.peek(url()),changed);
});

test('an uncached 304 recovers once with a full reload and cannot loop',async()=>{
  const calls=[];
  const cache=createReadingIndexCache({fetcher:async(target,options)=>{
    calls.push({target,options});
    return calls.length===1?new Response(null,{status:304}):reply(index('Recovered'));
  }});
  assert.equal((await cache.load(url())).text,'Recovered');
  assert.deepEqual(calls.map(call=>call.options.cache),['no-cache','reload']);
  assert.equal(calls[1].options.headers.has('If-None-Match'),false);
  const broken=createReadingIndexCache({fetcher:async()=>new Response(null,{status:304})});
  await assert.rejects(broken.load(url()),/without its cached body/);
});

test('a fresh module cache revalidates the canonical URL so the browser can reuse its persistent body',async()=>{
  const calls=[];
  const browserFetch=async(target,options)=>{
    calls.push({target,options});
    // Fetch exposes the merged cached body as 200 after browser-managed 304 validation.
    return reply(index('Persisted HTTP response'));
  };
  const beforeReload=createReadingIndexCache({fetcher:browserFetch});
  await beforeReload.load(url());
  const afterReload=createReadingIndexCache({fetcher:browserFetch});
  assert.equal((await afterReload.load(url(),{priority:'background',revalidate:true})).text,'Persisted HTTP response');
  assert.equal(calls[0].target,calls[1].target);
  assert.equal(calls[1].options.cache,'no-cache');
  assert.equal(calls[1].options.headers.has('If-None-Match'),false,'browser owns the validator when parsed memory is empty');
});

test('parsed cache uses LRU recency and both entry and estimated byte limits',async()=>{
  const calls=[];
  const cache=createReadingIndexCache({maxEntries:2,fetcher:async(target)=>{calls.push(target);return reply(index(target));}});
  await cache.load(url('a'));await cache.load(url('b'));
  const a=cache.peek(url('a'));
  await cache.load(url('c'));
  assert.equal(cache.peek(url('b')),null,'least-recently-read paper is evicted');
  assert.equal(await cache.load(url('a')),a);
  assert.equal(calls.length,3);
  await cache.load(url('b'));
  assert.equal(calls.length,4);
  const bytes=createReadingIndexCache({maxBytes:600,fetcher:async()=>reply(index(''))});
  await bytes.load(url('a'));await bytes.load(url('b'));await bytes.load(url('c'));
  assert.equal(bytes.peek(url('a')),null,'three 256-byte minimum entries exceed the byte limit');
  assert.ok(bytes.peek(url('b')));
  const oversized=createReadingIndexCache({maxBytes:600,fetcher:async()=>reply(index('x'.repeat(1000)))});
  assert.equal((await oversized.load(url())).text.length,1000);
  assert.equal(oversized.peek(url()),null,'oversized source is returned without entering the parsed LRU');
});

test('canonical source URLs and validation requests share a pending result instead of returning stale memory',async()=>{
  const gate=deferred();let calls=0;
  const cache=createReadingIndexCache({fetcher:async()=>++calls===1?reply(index('Old')):gate.promise});
  const old=await cache.load(url().replace('/reading-index','/source'));
  const validation=cache.load(url(),{priority:'background',revalidate:true});
  const interactive=cache.load(url());
  assert.equal(interactive,validation);
  assert.equal(cache.peek(url()),old);
  gate.resolve(reply(index('New'),'"new"'));
  assert.equal((await interactive).text,'New');
  assert.equal(calls,2);
});

test('invalid or unsupported source responses never enter the cache and can be retried',async()=>{
  let calls=0;
  const cache=createReadingIndexCache({fetcher:async()=>reply(calls++===0?{...index(),schema_version:999}:index())});
  await assert.rejects(cache.load(url()),/incompatible/);
  assert.equal(cache.peek(url()),null);
  assert.equal((await cache.load(url())).schema_version,3);
});


test('shared network timeout frees a hung request and ignores late stale completion',async()=>{
  const gate=deferred();const calls=[];
  const cache=createReadingIndexCache({timeoutMs:15,fetcher:(target,options)=>{
    calls.push({target,options});
    return calls.length===1?gate.promise:Promise.resolve(reply(index('Fresh source')));
  }});
  await assert.rejects(cache.load(url()),/timed out/);
  assert.equal(calls[0].options.signal.aborted,true);
  const fresh=await cache.load(url());
  gate.resolve(reply(index('Late stale source')));
  await new Promise(resolve=>setTimeout(resolve,0));
  assert.equal(cache.peek(url()),fresh,'late aborted work cannot overwrite the newer source');
  assert.equal(fresh.text,'Fresh source');
});
