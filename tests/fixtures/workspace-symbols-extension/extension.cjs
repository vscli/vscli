'use strict';
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const source=v.window.activeTextEditor?.document,root=v.Uri.file(v.workspace.rootPath);
 const original='猫🙂 source\r\nnext\r\n',targetText='α🙂 target\r\n';
 const calls=[],originals=new WeakSet();let target,hidden,indexed,resolved=0,armedList=false,armedResolve=false,held,capacity=[],capacityReleases=[];
 const uri=name=>v.Uri.joinPath(root,name),info=text=>v.window.showInformationMessage(text);
 const register=(id,fn)=>context.subscriptions.push(v.commands.registerCommand('workspaceSymbols.'+id,fn));
 const points=editor=>editor.selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]);
 function retained(){assert(source);assert.equal(source.getText(),'dirty '+original);assert(source.isDirty);assert(v.workspace.textDocuments.includes(source));if(target){assert.equal(target.getText(),'dirty '+targetText);assert(target.isDirty);}if(hidden)assert.equal(hidden.getText(),'β🙂 hidden\r\n');}
 function symbol(name,kind,container,resource,range,tags){const item=new v.SymbolInformation(name,kind,container,new v.Location(resource,range));if(tags)item.tags=tags;return item;}
 function lazy(query){
  const item=symbol(query==='lazy-resolve'?'lazy-resolve':query==='held-list'?'held-list':'AlphaLazy',v.SymbolKind.Function,'scope',uri('target.cpp'),undefined,[v.SymbolTag.Deprecated]);
  const data={query};data.self=data;item.data=data;originals.add(item);return item;
 }
 function rows(query){
  if(query==='lazy'||query==='lazy-resolve'||query==='held-list')return[lazy(query)];
  if(query==='hidden')return[symbol('Hidden',v.SymbolKind.Variable,'hidden scope',uri('hidden.cpp'),new v.Range(0,1,0,3))];
  return[lazy(query),symbol('Zulu',v.SymbolKind.Class,'z scope',uri('hidden.cpp'),new v.Range(0,1,0,3),[v.SymbolTag.Deprecated]),symbol('Beta',v.SymbolKind.Variable,'b scope',uri('hidden.cpp'),new v.Range(0,0,0,1))];
 }
 function record(provider,query){assert.equal(typeof query,'string');assert(calls.length<128,'bounded workspace fixture callbacks');calls.push({provider,query});}
 function remember(result,query){for(const item of result){originals.add(item);if(!item.data){const data={query};data.self=data;item.data=data;}}return result;}
 async function index(){
  const active=v.window.activeTextEditor,before=active&&points(active),visible=v.window.visibleTextEditors.slice();
  const existing=v.workspace.textDocuments.map(document=>({document,version:document.version,text:document.getText()}));
  let witnessed=false,opened;
  const listener=v.workspace.onDidOpenTextDocument(document=>{
   if(document.uri.toString()!==uri('indexed.cpp').toString())return;
   assert.equal(witnessed,false);assert.equal(document.getText(),'δ🙂 index\r\n');assert.equal(document.languageId,'cpp');
   assert.strictEqual(v.window.activeTextEditor,active);assert.deepEqual(v.window.visibleTextEditors,visible);opened=document;witnessed=true;
  });
  try{indexed=await v.workspace.openTextDocument(uri('indexed.cpp'));}finally{listener.dispose();}
  assert(witnessed,'actual onDidOpen must precede awaited open');assert.strictEqual(indexed,opened);assert(v.workspace.textDocuments.includes(indexed));
  assert.strictEqual(v.window.activeTextEditor,active);assert.deepEqual(v.window.visibleTextEditors,visible);if(active)assert.deepEqual(points(active),before);
  for(const state of existing){assert(v.workspace.textDocuments.includes(state.document));assert.equal(state.document.version,state.version);assert.equal(state.document.getText(),state.text);}
  if(source)retained();else{assert.equal(active,undefined);assert.equal(visible.length,0);}
  return[symbol('IndexTarget',v.SymbolKind.Function,'on-demand index',indexed.uri,new v.Range(0,1,0,3))];
 }
 function hold(phase,token,value){
  assert.equal(held,undefined);let release,cancel;const cancellation=new Promise(resolve=>{cancel=resolve;});
  const promise=new Promise(resolve=>{release=resolve;});held={phase,release:()=>release(value),cancellation,cancelled:false,finished:false};
  token.onCancellationRequested(()=>{held.cancelled=true;cancel();});
  void info('API workspace held '+phase+';entered=true;actualPending=true');
  return promise.then(result=>{held.finished=true;return result;});
 }
 const primary={
  async provideWorkspaceSymbols(query,token){assert.strictEqual(this,primary);assert.equal(token.isCancellationRequested,false);record('primary',query);const result=remember(query==='index'?await index():rows(query),query);if(query==='capacity')return new Promise(resolve=>{capacityReleases.push(()=>resolve(result));if(capacityReleases.length===8)void info('API workspace capacity;actualCallbacks=8;held=true');});if(armedList&&query==='held-list'){armedList=false;return hold('list',token,result);}return result;},
  resolveWorkspaceSymbol(item,token){
   assert.strictEqual(this,primary);assert(originals.has(item),'exact original list object');assert.strictEqual(item.data.self,item.data);assert.equal(token.isCancellationRequested,false);resolved++;
   const result=symbol(item.name,item.kind,item.containerName,item.location.uri,item.location.range||new v.Range(0,7,0,9),item.tags);
   if(armedResolve){armedResolve=false;return hold('resolve',token,result);}return result;
  }
 };
 const primaryRegistration=v.languages.registerWorkspaceSymbolProvider(primary);context.subscriptions.push(primaryRegistration);
 const peer={provideWorkspaceSymbols(query){assert.strictEqual(this,peer);record('peer',query);if(query!=='public')return[];return[
  symbol('AlphaLazy',v.SymbolKind.Function,'scope',uri('target.cpp'),new v.Range(0,0,0,1),[v.SymbolTag.Deprecated]),
  symbol('Beta',v.SymbolKind.Variable,'b scope',uri('hidden.cpp'),new v.Range(0,0,0,1)),
  symbol('Eta',v.SymbolKind.Namespace,'external scope',v.Uri.parse('workspace-preview:/external.cpp'),new v.Range(0,2,0,4))
 ];}};
 const failure={provideWorkspaceSymbols(query){assert.strictEqual(this,failure);record('error',query);throw new Error('intentional isolated workspace provider failure');}};
 const empty={provideWorkspaceSymbols(query){assert.strictEqual(this,empty);record('null',query);return null;}};
 for(const provider of [peer,failure,empty])context.subscriptions.push(v.languages.registerWorkspaceSymbolProvider(provider));
 register('queryZero',async()=>{
  assert.equal(v.window.activeTextEditor,undefined);assert.equal(v.window.visibleTextEditors.length,0);assert.equal(v.workspace.textDocuments.length,0);
  const count=resolved,begin=calls.length,result=await v.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','public');
  assert.deepEqual(new Set(calls.slice(begin).map(call=>call.provider)),new Set(['primary','peer','error','null']));
  assert.deepEqual(result.map(item=>item.name),['AlphaLazy','AlphaLazy','Beta','Eta','Zulu']);assert(result.every(item=>item instanceof v.SymbolInformation));assert.equal(resolved,count,'public query must not eagerly resolve');
  const alpha=result[0];assert.equal(alpha.kind,v.SymbolKind.Function);assert.equal(alpha.containerName,'scope');assert.deepEqual(alpha.tags,[v.SymbolTag.Deprecated]);assert(alpha.location instanceof v.Location);assert.equal(alpha.location.uri.toString(),uri('target.cpp').toString());assert.equal(alpha.location.range,undefined);assert(!originals.has(alpha),'fresh public result');assert.equal(alpha.data,undefined);
  // Resolver capability is a nonzero comparison when either range is absent;
  // both Alpha rows survive in different groups, with lazy ordered first.
  const ranged=result[1];assert.equal(ranged.kind,v.SymbolKind.Function);assert.equal(ranged.containerName,'scope');assert.deepEqual(ranged.tags,[v.SymbolTag.Deprecated]);assert.equal(ranged.location.uri.toString(),alpha.location.uri.toString());assert(ranged.location instanceof v.Location);assert(ranged.location.range instanceof v.Range);assert.deepEqual([ranged.location.range.start.character,ranged.location.range.end.character],[0,1]);
  const beta=result[2];assert.equal(beta.kind,v.SymbolKind.Variable);assert.equal(beta.containerName,'b scope');assert.equal(beta.location.uri.toString(),uri('hidden.cpp').toString());assert.deepEqual([beta.location.range.start.character,beta.location.range.end.character],[0,1]);
  assert.equal(result[3].kind,v.SymbolKind.Namespace);assert.equal(result[3].location.uri.toString(),'workspace-preview:/external.cpp');assert.equal(result[3].containerName,'external scope');assert.deepEqual([result[3].location.range.start.character,result[3].location.range.end.character],[2,4]);
  assert.equal(result[4].kind,v.SymbolKind.Class);assert.equal(result[4].containerName,'z scope');assert.equal(result[4].location.uri.toString(),uri('hidden.cpp').toString());assert.deepEqual(result[4].tags,[v.SymbolTag.Deprecated]);assert.deepEqual([result[4].location.range.start.character,result[4].location.range.end.character],[1,3]);
  assert.equal(v.window.activeTextEditor,undefined);assert.equal(v.workspace.textDocuments.length,0);await info('API workspace public;zeroEditors/fullFields/sort/dedup/isolation=true;resolve=0');
 });
 register('prepare',async()=>{
  retained();target=await v.workspace.openTextDocument(uri('target.cpp'));hidden=await v.workspace.openTextDocument(uri('hidden.cpp'));
  const editor=await v.window.showTextDocument(target,{preview:false,viewColumn:v.ViewColumn.Beside});assert.equal(await editor.edit(builder=>builder.insert(new v.Position(0,0),'dirty ')),true);
  const shared=await v.window.showTextDocument(target,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});assert.notStrictEqual(shared,editor);assert.strictEqual(shared.document,target);
  await v.window.showTextDocument(source,{preview:false,viewColumn:1});retained();await info('API workspace prepared;sharedDirty/hidden=true');
 });
 register('source',async()=>{retained();const editor=await v.window.showTextDocument(source,{preview:false,viewColumn:1});assert.strictEqual(editor.document,source);await info('API workspace source;identity/dirty=true');});
 register('target',()=>{retained();assert.strictEqual(v.window.activeTextEditor.document,target);assert.deepEqual(points(v.window.activeTextEditor),[[0,7,0,7]]);assert(resolved>0);return info('API workspace target;originalResolve/collapsed/shared=true');});
 register('hidden',()=>{retained();assert.strictEqual(v.window.activeTextEditor.document,hidden);assert.deepEqual(points(v.window.activeTextEditor),[[0,1,0,1]]);return info('API workspace hidden;identity/collapsed=true');});
 register('targetUndo',()=>{assert.strictEqual(v.window.activeTextEditor.document,target);assert.equal(target.getText(),targetText);assert.equal(source.getText(),'dirty '+original);return info('API workspace target Undo;exact=true');});
 register('targetRedo',()=>{retained();return info('API workspace target Redo;exact=true');});
 register('sourceUndo',()=>{assert.strictEqual(v.window.activeTextEditor.document,source);assert.equal(source.getText(),original);assert.equal(source.isDirty,false);return info('API workspace source Undo;exact=true');});
 register('sourceRedo',()=>{retained();return info('API workspace source Redo;exact=true');});
 register('armList',()=>{retained();assert.equal(held,undefined);armedList=true;return info('API workspace held list armed');});
 register('armResolve',()=>{retained();assert.equal(held,undefined);armedResolve=true;return info('API workspace held resolve armed');});
 register('release',async()=>{assert(held);await held.cancellation;assert(held.cancelled);held.release();await new Promise(resolve=>setImmediate(resolve));assert(held.finished);const phase=held.phase;held=undefined;return info('API workspace held '+phase+';cancelled/actualSettled=true');});
 register('dispose',async()=>{assert(held);primaryRegistration.dispose();await held.cancellation;assert(held.cancelled);if(source)await v.window.showTextDocument(source,{preview:false,viewColumn:1});return info('API workspace provider disposed;original=retained');});
 register('capacity',()=>{assert.equal(v.window.activeTextEditor,undefined);assert.equal(capacity.length,0);for(let i=0;i<8;i++){const promise=v.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','capacity');promise.catch(()=>{});capacity.push(promise);}});
 register('capacityOverflow',async()=>{assert.equal(capacityReleases.length,8);await assert.rejects(v.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','capacity'));assert.equal(capacityReleases.length,8);return info('API workspace capacity;ninth=refused;actualStill8=true');});
 register('capacityRelease',async()=>{assert.equal(capacityReleases.length,8);capacityReleases.splice(0).forEach(release=>release());const results=await Promise.all(capacity);assert.equal(results.length,8);assert(results.every(result=>result.length===3));const fresh=await v.commands.executeCommand('vscode.executeWorkspaceSymbolProvider','public');assert.deepEqual(fresh.map(item=>item.name),['AlphaLazy','AlphaLazy','Beta','Eta','Zulu']);assert.equal(fresh[0].location.range,undefined);assert.deepEqual([fresh[1].location.range.start.character,fresh[1].location.range.end.character],[0,1]);assert.equal(v.window.activeTextEditor,undefined);assert.equal(v.workspace.textDocuments.length,0);return info('API workspace capacity;originals/fresh=true;actualReleased=true');});
 register('indexReady',()=>{assert(indexed);assert.equal(indexed.getText(),'δ🙂 index\r\n');assert(v.workspace.textDocuments.includes(indexed));if(source){assert.strictEqual(v.window.activeTextEditor.document,source);retained();}else{assert.equal(v.window.activeTextEditor,undefined);assert.equal(v.window.visibleTextEditors.length,0);}return info('API workspace index;hidden/eventBeforeACK/noRetarget=true');});
 register('indexed',()=>{assert.strictEqual(v.window.activeTextEditor.document,indexed);assert.equal(indexed.getText(),'δ🙂 index\r\n');assert.deepEqual(points(v.window.activeTextEditor),[[0,1,0,1]]);if(source)retained();return info('API workspace index;acceptedIdentity/collapsed=true');});
};
