#!/usr/bin/env python3
"""Warm-cache Unix PTY benchmark. Reports observations, never speed rankings.

Build with cargo build --release --locked, then run this script. Optional
--nvim/--vim arguments compare installed binaries with configuration disabled.
The screen oracle is shared with the actual-executable smoke tests.
"""
import argparse
from datetime import datetime, timezone
import errno
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import pty
import random
import re
import select
import shutil
import signal
import statistics
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from pty_smoke import Screen  # noqa: E402

MARKER = "BENCH_READY_ABCDEFGHIJKLMNOPQRSTUVWXYZ"


class TerminalReplies:
    """Respond to startup queries so missing terminal replies don't become latency.

    Reference: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
    This fixture declares no optional private modes or enhanced keyboard support.
    """
    pattern = re.compile(
        rb"\x1b\[(?:[56]n|0?c|>[0-9]*c|\?[0-9]+\$p)"
        rb"|\x1b\]1[01];\?(?:\x07|\x1b\\)|\x1bP\$qm\x1b\\"
    )

    def __init__(self):
        self.pending = b""

    def feed(self, chunk, row, col):
        self.pending += chunk
        responses = []
        consumed = 0
        for match in self.pattern.finditer(self.pending):
            query = match[0]
            consumed = match.end()
            if query == b"\x1b[6n":
                reply = f"\x1b[{row + 1};{col + 1}R".encode()
            elif query == b"\x1b[5n":
                reply = b"\x1b[0n"
            elif query.startswith(b"\x1b[?"):
                reply = query[:-2] + b";0$y"
            elif query.startswith(b"\x1b[>"):
                reply = b"\x1b[>0;0;0c"
            elif query.startswith(b"\x1b]"):
                color = b"ffff/ffff/ffff" if query[3:4] == b"0" else b"0000/0000/0000"
                reply = query[:5] + b"rgb:" + color + b"\x1b\\"
            elif query.startswith(b"\x1bP"):
                reply = b"\x1bP0$r\x1b\\"
            else:
                reply = b"\x1b[?1;2c"
            responses.append(reply)
        self.pending = self.pending[consumed:][-64:]
        return b"".join(responses)


def distribution(values):
    ordered = sorted(values)
    return {
        "n": len(values), "min": ordered[0], "median": statistics.median(ordered),
        "p95": ordered[math.ceil(len(ordered) * .95) - 1],
        "p99": ordered[math.ceil(len(ordered) * .99) - 1], "max": ordered[-1],
    }


