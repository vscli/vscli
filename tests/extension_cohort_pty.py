#!/usr/bin/env python3
"""A 90-package, 1800-command session through actual native input, save and restart."""
import hashlib
import json
from pathlib import Path
import tempfile
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-cohort-pty-") as temporary:
        root = Path(temporary)
        arguments = []
        sources = {}
        for index in range(90):
            name = f"p{index:03}"
            folder = root / name
            folder.mkdir()
            manifest = {"publisher": "cohort", "name": name, "version": "1.0.0", "main": "main.cjs",
                "extensionDependencies": [f"cohort.p{index+1:03}"] if index < 89 else [],
                "padding": [0]*600,
                "contributes": {"commands": [{"command": f"cohort.{name}.c{i}", "title": f"Cohort {index} command {i}"} for i in range(20)]}}
            (folder / "package.json").write_text(json.dumps(manifest))
            (folder / "main.cjs").write_text("""const v=require('vscode');
exports.activate=context=>{for(let i=0;i<20;i++)context.subscriptions.push(v.commands.registerCommand('cohort.NAME.c'+i,async()=>{
 const ok=await v.window.activeTextEditor.edit(edit=>edit.insert(new v.Position(0,0),'cohort '+v.extensions.all.length+' '));
 await v.window.showInformationMessage('large cohort edit='+ok);
}));return {id:'cohort.NAME'};};""".replace("NAME", name))
            for path in folder.iterdir():
                sources[path] = hashlib.sha256(path.read_bytes()).digest()
            arguments += ["--extension", folder]
        keys = root / "keybindings.json"
        keys.write_text(json.dumps([{"key":"f9", "command":"cohort.p089.c19"}]))
        file = root / "native.txt"
        original = "λ🙂 original\r\n".encode()
        file.write_bytes(original)
        app = Editor(root, "--config-dir", root / "config", "--keybindings", keys, *arguments, file, enhanced=True)
        try:
            wait_screen(app, "1800 commands", timeout=20)
            app.send(b"\x1b[20~")
            wait_screen(app, "large cohort edit=true")
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == b"cohort 90 " + original)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.send(b"\x1bOP")
            app.send("Extensions: Restart Selected Session\r")
            wait_screen(app, "1800 commands", timeout=20)
            app.send(b"\x1b[20~")
            wait_screen(app, "large cohort edit=true")
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == b"cohort 90 " + original)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.finish()
        except BaseException:
            if app.process.poll() is None:
                app.process.kill()
                app.process.wait()
            app.close_fds()
            raise
        for path, digest in sources.items():
            assert hashlib.sha256(path.read_bytes()).digest() == digest, path
        print("PASS: 90 code packages, 1800 commands, 90-deep dependency activation, last-package binding, Unicode/CRLF save/Undo, restart and unchanged sources")


if __name__ == "__main__":
    run()
