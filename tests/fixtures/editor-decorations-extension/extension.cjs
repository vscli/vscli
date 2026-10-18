const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const first=v.window.activeTextEditor,document=first.document;
 const original='猫🙂 alpha\r\nsecond beta\r\n',dirty='dirty '+original;
 let second,primary,secondary,whole,stale;
 const register=(id,fn)=>context.subscriptions.push(v.commands.registerCommand('editorDecorations.'+id,fn));
 function source(text=dirty){assert.equal(document.getText(),text);assert.equal(document.isDirty,text!==original);assert.strictEqual(first.document,document);if(second)assert.strictEqual(second.document,document);}
 function range(end=15){return new v.Range(0,10,0,end);}
 async function barrier(){const shown=await v.window.showTextDocument(document,{preview:false,viewColumn:1});assert.strictEqual(shown,first);assert.strictEqual(v.window.activeTextEditor,first);}
 register('shared',async()=>{
  source();second=await v.window.showTextDocument(document,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});
  assert.notStrictEqual(second,first);assert.strictEqual(second.document,document);assert.strictEqual(v.window.activeTextEditor,first);
  primary=v.window.createTextEditorDecorationType({color:'#12ab34',backgroundColor:'#2345ab',fontWeight:'bold',rangeBehavior:v.DecorationRangeBehavior.OpenOpen});
  secondary=v.window.createTextEditorDecorationType({color:'#d98712',fontStyle:'italic',textDecoration:'underline line-through',rangeBehavior:v.DecorationRangeBehavior.ClosedClosed});
  whole=v.window.createTextEditorDecorationType({backgroundColor:'#122334',isWholeLine:true});
  context.subscriptions.push(primary,secondary,whole);
  first.setDecorations(primary,[range()]);second.setDecorations(secondary,[{range:range(),hoverMessage:new v.MarkdownString('Hover 猫🙂')}]);
  first.setDecorations(whole,[new v.Range(1,0,1,1)]);await barrier();source();
  await v.window.showInformationMessage('API decorations shared;views=independent;source=unchanged');
 });
 register('replace',async()=>{first.setDecorations(primary,[range(12)]);await barrier();source();await v.window.showInformationMessage('API decorations replacement;oldRange=removed;second=retained');});
 register('clear',async()=>{first.setDecorations(primary,[]);await barrier();source();await v.window.showInformationMessage('API decorations cleared;first=empty;second=retained');});
 register('dispose',async()=>{
  primary.dispose();primary.dispose();secondary.dispose();whole.dispose();
  assert.throws(()=>first.setDecorations(primary,[range()]));assert.throws(()=>first.setDecorations({key:secondary.key},[range()]));
  await barrier();source();await v.window.showInformationMessage('API decorations disposed;idempotent=true;forgery=refused');
 });
 register('armStart',async()=>{
  source();let resolve,reject;
  const done=new Promise((yes,no)=>{resolve=yes;reject=no;});
  const sub=v.window.onDidChangeTextEditorSelection(event=>{
   if(event.textEditor!==first)return;sub.dispose();
   try{assert.deepEqual(event.selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]),[[0,10,0,10]]);resolve();}catch(error){reject(error);}
  });
  first.selection=new v.Selection(0,10,0,10);await done;assert.strictEqual(v.window.activeTextEditor,first);
  await v.window.showInformationMessage('API decorations native start armed');
 });
 register('forcedInsert',async()=>{
  source();assert.equal(await first.edit(edit=>edit.insert(new v.Position(0,10),'Z')),true);await barrier();
  source('dirty 猫🙂 Zalpha\r\nsecond beta\r\n');assert.strictEqual(v.window.activeTextEditor,first);
  await v.window.showInformationMessage('API decorations forced insert;edit=true;shared=true');
 });
 register('inserted',()=>{source('dirty 猫🙂 Zalpha\r\nsecond beta\r\n');return v.window.showInformationMessage('API decorations native insertion;source=true;shared=true');});
 register('dirty',()=>{source();return v.window.showInformationMessage('API decorations undo;dirtyBaseline=preserved');});
 register('clean',()=>{source(original);return v.window.showInformationMessage('API decorations original undo;clean=true');});
 register('retire',async()=>{
  const other=await v.workspace.openTextDocument(v.Uri.joinPath(v.Uri.file(v.workspace.rootPath),'b.txt'));
  const replacement=await v.window.showTextDocument(other,{preview:false,viewColumn:2,preserveFocus:true});
  assert.notStrictEqual(replacement,second);assert.strictEqual(v.window.activeTextEditor,first);
  assert.throws(()=>second.setDecorations(secondary,[range()]));assert.throws(()=>first.setDecorations({key:primary.key},[range()]));
  assert.equal(other.getText(),'other\r\n');assert.equal(other.isDirty,false);source();
  await v.window.showInformationMessage('API decorations retired;setter=refused;replacement=untouched');
 });
 register('reentrant',async()=>{
  source();stale=v.window.createTextEditorDecorationType({color:'#a142f5',fontWeight:'bold'});context.subscriptions.push(stale);
  let editing,reads=0;
  first.setDecorations(stale,[{get range(){reads++;editing=first.edit(edit=>edit.insert(new v.Position(0,0),'stale '));return range();}}]);
  assert.equal(reads,1);assert.equal(await editing,true);await barrier();source('stale '+dirty);
  await v.window.showInformationMessage('API decorations reentrant edit;staleRequest=issued;edit=true');
 });
 register('reentrantUndo',()=>{source();return v.window.showInformationMessage('API decorations reentrant undo;original=retained');});
};
