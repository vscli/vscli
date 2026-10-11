'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {TextDocument,Uri,Range} = require('./api-types.cjs');
const {createProviders,normalize} = require('./providers.cjs');
const {DocumentSymbol,SymbolInformation,SymbolKind} = require('./provider-types.cjs');

function fixture(timeoutMs = 1000) {
  const documents = new Map([[1,new TextDocument({id:1,uri:'untitled:Untitled-1',version:3,languageId:'plaintext',text:'猫🙂x\r\n'})]]);
  const notifications = [];
  const providers = createProviders({session:7,timeoutMs,document:id=>documents.get(id),track:(_,value)=>value,
    notify:(method,params)=>notifications.push({method,params})});
  let sequence = 0;
  const request = (provider,patch={})=>({session:7,owner:'test.symbols',provider,document:1,
    version:documents.get(1).version,request:++sequence,symbolRequest:true,...patch});
  const register = provider=>{
    const disposable = providers.forOwner('test.symbols').registerDocumentSymbolProvider('*',provider);
    return {id:providers.snapshot().at(-1).id,disposable};
  };
  return {providers,documents,notifications,request,register};
}
function symbol() {
  return new DocumentSymbol('parent 猫','detail',SymbolKind.Class,new Range(0,0,0,4),new Range(0,1,0,3));
}
function deferred() {let resolve;const promise=new Promise(done=>{resolve=done;});return {promise,resolve};}
const tick=()=>new Promise(done=>setImmediate(done));
const releases=values=>values.filter(value=>value.method==='symbolsReleased');

test('document symbols preserve outer ranges, hierarchy and flat untitled information',async()=>{
  const {providers,documents,request,register,notifications}=fixture();
  const parent=symbol();parent.children.push(new DocumentSymbol('child','',SymbolKind.Method,new Range(0,1,0,3),new Range(0,1,0,3)));
  const {id}=register({marker:42,provideDocumentSymbols(document,token){assert.equal(this.marker,42);assert.equal(document,documents.get(1));assert.equal(token.isCancellationRequested,false);return [parent];}});
  const params=request(id),result=await providers.provide(params);
  assert.equal(result[0].kind,5);assert.equal(result[0].range.end.character,4);assert.equal(result[0].selectionRange.start.character,1);
  assert.equal(result[0].children[0].name,'child');assert.equal(providers.symbolsPending(),false);
  assert.equal(releases(notifications).filter(value=>value.params.request===params.request).length,1);
  const flat=new SymbolInformation('flat',SymbolKind.Function,'container',new Range(0,1,0,3),documents.get(1).uri);
  const converted=normalize('symbols',[flat],documents.get(1));
  assert.equal(converted[0].location.uri,'untitled:Untitled-1');assert.equal(converted[0].children,undefined);
  flat.location.uri=Uri.parse('untitled:Other');assert.throws(()=>normalize('symbols',[flat],documents.get(1)),/mirrored/);
});

test('canceled callbacks retain one actual symbol lane across providers and leave signatures independent',async()=>{
  const {providers,request,register,notifications}=fixture();const held=deferred();let actual=0,sibling=0;
  const first=register({provideDocumentSymbols(){actual++;return held.promise;}});
  const second=register({provideDocumentSymbols(){sibling++;return [];}});
  const signature=providers.forOwner('test.symbols').registerSignatureHelpProvider('*',{provideSignatureHelp:()=>null},'(');
  const signatureId=providers.snapshot().at(-1).id;
  try {
  const params=request(first.id),pending=providers.provide(params);await Promise.resolve();
  assert.equal(providers.cancel({...params,owner:'other.owner'}),false);assert.equal(providers.symbolsPending(),true);
  providers.cancel(params);await assert.rejects(pending,/canceled/);
  assert.equal(providers.pendingCount(),1);assert.equal(releases(notifications).filter(value=>value.params.request===params.request).length,0);
  for(let index=0;index<32;index++)await assert.rejects(providers.provide(request(second.id)),/already running/);
  assert.equal(actual,1);assert.equal(sibling,0);
  assert.equal(await providers.provide(request(signatureId,{symbolRequest:false,signatureRequest:true,position:{line:0,character:0}})),null);
  assert.equal(providers.symbolsPending(),true);assert.equal(providers.signaturePending(),false);
  held.resolve([symbol()]);await tick();assert.equal(providers.symbolsPending(),false);
  assert.equal(releases(notifications).filter(value=>value.params.request===params.request).length,1);
  assert.deepEqual(await providers.provide(request(second.id)),[]);
  } finally { held.resolve(null);await tick();signature.dispose(); }
});

test('deadline retains actual work and duplicate-ID rejection cannot acknowledge its release',async()=>{
  const {providers,request,register,notifications}=fixture(5);const held=deferred();let actual=0;
  const {id}=register({provideDocumentSymbols(){actual++;return held.promise;}});
  const params=request(id);await assert.rejects(providers.provide(params),/deadline/);
  assert.equal(providers.symbolsPending(),true);
  await assert.rejects(providers.provide(params),/duplicate/);
  assert.equal(releases(notifications).filter(value=>value.params.request===params.request).length,0);
  for(let index=0;index<16;index++)await assert.rejects(providers.provide(request(id)),/already running/);
  assert.equal(actual,1);assert.equal(providers.pendingCount(),1);
  held.resolve(null);await tick();assert.equal(providers.symbolsPending(),false);
  assert.equal(releases(notifications).filter(value=>value.params.request===params.request).length,1);
});

