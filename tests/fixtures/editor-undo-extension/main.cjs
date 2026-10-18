'use strict';
const assert=require('node:assert/strict'),v=require('vscode');
exports.activate=context=>{
 context.subscriptions.push(v.commands.registerCommand('fixture.editorUndo.append',async options=>{
  const editor=v.window.activeTextEditor;assert(editor);const document=editor.document;
  const before=document.getText(),version=document.version,events=[];
  const listener=v.workspace.onDidChangeTextDocument(event=>events.push(event));
  try {
   assert.equal(await editor.edit(edit=>edit.insert(document.lineAt(0).range.end,'A'),options),true);
   assert.equal(document.getText(),before.replace('\r\n','A\r\n'));
   assert.equal(document.eol,v.EndOfLine.CRLF);assert(document.version>version);
   assert(events.some(event=>event.document===document));
   assert(v.window.visibleTextEditors.every(item=>item.document===document));
  } finally {listener.dispose();}
  return v.window.showInformationMessage('Editor undo edit applied');
 }));
 context.subscriptions.push(v.commands.registerCommand('fixture.editorUndo.joined',async()=>{
  const editor=v.window.activeTextEditor;assert(editor);const document=editor.document;
  for(const [text,before,after] of [['A',true,false],['B',false,true]]) {
   assert.equal(await editor.edit(edit=>edit.insert(document.lineAt(0).range.end,text),{undoStopBefore:before,undoStopAfter:after}),true);
  }
  assert.equal(document.getText(),'λ🙂baseAB\r\n');
  return v.window.showInformationMessage('Joined API edits applied');
 }));
};
