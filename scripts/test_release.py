import hashlib
from pathlib import Path
import tempfile
import unittest

from check_commits import valid
from publish_release import verified_archives

class CommitPolicy(unittest.TestCase):
    def test_accepts_one_line_conventional_messages(self):
        for message in ("fix: retain unsaved edits\n", "feat(editor): add snippets", "refactor!: change document API"):
            self.assertTrue(valid(message), message)

    def test_rejects_bodies_merges_and_oversized_subjects(self):
        for message in ("fix: change\n\nBody", "Merge pull request #1", "updated files", "feat: " + "x" * 101, ""):
            self.assertFalse(valid(message), message)

class ReleaseIntegrity(unittest.TestCase):
    def test_requires_all_platforms_and_rejects_tampering(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with self.assertRaises(SystemExit):
                verified_archives(directory)
            for name in ("linux.tar.gz", "macos.tar.gz", "windows.zip"):
                archive = directory / name
                archive.write_bytes(name.encode())
                digest = hashlib.sha256(name.encode()).hexdigest()
                archive.with_name(name + ".sha256").write_text(f"{digest}  {name}\n")
            self.assertEqual(len(verified_archives(directory)), 3)
            (directory / "windows.zip").write_bytes(b"tampered")
            with self.assertRaisesRegex(SystemExit, "Checksum mismatch"):
                verified_archives(directory)

if __name__ == "__main__":
    unittest.main()
