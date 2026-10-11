#!/usr/bin/env python3
"""Actual public snippet APIs, native linked-field keys and protected shared histories."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, CTRL_SHIFT_W, eventually, wait_screen

ORIGINAL = '猫🙂 base\r\nnext\r\n'
REDO = b'\x1b[122;6u'


def unchanged(root):
    assert (root / 'a.cpp').read_bytes() == ORIGINAL.encode('utf-8')
    assert (root / 'b.txt').read_bytes() == b'other\r\n'


def ready(root):
    (root / 'a.cpp').write_bytes(ORIGINAL.encode('utf-8'))
    (root / 'b.txt').write_bytes(b'other\r\n')
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/editor-snippets-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual snippet API qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'a.cpp',
                 enhanced=True, extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.cpp')
    app.paste('dirty ')
    return app


def verify(app, query, receipt):
    command(app, query)
    wait_screen(app, receipt)


def linked(root):
    app = ready(root)
    verify(app, 'Editor snippets linked builder', 'API snippets linked;builder=true;events=true;UTF16=true')
    app.paste('x')
    verify(app, 'Editor snippets verify linked typing', 'API snippets typed;linked=true')
    app.send(b'\t')
    verify(app, 'Editor snippets verify second field', 'API snippets Tab;field=2;UTF16=true')
    app.send(b'\x1b[Z')
    verify(app, 'Editor snippets verify back field', 'API snippets BackTab;field=1;linked=true')
    app.send(b'\x1b')
    verify(app, 'Editor snippets verify escaped field', 'API snippets Escape;secondary=removed')
    app.paste('q')
    verify(app, 'Editor snippets verify single field typing', 'API snippets escaped typing;firstOnly=true')
    for query, receipt in [
        ('Editor snippets verify typed history', 'API snippets typed Undo;source=true'),
        ('Editor snippets verify expansion history', 'API snippets expansion Undo;source=true'),
        ('Editor snippets verify dirty history', 'API snippets dirty history;source=true'),
        ('Editor snippets verify clean history', 'API snippets original Undo;clean=true'),
    ]:
        app.send(CTRL_Z)
        verify(app, query, receipt)
    for query, receipt in [
        ('Editor snippets verify dirty history', 'API snippets dirty history;source=true'),
        ('Editor snippets verify expansion history', 'API snippets expansion Undo;source=true'),
        ('Editor snippets verify typed history', 'API snippets typed Undo;source=true'),
        ('Editor snippets verify single field typing', 'API snippets escaped typing;firstOnly=true'),
    ]:
        app.send(REDO)
        verify(app, query, receipt)
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: public SnippetString builder publishes linked Unicode fields before true; native typing/Tab/BackTab/Escape and Undo/Redo preserve CRLF baseline/disks')


def arrays(root):
    app = ready(root)
    verify(app, 'Editor snippets inactive range', 'API snippets inactive;focus=second;session=target;events=true')
    verify(app, 'Editor snippets show first pane', 'API snippets first pane;selection=original')
    verify(app, 'Editor snippets location overloads', 'API snippets overloads;omitted/empty/Position/Range/arrays=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify array undo', 'API snippets array Undo;both=true')
    app.send(REDO)
    verify(app, 'Editor snippets verify array final', 'API snippets array Redo;both=true')
    for _ in range(8):
        app.send(CTRL_Z)
    verify(app, 'Editor snippets verify dirty history', 'API snippets dirty history;source=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify clean history', 'API snippets original Undo;clean=true')
    app.send(REDO)
    verify(app, 'Editor snippets verify dirty history', 'API snippets dirty history;source=true')
    for _ in range(8):
        app.send(REDO)
    verify(app, 'Editor snippets verify array final', 'API snippets array Redo;both=true')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: inactive shared pane insertion focuses its own editor; omitted/empty selections and Position/Range arrays change exact locations with native per-call grouped Undo/Redo')


def refusals(root):
    app = ready(root)
    verify(app, 'Editor snippets inactive range', 'API snippets inactive;focus=second;session=target;events=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify dirty history', 'API snippets dirty history;source=true')
    verify(app, 'Editor snippets reentrant source', 'API snippets stale getter;insert=refused;edit=retained')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify dirty history', 'API snippets dirty history;source=true')
    verify(app, 'Editor snippets retired and readonly', 'API snippets retired=reject;readonly=false;source=unchanged')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify clean history', 'API snippets original Undo;clean=true')
    app.send(REDO)
    verify(app, 'Editor snippets verify dirty history', 'API snippets dirty history;source=true')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: stale reentrant source refuses snippet while retaining intentional edit/history; retired editor rejects and real readonly virtual insertion returns false with no source effects')



def options(root):
    app = ready(root)
    verify(app, 'Editor snippets prepare options', 'API snippets options;separateUntitled=true;CRLF=true')
    verify(app, 'Editor snippets default whitespace', 'API snippets default whitespace;childIndent=4;events=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify options initial', 'API snippets options Undo;initial=true;dirty=true')
    verify(app, 'Editor snippets keep whitespace', 'API snippets keepWhitespace;childIndent=2;events=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify options initial', 'API snippets options Undo;initial=true;dirty=true')
    verify(app, 'Editor snippets false undo stops', 'API snippets false stops;twoInsertions=true;openBoundary=true')
    # Ordinary native typing, rather than bracketed paste, extends the open boundary.
    app.send(b'C')
    verify(app, 'Editor snippets verify options typing', 'API snippets false stops typing;ABC=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify options initial', 'API snippets options Undo;initial=true;dirty=true')
    app.send(REDO)
    verify(app, 'Editor snippets verify options typing', 'API snippets false stops typing;ABC=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify options initial', 'API snippets options Undo;initial=true;dirty=true')
    verify(app, 'Editor snippets restore protected original', 'API snippets options protected original;identity=true;history=true')
    app.send(CTRL_Z)
    verify(app, 'Editor snippets verify clean history', 'API snippets original Undo;clean=true')
    app.send(REDO)
    verify(app, 'Editor snippets verify dirty history', 'API snippets dirty history;source=true')
    unchanged(root)
    # Supplied untitled content has its own dirty baseline even after complete Undo.
    app.send(CTRL_SHIFT_W)
    for _ in range(2):
        wait_screen(app, 'Unsaved Changes')
        app.send(b'd')
    eventually(lambda: app.read() and app.process.poll() is not None)
    app.finish()
    unchanged(root)
    print('PASS: default vs keepWhitespace preserve exact CRLF indentation; false undo stops group two API insertions plus native typing while protected original identity/history/disks survive')

if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-editor-snippets-') as directory:
        try:
            for name, run in [('linked', linked), ('arrays', arrays), ('refusals', refusals), ('options', options)]:
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
                        assert app.process.restored, 'Snippet interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
