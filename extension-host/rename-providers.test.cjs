'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {Uri,Range,Position}=require('./api-types.cjs');
const {WorkspaceEdit}=require('./edit-values.cjs');
function fixture(){
  const frames=[],notices=[],runtime=createApi((method,params)=>{frames.push({method,params});throw Error('Unexpected native request');},(method,params)=>notices.push({method,params}),{session:7,languageProviders:true,workspaceEdits:true});
  runtime.configure('/work',{},[]);
  const state={id:1,uri:Uri.file('/work/a.cpp').toString(),version:3,text:'dirty 猫🙂 source\r\nnext\r\n',eol:2,languageId:'cpp',languageEpoch:1,isDirty:true};
  let generation=0;const sync=()=>runtime.sync({generation:++generation,documents:[{...state}],editors:[],activeEditor:null});sync();
  const api=runtime.forExtension('consumer'),creator=runtime.forExtension('creator'),at=new Position(0,11);
  return{runtime,api,creator,state,sync,frames,notices,at,prepare:()=>api.commands.executeCommand('vscode.prepareRename',Uri.parse(state.uri),at),execute:name=>api.commands.executeCommand('vscode.executeDocumentRenameProvider',Uri.parse(state.uri),at,name)};
}
test('public rename prepares again, preserves successful provider index and returns fresh query-only edits',async()=>{
  const f=fixture(),seen=[],word=new Range(0,10,0,16);let original;
  f.creator.languages.registerRenameProvider('*',{prepareRename(){seen.push('prepare1');return {range:word,placeholder:'second'};},provideRenameEdits(doc,at,name){seen.push('edit1');assert.equal(doc,f.api.workspace.textDocuments[0]);assert(at instanceof Position);original=new WorkspaceEdit();original.replace(doc.uri,word,name);return original;}});
  f.creator.languages.registerRenameProvider('cpp',{prepareRename(){seen.push('prepare0');return null;},provideRenameEdits(){throw Error('must skip earlier provider');}});
  assert.equal((await f.prepare()).placeholder,'second');
  const edit=await f.execute('');assert(edit instanceof WorkspaceEdit);assert.notEqual(edit,original);assert.equal(edit.get(Uri.parse(f.state.uri))[0].newText,'');
  assert.deepEqual(seen,['prepare0','prepare1','prepare0','prepare1','edit1']);assert.equal(f.frames.length,0);assert.equal(f.api.workspace.textDocuments[0].getText(),f.state.text);
});
test('fallback uses captured text, aggregates typed rejection, and accepts empty unknown-brand results',async()=>{
  const f=fixture(),doc=f.api.workspace.textDocuments[0],registrations=[];
  registrations.push(f.creator.languages.registerRenameProvider('*',{provideRenameEdits(){return {};}}));
  registrations.push(f.creator.languages.registerRenameProvider('cpp',{prepareRename(){return {defaultBehavior:true};},provideRenameEdits(){return null;}}));
  doc.getText=()=> 'forged';doc.getWordRangeAtPosition=()=>new Range(0,0,0,1);
  assert.equal((await f.prepare()).placeholder,'source');assert.equal((await f.execute('new')).size,0);
  registrations.forEach(value=>value.dispose());
  f.creator.languages.registerRenameProvider('cpp',{prepareRename(){throw new Error('first');},provideRenameEdits(){throw Error('must not run');}});
  f.creator.languages.registerRenameProvider('*',{prepareRename(){throw 'second';},provideRenameEdits(){throw Error('must not run');}});
  await assert.rejects(f.execute('new'),/first\nsecond/);
});
test('actual ignored cancellation retains occupied callback until creator settlement and sends exact release',async()=>{
  const f=fixture();let resolve,entered,canceled;
  const ready=new Promise(done=>entered=done),cancelled=new Promise(done=>canceled=done),wait=new Promise(done=>resolve=done);
  f.creator.languages.registerRenameProvider('cpp',{prepareRename(doc,at,token){token.onCancellationRequested(canceled);entered();return wait;},provideRenameEdits(){return new WorkspaceEdit();}});
  const provider=f.runtime.providerSnapshot()[0].id;
  const params={session:7,owner:'creator',provider,request:31,document:1,version:3,position:{line:0,character:11}};
  const pending=f.runtime.prepareLanguageRename(params);await ready;
  f.runtime.cancelLanguageProvider(params);await cancelled;await assert.rejects(pending,/canceled/);
  assert(f.runtime.languageProviderPending(31));assert.equal(f.notices.filter(value=>value.method==='languageProviderReleased').length,0);
  resolve(new Range(0,10,0,16));await new Promise(done=>setImmediate(done));
  assert(!f.runtime.languageProviderPending(31));assert.deepEqual(f.notices.filter(value=>value.method==='languageProviderReleased').map(value=>value.params.request),[31]);
});
test('query input getters cannot substitute a newer mirrored source',async()=>{
  const f=fixture();let invoked=0;
  f.creator.languages.registerRenameProvider('cpp',{provideRenameEdits(){invoked++;return new WorkspaceEdit();}});
  const at={get line(){f.state.version++;f.state.text='replacement';f.sync();return 0;},character:0};
  await assert.rejects(f.api.commands.executeCommand('vscode.executeDocumentRenameProvider',Uri.parse(f.state.uri),at,'new'),/changed/);assert.equal(invoked,0);assert.equal(f.frames.length,0);
});

test('generic thrown values and empty Error messages reject instead of becoming successful word fallback',async()=>{
  for(const error of [{opaque:true},new Error(''),'']){
    const f=fixture();
    f.creator.languages.registerRenameProvider('cpp',{prepareRename(){throw error;},provideRenameEdits(){throw Error('must not invoke after generic prepare failure');}});
    await assert.rejects(f.prepare());await assert.rejects(f.execute('new'));assert.equal(f.frames.length,0);
  }
});
