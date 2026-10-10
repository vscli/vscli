#!/usr/bin/env python3
"""Stage immutable save-reference inputs and every available raw output for CI.

This never starts an editor or certifies a successful capture. The Rust reader
independently validates frozen sources, the trusted revision and complete data.
Original output remains available for upload even if staging refuses.
"""
import argparse
import os
from pathlib import Path
import stat

INPUT = Path("tests/vscode-reference-save-actions-stable-configuration-candidate")
OUTPUT = Path("target/save-actions-stable-configuration")
SOURCE_FILES = frozenset((
    "save-code-actions.cjs", "save-configuration-ready.cjs",
    "save-code-actions-cases.json", "save-code-actions-suite.cjs",
    "save-code-actions-run.cjs", "save-code-actions-worker.cjs", "supervisor.cjs",
    "package-lock.json", "package.json", "extension.cjs", "source-sha256.json",
    "README.md", "save-code-actions-readiness.test.cjs",
    "save-code-actions-diagnostics.test.cjs", "save-configuration-ready.test.cjs",
))
MIB = 1024 * 1024


def directory(path):
    if not stat.S_ISDIR(path.lstat().st_mode):
        raise ValueError(f"Expected an ordinary directory: {path}")


def copy_file(source, destination, cap):
    if not stat.S_ISREG(source.lstat().st_mode):
        raise ValueError(f"Expected an ordinary file: {source}")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    with os.fdopen(os.open(source, flags), "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > cap:
            raise ValueError(f"Capture file exceeds admission budget: {source}")
        copied = 0
        with destination.open("xb") as target:
            while True:
                chunk = stream.read(min(65536, cap - copied + 1))
                if not chunk:
                    break
                copied += len(chunk)
                if copied > cap:
                    raise ValueError(f"Capture file grew beyond budget: {source}")
                target.write(chunk)
        after = os.fstat(stream.fileno())
        if (copied != before.st_size or after.st_size != before.st_size
                or after.st_mtime_ns != before.st_mtime_ns):
            raise ValueError(f"Capture file changed during staging: {source}")
        return copied


def stage(source, output, destination):
    source, output, destination = map(Path, (source, output, destination))
    directory(source)
    directory(output)
    resolved_destination = destination.resolve()
    for original in (source.resolve(), output.resolve()):
        if (resolved_destination.is_relative_to(original)
                or original.is_relative_to(resolved_destination)):
            raise ValueError("Staging destination overlaps capture inputs")
    names = set()
    for entry in source.iterdir():
        if len(names) == 16:
            raise ValueError("Source inventory exceeds admission budget")
        names.add(entry.name)
    if names - {"node_modules"} != SOURCE_FILES:
        raise ValueError("Source inventory differs from declared capture package")
    if "node_modules" in names:
        directory(source / "node_modules")
    # An existing destination never authorizes reuse or overwriting an old run.
    destination.mkdir(parents=True, exist_ok=False)
    staged_input = destination / INPUT
    staged_output = destination / OUTPUT
    staged_input.mkdir(parents=True)
    staged_output.mkdir(parents=True)
    total = 0
    count = 0
    for name in sorted(SOURCE_FILES):
        cap = 16384 if name == "source-sha256.json" else MIB
        total += copy_file(source / name, staged_input / name, cap)
        count += 1

    def walk(current, staged, depth):
        nonlocal total, count
        if depth > 4:
            raise ValueError("Output directory depth exceeds admission budget")
        entries = 0
        for entry in current.iterdir():
            entries += 1
            count += 1
            if entries > 256 or count > 512:
                raise ValueError("Output inventory exceeds admission budget")
            mode = entry.lstat().st_mode
            target = staged / entry.name
            if stat.S_ISDIR(mode):
                target.mkdir()
                walk(entry, target, depth + 1)
            elif stat.S_ISREG(mode):
                total += copy_file(entry, target, min(64 * MIB, 256 * MIB - total))
                if total > 256 * MIB:
                    raise ValueError("Capture aggregate exceeds admission budget")
            else:
                raise ValueError(f"Nonregular capture output: {entry}")

    # Do not select preferred result names. Failed prefixes and unexpected raw
    # files are retained; a complete-success reader may subsequently reject them.
    walk(output, staged_output, 0)
    return total


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source")
    parser.add_argument("output")
    parser.add_argument("destination")
    args = parser.parse_args()
    total = stage(args.source, args.output, args.destination)
    print(f"Staged {total} byte-exact capture bytes; success is not certified")
