'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { CodeAction, CodeActionKind: K } = require('./code-action-types.cjs');
const { createProviders } = require('./providers.cjs');
const { TextDocument } = require('./api-types.cjs');
function fixture(callback, metadata) {
  const document = new TextDocument({id:1,uri:'untitled:kinds',version:1,languageId:'cpp',text:'猫🙂\r\nvalue',isDirty:true});
  const providers = createProviders({session:7,timeoutMs:100,notify:()=>{},track:(_,d)=>d,document:id=>id===1?document:undefined,diagnostics:()=>[]});
  const owner = providers.forOwner('test.kinds');
  if (callback) owner.registerCodeActionsProvider('cpp',{provideCodeActions:callback},metadata);
  const request = only => ({session:7,owner:'test.kinds',provider:1,request:1,document:1,version:1,
    range:{start:{line:0,character:0},end:{line:0,character:0}},
    actionContext:{triggerKind:1,...(only===undefined?{}:{only})},
    workspace:[{document:1,uri:'untitled:kinds',version:1}]});
  return {providers,owner,document,request};
}

test('pinned value constructors preserve arbitrary references and mutable own properties', () => {
  assert.deepEqual(Object.getOwnPropertyNames(K.prototype), ['constructor','append','intersects','contains']);
  for (const value of [undefined,null,0,17,{},Symbol('kind'),'x'.repeat(129)]) {
    const kind = new K(value); assert.equal(kind.value,value); assert.equal(Object.isFrozen(kind),false);
    assert.deepEqual(Object.getOwnPropertyDescriptor(kind,'value'),{value,writable:true,enumerable:true,configurable:true});
    kind.value='changed'; assert.equal(kind.value,'changed'); kind.extra=42; assert.equal(kind.extra,42);
  }
  const kind = new K('quickfix'), title = {opaque:true}, action = new CodeAction(title,kind);
  assert.equal(action.title,title); assert.equal(action.kind,kind); assert(Object.hasOwn(new CodeAction('unset'),'kind'));
  assert.equal(new CodeAction('unset').kind,undefined); assert.equal(new CodeAction(null,17).kind,17);
});

test('append follows pinned plus coercion, preserves empty-base references and throws for Symbols', () => {
  const opaque={toString:()=> 'child'}, empty = new K('');
  assert.equal(empty.append(opaque).value,opaque); assert.equal(new K('source').append(opaque).value,'source.child');
  assert.equal(new K('source').append(undefined).value,'source.undefined');
  assert.throws(()=>new K('source').append(Symbol('child')),TypeError);
  assert.throws(()=>new K(Symbol('parent')).append('child'),TypeError);
});

test('Empty contains equality and dot-prefixed descendants, without universal special casing', () => {
  assert(K.Empty.contains(new K(''))); assert(K.Empty.contains(new K('.child')));
  assert.equal(K.Empty.contains(K.QuickFix),false); assert.equal(K.Empty.intersects(K.Source),false);
  assert(K.Refactor.contains(K.RefactorExtract)); assert.equal(K.RefactorExtract.contains(K.Refactor),false);
  assert.equal(K.Source.contains(new K('sourceish')),false); assert(K.Source.intersects(K.SourceFixAll));
});

test('pinned constants remain writable and mutable and append uses the source separator', () => {
  const original=K.Notebook, separator=K.sep;
  try {
    const replacement=new K('custom'); K.Notebook=replacement; assert.equal(K.Notebook,replacement);
    assert.deepEqual(Object.getOwnPropertyDescriptor(K,'Notebook'),{value:replacement,writable:true,enumerable:true,configurable:true});
    K.sep='/'; assert.equal(new K('source').append('fix').value,'source/fix');
    assert(new K('source').contains(new K('source/fix')));
  } finally { K.Notebook=original; K.sep=separator; }
  const constantValue=K.QuickFix.value;
  try { K.QuickFix.value='changed'; assert.equal(K.QuickFix.value,'changed'); } finally { K.QuickFix.value=constantValue; }
});

test('pinned ES5 construction still retains subclass identities for permissive values', () => {
  function LegacyKind(value) { return K.call(this,value); }
  LegacyKind.prototype=Object.create(K.prototype); LegacyKind.prototype.constructor=LegacyKind;
  function LegacyAction(title,kind) { return CodeAction.apply(this,[title,kind]); }
  LegacyAction.prototype=Object.create(CodeAction.prototype); LegacyAction.prototype.constructor=LegacyAction;
  const kind=new LegacyKind(17), action=new LegacyAction('fix',kind);
  assert(kind instanceof LegacyKind); assert(kind instanceof K); assert.equal(kind.value,17);
  assert(action instanceof LegacyAction); assert(action instanceof CodeAction); assert.equal(action.kind,kind);
});

