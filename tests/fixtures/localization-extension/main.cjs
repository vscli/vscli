const v = require('vscode');
const assert = require('node:assert/strict');
const loaded = v.l10n.t('Ready');
exports.activate = context => {
  assert.equal(v.env.appName, 'VSCLI');
  assert.equal(v.env.uiKind, v.UIKind.Desktop);
  assert.equal(v.env.remoteName, undefined);
  assert(require('node:fs').statSync(v.env.appRoot).isDirectory());
  if (v.env.language === 'fr') {
    assert.equal(v.l10n.bundle.Ready, 'Prêt');
    assert.equal(v.l10n.uri.scheme, 'file');
  } else {
    assert.equal(v.l10n.bundle, undefined);
    assert.equal(v.l10n.uri, undefined);
  }
  context.subscriptions.push(v.commands.registerCommand('fixture.localized', async () => {
    const prefix = v.l10n.t('Hello {name}', {name:'猫🙂'});
    const ok = await v.window.activeTextEditor.edit(edit => edit.insert(new v.Position(0,0), prefix + ' '));
    await v.window.showInformationMessage(loaded + '/' + v.l10n.t({message:'Applied {0}',args:[ok],comment:'action'}) + ';language=' + v.env.language);
  }));
};
