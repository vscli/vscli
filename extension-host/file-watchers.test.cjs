'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const {createFileWatchers} = require('./file-watchers.cjs');
const {createFileSystem} = require('./filesystem.cjs');
const {Uri,Disposable,EventEmitter} = require('./api-types.cjs');
const {RelativePattern} = require('./workspace-files.cjs');
function fixture() {
  let root='/workspace', settings={}, alive=true;
  const starts=[],owned=[];
  const manager=createFileWatchers({captureCaller:()=>()=>{if(!alive)throw new Error('retired');},
    workspaceRoot:()=>root,configuration:()=>({get:(_key,fallback)=>settings || fallback}),
    event:(_owner,event)=>event,track:(_owner,item)=>{owned.push(item);return item;},
    fileSystem:{pathCaseSensitive:()=>true},
    localWatch:(uri,options,emit,error)=>{const child={uri,options,emit,error,closed:false,
      dispose(){this.closed=true;}};starts.push(child);return child;},
  });
  return {manager,api:manager.forOwner('A'),starts,owned,setRoot:value=>{root=value;},
    setSettings:value=>{settings=value;},retire:()=>{alive=false;}};
}
test('workspace strings filter paths and preserve independent create/change/delete flags',()=>{
  const f=fixture(),watch=f.api.createFileSystemWatcher('**/*.rs',false,true,false),seen=[];
  assert.equal(watch.ignoreCreateEvents,false);assert.equal(watch.ignoreChangeEvents,true);
  watch.onDidCreate(uri=>seen.push(['create',uri.path]));watch.onDidChange(uri=>seen.push(['change',uri.path]));
  watch.onDidDelete(uri=>seen.push(['delete',uri.path]));
  for(const [type,file] of [[2,'/workspace/a.rs'],[1,'/workspace/a.rs'],[3,'/workspace/a.rs'],[2,'/outside/a.rs'],[2,'/workspace/a.txt']])
    f.starts[0].emit({type,uri:Uri.file(file)});
  assert.deepEqual(seen,[['create','/workspace/a.rs'],['delete','/workspace/a.rs']]);
  watch.dispose();f.starts[0].emit({type:2,uri:Uri.file('/workspace/b.rs')});assert.equal(seen.length,2);
  assert(f.starts[0].closed);watch.dispose();
});
test('outside relative and file bases are snapshotted, nonrecursive watches bypass excludes',()=>{
  const f=fixture(),base=Uri.file('/outside'),pattern=new RelativePattern(base,'*.rs');
  const watch=f.api.createFileSystemWatcher(pattern),seen=[];watch.onDidCreate(uri=>seen.push(uri.path));
  pattern.base='/different';pattern.pattern='*.txt';
  assert.equal(f.starts[0].uri.path,'/outside');assert.equal(f.starts[0].options.recursive,false);
  assert.deepEqual(f.starts[0].options.excludes,[]);
  f.starts[0].emit({type:2,uri:Uri.file('/outside/a.rs')});f.starts[0].emit({type:2,uri:Uri.file('/outside/sub/a.rs')});
  assert.deepEqual(seen,['/outside/a.rs']);watch.dispose();
  const file=f.api.createFileSystemWatcher(new RelativePattern(Uri.file('/outside/a.rs'),'*'));
  let changed=0;file.onDidChange(()=>changed++);f.starts[1].emit({type:1,uri:Uri.file('/outside/a.rs')});
  assert.equal(changed,1);file.dispose();
});
test('configured exclusions refresh backend while stale callbacks and disposed owners stay inert',()=>{
  const f=fixture(),watch=f.api.createFileSystemWatcher('**/*'),seen=[];watch.onDidCreate(uri=>seen.push(uri.path));
  f.setSettings({'**/ignored/**':true});f.manager.refresh();assert.equal(f.starts.length,2);assert(f.starts[0].closed);
  f.starts[0].emit({type:2,uri:Uri.file('/workspace/stale')});
  f.starts[1].emit({type:2,uri:Uri.file('/workspace/ignored/a')});f.starts[1].emit({type:2,uri:Uri.file('/workspace/good')});
  assert.deepEqual(seen,['/workspace/good']);f.manager.disposeOwner('A');assert(f.starts[1].closed);
  f.starts[1].emit({type:2,uri:Uri.file('/workspace/late')});assert.equal(seen.length,1);
});
test('all ignored events and empty workspace create no watch and reentrant retirement cannot admit work',()=>{
  const f=fixture();f.api.createFileSystemWatcher('**/*',true,true,true);assert.equal(f.starts.length,0);
  f.setRoot(undefined);const empty=f.api.createFileSystemWatcher('**/*');assert.equal(f.starts.length,0);
  f.setRoot('/next');f.manager.refresh();assert.equal(f.starts.length,1);empty.dispose();
  assert.throws(()=>f.api.createFileSystemWatcher({baseUri:Uri.file('/x'),get pattern(){f.retire();return '*';}}),/retired/);
  assert.equal(f.starts.length,1);
});
test('registered filesystem provider receives watch options and original disposal; other schemes never leak',()=>{
  const events=new EventEmitter(),calls=[];
  const provider={onDidChangeFile:events.event,watch(uri,options){calls.push(['watch',uri.path,options]);return new Disposable(()=>calls.push(['dispose']));},
    stat(){},readDirectory(){},createDirectory(){},readFile(){},writeFile(){},delete(){},rename(){}};
  const fs=createFileSystem(),registration=fs.forOwner('provider').registerFileSystemProvider('mem',provider,{isCaseSensitive:true});
  const manager=createFileWatchers({captureCaller:()=>()=>{},track:(_o,d)=>d,event:(_o,e)=>e,
    workspaceRoot:()=>'/workspace',configuration:()=>({get:()=>({})}),fileSystem:fs});
  const watcher=manager.forOwner('consumer').createFileSystemWatcher(new RelativePattern(Uri.parse('mem:/outside'),'*.rs'));
  const seen=[];watcher.onDidCreate(uri=>seen.push(uri.toString()));
  events.fire([{type:2,uri:Uri.parse('mem:/outside/a.rs')},{type:1,uri:Uri.parse('mem:/outside/other.txt')}]);
  assert.deepEqual(seen,['mem:/outside/a.rs']);assert.equal(calls[0][2].recursive,false);
  registration.dispose();assert.deepEqual(calls.at(-1),['dispose']);
  events.fire([{type:2,uri:Uri.parse('mem:/outside/late.rs')}]);assert.equal(seen.length,1);watcher.dispose();
});

