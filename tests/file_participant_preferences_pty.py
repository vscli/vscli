#!/usr/bin/env python3
"""Native profile-choice reset works on the empty screen and survives restart."""
import json
from pathlib import Path
import tempfile
from extension_sessions_pty import command
from pty_smoke import Editor, eventually, wait_screen


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-file-choice-pty-") as directory:
        base=Path(directory).resolve();root=base/"workspace";root.mkdir()
        profile=base/"profile";profile.mkdir();settings=profile/"settings.json";settings.write_text("{}")
        path=profile/"file-participant-preferences.json"
        key="file.particpants.additionalEdits"
        path.write_text(json.dumps({"schema":1,"values":{key:True,"unrelated":{"text":"猫🙂"}}}))
        def values():return json.loads(path.read_text())["values"]
        for restart in range(2):
            if restart:
                path.write_text(json.dumps({"schema":1,"values":{key:False,"unrelated":{"text":"猫🙂"}}}))
            app=Editor(root,"--config-dir",profile,"--settings",settings,"--no-session",enhanced=True,extra_env={"PATH":""})
            completed=False
            try:
                wait_screen(app,"Open File","Command Palette")
                command(app,"Reset choice for 'File operation needs preview'")
                eventually(lambda:app.read() and key not in values())
                wait_screen(app,"File-operation preview choice reset")
                assert values()=={"unrelated":{"text":"猫🙂"}}
                assert not (root/"file-participant-preferences.json").exists()
                app.finish();completed=True
            finally:
                if not completed:app.finish(discard=True)
    print("PASS: native reset choice from blank screen, no Node, true/false profiles, restart, unrelated Unicode state and terminal restoration")


if __name__=="__main__":run()
