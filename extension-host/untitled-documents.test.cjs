'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const { Uri } = require('./api-types.cjs');
function deferred() { let resolve, reject; const promise = new Promise((a,b) => { resolve=a; reject=b; }); return {promise,resolve,reject}; }
function fixture(handle, options = {}) {
  let runtime, generation = 0, next = 0;
  const records = new Map(), calls = [], terminal = [];
  function sync() { runtime.sync({generation:++generation,documents:[...records.values()],active:null,selections:[]}); }
  function publish(id, values) {
    records.set(id,{id,uri:`untitled:Untitled-${id}`,version:1,text:'',languageId:'plaintext',isDirty:true,isReadonly:false,...values});sync();
  }
  runtime = createApi(async (method, params, lifecycle) => {
    calls.push({method,params});terminal.push(lifecycle);
    if (handle) return handle({method,params,lifecycle,publish,records,sync});
    const args=params.args,id=++next;
    if (method==='nativeUntitledDocumentOpen') publish(id,{uri:args.uri || `untitled:Untitled-${id}`,text:args.content,languageId:args.language || (args.uri?.endsWith('.cpp')?'cpp':'plaintext')});
    else if (method==='nativeVirtualDocumentOpen') publish(id,{uri:args.uri,text:args.content,isDirty:false,isReadonly:true});
    else publish(id,{uri:Uri.file(args.path).toString(),text:'猫🙂\r\nfile',isDirty:false});
    return {document:id};
  },()=>{}, {session:7,untitledDocuments:true,...options});
  runtime.configure('/tmp/workspace',[],[{},{}]);sync();
  return {runtime,api:runtime.forExtension('test.owner'),records,calls,publish,sync,terminal};
}
test('noarg, undefined, empty options and null create distinct writable dirty hidden documents',async()=>{
  const f=fixture(),seen=[];f.api.workspace.onDidOpenTextDocument(doc=>{seen.push(doc);assert.equal(f.api.window.activeTextEditor,undefined);});
  const docs=[await f.api.workspace.openTextDocument(),await f.api.workspace.openTextDocument(undefined),await f.api.workspace.openTextDocument({}),await f.api.workspace.openTextDocument(null)];
  assert.equal(new Set(docs).size,4);assert.deepEqual(seen,docs);assert.equal(f.api.workspace.textDocuments.length,4);
  for(const [i,doc] of docs.entries()) {assert.equal(doc.isUntitled,true);assert.equal(doc.isDirty,true);assert.equal(doc.getText(),'');assert.equal(doc.isClosed,false);assert.equal(doc.languageId,'plaintext');assert.deepEqual(f.calls[i].params.args,{uri:null,content:'',language:null});}
  assert.deepEqual(f.api.window.visibleTextEditors,[]);
});
test('content/language getters are read once, exact Unicode CRLF mirrors precede await',async()=>{
  const f=fixture();let content=0,language=0,encoding=0;
  const doc=await f.api.workspace.openTextDocument({get content(){content++;return '猫🙂\r\nsecond\r\n';},get language(){language++;return 'cpp';},get encoding(){encoding++;return 'utf8';}});
  assert.deepEqual([content,language,encoding],[1,1,1]);assert.equal(doc.getText(),'猫🙂\r\nsecond\r\n');assert.equal(doc.languageId,'cpp');
  assert.deepEqual(f.calls[0].params.args,{uri:null,content:'猫🙂\r\nsecond\r\n',language:'cpp'});
});
test('explicit untitled URI infers language and cached reopen returns edited original private mirror',async()=>{
  const f=fixture(),uri=Uri.file('/tmp/new.cpp').with({scheme:'untitled'}),doc=await f.api.workspace.openTextDocument(uri);
  assert.deepEqual(f.calls[0].params.args,{uri:uri.toString(),content:'',language:null});assert.equal(doc.languageId,'cpp');
  const id=f.calls.length;f.publish(1,{uri:uri.toString(),version:2,text:'edited猫🙂\r\n',languageId:'cpp'});
  // Extension-visible metadata is not the authority for cached URI identity.
  doc._snapshot={id:999,uri:'untitled:forged',version:99,text:'forged',isClosed:true};
  assert.equal(await f.api.workspace.openTextDocument(uri),doc);assert.equal(f.calls.length,id);
});
test('URI fields read once and public formatters cannot redirect native opening',async()=>{
  const f=fixture(),values={scheme:'untitled',authority:'',path:'/new.cpp',query:'',fragment:''},reads={};
  const uri={toString(){throw new Error('untrusted formatter');},get fsPath(){throw new Error('untrusted path');}};
  for(const [key,value] of Object.entries(values))Object.defineProperty(uri,key,{get(){reads[key]=(reads[key]||0)+1;return reads[key]===1?value:'different';}});
  await f.api.workspace.openTextDocument(uri);assert.deepEqual(reads,{scheme:1,authority:1,path:1,query:1,fragment:1});assert.equal(f.calls[0].params.args.uri,'untitled:/new.cpp');
});
test('file paths and file URIs accept UTF8 options, other encodings refuse before requests',async()=>{
  const f=fixture();let reads=0;
  const a=await f.api.workspace.openTextDocument('file.txt',{get encoding(){reads++;return 'utf8';}});
  const b=await f.api.workspace.openTextDocument(Uri.file('/tmp/other.txt'),{encoding:'utf8'});
  assert.equal(reads,1);assert.equal(a.isUntitled,false);assert.equal(b.isUntitled,false);assert.equal(f.calls[0].params.args.path,'/tmp/workspace/file.txt');
  for(const encoding of ['utf16le','utf8bom','unknown','utf-8',null,3])await assert.rejects(f.api.workspace.openTextDocument('/tmp/file',{encoding}),/encoding/);
  assert.equal(f.calls.length,2);
});
test('second options takes precedence for fresh creation and explicit URI never consumes content/language',async()=>{
  const f=fixture();const first={get content(){throw new Error('ignored');},get encoding(){throw new Error('ignored');}};
  await f.api.workspace.openTextDocument(first,{content:'second',language:'cpp'});assert.equal(f.calls[0].params.args.content,'second');
  await f.api.workspace.openTextDocument(Uri.parse('untitled:/uri.cpp'),{get content(){throw new Error('ignored URI content');},get language(){throw new Error('ignored URI language');},encoding:'utf8'});
  assert.deepEqual(f.calls[1].params.args,{uri:'untitled:/uri.cpp',content:'',language:null});
});
test('invalid inputs and raw/escaped payload bounds never start native work',async()=>{
  const f=fixture();
  for(const input of [42,true,[],{content:null},{content:{}},{language:42},{language:'猫'.repeat(50)},{content:'x'.repeat(1024*1024+1)},{content:'猫'.repeat(400000)},{content:'\0'.repeat(400000)}])await assert.rejects(f.api.workspace.openTextDocument(input));
  await assert.rejects(f.api.workspace.openTextDocument(Uri.parse('untitled:/'+'x'.repeat(4096))),/URI/);
  await assert.rejects(f.api.workspace.openTextDocument('/tmp/\0file'),/path/);assert.equal(f.calls.length,0);
});
test('options getter disposal cannot reacquire original caller or admit native work',async()=>{
  const f=fixture();await assert.rejects(f.api.workspace.openTextDocument({get content(){f.runtime.disposeOwner('test.owner');f.runtime.forExtension('test.owner');return 'late';}}),/retired/);assert.equal(f.calls.length,0);
});
test('caller disposal and reacquisition after actual reply cannot authorize the old promise',async()=>{
  const hold=deferred(),f=fixture(async({publish})=>{await hold.promise;publish(1,{text:'original'});return {document:1};});
  const opening=f.api.workspace.openTextDocument({content:'original'});f.runtime.disposeOwner('test.owner');f.runtime.forExtension('test.owner');hold.resolve();await assert.rejects(opening,/retired/);assert.equal(f.calls.length,1);
});
test('closed/incorrect/missing/readonly or uninitialized mirrors reject typed acknowledgements',async()=>{
  for(const mode of ['missing','invalid-id','wrong-uri','readonly','clean','content','language','existing','closed']){
    const f=fixture(({publish,params,records,sync})=>{
      if(mode==='invalid-id')return {document:0};if(mode==='missing')return {document:1};
      publish(1,{uri:mode==='wrong-uri'?'file:///tmp/wrong':'untitled:Untitled-1',text:mode==='content'?'wrong':params.args.content,isReadonly:mode==='readonly',isDirty:mode!=='clean',languageId:mode==='language'?'wrong':'cpp'});if(mode==='closed'){records.clear();sync();}return {document:1};
    });
    if(mode==='existing')f.publish(1,{text:'requested',languageId:'cpp'});
    await assert.rejects(f.api.workspace.openTextDocument({content:'requested',language:'cpp'}),/acknowledgement|mirror|published/);
  }
});
test('didOpen edit before acknowledgement retains the legitimately advanced mirror',async()=>{
  const f=fixture();f.api.workspace.onDidOpenTextDocument(()=>f.publish(1,{version:2,text:'edited',languageId:'cpp'}));
  const doc=await f.api.workspace.openTextDocument({content:'first',language:'cpp'});assert.equal(doc.version,2);assert.equal(doc.getText(),'edited');
});
test('timed-out actual untitled requests retain all eight service leases until terminal receipts',async()=>{
  let timeout=true;const f=fixture(({lifecycle,publish})=>{if(timeout){lifecycle.retained=true;throw new Error('timeout');}publish(1);return {document:1};});
  for(let i=0;i<8;i++)await assert.rejects(f.api.workspace.openTextDocument(),/timeout/);
  await assert.rejects(f.api.workspace.openTextDocument(),/queue limit/);assert.equal(f.calls.length,8);
  timeout=false;f.terminal[0].onTerminal();assert.equal((await f.api.workspace.openTextDocument()).isUntitled,true);assert.equal(f.calls.length,9);
  for(const lifecycle of f.terminal)lifecycle.onTerminal();
});
test('untitled capability gate preserves ordinary file opening and virtual provider routing',async()=>{
  const f=fixture(null,{untitledDocuments:false,virtualDocuments:true});await assert.rejects(f.api.workspace.openTextDocument({content:'no'}),/untitled/);await assert.rejects(f.api.workspace.openTextDocument(Uri.parse('untitled:/no')),/untitled/);
  await f.api.workspace.openTextDocument('/tmp/file');let calls=0;f.api.workspace.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent(){calls++;return 'virtual';}});
  const virtual=await f.api.workspace.openTextDocument(Uri.parse('preview-test:/sample.cpp'));assert.equal(virtual.getText(),'virtual');assert.equal(calls,1);assert.equal(f.calls.at(-1).method,'nativeVirtualDocumentOpen');
});

