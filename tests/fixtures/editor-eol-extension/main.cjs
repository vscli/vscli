'use strict';
const assert=require('node:assert/strict'),v=require('vscode');
exports.activate=context=>context.subscriptions.push(v.commands.registerCommand('fixture.editorEol.normalize',async()=>{
 const editor=v.window.activeTextEditor;assert(editor);assert.equal(editor.document.eol,v.EndOfLine.CRLF);
 assert.equal(await editor.edit(edit=>edit.insert(new v.Position(0,0),'α\nβ\rγ\r\n')),true);
 assert(editor.document.getText().startsWith('α\r\nβ\r\nγ\r\n'));
 return v.window.showInformationMessage('Multiline editor edit normalized');
}));
