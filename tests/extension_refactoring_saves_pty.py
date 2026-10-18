#!/usr/bin/env python3
"""Actual selected public/default F2 refactoring saves and original PTY history."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_A, CTRL_Z, CTRL_SHIFT_W, eventually, wait_screen

SOURCE = '猫🙂 source\r\nnext\r\n'
TARGET = 'α🙂 target\r\n'
HIDDEN = 'γ🙂 source\r\n'
UNOPENED = 'β🙂 source\r\n'
UNRELATED = 'δ🙂 unrelated\r\n'
ORIGINALS = {'source': SOURCE, 'target': TARGET, 'hidden': HIDDEN,
             'unopened': UNOPENED, 'unrelated': UNRELATED}
EDITED = {'source': 'dirty 猫🙂 renamed\r\nnext\r\n', 'target': 'dirty α🙂 renamed\r\n',
          'hidden': 'dirty γ🙂 renamed\r\n', 'unopened': 'β🙂 renamed\r\n'}
REDO = b'\x1b[122;6u'
F2 = b'\x1bOQ'


def ready(root, policy=None):
    for name, value in ORIGINALS.items():
        (root / (name + '.cpp')).write_bytes(value.encode())
    config = root / 'config'
    config.mkdir()
    settings = {'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
                'files.autoSave': 'off', 'editor.formatOnSave': False,
                'editor.codeActionsOnSave': {}}
    if policy is not None:
        settings['files.refactoring.autoSave'] = policy
    (config / 'settings.json').write_text(json.dumps(settings), encoding='utf-8')
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/refactoring-save-extension', extension)
    node = shutil.which('node')
    assert node, 'Actual selected refactoring save API requires optional Node'
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'source.cpp', enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'source.cpp')
    app.paste('dirty ')
    return app


def verify(app, title, expected):
    command(app, 'Refactoring fixture ' + title)
    wait_screen(app, expected)


def prepare(app):
    verify(app, 'prepare cohort', 'API refactoring cohort prepared;dirty/shared/hidden/unopened=true')


def original_disks(root):
    for name, value in ORIGINALS.items():
        assert (root / (name + '.cpp')).read_bytes() == value.encode(), name


def saved_disks(root, conflict=False):
    for name, value in EDITED.items():
        expected = 'external conflict 猫🙂\r\n' if conflict and name == 'target' else value
        assert (root / (name + '.cpp')).read_bytes() == expected.encode(), name
    assert (root / 'unrelated.cpp').read_bytes() == UNRELATED.encode()


def finish(app, dirty):
    app.send(CTRL_SHIFT_W)
    for _ in range(dirty):
        wait_screen(app, 'Unsaved Changes')
        app.send(b'd')
    eventually(lambda: app.read() and app.process.poll() is not None)
    app.finish()


def default_public(root):
    app = ready(root)
    prepare(app)
    original_disks(root)
    verify(app, 'apply default cohort', 'API refactoring default;applied=true;didSave=4;identity/shared/hidden=true')
    saved_disks(root)
    app.send(CTRL_Z)
    verify(app, 'verify grouped Undo', 'API refactoring grouped Undo;allOriginal=true;didSave=4')
    saved_disks(root)
    app.send(REDO)
    verify(app, 'verify grouped Redo', 'API refactoring Redo;applied=true;didSave=4;identity/shared/hidden=true')
    saved_disks(root)
    finish(app, 1)
    saved_disks(root)
    print('PASS: actual awaited public default multi-resource refactoring saves dirty/shared/hidden/unopened native models, publishes all didSave events before true, and keyboard grouped Undo/Redo retains Unicode CRLF disks and unrelated dirty work')


def default_f2(root):
    app = ready(root)
    prepare(app)
    verify(app, 'arm F2 cohort', 'API refactoring F2 armed')
    app.send(F2)
    wait_screen(app, 'Rename Symbol · Enter applies', 'source')
    app.send(CTRL_A)
    app.paste('renamed')
    app.send(b'\r')
    # This is aggregate completion; an intermediate clean source is insufficient.
    wait_screen(app, 'Renamed atomically · Undo restores the workspace', absent=('Rename Symbol',))
    verify(app, 'verify saved cohort', 'API refactoring F2;applied=true;didSave=4;identity/shared/hidden=true')
    saved_disks(root)
    app.send(CTRL_Z)
    verify(app, 'verify grouped Undo', 'API refactoring grouped Undo;allOriginal=true;didSave=4')
    saved_disks(root)
    app.send(REDO)
    verify(app, 'verify grouped Redo', 'API refactoring Redo;applied=true;didSave=4;identity/shared/hidden=true')
    finish(app, 1)
    saved_disks(root)
    print('PASS: original F2 default autosave waits for all four native receipts; keyboard Undo dirties the entire original identity cohort while preserving committed disks, and Redo returns it clean')


def disabled(root):
    app = ready(root, False)
    prepare(app)
    verify(app, 'apply disabled cohort', 'API refactoring off;applied=true;didSave=0;identity/shared/hidden=true')
    original_disks(root)
    app.send(CTRL_Z)
    verify(app, 'verify grouped Undo', 'API refactoring grouped Undo;allOriginal=true;didSave=0')
    original_disks(root)
    finish(app, 4)  # Source, shared target, hidden target and unrelated remain dirty.
    original_disks(root)
    print('PASS: explicit refactoring autosave false preserves unsaved atomic cohort edits and grouped keyboard Undo with original disk bytes unchanged')


def single_and_unsupported(root):
    app = ready(root)
    verify(app, 'refuse mixed edits', 'API refactoring unsupported;wholeRefused=true;didSave=0')
    assert not (root / 'unsupported-created.cpp').exists()
    original_disks(root)
    verify(app, 'apply single', 'API refactoring single;applied=true;didSave=0;dirty=true')
    original_disks(root)
    finish(app, 1)
    original_disks(root)
    print('PASS: single-resource default refactoring remains dirty and whole unsupported mixed edits refuse without native text or filesystem effects')


def save_as(root, cancel=False):
    app = ready(root)
    title = 'apply canceled SaveAs' if cancel else 'apply SaveAs'
    command(app, 'Refactoring fixture ' + title)
    wait_screen(app, 'Save As (existing files are protected)')
    destination = root / "saved-untitled 猫 # %'(.cpp"
    if cancel:
        app.send(b'\x1b')
        wait_screen(app, 'automatic refactoring save was canceled')
        wait_screen(app, 'API refactoring SaveAs canceled;applied=true;didSave=1;namedCommitted=true')
        assert not destination.exists()
        assert not (root / 'untitled.txt').exists()
        finish(app, 1)
    else:
        app.send(CTRL_A)
        app.paste(str(destination))
        app.send(b'\r')
        wait_screen(app, 'API refactoring SaveAs accepted;applied=true;didSave=2;namedCommitted=true')
        assert destination.read_bytes() == 'saved untitled 猫🙂 before\r\n'.encode()
        finish(app, 0)
    assert (root / 'source.cpp').read_bytes() == ('named dirty ' + SOURCE).encode()
    for name in ['target', 'hidden', 'unopened', 'unrelated']:
        assert (root / (name + '.cpp')).read_bytes() == ORIGINALS[name].encode()
    print('PASS: real automatic refactoring Save As ' + ('cancel retains applied true, warning and dirty untitled work while named peer commits' if cancel else 'acceptance preserves native identity and publishes old-close/new-open/didSave before awaited true'))


def conflict(root):
    app = ready(root)
    prepare(app)
    (root / 'target.cpp').write_bytes('external conflict 猫🙂\r\n'.encode())
    command(app, 'Refactoring fixture apply conflict')
    wait_screen(app, 'Automatic refactoring save failed; edits remain applied')
    wait_screen(app, 'API refactoring conflict;applied=true;didSave=3;dirtyFailedTarget=true;noRetry=true')
    saved_disks(root, True)
    finish(app, 2)
    saved_disks(root, True)
    print('PASS: actual disk conflict preserves atomic applied true and three peer commits, warns, retains external target bytes and dirty failed target without retry')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-refactoring-saves-') as directory:
        try:
            cases = [('default-public', default_public), ('default-f2', default_f2),
                     ('disabled', disabled), ('single-unsupported', single_and_unsupported),
                     ('saveas-accepted', save_as), ('saveas-canceled', lambda root: save_as(root, True)),
                     ('conflict', conflict)]
            for name, run in cases:
                root = Path(directory, name).resolve()
                root.mkdir()
                run(root)
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'Refactoring interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
