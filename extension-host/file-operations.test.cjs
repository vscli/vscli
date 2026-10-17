'use strict';
const test=require('node:test'), assert=require('node:assert/strict');
const {createFileOperations}=require('./file-operations.cjs');
const {createApi}=require('./api.cjs');
const {Uri}=require('./api-types.cjs');
const frame=(sequence,kind,files)=>({session:7,sequence,kind,files});
test('native create/rename/delete preserve source event shape, URI revival and outer-only freezing',()=>{
 const operations=createFileOperations(7), events=[];
 for(const name of ['onDidCreateFiles','onDidRenameFiles','onDidDeleteFiles'])operations.api[name](event=>events.push(event));
 operations.publish(frame(1,'create',[{target:'file:///tmp/a%20%F0%9F%99%82.cpp'}]));
 operations.publish(frame(2,'rename',[{source:'file:///tmp/a',target:'file:///tmp/b'}]));
 operations.publish(frame(3,'delete',[{target:'file:///tmp/b'}]));
 assert.equal(events.length,3);assert.ok(events[0].files[0] instanceof Uri);assert.equal(events[0].files[0].path,'/tmp/a 🙂.cpp');
 assert.equal(events[1].files[0].oldUri.path,'/tmp/a');assert.equal(events[1].files[0].newUri.path,'/tmp/b');
 for(const event of events){assert.ok(Object.isFrozen(event));assert.equal(Object.isFrozen(event.files),false);}
 assert.equal(Object.isFrozen(events[1].files[0]),false);
});
test('malformed, foreign, replayed and oversized frames cannot partially dispatch or consume sequence',()=>{
 const operations=createFileOperations(7), events=[];operations.api.onDidCreateFiles(event=>events.push(event));
 for(const invalid of [
  {...frame(1,'create',[]),session:8},frame(0,'create',[]),frame(1,'other',[]),
  {...frame(1,'create',[]),extra:true},frame(1,'create',[{target:'file:///tmp/a'},{target:1}]),
  frame(1,'create',[{source:'file:///tmp/a',target:'file:///tmp/b'}]),frame(1,'rename',[{target:'file:///tmp/b'}]),
  frame(1,'create',Array.from({length:129},()=>({target:'file:///tmp/a'}))),
  frame(1,'create',[{target:'x'.repeat(16385)}])
 ])assert.throws(()=>operations.publish(invalid));
 assert.equal(events.length,0);operations.publish(frame(1,'create',[{target:'file:///tmp/a'}]));
 assert.equal(events.length,1);assert.throws(()=>operations.publish(frame(1,'create',[])));assert.equal(events.length,1);
});
test('listeners share the same event and follow native emitter disposal during delivery',()=>{
 const operations=createFileOperations(7), calls=[];let later;
 operations.api.onDidCreateFiles(event=>{calls.push('first');event.files.push(Uri.parse('file:///extra'));later.dispose();});
 later=operations.api.onDidCreateFiles(()=>calls.push('disposed'));
 operations.api.onDidCreateFiles(event=>calls.push(event.files.length));
 operations.publish(frame(1,'create',[{target:'file:///tmp/a'}]));assert.deepEqual(calls,['first',2]);
});
test('operation API is reachable through scoped facades and disposed listeners cannot revive',()=>{
 const runtime=createApi(async()=>null,()=>{},{session:7});const api=runtime.forExtension('fixture.owner');
 const events=[];api.workspace.onDidRenameFiles(event=>events.push(event));
 runtime.fileOperation(frame(1,'rename',[{source:'file:///tmp/a',target:'file:///tmp/b'}]));
 assert.equal(events.length,1);runtime.disposeOwner('fixture.owner');
 runtime.fileOperation(frame(2,'rename',[{source:'file:///tmp/b',target:'file:///tmp/c'}]));assert.equal(events.length,1);
});
test('operation delivery does not synthesize file watcher notifications',()=>{
 const runtime=createApi(async()=>null,()=>{},{session:7});let operations=0,watchers=0;
 runtime.api.workspace.onDidCreateFiles(()=>operations++);
 const watcher=runtime.forExtension('fixture.owner').workspace.createFileSystemWatcher('**/*');watcher.onDidCreate(()=>watchers++);
 runtime.fileOperation(frame(1,'create',[{target:'file:///tmp/a'}]));assert.equal(operations,1);assert.equal(watchers,0);watcher.dispose();
});
