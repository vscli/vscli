'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),path=require('node:path');
const {Uri}=require('./api-types.cjs');
const {RelativePattern}=require('./workspace-files.cjs');
const {selector,score}=require('./language-selector.cjs');
const root=path.join(path.sep,'work'),uri=Uri.file(path.join(root,'src','猫.cpp')),document={languageId:'cpp',uri};
test('source scores exact and wildcard independently with conjunctive mismatch and max-array semantics',()=>{
  for(const [value,expected]of [['cpp',10],['*',5],['rust',0],[{},0],[{scheme:'file'},10],[{language:'*',scheme:'file'},10],
    [{language:'cpp',scheme:'untitled'},0],[{language:'',scheme:''},0],[[{language:'rust'},'*'],5],[{language:'cpp',scheme:'*'},10]])assert.equal(score(selector(value),document),expected);
});
test('full path glob braces classes globstar and patterns-only filters use the pinned matcher',()=>{
  for(const value of ['**/*.cpp','**/{src,test}/[!a]*.cpp','**/猫.?pp',uri.fsPath])assert.equal(score(selector({pattern:value}),document),10,value);
  assert.equal(score(selector({language:'rust',pattern:'**/*.cpp'}),document),0);
  assert.equal(score(selector({pattern:'**/*.rs'}),document),0);
  assert.equal(score(selector({pattern:''}),document),0);
});
test('RelativePattern confines basename matching to exact root boundary and supports outside roots',()=>{
  assert.equal(score(selector({pattern:new RelativePattern(Uri.file(root),'**/*.cpp')}),document),10);
  assert.equal(score(selector({pattern:new RelativePattern(Uri.file(path.join(root,'s')),'**/*.cpp')}),document),0);
  assert.equal(score(selector({pattern:{base:root,pattern:'src/*.cpp'}}),document),10);
  const elsewhere=Uri.file(path.join(path.sep,'outside','x.cpp'));
  assert.equal(score(selector({pattern:new RelativePattern(Uri.file(path.join(path.sep,'outside')),'*.cpp')}),{languageId:'cpp',uri:elsewhere}),10);
});
test('scheme filters match untitled and actual virtual schemes with effective language',()=>{
  for(const scheme of ['untitled','notes']){
    const candidate={uri:Uri.parse(`${scheme}:/sample.cpp`),languageId:'python'};
    assert.equal(score(selector({language:'python',scheme,pattern:'**/*.cpp'}),candidate),10);
    assert.equal(score(selector({language:'cpp',scheme}),candidate),0);
  }
});
test('known selector getters are captured once and unknown filter getters are not evaluated',()=>{
  const counts={};const value={};
  for(const [key,item]of Object.entries({language:'cpp',scheme:'file',pattern:'**/*.cpp',notebookType:'',exclusive:false}))Object.defineProperty(value,key,{get(){counts[key]=(counts[key]||0)+1;return item;}});
  Object.defineProperty(value,'unknown',{get(){throw Error('unknown getter read');}});
  const captured=selector(value);assert.equal(score(captured,document),10);assert.equal(score(captured,document),10);
  assert.deepEqual(counts,{language:1,scheme:1,pattern:1,notebookType:1,exclusive:1});
});
test('notebook and resource bounds refuse without invoking late pattern getters',()=>{
  assert.throws(()=>selector({notebookType:'*'}),/notebook backend/);
  assert.throws(()=>selector(Array(33).fill('cpp')),/32 entries/);
  assert.throws(()=>selector({pattern:'x'.repeat(4097)}),/over-budget/);
  assert.throws(()=>selector({scheme:'a'.repeat(129)}),/over-budget/);
  assert.throws(()=>selector({pattern:'**/\0'}),/over-budget/);
});
test('RelativePattern exact base equality matches the empty suffix when the original glob permits it',()=>{
  assert.equal(score(selector({pattern:new RelativePattern(uri,'*')}),document),10);
  assert.equal(score(selector({pattern:new RelativePattern(uri,'*.cpp')}),document),0);
});
