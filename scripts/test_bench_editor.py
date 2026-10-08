"""Check benchmark oracles; timing values themselves are not CI assertions."""
import sys
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

if sys.platform != "win32":
    from bench_editor import MARKER, Screen, TerminalReplies, distribution, fixture, process_tree, recovery_evidence


@unittest.skipIf(sys.platform == "win32", "The PTY benchmark is Unix-only")
class BenchmarkTests(unittest.TestCase):
    def test_fragmented_control_strings_do_not_move_cursor(self):
        stream = b"\x1b(B\x1b]11;rgb:0000/0000/0000\x07\x1bP$qm\x1b\\\x1b[2;4Htext"
        whole = Screen()
        whole.feed(stream)
        fragmented = Screen()
        for byte in stream:
            fragmented.feed(bytes([byte]))
        self.assertEqual(whole.cells, fragmented.cells)
        self.assertEqual({(1, 3 + i): ch for i, ch in enumerate("text")}, whole.cells)

    def test_queries_answered_once_across_every_split_boundary(self):
        queries = b"\x1b[6n\x1b[c\x1b[?2026$p\x1b]11;?\x07\x1b[5n\x1bP$qm\x1b\\"
        expected = (b"\x1b[2;4R\x1b[?1;2c\x1b[?2026;0$y"
                    b"\x1b]11;rgb:0000/0000/0000\x1b\\\x1b[0n\x1bP0$r\x1b\\")
        for boundary in range(len(queries) + 1):
            replies = TerminalReplies()
            actual = replies.feed(queries[:boundary], 1, 3) + replies.feed(queries[boundary:], 1, 3)
            self.assertEqual(expected, actual)
            self.assertEqual(b"", replies.feed(b"ordinary text", 1, 3))

    def test_exact_fixture_size_and_ready_marker(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.txt"
            fixture(path, 10241)
            self.assertEqual(10241, path.stat().st_size)
            self.assertTrue(path.read_text().startswith(MARKER))

    def test_percentiles_include_the_tail(self):
        result = distribution(list(range(100, 0, -1)))
        self.assertEqual(50.5, result["median"])
        self.assertEqual(95, result["p95"])
        self.assertEqual(99, result["p99"])
        self.assertEqual(100, result["max"])

    def test_single_line_fixture_has_exact_size_and_no_line_breaks(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.txt"
            fixture(path, 1048579, single_line=True)
            content = path.read_bytes()
            self.assertEqual(len(content), 1048579)
            self.assertTrue(content.startswith(MARKER.encode()))
            self.assertNotIn(b"\n", content)
            self.assertNotIn(b"\r", content)

    def test_recovery_evidence_requires_measured_edits_in_a_committed_journal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(RuntimeError):
                recovery_evidence(root, 10)
            journal = root / "session.json"
            for value in ("", "wrong" + MARKER, "xz" * 6 + MARKER):
                journal.write_text(json.dumps({"version": 1, "documents": [{"text": value}]}))
                with self.assertRaises(RuntimeError):
                    recovery_evidence(root, 10)
            journal.write_text(json.dumps({"version": 1, "documents": [{"text": "xzx" + MARKER}]}))
            self.assertEqual(3, recovery_evidence(root, 10)["persisted_key_prefix"])
            self.assertEqual(journal.stat().st_size, recovery_evidence(root, 10)["journal_bytes"])

    @unittest.skipUnless(sys.platform == "linux", "Process-tree accounting uses /proc")
    def test_memory_counts_a_live_child_process(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(10)"])
        try:
            tree = process_tree(os.getpid())
            pids = {p["pid"] for p in tree["processes"]}
            self.assertTrue({os.getpid(), child.pid}.issubset(pids))
            self.assertEqual(sum(p["rss_kib"] for p in tree["processes"]), tree["rss_kib"])
        finally:
            child.kill()
            child.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
