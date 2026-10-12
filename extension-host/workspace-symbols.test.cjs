'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {createProviders}=require('./providers.cjs');
const {Uri,Range}=require('./api-types.cjs');
const {Location,SymbolInformation}=require('./provider-types.cjs');
const {normalize,sortAndDedup,createWorkspaceSymbols}=require('./workspace-symbols.cjs');
const turn=()=>new Promise(resolve=>setImmediate(resolve));
function deferred(){let resolve;const promise=new Promise(done=>resolve=done);return{promise,resolve};}
function symbol(name='Alpha',range=new Range(0,0,0,2),uri=Uri.file('/work/a.cpp')){return new SymbolInformation(name,11,'Container',new Location(uri,range));}
function moduleFixture(timeoutMs=200){
 let workspace={};const notifications=[];
 const module=createProviders({session:7,timeoutMs,workspaceIdentity:()=>workspace,notify:(method,params)=>notifications.push({method,params}),track:(_,value)=>value,document:()=>undefined});
 const register=provider=>{module.forOwner('creator').registerWorkspaceSymbolProvider(provider);return module.snapshot().at(-1).id;};
 const request=(provider,request,query='')=>({session:7,owner:'creator',provider,request,query});
 return{module,register,request,notifications,retireWorkspace(){workspace={};module.documentChanged();}};
}
function apiFixture(){
 const frames=[],notifications=[],runtime=createApi((method,params)=>{frames.push({method,params});return Promise.reject(Error('unexpected native edit'));},(method,params)=>notifications.push({method,params}),{session:7,languageProviders:true});
 runtime.configure('/work',{},[]);runtime.sync({generation:1,documents:[],editors:[],activeEditor:null});
 return{runtime,consumer:runtime.forExtension('consumer'),creator:runtime.forExtension('creator'),frames,notifications};
}
test('public global queries work with zero editors, preserve SymbolInformation fields/URIs and never eagerly resolve',async()=>{
 const f=apiFixture(),seen=[];let resolves=0;
 const first={provideWorkspaceSymbols(query){assert.equal(this,first);assert.equal(query,'a');seen.push('first');const result=new SymbolInformation('Alpha',11,'Container',new Location(Uri.parse('notes:/target?q=猫#part')));result.tags=[1];return[result];},resolveWorkspaceSymbol(){resolves++;throw Error('eager resolver');}};
 f.creator.languages.registerWorkspaceSymbolProvider(first);
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){seen.push('null');return null;}});
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){seen.push('error');throw Error('isolated');}});
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){seen.push('last');return[symbol('Beta')];}});
 const result=await f.consumer.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','a');
 assert.deepEqual(seen,['first','null','error','last']);assert.deepEqual(result.map(value=>value.name),['Alpha','Beta']);assert(result.every(value=>value instanceof SymbolInformation));assert(result[0].location instanceof Location);assert.equal(result[0].location.range,undefined);assert.equal(Object.hasOwn(result[0].location,'range'),false);assert.equal(result[0].location.uri.toString(),'notes:/target?q%3D%E7%8C%AB#part');assert.deepEqual(result[0].tags,[1]);assert.equal(result[0].containerName,'Container');assert.equal(resolves,0);assert.equal(f.frames.length,0);assert.deepEqual(f.consumer.workspace.textDocuments,[]);assert.deepEqual(f.consumer.window.visibleTextEditors,[]);assert.equal(f.notifications.filter(value=>value.method==='languageProviderReleased').length,0);
});
test('equal duplicates preserve actual settlement order while public values remain fresh copies',async()=>{
 const f=apiFixture(),slow=deferred();let original;
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){return slow.promise;}});
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){original=symbol();return[original];}});
 const result=f.consumer.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','');await turn();slow.resolve([symbol()]);
 const values=await result;assert.equal(values.length,1);assert.notEqual(values[0],original);assert.notEqual(values[0].location,original.location);assert.deepEqual(values[0].location.range,original.location.range);
});
test('source comparator keeps touching intervals and groups overlapping ranges against the first element',()=>{
 const row=(start,end,resolves=false)=>({symbol:{name:'X',kind:12,location:{uri:'file:///a',range:{start:{line:0,character:start},end:{line:0,character:end}}}},resolves});
 assert.equal(sortAndDedup([row(0,2),row(2,4)]).length,2,'touching is not intersection');
 const values=sortAndDedup([row(0,3),row(2,5),row(4,7)]);assert.deepEqual(values.map(value=>value.symbol.location.range.start.character),[0,4],'group compares each row to original group first');
 const lazy={symbol:{name:'X',kind:12,location:{uri:'file:///a'}},resolves:true};
 const distinct=sortAndDedup([row(0,3),lazy]);
 assert.equal(distinct[0],lazy,'resolver sorts before ranged nonresolver');
 assert.equal(distinct.length,2,'nonzero resolver comparison retains both groups');
});
test('native lazy resolve receives exact original cyclic object/receiver and may replace all symbol fields',async()=>{
 const f=moduleFixture();const raw=symbol();delete raw.location.range;raw.data={self:raw};let resolves=0;
 const provider={provideWorkspaceSymbols(){assert.equal(this,provider);return[raw];},resolveWorkspaceSymbol(value){assert.equal(this,provider);assert.equal(value,raw);assert.equal(value.data.self,raw);resolves++;const result=symbol('Changed',new Range(2,1,3,2),Uri.parse('other:/resolved'));result.tags=[1];return result;}};
 const id=f.register(provider),listed=await f.module.provideWorkspaceSymbols(f.request(id,10));assert.equal(resolves,0);assert.equal(listed.symbols[0].location.range,undefined);assert.equal(f.module.retainedWorkspaceSymbolCount(),1);
 const result=await f.module.resolveWorkspaceSymbol({session:7,owner:'creator',provider:id,request:11,origin:10,handle:listed.symbols[0]._vscliWorkspaceSymbolHandle});assert.equal(result.name,'Changed');assert.equal(result.location.uri,'other:/resolved');assert.deepEqual(result.location.range,{start:{line:2,character:1},end:{line:3,character:2}});assert.deepEqual(result.tags,[1]);assert.equal(resolves,1);
 assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[10,11]);
});
test('resolve null is retained as a real null result without inventing a range',async()=>{
 const f=moduleFixture(),id=f.register({provideWorkspaceSymbols(){return[symbol()];},resolveWorkspaceSymbol(){return null;}}),listed=await f.module.provideWorkspaceSymbols(f.request(id,1));
 assert.equal(await f.module.resolveWorkspaceSymbol({session:7,owner:'creator',provider:id,request:2,origin:1,handle:listed.symbols[0]._vscliWorkspaceSymbolHandle}),null);
});
test('held canceled resolver keeps actual capacity and releases once only after original work settles',async()=>{
 const f=moduleFixture(),held=deferred();let token;const id=f.register({provideWorkspaceSymbols(){return[symbol()];},resolveWorkspaceSymbol(_,value){token=value;return held.promise;}}),listed=await f.module.provideWorkspaceSymbols(f.request(id,1));
 const params={session:7,owner:'creator',provider:id,request:2,origin:1,handle:listed.symbols[0]._vscliWorkspaceSymbolHandle},pending=f.module.resolveWorkspaceSymbol(params);await turn();
 await assert.rejects(f.module.resolveWorkspaceSymbol(params),/duplicate/);assert.equal(f.module.releaseRefusal(params),false);
 f.module.cancel(f.request(id,1));await assert.rejects(pending,/canceled/);assert(token.isCancellationRequested);assert.equal(f.module.pendingCount(),1);assert.equal(f.module.retainedWorkspaceSymbolCount(),0);assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[1]);
 held.resolve(symbol('late'));await turn();assert.equal(f.module.pendingCount(),0);assert.deepEqual(f.notifications.filter(value=>value.method==='languageProviderReleased').map(value=>value.params),[{session:7,owner:'creator',provider:id,request:1},{session:7,owner:'creator',provider:id,request:2}]);
});
test('provider disposal and workspace replacement fence held results and never revive old handles',async()=>{
 for(const retire of ['provider','workspace']){
  const f=moduleFixture(),held=deferred();let getters=0;const id=f.register({provideWorkspaceSymbols(){return held.promise;},resolveWorkspaceSymbol(value){return value;}});
  const pending=f.module.provideWorkspaceSymbols(f.request(id,1));await turn();if(retire==='provider')f.module.disposeOwner('creator');else f.retireWorkspace();await assert.rejects(pending,/canceled/);
  held.resolve([{get name(){getters++;return'late';},kind:11,location:new Location(Uri.file('/a'))}]);await turn();assert.equal(getters,0);assert.equal(f.module.retainedWorkspaceSymbolCount(),0);assert.equal(f.module.pendingCount(),0);
 }
});
test('wrong owner/provider/origin/handle refuses without calling a resolver or erasing live cached originals',async()=>{
 const f=moduleFixture();let resolved=0;const id=f.register({provideWorkspaceSymbols(){return[symbol()];},resolveWorkspaceSymbol(value){resolved++;return value;}}),list=await f.module.provideWorkspaceSymbols(f.request(id,1));
 const params={session:7,owner:'creator',provider:id,request:2,origin:1,handle:list.symbols[0]._vscliWorkspaceSymbolHandle};
 for(const change of [{owner:'other'},{provider:id+1},{origin:99},{handle:99}])await assert.rejects(f.module.resolveWorkspaceSymbol({...params,...change}),/Stale|stale/);
 assert.equal(resolved,0);assert.equal(f.module.retainedWorkspaceSymbolCount(),1);assert.equal((await f.module.resolveWorkspaceSymbol(params)).name,'Alpha');
});
test('all global providers beyond eight are queued and invoked within the shared actual lane',async()=>{
 const f=moduleFixture(),entered=[],held=[];
 for(let index=0;index<12;index++)f.register({provideWorkspaceSymbols(){entered.push(index);if(index<8){const wait=deferred();held.push(wait);return wait.promise;}return[symbol(`N${index}`)];}});
 const query=f.module.queryWorkspaceSymbols('consumer','');await turn();assert.deepEqual(entered,[0,1,2,3,4,5,6,7]);assert.equal(f.module.pendingCount(),8);
 held.forEach(wait=>wait.resolve([]));const results=await query;assert.deepEqual(entered,[0,1,2,3,4,5,6,7,8,9,10,11]);assert.equal(results.length,4);assert.equal(f.module.pendingCount(),0);assert.equal(f.notifications.filter(value=>value.method==='languageProviderReleased').length,0);
});
test('eight public held queries bound ninth admission and deadlines do not free actual callbacks',async()=>{
 const f=moduleFixture(30),waits=[];f.register({provideWorkspaceSymbols(){const value=deferred();waits.push(value);return value.promise;}});
 const queries=Array.from({length:8},()=>f.module.queryWorkspaceSymbols('consumer',''));const outcomes=queries.map(value=>assert.rejects(value,/deadline/));await turn();assert.equal(waits.length,8);
 await assert.rejects(f.module.queryWorkspaceSymbols('consumer',''),/query invocation limit/);await Promise.all(outcomes);assert.equal(f.module.pendingCount(),8);
 const id=f.module.snapshot()[0].id;await assert.rejects(f.module.provideWorkspaceSymbols(f.request(id,99)),/invocation limit/);assert.equal(waits.length,8);
 waits.forEach(wait=>wait.resolve(null));await turn();assert.equal(f.module.pendingCount(),0);assert.equal(f.notifications.filter(value=>value.method==='languageProviderReleased').length,1);
});
test('a public query creator error is isolated but original consumer retirement cannot normalize or return late values',async()=>{
 const f=apiFixture(),wait=deferred();f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){return wait.promise;}});
 const query=f.consumer.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','');await turn();f.runtime.disposeOwner('consumer');f.runtime.forExtension('consumer');wait.resolve([symbol()]);await assert.rejects(query,/retired|disposed/);assert.equal(f.frames.length,0);
});
test('canonical cache replacement is transactional at the aggregate handle limit and old origin cannot borrow a relist',()=>{
 const entries=new Set(),workspace={},cache=createWorkspaceSymbols({registered:entry=>entries.has(entry),workspaceIdentity:()=>workspace});
 const a={owner:'A',resolves:true},b={owner:'B',resolves:true};entries.add(a);entries.add(b);
 const converted=normalize(Array.from({length:512},(_,i)=>symbol(`N${i}`))),first=cache.retain({entry:a,workspace},converted,1);assert.equal(cache.count(),512);
 assert.throws(()=>cache.retain({entry:b,workspace},normalize([symbol()]),2),/budget/);assert.equal(cache.count(),512);assert.equal(cache.lookup({handle:first.symbols[0]._vscliWorkspaceSymbolHandle,origin:1},a).original,converted.originals[0]);
 const replacement=cache.retain({entry:a,workspace},normalize([symbol('new')]),3);assert.equal(cache.count(),1);assert.throws(()=>cache.lookup({handle:first.symbols[0]._vscliWorkspaceSymbolHandle,origin:1},a),/Stale/);assert.equal(cache.lookup({handle:replacement.symbols[0]._vscliWorkspaceSymbolHandle,origin:3},a).original.name,'new');
});
test('normalization bounds malformed metadata, captures range/name once, and preserves original opaque data without serializing it',()=>{
 const raw=symbol(),cyclic={};cyclic.self=cyclic;raw.data=cyclic;let names=0,ranges=0;Object.defineProperty(raw,'name',{get(){names++;return'X';}});const range=raw.location.range;Object.defineProperty(raw.location,'range',{get(){ranges++;return range;}});
 const result=normalize([raw]);assert.equal(names,1);assert.equal(ranges,1);assert.equal(result.originals[0],raw);assert.equal(Object.hasOwn(result.symbols[0],'data'),false);
 assert.throws(()=>normalize(Array(513).fill(raw)),/512/);assert.throws(()=>normalize([{name:'X',kind:99,location:raw.location}]),/kind/);assert.throws(()=>normalize([{name:'X',kind:1,location:{uri:Uri.file('/a'),range},tags:[0]}]),/tag/);
});
test('unrelated document mirrors never cancel document-independent global queries',async()=>{
 const f=apiFixture(),wait=deferred();let token;
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(_,value){token=value;return wait.promise;}});
 const query=f.consumer.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','');await turn();
 f.runtime.sync({generation:2,documents:[{id:1,uri:'untitled:test',version:1,languageId:'plaintext',text:'猫🙂\r\n',isDirty:true}],editors:[],activeEditor:null});
 assert.equal(token.isCancellationRequested,false);wait.resolve([symbol()]);assert.equal((await query)[0].name,'Alpha');
});
test('workspace Thenable and returned metadata normalize under original creator lease, not cross-owner consumer',async()=>{
 const f=apiFixture(),seen=[];
 const raw={get name(){seen.push(f.runtime.currentCallerOwner());return'Owned';},kind:11,location:new Location(Uri.file('/a'),new Range(0,0,0,1))};
 const thenable={get then(){seen.push(f.runtime.currentCallerOwner());return function(resolve){assert.equal(this,thenable);seen.push(f.runtime.currentCallerOwner());resolve([raw]);};}};
 const provider=Object.freeze({provideWorkspaceSymbols(){assert.equal(this,provider);seen.push(f.runtime.currentCallerOwner());return thenable;}});
 f.creator.languages.registerWorkspaceSymbolProvider(provider);assert.equal((await f.consumer.commands.executeCommand('vscode.executeWorkspaceSymbolProvider',''))[0].name,'Owned');assert.deepEqual(seen,['creator','creator','creator','creator']);
});
test('explicit original-list retirement refuses malformed origin and cancels actual resolver without early release',async()=>{
 const f=moduleFixture(),wait=deferred();const id=f.register({provideWorkspaceSymbols(){return[symbol()];},resolveWorkspaceSymbol(){return wait.promise;}}),list=await f.module.provideWorkspaceSymbols(f.request(id,1));
 const pending=f.module.resolveWorkspaceSymbol({session:7,owner:'creator',provider:id,request:2,origin:1,handle:list.symbols[0]._vscliWorkspaceSymbolHandle});await turn();
 assert.equal(f.module.retireWorkspaceSymbols({session:7,owner:'creator',origin:1,extra:true}),false);assert.equal(f.module.retainedWorkspaceSymbolCount(),1);
 assert.equal(f.module.retireWorkspaceSymbols({session:7,owner:'other',origin:1}),true);assert.equal(f.module.retainedWorkspaceSymbolCount(),1);
 f.module.retireWorkspaceSymbols({session:7,owner:'creator',origin:1});await assert.rejects(pending,/canceled/);assert.equal(f.module.pendingCount(),1);wait.resolve(null);await turn();assert.equal(f.module.pendingCount(),0);
});
test('public absent-range resolver and ranged nonresolver stay distinct while actual equal Beta rows deduplicate',async()=>{
 const f=apiFixture();let resolves=0;
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){return[new SymbolInformation('Alpha',11,'Container',new Location(Uri.file('/work/a.cpp'))),symbol('Beta')];},resolveWorkspaceSymbol(){resolves++;throw Error('query must not eagerly resolve');}});
 f.creator.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){return[symbol('Alpha'),symbol('Beta')];}});
 const result=await f.consumer.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','');
 assert.deepEqual(result.map(value=>value.name),['Alpha','Alpha','Beta']);assert.equal(result[0].location.range,undefined);assert.deepEqual(result[1].location.range,new Range(0,0,0,2));assert.deepEqual(result[2].location.range,new Range(0,0,0,2));assert.equal(resolves,0);assert.equal(f.frames.length,0);
});
