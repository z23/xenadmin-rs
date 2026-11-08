import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

import manage


class LabTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)
        self.iso = self.base / "fixture.iso"
        self.iso.write_bytes(b"disposable test installer")
        self.plan = self.base / "state"
        self.args = argparse.Namespace(directory=str(self.plan), iso=str(self.iso),
            sha256=hashlib.sha256(self.iso.read_bytes()).hexdigest(), hosts=2,
            subnet="192.168.254.0/24", memory_mib=8192, vcpus=4, disk_gib=64)

    def test_plan_is_offline_isolated_and_does_not_overwrite(self):
        with patch.object(manage, "run") as run:
            manage.write_plan(self.args)
            run.assert_not_called()
        directory, manifest = manage.load(self.plan)
        network = ET.fromstring((directory / "network.xml").read_text())
        self.assertIsNone(network.find("forward"))
        self.assertEqual(len(network.findall("ip/dhcp/host")), 2)
        for host in manifest["hosts"]:
            domain = ET.fromstring(manage.domain_xml(manifest, host, directory))
            self.assertEqual(domain.find("devices/interface/source").attrib["network"], manifest["name"])
            self.assertEqual(domain.find("devices/graphics").attrib["listen"], "127.0.0.1")
            self.assertIsNone(domain.find("devices/disk[@device='cdrom']"))
            self.assertEqual(domain.find("cpu").attrib["mode"], "host-passthrough")
        with self.assertRaisesRegex(ValueError, "never overwritten"):
            manage.write_plan(self.args)

    def test_invalid_hash_and_source_tree_paths_are_rejected(self):
        self.args.sha256 = "0" * 64
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            manage.write_plan(self.args)
        self.assertFalse(self.plan.exists())
        with self.assertRaisesRegex(ValueError, "outside"):
            manage.root_path(manage.REPOSITORY / "accidental-lab")

    def test_existing_name_with_wrong_uuid_is_never_adopted(self):
        manage.write_plan(self.args)
        _, manifest = manage.load(self.plan)
        xml = ET.fromstring(manage.network_xml(manifest))
        xml.find("uuid").text = "00000000-0000-0000-0000-000000000000"
        reply = subprocess.CompletedProcess([], 0, ET.tostring(xml, encoding="unicode"), "")
        with patch.object(manage, "virsh", return_value=reply) as virsh:
            with self.assertRaisesRegex(ValueError, "ownership differs"):
                manage.check_owner(manifest)
            self.assertEqual(virsh.call_count, 1)

    def test_owned_uuid_requires_matching_metadata_too(self):
        manage.write_plan(self.args)
        directory, manifest = manage.load(self.plan)
        host = manifest["hosts"][0]
        xml = ET.fromstring(manage.domain_xml(manifest, host, directory))
        xml.remove(xml.find("metadata"))
        reply = subprocess.CompletedProcess([], 0, ET.tostring(xml, encoding="unicode"), "")
        with patch.object(manage, "virsh", return_value=reply):
            with self.assertRaisesRegex(ValueError, "ownership differs"):
                manage.check_owner(manifest, host)

    def test_mutated_names_cannot_target_unrelated_domains(self):
        manage.write_plan(self.args)
        path = self.plan / "lab.json"
        manifest = json.loads(path.read_text())
        manifest["hosts"][0]["name"] = "unrelated-vm"
        path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "host name"):
            manage.load(self.plan)

    def test_existing_workstation_route_blocks_conflicting_lab_network(self):
        manage.write_plan(self.args)
        _, manifest = manage.load(self.plan)
        reply = subprocess.CompletedProcess([], 0,
            json.dumps([{"dst":"192.168.254.0/24", "dev":"physical-fixture"}]), "")
        with patch.object(manage, "run", return_value=reply), patch.object(manage, "virsh") as virsh:
            with self.assertRaisesRegex(ValueError, "workstation route"):
                manage.check_subnet(manifest)
            virsh.assert_not_called()

    def test_dangling_disk_symlink_is_rejected_before_qemu_img_can_write(self):
        manage.write_plan(self.args)
        directory, manifest = manage.load(self.plan)
        disk = directory / (manifest["hosts"][0]["name"] + "-system.qcow2")
        target = self.base / "unrelated-file"
        disk.symlink_to(target)
        with patch.object(manage, "check_owner", return_value=False), patch.object(manage, "check_subnet"), patch.object(manage, "run") as run:
            with self.assertRaisesRegex(ValueError, "symlinks"):
                manage.prepare(directory, manifest)
            run.assert_not_called()
        self.assertFalse(target.exists())


if __name__ == "__main__":
    unittest.main()
