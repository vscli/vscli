"""The syntax oracle must detect a white frame even if colors recover in one read."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "pty_color_workflows", Path(__file__).resolve().parents[1] / "tests/pty_smoke.py"
)
workflows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflows)


class PaintColorTests(unittest.TestCase):
    def test_transient_white_repaint_is_observed_before_same_read_recovery(self):
        screen = workflows.Screen()
        paints = []
        screen.paint_observer = lambda *args: paints.append(args)
        screen.feed(b"\x1b[1;1H\x1b[38;2;7;19;41mA")
        screen.feed(b"\x1b[1;1H\x1b[38;2;255;255;255mA"
                    b"\x1b[1;1H\x1b[38;2;7;19;41mA")
        self.assertEqual([paint[2] for paint in paints],
                         [(7, 19, 41), (255, 255, 255), (7, 19, 41)])
        self.assertEqual(screen.colors[(0, 0)], (7, 19, 41))

    def test_background_sequences_partial_input_reset_and_clear(self):
        screen = workflows.Screen()
        screen.feed(b"\x1b[38;2;7;")
        screen.feed(b"19;41m\x1b[48;2;99;88;77mA\x1b[0mB")
        self.assertEqual(screen.colors[(0, 0)], (7, 19, 41))
        self.assertIsNone(screen.colors[(0, 1)])
        screen.feed(b"\x1b[2J")
        self.assertEqual(screen.colors, {})


if __name__ == "__main__":
    unittest.main()
