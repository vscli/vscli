const vscode = require('vscode');
exports.activate = context => {
  const register = (name, fn) => context.subscriptions.push(vscode.commands.registerCommand(name, fn));
  const initialConfiguration = vscode.workspace.getConfiguration('fixture');
  let configurationChanges = 0;
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(event => {
    if (event.affectsConfiguration('fixture.value')) {
      configurationChanges++;
      vscode.window.showInformationMessage(`configuration changed=${vscode.workspace.getConfiguration('fixture').get('value')}`);
    }
  }));
  register('fixture.configuration', () => vscode.window.showInformationMessage(`config=${JSON.stringify({
    activation: initialConfiguration.get('value'),
    value: vscode.workspace.getConfiguration('fixture').get('value'), changes: configurationChanges,
  })}`));
  register('fixture.sort', async () => {
    const editor = vscode.window.activeTextEditor;
    const text = editor.document.getText(editor.selection);
    const sorted = text.split('\n').sort().join('\n');
    const applied = await editor.edit(edit => edit.replace(editor.selection, sorted));
    await vscode.window.showInformationMessage(`sort applied=${applied}; version=${editor.document.version}`);
  });
  register('fixture.invalid', () => vscode.window.activeTextEditor.edit(edit => {
    edit.replace(new vscode.Range(0, 0, 0, 1), 'first');
    edit.replace(new vscode.Range(0, 0, 0, 2), 'overlap');
  }));
  register('fixture.stale', async () => {
    const promise = vscode.window.activeTextEditor.edit(edit => edit.insert(new vscode.Position(0, 0), 'stale'));
    require('node:fs').writeFileSync(require('node:path').join(vscode.workspace.rootPath, 'edit-submitted'), 'ready');
    const applied = await promise;
    await vscode.window.showInformationMessage(`stale applied=${applied}`);
  });
  register('fixture.args', (...args) => vscode.window.showInformationMessage(`args=${JSON.stringify(args)}`));
  register('fixture.crash', () => process.exit(7));
  register('fixture.unsupported', () => vscode.window.createWebviewPanel('test'));
  register('fixture.version', () => vscode.window.showInformationMessage(`version=${vscode.window.activeTextEditor.document.version}`));
};
