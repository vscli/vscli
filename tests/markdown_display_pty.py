#!/usr/bin/env python3
"""Source-correct MarkdownString rendered inertly without changing native work."""
import json
from pathlib import Path
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_S, CTRL_Z, eventually, wait_screen

ORIGINAL='α😀\r\nsecond'
DIRTY='dirty '+ORIGINAL
FIXTURE=Path(__file__).resolve().parent/'fixtures'/'markdown-display-extension'


def save(app,file,expected):
    app.send(CTRL_S)
    eventually(lambda: app.read() and file.read_bytes()==expected.encode())
    wait_screen(app,'Saved',absent=(file.name+' *',))


def run(root):
    file=root/'input.txt';file.write_bytes(ORIGINAL.encode())
    app=Editor(root,'--extension',FIXTURE,'--no-session',file,enhanced=True)
    wait_screen(app,'Extension ready:','input.txt')
    app.send(b'\x1b[H');app.paste('dirty ')
    command(app,'Markdown inspect unchanged dirty model')
    wait_screen(app,'Markdown display: original dirty model intact')
    # Original Ctrl+K Ctrl+I, invoked once. Prepared callbacks confirm identity.
    app.send(b'\x1b[107;5u\x1b[105;5u')
    wait_screen(app,'Extension Hover','spaces  tab tilde ~x~','- literal 猫🙂',
                '&nbsp; \\*literal\\* [Run](command:unsafe)','MARKED CODE &nbsp;')
    screen=app.screen.text(); assert 'spaces&nbsp;' not in screen; assert '\\~x\\~' not in screen
    assert file.read_bytes()==ORIGINAL.encode()
    app.send(b'\x1b')
    app.send(b'\x1b[32;5u')  # original Ctrl+Space
    wait_screen(app,'Suggestions · Tab accepts','DocChoice','spaces  tab tilde ~x~')
    assert file.read_bytes()==ORIGINAL.encode();app.send(b'\x1b[B')
    wait_screen(app,'PlainChoice','PLAINTEXT &nbsp; \\*literal\\*')
    app.send(b'\x1b')
    app.send(b'\x1b[32;6u')  # original Ctrl+Shift+Space
    wait_screen(app,'Parameter Hints','fixture(猫)','SIGNATURE spaces  tab ~literal~')
    assert file.read_bytes()==ORIGINAL.encode();app.send(b'\x1b')
    command(app,'Markdown inspect unchanged dirty model')
    wait_screen(app,'Markdown display: original dirty model intact')
    trace=[json.loads(line) for line in (root/'markdown-callbacks.jsonl').read_text().splitlines()]
    for method in ('hover','completion','signature'):
        rows=[row for row in trace if row['method']==method]
        assert rows and all(row['text']==DIRTY and row['dirty'] for row in rows),method
    assert not (root/'unsafe-executed').exists(), 'Markdown link command executed'
    save(app,file,DIRTY);app.send(CTRL_Z);save(app,file,ORIGINAL)
    app.finish()
    print('PASS: actual extension appendText and safe code fences project in hover/completion/signature; literal code retained, dirty native model/UnicodeCRLF/Undo/disk and terminal restored')


if __name__=='__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-markdown-display-pty-') as directory:
        try:
            run(Path(directory))
        finally:
            failed=sys.exc_info()[0] is not None; errors=[]
            for app in LIVE[:]:
                try:
                    stop_surface_process(app);assert app.process.restored,'Terminal mode leaked';app.close_fds();LIVE.remove(app)
                except Exception as error: errors.append(error)
            if errors:
                if failed:print(f'Additional cleanup errors: {errors}',file=sys.stderr)
                else:raise errors[0]
