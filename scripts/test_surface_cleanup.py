"""Surface-workflow cleanup drains output without extending its exit deadline."""
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch

TESTS = Path(__file__).resolve().parents[1] / 'tests'
sys.path.insert(0, str(TESTS))
try:
    spec = importlib.util.spec_from_file_location('surface_workflows', TESTS / 'extension_surfaces_pty.py')
    surfaces = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(surfaces)
finally:
    sys.path.remove(str(TESTS))


class SurfaceCleanupTests(unittest.TestCase):
    def test_exit_waits_for_output_drain_before_reaping_supervisor(self):
        app = Mock()
        app.process.pid = 42
        app.process.poll.side_effect = lambda: 0 if app.read.call_count else None
        app.read.return_value = True
        def reaped(*, timeout):
            self.assertGreater(app.read.call_count, 0)
            self.assertGreater(timeout, 0)
            self.assertLessEqual(timeout, 3)
        app.process.wait.side_effect = reaped
        with patch.object(surfaces.os, 'kill') as terminate:
            surfaces.stop_surface_process(app)
            terminate.assert_called_once_with(42, surfaces.signal.SIGTERM)
        app.process.kill.assert_not_called()
        app.process.wait.assert_called_once()

    def test_expired_drain_retains_bounded_kill_fallback_and_propagates_failure(self):
        app = Mock()
        app.process.pid = 42
        app.process.poll.return_value = None
        app.read.return_value = True
        def expires(predicate, *, timeout):
            self.assertGreater(timeout, 0)
            self.assertLessEqual(timeout, 3)
            self.assertFalse(predicate())
            raise AssertionError('Exit deadline')
        app.process.wait.side_effect = RuntimeError('supervisor still unavailable')
        with patch.object(surfaces.os, 'kill'), patch.object(surfaces, 'eventually', side_effect=expires):
            with self.assertRaisesRegex(RuntimeError, 'supervisor still unavailable'):
                surfaces.stop_surface_process(app)
        app.process.kill.assert_called_once()
        app.process.wait.assert_called_once_with(timeout=3)

    def test_already_exited_editor_is_not_signalled_again(self):
        app = Mock()
        app.process.poll.return_value = 0
        with patch.object(surfaces.os, 'kill') as terminate:
            surfaces.stop_surface_process(app)
            terminate.assert_not_called()
        app.read.assert_not_called()


if __name__ == '__main__':
    unittest.main()
