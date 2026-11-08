#!/usr/bin/env python3
import argparse
import json
import os
from pathlib import Path
import stat
from install import MANIFEST, atomic, digest, manifest, verify


def generate(root, version, arch):
    files = {}
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError("Package symlinks are not supported")
        if path.is_dir() or path == root / MANIFEST:
            continue
        mode = 0o755 if path.stat().st_mode & stat.S_IXUSR else 0o644
        path.chmod(mode)
        files[path.relative_to(root).as_posix()] = {"sha256": digest(path), "mode": mode}
    data = {"schema": 1, "name": "xenadmin-rs", "version": version, "arch": arch, "files": files}
    atomic(root / MANIFEST, json.dumps(data, sort_keys=True, indent=2).encode())
    verify(root, manifest(root))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("version")
    parser.add_argument("--arch", default=os.uname().machine)
    args = parser.parse_args()
    generate(args.directory, args.version, args.arch)