test('selected facade callbacks retain consumer lease; failed provider cleanup closes original watch',()=>{
  const {createApi}=require('./api.cjs');
  const active=new Set(['provider','consumer']);
  const runtime=createApi(async()=>({}),()=>{}, {session:31});
  runtime.setActivation({accepts:owner=>active.has(owner),facade:()=>({})});runtime.configure('/workspace',[],[]);
  const providerApi=runtime.forExtension('provider'),consumer=runtime.forExtension('consumer');
  const events=new EventEmitter();let disposed=0,watchOwner,seen=0;
  const provider={onDidChangeFile:events.event,watch(){watchOwner=runtime.currentCallerOwner();return new Disposable(()=>disposed++);},
    stat(){},readDirectory(){},createDirectory(){},readFile(){},writeFile(){},delete(){},rename(){}};
  runtime.runWithOwner('provider',()=>providerApi.workspace.registerFileSystemProvider('scoped',provider,{isCaseSensitive:true}));
  const watcher=runtime.runWithOwner('consumer',()=>consumer.workspace.createFileSystemWatcher(new RelativePattern(Uri.parse('scoped:/base'),'*.rs')));
  watcher.onDidCreate(()=>{assert.equal(runtime.currentCallerOwner(),'consumer');seen++;});
  assert.equal(watchOwner,'provider');events.fire([{type:2,uri:Uri.parse('scoped:/base/a.rs')}]);assert.equal(seen,1);
  active.delete('provider');runtime.disposeOwner('provider');assert.equal(disposed,1);
  watcher.dispose();assert.equal(disposed,1);runtime.disposeOwner('consumer');
});

test('same-root watchers share one OS backend and last disposal owns shutdown',()=>{
  const f=fixture(),a=f.api.createFileSystemWatcher('**/*.rs'),b=f.api.createFileSystemWatcher('**/*.txt');
  assert.equal(f.starts.length,1);let seen=0;b.onDidCreate(()=>seen++);
  a.dispose();assert(!f.starts[0].closed);f.starts[0].emit({type:2,uri:Uri.file('/workspace/a.txt')});
  assert.equal(seen,1);b.dispose();assert(f.starts[0].closed);
});

test('failed old shared session cannot deliver events or delete a replacement session',()=>{
  const f=fixture(),a=f.api.createFileSystemWatcher('**/*.rs');let seen=0;
  a.onDidCreate(()=>seen++);f.starts[0].error(new Error('OS watch failed'));
  const b=f.api.createFileSystemWatcher('**/*.rs');b.onDidCreate(()=>seen++);
  a.dispose();const c=f.api.createFileSystemWatcher('**/*.txt');assert.equal(f.starts.length,2);
  f.starts[0].emit({type:2,uri:Uri.file('/workspace/stale.rs')});assert.equal(seen,0);
  f.starts[1].emit({type:2,uri:Uri.file('/workspace/current.rs')});assert.equal(seen,1);
  b.dispose();assert(!f.starts[1].closed);c.dispose();assert(f.starts[1].closed);
});
