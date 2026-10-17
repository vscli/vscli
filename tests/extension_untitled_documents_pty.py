#!/usr/bin/env python3
"""Public API-created native untitled identities, real SaveAs and no-clobber saves."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor,LIVE,command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_A,CTRL_Z,CTRL_SHIFT_W,eventually,wait_screen

ORIGINAL='猫🙂 A\r\nlast\r\n'
INITIAL='猫🙂 generated\r\nint first = 1;\r\n'
ASSOCIATED='猫🙂 associated\nint associated = 2;\n'
PROTECTED=b'existing bytes must survive\r\n'


def ready(root):
    (root/'a.txt').write_bytes(ORIGINAL.encode('utf-8'))
    extension=root/'extension';shutil.copytree(Path(__file__).parent/'fixtures/untitled-document-extension',extension)
    config=root/'config';config.mkdir()
    (config/'settings.json').write_text(json.dumps({'vscli.languageServer.enabled':False,'breadcrumbs.enabled':False,
        'files.autoSave':'off','editor.formatOnSave':False,'editor.codeActionsOnSave':{}}),encoding='utf-8')
    node=shutil.which('node')
    if not node:raise AssertionError('Real untitled gateway qualification requires Node')
    app=Editor(root,'--extension',extension,'--extension-node',str(Path(node).resolve()),'--config-dir',config,'--no-session',root/'a.txt',enhanced=True,extra_env={'PATH':''})
    wait_screen(app,'Extension ready:','a.txt');app.paste('dirty ');return app


def original_history(app,root,discard_count=1):
    command(app,'Untitled API restore original');wait_screen(app,'API untitled original restored;dirty=true;identity=true','a.txt *')
    app.send(CTRL_Z);command(app,'Untitled API verify original clean');wait_screen(app,'API untitled original clean history retained',absent=('a.txt *',))
    app.send(b'\x1b[122;6u');command(app,'Untitled API verify original dirty');wait_screen(app,'API untitled original dirty history retained','a.txt *')
    assert (root/'a.txt').read_bytes()==ORIGINAL.encode('utf-8')
    # Each dirty retained model has its own real Save/Discard/Cancel prompt,
    # including the two empty hidden models created by noarg/options opens.
    app.send(CTRL_SHIFT_W)
    for _ in range(discard_count):
        wait_screen(app,'Unsaved Changes');app.send(b'd')
    eventually(lambda:app.read() and app.process.poll() is not None)
    app.finish()
    assert (root/'a.txt').read_bytes()==ORIGINAL.encode('utf-8')


def options(root):
    app=ready(root);command(app,'Untitled API content options');wait_screen(app,'API untitled options;hidden=true;fresh=true;CRLF=true')
    command(app,'Untitled API edit supplied content');wait_screen(app,'API untitled edit=true;event=true')
    app.send(CTRL_Z);command(app,'Untitled API verify initial content');wait_screen(app,'API untitled initial retained;dirty=true')
    # Exhausting native Undo must not remove API-provided initial content.
    app.send(CTRL_Z);command(app,'Untitled API verify initial content');wait_screen(app,'API untitled initial retained;dirty=true')
    command(app,'Untitled API edit supplied content');wait_screen(app,'API untitled edit=true;event=true')
    command(app,'Untitled API public undo');wait_screen(app,'API untitled public undo;initial=true;dirty=true')
    command(app,'Untitled API save options');wait_screen(app,'Save As (existing files are protected)');app.send(b'\x1b')
    wait_screen(app,'API untitled cancelled;dirty=true;event=false')
    target=root/'options-saved.cpp';assert not target.exists()
    command(app,'Untitled API save options');wait_screen(app,'Save As (existing files are protected)')
    app.send(CTRL_A);app.paste(str(target));app.send(b'\r')
    wait_screen(app,'API untitled saved;transition=true;events=close/open/save','options-saved.cpp')
    assert target.read_bytes()==INITIAL.encode('utf-8')
    command(app,'Untitled API edit saved file');wait_screen(app,'API new file edited;dirty=true;oldClosed=true')
    app.send(CTRL_Z);command(app,'Untitled API verify saved file clean');wait_screen(app,'API new file undo;clean=true;oldClosed=true')
    app.send(b'\x1b[122;6u');command(app,'Untitled API verify saved file dirty');wait_screen(app,'API new file redo;dirty=true;oldClosed=true')
    assert target.read_bytes()==INITIAL.encode('utf-8')
    app.send(CTRL_Z);command(app,'Untitled API verify saved file clean');wait_screen(app,'API new file undo;clean=true;oldClosed=true')
    original_history(app,root,discard_count=3)
    print('PASS: API options/noarg/empty opens retain fresh hidden identities and initial CRLF Undo; real SaveAs Cancel/accept publishes close-old/open-new/save-new before true and preserves native history')


def associated(root):
    app=ready(root);command(app,'Untitled API associated path');wait_screen(app,'API associated untitled;cached=true;cpp=true')
    assert not (root/'new.cpp').exists();command(app,'Untitled API save associated path')
    wait_screen(app,'API associated saved;direct=true;transition=true;events=close/open/save','new.cpp',absent=('Save As (existing files are protected)',))
    assert (root/'new.cpp').read_bytes()==ASSOCIATED.encode('utf-8');original_history(app,root)
    print('PASS: associated untitled repeat-open preserves edited shared identity/cpp; direct create-only native Save publishes close-old/open-new/save-new without SaveAs')


def protected(root):
    app=ready(root);(root/'exists.cpp').write_bytes(PROTECTED)
    command(app,'Untitled API reject existing target');wait_screen(app,'API associated existing rejected;original=true')
    assert (root/'exists.cpp').read_bytes()==PROTECTED
    command(app,'Untitled API late target open');wait_screen(app,'API associated late target ready;dirty=true')
    (root/'late.cpp').write_bytes(PROTECTED)
    command(app,'Untitled API protected late save');wait_screen(app,'API associated late save refused;dirty=true;event=false',absent=('Save As (existing files are protected)',))
    assert (root/'late.cpp').read_bytes()==PROTECTED;original_history(app,root,discard_count=2)
    assert (root/'exists.cpp').read_bytes()==PROTECTED;assert (root/'late.cpp').read_bytes()==PROTECTED
    print('PASS: existing and newly appeared associated destinations refuse overwrite, preserve dirty identity and original ordinary shared Undo/Redo/disks')


if __name__=='__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-untitled-api-') as directory:
        try:
            for name,journey in [('options',options),('associated',associated),('protected',protected)]:
                root=Path(directory).resolve()/name;root.mkdir();journey(root)
        finally:
            failed=sys.exc_info()[0] is not None;errors=[]
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app);assert app.process.restored,'Untitled interruption leaked terminal mode';app.close_fds();LIVE.remove(app)
                    else:app.finish()
                except Exception as error:errors.append(error)
            if errors and not failed:raise errors[0]
