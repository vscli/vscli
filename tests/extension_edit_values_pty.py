#!/usr/bin/env python3
"""Mixed unsupported edits preserve actual native dirty text and Undo history."""
import json
from pathlib import Path
import sys
import tempfile

from extension_sessions_pty import Editor, LIVE, command, save
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, wait_screen

ORIGINAL = 'α😀\r\nsecond'
DIRTY = 'dirty ' + ORIGINAL
QUICK_FIX = b'\x1b[46;5u'


def fixture(root):
    extension = root / 'extension'
    extension.mkdir()
    (extension / 'package.json').write_text(json.dumps({
        'publisher': 'fixture', 'name': 'edit-values', 'version': '1.0.0',
        'main': 'extension.cjs', 'contributes': {'commands': [
            {'command': 'edits.inspect', 'title': 'Edit values inspect dirty model'},
        ]},
    }))
    (extension / 'extension.cjs').write_text(r"""
const vscode = require('vscode'); const assert = require('node:assert/strict');
exports.activate = context => {
 context.subscriptions.push(vscode.commands.registerCommand('edits.inspect', () => {
  const document = vscode.window.activeTextEditor.document;
  assert.equal(document.getText(), 'dirty α😀\r\nsecond'); assert(document.isDirty);
  return vscode.window.showInformationMessage('mixed edit refused; dirty model intact');
 }));
 context.subscriptions.push(vscode.languages.registerCodeActionsProvider('plaintext', {
  provideCodeActions(document) {
   const bad = new vscode.CodeAction('Reject mixed EOL edit', vscode.CodeActionKind.QuickFix);
   bad.edit = new vscode.WorkspaceEdit();
   bad.edit.set(document.uri, [vscode.TextEdit.replace(new vscode.Range(1,0,1,6), 'CORRUPT')]);
   bad.edit.set(document.uri, [vscode.TextEdit.setEndOfLine(vscode.EndOfLine.LF)]);
   assert.equal(bad.edit.get(document.uri).length, 2);
   const good = new vscode.CodeAction('Apply ordinary text edit', vscode.CodeActionKind.QuickFix);
   good.edit = new vscode.WorkspaceEdit();
   good.edit.set(document.uri, [vscode.TextEdit.replace(new vscode.Range(1,0,1,6), 'SECOND')]);
   return [bad, good];
  },
 }, {providedCodeActionKinds: [vscode.CodeActionKind.QuickFix]}));
};
""")
    return extension


def choose(app, title):
    app.send(QUICK_FIX)
    wait_screen(app, 'Code Actions', 'Reject mixed EOL edit', 'Apply ordinary text edit')
    rows = [line for line in app.screen.text().splitlines()
            if 'Reject mixed EOL edit' in line or 'Apply ordinary text edit' in line]
    selected = next(index for index, line in enumerate(rows) if title in line)
    app.send(b'\x1b[A' * 4 + b'\x1b[B' * selected + b'\r')


def run(root):
    extension = fixture(root)
    file = root / 'input.txt'
    file.write_bytes(ORIGINAL.encode())
    app = Editor(root, '--extension', extension, '--no-session', file, enhanced=True)
    wait_screen(app, 'Extension ready:', 'input.txt')
    app.send(b'\x1b[H')
    app.paste('dirty ')
    choose(app, 'Reject mixed EOL edit')
    wait_screen(app, 'Language action failed:', 'disabled:', 'text end-of-line')
    assert file.read_bytes() == ORIGINAL.encode()
    command(app, 'Edit values inspect dirty model')
    wait_screen(app, 'mixed edit refused; dirty model intact')
    choose(app, 'Apply ordinary text edit')
    wait_screen(app, 'Applied extension code action')
    save(app, file, DIRTY.replace('second', 'SECOND'))
    app.send(CTRL_Z)
    save(app, file, DIRTY)
    app.send(CTRL_Z)
    save(app, file, ORIGINAL)
    app.finish()
    print('PASS: mixed text/EOL action wholly refused; dirty native text and disk retained; ordinary edit/CRLF save and original Undo history preserved')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-edit-values-pty-') as directory:
        try:
            run(Path(directory))
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    stop_surface_process(app)
                    assert app.process.restored, 'Terminal mode leaked on cleanup'
                    app.close_fds()
                    LIVE.remove(app)
                except Exception as error:
                    errors.append(error)
            if errors:
                if failed:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
