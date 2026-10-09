const v = require('vscode');
const fs = require('node:fs');
const path = require('node:path');
exports.activate = context => {
  const opaque = { callback: () => 'original identity' }; opaque.self = opaque;
  let original;
  context.subscriptions.push(v.languages.registerCompletionItemProvider('sql', {
    provideCompletionItems() {
      original = new v.CompletionItem('SELECT', v.CompletionItemKind.Snippet);
      original.insertText = new v.SnippetString('SELECT ${1:猫}$0');
      original.range = new v.Range(0, 0, 0, 3); original.data = opaque;
      return [original];
    },
    async resolveCompletionItem(item, token) {
      if (item !== original || item.data.callback() !== 'original identity') throw new Error('Completion identity lost');
      const hold = path.join(v.workspace.rootPath, 'hold-resolve');
      fs.writeFileSync(path.join(v.workspace.rootPath, 'resolve-started'), 'started');
      while (fs.existsSync(hold)) await new Promise(done => setTimeout(done, 5));
      if (token.isCancellationRequested) throw new Error('Completion canceled');
      item.documentation = new v.MarkdownString('Resolved SELECT snippet with Unicode import');
      item.detail = 'Resolved import and snippet';
      item.additionalTextEdits = [v.TextEdit.insert(new v.Position(1, 0), '-- import 界\n')];
      return item;
    },
  }));
};
