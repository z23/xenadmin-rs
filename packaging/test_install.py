import contextlib
import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from install import Installer, exec_quote
from manifest import generate


class Installation(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.installer = Installer(self.root / 'local space % quoted"$')

    def package(self, version):
        root = self.root / version
        root.mkdir()
        for name in ("bin/xenadmin-rs", "bin/host-probe", "install.py", "share/icons/hicolor/scalable/apps/xenadmin-rs.svg", "README.md", "share/package.json"):
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(version + name)
            path.chmod(0o755 if name.startswith("bin/") or name == "install.py" else 0o644)
        generate(root, version, os.uname().machine)
        return root

    @contextlib.contextmanager
    def locked(self):
        with self.installer.locked():
            yield self.installer

    def test_install_upgrade_activate_uninstall_and_reinstall_keep_user_state(self):
        a, b = self.package("1.0.0"), self.package("1.1.0")
        state = self.installer.prefix / "state/xenadmin-rs/operations.sqlite3"
        state.parent.mkdir(parents=True)
        state.write_text("private operator history")
        with self.locked() as i:
            first = i.install(a)
            self.assertEqual(first, i.install(a))
            second = i.install(b)
            self.assertEqual(i.active(i.load()), second)
            self.assertTrue((i.releases / first).exists())
            i.activate(first)
            self.assertEqual(i.active(i.load()), first)
            i.uninstall()
            self.assertFalse(os.path.lexists(i.prefix / "bin/xenadmin-rs"))
            self.assertEqual(state.read_text(), "private operator history")
            i.uninstall()
            self.assertEqual(i.install(b), second)

    def test_installed_management_launcher_keeps_its_custom_prefix(self):
        source = self.package("1.0.0")
        (source / "install.py").write_bytes((Path(__file__).parent / "install.py").read_bytes())
        generate(source, "1.0.0", os.uname().machine)
        with self.locked() as installer:
            key = installer.install(source)
        launcher = str(self.installer.prefix / "bin/xenadmin-rs-install")
        result = subprocess.run([launcher, "status"], check=True, capture_output=True, text=True)
        status = json.loads(result.stdout)
        self.assertEqual(status["prefix"], str(self.installer.prefix))
        self.assertEqual(status["active"], key)
        subprocess.run([launcher, "uninstall"], check=True, capture_output=True, text=True)
        self.assertFalse(os.path.lexists(launcher))

    def test_modified_installed_file_blocks_uninstall_before_removing_anything(self):
        source = self.package("1.0.0")
        with self.locked() as i:
            key = i.install(source)
            (i.releases / key / "README.md").write_text("operator changes")
            with self.assertRaises(ValueError):
                i.uninstall()
            self.assertTrue((i.prefix / "bin/xenadmin-rs").exists())
            self.assertTrue(i.state_path.exists())

    def test_unowned_launcher_and_symlinked_directory_are_preserved(self):
        source = self.package("1.0.0")
        with self.locked() as i:
            path = i.prefix / "bin/xenadmin-rs"
            path.parent.mkdir()
            path.write_text("another program")
            with self.assertRaises(ValueError):
                i.install(source)
            self.assertEqual(path.read_text(), "another program")
            path.unlink()
            path.parent.rmdir()
            elsewhere = self.root / "elsewhere"
            elsewhere.mkdir()
            path.parent.symlink_to(elsewhere, target_is_directory=True)
            with self.assertRaises(ValueError):
                i.install(source)
            self.assertEqual(list(elsewhere.iterdir()), [])

    def test_package_hash_path_symlink_and_extra_directory_are_checked(self):
        source = self.package("1.0.0")
        binary = source / "bin/xenadmin-rs"
        binary.write_text("modified binary")
        with self.locked() as i:
            with self.assertRaises(ValueError):
                i.install(source)
            generate(source, "1.0.0", os.uname().machine)
            binary.unlink()
            binary.symlink_to(source / "README.md")
            with self.assertRaises(ValueError):
                i.install(source)
            binary.unlink()
            binary.write_text("fixed")
            generate(source, "1.0.0", os.uname().machine)
            (source / "operator-directory").mkdir()
            with self.assertRaises(ValueError):
                i.install(source)
            (source / "operator-directory").rmdir()
            manifest = source / "package.json"
            data = json.loads(manifest.read_text())
            data["files"]["../escaped"] = data["files"].pop("README.md")
            manifest.write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                i.install(source)

    def test_interrupted_initial_registration_and_missing_launchers_are_repairable(self):
        source = self.package("1.0.0")
        with self.locked() as i:
            i.save({"schema": 1, "prefix": str(i.prefix), "releases": {}})
            key = i.install(source)
            (i.prefix / "bin/xenadmin-rs").unlink()
            i.current.unlink()
            self.assertEqual(i.install(source), key)
            self.assertTrue((i.prefix / "bin/xenadmin-rs").exists())

    def test_external_metadata_changes_and_foreign_current_link_are_refused(self):
        source = self.package("1.0.0")
        with self.locked() as i:
            i.install(source)
            path = i.prefix / "share/applications/xenadmin-rs.desktop"
            original = path.read_text()
            path.write_text("custom launcher")
            with self.assertRaises(ValueError):
                i.install(source)
            path.write_text(original)
            i.current.unlink()
            i.current.symlink_to(self.root)
            with self.assertRaises(ValueError):
                i.uninstall()

    def test_desktop_escaping_and_unsupported_paths(self):
        self.assertEqual(exec_quote('/a b/%x/"`$\\'), '"/a b/%%x/\\\\"\\\\`\\\\$\\\\\\\\"')
        for path in ("/a\nb", "/a=b", "/a\tb"):
            with self.assertRaises(ValueError):
                exec_quote(path)

    def test_replaced_release_directory_is_never_followed(self):
        source = self.package("1.0.0")
        with self.locked() as i:
            key = i.install(source)
            outside = self.root / "outside"
            i.releases.rename(outside)
            i.releases.symlink_to(outside, target_is_directory=True)
            for action in (lambda: i.install(source), lambda: i.activate(key), i.uninstall):
                with self.assertRaises(ValueError):
                    action()
            self.assertTrue((outside / key / "README.md").exists())
            self.assertTrue(i.state_path.exists())

    def test_activate_repairs_missing_launchers_and_rejects_bad_registry(self):
        source = self.package("1.0.0")
        with self.locked() as i:
            key = i.install(source)
            launcher = i.prefix / "bin/xenadmin-rs"
            launcher.unlink()
            i.activate(key)
            self.assertTrue(launcher.exists())
            i.state_path.write_text("[]")
            with self.assertRaises(ValueError):
                i.load()


if __name__ == "__main__":
    unittest.main()
