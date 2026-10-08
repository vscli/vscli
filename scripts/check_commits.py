#!/usr/bin/env python3
"""Validate one-line Conventional Commit messages in a revision range."""
import re
import subprocess
import sys

PATTERN = re.compile(r"(?:feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(?:\([a-z0-9_-]+\))?!?: [^\s].*")

def valid(message):
    lines = message.rstrip("\n").splitlines()
    return len(lines) == 1 and len(lines[0]) <= 100 and PATTERN.fullmatch(lines[0]) is not None

def main():
    revision = sys.argv[1] if len(sys.argv) > 1 else "HEAD"
    hashes = subprocess.check_output(["git", "rev-list", revision], text=True).splitlines()
    failures = []
    for commit in hashes:
        message = subprocess.check_output(["git", "show", "-s", "--format=%B", commit], text=True)
        if not valid(message):
            failures.append(f"{commit[:12]}: expected one Conventional Commit line, at most 100 characters")
    if failures:
        sys.exit("\n".join(failures))
    print(f"Validated {len(hashes)} commit messages")

if __name__ == "__main__":
    main()
