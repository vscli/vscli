'use strict';
const assert=require('node:assert/strict'),v=require('vscode');
exports.activate=context=>{
 context.subscriptions.push(v.commands.registerCommand('fixture.editorSetEol.concurrent',async()=>{
  const editor=v.window.activeTextEditor;assert(editor);const document=editor.document;
  const original=document.getText();assert.equal(document.eol,v.EndOfLine.LF);
  const eol=editor.edit(edit=>edit.setEndOfLine(v.EndOfLine.CRLF));
  const later=editor.edit(edit=>edit.insert(new v.Position(0,0),'forbidden'));
  assert.deepEqual(await Promise.all([eol,later]),[true,false]);
  assert.equal(document.eol,v.EndOfLine.CRLF);assert.equal(document.getText(),original.replaceAll('\n','\r\n'));
  return v.window.showInformationMessage('Concurrent editor EOL order preserved');
 }));
 context.subscriptions.push(v.commands.registerCommand('fixture.editorSetEol.mixed',async()=>{
  const editor=v.window.activeTextEditor;assert(editor);const document=editor.document;
  assert.equal(document.getText(),'λ🙂 original\nsecond\n');assert.equal(document.eol,v.EndOfLine.LF);
  const events=[],listener=v.workspace.onDidChangeTextDocument(event=>{
   if(event.document===document)events.push({eol:document.eol,text:document.getText(),version:document.version});
  });
  try {
   assert.equal(await editor.edit(edit=>{edit.setEndOfLine(v.EndOfLine.CRLF);edit.insert(new v.Position(0,3),'\nX');}),true);
   assert.deepEqual(events.map(event=>[event.eol,event.text]),[
    [2,'λ🙂 original\r\nsecond\r\n'],[2,'λ🙂\r\nX original\r\nsecond\r\n'],
   ]);
   assert(events[1].version>events[0].version);
   assert(v.window.visibleTextEditors.every(item=>item.document===document));
  } finally {listener.dispose();}
  return v.window.showInformationMessage('Editor EOL and text applied');
 }));
 context.subscriptions.push(v.commands.registerCommand('fixture.editorSetEol.only',async()=>{
  const editor=v.window.activeTextEditor;assert(editor);const document=editor.document;
  assert.equal(await editor.edit(edit=>{edit.setEndOfLine(v.EndOfLine.CRLF);edit.setEndOfLine(v.EndOfLine.LF);}),true);
  assert.equal(document.eol,v.EndOfLine.LF);assert(!document.getText().includes('\r'));
  return v.window.showInformationMessage('Editor EOL only applied');
 }));
 context.subscriptions.push(v.commands.registerCommand('fixture.editorSetEol.same',async()=>{
  const editor=v.window.activeTextEditor;assert(editor);const document=editor.document;
  const version=document.version;let events=0;
  const listener=v.workspace.onDidChangeTextDocument(event=>{if(event.document===document)events++;});
  try {assert.equal(await editor.edit(edit=>edit.setEndOfLine(document.eol)),true);assert.equal(document.version,version);assert.equal(events,0);}
  finally {listener.dispose();}
  return v.window.showInformationMessage('Editor EOL unchanged');
 }));
};
