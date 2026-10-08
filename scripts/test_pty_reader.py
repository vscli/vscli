"""PTY polling must return to the outer workflow deadline under continuous output."""
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
