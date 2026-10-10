'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { TextDocument, Position } = require('./api-types.cjs');
const { createProviders, normalize } = require('./providers.cjs');
const { SignatureHelp, SignatureInformation, ParameterInformation, MarkdownString } = require('./provider-types.cjs');
const { HANDLE } = require('./signatures.cjs');

function help() {
  const value = new SignatureHelp();
  const first = new SignatureInformation('f(猫,🙂)', new MarkdownString('First documentation'));
  first.parameters = [new ParameterInformation([2,3], 'cat'), new ParameterInformation([4,6], 'face')];
  const second = new SignatureInformation('f(left, right)', 'Second documentation');
  second.parameters = [new ParameterInformation('left'), new ParameterInformation('right')];
  second.activeParameter = 1;
  value.signatures = [first, second];
  return value;
}
function fixture(timeoutMs = 1000) {
  const documents = new Map([[1, new TextDocument({ id:1, uri:'file:///test.cpp', version:3, languageId:'cpp', text:'猫🙂x\r\n', isDirty:true })]]);
  const notifications = [];
  const providers = createProviders({ session:7, timeoutMs, document: id => documents.get(id), track: (_, disposable) => disposable,
    notify: (method, params) => notifications.push({method,params}) });
  let nextRequest = 0;
  const request = (provider, patch = {}) => ({session:7, owner:'test.extension', provider, document:1, version:documents.get(1).version,
    position:{line:0,character:3}, request:++nextRequest, signatureRequest:true, ...patch});
  const register = (provider, metadata = {triggerCharacters:['('],retriggerCharacters:[',']}) => {
    const disposable = providers.forOwner('test.extension').registerSignatureHelpProvider('cpp', provider, metadata);
    return { id:providers.snapshot().at(-1).id, disposable };
  };
  return {providers,documents,notifications,request,register};
}
function deferred() { let resolve; const promise = new Promise(done => {resolve = done;}); return {promise,resolve}; }
const tick = () => new Promise(done => setImmediate(done));
function releases(notifications) { return notifications.filter(value => value.method === 'signatureReleased'); }

test('signature metadata preserves distinct bounded trigger sets and captured values', () => {
  const {providers,register} = fixture();
  register({provideSignatureHelp:help}, {triggerCharacters:['('],retriggerCharacters:[',',')']});
  assert.deepEqual(providers.snapshot()[0].triggers, ['(']);
  assert.deepEqual(providers.snapshot()[0].retriggers, [',',')']);
  register({provideSignatureHelp:help}, {triggerCharacters:Array(16).fill('('),retriggerCharacters:Array(16).fill(',')});
  assert.throws(() => register({provideSignatureHelp:help}, {retriggerCharacters:Array(17).fill(',')}), /16/);
  const growing = ['(']; Object.defineProperty(growing, 0, {get() {growing.length = 1e9; return '(';}});
  assert.throws(() => register({provideSignatureHelp:help}, {triggerCharacters:growing}), /changed/);
  assert.equal(providers.snapshot().length, 2);
});

test('signature context forwards all trigger kinds and typed fallback help without mutation', async () => {
  const {providers,request,register} = fixture(); const seen = [];
  const {id} = register({marker:42, provideSignatureHelp(document,position,token,context) {
    assert.equal(this.marker,42); assert.equal(document.version,3); assert.deepEqual(position,new Position(0,3));
    assert.equal(token.isCancellationRequested,false); seen.push(context); return help();
  }});
  for (const context of [{triggerKind:1,isRetrigger:false}, {triggerKind:2,triggerCharacter:'(',isRetrigger:false}, {triggerKind:3,isRetrigger:true,activeSignatureHelp:normalize('signature',help())}]) {
    await providers.provide(request(id,{signatureContext:context}));
  }
  assert.deepEqual(seen.slice(0,2), [{triggerKind:1,isRetrigger:false},{triggerKind:2,triggerCharacter:'(',isRetrigger:false}]);
  assert.ok(seen[2].activeSignatureHelp instanceof SignatureHelp);
  assert.ok(seen[2].activeSignatureHelp.signatures[0] instanceof SignatureInformation);
  assert.ok(seen[2].activeSignatureHelp.signatures[0].parameters[0] instanceof ParameterInformation);
  assert.ok(seen[2].activeSignatureHelp.signatures[0].documentation instanceof MarkdownString);
  assert.equal(seen[2].activeSignatureHelp.signatures[1].activeParameter,1);
  for (const context of [{triggerKind:0}, {triggerKind:null}, {triggerKind:2}, {triggerKind:2,triggerCharacter:'..'}, {triggerKind:3,isRetrigger:1}, {isRetrigger:null}, {proposed:true}, {activeSignatureHelp:{signatures:[],activeSignature:-1}}]) {
    await assert.rejects(providers.provide(request(id,{signatureContext:context})), /signature|Signature/);
  }
  assert.equal(seen.length,3); assert.equal(providers.signaturePending(),false);
});

