'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {Uri}=require('./api-types.cjs');
function fixture(options={}) {
  let generation=0;
  const calls=[];
  const runtime=createApi((method,params)=>{calls.push({method,params});throw new Error('Unexpected native request');},()=>{}, {session:1,...options});
  const snapshot={id:7,uri:Uri.parse('preview:?revision=1').toString(),version:1,text:'猫🙂\noriginal',languageId:'plaintext',isReadonly:true,isDirty:false};
  const sync=(documents=[snapshot])=>runtime.sync({generation:++generation,documents,active:null,selections:[]});
  sync();
  const api=runtime.forExtension('consumer'),doc=api.workspace.textDocuments[0];
  return {runtime,api,doc,snapshot,sync,calls};
}
test('mirrored provider resources open across owners after provider disposal without invoking or renewing it',async()=>{
  const f=fixture({virtualDocuments:true});let invoked=0,opened=0;
  const registration=f.runtime.forExtension('creator').workspace.registerTextDocumentContentProvider('preview',{provideTextDocumentContent(){invoked++;throw new Error('Cached provider must not run');}});
  registration.dispose();
  f.api.workspace.onDidOpenTextDocument(()=>opened++);
  assert.equal(await f.api.workspace.openTextDocument(Uri.parse(f.snapshot.uri)),f.doc);
  assert.equal(await f.runtime.forExtension('other').workspace.openTextDocument(Uri.parse(f.snapshot.uri)),f.doc);
  assert.deepEqual([invoked,opened,f.calls.length],[0,0,0]);
  assert.equal(f.api.window.activeTextEditor,undefined);assert.deepEqual(f.api.window.visibleTextEditors,[]);
});
test('existing native mirrors need no creation capability, while missing resources still require their backend',async()=>{
  for(const uri of [Uri.parse('preview:?revision=1').toString(),'untitled:/existing.cpp']) {
    const f=fixture({virtualDocuments:false,untitledDocuments:false});f.sync([{...f.snapshot,uri,isReadonly:uri.startsWith('preview:')}]);
    const doc=f.api.workspace.textDocuments[0];
    assert.equal(await f.api.workspace.openTextDocument(Uri.parse(uri)),doc);
    await assert.rejects(f.api.workspace.openTextDocument(Uri.parse(uri+'missing')),/provider|untitled/);
    assert.equal(f.calls.length,0);
  }
});
test('original cached provider identity cannot be replaced during URI geometry getters',async()=>{
  const f=fixture({virtualDocuments:true}),uri=Uri.parse(f.snapshot.uri),input={};
  for(const field of ['scheme','authority','path','query','fragment'])Object.defineProperty(input,field,{get(){
    if(field==='path'){f.sync([]);f.sync();}return uri[field];
  }});
  await assert.rejects(f.api.workspace.openTextDocument(input),/Cached native document mirror was retired/);
  assert(f.doc.isClosed);assert.notEqual(f.api.workspace.textDocuments[0],f.doc);assert.equal(f.calls.length,0);
});
test('a legitimate provider mirror advance returns current text on the original cached model',async()=>{
  const f=fixture({virtualDocuments:true});
  const original=Uri.parse(f.snapshot.uri),input={};
  for(const field of ['scheme','authority','path','query','fragment'])Object.defineProperty(input,field,{get(){
    if(field==='query')f.sync([{...f.snapshot,version:2,text:'updated 猫🙂\n'}]);return original[field];
  }});
  f.doc.uri.path='/public-forgery';f.doc._snapshot={id:99,uri:'preview:/public-forgery',text:'forged',isClosed:true};
  assert.equal(await f.api.workspace.openTextDocument(input),f.doc);
  assert.equal(f.doc.getText(),'updated 猫🙂\n');assert.equal(f.calls.length,0);
});
test('cached custom resources preserve requesting caller and encoding guards',async()=>{
  const f=fixture({virtualDocuments:true}),uri=Uri.parse(f.snapshot.uri);
  await assert.rejects(f.api.workspace.openTextDocument(uri,{encoding:'utf16le'}),/encoding/);
  await assert.rejects(f.api.workspace.openTextDocument(uri,{get encoding(){
    f.runtime.disposeOwner('consumer');f.runtime.forExtension('consumer');return 'utf8';
  }}),/retired/);
  assert.equal(f.calls.length,0);
});
