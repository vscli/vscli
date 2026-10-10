#!/usr/bin/env python3
"""Integrity checks using genuine captures and explicitly synthetic failures."""
from pathlib import Path
import tempfile
import unittest

from stage_save_actions_reference import INPUT, OUTPUT, copy_file, stage

ARCHIVE = Path(__file__).parent / "vscode-reference/observations/1.95.0/save-configuration-stable/1338970"


def files(root):
    return {p.relative_to(root): p.read_bytes() for p in root.rglob("*") if p.is_file()}


class CaptureStaging(unittest.TestCase):
    def test_all_three_genuine_captures_remain_byte_exact(self):
        for platform in ("linux", "darwin", "win32"):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory:
                original = ARCHIVE / platform
                destination = Path(directory) / "capture"
                stage(original / INPUT, original / OUTPUT, destination)
                self.assertEqual(files(original), files(destination))
                self.assertEqual(len(files(destination)), 90)

    def test_failed_and_unexpected_output_is_retained_without_certifying_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "partial"
            (output / "result").mkdir(parents=True)
            prefix = b'{"phase":"target","originalError":"Synthetic failure"}\r\n'
            (output / "result/failed-case-failure.json").write_bytes(prefix)
            (output / "unexpected-debug.bin").write_bytes(b"\0raw\xff")
            destination = root / "capture"
            stage(ARCHIVE / "linux" / INPUT, output, destination)
            self.assertEqual(files(output), files(destination / OUTPUT))
            self.assertFalse((destination / OUTPUT / "result/save-code-actions.json").exists())
            self.assertEqual((output / "result/failed-case-failure.json").read_bytes(), prefix)

    def test_existing_capture_cannot_be_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sentinel = root / "unsaved"
            sentinel.write_bytes(b"retained")
            original = ARCHIVE / "linux"
            with self.assertRaises(FileExistsError):
                stage(original / INPUT, original / OUTPUT, root)
            self.assertEqual(sentinel.read_bytes(), b"retained")

    def test_staging_cannot_create_outputs_inside_the_original_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "partial"
            output.mkdir()
            original = b"original failure prefix\r\n"
            (output / "capture.log").write_bytes(original)
            with self.assertRaisesRegex(ValueError, "overlaps"):
                stage(ARCHIVE / "linux" / INPUT, output, output / "staged")
            self.assertEqual(files(output), {Path("capture.log"): original})

    def test_oversized_or_nonregular_source_refuses_without_rewriting_original(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "oversized"
            source.write_bytes(b"12345")
            with self.assertRaises(ValueError):
                copy_file(source, root / "copy", 4)
            self.assertFalse((root / "copy").exists())
            self.assertEqual(source.read_bytes(), b"12345")
            with self.assertRaises(ValueError):
                copy_file(root, root / "copy", 4)

    @unittest.skipUnless(hasattr(__import__("os"), "O_NOFOLLOW"), "POSIX symlink admission")
    def test_symlink_output_cannot_copy_an_outside_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            secret = root / "outside"
            secret.write_bytes(b"outside fixture")
            output = root / "partial"
            output.mkdir()
            (output / "escape").symlink_to(secret)
            destination = root / "capture"
            with self.assertRaises(ValueError):
                stage(ARCHIVE / "linux" / INPUT, output, destination)
            self.assertFalse((destination / OUTPUT / "escape").exists())
            self.assertEqual(secret.read_bytes(), b"outside fixture")


if __name__ == "__main__":
    unittest.main()
