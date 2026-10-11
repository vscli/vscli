const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const first=v.window.activeTextEditor,document=first.document,original='猫🙂 base\r\nnext\r\n',dirty='dirty '+original;
 const expanded='猫-猫-🙂'+dirty,typed='x-x-🙂'+dirty,single='q-x-🙂'+dirty;
 let second,changes=0,optionsDocument,optionsEditor;
 const changeCounts=new Map();
 const register=(id,fn)=>context.subscriptions.push(v.commands.registerCommand('editorSnippets.'+id,fn));
 context.subscriptions.push(v.workspace.onDidChangeTextDocument(event=>{
  changeCounts.set(event.document,(changeCounts.get(event.document)||0)+1);
  if(event.document===document)changes++;
 }));
 const points=editor=>editor.selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]);
 function source(text){assert.strictEqual(first.document,document);if(second)assert.strictEqual(second.document,document);assert.equal(document.getText(),text);assert.equal(document.isDirty,text!==original);}
 async function insert(editor,snippet,where,text,options){
  const before=changes,version=document.version;
  const result=options===undefined?await editor.insertSnippet(snippet,where):await editor.insertSnippet(snippet,where,options);
  assert.equal(result,true);assert.strictEqual(v.window.activeTextEditor,editor);source(text);
  assert(document.version>version);assert(changes>before,'native changed mirror/event precedes true');
 }
 register('linked',async()=>{
  source(dirty);
  const snippet=new v.SnippetString().appendPlaceholder('猫',1).appendText('-').appendTabstop(1).appendText('-').appendPlaceholder('🙂',2).appendTabstop(0);
  await insert(first,snippet,new v.Position(0,0),expanded);
  assert.deepEqual(points(first),[[0,0,0,1],[0,2,0,3]]);
  await v.window.showInformationMessage('API snippets linked;builder=true;events=true;UTF16=true');
 });
 register('typed',()=>{source(typed);assert.deepEqual(points(first),[[0,1,0,1],[0,3,0,3]]);return v.window.showInformationMessage('API snippets typed;linked=true');});
 register('field2',()=>{source(typed);assert.deepEqual(points(first),[[0,4,0,6]]);return v.window.showInformationMessage('API snippets Tab;field=2;UTF16=true');});
 register('back',()=>{source(typed);assert.deepEqual(points(first),[[0,0,0,1],[0,2,0,3]]);return v.window.showInformationMessage('API snippets BackTab;field=1;linked=true');});
 register('escaped',()=>{source(typed);assert.deepEqual(points(first),[[0,0,0,1]]);return v.window.showInformationMessage('API snippets Escape;secondary=removed');});
 register('q',()=>{source(single);return v.window.showInformationMessage('API snippets escaped typing;firstOnly=true');});
 register('textTyped',()=>{source(typed);return v.window.showInformationMessage('API snippets typed Undo;source=true');});
 register('expanded',()=>{source(expanded);return v.window.showInformationMessage('API snippets expansion Undo;source=true');});
 register('dirty',()=>{source(dirty);return v.window.showInformationMessage('API snippets dirty history;source=true');});
 register('clean',()=>{source(original);return v.window.showInformationMessage('API snippets original Undo;clean=true');});
 register('inactive',async()=>{
  source(dirty);const firstBefore=points(first);
  second=await v.window.showTextDocument(document,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});
  assert.notStrictEqual(second,first);assert.strictEqual(v.window.activeTextEditor,first);
  await insert(second,new v.SnippetString('${1:tail}$0'),new v.Range(1,0,1,4),'dirty 猫🙂 base\r\ntail\r\n');
  assert.deepEqual(points(first),firstBefore);assert.deepEqual(points(second),[[1,0,1,4]]);
  await v.window.showInformationMessage('API snippets inactive;focus=second;session=target;events=true');
 });
 register('first',async()=>{
  assert.strictEqual(await v.window.showTextDocument(document,{preview:false,viewColumn:1}),first);
  assert.strictEqual(v.window.activeTextEditor,first);assert.deepEqual(points(first),[[0,6,0,6]]);
  source('dirty 猫🙂 base\r\ntail\r\n');await v.window.showInformationMessage('API snippets first pane;selection=original');
 });
 register('overloads',async()=>{
  assert.strictEqual(await v.window.showTextDocument(document,{preview:false,viewColumn:2}),second);
  await insert(second,new v.SnippetString('L'),undefined,'dirty 猫🙂 base\r\nL\r\n');
  await insert(second,new v.SnippetString('E'),[],'dirty 猫🙂 base\r\nLE\r\n');
  await insert(second,new v.SnippetString('B'),new v.Position(1,0),'dirty 猫🙂 base\r\nBLE\r\n');
  await insert(second,new v.SnippetString('R'),new v.Range(1,0,1,3),'dirty 猫🙂 base\r\nR\r\n');
  await insert(second,new v.SnippetString('P'),[new v.Position(1,0),new v.Position(0,0)],'Pdirty 猫🙂 base\r\nPR\r\n');
  assert.deepEqual(points(second),[[0,1,0,1],[1,1,1,1]]);
  await insert(second,new v.SnippetString('A'),[],'PAdirty 猫🙂 base\r\nPAR\r\n');
  await insert(second,new v.SnippetString('M'),[new v.Range(1,0,1,2),new v.Range(0,0,0,2)],'Mdirty 猫🙂 base\r\nMR\r\n');
  assert.deepEqual(points(second),[[0,1,0,1],[1,1,1,1]]);
  await v.window.showInformationMessage('API snippets overloads;omitted/empty/Position/Range/arrays=true');
 });
 register('arrayUndo',()=>{source('PAdirty 猫🙂 base\r\nPAR\r\n');return v.window.showInformationMessage('API snippets array Undo;both=true');});
 register('arrayFinal',()=>{source('Mdirty 猫🙂 base\r\nMR\r\n');return v.window.showInformationMessage('API snippets array Redo;both=true');});
 register('reentrant',async()=>{
  source(dirty);let editing,reads=0,result,refused=false;
  const snippet={get value(){reads++;editing=first.edit(edit=>edit.insert(new v.Position(0,0),'stale '));return 'FORBIDDEN';}};
  try{result=await first.insertSnippet(snippet,new v.Position(0,0));}catch(error){refused=true;}
  assert.equal(reads,1);assert.equal(await editing,true);assert(refused||result===false);
  assert.strictEqual(await v.window.showTextDocument(document,{preview:false,viewColumn:1}),first);
  source('stale '+dirty);await v.window.showInformationMessage('API snippets stale getter;insert=refused;edit=retained');
 });
 const optionsInitial='  tail\r\n';
 function optionsSource(text){
  assert.strictEqual(optionsEditor.document,optionsDocument);assert.strictEqual(v.window.activeTextEditor,optionsEditor);
  assert.equal(optionsDocument.getText(),text);assert.equal(optionsDocument.isUntitled,true);assert.equal(optionsDocument.isDirty,true);source(dirty);
 }
 async function optionsInsert(template,where,text,options){
  const before=changeCounts.get(optionsDocument)||0,version=optionsDocument.version;
  const result=options===undefined?await optionsEditor.insertSnippet(new v.SnippetString(template),where)
   :await optionsEditor.insertSnippet(new v.SnippetString(template),where,options);
  assert.equal(result,true);optionsSource(text);assert(optionsDocument.version>version);
  assert((changeCounts.get(optionsDocument)||0)>before,'options native mirror/event precedes true');
 }
 register('optionsPrepare',async()=>{
  source(dirty);const before=points(first);
  optionsDocument=await v.workspace.openTextDocument({language:'cpp',content:optionsInitial});
  optionsEditor=await v.window.showTextDocument(optionsDocument,{preview:false,viewColumn:v.ViewColumn.Beside});
  optionsSource(optionsInitial);assert.deepEqual(points(first),before);
  await v.window.showInformationMessage('API snippets options;separateUntitled=true;CRLF=true');
 });
 register('optionsDefault',async()=>{
  optionsSource(optionsInitial);
  await optionsInsert('first\n  child$0',new v.Position(0,2),'  first\r\n    childtail\r\n');
  assert.deepEqual(points(optionsEditor),[[1,9,1,9]]);
  await v.window.showInformationMessage('API snippets default whitespace;childIndent=4;events=true');
 });
 register('optionsInitial',()=>{
  optionsSource(optionsInitial);
  return v.window.showInformationMessage('API snippets options Undo;initial=true;dirty=true');
 });
 register('optionsKeep',async()=>{
  optionsSource(optionsInitial);
  await optionsInsert('first\n  child$0',new v.Position(0,2),'  first\r\n  childtail\r\n',
   {keepWhitespace:true,undoStopBefore:true,undoStopAfter:true});
  assert.deepEqual(points(optionsEditor),[[1,7,1,7]]);
  await v.window.showInformationMessage('API snippets keepWhitespace;childIndent=2;events=true');
 });
 register('optionsGrouped',async()=>{
  optionsSource(optionsInitial);
  const options={undoStopBefore:false,undoStopAfter:false};
  await optionsInsert('A',new v.Position(0,2),'  Atail\r\n',options);
  await optionsInsert('B',new v.Position(0,3),'  ABtail\r\n',options);
  assert.deepEqual(points(optionsEditor),[[0,4,0,4]]);
  await v.window.showInformationMessage('API snippets false stops;twoInsertions=true;openBoundary=true');
 });
 register('optionsTyped',()=>{
  optionsSource('  ABCtail\r\n');
  return v.window.showInformationMessage('API snippets false stops typing;ABC=true');
 });
 register('optionsOriginal',async()=>{
  optionsSource(optionsInitial);
  assert.strictEqual(await v.window.showTextDocument(document,{preview:false,viewColumn:1}),first);
  source(dirty);assert.deepEqual(points(first),[[0,6,0,6]]);
  await v.window.showInformationMessage('API snippets options protected original;identity=true;history=true');
 });
 register('retiredReadonly',async()=>{
  source(dirty);const before=changes,version=document.version;
  const provider=v.workspace.registerTextDocumentContentProvider('snippet-readonly',{provideTextDocumentContent(){return 'readonly 猫🙂\r\n';}});context.subscriptions.push(provider);
  const virtual=await v.workspace.openTextDocument(v.Uri.parse('snippet-readonly:/sample.cpp'));
  const editor=await v.window.showTextDocument(virtual,{preview:false,viewColumn:2});
  await assert.rejects(()=>second.insertSnippet(new v.SnippetString('FORBIDDEN')));
  assert.equal(await editor.insertSnippet(new v.SnippetString('FORBIDDEN')),false);
  assert.equal(virtual.getText(),'readonly 猫🙂\n');assert.equal(virtual.isDirty,false);assert.equal(document.version,version);assert.equal(changes,before);source(dirty);
  assert.strictEqual(await v.window.showTextDocument(document,{preview:false,viewColumn:1}),first);
  await v.window.showInformationMessage('API snippets retired=reject;readonly=false;source=unchanged');
 });
};
