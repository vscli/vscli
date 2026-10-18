'use strict';
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const original=v.window.activeTextEditor.document,ordinary='猫🙂 tabs\r\nlast\r\n',dirty='dirty '+ordinary;
 const root=v.Uri.file(v.workspace.rootPath),groups=v.window.tabGroups;
 const initialGroup=groups.activeTabGroup,initialTab=initialGroup.activeTab;
 const tabEvents=[],groupEvents=[],saves=[],lifecycle=[];
 let sharedGroup,sharedTab,b,c,bTab,cTab,peer,peerEditor,peerGroup,untitled,untitledTab,untitledGroup;
 const register=(id,fn)=>context.subscriptions.push(v.commands.registerCommand('editorTabs.'+id,fn));
 context.subscriptions.push(groups.onDidChangeTabs(event=>{assert(Object.isFrozen(event));tabEvents.push(event);}));
 context.subscriptions.push(groups.onDidChangeTabGroups(event=>{assert(Object.isFrozen(event));groupEvents.push(event);}));
 context.subscriptions.push(v.workspace.onDidSaveTextDocument(document=>{saves.push(document);lifecycle.push(['save',document]);}));
 context.subscriptions.push(v.workspace.onDidOpenTextDocument(document=>lifecycle.push(['open',document])));
 context.subscriptions.push(v.workspace.onDidCloseTextDocument(document=>lifecycle.push(['close',document])));
 const tabs=()=>groups.all.flatMap(group=>group.tabs);
 const tabFor=(doc,group=groups.activeTabGroup)=>group.tabs.find(tab=>tab.input instanceof v.TabInputText&&tab.input.uri.toString()===doc.uri.toString());
 const info=text=>v.window.showInformationMessage(text);
 function source(text){assert.equal(original.getText(),text);assert.equal(original.isDirty,text!==ordinary);}
 function inventory(){
  assert.strictEqual(v.window.tabGroups,groups);assert(Object.isFrozen(groups));assert(Object.isFrozen(groups.all));
  assert.equal(groups.all.filter(group=>group.isActive).length,1);assert(groups.all.includes(groups.activeTabGroup));
  for(const group of groups.all){
   assert(Object.isFrozen(group));assert(Object.isFrozen(group.tabs));assert(group.viewColumn>=1&&group.viewColumn<=4);
   assert.equal(group.tabs.filter(tab=>tab.isActive).length,group.tabs.length?1:0);
   assert.equal(group.activeTab,group.tabs.find(tab=>tab.isActive));
   for(const tab of group.tabs){assert(Object.isFrozen(tab));assert.strictEqual(tab.group,group);assert(tab.input instanceof v.TabInputText);assert(tab.input.uri instanceof v.Uri);}
  }
 }
 function closed(before,target){assert(tabEvents.slice(before).some(event=>event.closed.includes(target)),'closed tab event before true');assert(!tabs().includes(target));}
 function peerSource(){assert.equal(peer.getText(),'peer other\r\n');assert(peer.isDirty);assert.strictEqual(v.window.activeTextEditor,peerEditor);}
 register('setup',async()=>{
  source(dirty);inventory();assert.strictEqual(initialTab,tabFor(original));assert.equal(initialTab.label,'a.cpp');assert(initialTab.isDirty);assert.equal(initialTab.isPreview,false);assert.equal(initialTab.isPinned,false);
  const savedAll=groups.all,savedTabs=initialGroup.tabs;
  assert.throws(()=>savedTabs.push(initialTab));assert.throws(()=>savedAll.push(initialGroup));assert.throws(()=>Object.assign(initialTab,{label:'forged'}));assert.throws(()=>Object.assign(initialGroup,{viewColumn:99}));
  const uri=original.uri,other=v.Uri.joinPath(root,'b.txt');
  for(const [value,expected] of [
   [new v.TabInputText(uri),{uri}], [new v.TabInputTextDiff(uri,other),{original:uri,modified:other}],
   [new v.TabInputCustom(uri,'fixture.custom'),{uri,viewType:'fixture.custom'}],
   [new v.TabInputWebview('fixture.webview'),{viewType:'fixture.webview'}],
   [new v.TabInputNotebook(uri,'fixture.notebook'),{uri,notebookType:'fixture.notebook'}],
   [new v.TabInputNotebookDiff(uri,other,'fixture.notebook'),{original:uri,modified:other,notebookType:'fixture.notebook'}],
   [new v.TabInputTerminal(),{}]
  ])for(const [key,expectedValue]of Object.entries(expected))assert.strictEqual(value[key],expectedValue);
  const before=tabEvents.length;
  b=await v.workspace.openTextDocument(v.Uri.joinPath(root,'b.txt'));await v.window.showTextDocument(b,{preview:false,viewColumn:1});bTab=tabFor(b);
  c=await v.workspace.openTextDocument(v.Uri.joinPath(root,'c.txt'));await v.window.showTextDocument(c,{preview:false,viewColumn:1});cTab=tabFor(c);
  assert.deepEqual(initialGroup.tabs,[initialTab,bTab,cTab]);assert.deepEqual(savedTabs,[initialTab]);assert.deepEqual(savedAll,[initialGroup]);
  assert(tabEvents.slice(before).some(event=>event.opened.includes(bTab)));assert(tabEvents.slice(before).some(event=>event.opened.includes(cTab)));
  await v.window.showTextDocument(original,{preview:false,viewColumn:1});
  const groupBefore=groupEvents.length;
  const editor=await v.window.showTextDocument(original,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});
  assert.strictEqual(editor.document,original);assert.equal(editor.viewColumn,2);assert.equal(v.window.activeTextEditor.viewColumn,1);
  sharedGroup=groups.all.find(group=>group.viewColumn===2);sharedTab=tabFor(original,sharedGroup);
  assert.notStrictEqual(sharedTab,initialTab);assert.equal(sharedTab.input.uri.toString(),initialTab.input.uri.toString());
  assert(groupEvents.slice(groupBefore).some(event=>event.opened.includes(sharedGroup)));
  assert.strictEqual(groups.activeTabGroup,initialGroup);assert.strictEqual(initialGroup.activeTab,initialTab);inventory();
  await info('API tabs inventory;readonly=true;inputs=7;sharedMemberships=true;events=true');
 });
 register('history',()=>{
  source(dirty);assert(initialGroup.tabs.includes(initialTab));assert(sharedGroup.tabs.includes(sharedTab));assert(initialTab.isDirty&&sharedTab.isDirty);inventory();
  return info('API tabs dirty shared history;identities=true');
 });
 register('clean',()=>{source(ordinary);assert.equal(initialTab.isDirty,false);if(groups.all.includes(sharedGroup))assert.equal(sharedTab.isDirty,false);inventory();return info('API tabs clean history;identities=true');});
 register('pinned',()=>{assert.equal(initialTab.isPinned,true);assert.equal(initialTab.isPreview,false);return info('API tabs pinned;sticky=true;preview=false');});
 register('unpinned',()=>{assert.equal(initialTab.isPinned,false);assert.equal(initialTab.isPreview,false);return info('API tabs unpinned;committed=true');});
 register('closeClean',async()=>{
  source(dirty);const active=v.window.activeTextEditor,before=tabEvents.length;
  assert.equal(await groups.close([bTab,cTab],true),true);closed(before,bTab);closed(before,cTab);assert.strictEqual(v.window.activeTextEditor,active);
  assert.deepEqual(initialGroup.tabs,[initialTab]);assert(sharedGroup.tabs.includes(sharedTab));source(dirty);
  const beforeTabs=tabEvents.length,beforeGroups=groupEvents.length;
  assert.equal(await groups.close(sharedGroup,true),true);closed(beforeTabs,sharedTab);
  assert(groupEvents.slice(beforeGroups).some(event=>event.closed.includes(sharedGroup)),'closed group event before true');
  assert.strictEqual(groups.activeTabGroup,initialGroup);assert.deepEqual(groups.all,[initialGroup]);inventory();
  const counts=[tabEvents.length,groupEvents.length];
  assert.equal(await groups.close([],true),true);assert.deepEqual([tabEvents.length,groupEvents.length],counts);
  await assert.rejects(()=>groups.close(bTab));await assert.rejects(()=>groups.close(sharedGroup));
  await assert.rejects(()=>groups.close({...initialTab}));await assert.rejects(()=>groups.close([initialTab,initialGroup]));
  assert.deepEqual([tabEvents.length,groupEvents.length],counts);assert.deepEqual(initialGroup.tabs,[initialTab]);source(dirty);
  await info('API tabs close clean;array/group=true;arrayPreserveFocus=true;retired/fake=refused;empty=true');
 });
 register('welcome',async()=>{
  source(ordinary);const before=tabEvents.length;
  assert.equal(await groups.close(initialGroup),true);closed(before,initialTab);
  inventory();assert.equal(groups.all.length,1);assert.equal(groups.activeTabGroup.tabs.length,0);assert.equal(groups.activeTabGroup.activeTab,undefined);
  assert.equal(v.window.visibleTextEditors.length,0);assert.equal(v.window.activeTextEditor,undefined);
  await info('API tabs welcome;oneEmptyGroup=true;activeTab=undefined;editors=0');
 });
 register('peer',async()=>{
  source(dirty);peer=await v.workspace.openTextDocument(v.Uri.joinPath(root,'b.txt'));
  peerEditor=await v.window.showTextDocument(peer,{preview:false,viewColumn:v.ViewColumn.Beside});
  assert.equal(await peerEditor.edit(edit=>edit.insert(new v.Position(0,0),'peer ')),true);peerGroup=groups.activeTabGroup;peerSource();inventory();
  await info('API tabs protected peer;dirty=true;origin=inactive');
 });
 register('cancel',async()=>{
  source(dirty);peerSource();const before=tabEvents.length,beforeSaves=saves.length;
  assert.equal(await groups.close(initialTab,true),false);assert.equal(saves.length,beforeSaves);assert(!tabEvents.slice(before).some(event=>event.closed.includes(initialTab)));
  assert(initialGroup.tabs.includes(initialTab));assert.strictEqual(groups.activeTabGroup,peerGroup);peerSource();source(dirty);
  await info('API tabs dirty Cancel=false;source/history=retained;peer=unchanged');
 });
 register('discard',async()=>{
  peerSource();const before=tabEvents.length,beforeSaves=saves.length;
  assert.equal(await groups.close(initialTab,true),true);closed(before,initialTab);assert.equal(saves.length,beforeSaves);peerSource();inventory();
  await assert.rejects(()=>groups.close(initialTab));await info('API tabs dirty Discard=true;events=true;peer=unchanged');
 });
 register('saved',async()=>{
  source(dirty);peerSource();const before=tabEvents.length,beforeSaves=saves.length;
  assert.equal(await groups.close(initialTab,true),true);closed(before,initialTab);
  assert(saves.slice(beforeSaves).some(doc=>doc.uri.toString()===v.Uri.joinPath(root,'a.cpp').toString()&&doc.getText()===dirty),'actual didSave precedes close true');
  peerSource();inventory();await info('API tabs dirty Save=true;receipt/event=true;peer=unchanged');
 });
 register('original',async()=>{
  await v.window.showTextDocument(original,{preview:false,viewColumn:initialGroup.viewColumn});
  assert.strictEqual(v.window.activeTextEditor.document,original);assert.strictEqual(groups.activeTabGroup,initialGroup);assert.strictEqual(initialGroup.activeTab,initialTab);source(dirty);
  await info('API tabs original retained;identity/history=true');
 });
 register('dirty',()=>{source(dirty);assert(initialTab.isDirty);inventory();return info('API tabs original Redo;dirty=true');});
 register('showPeer',async()=>{
  assert.strictEqual(await v.window.showTextDocument(peer,{preview:false,viewColumn:peerGroup.viewColumn}),peerEditor);peerSource();
  await info('API tabs peer active;identity=true');
 });
 register('reopenDiscarded',async()=>{
  peerSource();const reopened=await v.workspace.openTextDocument(v.Uri.joinPath(root,'a.cpp'));
  assert.equal(reopened.getText(),ordinary);assert.equal(reopened.isDirty,false);
  const editor=await v.window.showTextDocument(reopened,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});
  assert.strictEqual(editor.document,reopened);peerSource();inventory();
  await info('API tabs discarded reopened;diskBaseline=true;peer=unchanged');
 });
 register('peerClean',()=>{assert.strictEqual(v.window.activeTextEditor,peerEditor);assert.equal(peer.getText(),'other\r\n');assert.equal(peer.isDirty,false);return info('API tabs peer Undo;clean=true');});
 register('peerDirty',()=>{peerSource();return info('API tabs peer Redo;dirty=true');});
 register('untitled',async()=>{
  peerSource();untitled=await v.workspace.openTextDocument({language:'cpp',content:'猫🙂 new\r\ntail\r\n'});
  const editor=await v.window.showTextDocument(untitled,{preview:false,viewColumn:v.ViewColumn.Beside});assert.strictEqual(editor.document,untitled);assert(untitled.isDirty&&untitled.isUntitled);
  untitledGroup=groups.activeTabGroup;untitledTab=tabFor(untitled);assert(untitledTab);inventory();
  await info('API tabs untitled ready;dirty=true;identity=true');
 });
 register('cancelUntitled',async()=>{
  const before=tabEvents.length,beforeSaves=saves.length,uri=untitled.uri.toString();
  assert.equal(await groups.close(untitledTab),false);assert(untitledGroup.tabs.includes(untitledTab));assert.equal(untitled.uri.toString(),uri);assert.equal(untitled.getText(),'猫🙂 new\r\ntail\r\n');assert(untitled.isDirty&&!untitled.isClosed);
  assert.equal(saves.length,beforeSaves);assert(!tabEvents.slice(before).some(event=>event.closed.includes(untitledTab)));
  await info('API tabs untitled SaveAs Cancel=false;identity/source=true');
 });
 register('saveUntitled',async()=>{
  const beforeSaves=saves.length,oldUri=untitled.uri.toString(),beforeLife=lifecycle.length;
  assert.equal(await groups.close(untitledTab),true);assert(!tabs().includes(untitledTab));
  const saved=saves.slice(beforeSaves).find(doc=>doc.uri.toString()===v.Uri.joinPath(root,'tabs-saved.cpp').toString());
  assert(saved,'new file didSave before close true');assert.notStrictEqual(saved,untitled);assert.equal(saved.getText(),'猫🙂 new\r\ntail\r\n');
  assert.equal(untitled.uri.toString(),oldUri);assert.equal(untitled.isClosed,true);assert.equal(untitled.isDirty,false);
  const relevant=lifecycle.slice(beforeLife);const close=relevant.findIndex(([type,doc])=>type==='close'&&doc===untitled),open=relevant.findIndex(([type,doc])=>type==='open'&&doc===saved),save=relevant.findIndex(([type,doc])=>type==='save'&&doc===saved);
  assert(close>=0&&open>close&&save>open,'SaveAs old close/new open/save order');
  assert.equal(peer.getText(),'peer other\r\n');assert(peer.isDirty);inventory();
  await info('API tabs untitled SaveAs=true;close/open/save=true;peer=unchanged');
 });
};
