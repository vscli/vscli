'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createProviderOrdering}=require('./provider-order.cjs');
const {selector}=require('./language-selector.cjs');
const source=(language,uri='file:///work/file.cpp',patch={})=>({languageId:language,uri,...patch});
const entry=(name,value)=>({name,selector:selector(value)});
const names=rows=>rows.map(value=>value.name);
test('initial exclusive winner is first current entry even when a newer exclusive scores higher',()=>{
  const registry=createProviderOrdering(),older=entry('older-wild',{language:'*',exclusive:true}),newer=entry('newer-cpp',{language:'cpp',exclusive:true});
  registry.register(older);registry.register(newer);
  assert.deepEqual(names(registry.ordered(source('cpp'))),['older-wild']);
});
test('query A reorders current entries and changes later B winner; A-B-A retains that actual history',()=>{
  const registry=createProviderOrdering(),older=entry('older-cpp',{language:'cpp',exclusive:true}),newer=entry('newer-wild',{language:'*',exclusive:true});
  registry.register(older);registry.register(newer);
  assert.deepEqual(names(registry.ordered(source('cpp'))),['older-cpp']);
  assert.deepEqual(names(registry.ordered(source('rust','file:///work/a.rs'))),['newer-wild']);
  assert.deepEqual(names(registry.ordered(source('cpp'))),['newer-wild']);
  assert.deepEqual(names(registry.ordered(source('rust','file:///work/a.rs'))),['newer-wild']);
});
test('registration appends after a prior sort; disposal removes exact identity and preserves remaining current order',()=>{
  const registry=createProviderOrdering(),cpp=entry('cpp',{language:'cpp',exclusive:true}),wild=entry('wild',{language:'*',exclusive:true}),latest=entry('latest',{language:'*',exclusive:true});
  registry.register(cpp);registry.register(wild);registry.ordered(source('rust'));
  registry.register(latest);assert.deepEqual(names(registry.ordered(source('cpp'))),['wild']);
  assert.equal(registry.remove({...wild}),false);assert.equal(registry.remove(wild),true);assert.equal(registry.remove(wild),false);
  assert.deepEqual(names(registry.ordered(source('cpp'))),['latest']);
});
test('cached same candidate does not rescore after text versions; URI and language changes do rescore',()=>{
  const registry=createProviderOrdering(),provider=entry('provider',{language:'*'});let reads=0;
  Object.defineProperty(provider.selector[0],'language',{get(){reads++;return '*';}});
  registry.register(provider);registry.ordered(source('cpp'));const first=reads;assert(first>0);
  registry.ordered(source('cpp','file:///work/file.cpp',{version:200,languageEpoch:99}));assert.equal(reads,first);
  registry.ordered(source('cpp','file:///work/other.cpp'));assert(reads>first);const second=reads;
  registry.ordered(source('rust','file:///work/other.cpp'));assert(reads>second);
});
test('ordinary score ties prefer latest registration and trusted nonbuiltin metadata; user flags have no authority',()=>{
  const registry=createProviderOrdering(),older=entry('older','cpp'),newer=entry('newer','cpp'),builtin=entry('builtin','cpp');
  registry.register(older);registry.register(newer);registry.register(builtin,true);
  assert.deepEqual(names(registry.ordered(source('cpp'))),['newer','older','builtin']);
  const exact=entry('exact','cpp'),wild=entry('wild','*'),scores=createProviderOrdering();scores.register(exact);scores.register(wild);
  assert.deepEqual(names(scores.ordered(source('cpp'))),['exact','wild']);
});
test('mixed exclusive arrays do not suppress; recursive source queries ignore all-exclusive providers',()=>{
  const registry=createProviderOrdering(),ordinary=entry('ordinary','cpp'),mixed=entry('mixed',[{language:'cpp',exclusive:true},{scheme:'file'}]),exclusive=entry('exclusive',[{language:'cpp',exclusive:true}]);
  registry.register(ordinary);registry.register(mixed);assert.deepEqual(names(registry.ordered(source('cpp'))),['mixed','ordinary']);
  registry.register(exclusive);assert.deepEqual(names(registry.ordered(source('cpp'))),['exclusive']);
  assert.deepEqual(names(registry.ordered(source('cpp'),true)),['mixed','ordinary']);
  assert.deepEqual(names(registry.ordered(source('cpp'))),['exclusive']);
});
test('capacity, clock and duplicate refusal do not change retained providers or cached order',()=>{
  const registry=createProviderOrdering(),first=entry('first','cpp');registry.register(first);
  assert.throws(()=>registry.register(first),/already registered/);assert.equal(registry.size(),1);
  for(let index=1;index<128;index++)registry.register(entry(String(index),'cpp'));
  const before=registry.ordered(source('cpp'));
  assert.throws(()=>registry.register(entry('overflow','cpp')),/128/);assert.deepEqual(registry.ordered(source('cpp')),before);
  const exhausted=createProviderOrdering({initialClock:Number.MAX_SAFE_INTEGER-1}),last=entry('last','cpp');exhausted.register(last);
  assert.throws(()=>exhausted.register(entry('overflow','cpp')),/clock exhausted/);assert.deepEqual(exhausted.ordered(source('cpp')),[last]);
});
test('trusted ranking matches private URI metadata without extension-replaced fsPath or file-constructor callbacks',()=>{
  const {createProviders}=require('./providers.cjs'),{Uri,TextDocument}=require('./api-types.cjs');
  const doc=new TextDocument({id:1,uri:'file:///work/a.cpp',version:1,languageId:'cpp',text:'a'}),source={model:doc,document:1,uri:'file:///work/a.cpp',version:1,languageId:'cpp',languageEpoch:0,isClosed:false};
  const providers=createProviders({session:1,document:()=>doc,documentSource:()=>source,notify:()=>{},track:(_,value)=>value});
  providers.forOwner('creator').registerDefinitionProvider({pattern:'**/*.cpp'},{provideDefinition(){throw Error('ranking invoked provider');}});
  const prototype=Object.getPrototypeOf(Uri.parse('file:///work/a.cpp')),descriptor=Object.getOwnPropertyDescriptor(prototype,'fsPath'),file=Uri.file;let calls=0;
  try{
    Object.defineProperty(prototype,'fsPath',{configurable:true,get(){calls++;throw Error('extension fsPath invoked');}});
    Uri.file=()=>{calls++;throw Error('extension constructor invoked');};
    providers.forOwner('creator').registerDefinitionProvider({pattern:{base:'/work',pattern:'*.cpp'}},{provideDefinition(){throw Error('ranking invoked provider');}});
    assert.deepEqual(providers.rank({session:1,type:'definition',document:1,version:1,languageEpoch:0}),{providers:[{id:2,owner:'creator'},{id:1,owner:'creator'}]});
    assert.equal(calls,0);
  }finally{Object.defineProperty(prototype,'fsPath',descriptor);Uri.file=file;}
});
