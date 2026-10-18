'use strict';
const assert=require('node:assert/strict'),v=require('vscode');
exports.activate=context=>context.subscriptions.push(v.commands.registerCommand('fixture.editorPosition.replace',async()=>{
 const editor=v.window.activeTextEditor;assert(editor);const document=editor.document;
 assert.equal(document.getText(),'λ🙂 original\r\n');assert.equal(document.eol,v.EndOfLine.CRLF);
 assert(v.window.visibleTextEditors.every(item=>item.document===document));
 const events=[],listener=v.workspace.onDidChangeTextDocument(event=>events.push(event));
 try {
  assert.equal(await editor.edit(edit=>{
   edit.replace(new v.Position(0,3),'α\n');
   edit.replace(new v.Position(999,999),'tail');
  }),true);
  assert.equal(document.getText(),'λ🙂α\r\n original\r\ntail');
  assert.equal(document.eol,v.EndOfLine.CRLF);
  assert(events.some(event=>event.document===document));
 } finally {listener.dispose();}
 return v.window.showInformationMessage('Position replacement applied');
}));