test('mixed line endings validate against trusted native EOL, not extension-visible metadata',async()=>{
  for(const [content,eol,text] of [['a\r\nb\rc\nd',2,'a\r\nb\r\nc\r\nd'],['a\r\nb\nc\nd',1,'a\nb\nc\nd']]){
    const f=fixture(({publish})=>{publish(1,{text,eol});return {document:1};});
    const doc=await f.api.workspace.openTextDocument({content});assert.equal(doc.getText(),text);assert.equal(doc.eol,eol);
  }
});
test('Save As closes old URI identity, publishes all new mirrors before close/open/save, and resolves old authorized save',async()=>{
  const hold=deferred(),oldURI='untitled:/new.cpp',newURI=Uri.file('/tmp/saved.cpp').toString();
  const f=fixture(()=>hold.promise);f.publish(7,{uri:oldURI,text:'猫🙂\r\noriginal',languageId:'cpp',savedGeneration:0});
  const old=f.api.workspace.textDocuments[0],events=[];
  const saved=f.runtime.runWithOwner('test.owner',()=>old.save());
  for(const [kind,event] of [['close',f.api.workspace.onDidCloseTextDocument],['open',f.api.workspace.onDidOpenTextDocument],['save',f.api.workspace.onDidSaveTextDocument]])event(doc=>{
    events.push([kind,doc]);const live=f.api.workspace.textDocuments[0];assert.notEqual(live,old);assert.equal(live.uri.toString(),newURI);
    if(kind==='close'){assert.equal(doc,old);assert.equal(old.isClosed,true);assert.equal(old.isDirty,false);assert.equal(old.uri.toString(),oldURI);assert.equal(old.getText(),'猫🙂\r\noriginal');}
    else{assert.equal(doc,live);assert.equal(doc.isClosed,false);assert.equal(doc.getText(),'猫🙂\r\nnewer');}
  });
  // Public retargeting cannot alter the resource transition or receipt target.
  old.uri.path='/forged-public-uri.cpp';
  old._snapshot={id:99,uri:newURI,version:88,text:'forged',isDirty:false,savedGeneration:999};
  f.publish(7,{uri:newURI,text:'猫🙂\r\nnewer',version:2,languageId:'cpp',isDirty:true,savedGeneration:1});hold.resolve(true);
  assert.equal(await saved,true);const current=f.api.workspace.textDocuments[0];
  assert.deepEqual(events.map(([kind])=>kind),['close','open','save']);assert.notEqual(current,old);assert.equal(old.version,1);assert.equal(current.isDirty,true);
  const calls=f.calls.length;await assert.rejects(f.runtime.runWithOwner('test.owner',()=>old.save()),/closed/);assert.equal(f.calls.length,calls);
});
test('resource replacement retires old scoped editor edit and leaves ordinary same URI identity unchanged',async()=>{
  let runtime;const frames=[];
  runtime=createApi((method,params)=>{frames.push({method,params});return Promise.resolve({applied:true});},()=>{}, {session:1});
  const first={id:1,uri:'untitled:/first.cpp',text:'猫🙂\r\n',version:1,languageId:'cpp',isDirty:true,savedGeneration:0};
  const publish=(generation,snapshot)=>runtime.sync({generation,documents:[snapshot],active:1,selections:[{anchor:{line:0,character:1},active:{line:0,character:1}}]});
  publish(1,first);const api=runtime.forExtension('test.owner'),editor=api.window.activeTextEditor,doc=editor.document;
  publish(2,{...first,version:2,text:'new猫🙂\r\n'});assert.equal(api.window.activeTextEditor,editor);assert.equal(api.window.activeTextEditor.document,doc);
  publish(3,{...first,uri:Uri.file('/tmp/first.cpp').toString(),version:3,text:'new猫🙂\r\n',isDirty:false,savedGeneration:1});
  const current=api.window.activeTextEditor;assert.notEqual(current,editor);assert.notEqual(current.document,doc);assert.equal(doc.uri.toString(),first.uri);assert(doc.isClosed);
  let called=false;await assert.rejects(editor.edit(()=>{called=true;}),/no longer open/);assert.equal(called,false);assert.equal(frames.length,0);
});
test('all URI changes retire old public document even without save, keeping closed text/URI intact',()=>{
  const f=fixture();f.publish(1,{uri:Uri.file('/tmp/a.cpp').toString(),text:'猫🙂\r\n',languageId:'cpp'});const old=f.api.workspace.textDocuments[0],events=[];
  f.api.workspace.onDidCloseTextDocument(doc=>events.push(['close',doc]));f.api.workspace.onDidOpenTextDocument(doc=>events.push(['open',doc]));f.api.workspace.onDidSaveTextDocument(()=>assert.fail('No save publication occurred'));
  f.publish(1,{uri:Uri.file('/tmp/b.cpp').toString(),text:'other\r\n',languageId:'cpp'});const current=f.api.workspace.textDocuments[0];
  assert.deepEqual(events,[['close',old],['open',current]]);assert(old.isClosed);assert.equal(old.isDirty,false);assert.equal(old.uri.toString(),Uri.file('/tmp/a.cpp').toString());assert.equal(old.getText(),'猫🙂\r\n');
});
test('unchanged URI spoofing cannot turn private saved-generation failure into true',async()=>{
  const f=fixture(()=>true);f.publish(1,{savedGeneration:0});const doc=f.api.workspace.textDocuments[0];
  doc._snapshot={id:999,version:99,uri:'file:///forged',savedGeneration:999};
  await assert.rejects(f.runtime.runWithOwner('test.owner',()=>doc.save()),/mirror.*before/);assert.deepEqual(f.calls[0].params.args,{document:1,version:1});
});
test('replacement without native save-generation advance cannot authorize held Save As acknowledgement',async()=>{
  const held=deferred(),f=fixture(()=>held.promise);f.publish(1,{uri:'untitled:/old.cpp',savedGeneration:4});const old=f.api.workspace.textDocuments[0];
  const saving=f.runtime.runWithOwner('test.owner',()=>old.save());f.publish(1,{uri:Uri.file('/tmp/new.cpp').toString(),savedGeneration:4});held.resolve(true);
  await assert.rejects(saving,/mirror.*before/);assert(old.isClosed);assert.notEqual(f.api.workspace.textDocuments[0],old);
});