test('retrigger passes the original circular help identity across normal document versions', async () => {
  const {providers,documents,request,register} = fixture(); const original = help(); let previous;
  original.opaque = {callback:()=>42}; original.opaque.self = original;
  const {id} = register({provideSignatureHelp(_,__,___,context) {
    if (previous) {assert.equal(context.activeSignatureHelp,original); assert.equal(context.activeSignatureHelp.opaque.callback(),42); assert.equal(original.activeSignature,1); assert.equal(original.activeParameter,1);}
    return original;
  }});
  const firstRequest = request(id); previous = await providers.provide(firstRequest);
  assert.equal(typeof previous[HANDLE],'number'); assert.equal(previous.opaque,undefined);
  const document = documents.get(1); document._update({...document._snapshot,version:4,text:'猫🙂xy\r\n'}); providers.documentChanged();
  assert.equal(providers.retainedSignatureCount(),1);
  const nextRequest = request(id,{signatureContext:{triggerKind:3,isRetrigger:true,activeSignatureHelp:{...previous,activeSignature:1,activeParameter:1}}});
  const next = await providers.provide(nextRequest);
  assert.notEqual(next[HANDLE],previous[HANDLE]); assert.equal(providers.retainedSignatureCount(),2,'Prior help remains available until the editor accepts or drops its replacement');
  providers.cancel({...firstRequest,releaseSignatureHelp:true,signatureHandle:previous[HANDLE]});
  assert.equal(providers.retainedSignatureCount(),1,'Releasing replaced help cannot retire the current help');
  providers.cancel({...nextRequest,releaseSignatureHelp:true,signatureHandle:next[HANDLE]});
  assert.equal(providers.retainedSignatureCount(),0);
});

test('cancellation retains one actual signature slot across providers until positive release', async () => {
  const {providers,request,register,notifications} = fixture(); const held = deferred(); let firstCalls = 0, secondCalls = 0;
  const first = register({provideSignatureHelp(){firstCalls++; return held.promise;}});
  const second = register({provideSignatureHelp(){secondCalls++; return help();}});
  const params = request(first.id); const pending = providers.provide(params);
  await Promise.resolve(); providers.cancel(params);
  await assert.rejects(pending,/canceled/);
  assert.equal(providers.signaturePending(),true); assert.equal(providers.pendingCount(),1);
  assert.equal(releases(notifications).filter(value=>value.params.request === params.request).length,0);
  await assert.rejects(providers.provide(request(second.id)),/already running/); assert.equal(secondCalls,0);
  const hover = providers.forOwner('test.extension').registerHoverProvider('cpp',{provideHover:()=>({contents:'independent'})});
  assert.deepEqual(await providers.provide(request(providers.snapshot().at(-1).id,{signatureRequest:false})),{contents:['independent']});
  held.resolve(help()); await tick();
  assert.equal(firstCalls,1); assert.equal(providers.signaturePending(),false);
  assert.equal(providers.retainedSignatureCount(),0);
  assert.equal(releases(notifications).filter(value=>value.params.request === params.request).length,1);
  assert.ok(await providers.provide(request(second.id))); hover.dispose();
});

