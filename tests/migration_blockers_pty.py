#!/usr/bin/env python3
"""Known host admission failures appear before migration grants or publication."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from pty_smoke import BINARY, CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from profile_migration_pty import command, finish_after_failure, fixture


def choose(app, user, extensions):
    command(app, "Preferences: Import from VS Code")
    wait_screen(app, "custom User directory")
    app.send("p")
    wait_screen(app, "User directory path")
    app.send(str(user) + "\r")
    wait_screen(app, "Custom VS Code / Default")
    app.send("e")
    wait_screen(app, "extensions directory path")
    app.send(str(extensions) + "\r")
    wait_screen(app, "Review VS Code Import", "Code activation blocked for 1", timeout=15)


def snapshot(directory):
    return {str(path.relative_to(directory)): None if path.is_dir() else path.read_bytes()
        for path in directory.rglob("*")}


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-migration-blockers-") as temporary:
        root = Path(temporary)
        user, extensions = fixture(root)
        manifest = extensions / "fixture.native-1.0.0/package.json"
        value = json.loads(manifest.read_text())
        value["enabledApiProposals"] = ["fixtureProposal"]
        manifest.write_text(json.dumps(value))
        originals = {path: hashlib.sha256(path.read_bytes()).digest()
            for directory in [user, extensions] for path in directory.rglob("*") if path.is_file()}
        document = root / "document.txt"
        original = "λ🙂 original\r\n".encode()
        document.write_bytes(original)
        native, sentinel = root / "native", root / "must-not-activate"
        app = Editor(root, "--config-dir", native, "--extensions-dir", root / "empty-store",
            "--extension-node", root / "no-node", document, enhanced=True,
            extra_env={"VSCLI_MIGRATION_SENTINEL": str(sentinel)})
        try:
            app.send(b"\x1b[F")
            app.send(" dirty")
            choose(app, user, extensions)
            before = snapshot(native)
            app.send("a\r")
            wait_screen(app, "Migration failed", "Imported code activation is blocked", timeout=15)
            assert not (native / "active-profile.json").exists()
            assert snapshot(native) == before
            assert document.read_bytes() == original and not sentinel.exists()
            app.send(b"\x1b")
            wait_screen(app, "λ🙂 original dirty")
            choose(app, user, extensions)
            app.send("\r")
            wait_screen(app, "VS Code Migration Complete", "1 local extensions", "0 code execution grants", timeout=15)
            app.send(b"\x1b")
            app.send(b"\x1b[17~")
            app.send(CTRL_S)
            eventually(lambda: app.read() and document.read_bytes() == "λ🙂 original dirty imported\r\n".encode())
            for expected in ["λ🙂 original dirty\r\n".encode(), original]:
                app.send(CTRL_Z)
                app.send(CTRL_S)
                eventually(lambda: app.read() and document.read_bytes() == expected)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        active = native / "imports" / json.loads((native / "active-profile.json").read_text())["profile"]
        report = json.loads((active / "extension-migration.json").read_text())
        assert len(report["selected"][0]["runtime_blockers"]) == 1
        assert not any(json.loads((active / "extensions-enabled.json").read_text()).get("extensions", {}).values())
        assert not sentinel.exists()
        assert all(hashlib.sha256(path.read_bytes()).digest() == digest for path, digest in originals.items())
        print("PASS: native proposed-API blocker review and pre-publication rejection preserve dirty models; explicit copy without grants retains all package/configuration bytes and Unicode/CRLF save/Undo without Node")
        destination = root / "cli-native"
        result = subprocess.run([BINARY, "--import-vscode", str(user), "--vscode-extensions", str(extensions),
            "--config-dir", str(destination), "--import-extensions", "--activate-imported-extensions", "--apply-import"],
            capture_output=True, timeout=20)
        assert result.returncode != 0 and b"fixture.native" in result.stderr and b"proposed APIs" in result.stderr
        assert not destination.exists()
        print("PASS: CLI activation preflight lists the blocking package before writing the destination")


if __name__ == "__main__":
    run()
