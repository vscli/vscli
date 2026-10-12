'use strict';
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const source=v.window.activeTextEditor.document,root=v.Uri.file(v.workspace.rootPath);
 const sourceText='猫🙂 source\r\nnext\r\n',targetText='α🙂 target\r\n';
 const targetNames={early:'early.cpp',wildcard:'wildcard.cpp',third:'third.cpp'};
 const targets=new Map(),calls=[];let rust,thirdRegistration;
 const resource=name=>v.Uri.joinPath(root,name);
 const register=(name,fn)=>context.subscriptions.push(v.commands.registerCommand('navigationRegistry.'+name,fn));
 const info=text=>v.window.showInformationMessage(text);
 const points=editor=>editor.selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]);
 function retained(){
  assert.equal(source.getText(),'dirty '+sourceText);assert(source.isDirty);
  assert(v.workspace.textDocuments.includes(source));
  for(const [name,doc] of targets)assert.equal(doc.getText(),(name==='wildcard'?'dirty ':'')+targetText);
 }
 function provider(name,selector){
  const object={provideDefinition(document,position,token){
   assert.strictEqual(this,object);assert(v.workspace.textDocuments.includes(document));
   assert(position instanceof v.Position);assert.equal(token.isCancellationRequested,false);
   assert(calls.length<64,'bounded registry callback evidence');calls.push({name,document});
   const start=name==='wildcard'?7:1;
   return [new v.Location(resource(targetNames[name]),new v.Range(0,start,0,start+2))];
  }};
  const disposable=v.languages.registerDefinitionProvider(selector,object);
  context.subscriptions.push(disposable);return disposable;
 }
 // Pinned LanguageFeatureRegistry scans its CURRENT ordered entries before
 // sorting. These selectors deliberately have different cpp scores (10/5).
 provider('early',{language:'cpp',exclusive:true});
 const wildcardRegistration=provider('wildcard',{language:'*',exclusive:true});
 async function showSource(){const editor=await v.window.showTextDocument(source,{preview:false,viewColumn:1});assert.strictEqual(editor.document,source);retained();}
 async function query(document,expected){
  retained();const active=v.window.activeTextEditor,before=points(active),version=source.version;
  const begin=calls.length,result=await v.commands.executeCommand('vscode.executeDefinitionProvider',document.uri,new v.Position(0,0));
  assert.deepEqual(calls.slice(begin).map(call=>call.name),expected===null?[]:[expected]);
  assert.equal(result.length,expected===null?0:1);
  if(expected!==null){assert(result[0] instanceof v.Location);assert.equal(result[0].uri.toString(),resource(targetNames[expected]).toString());const start=expected==='wildcard'?7:1;assert.deepEqual([result[0].range.start.character,result[0].range.end.character],[start,start+2]);}
  assert.strictEqual(v.window.activeTextEditor,active);assert.deepEqual(points(active),before);assert.equal(source.version,version);retained();
  await info('API registry query;'+document.languageId+'='+String(expected)+';inactive/noEdit=true');
 }
 register('prepare',async()=>{
  rust=await v.workspace.openTextDocument(resource('candidate.rs'));assert.equal(rust.languageId,'rust');
  for(const name of ['early','wildcard','third'])targets.set(name,await v.workspace.openTextDocument(resource(targetNames[name])));
  const target=targets.get('wildcard'),editor=await v.window.showTextDocument(target,{preview:false,viewColumn:v.ViewColumn.Beside});
  assert.equal(await editor.edit(builder=>builder.insert(new v.Position(0,0),'dirty ')),true);
  const shared=await v.window.showTextDocument(target,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});assert.notStrictEqual(shared,editor);assert.strictEqual(shared.document,target);
  await showSource();await info('API registry prepared;shared/dirty=true');
 });
 register('source',async()=>{await showSource();await info('API registry source;retained=true');});
 for(const [suffix,name]of [['Early','early'],['Wildcard','wildcard'],['Third','third']]){
  register('queryCpp'+suffix,()=>query(source,name));
  register('native'+suffix,()=>{
   retained();assert.strictEqual(v.window.activeTextEditor.document,targets.get(name));
   assert.deepEqual(points(v.window.activeTextEditor),[[0,name==='wildcard'?7:1,0,name==='wildcard'?7:1]]);
   assert.equal(calls.at(-1).name,name);assert.strictEqual(calls.at(-1).document,source);
   return info('API registry native;winner='+name+';collapsed/shared=true');
  });
 }
 register('queryRustWildcard',()=>query(rust,'wildcard'));
 register('queryRustNone',()=>query(rust,null));
 register('registerThird',async()=>{assert.equal(thirdRegistration,undefined);thirdRegistration=provider('third',{language:'cpp',exclusive:true});await showSource();await info('API registry third registered;source=retained');});
 register('disposeWildcard',async()=>{wildcardRegistration.dispose();await showSource();await info('API registry wildcard disposed;source=retained');});
 register('disposeThird',async()=>{thirdRegistration.dispose();await showSource();await info('API registry third disposed;source=retained');});
 register('targetClean',()=>{assert.strictEqual(v.window.activeTextEditor.document,targets.get('wildcard'));assert.equal(targets.get('wildcard').getText(),targetText);assert.equal(source.getText(),'dirty '+sourceText);return info('API registry target Undo;exact=true');});
 register('targetDirty',()=>{retained();return info('API registry target Redo;exact=true');});
 register('sourceClean',()=>{assert.strictEqual(v.window.activeTextEditor.document,source);assert.equal(source.getText(),sourceText);assert.equal(source.isDirty,false);return info('API registry source Undo;exact=true');});
 register('sourceDirty',()=>{retained();return info('API registry source Redo;exact=true');});
};