test('mixed-EOL changes preserve canonical UTF16 replacement length and text without invoking public getText',()=>{
  const f=fixture(),raw='猫🙂\rA\n🙂\r\nlast',canonical='猫🙂\r\nA\r\n🙂\r\nlast';
  f.publish(1,{uri:Uri.file('/tmp/mixed.cpp').toString(),text:raw,eol:2,languageId:'cpp'});
  const doc=f.api.workspace.textDocuments[0];assert.equal(doc.getText(),canonical);
  let reads=0;doc.getText=()=>{reads++;throw new Error('Extension override must not prepare native change events');};
  const events=[];f.api.workspace.onDidChangeTextDocument(event=>events.push(event));
  f.publish(1,{uri:Uri.file('/tmp/mixed.cpp').toString(),text:'x\n猫🙂\r\ny\rz',eol:2,version:2,languageId:'cpp'});
  assert.equal(reads,0);assert.equal(events.length,1);assert.equal(events[0].document,doc);
  const change=events[0].contentChanges[0];assert.equal(change.rangeOffset,0);assert.equal(change.rangeLength,canonical.length);
  assert.deepEqual([change.range.start.line,change.range.start.character,change.range.end.line,change.range.end.character],[0,0,3,4]);
  assert.equal(change.text,'x\r\n猫🙂\r\ny\r\nz');
  // A new transmitted version with equal source bytes remains observable;
  // the retained canonical value remains private and does not call user code.
  f.publish(1,{uri:Uri.file('/tmp/mixed.cpp').toString(),text:'x\n猫🙂\r\ny\rz',eol:2,version:3,languageId:'cpp'});
  assert.equal(reads,0);assert.equal(events.length,2);assert.equal(events[1].contentChanges[0].rangeLength,'x\r\n猫🙂\r\ny\r\nz'.length);
  assert.equal(events[1].contentChanges[0].text,'x\r\n猫🙂\r\ny\r\nz');
});
