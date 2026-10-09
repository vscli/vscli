#!/usr/bin/env python3
"""Observe every painted sentinel cell, including transient frames during typing."""
import json
from pathlib import Path
import tempfile
import sys

from pty_smoke import Editor, CTRL_S, CTRL_Z, eventually


def locate(screen, token):
    for row in range(40):
        line = "".join(screen.cells.get((row, col), " ") for col in range(180))
        start = line.find(token)
        if start >= 0:
            return [(row, start + i) for i in range(len(token))]
    return []


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-syntax-stability-") as directory:
        root = Path(directory)
        source = root / "stable.cpp"
        original = ('// heading\r\n'
                    'constexpr auto value = R"tag(first\r\nrawStableMarker)tag";\r\n'
                    '/* first\r\ncommentStableMarker */\r\n'
                    'int main() { return 42; }\r\n').encode()
        source.write_bytes(original)
        theme = root / "theme.json"
        theme.write_text(json.dumps({"name": "Stable syntax oracle", "tokenColors": [
            {"scope": "string", "settings": {"foreground": "#071329"}},
            {"scope": "comment", "settings": {"foreground": "#0b172f"}},
        ]}))
        app = Editor(root, "--theme", theme, source, enhanced=True)
        finished = False
        try:
            watched = {}

            def ready():
                app.read()
                for token, color in [("rawStableMarker", (7, 19, 41)),
                                     ("commentStableMarker", (11, 23, 47))]:
                    positions = locate(app.screen, token)
                    if not positions or not all(app.screen.colors.get(pos) == color for pos in positions):
                        return False
                    watched.update({pos: (ch, color) for pos, ch in zip(positions, token)})
                return True

            eventually(ready)
            failures = []

            def observe(pos, ch, color):
                expected = watched.get(pos)
                if expected and ch == expected[0] and color != expected[1]:
                    failures.append((pos, ch, color, expected))

            app.screen.paint_observer = observe
            # Individual input frames exercise the old parse/fallback/parse flash.
            # Mutations stay on the first line, leaving sentinel geometry unchanged.
            for _ in range(12):
                app.send("e\u0301🙂")
                app.send(b"\x7f")  # Backspace; Ctrl+H would open Replace All.
            # One grouped typing undo plus independent backspaces require repeated undo.
            app.send(b"\x1b[1;5H")  # Ctrl+Home
            for _ in range(30):
                app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and source.read_bytes() == original)
            app.screen.paint_observer = None
            assert not failures, f"Transient grammar color loss: {failures[:8]}"
            assert ready(), app.screen.text()
            app.finish()
            finished = True
            assert source.read_bytes() == original
            print("PASS: every emitted C++ raw-string/comment sentinel cell retains its color during Unicode typing and undo")
        finally:
            if not finished:
                app.screen.paint_observer = None
                try:
                    app.process.kill()
                    app.process.wait(timeout=3)
                    app.close_fds()
                except Exception as error:
                    print(f"PTY cleanup failed: {error}", file=sys.stderr)



if __name__ == "__main__":
    run()
