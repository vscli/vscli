#!/usr/bin/env python3
"""Package the verified native binary, documentation, and dependency notices."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parent.parent

def write_zip(package, archive):
    # Cargo crate archives can contain epoch-dated notices; ZIP starts at 1980.
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, strict_timestamps=False) as output:
        for path in sorted(package.rglob("*")):
            if path.is_file():
                output.write(path, path.relative_to(package.parent))

def main():
    os.chdir(ROOT)
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    ref = os.environ.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/") and ref != f"refs/tags/v{version}":
        sys.exit(f"Release tag must match Cargo.toml: v{version}")
    rust = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(line.split(": ", 1)[1] for line in rust.splitlines() if line.startswith("host:"))
    executable = "vscli.exe" if sys.platform == "win32" else "vscli"
    binary = ROOT / "target" / "release" / executable
    reported = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if reported != f"vscli {version}":
        sys.exit(f"Binary version mismatch: {reported}")
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version=1"], text=True))
    name = f"vscli-{version}-{host}"
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        package = Path(temporary) / name
        package.mkdir()
        shutil.copy2(binary, package / executable)
        for file in ("README.md", "LICENSE-MIT", "LICENSE-APACHE", "Cargo.lock"):
            shutil.copy2(ROOT / file, package / file)
        shutil.copytree(ROOT / "docs", package / "docs")
        inventory = []
        for dependency in metadata["packages"]:
            if dependency["id"] in metadata["workspace_members"]:
                continue
            source = Path(dependency["manifest_path"]).parent
            candidates = [p for p in source.iterdir() if p.is_file() and p.name.lower().startswith(("license", "licence", "copying", "notice", "copyright"))]
            if dependency.get("license_file"):
                candidates.append(source / dependency["license_file"])
            copied = []
            for candidate in sorted(set(candidates)):
                if candidate.is_file():
                    destination = package / "licenses" / f'{dependency["name"]}-{dependency["version"]}' / candidate.name
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(candidate, destination)
                    copied.append(str(destination.relative_to(package)))
            inventory.append({"name": dependency["name"], "version": dependency["version"], "license": dependency["license"], "repository": dependency["repository"], "bundled_notices": copied})
        (package / "DEPENDENCIES.json").write_text(json.dumps(inventory, indent=2) + "\n")
        (package / "BUILD.json").write_text(json.dumps({"version": version, "target": host, "rustc": rust, "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()}, indent=2) + "\n")
        if sys.platform == "win32":
            archive = dist / f"{name}.zip"
            write_zip(package, archive)
        else:
            archive = dist / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.add(package, arcname=name)
        digest = hashlib.file_digest(archive.open("rb"), "sha256").hexdigest()
        archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n")
        print(archive)

if __name__ == "__main__":
    main()
