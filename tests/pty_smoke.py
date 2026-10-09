#!/usr/bin/env python3
"""Exercise the actual executable through a Unix PTY, using only Python stdlib.

Run: cargo build && python3 tests/pty_smoke.py target/debug/vscli
All file edits and recovery snapshots use temporary directories.
"""
import codecs
import faulthandler
import hashlib
import re
import unicodedata
import zipfile
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
                # Character-set designations have a third byte (e.g. ESC ( B).
                # Treating that byte as text shifts the screen oracle's cursor.
                if self.pending[1] in "()*+":
                    if len(self.pending) < 3:
                        return
                    self.pending = self.pending[3:]
                    continue
                # OSC and DCS payloads are terminal control strings, not cells.
                if self.pending[1] in "]P_^":
                    ending = re.search(r"\x07|\x1b\\", self.pending[2:])
                    if not ending:
                        return
                    self.pending = self.pending[2 + ending.end():]
                    continue
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
        os.set_blocking(self.master, False)
        self.pending_input = bytearray()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 110, 0, 0))
        self.original_termios = termios.tcgetattr(self.slave)

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        options = ["--no-mouse", "--keymap", "linux"]
        self.config_directory = None
        if "--config-dir" not in args:
            self.config_directory = tempfile.TemporaryDirectory(prefix="vscli-pty-config-")
            options += ["--config-dir", self.config_directory.name]
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
        # A continuously writing editor or child must not monopolize polling and
        # prevent eventually() from enforcing its deadline. Continue next poll.
        deadline = time.monotonic() + 0.05
        remaining = 1024 * 1024
        while remaining and time.monotonic() < deadline and select.select([self.master], [], [], 0)[0]:
            try:
                chunk = os.read(self.master, min(65536, remaining))
            except OSError:
                break
            if not chunk:
                break
            remaining -= len(chunk)
            self.output.extend(chunk)
            self.screen.feed(chunk)
            if b"\x1b[6n" in chunk:
                self.pending_input.extend(b"\x1b[1;1R")
            # Respond like a terminal supporting Kitty keyboard negotiation.
            if self.enhanced and b"\x1b[?u" in chunk:
                self.pending_input.extend(b"\x1b[?0u\x1b[?1;2c")
        self.flush_input()
        return True

    def flush_input(self):
        if not self.pending_input:
            return
        try:
            written = os.write(self.master, self.pending_input)
        except BlockingIOError:
            return
        except OSError as error:
            raise AssertionError(f"PTY input failed with {len(self.pending_input)} bytes pending: {error}") from error
        if written == 0:
            raise AssertionError("PTY input made no progress")
        del self.pending_input[:written]

    def send(self, data, timeout=4):
        # A blocking write can deadlock when the editor is simultaneously
        # blocked rendering to a full PTY output queue (observed on macOS).
        # Keep exact byte order across short writes and terminal replies while
        # draining output; no extra keystroke is added to wake the editor.
        self.pending_input.extend(data.encode() if isinstance(data, str) else data)
        deadline = time.monotonic() + timeout
        self.flush_input()
        while self.pending_input:
            self.read()
            if not self.pending_input:
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(f"PTY input timed out with {len(self.pending_input)} bytes pending\n{self.screen.text()}")
            select.select([self.master], [self.master], [], min(remaining, 0.01))
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
        if self.config_directory is not None:
            self.config_directory.cleanup()


