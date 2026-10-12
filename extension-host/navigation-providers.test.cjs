'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {createProviders}=require('./providers.cjs');
const {Uri,Range,Position,TextDocument}=require('./api-types.cjs');
const {Location,CompletionItem}=require('./provider-types.cjs');
const {COMMANDS,normalizeNavigation,sortAndDedup}=require('./navigation-providers.cjs');
function deferred(){let resolve;const promise=new Promise(done=>resolve=done);return{promise,resolve};}
const turn=()=>new Promise(done=>setImmediate(done));
function apiFixture(){
  const frames=[],notifications=[],runtime=createApi((method,params)=>{frames.push({method,params});return Promise.reject(Error('unexpected native request'));},(method,params)=>notifications.push({method,params}),{session:7,languageProviders:true});
  runtime.configure('/work',{},[]);
  const a={id:1,uri:Uri.file('/work/a.cpp').toString(),version:3,text:'猫🙂x\r\nsecond',eol:2,languageId:'cpp',languageEpoch:1,isDirty:true};
  const b={id:2,uri:Uri.file('/work/b.cpp').toString(),version:1,text:'untouched\r\n',eol:2,languageId:'cpp',languageEpoch:0,isDirty:false};
  let generation=0;const sync=(first=a)=>runtime.sync({generation:++generation,documents:[first,b],editors:[],activeEditor:null});sync();
  return{runtime,api:runtime.forExtension('consumer'),creator:runtime.forExtension('creator'),a,b,sync,frames,notifications};
}
function providerFixture(timeoutMs=100){
  const document=new TextDocument({id:1,uri:'file:///work/a.cpp',version:3,languageId:'cpp',text:'猫🙂x\r\nsecond'}),notifications=[];
  const module=createProviders({session:7,timeoutMs,document:id=>id===1?document:undefined,notify:(method,params)=>notifications.push({method,params}),track:(_,value)=>value});
  const request=(provider,request)=>({session:7,owner:'creator',provider,request,document:1,version:3,position:{line:0,character:3}});
  return{document,module,notifications,request};
}
test('all five actual public commands aggregate matching providers then sort/dedup full ranges without focus or edits',async()=>{
  for(const [command,type]of Object.entries(COMMANDS)){
    const f=apiFixture(),seen=[],registration={definition:'registerDefinitionProvider',references:'registerReferenceProvider',implementation:'registerImplementationProvider',typeDefinition:'registerTypeDefinitionProvider',declaration:'registerDeclarationProvider'}[type];
    const callback={definition:'provideDefinition',references:'provideReferences',implementation:'provideImplementation',typeDefinition:'provideTypeDefinition',declaration:'provideDeclaration'}[type];
    const early={marker:'early',[callback](doc,at,context){assert.equal(this,early);assert.equal(doc,f.api.workspace.textDocuments[0]);assert.deepEqual(at,new Position(0,3));if(type==='references')assert.deepEqual(context,{includeDeclaration:true});seen.push(this.marker);return [new Location(Uri.file('/work/z.cpp'),new Range(1,0,1,2)),new Location(Uri.file('/work/a.cpp'),new Range(0,0,0,1))];}};
    const latest={marker:'latest',[callback](){assert.equal(this,latest);seen.push(this.marker);return [new Location(Uri.file('/work/a.cpp'),new Range(0,0,0,1)),new Location(Uri.file('/work/m.cpp'),new Range(0,0,1,0))];}};
    f.creator.languages[registration]('cpp',early);f.creator.languages[registration]({language:'cpp',scheme:'file'},latest);
    f.creator.languages[registration]('*',{[callback](){seen.push('wildcard');return null;}});
    f.creator.languages[registration]('cpp',{[callback](){seen.push('error');throw Error('provider failure');}});
    f.creator.languages[registration]('rust',{[callback](){throw Error('wrong selector');}});
    const before=f.api.workspace.textDocuments.map(doc=>doc.getText());
    const result=await f.api.commands.executeCommand(command,Uri.file('/work/a.cpp'),new Position(0,3));
    assert.deepEqual(seen,['error','latest','early','wildcard']);
    assert.deepEqual(result.map(value=>value.uri.path),['/work/a.cpp','/work/m.cpp','/work/z.cpp']);assert(result.every(value=>value instanceof Location));
    assert.deepEqual(f.api.workspace.textDocuments.map(doc=>doc.getText()),before);assert.equal(f.api.window.activeTextEditor,undefined);assert.deepEqual(f.api.window.visibleTextEditors,[]);assert.equal(f.frames.length,0);
    assert.equal(f.notifications.filter(value=>value.method==='languageProviderReleased').length,0,'internal Symbol queries do not release native callbacks');
  }
});
test('LocationLink fields survive normalization; dedup uses outer target range and preserves source provider precedence',async()=>{
  const f=apiFixture(),uri=Uri.parse('notes:/target.cpp?q=猫#part'),outer=new Range(2,0,4,3),selected=new Range(3,1,3,2),origin=new Range(0,1,0,3);
  const link={targetUri:uri,targetRange:outer,targetSelectionRange:selected,originSelectionRange:origin};
  f.creator.languages.registerDefinitionProvider('cpp',{provideDefinition(){return [link,{...link,targetSelectionRange:new Range(2,0,2,1)}];}});
  const result=await f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3));
  assert.equal(result.length,1);assert.equal(result[0].targetUri.toString(),uri.toString());assert.deepEqual(result[0].targetRange,outer);assert.deepEqual(result[0].targetSelectionRange,selected);assert.deepEqual(result[0].originSelectionRange,origin);
  assert.equal(result[0] instanceof Location,false);
  const references=normalizeNavigation('references',[new Location(uri,selected)]);assert.equal(references[0].uri,uri.toString());
});
test('queries and matching use private mirrored source, not extension-replaced document metadata',async()=>{
  const f=apiFixture(),doc=f.api.workspace.textDocuments[0];let count=0;
  f.creator.languages.registerImplementationProvider({language:'cpp',scheme:'file',pattern:'**/*.cpp'},{provideImplementation(value){count++;assert.equal(value,doc);return new Location(Uri.parse('untitled:/result.cpp'),new Range(0,0,0,0));}});
  Object.defineProperty(doc,'version',{get(){throw Error('public version is not authority');}});
  Object.defineProperty(doc,'languageId',{get(){throw Error('public language is not authority');}});
  Object.defineProperty(doc,'uri',{get(){throw Error('public URI is not authority');}});
  assert.equal(f.creator.languages.match({language:'cpp',scheme:'file'},doc),10);
  const result=await f.api.commands.executeCommand('vscode.executeImplementationProvider',Uri.file('/work/a.cpp'),new Position(0,3));assert.equal(result.length,1);assert.equal(count,1);
});
test('URI and Position getters cannot retarget a captured source or revive its consumer owner',async()=>{
  for(const retire of [false,true]){
    const f=apiFixture();let callbacks=0;
    f.creator.languages.registerDeclarationProvider('cpp',{provideDeclaration(){callbacks++;return [];}});
    const uri=Uri.file('/work/a.cpp'),input={};
    for(const key of ['scheme','authority','path','query','fragment'])Object.defineProperty(input,key,{get(){if(key==='path'){if(retire){f.runtime.disposeOwner('consumer');f.runtime.forExtension('consumer');}else f.sync({...f.a,version:4,text:'new\r\n'});}return uri[key];}});
    await assert.rejects(f.api.commands.executeCommand('vscode.executeDeclarationProvider',input,new Position(0,3)),/retired|changed/);assert.equal(callbacks,0);assert.equal(f.frames.length,0);
  }
});
test('actual generic release follows held cancellation settlement once; duplicate and wrong owner cannot release original',async()=>{
  const f=providerFixture(),wait=deferred();let token;
  f.module.forOwner('creator').registerImplementationProvider('cpp',{provideImplementation(_,__,value){token=value;return wait.promise;}});
  const provider=f.module.snapshot()[0].id,params=f.request(provider,51),pending=f.module.provide(params);await turn();
  assert.equal(f.module.releaseRefusal(params),false);assert.equal(f.module.releaseRefusal({...params,owner:'other'}),false);
  await assert.rejects(f.module.provide(params),/duplicate/);
  f.module.cancel(params);await assert.rejects(pending,/canceled/);assert.equal(token.isCancellationRequested,true);assert.equal(f.module.pendingCount(),1);
  assert.equal(f.notifications.filter(value=>value.method==='languageProviderReleased').length,0);
  wait.resolve(new Location(Uri.file('/work/z.cpp'),new Range(0,0,0,1)));await turn();
  assert.equal(f.module.pendingCount(),0);assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params),[{session:7,owner:'creator',provider,request:51}]);
});
test('deadline returns without releasing actual generic callback capacity; eight held calls bound further invocation',async()=>{
  const f=providerFixture(5),waits=[];
  f.module.forOwner('creator').registerTypeDefinitionProvider('cpp',{provideTypeDefinition(){const value=deferred();waits.push(value);return value.promise;}});
  const provider=f.module.snapshot()[0].id;
  for(let id=1;id<=8;id++)await assert.rejects(f.module.provide(f.request(provider,id)),/deadline/);
  assert.equal(f.module.pendingCount(),8);assert.equal(f.notifications.filter(value=>value.method==='languageProviderReleased').length,0);
  await assert.rejects(f.module.provide(f.request(provider,9)),/invocation limit/);
  assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[9]);
  for(const wait of waits)wait.resolve(null);await turn();assert.equal(f.module.pendingCount(),0);
  assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request).sort((a,b)=>a-b),[1,2,3,4,5,6,7,8,9]);
});
test('returned result getters remain in original creator scope and retired creators cannot normalize late values',async()=>{
  const f=apiFixture(),wait=deferred();let entered=false;
  f.creator.languages.registerDefinitionProvider('cpp',{provideDefinition(){entered=true;return wait.promise;}});
  const query=f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3));await turn();assert(entered);
  let getters=0;const raw={get uri(){getters++;throw Error('retired normalization getter executed');},range:new Range(0,0,0,1)};
  f.runtime.disposeOwner('creator');f.runtime.forExtension('creator');wait.resolve(raw);
  assert.deepEqual(await query,[]);assert.equal(getters,0);assert.equal(f.frames.length,0);
});
test('common release covers actual completion resolve settlement independently of provide callback',async()=>{
  const f=providerFixture(),wait=deferred();
  f.module.forOwner('creator').registerCompletionItemProvider('cpp',{provideCompletionItems(){return [new CompletionItem('x')];},resolveCompletionItem(){return wait.promise;}});
  const provider=f.module.snapshot()[0].id,provided=await f.module.provide(f.request(provider,61));
  const pending=f.module.resolveCompletion({...f.request(provider,62),origin:61,handle:provided.items[0]._vscliCompletionHandle});await turn();
  f.module.cancel(f.request(provider,62));await assert.rejects(pending,/canceled/);
  assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[61]);
  wait.resolve(undefined);await turn();assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[61,62]);
});
test('pure sorting treats URI fragments as distinct, includes same internal scheme and bounds malformed provider output',()=>{
  const rows=[{uri:'file:///work/b#2',range:{start:{line:0,character:0},end:{line:0,character:1}}},{uri:'file:///work/b#1',range:{start:{line:0,character:0},end:{line:0,character:1}}},{uri:'vscode-chat-code-block:/hidden',range:{start:{line:0,character:0},end:{line:0,character:1}}}];
  assert.deepEqual(sortAndDedup(rows,'file:///work/a').map(value=>value.uri),['file:///work/b#1','file:///work/b#2']);assert.equal(sortAndDedup(rows,'vscode-chat-code-block:/source').length,3);
  assert.throws(()=>normalizeNavigation('definition',Array(513).fill(null)),/512/);assert.throws(()=>normalizeNavigation('definition',{uri:Uri.file('/work/a'),range:new Range(0,0,0,0),get targetUri(){throw Error('original getter');}}),/original getter/);
});
test('all matching providers beyond the actual eight-callback bound are queried without silently dropping registrations',async()=>{
  const f=apiFixture(),seen=[];
  for(let id=0;id<16;id++)f.creator.languages.registerDefinitionProvider('cpp',{provideDefinition(){seen.push(id);return new Location(Uri.file('/work/result.cpp'),new Range(0,0,0,1));}});
  const result=await f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3));
  assert.deepEqual(seen,Array.from({length:16},(_,index)=>15-index));assert.equal(result.length,1);assert.equal(f.frames.length,0);
});
test('public eight-query capacity retains actual callbacks; ninth refuses and release permits a fresh successful query',async()=>{
  const f=apiFixture(),waits=[];
  f.creator.languages.registerDefinitionProvider('cpp',{provideDefinition(){const value=deferred();waits.push(value);return value.promise;}});
  const queries=Array.from({length:8},()=>f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3)));
  try{
    await turn();assert.equal(waits.length,8);
    await assert.rejects(f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3)),/invocation limit/);assert.equal(waits.length,8);
    for(const wait of waits)wait.resolve(new Location(Uri.file('/work/result.cpp'),new Range(0,0,0,1)));
    for(const answer of await Promise.all(queries))assert.equal(answer.length,1);
    const fresh=f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3));await turn();assert.equal(waits.length,9);waits[8].resolve(null);assert.deepEqual(await fresh,[]);
  }finally{for(const wait of waits)wait.resolve(null);await Promise.allSettled(queries);}
});
test('provider Thenable and returned location getters execute under the original creator with original receiver',async()=>{
  const f=apiFixture(),scopes=[],raw={};let uriReads=0,rangeReads=0;
  Object.defineProperty(raw,'uri',{get(){uriReads++;scopes.push(f.runtime.currentCallerOwner());return Uri.file('/work/result.cpp');}});
  Object.defineProperty(raw,'range',{get(){rangeReads++;scopes.push(f.runtime.currentCallerOwner());return new Range(0,0,0,1);}});
  const provider={provideTypeDefinition(){assert.equal(this,provider);scopes.push(f.runtime.currentCallerOwner());return {get then(){scopes.push(f.runtime.currentCallerOwner());return resolve=>resolve(raw);}};}};
  f.creator.languages.registerTypeDefinitionProvider('cpp',provider);
  const result=await f.api.commands.executeCommand('vscode.executeTypeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3));
  assert.equal(result.length,1);assert.equal(uriReads,1);assert.equal(rangeReads,1);assert(scopes.length>=4);assert(scopes.every(owner=>owner==='creator'));
});
test('held query cannot publish after consumer retirement and does not cancel or borrow another creator lifetime',async()=>{
  const f=apiFixture(),wait=deferred();let token;
  f.creator.languages.registerImplementationProvider('cpp',{provideImplementation(_,__,value){token=value;return wait.promise;}});
  const query=f.api.commands.executeCommand('vscode.executeImplementationProvider',Uri.file('/work/a.cpp'),new Position(0,3)),rejection=assert.rejects(query,/retired/);await turn();
  f.runtime.disposeOwner('consumer');f.runtime.forExtension('consumer');assert.equal(token.isCancellationRequested,false);
  wait.resolve(new Location(Uri.file('/work/result.cpp'),new Range(0,0,0,1)));await rejection;assert.equal(f.frames.length,0);
});
test('pre-entry retired runtime scope emits no-callback release once without invoking a provider',async()=>{
  const f=apiFixture();let invoked=0;f.creator.languages.registerHoverProvider('cpp',{provideHover(){invoked++;return null;}});
  const provider=f.runtime.providerSnapshot()[0].id;
  const rejected=f.runtime.runWithOwner('creator',()=>{f.runtime.disposeOwner('creator');return f.runtime.provideLanguage({session:7,owner:'creator',provider,request:90,document:1,version:3,position:{line:0,character:3}});});
  await assert.rejects(rejected,/retired/);assert.equal(invoked,0);
  assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params),[{session:7,owner:'creator',provider,request:90}]);
});
test('resolve action actual settlement emits its own release after cancellation while retaining the original provide receipt',async()=>{
  const {CodeAction,CodeActionKind}=require('./provider-types.cjs'),f=providerFixture(),wait=deferred(),action=new CodeAction('lazy',CodeActionKind.QuickFix);
  f.module.forOwner('creator').registerCodeActionsProvider('cpp',{provideCodeActions(){return [action];},resolveCodeAction(){return wait.promise;}});
  const provider=f.module.snapshot()[0].id,params={...f.request(provider,101),range:{start:{line:0,character:0},end:{line:0,character:1}},actionContext:{triggerKind:1},workspace:[{document:1,uri:'file:///work/a.cpp',version:3}]};
  const provided=await f.module.provide(params),handle=provided[0]._vscliCodeActionHandle;
  const resolve={...params,request:102,origin:101,handle};
  const pending=f.module.resolveAction(resolve),rejected=assert.rejects(pending,/canceled/);await turn();f.module.cancel(resolve);await rejected;
  assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[101]);
  wait.resolve(action);await turn();assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[101,102]);
});
test('source exclusive selector arrays require every member exclusive rather than any matching member',async()=>{
  const f=apiFixture(),seen=[];
  const provider=name=>({provideDefinition(){seen.push(name);return new Location(Uri.file(`/work/${name}.cpp`),new Range(0,0,0,1));}});
  f.creator.languages.registerDefinitionProvider('cpp',provider('ordinary'));
  const mixed=f.creator.languages.registerDefinitionProvider([{language:'cpp',exclusive:true},{language:'rust'}],provider('mixed'));
  let result=await f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3));
  assert.deepEqual(seen,['mixed','ordinary']);assert.equal(result.length,2);
  mixed.dispose();seen.length=0;
  f.creator.languages.registerDefinitionProvider([{language:'cpp',exclusive:true},{scheme:'file',exclusive:true}],provider('exclusive'));
  result=await f.api.commands.executeCommand('vscode.executeDefinitionProvider',Uri.file('/work/a.cpp'),new Position(0,3));
  assert.deepEqual(seen,['exclusive']);assert.equal(result.length,1);
});
