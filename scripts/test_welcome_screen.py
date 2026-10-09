"""The welcome PTY oracle must compare a complete, possibly wrapped settings path."""
import importlib.util
from pathlib import Path
import sys
import unittest

TESTS = Path(__file__).resolve().parents[1] / 'tests'
sys.path.insert(0, str(TESTS))
try:
    spec = importlib.util.spec_from_file_location('welcome_workflows', TESTS / 'welcome_brand_pty.py')
    welcome = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(welcome)
finally:
    sys.path.remove(str(TESTS))


class WelcomeScreenTests(unittest.TestCase):
    def screen(self, path, width):
        screen = welcome.GraphicsScreen()
        def row(line, column, value):
            for offset, char in enumerate(value):
                screen.cells[line, column + offset] = char
        row(5, 0, 'EXPLORER │')
        row(5, 20, 'User settings · JSON')
        row(6, 0, 'sidebar │')
        row(7, 0, 'sidebar │')
        row(6, 20, path[:width].ljust(width))
        row(7, 20, path[width:].ljust(width))
        return screen

    def test_exact_path_survives_wrapping_and_internal_spaces(self):
        path = '/var/folders/platform prefix/project/config/settings.json'
        for width in [28, 40, 88]:
            screen = self.screen(path, width)
            self.assertEqual(welcome.displayed_settings_path(screen), path)
            self.assertNotEqual(welcome.displayed_settings_path(screen), path.replace(' ', ''))

    def test_missing_or_incomplete_path_never_satisfies_exact_oracle(self):
        path = '/long/path/config/settings.json'
        screen = self.screen(path, 20)
        del screen.cells[7, 20]
        self.assertNotEqual(welcome.displayed_settings_path(screen), path)
        self.assertIsNone(welcome.displayed_settings_path(welcome.GraphicsScreen()))


if __name__ == '__main__':
    unittest.main()
