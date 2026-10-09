"""PTY polling must return to the outer workflow deadline under continuous output."""
import importlib.util
import socket
import threading
import time
from pathlib import Path
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location(
    "pty_workflows", Path(__file__).resolve().parents[1] / "tests/pty_smoke.py"
)
pty_workflows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pty_workflows)


class ReaderBudgetTests(unittest.TestCase):
    def test_continuous_output_yields_before_consuming_an_unbounded_stream(self):
        editor = pty_workflows.Editor.__new__(pty_workflows.Editor)
        editor.master = 123
        editor.output = bytearray()
        editor.pending_input = bytearray()
        editor.screen = Mock()
        editor.enhanced = False
        reads = 0

        def flood(_fd, size):
            nonlocal reads
            reads += 1
            if reads > 300:
                self.fail("PTY polling did not yield under continuous output")
            return b"x" * min(size, 4096)

        with patch.object(pty_workflows.select, "select", return_value=([123], [], [])), \
                patch.object(pty_workflows.os, "read", side_effect=flood):
            self.assertTrue(editor.read())
        self.assertGreater(reads, 0)
        self.assertLessEqual(len(editor.output), 1024 * 1024)
        self.assertEqual(editor.screen.feed.call_count, reads)


class DuplexInputTests(unittest.TestCase):
    def editor(self, descriptor=123):
        editor = pty_workflows.Editor.__new__(pty_workflows.Editor)
        editor.master = descriptor
        editor.output = bytearray()
        editor.pending_input = bytearray()
        editor.screen = Mock()
        editor.process = Mock()
        editor.process.poll.return_value = None
        editor.enhanced = True
        return editor

    def test_short_writes_and_backpressure_preserve_input_and_reply_order(self):
        editor = self.editor()
        written = bytearray()
        attempts = 0
        reads = iter([b"\x1b[6n\x1b[?u"])

        def write(_fd, data):
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                written.extend(data[:2])
                return 2
            if attempts == 2:
                raise BlockingIOError()
            length = min(3, len(data))
            written.extend(data[:length])
            return length

        def read(_fd, _size):
            return next(reads, b"")

        with patch.object(pty_workflows.os, "write", side_effect=write), \
                patch.object(pty_workflows.os, "read", side_effect=read), \
                patch.object(pty_workflows.select, "select", return_value=([123], [123], [])):
            editor.send("猫x")
        self.assertEqual(written, "猫x".encode() + b"\x1b[1;1R\x1b[?0u\x1b[?1;2c")
        self.assertFalse(editor.pending_input)
        self.assertGreater(attempts, 3)

    def test_stalled_writer_fails_at_deadline_instead_of_hanging(self):
        editor = self.editor()
        with patch.object(pty_workflows.os, "write", side_effect=BlockingIOError()), \
                patch.object(pty_workflows.select, "select", return_value=([], [], [])):
            started = time.monotonic()
            with self.assertRaisesRegex(AssertionError, "PTY input timed out.*4 bytes pending"):
                editor.send(b"test", timeout=0.02)
            self.assertLess(time.monotonic() - started, 1)
        self.assertEqual(editor.pending_input, b"test")

    def test_real_duplex_backpressure_drains_output_before_peer_accepts_input(self):
        master, peer = socket.socketpair()
        for connection in [master, peer]:
            connection.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 1024)
        master.setblocking(False)
        peer.settimeout(3)
        editor = self.editor(master.fileno())
        payload = "猫x".encode() * 32_768 + b"\x13"
        output = b"rendered output\n" * 16_384
        received = bytearray()
        errors = []

        def renderer():
            try:
                # Deliberately fill output before reading any editor input.
                # A blocking harness write deadlocks on the opposite queue.
                peer.sendall(output)
                while len(received) < len(payload):
                    received.extend(peer.recv(min(65536, len(payload) - len(received))))
            except Exception as error:
                errors.append(error)

        worker = threading.Thread(target=renderer, daemon=True)
        worker.start()
        try:
            editor.send(payload, timeout=3)
            worker.join(timeout=3)
            self.assertFalse(worker.is_alive())
            self.assertEqual(errors, [])
            self.assertEqual(received, payload)
            self.assertEqual(editor.output, output)
            self.assertFalse(editor.pending_input)
        finally:
            master.close()
            peer.close()
            worker.join(timeout=3)


if __name__ == "__main__":
    unittest.main()
