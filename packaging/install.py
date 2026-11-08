#!/usr/bin/env python3
"""Install a verified extracted XenAdmin RS archive into a local prefix."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import tempfile

APP = "xenadmin-rs"
MANIFEST = "package.json"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def regular(path, limit=None):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or (limit is not None and info.st_size > limit):
        raise ValueError(f"Not a bounded regular file: {path}")
    return info


def read_json(path):
    regular(path, 1024 * 1024)
    return json.loads(path.read_bytes())


def relative(value):
    path = PurePosixPath(value)
    if not value or path.is_absolute() or any(p in (".", "..") for p in value.split("/")) or str(path) != value:
        raise ValueError("Invalid package path")
    return path


def manifest(root):
    data = read_json(root / MANIFEST)
    if not isinstance(data, dict) or set(data) != {"schema", "name", "version", "arch", "files"} or data["schema"] != 1 or data["name"] != APP:
        raise ValueError("Unsupported package manifest")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][a-zA-Z0-9.-]+)?", data["version"]):
        raise ValueError("Invalid package version")
    if data["arch"] not in ("x86_64", "aarch64") or not isinstance(data["files"], dict) or not 1 <= len(data["files"]) <= 4096:
        raise ValueError("Invalid package architecture or file count")
    for name, info in data["files"].items():
        relative(name)
        if name == MANIFEST or not isinstance(info, dict) or set(info) != {"sha256", "mode"} or info["mode"] not in (0o644, 0o755) or not re.fullmatch(r"[a-f0-9]{64}", info["sha256"]):
            raise ValueError("Invalid package file descriptor")
    for required in ("bin/xenadmin-rs", "bin/host-probe", "install.py", "share/icons/hicolor/scalable/apps/xenadmin-rs.svg"):
        if required not in data["files"]:
            raise ValueError(f"Missing package member: {required}")
    return data


def verify(root, data):
    actual = set()
    total = 0
    directories = {parent.as_posix() for name in data["files"] for parent in PurePosixPath(name).parents}
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"Unexpected symlink in package: {path}")
        if path.is_dir():
            if path.relative_to(root).as_posix() not in directories:
                raise ValueError(f"Unowned directory in package: {path}")
            continue
        name = path.relative_to(root).as_posix()
        if name == MANIFEST:
            continue
        info = regular(path)
        total += info.st_size
        wanted = data["files"].get(name)
        if total > 1024 ** 3 or wanted is None or digest(path) != wanted["sha256"] or stat.S_IMODE(info.st_mode) != wanted["mode"]:
            raise ValueError(f"Package file changed or unowned: {path}")
        actual.add(name)
    if actual != set(data["files"]):
        raise ValueError("Package files are missing")


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic(path, data, mode=0o644):
    fd, temporary = tempfile.mkstemp(prefix=".xenadmin-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        sync_dir(path.parent)
    finally:
        if os.path.lexists(temporary):
            os.unlink(temporary)


def link(path, target):
    temporary = path.parent / (".xenadmin-link-" + os.urandom(8).hex())
    try:
        temporary.symlink_to(target)
        os.replace(temporary, path)
        sync_dir(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def exec_quote(value):
    if "=" in value or any(ord(c) < 32 for c in value):
        raise ValueError("The install path contains unsupported desktop-entry characters")
    escaped = "".join("\\" + c if c in '\\"`$' else c for c in value).replace("%", "%%")
    return '"' + escaped.replace("\\", "\\\\") + '"'


class Installer:
    def __init__(self, prefix):
        self.prefix = Path(prefix).expanduser().absolute().resolve()
        exec_quote(str(self.prefix))
        self.root = self.prefix / "lib" / APP
        self.state_path = self.root / "installation.json"
        self.current = self.root / "current"
        self.releases = self.root / "releases"
        executable = str(self.current / "bin" / APP)
        icon = str(self.prefix / "share/icons/hicolor/scalable/apps/xenadmin-rs.svg").replace("\\", "\\\\")
        desktop = f"[Desktop Entry]\nType=Application\nName=XenAdmin RS\nComment=Connect directly to XCP-ng hosts and pools\nExec={exec_quote(executable)}\nIcon={icon}\nTerminal=false\nCategories=System;Network;RemoteAccess;\nStartupNotify=true\n"
        self.external = {
            "bin/xenadmin-rs": ("link", executable),
            "bin/xenadmin-rs-probe": ("link", str(self.current / "bin/host-probe")),
            "bin/xenadmin-rs-install": ("link", str(self.current / "install.py")),
            "share/applications/xenadmin-rs.desktop": ("file", desktop),
            "share/icons/hicolor/scalable/apps/xenadmin-rs.svg": ("link", str(self.current / "share/icons/hicolor/scalable/apps/xenadmin-rs.svg")),
        }

    def parents(self, path):
        current = self.prefix
        current.mkdir(parents=True, exist_ok=True)
        for part in path.relative_to(self.prefix).parts:
            current /= part
            if current.is_symlink():
                raise ValueError(f"Refusing a symlinked install directory: {current}")
            current.mkdir(exist_ok=True)

    @contextlib.contextmanager
    def locked(self):
        self.parents(self.root)
        fd = os.open(self.root / ".install.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            yield
        finally:
            os.close(fd)

    def load(self):
        if not os.path.lexists(self.state_path):
            return None
        state = read_json(self.state_path)
        if not isinstance(state, dict) or set(state) != {"schema", "prefix", "releases"} or state["schema"] != 1 or state["prefix"] != str(self.prefix) or not isinstance(state["releases"], dict) or len(state["releases"]) > 100:
            raise ValueError("Invalid install ownership record")
        for key, checksum in state["releases"].items():
            if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][a-zA-Z0-9.-]+)?-[a-f0-9]{16}", key) or not re.fullmatch(r"[a-f0-9]{64}", checksum):
                raise ValueError("Invalid installed release identity")
        return state

    def save(self, state):
        atomic(self.state_path, json.dumps(state, sort_keys=True, indent=2).encode(), 0o600)

    def check_links(self, owned):
        for name, (kind, content) in self.external.items():
            path = self.prefix / name
            self.parents(path.parent)
            if not os.path.lexists(path):
                continue
            if not owned:
                raise ValueError(f"An unowned file already exists: {path}")
            if kind == "link":
                matches = path.is_symlink() and os.readlink(path) == content
            else:
                regular(path, 64 * 1024)
                matches = path.read_text() == content
            if not matches:
                raise ValueError(f"Installed file was changed; preserve or move it before continuing: {path}")

    def active(self, state):
        if not os.path.lexists(self.current):
            return None
        if not self.current.is_symlink():
            raise ValueError("The managed current link was replaced")
        target = os.readlink(self.current)
        if not target.startswith("releases/") or target[9:] not in state["releases"]:
            raise ValueError("The managed current link points outside registered releases")
        return target[9:]

    def check_release(self, key, checksum):
        self.parents(self.releases)
        path = self.releases / key
        if path.is_symlink() or not path.is_dir():
            raise ValueError(f"Installed release identity changed: {key}")
        data = manifest(path)
        if digest(path / MANIFEST) != checksum or key != data["version"] + "-" + checksum[:16]:
            raise ValueError(f"Installed release manifest changed: {key}")
        verify(path, data)
        return path

    def restore_links(self):
        for name, (kind, content) in self.external.items():
            path = self.prefix / name
            if not os.path.lexists(path):
                if kind == "link":
                    link(path, content)
                else:
                    atomic(path, content.encode())

    def install(self, source):
        source = Path(source).resolve()
        data = manifest(source)
        verify(source, data)
        if data["arch"] != os.uname().machine:
            raise ValueError("Package architecture does not match this machine")
        checksum = digest(source / MANIFEST)
        key = data["version"] + "-" + checksum[:16]
        state = self.load()
        self.check_links(state is not None)
        if state is None:
            if os.path.lexists(self.current) or os.path.lexists(self.releases):
                raise ValueError("Unowned release directory or current link exists")
            state = {"schema": 1, "prefix": str(self.prefix), "releases": {}}
            self.save(state)
        self.active(state)
        self.parents(self.releases)
        destination = self.releases / key
        if os.path.lexists(destination):
            self.check_release(key, checksum)
        else:
            staging = Path(tempfile.mkdtemp(prefix=".staging-", dir=self.releases))
            try:
                for name, info in data["files"].items():
                    path = staging / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(source / name, path)
                    path.chmod(info["mode"])
                atomic(staging / MANIFEST, (source / MANIFEST).read_bytes())
                verify(staging, data)
                for path in staging.rglob("*"):
                    if path.is_file():
                        with path.open("rb") as stream:
                            os.fsync(stream.fileno())
                for directory in sorted((p for p in staging.rglob("*") if p.is_dir()), key=lambda p: len(p.parts), reverse=True):
                    sync_dir(directory)
                sync_dir(staging)
                os.rename(staging, destination)
                sync_dir(self.releases)
            finally:
                if staging.exists():
                    shutil.rmtree(staging)
        if key not in state["releases"] and len(state["releases"]) >= 100:
            raise ValueError("Too many retained releases; uninstall reviewed files before adding more")
        state["releases"][key] = checksum
        self.save(state)
        self.restore_links()
        link(self.current, "releases/" + key)
        return key

    def activate(self, key):
        state = self.load()
        if not state or key not in state["releases"]:
            raise ValueError("Select an installed release shown by status")
        self.check_links(True)
        self.active(state)
        self.check_release(key, state["releases"][key])
        self.restore_links()
        link(self.current, "releases/" + key)

    def uninstall(self):
        state = self.load()
        if state is None:
            return
        self.check_links(True)
        self.active(state)
        self.parents(self.releases)
        for key, checksum in state["releases"].items():
            if os.path.lexists(self.releases / key):
                self.check_release(key, checksum)
        for name in self.external:
            (self.prefix / name).unlink(missing_ok=True)
        self.current.unlink(missing_ok=True)
        for key in state["releases"]:
            if (self.releases / key).exists():
                shutil.rmtree(self.releases / key)
        try:
            self.releases.rmdir()
        except OSError:
            pass
        self.state_path.unlink()
        sync_dir(self.root)


def default_prefix():
    invoked = Path(sys.argv[0]).absolute()
    if invoked.name == "xenadmin-rs-install" and invoked.parent.name == "bin" and invoked.is_symlink():
        prefix = invoked.parent.parent
        expected = prefix / "lib" / APP / "current" / "install.py"
        if invoked.resolve() == expected.resolve():
            return prefix
    return Path.home() / ".local"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, default=default_prefix())
    sub = parser.add_subparsers(dest="command", required=True)
    install = sub.add_parser("install")
    install.add_argument("--source", type=Path, default=Path(__file__).resolve().parent)
    sub.add_parser("status")
    activate = sub.add_parser("activate", help="Switch to a verified retained release")
    activate.add_argument("release")
    sub.add_parser("uninstall", help="Remove only unmodified owned application files; keep connection state")
    args = parser.parse_args()
    try:
        installer = Installer(args.prefix)
        with installer.locked():
            if args.command == "install":
                print("Installed", installer.install(args.source), "in", installer.prefix)
            elif args.command == "activate":
                installer.activate(args.release)
                print("Activated", args.release)
            elif args.command == "uninstall":
                installer.uninstall()
                print("Removed owned application files. Connection profiles, credentials and operation history were retained.")
            else:
                state = installer.load()
                print(json.dumps({"prefix": str(installer.prefix), "active": installer.active(state) if state else None, "releases": sorted(state["releases"]) if state else []}, indent=2))
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f"Install operation refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
