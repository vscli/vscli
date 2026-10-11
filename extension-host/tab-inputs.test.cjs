'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const types=require('./tab-inputs.cjs');
const {Uri}=require('./api-types.cjs');
test('all seven stable tab input classes preserve exact URI/type fields and constructor identity',()=>{
  const original=Uri.file('/tmp/猫.cpp'),modified=Uri.parse('sample:/modified'),kind={original:true};
  const cases=[['TabInputText',[original],{uri:original}],['TabInputTextDiff',[original,modified],{original,modified}],
    ['TabInputCustom',[original,kind],{uri:original,viewType:kind}],['TabInputWebview',[kind],{viewType:kind}],
    ['TabInputNotebook',[original,kind],{uri:original,notebookType:kind}],['TabInputNotebookDiff',[original,modified,kind],{original,modified,notebookType:kind}],
    ['TabInputTerminal',[],{}]];
  assert.deepEqual(Object.keys(types),cases.map(([name])=>name));
  assert.deepEqual(Object.values(types).map(type=>type.name),['TextTabInput','TextDiffTabInput','CustomEditorTabInput','WebviewEditorTabInput','NotebookEditorTabInput','NotebookDiffEditorTabInput','TerminalEditorTabInput']);
  for(const [name,args,expected]of cases){const value=new types[name](...args);assert(value instanceof types[name]);assert.equal(value.constructor,types[name]);assert.deepEqual({...value},expected);
    for(const key of Object.keys(expected))assert.equal(value[key],expected[key]);}
});
test('parameter properties are mutable, enumerable and unvalidated just as pinned constructors',()=>{
  let reads=0;const input={get uri(){reads++;throw Error('not read');}},value=new types.TabInputText(input);
  assert.equal(value.uri,input);assert.equal(reads,0);assert.equal(Object.isFrozen(value),false);
  const descriptor=Object.getOwnPropertyDescriptor(value,'uri');assert.equal(descriptor.writable,true);assert.equal(descriptor.enumerable,true);assert.equal(descriptor.configurable,true);
  value.uri=null;assert.equal(value.uri,null);assert.equal(new types.TabInputNotebook(undefined,17).notebookType,17);
  assert.equal(new types.TabInputWebview().viewType,undefined);assert.deepEqual(new types.TabInputTerminal(),new types.TabInputTerminal());
});
test('ordinary class inheritance is supported while function-style class invocation still rejects',()=>{
  class Derived extends types.TabInputText {constructor(uri){super(uri);this.extra=1;}}
  const uri=Uri.file('/tmp/a'),value=new Derived(uri);assert(value instanceof Derived);assert(value instanceof types.TabInputText);assert.equal(value.uri,uri);
  assert.throws(()=>types.TabInputText.call({},uri),/class constructor/i);
});