def process_stats(pid):
    """Linux process-only RSS/HWM and CPU, not system-wide or child totals."""
    try:
        status = Path(f"/proc/{pid}/status").read_text()
        fields = {line.split(":", 1)[0]: line.split(":", 1)[1].strip()
                  for line in status.splitlines() if ":" in line}
        stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return {
            "rss_kib": int(fields["VmRSS"].split()[0]),
            "peak_rss_kib": int(fields["VmHWM"].split()[0]),
            "cpu_seconds": (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK"),
        }
    except (OSError, KeyError, ValueError):
        return None


def process_tree(pid):
    """Sum the live tree, including Neovim's separate UI/editor processes.

    RSS sums count shared pages in each process. This is not PSS or peak memory.
    """
    pending, seen, processes = [pid], set(), []
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        stats = process_stats(current)
        if stats:
            processes.append({"pid": current, **stats})
        for child_file in Path(f"/proc/{current}/task").glob("*/children"):
            try:
                pending.extend(map(int, child_file.read_text().split()))
            except OSError:
                pass  # Process exited during observation.
    if not processes:
        return None
    return {"rss_kib": sum(p["rss_kib"] for p in processes),
            "cpu_seconds": sum(p["cpu_seconds"] for p in processes), "processes": processes}


def fixture(path, size):
    first = (MARKER + "A" * 100 + "\n").encode()
    row = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 " * 2 + b"\n"
    with path.open("wb") as stream:
        stream.write(first)
        remaining = size - len(first)
        block = row * 4096
        while remaining:
            chunk = block[:remaining]
            stream.write(chunk)
            remaining -= len(chunk)


class Session:
    def __init__(self, argv, cwd, env, timeout):
        self.screen = Screen()
        self.timeout = timeout
        self.bytes_read = 0
        self.replies = TerminalReplies()
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))

        def control_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.started = time.perf_counter_ns()
        try:
            self.process = subprocess.Popen(
                argv, cwd=cwd, env=env, stdin=slave, stdout=slave, stderr=slave,
                preexec_fn=control_terminal,
            )
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)

    def read(self, timeout):
        if select.select([self.master], [], [], timeout)[0]:
            try:
                chunk = os.read(self.master, 65536)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                chunk = b""
            if not chunk:
                raise RuntimeError(f"Editor exited during measurement: {self.process.poll()}")
            self.bytes_read += len(chunk)
            self.screen.feed(chunk)
            response = self.replies.feed(chunk, self.screen.row, self.screen.col)
            if response:
                os.write(self.master, response)

    def until(self, predicate):
        deadline = time.monotonic() + self.timeout
        while not predicate():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Expected screen update was not observed")
            self.read(remaining)
        return time.perf_counter_ns()

    def marker_position(self):
        for (row, col), value in self.screen.cells.items():
            if value == MARKER[0] and all(
                self.screen.cells.get((row, col + i)) == ch for i, ch in enumerate(MARKER)
            ):
                return row, col
        return None

    def idle(self, seconds):
        before = process_tree(self.process.pid)
        started = time.monotonic()
        self.pump_for(seconds)
        elapsed = time.monotonic() - started
        after = process_tree(self.process.pid)
        return ((after["cpu_seconds"] - before["cpu_seconds"]) / elapsed * 100
                if before and after else None)

    def pump_for(self, seconds):
        deadline = time.monotonic() + seconds
        while (remaining := deadline - time.monotonic()) > 0:
            self.read(remaining)

    def close(self):
        # All edits are disposable fixtures. No save/exit time is measured.
        try:
            os.killpg(self.process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.process.wait(timeout=5)
        os.close(self.master)


def command(editor, binary, source, directory, recovery_directory=None):
    if editor in ("vscli", "vscli_comparison"):
        recovery = (["--recovery-dir", str(recovery_directory)] if recovery_directory
                    else ["--no-recovery"])
        return [binary, "--legacy-keys", "--no-mouse", *recovery, "--keymap", "linux",
                "--settings", str(directory / "settings.json"),
                "--keybindings", str(directory / "keybindings.json"), str(source)]
    return [binary, "-u", "NONE", "-i", "NONE", "-n", "--noplugin",
            "-c", "set noswapfile noundofile nowrap", "-c", "startinsert", str(source)]


def run_trial(editor, binary, source, directory, args):
    env = {**os.environ, "TERM": "xterm-256color", "NO_COLOR": "",
           "XDG_CONFIG_HOME": str(directory / "config"),
           "XDG_STATE_HOME": str(directory / "state"),
           "XDG_DATA_HOME": str(directory / "data"), "VIMINIT": "", "EXINIT": ""}
    recovery_directory = Path(tempfile.mkdtemp(prefix="recovery-", dir=directory)) if args.recovery else None
    argv = command(editor, binary, source, directory, recovery_directory)
    session = Session(argv, directory, env, args.timeout)
    try:
        ready = session.until(session.marker_position)
        origin = session.marker_position()
        idle_cpu = session.idle(args.idle_seconds)
        memory_ready = process_tree(session.process.pid)
        samples, output = [], []
        for i in range(args.keys):
            if i and args.key_interval_ms:
                session.pump_for(args.key_interval_ms / 1000)
            row, col = origin[0], origin[1] + i
            character = "x" if i % 2 == 0 else "z"
            if session.screen.cells.get((row, col)) == character:
                raise RuntimeError("Screen oracle must change for every measured input")
            before_bytes = session.bytes_read
            started = time.perf_counter_ns()
            os.write(session.master, character.encode())
            finished = session.until(lambda: session.screen.cells.get((row, col)) == character)
            samples.append((finished - started) / 1_000_000)
            output.append(session.bytes_read - before_bytes)
        memory_after = process_tree(session.process.pid)
        # Validate actual dirty-buffer recovery after timing and RSS sampling.
        # Never include journal parsing/validation in a key latency sample.
        recovery = recovery_evidence(recovery_directory, args.keys) if args.recovery else None
        return {
            "command": argv, "startup_ms": (ready - session.started) / 1_000_000,
            "key_ms": samples, "key_output_bytes": output,
            "idle_cpu_percent_one_core": idle_cpu, "ready_tree": memory_ready,
            "after_typing_tree": memory_after, "recovery": recovery,
        }
    finally:
        session.close()
        if recovery_directory:
            shutil.rmtree(recovery_directory)


def recovery_evidence(directory, keys):
    journals = list(directory.glob("*.json"))
    if len(journals) != 1:
        raise RuntimeError("Expected one completed recovery journal during the typing workload")
    # The live editor replaces the journal atomically, so reading an older
    # committed snapshot is valid. The final keys need not yet be persisted.
    data = journals[0].read_bytes()
    session = json.loads(data)
    documents = session["documents"]
    if session["version"] != 1 or len(documents) != 1:
        raise RuntimeError("Expected recovery of one dirty fixture during the typing workload")
    text = documents[0]["text"]
    inserted = text.find(MARKER)
    if not 0 < inserted <= keys or text[:inserted] != ("xz" * keys)[:inserted]:
        raise RuntimeError("Recovery journal did not contain the expected measured edits")
    return {"journal_bytes": len(data), "persisted_key_prefix": inserted}


def describe(binary):
    output = subprocess.check_output([binary, "--version"], text=True, timeout=10)
    with open(binary, "rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return {"path": binary, "version": output.splitlines()[0],
            "sha256": digest}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vscli", default=str(ROOT / "target/release/vscli"))
    parser.add_argument("--compare-vscli", help="Second VSCLI binary, interleaved with identical flags")
    parser.add_argument("--nvim", help="Installed Neovim binary; omitted means no comparison")
    parser.add_argument("--vim", help="Installed Vim binary; omitted means no comparison")
    parser.add_argument("--sizes", type=int, nargs="+", default=[10240, 1048576, 10485760])
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--keys", type=int, default=40)
    parser.add_argument("--idle-seconds", type=float, default=1)
    parser.add_argument("--recovery", action="store_true", help="Enable isolated VSCLI recovery in every trial")
    parser.add_argument("--key-interval-ms", type=float, default=0, help="Pump terminal output between key samples")
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if (args.trials < 1 or not 1 <= args.keys <= 50 or min(args.sizes) < 1024
            or not math.isfinite(args.idle_seconds) or not math.isfinite(args.timeout)
            or args.idle_seconds <= 0 or args.timeout <= 0
            or not math.isfinite(args.key_interval_ms) or args.key_interval_ms < 0):
        parser.error("Require trials >= 1, 1..50 keys, sizes >= 1024 and positive timeouts")
    if args.recovery and (args.nvim or args.vim or (args.keys - 1) * args.key_interval_ms < 2500):
        parser.error("Recovery requires VSCLI-only comparisons and at least 2500 ms between first and last key")
    binaries = {}
    for name, value in (("vscli", args.vscli), ("vscli_comparison", args.compare_vscli),
                        ("nvim", args.nvim), ("vim", args.vim)):
        if value:
            resolved = shutil.which(value)
            if not resolved:
                parser.error(f"Binary not found: {value}")
            binaries[name] = str(Path(resolved).resolve())
    try:
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    except subprocess.CalledProcessError:
        revision = None
    report = {
        "schema": 1, "started_at": datetime.now(timezone.utc).isoformat(),
        "source_revision": revision, "platform": platform.platform(),
        "cpu": platform.processor(), "logical_cpus": os.cpu_count(),
        "python": platform.python_version(), "terminal_cells": [120, 40],
        "idle_sample_seconds": args.idle_seconds,
        "recovery_enabled": args.recovery, "key_interval_ms": args.key_interval_ms,
        "cpu_model": next((line.split(":", 1)[1].strip() for line in
                           Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), "unknown")
                     if Path("/proc/cpuinfo").exists() else platform.processor(),
        "fixtures": [],
        "binaries": {name: describe(binary) for name, binary in binaries.items()},
        "method": "warm file cache; ASCII plain text; no user config, swap, LSP or extensions; "
                  "VSCLI recovery as recorded in recovery_enabled; "
                  "spawn-to-visible-marker and serial key-to-observed-PTY-cell; includes Python oracle; "
                  "excludes graphical terminal painting and physical input; Linux RSS sums the process tree "
                  "and counts shared pages per process; sampled RSS is not peak memory",
        "trials": [], "summary": [],
    }
    with tempfile.TemporaryDirectory(prefix="vscli-bench-") as temp:
        directory = Path(temp)
        (directory / "settings.json").write_text("{}")
        (directory / "keybindings.json").write_text("[]")
        for size in args.sizes:
            source = directory / "fixture.txt"
            fixture(source, size)
            with source.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            report["fixtures"].append({"bytes": size, "sha256": digest})
            # Newly written fixtures are warm-cache inputs; no cold-cache claim.
            for trial in range(args.trials):
                order = list(binaries)
                random.Random(trial).shuffle(order)
                for editor in order:
                    print(f"{editor}: {size} bytes, trial {trial + 1}/{args.trials}", file=sys.stderr)
                    try:
                        result = run_trial(editor, binaries[editor], source, directory, args)
                    except (TimeoutError, RuntimeError, OSError) as error:
                        result = {"error": str(error)}
                    report["trials"].append({"editor": editor, "bytes": size, "trial": trial, **result})
    for size in args.sizes:
        for editor in binaries:
            trials = [t for t in report["trials"] if t["editor"] == editor and t["bytes"] == size]
            successful = [t for t in trials if "error" not in t]
            summary = {"editor": editor, "bytes": size, "failures": len(trials) - len(successful)}
            if successful:
                summary.update({"startup_ms": distribution([t["startup_ms"] for t in successful]),
                                "key_ms": distribution([v for t in successful for v in t["key_ms"]]),
                                "key_output_bytes": distribution([v for t in successful for v in t["key_output_bytes"]])})
                values = [t["after_typing_tree"]["rss_kib"] for t in successful if t["after_typing_tree"]]
                if values:
                    summary["tree_rss_kib"] = distribution(values)
                values = [t["idle_cpu_percent_one_core"] for t in successful if t["idle_cpu_percent_one_core"] is not None]
                if values:
                    summary["idle_cpu_percent_one_core"] = distribution(values)
            report["summary"].append(summary)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["summary"], indent=2))
    return 1 if any("error" in t for t in report["trials"]) else 0


if __name__ == "__main__":
    sys.exit(main())
