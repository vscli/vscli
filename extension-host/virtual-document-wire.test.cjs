'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const { createNativeRequests } = require('./native-requests.cjs');
const { createDocumentServices } = require('./document-services.cjs');
function clock() {
  let id = 0;
  const entries = new Map();
  return { entries, setTimeout(fn,ms) { entries.set(++id,{fn,ms}); return id; },
    clearTimeout(id) { entries.delete(id); }, fire(id) { const entry=entries.get(id); entries.delete(id); entry.fn(); } };
}
for (const method of ['nativeDocumentLoad','nativeVirtualDocumentOpen','nativeVirtualDocumentChange','nativeUntitledDocumentOpen']) {
  test(`${method} retains service capacity after timeout and only exact typed receipt releases it`,async()=>{
    const frames=[], timers=clock();
    const queue=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:7,owner:'origin'}),timers);
    const services=createDocumentServices(queue.request,{session:2},{generation:()=>1});
    const pending=[];
    for(let i=0;i<8;i++)pending.push(services.call('caller',method,{uri:'test:/a',content:'text'}).catch(error=>error.message));
    assert.equal(frames[0].params.commandOwner,'origin');
    assert.equal(frames[0].params.owner,'caller');
    timers.fire(1); assert.match(await pending[0],/outcome is unknown/);
    await assert.rejects(services.call('caller',method,{}),/queue limit/);
    for(const result of [true,{document:0},{document:1.5},{document:1,unexpected:true}]) {
      assert.equal(queue.settle({id:frames[0].id,result}),false);
    }
    assert.equal(queue.settle({id:frames[0].id,result:{document:1},error:{message:'both'}}),false);
    await assert.rejects(services.call('caller',method,{}),/queue limit/);
    assert.equal(queue.settle({id:frames[0].id,result:{document:1}}),true);
    assert.equal(queue.settle({id:frames[0].id,result:{document:1}}),false);
    const next=services.call('caller',method,{}).catch(error=>error.message);
    assert.equal(frames.length,9);
    queue.shutdown(); await Promise.all([...pending,next]);
  });
}
