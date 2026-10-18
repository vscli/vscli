'use strict';
const v = require('vscode'), fs = require('node:fs'), path = require('node:path');
exports.activate = async context => {
  const root = v.workspace.workspaceFolders[0].uri.fsPath;
  await v.workspace.openTextDocument(v.Uri.file(path.join(root, 'peer.cpp')));
  const record = value => fs.appendFileSync(path.join(root, 'log'), JSON.stringify(value) + '\n');
  function participant(kind, event) {
    const mode = fs.readFileSync(path.join(root, 'mode'), 'utf8');
    record({ will: true, kind });
    if (mode === 'hold') {
      event.waitUntil(new Promise(resolve => {
        const timer = setInterval(() => {
          if (fs.existsSync(path.join(root, 'release'))) { clearInterval(timer); resolve(); }
        }, 5);
      }));
      return;
    }
    const target = kind === 'rename' ? event.files[0].oldUri
      : kind === 'delete' ? event.files[0] : v.Uri.file(path.join(root, 'a.cpp'));
    const edit = new v.WorkspaceEdit();
    edit.insert(target, new v.Position(0, 0), 'participant ',
      mode === 'confirm' ? { label: 'Needs review', needsConfirmation: true } : undefined);
    if (mode === 'mixed') edit.createFile(v.Uri.file(path.join(root, 'extra.cpp')));
    else edit.insert(v.Uri.file(path.join(root, 'peer.cpp')), new v.Position(0, 0), 'participant ');
    event.waitUntil(Promise.resolve(edit));
  }
  const did = (kind, event) => {
    const uri = kind === 'rename' ? event.files[0].newUri : event.files[0];
    const document = v.workspace.textDocuments.find(document => document.uri.toString() === uri.toString());
    record({ did: true, kind, text: document?.getText(),
      peer: v.workspace.textDocuments.find(document => document.uri.fsPath === path.join(root, 'peer.cpp'))?.getText() });
  };
  context.subscriptions.push(
    v.workspace.onWillCreateFiles(event => participant('create', event)),
    v.workspace.onWillRenameFiles(event => participant('rename', event)),
    v.workspace.onWillDeleteFiles(event => participant('delete', event)),
    v.workspace.onDidCreateFiles(event => did('create', event)),
    v.workspace.onDidRenameFiles(event => did('rename', event)),
    v.workspace.onDidDeleteFiles(event => did('delete', event)),
  );
};
