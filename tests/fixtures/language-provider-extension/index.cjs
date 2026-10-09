const v = require('vscode');
const path = require('node:path');
exports.activate = () => {
  const selector = [{ language: 'sql' }, { language: 'plaintext' }];
  v.languages.registerCompletionItemProvider(selector, {
    provideCompletionItems(doc, position) {
      const item = new v.CompletionItem('SELECT', v.CompletionItemKind.Keyword);
      item.range = new v.Range(0, 0, 0, 3); item.insertText = 'SELECT';
      return [item];
    }
  });
  v.languages.registerHoverProvider(selector, { provideHover: () => new v.Hover('Native provider hover') });
  v.languages.registerDefinitionProvider(selector, {
    provideDefinition: () => new v.Location(v.Uri.file(path.join(v.workspace.rootPath, 'target.sql')), new v.Range(0, 1, 0, 3))
  });
  v.languages.registerReferenceProvider(selector, {
    provideReferences: () => [new v.Location(v.Uri.file(path.join(v.workspace.rootPath, 'target.sql')), new v.Range(0, 1, 0, 3))]
  });
  v.languages.registerDocumentFormattingEditProvider(selector, {
    provideDocumentFormattingEdits: doc => [v.TextEdit.replace(new v.Range(0, 0, doc.lineCount - 1, doc.lineAt(doc.lineCount - 1).text.length), 'SELECT 🙂\nFROM table;\n')]
  });
  v.languages.registerDocumentSymbolProvider(selector, {
    provideDocumentSymbols: () => [new v.DocumentSymbol('query', '', v.SymbolKind.Function, new v.Range(0, 0, 0, 3), new v.Range(0, 0, 0, 3))]
  });
  v.languages.registerSignatureHelpProvider(selector, {
    provideSignatureHelp: () => {
      const hint = new v.SignatureHelp();
      const sig = new v.SignatureInformation('query(🙂value)');
      sig.parameters = [new v.ParameterInformation([6, 13])]; hint.signatures = [sig]; return hint;
    }
  });
};