test('metadata moves string and UTF8 byte admission outside the permissive constructor', () => {
  const f=fixture(), callback={provideCodeActions:()=>[]};
  for(const value of [undefined,null,17,{},Symbol('bad'),'x'.repeat(129),'🙂'.repeat(33)]) {
    const kind=new K(value);
    assert.throws(()=>f.owner.registerCodeActionsProvider('cpp',callback,{providedCodeActionKinds:[kind]}),/128 UTF-8 bytes/);
    assert.deepEqual(f.providers.snapshot(),[]);
  }
  const valid=new K('🙂'.repeat(32)); let reads=0;
  const metadata={get providedCodeActionKinds(){reads++; return [valid];}};
  f.owner.registerCodeActionsProvider('cpp',callback,metadata); assert.equal(reads,1);
  valid.value=17; assert.deepEqual(f.providers.snapshot()[0].actionKinds,['🙂'.repeat(32)]);
});

test('row serialization rejects unvalidated or mutated kinds while preserving valid peer rows', async () => {
  const invalid=[undefined,null,17,{},Symbol('bad'),'x'.repeat(129),'🙂'.repeat(33)].map(value=>new CodeAction('invalid',new K(value)));
  const changed=new K('quickfix'); changed.value='x'.repeat(129); invalid.push(new CodeAction('changed',changed));
  const f=fixture(()=>[...invalid,new CodeAction('valid',new K('🙂'.repeat(32)))]);
  const rows=await f.providers.provide(f.request()); assert.equal(rows.length,invalid.length+1);
  for(const row of rows.slice(0,-1)) { assert(row.disabled); assert.equal(row.kind,undefined); assert.equal(row.edit,undefined); }
  assert.equal(rows.at(-1).title,'valid'); assert.equal(rows.at(-1).kind,'🙂'.repeat(32));
  assert.equal(f.document.getText(),'猫🙂\r\nvalue'); assert.equal(f.document.version,1); assert(f.document.isDirty);
});

test('kind getter is captured once and later action getters cannot alter the serialized primitive', async () => {
  let reads=0; const kind=new K('unused');
  Object.defineProperty(kind,'value',{get(){reads++; return reads===1?'quickfix':17;}});
  const later=new K('quickfix'), action=new CodeAction('later getter',later);
  action.disabled={get reason(){later.value=17; return 'disabled';}};
  const f=fixture(()=>[new CodeAction('once',kind),action]);
  const rows=await f.providers.provide(f.request()); assert.equal(reads,1); assert.equal(rows[0].kind,'quickfix');
  assert.equal(rows[1].kind,'quickfix'); assert.equal(later.value,17);
});

test('native kind authorization is detached from mutable public context and methods', async () => {
  const f=fixture((_,__,context)=>{
    context.only.value=''; context.only.contains=()=>true;
    const broad=new K('quickfix'); broad.contains=()=>true;
    return [new CodeAction('outside',broad),new CodeAction('inside',new K('refactor.extract'))];
  });
  const rows=await f.providers.provide(f.request('refactor'));
  assert.equal(rows.length,1); assert.equal(rows[0].title,'inside'); assert.equal(rows[0].kind,'refactor.extract');
});

test('empty native kind requests keep the pinned adapter no-filter context', async () => {
  const f=fixture((_,__,context)=>{assert.equal(context.only,undefined); return [new CodeAction('quick',K.QuickFix),new CodeAction('untyped')];});
  const rows=await f.providers.provide(f.request('')); assert.equal(rows.length,2); assert.equal(rows[0].kind,'quickfix');
});

test('kind getter disposal cannot publish a row or retain lazy resolve handles', async () => {
  const f=fixture(); let registration;
  registration=f.owner.registerCodeActionsProvider('cpp',{provideCodeActions(){
    const kind=new K('quickfix'); Object.defineProperty(kind,'value',{get(){registration.dispose(); return 'quickfix';}});
    return [new CodeAction('retired',kind)];
  },resolveCodeAction:item=>item});
  await assert.rejects(f.providers.provide(f.request()),/stale|canceled/);
  assert.equal(f.providers.retainedActionCount(),0); assert.equal(f.providers.pendingCount(),0);
  assert.deepEqual(f.providers.snapshot(),[]);
});

test('wire kind resource admission precedes UTF8 measurement of an oversized value', () => {
  const {kindValue}=require('./code-actions.cjs'), measure=Buffer.byteLength;
  const kind=new K('x'.repeat(100000)); let measured=false;
  try {
    Buffer.byteLength=()=>{measured=true; throw new Error('must not scan oversized string');};
    assert.throws(()=>kindValue(kind),/128 UTF-8 bytes/); assert.equal(measured,false);
  } finally { Buffer.byteLength=measure; }
});

test('invalid native kind context fails before any extension callback starts', async () => {
  let calls=0; const f=fixture(()=>{calls++;return [];});
  for(const value of [null,17,{},'x'.repeat(129),'🙂'.repeat(33)]) {
    await assert.rejects(f.providers.provide(f.request(value)),/128 UTF-8 bytes/);
    assert.equal(calls,0); assert.equal(f.providers.pendingCount(),0);
  }
});
