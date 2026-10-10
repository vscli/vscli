'use strict';
const fs=require('node:fs'), path=require('node:path'), assert=require('node:assert/strict');
const {createHash}=require('node:crypto');
const vscode=require('vscode');
const {parse}=require('../../tests/vscode-reference/node_modules/jsonc-parser');
const {prepareLanguages}=require('../../tests/vscode-reference/language-readiness.cjs');
const cases=require('./cases.json');
const mode=process.env.VSCLI_REFERENCE_INDENT_MODE;
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function snapshot(editor){
 const text=editor.document.getText();
 const scalar=position=>[...text.slice(0,editor.document.offsetAt(position))].length;
 return {text,selections:editor.selections.map(s=>({anchor:scalar(s.anchor),cursor:scalar(s.active)})),version:editor.document.version,eol:editor.document.eol===vscode.EndOfLine.CRLF?'CRLF':'LF'};
}
function effective(document){
 const c=vscode.workspace.getConfiguration('editor',document);
 return Object.fromEntries(['autoIndent','tabSize','insertSpaces','detectIndentation','autoClosingQuotes','autoClosingBrackets'].map(key=>[key,c.get(key)]));
}
async function close(){await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');}
async function show(text,language){
 const doc=await vscode.workspace.openTextDocument({content:text,language});
 const editor=await vscode.window.showTextDocument(doc,{preview:false});
 assert.equal(effective(doc).autoIndent,mode);
 return editor;
}
async function enterWitness(label){
 const editor=await show('    value\r\n','plaintext');
 try{
  const p=editor.document.positionAt(9);editor.selection=new vscode.Selection(p,p);
  const before=snapshot(editor);
  await vscode.commands.executeCommand('type',{text:'\n'});
  const after=snapshot(editor);
  assert.equal(after.text,'    value\r\n'+(mode==='none'?'':'    ')+'\r\n',`Independent plain Enter mode witness failed: ${mode}/${label}`);
  return {label,language:'plaintext',effective:effective(editor.document),before,after};
 }finally{await close();}
}
async function grammarWitness(configuration){
 assert.ok(configuration.autoClosingPairs.some(p=>p.open==="'"&&p.close==="'"&&p.notIn?.includes('comment')));
 const original='vscli_probe \r\n    \r\n// ';
 const editor=await show(original,'cpp');
 const deadline=Date.now()+10000;
 try{
  for(let attempts=1;;attempts++){
   assert.equal(editor.document.getText(),original,'Independent grammar probe must restore initial bytes');
   let p=editor.document.positionAt(original.indexOf('\r\n//'));
   editor.selection=new vscode.Selection(p,p);
   await vscode.commands.executeCommand('type',{text:"'"});
   const positive=snapshot(editor);
   if(positive.text===original.replace('    \r\n','    \'\'\r\n')){
    p=editor.document.positionAt(editor.document.getText().length);editor.selection=new vscode.Selection(p,p);
    await vscode.commands.executeCommand('type',{text:"'"});
    const negative=snapshot(editor);
    if(negative.text===positive.text+"'")return {attempts,positive,negative,effective:effective(editor.document),witness:'Positive code pair at line2 then comment suppression at line3; independent of electric indentation'};
    await vscode.commands.executeCommand('undo');assert.equal(editor.document.getText(),positive.text);
   }
   await vscode.commands.executeCommand('undo');assert.equal(editor.document.getText(),original);
   assert.ok(Date.now()<deadline,'Positive/negative actual grammar witness timed out');
   await sleep(20);
  }
 }finally{await close();}
}
async function changeTabSize(editor,value){
 const changed=new Promise((resolve,reject)=>{
  const timer=setTimeout(()=>{subscription.dispose();reject(new Error('Public editor model-option change did not settle'));},5000);
  const subscription=vscode.window.onDidChangeTextEditorOptions(event=>{
   if(event.textEditor===editor&&event.options.tabSize===value){clearTimeout(timer);subscription.dispose();resolve();}
  });
 });
 editor.options={tabSize:value};await changed;assert.equal(editor.options.tabSize,value);
}
exports.run=async()=>{
 assert.equal(vscode.version,'1.95.0');assert.ok(['none','keep','brackets','advanced','full'].includes(mode));
 const productBytes=fs.readFileSync(path.join(vscode.env.appRoot,'product.json')),product=JSON.parse(productBytes);
 assert.equal(product.commit,'912bb683695358a54ae0c670461738984cbb5b95');
 const extension=vscode.extensions.getExtension('vscode.cpp');assert.ok(extension);
 const contribution=extension.packageJSON.contributes.languages.find(l=>l.id==='cpp'), configurationBytes=fs.readFileSync(path.join(extension.extensionPath,contribution.configuration)), errors=[];
 const configuration=parse(configurationBytes.toString(),errors);assert.deepEqual(errors,[]);
 const c=vscode.workspace.getConfiguration('editor',{languageId:'cpp'}),initialConfiguration={global:c.inspect('autoIndent').globalValue,cpp:c.inspect('autoIndent').globalLanguageValue,effective:c.get('autoIndent')};
 assert.deepEqual(initialConfiguration,{global:mode,cpp:mode,effective:mode});
 const globalBefore=(await enterWitness('before-language-activation'));
 await prepareLanguages(vscode,[{language:'cpp'}]);
 const grammar=await grammarWitness(configuration);
 const commands=new Set(await vscode.commands.getCommands(true));assert.ok(commands.has('editor.action.forceRetokenize'));
 const traces=[];
 for(const fixture of cases){
  const modeBefore=await enterWitness(`${fixture.name}/before`);
  const editor=await show(fixture.text,'cpp');
  try{
   const pos=editor.document.positionAt([...fixture.text].slice(0,fixture.cursor).join('').length);editor.selection=new vscode.Selection(pos,pos);
   const beforePreparation=snapshot(editor);
   if(fixture.arm==='force-retokenize')await vscode.commands.executeCommand('editor.action.forceRetokenize');
   if(fixture.arm==='model-options-recreation'){await changeTabSize(editor,5);await changeTabSize(editor,4);}
   const initial=snapshot(editor);
   assert.deepEqual(initial,beforePreparation,'Preparation must preserve exact bytes, version and selections');
   await vscode.commands.executeCommand('type',{text:'}'});const typed=snapshot(editor);
   await vscode.commands.executeCommand('undo');const undone=snapshot(editor);
   await vscode.commands.executeCommand('redo');const redone=snapshot(editor);
   for(const key of ['text','selections','eol']){assert.deepEqual(undone[key],initial[key],`${fixture.name} Undo/${key}`);assert.deepEqual(redone[key],typed[key],`${fixture.name} Redo/${key}`);}
   traces.push({name:fixture.name,arm:fixture.arm,mode,effective:effective(editor.document),beforePreparation,initial,typed,undone,redone,modeBefore});
  }finally{await close();}
  traces.at(-1).modeAfter=await enterWitness(`${fixture.name}/after`);
  console.log(`Observed ${mode} ${fixture.name}: ${JSON.stringify(traces.at(-1).typed.text)}`);
 }
 const globalAfter=await enterWitness('after-all-targets');
 const finalConfiguration={global:c.inspect('autoIndent').globalValue,cpp:c.inspect('autoIndent').globalLanguageValue,effective:c.get('autoIndent')};assert.deepEqual(finalConfiguration,initialConfiguration);
 const traceFile=`observed-${mode}.json`;fs.writeFileSync(path.join(__dirname,traceFile),JSON.stringify({mode,initialConfiguration,finalConfiguration,globalBefore,grammar,traces,globalAfter},null,2)+'\n');
 const artifact=name=>hash(fs.readFileSync(path.join(__dirname,name)));
 fs.writeFileSync(path.join(__dirname,`provenance-${mode}.json`),JSON.stringify({version:vscode.version,commit:product.commit,platform:process.platform,architecture:process.arch,observedAt:new Date().toISOString(),mode,fixtureCount:traces.length,productSha256:hash(productBytes),observerSha256:artifact('suite.cjs'),casesSha256:artifact('cases.json'),traceSha256:artifact(traceFile),runnerSha256:artifact('run.cjs'),workerSha256:artifact('worker.cjs'),readinessSha256:hash(fs.readFileSync(path.resolve(__dirname,'../../tests/vscode-reference/language-readiness.cjs'))),testElectronLockSha256:hash(fs.readFileSync(path.resolve(__dirname,'../../tests/vscode-reference/package-lock.json'))),profileSettingsSha256:process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256,configuration:{extension:extension.id,version:extension.packageJSON.version,path:contribution.configuration,sha256:hash(configurationBytes),actual:configuration},projection:'Fresh isolated process per mode; global and CPP override fixed before startup; independent plain Enter witnesses before/after targets; positive then negative grammar readiness only retried; target gesture never retried; natural, full force-retokenize and public model-options recreation arms explicitly distinct.'},null,2)+'\n');
 console.log(`Observed actual fixed-${mode} process ${traces.length} targets; before/after mode and history witnesses passed`);
};
