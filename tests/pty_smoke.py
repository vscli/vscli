#!/usr/bin/env python3
"""Exercise the actual executable through a Unix PTY, using only Python stdlib.

Run: cargo build && python3 tests/pty_smoke.py target/debug/vscli
All file edits and recovery snapshots use temporary directories.
"""
import codecs
import re
import unicodedata
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

BINARY = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/vscli").resolve())
CTRL_S = b"\x13"
CTRL_A = b"\x01"
CTRL_Z = b"\x1a"
CTRL_P = b"\x10"
CTRL_SHIFT_W = b"\x1b[119;6u"
CTRL_SHIFT_S = b"\x1b[115;6u"


def eventually(predicate, timeout=4):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.025)
    raise AssertionError("Timed out waiting for expected state")


class Screen:
    """Small display model for the cursor-addressed ANSI subset emitted by Ratatui."""
    def __init__(self):
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.pending = ""
        self.cells = {}
        self.row = self.col = 0

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith("\x1b["):
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if not match:
                    return
                raw, _, code = match.groups()
                self.pending = self.pending[match.end():]
                args = [int(x) if x.isdigit() else 0 for x in raw.split(";")]
                n = args[0] or 1
                if code in ("H", "f"):
                    self.row = max(0, args[0] - 1)
                    self.col = max(0, (args[1] if len(args) > 1 else 1) - 1)
                elif code == "G":
                    self.col = n - 1
                elif code == "d":
                    self.row = n - 1
                elif code in "ABCD":
                    dr, dc = {"A": (-n, 0), "B": (n, 0), "C": (0, n), "D": (0, -n)}[code]
                    self.row, self.col = max(0, self.row + dr), max(0, self.col + dc)
                elif code == "J" and args[0] in (2, 3):
                    self.cells.clear()
                elif code == "K":
                    for pos in list(self.cells):
                        if pos[0] == self.row and (args[0] == 2 or (args[0] == 0 and pos[1] >= self.col) or (args[0] == 1 and pos[1] <= self.col)):
                            del self.cells[pos]
                continue
            if self.pending.startswith("\x1b"):
                if len(self.pending) < 2:
                    return
                self.pending = self.pending[2:]
                continue
            ch, self.pending = self.pending[0], self.pending[1:]
            if ch == "\r":
                self.col = 0
            elif ch == "\n":
                self.row += 1
            elif ch == "\b":
                self.col = max(0, self.col - 1)
            elif ord(ch) >= 32:
                if unicodedata.combining(ch):
                    continue
                self.cells[(self.row, self.col)] = ch
                width = 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
                if width == 2:
                    self.cells[(self.row, self.col + 1)] = ""
                self.col += width

    def text(self):
        return "\n".join("".join(self.cells.get((row, col), " ") for col in range(250)).rstrip() for row in range(80))


class SupervisedProcess:
    def __init__(self, supervisor, report):
        self.supervisor = supervisor
        self.report = os.fdopen(report, "r")
        if not select.select([self.report], [], [], 4)[0]:
            supervisor.kill()
            supervisor.wait(timeout=3)
            raise AssertionError("PTY supervisor did not start")
        self.pid = json.loads(self.report.readline())["pid"]
        self.returncode = None
        self.restored = False

    def poll(self):
        if self.returncode is None and select.select([self.report], [], [], 0)[0]:
            line = self.report.readline()
            if line:
                result = json.loads(line)
                self.returncode = result["status"]
                self.restored = result["restored"]
            elif self.supervisor.poll() is not None:
                raise AssertionError("PTY supervisor exited without a status report")
        return self.returncode

    def kill(self):
        if self.poll() is None:
            os.kill(self.pid, signal.SIGKILL)

    def wait(self, timeout=3):
        eventually(lambda: self.poll() is not None, timeout=timeout)
        self.supervisor.wait(timeout=timeout)
        return self.returncode

    def close(self):
        self.supervisor.wait(timeout=3)
        self.report.close()


