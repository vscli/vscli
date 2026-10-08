#!/usr/bin/env python3
"""Publish only a matching version tag after validating all downloaded archives."""
import hashlib
import os
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent

def verified_archives(directory):
    archives = sorted([*directory.glob("*.tar.gz"), *directory.glob("*.zip")])
    if len(archives) != 3:
        raise SystemExit("Expected exactly three platform archives")
    for archive in archives:
        expected = archive.with_name(archive.name + ".sha256").read_text().split()[0]
        with archive.open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
        if actual != expected:
            raise SystemExit(f"Checksum mismatch: {archive}")
    return archives

def main():
    os.chdir(ROOT)
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    tag = os.environ["GITHUB_REF_NAME"]
    if tag != f"v{version}":
        raise SystemExit("Refusing release: tag and package version differ")
    archives = verified_archives(Path("dist"))
    existing = subprocess.run(["gh", "release", "view", tag], capture_output=True)
    if existing.returncode == 0:
        raise SystemExit("Release already exists; refusing to replace published artifacts")
    subprocess.run(["gh", "release", "create", tag, "--verify-tag", "--draft", "--title", f"VSCLI {version}", "--notes-file", "RELEASE_NOTES.md"], check=True)
    assets = [str(path) for path in archives]
    assets += [str(path.with_name(path.name + ".sha256")) for path in archives]
    subprocess.run(["gh", "release", "upload", tag, *assets], check=True)
    command = ["gh", "release", "edit", tag, "--draft=false"]
    if version.startswith("0.") or "-" in version:
        command += ["--prerelease"]
    subprocess.run(command, check=True)

if __name__ == "__main__":
    main()