test('deadline errors do not release actual capacity, and early rejection acknowledges no callback ran', async () => {
  const {providers,request,register,notifications} = fixture(5); const held = deferred(); let calls = 0;
  const {id} = register({provideSignatureHelp(){calls++; return held.promise;}});
  const params = request(id); await assert.rejects(providers.provide(params),/deadline/);
  assert.equal(providers.signaturePending(),true);
  for (let index=0; index<20; index++) await assert.rejects(providers.provide(request(id)),/already running/);
  assert.equal(calls,1); assert.equal(providers.pendingCount(),1);
  assert.equal(releases(notifications).filter(value=>value.params.request === params.request).length,0);
  held.resolve(null); await tick(); assert.equal(providers.signaturePending(),false);
  assert.equal(releases(notifications).filter(value=>value.params.request === params.request).length,1);
  const invalid = request(id,{signatureContext:{triggerKind:2}});
  await assert.rejects(providers.provide(invalid),/missing/);
  assert.equal(releases(notifications).filter(value=>value.params.request === invalid.request).length,1);
  const stale = request(999);
  await assert.rejects(providers.provide(stale),/owner/);
  assert.equal(releases(notifications).filter(value=>value.params.request === stale.request).length,1);
});

test('all overloads, UTF-16 offsets, field and aggregate budgets validate before publication', () => {
  let value = help(); assert.equal(normalize('signature',value).signatures[1].activeParameter,1);
  value.signatures[1].parameters[0].label = [2,1]; assert.throws(()=>normalize('signature',value),/offset/);
  value = help(); value.signatures[1].documentation = 'x'.repeat(8193); assert.throws(()=>normalize('signature',value),/budget/);
  value = help(); value.signatures[0].parameters[1].label = [4,5]; assert.throws(()=>normalize('signature',value),/UTF-16/);
  value = help(); value.signatures[1].activeParameter = -1; assert.throws(()=>normalize('signature',value),/index/);
  value = help(); value.signatures[1].documentation = {value:'x',isTrusted:true}; assert.throws(()=>normalize('signature',value),/Trusted/);
  value = help(); value.signatures = Array(33).fill(value.signatures[0]); assert.throws(()=>normalize('signature',value),/32/);
  value = help(); value.signatures[1].parameters = Array(129).fill({label:'x'}); assert.throws(()=>normalize('signature',value),/128/);
  value = help(); value.signatures = Array.from({length:9},()=>({label:'x'.repeat(8192),parameters:[]})); assert.throws(()=>normalize('signature',value),/64 KiB/);
  value = help(); value.signatures = Array.from({length:32},()=>({label:'f(x)',parameters:[{label:'x',documentation:'x'.repeat(8192)}]})); assert.throws(()=>normalize('signature',value),/256 KiB/);
});

test('array element getters cannot expand normalization loops or change captured parameter labels', () => {
  for (const path of ['signatures','parameters','pair']) {
    const value = help(), array = path === 'signatures' ? value.signatures : path === 'parameters' ? value.signatures[1].parameters : value.signatures[0].parameters[0].label;
    const first = array[0]; Object.defineProperty(array,0,{get(){array.length=1e9;return first;}});
    assert.throws(()=>normalize('signature',value),/changed/);
  }
  const value = help(); let reads = 0;
  Object.defineProperty(value.signatures[0],'label',{get(){reads++; return reads === 1 ? 'f(猫,🙂)' : 'different';}});
  assert.equal(normalize('signature',value).signatures[0].label,'f(猫,🙂)'); assert.equal(reads,1);
});

test('normalization getters cannot publish help after registration or document invalidation', async () => {
  for (const mutation of ['dispose','registry','document']) {
    const {providers,documents,request,register} = fixture(); let disposable;
    const value = help(); Object.defineProperty(value.signatures[1],'documentation',{get(){
      if (mutation === 'dispose') disposable.dispose();
      else if (mutation === 'registry') providers.forOwner('other').registerHoverProvider('*',{provideHover:()=>null});
      else documents.get(1)._update({...documents.get(1)._snapshot,version:4,text:'new'});
      return 'old';
    }});
    const registration = register({provideSignatureHelp:()=>value}); disposable = registration.disposable;
    await assert.rejects(providers.provide(request(registration.id)),/stale|canceled/);
    assert.equal(providers.retainedSignatureCount(),0); assert.equal(providers.signaturePending(),false);
  }
});