def text(path):
    return path.read_bytes().decode() if path.exists() else None


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-pty-") as directory:
        root = Path(directory)
        welcome_state = root / "welcome-recovery"
        app = Editor(root, "--recovery-dir", welcome_state, recovery=True, enhanced=True)
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        assert "Untitled" not in app.screen.text()
        assert "Command Palette" in app.screen.text()
        app.send("ignored")
        app.paste("also ignored")
        app.send(CTRL_S)
        app.send(b"\x1b[6~\x1b[A")
        assert "No open editors" in app.screen.text()
        app.send(b"\x10")  # Quick Open works with no active document.
        eventually(lambda: app.read() and "Go to File" in app.screen.text())
        app.send(b"\x1b")
        app.send(b"\x1bOP")
        app.send("Insert Snippet\r")
        eventually(lambda: app.read() and "Open a file or create a new file first" in app.screen.text())
        assert "No open editors" in app.screen.text()
        assert "Untitled" not in app.screen.text()
        app.send(b"\x0e")  # Ctrl+N explicitly creates the first editor.
        eventually(lambda: app.read() and "Untitled" in app.screen.text())
        app.send("welcome edit")
        app.send(b"\x17")  # Ctrl+W still protects an unsaved first document.
        eventually(lambda: app.read() and "Save changes" in app.screen.text())
        app.send(b"\x1b")
        app.send(CTRL_SHIFT_S)
        welcome_file = root / "welcome.txt"
        app.send(CTRL_A)
        app.send(str(welcome_file))
        app.send(b"\r")
        eventually(lambda: app.read() and text(welcome_file) == "welcome edit")
        app.send(b"\x17")
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        assert "Untitled" not in app.screen.text()
        fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 10, 40, 0, 0))
        os.kill(app.process.pid, signal.SIGWINCH)
        app.read()
        app.finish()
        app = Editor(root, "--recovery-dir", welcome_state, recovery=True)
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        assert "Recovered" not in app.screen.text()
        app.finish()
        print("PASS: empty welcome, explicit new file, dirty close protection, save, last-tab close and empty recovery")

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

        burst_file = root / "burst.txt"
        app = Editor(root, burst_file, enhanced=True)
        burst_text = "xz" * 600
        # Cross the event reader's 1 KiB batch boundary. The save key is part of
        # the same input burst: no later key may be needed to wake buffered bytes.
        app.send(burst_text.encode() + CTRL_S)
        eventually(lambda: app.read() and text(burst_file) == burst_text)
        app.finish()
        print("PASS: raw input beyond 1 KiB reaches Save without another key")

        long_file = root / "long-line.txt"
        long_prefix = "ab" * 4096
        long_original = long_prefix + "🇬🇧e\u0301👩\u200d💻\r\nshort\r\n"
        long_file.write_bytes(long_original.encode())
        app = Editor(root, long_file, enhanced=True)
        app.send(b"\x1b[F")  # End crosses rope leaves and scrolls horizontally.
        app.send(b"\x1b[D\x7f" + CTRL_S)  # Left over ZWJ emoji; delete accented e.
        long_edited = long_prefix + "🇬🇧👩\u200d💻\r\nshort\r\n"
        eventually(lambda: app.read() and long_file.read_bytes() == long_edited.encode())
        app.send(CTRL_Z + CTRL_S)
        eventually(lambda: app.read() and long_file.read_bytes() == long_original.encode())
        app.send(b"\x1b[H")  # Return to the bounded viewport at the line start.
        app.send(b"START" + CTRL_S)
        eventually(lambda: app.read() and long_file.read_bytes() == ("START" + long_original).encode())
        app.finish()
        print("PASS: long-line Unicode movement, deletion, undo, scrolling and CRLF save")

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

        app = Editor(root, "--recovery-dir", journal, recover_file, recovery=True)
        app.send(CTRL_A)
        app.paste("FINAL unsaved before signal 🙂\n")
        # Interrupt after the screen reflects the edit, without waiting for the
        # periodic snapshot. Shutdown must preserve the latest buffer itself.
        eventually(lambda: app.read() and b"FINAL unsaved before signal" in app.output)
        os.kill(app.process.pid, signal.SIGTERM)
        eventually(lambda: app.read() and app.process.poll() is not None)
        assert app.process.returncode != 0
        assert app.process.restored, "Terminal mode leaked on interruption"
        app.close_fds()
        saved = [json.loads(p.read_text()) for p in journal.glob("*.json")]
        assert len(saved) == 1
        assert saved[0]["documents"][0]["text"] == "FINAL unsaved before signal 🙂\n"
        assert text(recover_file) == "RECOVERED unsaved 猫\n"
        app = Editor(root, "--recovery-dir", journal, recover_file, recovery=True)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(recover_file) == "FINAL unsaved before signal 🙂\n")
        app.finish()
        assert not list(journal.glob("*.json"))
        print("PASS: SIGTERM preserves the latest buffer and restores terminal modes")

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
        burst = "xz" * 600
        app.send(f"printf '%s' '{burst}' > terminal-output.txt\r")
        eventually(lambda: app.read() and text(terminal_file) == burst)
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

        tail_file = root / "reload-tail.txt"
        tail_original = "old header\r\nkeep 猫🙂 tail\r\nlast"
        tail_external = "new longer header\r\nkeep 猫🙂 tail\r\nlast"
        tail_file.write_bytes(tail_original.encode())
        app = Editor(root, tail_file, enhanced=True)
        app.send(b"\x1b[B" + b"\x1b[C" * 4)  # After 'keep' on the unchanged line.
        tail_file.write_bytes(tail_external.encode())
        eventually(lambda: app.read() and "new longer header" in app.screen.text())
        app.send(b"!" + CTRL_S)
        eventually(lambda: app.read() and tail_file.read_bytes() == tail_external.replace("keep", "keep!").encode())
        app.send(CTRL_Z + CTRL_Z)
        eventually(lambda: app.read() and "old header" in app.screen.text())
        app.finish(discard=True)
        print("PASS: external reload retains the cursor in unchanged Unicode/CRLF text")

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

        extension = Path(__file__).resolve().parent / "fixtures" / "command-extension"
        extension_file = root / "extension.txt"
        extension_file.write_text("zebra\napple\npear")
        extension_settings = root / ".vscode/settings.json"
        extension_settings.write_text('{"fixture.value":"initial"}')
        app = Editor(root, "--extension", extension, extension_file)
        eventually(lambda: app.read() and "Extension ready" in app.screen.text())
        app.send(b"\x1bOP")
        app.send("fixture.configuration")
        app.send(b"\r")
        eventually(lambda: app.read() and '"activation":"initial","value":"initial","changes":0' in app.screen.text())
        extension_settings.write_text('{"fixture.value":"reloaded"}')
        eventually(lambda: app.read() and "configuration changed=reloaded" in app.screen.text())
        app.send(b"\x1bOP")
        app.send("fixture.configuration")
        app.send(b"\r")
        eventually(lambda: app.read() and '"activation":"initial","value":"reloaded","changes":1' in app.screen.text())
        app.send(CTRL_A)
        app.send(b"\x1bOP")  # F1 command palette.
        app.send("Fixture Sort Lines")
        app.send(b"\r")
        eventually(lambda: app.read() and "sort applied=true" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(extension_file) == "apple\npear\nzebra")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(extension_file) == "zebra\napple\npear")
        app.send(CTRL_A)
        app.send(b"\x1b[20~")  # Original extension-contributed F9 binding.
        eventually(lambda: app.read() and "sort applied=true" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(extension_file) == "apple\npear\nzebra")
        app.send(b"\x1bOP")
        app.send("Extensions: Stop Host")
        app.send(b"\r")
        eventually(lambda: app.read() and "Extension host stopped" in app.screen.text())
        app.finish()
        print("PASS: extension activation/settings reload, palette/F9 commands, native edit/save/undo and host stop")

        package = root / "fixture.vsix"
        with zipfile.ZipFile(package, "w", zipfile.ZIP_DEFLATED) as archive:
            for entry in extension.iterdir():
                if entry.is_file():
                    archive.write(entry, "extension/" + entry.name)
        extensions_dir = root / "installed-extensions"
        installed_file = root / "installed-extension.txt"
        installed_file.write_text("zebra\napple\npear")
        app = Editor(root, "--extensions-dir", extensions_dir, enhanced=True)
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        app.send(b"\x1bOP")
        app.send("Extensions: Install from VSIX")
        app.send(b"\r")
        eventually(lambda: app.read() and "Install Extension from local VSIX" in app.screen.text())
        app.send(str(package))
        app.send(b"\r")
        eventually(lambda: app.read() and "vscli-test.command-fixture@1.0.0" in app.screen.text())
        assert text(installed_file) == "zebra\napple\npear", "Installation executed package editing code"
        app.send(b"\r")
        eventually(lambda: app.read() and "Run installed extension?" in app.screen.text())
        app.send(b"\r")
        eventually(lambda: app.read() and "Extension ready" in app.screen.text())
        assert "No open editors" in app.screen.text(), "Activation fabricated an editor"
        app.send(b"\x0f")  # Ctrl+O opens the first editor after package activation.
        eventually(lambda: app.read() and "Open File" in app.screen.text())
        app.send(CTRL_A)
        app.send(str(installed_file))
        app.send(b"\r")
        eventually(lambda: app.read() and "installed-extension.txt" in app.screen.text())
        app.send(CTRL_A)
        app.send(b"\x1b[20~")  # Installed package's original F9 binding.
        eventually(lambda: app.read() and "sort applied=true" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(installed_file) == "apple\npear\nzebra")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(installed_file) == "zebra\napple\npear")
        app.send(b"\x1b[120;6u")  # Replacing the current host uses the startup worker.
        eventually(lambda: app.read() and "Installed Extensions" in app.screen.text())
        app.send(b"\r")
        eventually(lambda: app.read() and "Run installed extension?" in app.screen.text())
        app.send(b"\r")
        eventually(lambda: app.read() and "Extension ready" in app.screen.text())
        app.send(CTRL_A)
        app.send(b"\x1b[20~")
        eventually(lambda: app.read() and "sort applied=true" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(installed_file) == "apple\npear\nzebra")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(installed_file) == "zebra\napple\npear")
        app.send(b"\x1b[120;6u")  # Original Ctrl+Shift+X extensions shortcut.
        eventually(lambda: app.read() and "Installed Extensions" in app.screen.text())
        app.send(b"\x1b[3~")  # Delete removes registry entry while preserving host files.
        eventually(lambda: app.read() and "No packages installed" in app.screen.text())
        app.send(b"\x1b")
        app.send(b"\x17")  # Ctrl+W closes the last editor while the host remains live.
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        assert "Untitled" not in app.screen.text()
        app.finish()
        print("PASS: empty welcome, VSIX install/activation, open/F9/save/undo, host replacement, uninstall and close-last")


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

        snippet_file = root / "snippet.txt"
        snippet_file.write_bytes(b"  seed\r\n")
        snippet_bindings = root / "snippet-keys.json"
        snippet_bindings.write_text(json.dumps([
            {"key": "f6", "command": "editor.action.insertSnippet", "when": "editorTextFocus",
             "args": {"snippet": "call(${1:name}, ${2:value});\n$1$0"}},
            {"key": "f7", "command": "editor.action.insertSnippet", "when": "editorTextFocus",
             "args": {"snippet": "${1:$TM_FILENAME}-$1$0"}},
        ]))
        app = Editor(root, "--keybindings", snippet_bindings, snippet_file, enhanced=True)
        app.send(b"\x1b[C\x1b[C\x1b[1;2F")  # Select seed after its indentation.
        app.send(b"\x1b[17~")  # F6 inserts the literal template.
        app.send("猫\t\x1b[Zfox\tbar\t")  # Linked typing, next, previous, final stop.
        edited = b"  call(fox, bar);\r\n  fox\r\n"
        defaults = b"  call(name, value);\r\n  name\r\n"
        app.send(CTRL_S)
        eventually(lambda: app.read() and snippet_file.read_bytes() == edited)
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and snippet_file.read_bytes() == defaults)
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and snippet_file.read_bytes() == b"  seed\r\n")
        app.send(b"\x19\x19")  # Redo insertion and the typing transaction.
        app.send(CTRL_S)
        eventually(lambda: app.read() and snippet_file.read_bytes() == edited)
        app.send(CTRL_A)
        app.send(b"\x1b[18~")  # F7 resolves the file variable.
        app.send(b"\x1b")       # Escape keeps the primary selection and unlinks mirrors.
        app.send("only")
        app.send(CTRL_S)
        eventually(lambda: app.read() and snippet_file.read_bytes() == b"only-snippet.txt")
        app.finish()
        print("PASS: native snippet insertion, variables, Tab/Shift+Tab/Escape, linked edits, undo/redo and CRLF save")


        catalog_dir = root / "catalog-user" / "snippets"
        catalog_dir.mkdir(parents=True)
        catalog_settings = catalog_dir.parent / "settings.json"
        catalog_settings.write_text("{}")
        (catalog_dir / "rust.json").write_text(json.dumps({
            "Named Rust": {"body": "${1:Rust}-$1$0", "prefix": ["rs", "rust"]},
        }))
        (root / ".vscode").mkdir(exist_ok=True)
        (root / ".vscode" / "local.code-snippets").write_text(
            '{ // workspace JSONC catalog\n'
            '"Workspace greeting":{"scope":"plaintext", "prefix":"greet",'
            '"description":"catalog description", "body":["${1:hello}", "$1$0"],},}'
        )
        catalog_file = root / "catalog.txt"
        catalog_file.write_bytes(b"seed\r\n")
        catalog_keys = root / "catalog-keys.json"
        catalog_keys.write_text(json.dumps([
            {"key":"f7", "command":"editor.action.insertSnippet", "args":{"name":"Named Rust", "langId":"rust"}},
        ]))
        app = Editor(root, "--settings", catalog_settings, "--keybindings", catalog_keys, catalog_file, enhanced=True)
        app.send(CTRL_A)
        app.send(b"\x1b[101;6u")  # Ctrl+Shift+E: invoke the snippet palette from Explorer.
        app.send(b"\x1bOP")
        app.send("Insert Snippet\r")
        eventually(lambda: app.read() and "Workspace greeting" in app.screen.text())
        app.send("greet")
        app.send(b"\r")
        app.send("world\t")
        app.send(CTRL_S)
        eventually(lambda: app.read() and catalog_file.read_bytes() == b"world\r\nworld")
        app.send(CTRL_Z)
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and catalog_file.read_bytes() == b"seed\r\n")
        app.send(CTRL_A)
        app.send(b"\x1b[18~")
        eventually(lambda: app.read() and "Rust-Rust" in app.screen.text())
        app.send("native\t")
        app.send(CTRL_S)
        eventually(lambda: app.read() and catalog_file.read_bytes() == b"native-native")
        app.finish()
        print("PASS: user/workspace JSONC snippet catalogs, picker, named language lookup and CRLF undo")
        import_source = root / "vscode-user"
        import_source.mkdir()
        source_settings = '{ // retain original\n"editor.tabSize":2, "extension.unknown":true,}'
        (import_source / "settings.json").write_text(source_settings)
        (import_source / "keybindings.json").write_text(json.dumps([
            {"key":"f6", "command":"type", "args":{"text":"imported"}},
        ]))
        imported_config = root / "native-config"
        preview = subprocess.run([BINARY, "--import-vscode", str(import_source), "--config-dir", str(imported_config)], capture_output=True, check=True)
        assert json.loads(preview.stdout)["activated_profile"] is None
        assert not imported_config.exists()
        subprocess.run([BINARY, "--import-vscode", str(import_source), "--config-dir", str(imported_config), "--apply-import"], capture_output=True, check=True)
        migration_file = root / "migration.txt"
        migration_file.write_text("")
        app = Editor(root, "--config-dir", imported_config, migration_file, enhanced=True)
        app.send(b"\x1b[17~\t")  # Imported F6 command followed by imported two-space indentation.
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(migration_file) == "imported  ")
        app.finish()
        assert (import_source / "settings.json").read_text() == source_settings
        print("PASS: read-only VS Code import preview, atomic activation, imported settings/keybindings and source preservation")

        theme_file = root / "custom-theme.json"
        theme_file.write_text(json.dumps({"name":"PTY Custom", "colors":{"editor.background":"#123456", "editor.lineHighlightBackground":"#123456", "editor.foreground":"#abcdef"}, "tokenColors":[{"scope":"keyword", "settings":{"foreground":"#010203"}}]}))
        app = Editor(root, "--config-dir", imported_config, "--theme", theme_file, migration_file, enhanced=True)
        eventually(lambda: app.read() and "Color theme: PTY Custom" in app.screen.text())
        assert b"48;2;18;52;86" in app.output
        app.send(b"\x1bOP")
        app.send("Preferences: Color Theme\r")
        eventually(lambda: app.read() and "VSCLI Light" in app.screen.text())
        app.send("VSCLI Light\r")
        eventually(lambda: app.read() and "Color theme: VSCLI Light" in app.screen.text())
        app.finish()
        app = Editor(root, "--config-dir", imported_config, migration_file, enhanced=True)
        eventually(lambda: app.read() and "Color theme: VSCLI Light" in app.screen.text())
        assert b"48;2;255;255;255" in app.output
        app.send(b"\x1bOP")
        app.send("Preferences: Load Color Theme File\r")
        app.send(str(root / "missing-theme.json") + "\r")
        eventually(lambda: app.read() and "Theme unchanged" in app.screen.text())
        app.finish()
        assert text(migration_file) == "imported  "
        print("PASS: native theme RGB rendering, picker, persistent selection, explicit file override and failed-load retention")

        cpp_source = root / "native-grammar.cpp"
        cpp_bytes = ('template<typename T> class Box {};\r\n'
                     'constexpr auto text = R"tag(first\r\n🌍 raw)tag";\r\n'
                     '/* first\r\n🌍 comment */\r\n'
                     'int main() { return 42; }\r\n').encode('utf-8')
        cpp_source.write_bytes(cpp_bytes)
        cpp_theme = root / "cpp-theme.json"
        cpp_theme.write_text(json.dumps({"name": "C++ grammar PTY", "tokenColors": [
            {"scope": "keyword", "settings": {"foreground": "#030509"}},
            {"scope": "string", "settings": {"foreground": "#071329"}},
            {"scope": "comment", "settings": {"foreground": "#0b172f"}},
            {"scope": "entity.name.type", "settings": {"foreground": "#0d1f35"}}
        ]}))
        app = Editor(root, "--theme", cpp_theme, cpp_source, enhanced=True)
        colors = [b"38;2;3;5;9", b"38;2;7;19;41", b"38;2;11;23;47", b"38;2;13;31;53"]
        eventually(lambda: app.read() and all(color in app.output for color in colors))
        assert "🌍 raw" in app.screen.text() and "🌍 comment" in app.screen.text()
        app.send("X")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and cpp_source.read_bytes() == cpp_bytes)
        app.finish()
        assert cpp_source.read_bytes() == cpp_bytes
        print("PASS: native C++ grammar colors, templates/multiline raw strings/Unicode comments and CRLF edit/undo/save")


        journey_root = root / "integrated-journey"
        journey_root.mkdir()
        journey_config = root / "journey-config"
        journey_store = root / "journey-extensions"
        app = Editor(journey_root, "--config-dir", journey_config, "--extensions-dir", journey_store, enhanced=True)
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        assert "Untitled" not in app.screen.text()
        app.finish()

        journey_user = root / "journey-vscode-user"
        (journey_user / "snippets").mkdir(parents=True)
        (journey_user / "settings.json").write_text('{ // original profile\n"editor.tabSize":2,"workbench.colorTheme":"Journey Theme","extension.unknown":true,}')
        (journey_user / "keybindings.json").write_text(json.dumps([
            {"key":"f6", "command":"editor.action.insertSnippet", "when":"editorTextFocus"},
        ]))
        (journey_user / "snippets" / "plaintext.json").write_text(json.dumps({
            "Imported lines": {"prefix":"journey", "body":["${1:zebra}", "apple", "pear$0"]},
        }))
        journey_source_extensions = root / "journey-vscode-extensions"
        theme_package = journey_source_extensions / "fixture.theme-1.0.0"
        theme_package.mkdir(parents=True)
        (theme_package / "package.json").write_text(json.dumps({"publisher":"fixture", "name":"theme", "version":"1.0.0", "contributes":{"themes":[{"id":"journey-theme", "label":"Journey Theme", "path":"theme.json"}]}}))
        (theme_package / "base.json").write_text('{"colors":{"editor.background":"#16283a","editor.foreground":"#cbdced"}}')
        (theme_package / "theme.json").write_text('{ // original theme\n"include":"base.json","name":"Journey Theme",}')
        source_files = [path for source_root in [journey_user, journey_source_extensions] for path in source_root.rglob("*") if path.is_file()]
        original_hashes = {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in source_files}
        import_args = [BINARY, "--import-vscode", str(journey_user), "--vscode-extensions", str(journey_source_extensions), "--config-dir", str(journey_config)]
        report = json.loads(subprocess.run(import_args, capture_output=True, check=True).stdout)
        assert report["activated_profile"] is None
        assert not (journey_config / "active-profile.json").exists()
        report = json.loads(subprocess.run([*import_args, "--apply-import"], capture_output=True, check=True).stdout)
        active_profile = Path(report["activated_profile"])
        for source_path in journey_user.rglob("*"):
            if source_path.is_file():
                assert (active_profile / source_path.relative_to(journey_user)).read_bytes() == source_path.read_bytes()
        assert (active_profile / "themes" / "imported" / "theme.json").read_bytes() == (theme_package / "theme.json").read_bytes()

        app = Editor(journey_root, "--config-dir", journey_config, "--extensions-dir", journey_store, enhanced=True)
        eventually(lambda: app.read() and "Color theme: Journey Theme" in app.screen.text())
        assert "No open editors" in app.screen.text()
        assert b"48;2;22;40;58" in app.output
        app.send(b"\x0e")  # New File in the empty workbench.
        app.send(b"\x1b[17~")  # Imported F6 opens the imported snippet catalog.
        eventually(lambda: app.read() and "Imported lines" in app.screen.text())
        app.send("Imported lines\r")
        app.send("zulu\t")
        journey_file = journey_root / "edited.txt"
        app.send(CTRL_SHIFT_S)
        app.send(CTRL_A)
        app.send(str(journey_file) + "\r")
        eventually(lambda: app.read() and text(journey_file) == "zulu\napple\npear")
        app.send(b"\x1bOP")
        app.send("Extensions: Install from VSIX\r")
        eventually(lambda: app.read() and "Install Extension from local VSIX" in app.screen.text())
        app.send(str(package) + "\r")  # The known command fixture package created above.
        eventually(lambda: app.read() and "vscli-test.command-fixture@1.0.0" in app.screen.text())
        assert text(journey_file) == "zulu\napple\npear"
        listed = json.loads(subprocess.run([BINARY, "--extensions-dir", str(journey_store), "--list-extensions"], capture_output=True, check=True).stdout)
        assert [entry["id"] for entry in listed] == ["vscli-test.command-fixture"]
        app.send(b"\r")
        eventually(lambda: app.read() and "Run installed extension?" in app.screen.text())
        app.send(b"\r")
        eventually(lambda: app.read() and "Extension ready" in app.screen.text())
        app.send(CTRL_A)
        app.send(b"\x1b[20~")  # Installed extension's F9 binding sorts the imported snippet.
        eventually(lambda: app.read() and "sort applied=true" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(journey_file) == "apple\npear\nzulu")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(journey_file) == "zulu\napple\npear")
        app.send(b"\x17")
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        assert "Untitled" not in app.screen.text()
        app.finish()

        app = Editor(journey_root, "--config-dir", journey_config, "--extensions-dir", journey_store, enhanced=True)
        eventually(lambda: app.read() and "Color theme: Journey Theme" in app.screen.text())
        assert "No open editors" in app.screen.text()
        assert b"48;2;22;40;58" in app.output
        app.send(b"\x0e\x1b[17~")
        eventually(lambda: app.read() and "Imported lines" in app.screen.text())
        app.send(b"\x1b")
        app.send(b"\x17")
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        app.finish()
        assert text(journey_file) == "zulu\napple\npear"
        assert {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in source_files} == original_hashes
        copied_keys = active_profile / "keybindings.json"
        copied_keys.write_text("{malformed copied configuration")
        app = Editor(journey_root, "--config-dir", journey_config, "--extensions-dir", journey_store, enhanced=True)
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        app.send(b"\x1bOP")
        app.send("Settings: Compatibility Report\r")
        eventually(lambda: app.read() and "Imported keybindings unavailable" in app.screen.text())
        assert "No open editors" in app.screen.text()
        app.send(b"\x1b")
        app.send(b"\x0e")
        eventually(lambda: app.read() and "Untitled" in app.screen.text())
        app.send(b"\x17")
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        app.finish()
        assert copied_keys.read_text() == "{malformed copied configuration"
        assert {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in source_files} == original_hashes
        print("PASS: empty welcome → copied profile/theme/snippet/key import → VSIX install/list/activate → edit/save/undo → empty restart, with original source hashes intact")

        native_snippet_store = root / "native-snippet-store"
        native_snippet_package = root / "native-snippets.vsix"
        activation_marker = root / "snippet-code-must-not-run"
        with zipfile.ZipFile(native_snippet_package, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("extension/package.json", json.dumps({
                "publisher":"vscli-test", "name":"native-snippets", "version":"1.0.0", "main":"activate.cjs",
                "contributes":{"snippets":[{"language":"cpp", "path":"snippets/cpp.json"}]},
            }))
            archive.writestr("extension/activate.cjs", "require('node:fs').writeFileSync(" + json.dumps(str(activation_marker)) + ", 'executed');")
            archive.writestr("extension/snippets/cpp.json", json.dumps({
                "Installed C++ class":{"prefix":"fixturecpp", "body":["class ${1:Thing} {", "  $1 value;", "};$0"]},
            }))
        subprocess.run([BINARY, "--extensions-dir", str(native_snippet_store), "--install-extension", str(native_snippet_package)], check=True, capture_output=True)
        native_snippet_file = root / "native-snippet.cpp"
        native_snippet_file.write_bytes(b"seed\r\n")
        native_snippet_settings = root / "native-snippet-settings.json"
        native_snippet_settings.write_text("{}")
        app = Editor(root, "--extensions-dir", native_snippet_store, "--extension-node", root / "node-does-not-exist", "--settings", native_snippet_settings, native_snippet_file, enhanced=True)
        app.send(CTRL_A)
        app.send(b"\x1bOP")
        app.send("Insert Snippet\r")
        eventually(lambda: app.read() and "Installed C++ class" in app.screen.text())
        app.send("fixturecpp\r")
        app.send("猫\t")
        app.send(CTRL_S)
        eventually(lambda: app.read() and native_snippet_file.read_bytes() == "class 猫 {\r\n  猫 value;\r\n};".encode())
        app.send(CTRL_Z)
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and native_snippet_file.read_bytes() == b"seed\r\n")
        subprocess.run([BINARY, "--extensions-dir", str(native_snippet_store), "--uninstall-extension", "vscli-test.native-snippets"], check=True, capture_output=True)
        app.send(b"\x1bOP")
        app.send("Insert Snippet\r")
        eventually(lambda: app.read() and "0 snippets" in app.screen.text())
        assert "Installed C++ class" not in app.screen.text()
        app.send(b"\x1b")
        app.finish()
        assert not activation_marker.exists()
        print("PASS: installed C++ snippets without Node/code activation, CRLF linked edits/save/undo and uninstall refresh")


        navigation_root = root / "navigation"
        navigation_root.mkdir()
        navigation_config = root / "navigation-config"
        navigation_file = navigation_root / "navigation.txt"
        navigation_file.write_text("base\n")
        app = Editor(navigation_root, navigation_file, "--config-dir", navigation_config, enhanced=True)
        app.send("changed ")
        app.send(b"\x17")
        eventually(lambda: app.read() and "Save changes" in app.screen.text())
        app.send(b"\x1b")
        assert text(navigation_file) == "base\n"
        app.send(b"\x17")
        app.send("s")
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        assert text(navigation_file) == "changed base\n"
        eventually(lambda: app.read() and "Recent files" in app.screen.text())
        app.send(b"\x1b[116;6u")  # Ctrl+Shift+T reopens the last closed file.
        eventually(lambda: app.read() and "changed base" in app.screen.text())
        app.send("prefix ")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(navigation_file) == "changed base\n")
        app.send(b"\x17")
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        app.send(b"\x12")  # Ctrl+R opens recent files even with no editor.
        eventually(lambda: app.read() and "Open Recent File" in app.screen.text())
        assert "navigation.txt" in app.screen.text()
        app.send(b"\x1b")
        navigation_file.unlink()
        app.send(b"\x1b[116;6u")
        eventually(lambda: app.read() and "history retained" in app.screen.text())
        assert "No open editors" in app.screen.text()
        assert not navigation_file.exists()
        navigation_file.write_text("external\n")
        app.send(b"\x1b[116;6u")
        eventually(lambda: app.read() and "external" in app.screen.text() and "No open editors" not in app.screen.text())
        app.send("new edit")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and text(navigation_file) == "external\n")
        app.send(b"\x17")
        eventually(lambda: app.read() and "No open editors" in app.screen.text())
        app.finish()
        recent_state = json.loads((navigation_config / "state/recent-files.json").read_text())
        assert recent_state["schema"] == 1
        assert recent_state["files"][0]["path"] == str(navigation_file.resolve())
        app = Editor(navigation_root, "--config-dir", navigation_config, enhanced=True)
        eventually(lambda: app.read() and "Recent files" in app.screen.text() and "navigation.txt" in app.screen.text())
        assert "No open editors" in app.screen.text()
        app.send(b"\x1b[116;6u")
        eventually(lambda: app.read() and "No closed file-backed editors" in app.screen.text())
        assert "No open editors" in app.screen.text()
        app.send(b"\x12")
        app.send("navigation.txt\r")
        eventually(lambda: app.read() and "external" in app.screen.text() and "No open editors" not in app.screen.text())
        app.send(b"\x17")
        app.finish()
        assert text(navigation_file) == "external\n"
        print("PASS: recent-file picker, dirty close cancel/save, reopen/undo, missing-file retry, welcome recents and persisted restart")


        action_root = root / "code-actions"
        action_root.mkdir()
        action_file = action_root / "main.rs"
        action_file.write_bytes(b"bad\r\n")
        action_server = Path(__file__).resolve().parent / "fixtures" / "code_action_server.py"
        app = Editor(action_root, action_file, "--lsp", sys.executable, "--lsp-arg", action_server, "--lsp-language", "rust", enhanced=True)
        eventually(lambda: app.read() and "Language server ready" in app.screen.text())
        app.send(b"\x1b[1;2C" * 3)  # Select bad with Shift+Right.
        app.send(b"\x1b[46;5u")  # Ctrl+. Quick Fix with enhanced keyboard protocol.
        eventually(lambda: app.read() and "Fix selected text" in app.screen.text())
        assert action_file.read_bytes() == b"bad\r\n"
        app.send(b"\r")
        eventually(lambda: app.read() and "Applied code action" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and action_file.read_bytes() == b"fixed\r\n")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and action_file.read_bytes() == b"bad\r\n")
        app.send(CTRL_A)
        app.send(b"\x1b[114;6u")  # Ctrl+Shift+R, same refactor key across profiles.
        eventually(lambda: app.read() and "Resolve refactor" in app.screen.text())
        app.send(b"\r")
        eventually(lambda: app.read() and "Applied code action" in app.screen.text())
        app.send(CTRL_S)
        eventually(lambda: app.read() and action_file.read_bytes() == b"resolved")
        app.send(CTRL_Z)
        app.send(CTRL_S)
        eventually(lambda: app.read() and action_file.read_bytes() == b"bad\r\n")
        app.finish()
        print("PASS: native LSP Quick Fix/Refactor shortcuts, action picker, lazy resolve, CRLF save and undo")



if __name__ == "__main__":
    faulthandler.enable()
    faulthandler.dump_traceback_later(120, repeat=True)
    try:
        run()
    finally:
        faulthandler.cancel_dump_traceback_later()
