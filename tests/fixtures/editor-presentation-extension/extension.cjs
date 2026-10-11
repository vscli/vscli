const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const first=v.window.activeTextEditor,document=first.document;
 const original='猫🙂 header\r\n'+Array.from({length:96},(_,i)=>'    line '+String(i+1).padStart(2,'0')+' 猫🙂\r\n').join('');
 const dirty='dirty '+original,tabbed='dirty   '+original;
 let second,optionsWait,rangesWait,resize;
 const optionEvents=[],rangeEvents=[];
 const register=(id,callback)=>context.subscriptions.push(v.commands.registerCommand('editorPresentation.'+id,callback));
 const points=editor=>editor.selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]);
 const rangePoints=editor=>editor.visibleRanges.map(r=>[r.start.line,r.start.character,r.end.line,r.end.character]);
 const optionState=editor=>{const o=editor.options;return {tabSize:o.tabSize,indentSize:o.indentSize,insertSpaces:o.insertSpaces,cursorStyle:o.cursorStyle,lineNumbers:o.lineNumbers};};
 context.subscriptions.push(v.window.onDidChangeTextEditorOptions(event=>{
  optionEvents.push({editor:event.textEditor,options:{...event.options}});
  if(optionsWait&&event.textEditor===optionsWait.editor){
   const waiter=optionsWait;optionsWait=undefined;
   try{assert.deepEqual(optionState(event.textEditor),waiter.expected);for(const [key,value] of Object.entries(waiter.expected))assert.equal(event.options[key],value);waiter.resolve();}
   catch(error){waiter.reject(error);}
  }
 }));
 context.subscriptions.push(v.window.onDidChangeTextEditorVisibleRanges(event=>{
  rangeEvents.push({editor:event.textEditor,ranges:event.visibleRanges.map(r=>[r.start.line,r.start.character,r.end.line,r.end.character])});
  if(rangesWait&&event.textEditor===rangesWait.editor){const waiter=rangesWait;rangesWait=undefined;waiter.resolve(event);}
  // Resize retires the old measured frame. Wait for the real replacement
  // viewport, rather than treating its temporary unknown [] as dimensions.
  if(resize&&event.textEditor===second&&event.visibleRanges.length>0){
   const before=resize;resize=undefined;
   try{assert.notDeepEqual(rangePoints(second),before.ranges);unchangedPresentation(before);validateRanges(second);void v.window.showInformationMessage('API presentation resized;event=true;selections=preserved');}
   catch(error){void v.window.showErrorMessage('Presentation resize failed: '+String(error.message).slice(0,4096));}
  }
 }));
 function source(text=dirty){assert.strictEqual(first.document,document);if(second)assert.strictEqual(second.document,document);assert.equal(document.getText(),text);assert.equal(document.isDirty,text!==original);}
 function capturePresentation(){return {active:v.window.activeTextEditor,first:points(first),second:points(second),ranges:rangePoints(second)};}
 function unchangedPresentation(before){assert.strictEqual(v.window.activeTextEditor,before.active);assert.deepEqual(points(first),before.first);assert.deepEqual(points(second),before.second);source();}
 function validateRanges(editor){
  assert(editor.visibleRanges.length>0,'actual drawn editor has public ranges');
  for(const range of editor.visibleRanges){assert(range instanceof v.Range);assert.deepEqual(document.validateRange(range),range);assert(range.start.isBeforeOrEqual(range.end));}
 }
 function changeOptions(editor,value,expected,immediate,single){
  assert.equal(optionsWait,undefined);let resolve,reject;const done=new Promise((yes,no)=>{resolve=yes;reject=no;});
  optionsWait={editor,expected,resolve,reject};const before=optionEvents.length;
  try{if(single)editor.options[single]=value;else editor.options=value;if(immediate)assert.deepEqual(optionState(editor),immediate);assert.equal(optionEvents.length,before,'no synthetic synchronous options event');}
  catch(error){optionsWait=undefined;throw error;}
  return done;
 }
 async function reveal(line,type,label){
  assert.equal(rangesWait,undefined);const before=capturePresentation();let resolve;
  const done=new Promise(yes=>{resolve=yes;});rangesWait={editor:second,resolve};
  second.revealRange(new v.Range(line,0,line,5),type);assert.deepEqual(rangePoints(second),before.ranges,'reveal does not fabricate immediate viewport');
  const event=await done;assert.strictEqual(event.textEditor,second);validateRanges(second);unchangedPresentation(before);
  assert(second.visibleRanges.some(r=>r.start.line<=line&&r.end.line>=line),'requested logical line is painted');
  if(type===v.TextEditorRevealType.AtTop)assert.equal(second.visibleRanges[0].start.line,line);
  await v.window.showInformationMessage('API presentation reveal '+label+';event=true;inactive=true;selections=preserved');
 }
 register('unrendered',()=>{source();assert.deepEqual(first.visibleRanges,[]);return v.window.showInformationMessage('API presentation unrendered;ranges=0');});
 register('rendered',()=>{source();validateRanges(first);assert.equal(first.visibleRanges[0].start.line,0);assert(first.visibleRanges.at(-1).end.line<document.lineCount-1);return v.window.showInformationMessage('API presentation rendered;ranges=actual;UTF16=true');});
 register('shared',async()=>{
  const before=points(first);second=await v.window.showTextDocument(document,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});
  assert.notStrictEqual(second,first);assert.strictEqual(second.document,document);assert.strictEqual(v.window.activeTextEditor,first);assert.deepEqual(points(first),before);source();
  await v.window.showInformationMessage('API presentation shared;identity=true;focus=first');
 });
 async function explicit(){
  const firstBefore=optionState(first),before=capturePresentation();
  const expected={tabSize:4,indentSize:2,insertSpaces:true,cursorStyle:2,lineNumbers:0};
  await changeOptions(second,expected,expected,expected);
  assert.equal(first.options.tabSize,4);assert.equal(first.options.indentSize,2);assert.equal(first.options.insertSpaces,true);
  assert.equal(first.options.cursorStyle,firstBefore.cursorStyle);assert.equal(first.options.lineNumbers,firstBefore.lineNumbers);unchangedPresentation(before);
 }
 register('options',async()=>{
  await explicit();
  const expected={...optionState(second),lineNumbers:3};await changeOptions(second,3,expected,expected,'lineNumbers');
  const cursor={...optionState(second),cursorStyle:6};await changeOptions(second,6,cursor,cursor,'cursorStyle');
  const events=optionEvents.length,unchanged=optionState(second);
  second.options.tabSize=-1;second.options.indentSize='invalid';assert.deepEqual(optionState(second),unchanged);assert.equal(optionEvents.length,events,'invalid source-normalized options are ignored');
  assert.equal(first.options.cursorStyle,1);assert.equal(first.options.lineNumbers,1);source();
  await v.window.showInformationMessage('API presentation options;sharedIndent=true;perPane=true;Interval=3');
 });
 register('auto',async()=>{
  const old=optionState(second),expected={...old,tabSize:4,indentSize:4,insertSpaces:true};
  await changeOptions(second,{tabSize:'auto',insertSpaces:'auto'},expected,old);
  assert.equal(first.options.tabSize,4);assert.equal(first.options.indentSize,4);assert.equal(first.options.insertSpaces,true);
  await changeOptions(second,{indentSize:2},{...expected,indentSize:2},{...expected,indentSize:2});
  const numeric=optionState(second);
  await changeOptions(second,{indentSize:'tabSize'},{...numeric,indentSize:4},numeric);
  source();await v.window.showInformationMessage('API presentation auto;detected=4;tabSizeAlias=true');
 });
 register('explicit',async()=>{await explicit();await v.window.showInformationMessage('API presentation explicit;tab=4;indent=2;spaces=true');});
 register('tab',()=>{source(tabbed);return v.window.showInformationMessage('API presentation native Tab;indent=2;sharedText=true');});
 register('dirty',()=>{source();return v.window.showInformationMessage('API presentation undo;dirtyBaseline=preserved');});
 register('clean',()=>{source(original);return v.window.showInformationMessage('API presentation original Undo;clean=true');});
 register('revealTop',()=>reveal(60,v.TextEditorRevealType.AtTop,'top'));
 register('revealCenter',()=>reveal(20,v.TextEditorRevealType.InCenter,'center'));
 register('revealInside',async()=>{
  const before=capturePresentation(),events=rangeEvents.length;second.revealRange(new v.Range(20,0,20,5),v.TextEditorRevealType.InCenterIfOutsideViewport);
  assert.strictEqual(await v.window.showTextDocument(document,{preview:false,viewColumn:1}),first);
  unchangedPresentation(before);assert.deepEqual(rangePoints(second),before.ranges);assert.equal(rangeEvents.length,events);
  await v.window.showInformationMessage('API presentation reveal inside;quiet=true;selections=preserved');
 });
 register('revealDefault',()=>reveal(80,v.TextEditorRevealType.Default,'default'));
 register('armResize',()=>{validateRanges(second);resize=capturePresentation();return v.window.showInformationMessage('API presentation resize armed');});
 register('retire',async()=>{
  const other=await v.workspace.openTextDocument(v.Uri.joinPath(v.Uri.file(v.workspace.rootPath),'b.txt'));
  const replacement=await v.window.showTextDocument(other,{preview:false,viewColumn:2});assert.notStrictEqual(replacement,second);
  const before=optionState(replacement),ranges=rangePoints(replacement),selections=points(replacement);
  assert.throws(()=>{second.options.tabSize=8;});assert.throws(()=>second.revealRange(new v.Range(0,0,0,1)));
  assert.deepEqual(optionState(replacement),before);assert.deepEqual(rangePoints(replacement),ranges);assert.deepEqual(points(replacement),selections);
  assert.equal(other.getText(),'other\r\n');assert.equal(other.isDirty,false);source();
  await v.window.showInformationMessage('API presentation retired;options/reveal=refused;replacement=untouched');
 });
};