class Editor:
    def __init__(self, root, *args, recovery=False, enhanced=False, extra_env=None):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 110, 0, 0))
        self.original_termios = termios.tcgetattr(self.slave)

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        options = ["--no-mouse", "--keymap", "linux"]
        if not enhanced:
            options += ["--legacy-keys"]
        if not recovery:
            options += ["--no-recovery"]
        report_read, report_write = os.pipe()
        supervisor = Path(__file__).resolve().parent / "fixtures" / "pty_supervisor.py"
        process = subprocess.Popen(
            [sys.executable, str(supervisor), str(report_write), BINARY, *options, *map(str, args)], cwd=root,
            stdin=self.slave, stdout=self.slave, stderr=self.slave,
            preexec_fn=controlling_terminal,
            pass_fds=(report_write,),
            env={**os.environ, "TERM": "xterm-256color", "NO_COLOR": "", **(extra_env or {})},
        )
        os.close(report_write)
        self.process = SupervisedProcess(process, report_read)
        self.output = bytearray()
        self.screen = Screen()
        self.enhanced = enhanced
        try:
            eventually(lambda: self.read() and b"VSCLI" in self.output)
        except AssertionError:
            self.process.kill()
            self.process.wait(timeout=3)
            raise AssertionError(f"Startup failed: {bytes(self.output)!r}") from None

    def read(self):
        while select.select([self.master], [], [], 0)[0]:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                break
            if not chunk:
                break
            self.output.extend(chunk)
            self.screen.feed(chunk)
            if b"\x1b[6n" in chunk:
                os.write(self.master, b"\x1b[1;1R")
            # Respond like a terminal supporting Kitty keyboard negotiation.
            if self.enhanced and b"\x1b[?u" in chunk:
                os.write(self.master, b"\x1b[?0u\x1b[?1;2c")
        return True

    def send(self, data):
        os.write(self.master, data.encode() if isinstance(data, str) else data)
        time.sleep(0.08)
        self.read()
        if self.process.poll() is not None and self.process.returncode != 0:
            raise AssertionError(self.output.decode(errors="replace")[-4000:])

    def paste(self, text):
        self.send(b"\x1b[200~" + text.encode() + b"\x1b[201~")

    def finish(self, discard=False):
        if self.process.poll() is None:
            self.send(CTRL_SHIFT_W)
            if discard:
                self.send(b"d")
        eventually(lambda: self.read() and self.process.poll() is not None)
        assert self.process.returncode == 0, self.output.decode(errors="replace")[-4000:]
        assert self.process.restored, "Terminal mode leaked on exit"
        self.close_fds()

    def close_fds(self):
        os.close(self.master)
        os.close(self.slave)
        self.process.close()


