import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import manifest

spec = importlib.util.spec_from_file_location(
    "collect_notices", Path(__file__).resolve().parents[1] / "scripts/collect-notices.py"
)
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)


class NoticeTests(unittest.TestCase):
    def test_registry_package_without_notices_is_owned_and_does_not_inherit_unrelated_license(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry = root / "registry"
            crate = registry / "minimal-1.0"
            crate.mkdir(parents=True)
            (crate / "Cargo.toml").write_text("[package]\nname='minimal'\n")
            (registry / "LICENSE").write_text("Unrelated parent license")
            package = {
                "name": "minimal", "version": "1.0", "manifest_path": str(crate / "Cargo.toml"),
                "license": "MIT OR Apache-2.0", "source": "registry+https://example.invalid/index",
                "repository": "https://example.invalid/minimal", "authors": ["Example author"],
            }
            destination = root / "payload" / "third-party"
            with patch.object(collector.sys, "argv", ["collect-notices.py", str(destination)]), patch.object(
                collector.subprocess, "check_output",
                side_effect=["host: x86_64-unknown-linux-gnu\n", json.dumps({"packages": [package]})],
            ):
                collector.main()
            declaration = json.loads((destination / "minimal-1.0" / "package.json").read_text())
            self.assertEqual(declaration["notices"], [])
            self.assertEqual(declaration["license"], package["license"])
            self.assertEqual(list((destination / "minimal-1.0").iterdir()), [destination / "minimal-1.0" / "package.json"])
            for member in ("bin/xenadmin-rs", "bin/host-probe", "install.py", "share/icons/hicolor/scalable/apps/xenadmin-rs.svg"):
                file = root / "payload" / member
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("synthetic package member")
            manifest.generate(root / "payload", "1.0.0", "x86_64")


if __name__ == "__main__":
    unittest.main()