test('original-help setters revalidate ownership before invoking a retrigger callback', async () => {
  const {providers,request,register} = fixture(); const value = help(); let calls = 0;
  const {id,disposable} = register({provideSignatureHelp(){calls++; return value;}});
  const first = await providers.provide(request(id));
  Object.defineProperty(value,'activeSignature',{set(){disposable.dispose();}});
  await assert.rejects(providers.provide(request(id,{signatureContext:{triggerKind:3,isRetrigger:true,activeSignatureHelp:first}})),/stale|canceled/);
  assert.equal(calls,1); assert.equal(providers.retainedSignatureCount(),0); assert.equal(providers.signaturePending(),false);
});

test('provider callback getters cannot invoke a callback after owner disposal', async () => {
  const {providers,request,register} = fixture(); let disposable, reads = 0, calls = 0;
  const provider = {}; Object.defineProperty(provider,'provideSignatureHelp',{get(){
    if (++reads === 2) disposable.dispose();
    return () => {calls++; return help();};
  }});
  const registration = register(provider); disposable = registration.disposable;
  await assert.rejects(providers.provide(request(registration.id)),/stale|canceled/);
  await tick(); assert.equal(calls,0); assert.equal(providers.retainedSignatureCount(),0); assert.equal(providers.signaturePending(),false);
});

test('explicit release and close/reopen retire original help without admitting stale contexts', async () => {
  const {providers,documents,request,register} = fixture(); const {id} = register({provideSignatureHelp:help});
  const params = request(id), value = await providers.provide(params);
  providers.cancel({...params,releaseSignatureHelp:true,signatureHandle:value[HANDLE],document:2});
  assert.equal(providers.retainedSignatureCount(),1);
  providers.cancel({...params,releaseSignatureHelp:true,signatureHandle:value[HANDLE]});
  assert.equal(providers.retainedSignatureCount(),0);
  await assert.rejects(providers.provide(request(id,{signatureContext:{triggerKind:3,isRetrigger:true,activeSignatureHelp:value}})),/Stale signature/);
  const second = await providers.provide(request(id));
  documents.set(1,new TextDocument({...documents.get(1)._snapshot})); providers.documentChanged();
  assert.equal(providers.retainedSignatureCount(),0);
  await assert.rejects(providers.provide(request(id,{signatureContext:{triggerKind:3,isRetrigger:true,activeSignatureHelp:second}})),/Stale signature/);
});

test('original help cache has a bounded handle count and reclaimable lifetime', async () => {
  const {providers,documents,request,register} = fixture(); const {id} = register({provideSignatureHelp:help}); const retained = [];
  for (let index=1; index<=33; index++) documents.set(index,new TextDocument({id:index,uri:`file:///test${index}.cpp`,version:3,languageId:'cpp',text:'猫🙂x'}));
  for (let index=1; index<=32; index++) { const params = request(id,{document:index}); retained.push({params,value:await providers.provide(params)}); }
  await assert.rejects(providers.provide(request(id,{document:33})),/cache budget/);
  assert.equal(providers.retainedSignatureCount(),32);
  const first = retained[0]; providers.cancel({...first.params,releaseSignatureHelp:true,signatureHandle:first.value[HANDLE]});
  assert.ok(await providers.provide(request(id,{document:33}))); assert.equal(providers.retainedSignatureCount(),32);
});

test('original help cache normalized metadata stays within 2 MiB across retained requests', async () => {
  const {providers,request,register} = fixture();
  const value = help(); value.signatures = Array.from({length:28},()=>({label:'f(x)',parameters:[{label:'x',documentation:'x'.repeat(8192)}]}));
  const {id} = register({provideSignatureHelp:()=>value}); const retained = [];
  for (let index=0; index<9; index++) { const params = request(id); retained.push({params,value:await providers.provide(params)}); }
  await assert.rejects(providers.provide(request(id)),/cache budget/);
  assert.equal(providers.retainedSignatureCount(),9);
  const first = retained[0]; providers.cancel({...first.params,releaseSignatureHelp:true,signatureHandle:first.value[HANDLE]});
  assert.ok(await providers.provide(request(id))); assert.equal(providers.retainedSignatureCount(),9);
});
