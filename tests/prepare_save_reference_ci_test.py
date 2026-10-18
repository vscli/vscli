import pathlib
import subprocess
import tempfile
import unittest
from prepare_save_reference_ci import prepare, CAPTURE, STAGED


class Preparation(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        subprocess.run(['git', 'init', '-q', self.root], check=True)
        (self.root / 'source').write_bytes(b'original source\r\n')
        subprocess.run(['git', 'add', 'source'], cwd=self.root, check=True)
        subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                        '-c', 'commit.gpgsign=false', 'commit', '-qm', 'test: fixture'], cwd=self.root, check=True)
        self.revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=self.root, text=True).strip()

    def test_cached_partial_and_staged_evidence_survive_a_fresh_capture(self):
        for directory in [CAPTURE, STAGED]:
            (self.root / directory).mkdir(parents=True)
            (self.root / directory / 'partial').write_bytes(b'old\x00evidence')
        prepare(self.root, self.revision)
        self.assertEqual((self.root / CAPTURE / 'candidate-revision.txt').read_text(), self.revision + '\n')
        self.assertFalse((self.root / STAGED).exists())
        archived = list((self.root / 'target').glob('*.previous-*'))
        self.assertEqual(len(archived), 2)
        self.assertTrue(all((directory / 'partial').read_bytes() == b'old\x00evidence' for directory in archived))
        self.assertEqual((self.root / 'source').read_bytes(), b'original source\r\n')

    def test_ci_archives_leave_the_cached_build_directory(self):
        for directory in [CAPTURE, STAGED]:
            (self.root / directory).mkdir(parents=True)
            (self.root / directory / 'partial').write_bytes(b'original evidence')
        archives = self.root / 'runner-temp'
        prepare(self.root, self.revision, archives)
        previous = list(archives.glob('*.previous-*'))
        self.assertEqual(len(previous), 2)
        self.assertTrue(all((directory / 'partial').read_bytes() == b'original evidence' for directory in previous))
        self.assertFalse(list((self.root / 'target').glob('*.previous-*')))

    def test_wrong_revision_leaves_cached_evidence_untouched(self):
        (self.root / CAPTURE).mkdir(parents=True)
        (self.root / CAPTURE / 'partial').write_bytes(b'keep')
        with self.assertRaisesRegex(ValueError, 'trusted workflow revision'):
            prepare(self.root, '0' * 40)
        self.assertEqual((self.root / CAPTURE / 'partial').read_bytes(), b'keep')
        self.assertFalse(list((self.root / 'target').glob('*.previous-*')))

    def test_repeated_preparation_archives_instead_of_reusing_a_capture(self):
        prepare(self.root, self.revision)
        (self.root / CAPTURE / 'partial').write_bytes(b'first capture')
        prepare(self.root, self.revision)
        self.assertFalse((self.root / CAPTURE / 'partial').exists())
        previous = list((self.root / 'target').glob('*.previous-*'))
        self.assertEqual(len(previous), 1)
        self.assertEqual((previous[0] / 'partial').read_bytes(), b'first capture')


if __name__ == '__main__':
    unittest.main()
