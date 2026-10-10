const v = require('vscode');
const fs = require('node:fs');
const path = require('node:path');
exports.activate = context => {
  let previous;
  let count = 0;
  context.subscriptions.push(v.languages.registerSignatureHelpProvider('cpp', {
    provideSignatureHelp(doc, position, token, request) {
      const number = ++count;
      const record = {
        number, text: doc.getText(), position, triggerKind: request.triggerKind,
        triggerCharacter: request.triggerCharacter, isRetrigger: request.isRetrigger,
        originalObject: !!previous && request.activeSignatureHelp === previous,
        activeSignature: request.activeSignatureHelp?.activeSignature,
        activeParameter: request.activeSignatureHelp?.activeParameter,
      };
      fs.appendFileSync(path.join(v.workspace.rootPath, 'extension-signatures.jsonl'), JSON.stringify(record) + '\n');
      if (previous && !record.originalObject) throw new Error('Original signature object was not revived');
      const help = new v.SignatureHelp();
      help.activeSignature = 0;
      help.activeParameter = doc.getText(new v.Range(new v.Position(position.line, 0), position)).includes(',') ? 1 : 0;
      help.signatures = ['int left', '猫🙂 left'].map((left, index) => {
        const right = index ? 'double right' : 'int right';
        const item = new v.SignatureInformation(`extension${number}(${left}, ${right})`, index ? 'Unicode overload' : 'integer overload');
        item.parameters = [new v.ParameterInformation(left), new v.ParameterInformation(right, 'right argument')];
        return item;
      });
      help.opaque = { owner: context.extension.id, circular: help };
      previous = help;
      return help;
    }
  }, { triggerCharacters: ['('], retriggerCharacters: [',', ';'] }));
  context.subscriptions.push(v.languages.registerCompletionItemProvider('cpp', {
    provideCompletionItems() {
      return ['choiceOne', 'choiceTwo'].map(label => new v.CompletionItem(label, v.CompletionItemKind.Variable));
    }
  }));
};
