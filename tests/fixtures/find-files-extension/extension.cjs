const v=require('vscode'),assert=require('node:assert/strict'),path=require('node:path');
exports.activate=context=>{
 const document=v.window.activeTextEditor.document;
 const original='猫🙂 A\r\nlast\r\n',dirty='dirty '+original;
 const all=['ignored/hidden.rs','src/excluded.rs','src/keep.rs','src/sub/deep.rs'];
 const register=(id,callback)=>context.subscriptions.push(v.commands.registerCommand('find.'+id,callback));
 function paths(uris){
  assert(Array.isArray(uris));
  const names=uris.map(uri=>{
   assert.equal(uri.scheme,'file');
   const relative=path.relative(v.workspace.rootPath,uri.fsPath).replace(/\\/g,'/');
   assert(!relative.startsWith('../')&&!path.isAbsolute(relative));assert(all.includes(relative));return relative;
  });
  assert.equal(new Set(names).size,names.length);return names.sort();
 }
 function identity(expected){
  assert.strictEqual(v.window.activeTextEditor.document,document);
  assert(v.workspace.textDocuments.includes(document));assert.equal(document.getText(),expected);
  assert.equal(document.isDirty,expected===dirty);
 }
 async function protectedSearch(callback){
  identity(dirty);const version=document.version,selection=v.window.activeTextEditor.selection;
  await callback();identity(dirty);assert.equal(document.version,version);
  assert.deepEqual(v.window.activeTextEditor.selection,selection);
 }
 register('query',async()=>{
  await protectedSearch(async()=>{
   // Default reads files.exclude; search.exclude must not filter keep.rs.
   assert.deepEqual(paths(await v.workspace.findFiles('**/*.rs')),['src/keep.rs','src/sub/deep.rs']);
   assert.deepEqual(paths(await v.workspace.findFiles('**/*.rs',null)),all);
   // An explicit pattern is additive to configured files.exclude.
   assert.deepEqual(paths(await v.workspace.findFiles('**/*.rs','**/sub/**')),['src/keep.rs']);
   const relative=new v.RelativePattern(v.Uri.joinPath(v.Uri.file(v.workspace.rootPath),'src'),'**/*.rs');
   assert.deepEqual(paths(await v.workspace.findFiles(relative,null)),['src/excluded.rs','src/keep.rs','src/sub/deep.rs']);
   const limited=paths(await v.workspace.findFiles('**/*.rs',null,1));assert.equal(limited.length,1);
   assert.deepEqual(await v.workspace.findFiles('**/*.rs',null,0),[]);
  });
  await v.window.showInformationMessage('API find scopes=true;relative=true;limits=true');
 });
 register('cancel',async()=>{
  await protectedSearch(async()=>{
   const before=new v.CancellationTokenSource();before.cancel();
   try{assert.deepEqual(await v.workspace.findFiles('**/*.rs',null,undefined,before.token),[]);}finally{before.dispose();}
   // Pre-cancel has no native request; a fresh traversal is valid immediately.
   assert.deepEqual(paths(await v.workspace.findFiles('**/*.rs',null)),all);
   const active=new v.CancellationTokenSource();
   try{
    const pending=v.workspace.findFiles('**/*.rs',null,undefined,active.token);
    active.cancel();assert.deepEqual(await pending,[]);
   }finally{active.dispose();}
   // Active cancellation resolves [] before joined actual capacity release.
   // Do not assert immediate native lane reuse here.
  });
  await v.window.showInformationMessage('API find cancelled=true;pre-fresh=true');
 });
 register('dirty',()=>{identity(dirty);return v.window.showInformationMessage('API search retained dirty identity');});
 register('clean',()=>{identity(original);return v.window.showInformationMessage('API search retained clean identity');});
};
