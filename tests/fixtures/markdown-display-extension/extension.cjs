'use strict';
const vscode=require('vscode'), assert=require('node:assert/strict');
const fs=require('node:fs'), path=require('node:path');
function document() {
  const value=new vscode.MarkdownString();
  value.appendText('spaces  tab\ttilde ~x~\r\n- literal 猫🙂');
  value.appendCodeblock('&nbsp; \\*literal\\* [Run](command:unsafe)\r\n```embedded','cpp');
  return value;
}
exports.activate=context=>{
  const record=(method,value)=>fs.appendFileSync(path.join(vscode.workspace.rootPath,'markdown-callbacks.jsonl'),JSON.stringify({method,version:value.version,text:value.getText(),dirty:value.isDirty})+'\n');
  context.subscriptions.push(vscode.languages.registerHoverProvider('plaintext',{
    provideHover(value){record('hover',value);return new vscode.Hover([document(),{language:'cpp',value:'MARKED CODE &nbsp; \\*literal\\*'}]);}
  }));
  context.subscriptions.push(vscode.languages.registerCompletionItemProvider('plaintext',{
    provideCompletionItems(value){record('completion',value);const item=new vscode.CompletionItem('DocChoice');item.documentation=document();const plain=new vscode.CompletionItem('PlainChoice');plain.documentation='PLAINTEXT &nbsp; \\*literal\\*';return [item,plain];}
  }));
  context.subscriptions.push(vscode.languages.registerSignatureHelpProvider('plaintext',{
    provideSignatureHelp(value){record('signature',value);const help=new vscode.SignatureHelp();help.signatures=[new vscode.SignatureInformation('fixture(猫)',new vscode.MarkdownString().appendText('SIGNATURE spaces  tab\t~literal~'))];help.activeSignature=0;help.activeParameter=0;return help;}
  }));
  context.subscriptions.push(vscode.commands.registerCommand('unsafe',()=>fs.writeFileSync(path.join(vscode.workspace.rootPath,'unsafe-executed'),'BAD')));
  context.subscriptions.push(vscode.commands.registerCommand('markdown.inspect',()=>{
    const value=vscode.window.activeTextEditor.document;
    assert.equal(value.getText(),'dirty α😀\r\nsecond');assert(value.isDirty);
    return vscode.window.showInformationMessage('Markdown display: original dirty model intact');
  }));
};
