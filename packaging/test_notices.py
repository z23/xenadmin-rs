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
            # It ships no licence text: the standard text of each licence it declares is
            # kept once beside the packages and named in its declaration.
            self.assertEqual(declaration["standard_texts"], ["licenses/MIT.txt", "licenses/Apache-2.0.txt"])
            for name in declaration["standard_texts"]:
                self.assertEqual(
                    (destination / name).read_bytes(),
                    (collector.STANDARD_TEXTS / Path(name).name).read_bytes(),
                )
            self.assertIn("Apache License", (destination / "licenses/Apache-2.0.txt").read_text())
            self.assertIn("Permission is hereby granted", (destination / "licenses/MIT.txt").read_text())
            self.assertEqual(list((destination / "minimal-1.0").iterdir()), [destination / "minimal-1.0" / "package.json"])
            for member in ("bin/xenadmin-rs", "bin/host-probe", "install.py", "share/icons/hicolor/scalable/apps/xenadmin-rs.svg"):
                file = root / "payload" / member
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("synthetic package member")
            manifest.generate(root / "payload", "1.0.0", "x86_64")

    def collect(self, root, packages):
        destination = root / "payload" / "third-party"
        with patch.object(collector.sys, "argv", ["collect-notices.py", str(destination)]), patch.object(
            collector.subprocess, "check_output",
            side_effect=["host: x86_64-unknown-linux-gnu\n", json.dumps({"packages": packages})],
        ):
            collector.main()
        return destination

    def package(self, root, name, license, files=()):
        crate = root / "registry" / f"{name}-1.0"
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text(f"[package]\nname='{name}'\n")
        for file in files:
            (crate / file).write_text(f"{file} of {name}")
        return {
            "name": name, "version": "1.0", "manifest_path": str(crate / "Cargo.toml"),
            "license": license, "source": "registry+https://example.invalid/index",
            "repository": None, "authors": [],
        }

    def test_a_package_with_its_own_notices_keeps_them_and_takes_no_standard_text(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            # As gpui-kit-assets does: the crate's licence is declared, the notice of the
            # icons it embeds is a file of its own.
            icons = self.package(root, "icons", "Apache-2.0", ["LICENSE-LUCIDE", "lucide.json"])
            both = self.package(root, "both", "MPL-2.0", ["LICENSE", "NOTICE"])
            destination = self.collect(root, [icons, both])
            declaration = json.loads((destination / "icons-1.0" / "package.json").read_text())
            self.assertEqual(declaration["notices"], ["00-LICENSE-LUCIDE"])
            self.assertEqual((destination / "icons-1.0/00-LICENSE-LUCIDE").read_text(), "LICENSE-LUCIDE of icons")
            # That notice is not the text of the licence the crate declares: the standard one is added.
            self.assertEqual(declaration["standard_texts"], ["licenses/Apache-2.0.txt"])
            # A licence with no standard text here is no obstacle to a package that ships its own.
            declaration = json.loads((destination / "both-1.0" / "package.json").read_text())
            self.assertEqual(declaration["notices"], ["00-LICENSE", "01-NOTICE"])
            self.assertEqual(declaration["standard_texts"], [])
            self.assertEqual(sorted(path.name for path in (destination / "licenses").iterdir()), ["Apache-2.0.txt"])

    def test_a_supplied_text_of_one_declared_licence_is_enough(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dual = self.package(root, "dual", "MIT OR Apache-2.0", ["LICENSE-MIT"])
            (root / "registry/dual-1.0/LICENSE-MIT").write_text(
                "Copyright (c) someone\n\nPermission is hereby granted, free\nof charge, to any person")
            destination = self.collect(root, [dual])
            declaration = json.loads((destination / "dual-1.0" / "package.json").read_text())
            self.assertEqual(declaration["notices"], ["00-LICENSE-MIT"])
            self.assertEqual(declaration["standard_texts"], [])
            self.assertFalse((destination / "licenses").exists())

    def test_a_package_without_any_licence_text_stops_the_collector_and_is_named(self):
        for license in ("MPL-2.0", "MIT OR MPL-2.0", None):
            with self.subTest(license=license), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                packages = [self.package(root, "fine", "MIT"), self.package(root, "bare", license)]
                with self.assertRaises(SystemExit) as stopped:
                    self.collect(root, packages)
                self.assertIn("bare 1.0", str(stopped.exception))
                self.assertNotIn("fine 1.0", str(stopped.exception))
                self.assertFalse((root / "payload/third-party/dependencies.json").exists())

    def test_an_expression_names_its_licences_and_not_its_exceptions(self):
        self.assertEqual(collector.license_ids("MIT OR Apache-2.0"), ["MIT", "Apache-2.0"])
        self.assertEqual(collector.license_ids("MIT/Apache-2.0"), ["MIT", "Apache-2.0"])
        self.assertEqual(collector.license_ids("MIT / Apache-2.0"), ["MIT", "Apache-2.0"])
        self.assertEqual(
            collector.license_ids("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT"), ["Apache-2.0", "MIT"])
        self.assertEqual(collector.license_ids("(MIT OR Apache-2.0) AND Unicode-3.0"), ["MIT", "Apache-2.0", "Unicode-3.0"])
        self.assertEqual(collector.license_ids(None), [])

    def test_every_standard_text_is_named_by_its_identifier(self):
        names = sorted(path.name for path in collector.STANDARD_TEXTS.glob("*.txt"))
        self.assertEqual(names, ["Apache-2.0.txt", "MIT.txt", "Zlib.txt"])
        # And each is recognised by the words the collector looks for in a supplied notice.
        self.assertEqual(sorted(f"{name}.txt" for name in collector.SIGNATURES), names)
        for name, words in collector.SIGNATURES.items():
            text = " ".join((collector.STANDARD_TEXTS / f"{name}.txt").read_text().split()).lower()
            self.assertIn(words, text)


if __name__ == "__main__":
    unittest.main()