test('early rejection and retired registry IDs acknowledge only work known not to have started',async()=>{
  const {providers,request,register,notifications}=fixture();
  const entry=register({provideDocumentSymbols:()=>[]});entry.disposable.dispose();
  const params=request(entry.id);await assert.rejects(providers.provide(params),/owner/);
  assert.equal(releases(notifications).filter(value=>value.params.request===params.request).length,1);
  assert.equal(providers.symbolsPending(),false);
  const fresh=register({provideDocumentSymbols:()=>[]});const invalid=request(fresh.id,{version:2});
  await assert.rejects(providers.provide(invalid),/changed/);
  assert.equal(releases(notifications).filter(value=>value.params.request===invalid.request).length,1);
});

test('registry A→B→A retirement and document replacement cannot publish stale symbol results',async()=>{
  const {providers,documents,request,register}=fixture();const held=deferred();
  const first=register({provideDocumentSymbols:()=>held.promise});
  const pending=providers.provide(request(first.id));await Promise.resolve();
  const unrelated=providers.forOwner('test.symbols').registerHoverProvider('*',{provideHover:()=>null});unrelated.dispose();
  await assert.rejects(pending,/canceled/);assert.equal(providers.symbolsPending(),true);
  held.resolve([symbol()]);await tick();assert.equal(providers.symbolsPending(),false);
  const replacement=deferred();const second=register({provideDocumentSymbols:()=>replacement.promise});
  const next=providers.provide(request(second.id));await Promise.resolve();
  documents.set(1,new TextDocument({...documents.get(1)._snapshot}));
  replacement.resolve([symbol()]);await assert.rejects(next,/identity/);assert.equal(providers.symbolsPending(),false);
});

test('symbol normalization is bounded against growing arrays, malformed peers and excessive hierarchy',()=>{
  const {documents}=fixture();const document=documents.get(1);
  const growing=[symbol()];Object.defineProperty(growing,0,{get(){growing.length=1e9;return symbol();}});
  assert.throws(()=>normalize('symbols',growing,document),/changed/);
  const parent=symbol(),children=[symbol()];Object.defineProperty(children,0,{get(){children.length=1e9;return symbol();}});parent.children=children;
  assert.throws(()=>normalize('symbols',[parent],document),/changed/);
  assert.throws(()=>normalize('symbols',Array(513).fill(symbol()),document),/512/);
  let deep=symbol();for(let index=0;index<17;index++){const parent=symbol();parent.children=[deep];deep=parent;}
  assert.throws(()=>normalize('symbols',[deep],document),/depth/);
  const wide=symbol();wide.name='x'.repeat(4096);assert.throws(()=>normalize('symbols',Array(17).fill(wide),document),/64 KiB/);
  const invalid=symbol();invalid.selectionRange=new Range(0,0,1,0);assert.throws(()=>normalize('symbols',[symbol(),invalid],document),/outside/);
});

test('normalization getters cannot publish after owner disposal or mirror mutation',async()=>{
  for(const mutation of ['dispose','registry','document']){
    const {providers,documents,request,register}=fixture();let entry;const value=symbol();
    Object.defineProperty(value,'name',{get(){
      if(mutation==='dispose')entry.disposable.dispose();
      else if(mutation==='registry')providers.forOwner('test.symbols').registerHoverProvider('*',{provideHover:()=>null});
      else documents.get(1)._update({...documents.get(1)._snapshot,version:4,text:'猫🙂xy\r\n'});
      return 'stale';
    }});
    entry=register({provideDocumentSymbols:()=>[value]});
    await assert.rejects(providers.provide(request(entry.id)),/stale|canceled/);
    await tick();
    assert.equal(providers.symbolsPending(),false);assert.equal(providers.pendingCount(),0);
  }
});

test('global retained callback budget rejects symbol work without occupying its dedicated lane',async()=>{
  const {providers,request,register}=fixture();const held=deferred();let actual=0;
  const {id}=register({provideDocumentSymbols(){actual++;return [];}});
  providers.forOwner('test.symbols').registerHoverProvider('*',{provideHover:()=>held.promise});const hover=providers.snapshot().at(-1).id;
  const calls=Array.from({length:8},()=>providers.provide(request(hover,{symbolRequest:false,position:{line:0,character:0}})));
  try {
  await Promise.resolve();assert.equal(providers.pendingCount(),8);
  await assert.rejects(providers.provide(request(id)),/invocation limit/);
  assert.equal(actual,0);assert.equal(providers.symbolsPending(),false);
  held.resolve(null);await Promise.all(calls);assert.deepEqual(await providers.provide(request(id)),[]);
  } finally { held.resolve(null);await Promise.allSettled(calls); }
});

test('deprecated tags retain the pinned public value and reject malformed or growing arrays',()=>{
  const {documents}=fixture(),document=documents.get(1),value=symbol();
  value.tags=[1];value.deprecated=true;
  assert.deepEqual(normalize('symbols',[value],document)[0].tags,[1]);
  assert.equal(normalize('symbols',[value],document)[0].deprecated,true);
  value.tags=[0];assert.throws(()=>normalize('symbols',[value],document),/tag/);
  value.tags=[1];Object.defineProperty(value.tags,0,{get(){value.tags.length=1e9;return 1;}});
  assert.throws(()=>normalize('symbols',[value],document),/changed/);
  value.tags=[];value.deprecated='true';assert.throws(()=>normalize('symbols',[value],document),/flag/);
});
