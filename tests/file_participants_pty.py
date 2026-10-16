#!/usr/bin/env python3
"""Native will-file participants: review, shared history, policy and cancellation."""
import json
from pathlib import Path
import shutil
import tempfile
from extension_sessions_pty import command
from pty_smoke import Editor, CTRL_A, CTRL_S, CTRL_Z, eventually, wait_screen

ORIGINAL = 'original 猫🙂\r\n'
DIRTY = 'dirty ' + ORIGINAL
REDO = b'\x1b[122;6u'
EDITOR = b'\x1b[49;5u'
BACKTAB = b'\x1b[Z'


def fixture(base, mode='text', timeout=60_000):
    root = base / 'workspace'; root.mkdir()
    profile = base / 'profile'; profile.mkdir()
    extension = base / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/file-participants-extension', extension)
    (root / 'a.cpp').write_bytes(ORIGINAL.encode())
    (root / 'peer.cpp').write_bytes('peer 猫🙂\r\n'.encode())
    (root / 'mode').write_text(mode)
    (profile / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'files.participants.timeout': timeout,
        'files.autoSave': 'off', 'breadcrumbs.enabled': False,
    }))
    node = shutil.which('node'); assert node, 'Participant qualification requires Node'
    data = base / 'data'; data.mkdir()
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', profile, '--no-session', root / 'a.cpp', enhanced=True,
                 extra_env={'PATH': '', 'XDG_DATA_HOME': str(data)})
    wait_screen(app, 'Extension ready:', 'a.cpp')
    app.paste('dirty '); wait_screen(app, 'dirty original', 'a.cpp *')
    return app, root, profile


def rename(app, destination):
    command(app, 'Explorer: Rename Selected Item')
    app.send(CTRL_A); app.paste(str(destination)); app.send(b'\r')


def records(root):
    return [json.loads(line) for line in (root / 'log').read_text().splitlines()] if (root / 'log').exists() else []


def renamed(app, root):
    eventually(lambda: app.read() and (root / 'b.cpp').exists()
               and not (root / 'a.cpp').exists() and any(row.get('did') for row in records(root)))


def apply_and_remember(base):
    app, root, profile = fixture(base)
    completed = False
    try:
        rename(app, root / 'b.cpp'); wait_screen(app, 'Additional file edits', '[Apply]', '[Preview]')
        assert (root / 'a.cpp').read_bytes() == ORIGINAL.encode()
        assert not (root / 'b.cpp').exists()
        app.send(BACKTAB); app.send(b' '); wait_screen(app, '[x] Remember this choice')
        app.send(b'\t\r'); renamed(app, root)
        event = next(row for row in records(root) if row.get('did'))
        assert event['text'] == 'participant ' + DIRTY
        assert event['peer'] == 'participant peer 猫🙂\r\n'
        assert (root / 'b.cpp').read_bytes() == ORIGINAL.encode()
        assert (root / 'peer.cpp').read_bytes() == 'peer 猫🙂\r\n'.encode()
        app.send(EDITOR); app.send(CTRL_Z)
        wait_screen(app, 'dirty original', absent=('participant dirty original',))
        app.send(REDO); wait_screen(app, 'participant dirty original')
        app.send(CTRL_S)
        eventually(lambda: app.read() and (root / 'b.cpp').read_bytes() == ('participant ' + DIRTY).encode())
        wait_screen(app, 'Saved', absent=('b.cpp *',))
        choice = profile / 'file-participant-preferences.json'
        eventually(lambda: app.read() and choice.exists())
        assert json.loads(choice.read_text())['values']['file.particpants.additionalEdits'] is False
        app.finish(discard=True); completed = True
    finally:
        if not completed: app.finish(discard=True)
    print('PASS: will-rename review and remember, shared hidden text before did event, grouped Undo/Redo, exact CRLF Save')


def preview_and_skip(base):
    app, root, _profile = fixture(base, mode='confirm')
    completed = False
    try:
        rename(app, root / 'b.cpp'); wait_screen(app, 'Additional file edits', '[Preview]')
        assert '[Apply]' not in app.screen.text(), 'Confirmation requires preview'
        app.send(b'\r'); wait_screen(app, 'Review proposed file edits', 'proposed replacement', '+ participant')
        assert not (root / 'b.cpp').exists()
        app.send(b'\x1b'); renamed(app, root)
        assert (root / 'b.cpp').read_bytes() == ORIGINAL.encode()
        event = next(row for row in records(root) if row.get('did'))
        assert event['text'] == DIRTY
        app.send(EDITOR); app.send(CTRL_Z); wait_screen(app, 'original 猫🙂', absent=('dirty original',))
        app.send(REDO); wait_screen(app, 'dirty original')
        app.finish(discard=True); completed = True
    finally:
        if not completed: app.finish(discard=True)
    print('PASS: confirmation forces native range preview; Escape skips contributions, original rename and dirty history survive')


def cancel_callback(base):
    app, root, _profile = fixture(base, mode='hold')
    completed = False
    try:
        rename(app, root / 'b.cpp')
        eventually(lambda: app.read() and any(row.get('will') for row in records(root)))
        app.send(b'\x1b'); renamed(app, root)
        assert (root / 'b.cpp').read_bytes() == ORIGINAL.encode()
        (root / 'release').write_text('release')
        app.send(EDITOR); app.paste('responsive ')
        wait_screen(app, 'dirty responsive original')
        app.send(CTRL_S)
        expected = 'dirty responsive ' + ORIGINAL
        eventually(lambda: app.read() and (root / 'b.cpp').read_bytes() == expected.encode())
        app.finish(); completed = True
    finally:
        (root / 'release').write_text('release')
        if not completed: app.finish(discard=True)
    print('PASS: Escape cancels a held real callback, original rename continues, native editing and Save remain usable')


def create_delete(base):
    app, root, _profile = fixture(base)
    completed = False
    try:
        created = root / 'created.cpp'
        command(app, 'Explorer: New File'); app.send(CTRL_A); app.paste(str(created)); app.send(b'\r')
        wait_screen(app, 'Additional file edits'); assert not created.exists()
        app.send(b'\x1b')
        eventually(lambda: app.read() and created.exists() and any(row.get('did') and row.get('kind') == 'create' for row in records(root)))
        command(app, 'Explorer: Move Selected Item to Trash'); wait_screen(app, 'Trash'); app.send(b'y')
        wait_screen(app, 'Additional file edits'); assert (root / 'a.cpp').exists()
        app.send(b'\x1b')
        eventually(lambda: app.read() and not (root / 'a.cpp').exists()
                   and any(row.get('did') and row.get('kind') == 'delete' for row in records(root)))
        assert [row['kind'] for row in records(root)] == ['create', 'create', 'delete', 'delete']
        assert created.exists()
        app.finish(discard=True); completed = True
    finally:
        if not completed: app.finish(discard=True)
    print('PASS: native create/delete will events precede file effects, skipped contributions preserve dirty data, isolated trash')


def run():
    for journey in [apply_and_remember, preview_and_skip, cancel_callback, create_delete]:
        with tempfile.TemporaryDirectory(prefix='vscli-file-participants-pty-') as directory:
            journey(Path(directory).resolve())


if __name__ == '__main__': run()