def text(path):
    return path.read_bytes().decode() if path.exists() else None


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-pty-") as directory:
        root = Path(directory)
        source = root / "main.rs"
        source.write_text("fn main() {}\n")
        app = Editor(root, source)
        app.send(CTRL_A)
        expected = "// Unicode: 猫🙂 e\u0301\nfn main() {\n    println!(\"hello\");\n}\n"
        app.paste(expected)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(source) == expected)
        app.send(CTRL_A)
        app.send("temporary")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(source) == expected)
        # Exact chord: Ctrl+K Ctrl+C adds a line comment. Undo restores it.
        app.send(b"\x1b")
        app.send(b"\x0b\x03")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(source) == expected)
        # CSI-u delivers Ctrl+Shift+S distinctly even through a PTY.
        app.send(CTRL_SHIFT_S)
        destination = root / "copy.rs"
        app.send(CTRL_A)
        app.paste(str(destination))
        app.send(b"\r")
        eventually(lambda: app.read() and text(destination) == expected)
        # Resize down and back; the real renderer must survive both.
        for rows, cols in [(3, 10), (40, 120)]:
            fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            os.kill(app.process.pid, signal.SIGWINCH)
            time.sleep(0.1)
            app.read()
        app.finish()
        print("PASS: Unicode paste, save, selection, undo, chord, Save As, resize, terminal restoration")

        other = root / "other.txt"
        other.write_text("original")
        app = Editor(root, other, enhanced=True)
        app.paste("edited ")
        other.write_text("external change")
        app.send(CTRL_S)
        assert text(other) == "external change", "External modifications were overwritten"
        app.finish(discard=True)
        print("PASS: enhanced keyboard negotiation, external-change protection, discard confirmation")

        app = Editor(root, root)
        app.send(CTRL_P)
        app.send("main.rs")
        app.send(b"\r")
        app.send(CTRL_A)
        app.paste("opened via quick open\n")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(source) == "opened via quick open\n")
        app.finish()
        print("PASS: asynchronous file indexing and Ctrl+P quick open")

        journal = root / "recovery"
        recover_file = root / "recover.txt"
        recover_file.write_text("disk baseline")
        app = Editor(root, "--recovery-dir", journal, recover_file, recovery=True)
        app.send(CTRL_A)
        app.paste("RECOVERED unsaved 猫\n")

        def persisted():
            app.read()
            return any("RECOVERED" in p.read_text() for p in journal.glob("*.json"))

        eventually(persisted, timeout=5)
        app.process.kill()
        app.process.wait(timeout=3)
        app.close_fds()
        assert text(recover_file) == "disk baseline"
        app = Editor(root, "--recovery-dir", journal, recover_file, recovery=True)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(recover_file) == "RECOVERED unsaved 猫\n")
        app.finish()
        assert not list(journal.glob("*.json")), "Clean exit left recovery documents"
        print("PASS: SIGKILL recovery and explicit save without modifying disk during recovery")

        multi = root / "multi.txt"
        multi.write_text("cat cat cat")
        app = Editor(root, multi, enhanced=True)
        app.send(b"\x04\x04")  # Ctrl+D selects word, then next occurrence.
        app.send("dog")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(multi) == "dog dog cat")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(multi) == "cat cat cat")
        app.send(b"\x1b[108;6u")  # Ctrl+Shift+L selects all occurrences.
        app.paste("猫")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(multi) == "猫 猫 猫")
        app.send(b"\x1b")
        app.finish()
        print("PASS: multi-cursor shortcuts, Unicode replacement, grouped undo, collapse cursors")

        split_file = root / "split.txt"
        split_file.write_text("abc\nxyz")
        app = Editor(root, split_file, enhanced=True)
        app.send(b"\x1b[C")  # First pane cursor after a.
        app.send(b"\x1b[92;5u")  # Ctrl+backslash splits.
        app.send(b"\x1b[B")  # Second pane cursor after x.
        app.send(b"\x1b[49;5u")  # Ctrl+1
        app.send("!")
        app.send(b"\x1b[50;5u")  # Ctrl+2
        app.send("?")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(split_file) == "a!bc\nx?yz")
        app.send(b"\x17")  # Ctrl+W closes this view, retaining shared buffer.
        app.finish()
        print("PASS: split editors share text, preserve independent cursors, save and close views")

        searched = root / "search.txt"
        searched.write_text("unique_search_token\n")
        app = Editor(root, root, enhanced=True)
        app.send(b"\x1b[102;6u")  # Ctrl+Shift+F
        app.send("unique_search_token")
        app.send(b"\r")
        eventually(lambda: app.read() and "search.txt" in app.screen.text())
        app.send(b"\r")
        app.send("replaced")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(searched) == "replaced\n")
        app.finish()
        print("PASS: workspace text search, result navigation, selection and save")

        fixture = Path(__file__).resolve().parent / "fixtures" / "lsp_server.py"
        language_file = root / "language.rs"
        language_file.write_text("ans\n")
        app = Editor(root, "--lsp", "python3", "--lsp-arg", fixture, language_file, enhanced=True)
        eventually(lambda: app.read() and "Language server ready" in app.screen.text())
        app.send(b"\x1b[F")  # End
        app.send(b"\x1b[32;5u")  # Ctrl+Space
        eventually(lambda: app.read() and "Completion" in app.screen.text())
        app.send(b"\r")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(language_file) == "answer\n")
        app.send(b"\x1b[105;6u")  # Linux Ctrl+Shift+I formats.
        eventually(lambda: app.read() and "formatted" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(language_file) == "formatted\n")
        app.finish()
        print("PASS: native LSP startup, Ctrl+Space completion, formatting, save")

        terminal_file = root / "terminal-output.txt"
        app = Editor(root, root, enhanced=True, extra_env={
            "SHELL": "/bin/sh", "PS1": "VSCLI_SHELL_READY> ", "ENV": "",
        })
        app.send(b"\x1b[96;5u")  # Ctrl+` opens terminal.
        # A panel being visible does not mean the shell has finished initializing
        # its line discipline. Wait for its prompt before sending input.
        eventually(lambda: app.read() and "VSCLI_SHELL_READY>" in app.screen.text())
        app.send("printf terminal_roundtrip > terminal-output.txt\r")
        try:
            eventually(lambda: app.read() and text(terminal_file) == "terminal_roundtrip")
        except AssertionError:
            raise AssertionError(f"Shell roundtrip failed:\n{app.screen.text()}") from None
        app.send("exit\r")
        time.sleep(0.2)
        app.send(b"\x1b[96;5u")  # Hide terminal and restore editor focus.
        app.finish()
        print("PASS: embedded PTY shell, command execution, exit, editor focus restoration")

        task_dir = root / ".vscode"
        task_dir.mkdir(exist_ok=True)
        task_file = root / "task-result.txt"
        (task_dir / "tasks.json").write_text(json.dumps({"version": "2.0.0", "tasks": [{"label": "fixture build", "type": "process", "command": "python3", "args": ["-c", "from pathlib import Path; Path('task-result.txt').write_text('task complete')"], "group": {"kind": "build", "isDefault": True}, "problemMatcher": []}]}))
        app = Editor(root, root, enhanced=True)
        app.send(b"\x1b[98;6u")  # Ctrl+Shift+B
        eventually(lambda: app.read() and "Run Workspace Task?" in app.screen.text())
        assert not task_file.exists(), "Workspace command ran before trust confirmation"
        app.send(b"\r")
        eventually(lambda: app.read() and text(task_file) == "task complete")
        app.send(b"\x1b[96;5u")
        app.finish()
        print("PASS: tasks.json process task, build shortcut, workspace trust, PTY output")

        git_root = root / "git-fixture"
        git_root.mkdir()
        subprocess.run(["git", "init", "-q", str(git_root)], check=True)
        tracked = git_root / "tracked.txt"
        tracked.write_text("kept on disk\n")
        app = Editor(git_root, git_root, enhanced=True)
        app.send(b"\x1b[103;6u")  # Ctrl+Shift+G
        eventually(lambda: app.read() and "?? tracked.txt" in app.screen.text())
        app.send("s")
        eventually(lambda: app.read() and "A  tracked.txt" in app.screen.text() and "working…" not in app.screen.text())
        staged = subprocess.check_output(["git", "-C", str(git_root), "diff", "--cached", "--name-only"])
        assert staged.strip() == b"tracked.txt"
        app.send("u")
        eventually(lambda: app.read() and "?? tracked.txt" in app.screen.text())
        assert text(tracked) == "kept on disk\n"
        app.send(b"\x1b")
        app.finish()
        print("PASS: Git source control, stage/unstage, saved file preservation")

        files_root = root / "files-fixture"
        files_root.mkdir()
        trash_data = root / "isolated-trash-data"
        trash_data.mkdir()
        created = files_root / "source.txt"
        renamed = files_root / "renamed.txt"
        restored = files_root / "restored.txt"
        app = Editor(files_root, files_root, enhanced=True, extra_env={"XDG_DATA_HOME": str(trash_data)})
        app.send(b"\x1b[112;6u")  # Command palette.
        app.send("Explorer: New File")
        app.send(b"\r")
        app.send(str(created))
        app.send(b"\r")
        eventually(lambda: app.read() and created.exists() and "Opened source.txt" in app.screen.text())
        app.send("contents")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(created) == "contents")
        app.send(b"\x1b[101;6u")  # Ctrl+Shift+E
        app.send(b"\x1bOQ")  # F2 rename.
        app.send(str(renamed))
        app.send(b"\r")
        eventually(lambda: app.read() and renamed.exists() and not created.exists() and "Renamed to" in app.screen.text())
        app.send(b"\x1b[49;5u")
        app.send("!")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(renamed) == "contents!")
        if sys.platform.startswith("linux"):
            app.send(b"\x1b[101;6u")
            app.send(b"\x1b[3~")
            app.send(b"\r")
            eventually(lambda: app.read() and not renamed.exists())
            assert any(p.read_text() == "contents!" for p in (trash_data / "Trash" / "files").iterdir())
            app.send(b"\x1b[49;5u")
            app.send(CTRL_SHIFT_S)
            app.send(str(restored))
            app.send(b"\r")
            eventually(lambda: app.read() and text(restored) == "contents!")
        app.finish()
        print("PASS: explorer create/rename, retained buffer edits, isolated Linux trash and Save As")

        watched_file = root / "watched.txt"
        watched_file.write_text("first version")
        app = Editor(root, watched_file, enhanced=True)
        watched_file.write_text("external version")
        eventually(lambda: app.read() and "external version" in app.screen.text())
        app.send(CTRL_Z)
        eventually(lambda: app.read() and "first version" in app.screen.text())
        watched_file.write_text("second external version")
        eventually(lambda: app.read() and "unsaved edits retained" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and "File changed on disk" in app.screen.text())
        assert watched_file.read_text() == "second external version"
        app.send(CTRL_SHIFT_W)
        app.send("d")
        app.finish()
        print("PASS: external reload, undo, dirty conflict and save protection")

        debug_file = root / "debug-fixture.py"
        debug_file.write_text("one\ntwo\nthree\n")
        debug_adapter = Path(__file__).resolve().parent / "fixtures" / "debug_adapter.py"
        app = Editor(root, debug_file, "--debug-adapter", "python3", "--debug-arg", debug_adapter, enhanced=True)
        app.send(b"\x1b[20~")  # F9 breakpoint.
        app.send(b"\x1b[15~")  # F5 launch.
        eventually(lambda: app.read() and "Debugger stopped" in app.screen.text())
        app.send(b"\x1b[100;6u")  # Ctrl+Shift+D.
        eventually(lambda: app.read() and "Stack" in app.screen.text() and "main" in app.screen.text())
        app.send(b"\t\t")
        eventually(lambda: app.read() and "value10" in app.screen.text())
        app.send(b"\x1b")
        app.send(b"\x1b[21~")  # F10 step over.
        eventually(lambda: app.read() and "Ln 3" in app.screen.text())
        app.finish()
        assert debug_file.with_suffix(".py.disconnected").read_text() == "clean shutdown"
        print("PASS: DAP breakpoint/start/step, stack/variables UI, adapter shutdown")

        settings_root = root / "settings-fixture"
        settings_root.mkdir()
        (settings_root / ".vscode").mkdir()
        settings_file = settings_root / ".vscode/settings.json"
        settings_file.write_text('{"editor.tabSize": 4, "[python]": {"editor.tabSize": 3}}')
        user_settings = settings_root / "user-settings.json"
        user_settings.write_text('{"editor.tabSize": 2}')
        configured = settings_root / "configured.py"
        app = Editor(settings_root, configured, "--settings", user_settings, enhanced=True)
        app.send(b"\ta")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(configured) == "   a")
        settings_file.write_text('{"editor.insertSpaces": false, "editor.tabSize": 8}')
        eventually(lambda: app.read() and "Settings reloaded" in app.screen.text())
        app.send(b"\r\tb")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(configured) == "   a\n   \tb")
        settings_file.write_text("invalid JSON")
        eventually(lambda: app.read() and "previous settings retained" in app.screen.text())
        app.send(b"\tc")
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(configured) == "   a\n   \tb\tc")
        app.finish()
        print("PASS: settings scope precedence, indentation, live reload and invalid-config retention")

        bindings = root / "keybindings.json"
        bindings.write_text(json.dumps([{"key": "ctrl+k ctrl+b", "command": "type", "args": {"text": "custom"}, "when": "editorTextFocus"}]))
        custom_file = root / "custom.txt"
        app = Editor(root, "--keybindings", bindings, custom_file)
        app.send(b"\x0b\x02")
        eventually(lambda: app.read() and "custom" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(custom_file) == "custom")
        app.finish()
        print("PASS: user keybinding import with chord, context, and arguments")


if __name__ == "__main__":
    run()
