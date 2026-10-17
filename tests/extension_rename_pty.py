#!/usr/bin/env python3
"""Public rename skeleton, real F2/grouped history and a no-Node LSP control."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_A, CTRL_S, CTRL_Z, CTRL_SHIFT_W, eventually, wait_screen

SOURCE = '猫🙂 source\r\nnext\r\n'
TARGET = 'α🙂 target\r\n'
HIDDEN = 'γ🙂 source\r\n'
UNOPENED = 'β🙂 source\r\n'
DIRTY = 'dirty ' + SOURCE
REDO = b'\x1b[122;6u'
F2 = b'\x1bOQ'


def files(root):
    for name, text in [('source.cpp', SOURCE), ('target.cpp', TARGET),
                       ('hidden.cpp', HIDDEN), ('unopened.cpp', UNOPENED)]:
        (root / name).write_bytes(text.encode())


def configuration(root, refactoring=False):
    config = root / 'config'
    config.mkdir()
    settings = {'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
                'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {}}
    if refactoring is not None:
        settings['files.refactoring.autoSave'] = refactoring
    (config / 'settings.json').write_text(json.dumps(settings), encoding='utf-8')
    return config


def ready(root, refactoring=False):
    files(root)
    config = configuration(root, refactoring)
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/rename-provider-extension', extension)
    node = shutil.which('node')
    assert node, 'Selected rename provider qualification requires optional Node'
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'source.cpp', enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'source.cpp')
    app.paste('dirty ')
    return app


def verify(app, title, receipt):
    command(app, 'Rename fixture ' + title)
    wait_screen(app, receipt)


def rename(app, name):
    app.send(F2)
    wait_screen(app, 'Rename Symbol · Enter applies', 'source', absent=('Command Palette',))
    app.send(CTRL_A)
    app.paste(name)
    app.send(b'\r')


def disks(root):
    for name, text in [('source.cpp', SOURCE), ('target.cpp', TARGET),
                       ('hidden.cpp', HIDDEN), ('unopened.cpp', UNOPENED)]:
        assert (root / name).read_bytes() == text.encode()
    for name in ['created.cpp', 'old.cpp', 'new.cpp', 'deleted.cpp']:
        assert not (root / name).exists()


def history(app):
    verify(app, 'show source', 'API rename source;identity=true')
    app.send(CTRL_Z)
    verify(app, 'verify original Undo', 'API rename source Undo;exact=true')
    app.send(REDO)
    verify(app, 'verify original Redo', 'API rename source Redo;exact=true')


def finish(app, dirty=1):
    app.send(CTRL_SHIFT_W)
    for _ in range(dirty):
        wait_screen(app, 'Unsaved Changes')
        app.send(b'd')
    eventually(lambda: app.read() and app.process.poll() is not None)
    app.finish()


def queries(root):
    app = ready(root)
    verify(app, 'public queries', 'API rename queries;prepare/fallback/index/empty/fullValue=true;noMutation=true')
    history(app)
    disks(root)
    finish(app)
    disks(root)
    print('PASS: actual public prepare/execute queries prepare again, retain fallback index and empty names, return fresh typed values and refuse unsupported mixed apply without focus/query edits or disk effects')


def grouped(root):
    app = ready(root)
    verify(app, 'prepare shared hidden targets', 'API rename targets;shared/hiddenDirty=true;unopenedUntouched=true')
    rename(app, 'renamed')
    wait_screen(app, 'Renamed atomically · Undo restores the workspace', absent=('Rename Symbol',))
    verify(app, 'verify grouped apply', 'API rename applied;grouped/source/shared/hidden/unopened=true;didSave=0')
    disks(root)
    app.send(CTRL_Z)
    verify(app, 'verify grouped Undo', 'API rename Undo;grouped/source/shared/hidden/unopened=true;didSave=0')
    app.send(REDO)
    verify(app, 'verify grouped Redo', 'API rename Redo;grouped/source/shared/hidden/unopened=true;didSave=0')
    app.send(CTRL_Z)
    verify(app, 'verify grouped Undo', 'API rename Undo;grouped/source/shared/hidden/unopened=true;didSave=0')
    history(app)
    disks(root)
    finish(app, 3)
    disks(root)
    print('PASS: original F2 atomically renames retained dirty/shared, hidden dirty and worker-opened targets under explicit autosave-off; one keyboard Undo/Redo restores the entire Unicode CRLF cohort without saving disks')


def single(root):
    app = ready(root, None)
    rename(app, 'renamed')
    wait_screen(app, 'Renamed atomically · Undo restores the workspace', absent=('Rename Symbol',))
    verify(app, 'verify single apply', 'API rename single;identity/dirty=true;defaultPolicy/didSave=0')
    disks(root)
    app.send(CTRL_Z)
    verify(app, 'verify original Redo', 'API rename source Redo;exact=true')
    app.send(REDO)
    verify(app, 'verify single apply', 'API rename single;identity/dirty=true;defaultPolicy/didSave=0')
    app.send(CTRL_Z)
    history(app)
    disks(root)
    finish(app)
    print('PASS: genuine default refactoring policy does not autosave a single-resource original F2 rename; dirty identity and keyboard rename/original history remain recoverable')


def held(root):
    app = ready(root)
    verify(app, 'arm held prepare', 'API rename held prepare armed')
    app.send(F2)
    wait_screen(app, 'API rename held prepare;entered=true;actualPending=true')
    app.paste('X')
    app.send(CTRL_Z)
    wait_screen(app, 'API rename held prepare;cancelled=true;actualPending=true')
    verify(app, 'release actual callback', 'API rename held prepare;cancelled/actualSettled=true')
    app.send(REDO)
    verify(app, 'verify held Redo', 'API rename held Redo;intentionalEdit/lateReply=false')
    app.send(CTRL_Z)
    verify(app, 'arm held edit', 'API rename held edit armed')
    rename(app, 'ignored')
    wait_screen(app, 'API rename held edit;entered=true;actualPending=true')
    app.send(b'\x1b')
    wait_screen(app, 'API rename held edit;cancelled=true;actualPending=true')
    verify(app, 'release actual callback', 'API rename held edit;cancelled/actualSettled=true')
    history(app)
    disks(root)
    finish(app)
    disks(root)
    print('PASS: actual prepare source-edit/Undo and edit Escape cancel real tokens; ignored-cancel callbacks settle without late prompt/apply or loss of intentional Redo/original histories/disks')


def capacity(root):
    app = ready(root)
    verify(app, 'hold capacity', 'API rename capacity;actualCallbacks=8;held=true')
    verify(app, 'verify capacity overflow', 'API rename capacity;ninth=refused;actualStill8=true')
    verify(app, 'release capacity', 'API rename capacity;originals/fresh=true;actualReleased=true')
    history(app)
    disks(root)
    finish(app)
    print('PASS: eight public rename callbacks hold the real shared lane, ninth refuses and original/fresh typed queries recover only after actual settlement, with dirty source and protected disks intact')


def native_lsp(root):
    files(root)
    config = configuration(root)
    python = str(Path(sys.executable).resolve())
    server = str((Path(__file__).parent / 'fixtures/lsp_server.py').resolve())
    app = Editor(root, '--lsp', python, '--lsp-language', 'cpp', '--lsp-arg', server,
                 '--config-dir', config, '--no-session', root / 'source.cpp', enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Language server ready', 'source.cpp')
    app.paste('dirty ')
    app.send(F2)
    wait_screen(app, 'Rename Symbol · Enter applies')
    app.send(CTRL_A)
    app.paste('renamed')
    app.send(b'\r')
    wait_screen(app, 'Renamed atomically · Undo restores the workspace', 'renamedirty', absent=('Rename Symbol',))
    disks(root)
    app.send(CTRL_Z)
    wait_screen(app, 'dirty 猫🙂 source', absent=('renamedirty',))
    app.send(REDO)
    wait_screen(app, 'renamedirty')
    app.send(CTRL_S)
    wait_screen(app, 'Saved source.cpp', absent=('source.cpp *',))
    assert (root / 'source.cpp').read_bytes() == ('renamedirty '+SOURCE).encode()
    for name, value in [('target.cpp', TARGET), ('hidden.cpp', HIDDEN), ('unopened.cpp', UNOPENED)]:
        assert (root / name).read_bytes() == value.encode()
    app.send(CTRL_Z)
    wait_screen(app, 'dirty 猫🙂 source', absent=('renamedirty',))
    assert (root / 'source.cpp').read_bytes() == ('renamedirty '+SOURCE).encode()
    finish(app)
    print('PASS: actual framed native LSP F2 fallback/rename, keyboard Undo/Redo and explicit CRLF Save work with no extension/Node and empty PATH; Undo keeps committed disk bytes and dirty history')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-rename-provider-') as directory:
        try:
            for name, run in [('queries', queries), ('grouped', grouped), ('single', single),
                              ('held', held), ('capacity', capacity), ('native-lsp', native_lsp)]:
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
                        assert app.process.restored, 'Rename interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
