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

    def test_draft_needs_no_installer_or_hypervisor_and_is_not_an_active_manifest(self):
        args = argparse.Namespace(directory=str(self.base / "preparation"), hosts=3, subnet="192.168.254.0/24")
        with patch.object(manage, "run") as run:
            manage.draft(args)
            run.assert_not_called()
        directory = Path(args.directory)
        document = json.loads((directory / "preparation.json").read_text())
        self.assertIsNone(document["installer"])
        self.assertFalse(document["forwarding"])
        self.assertEqual(document["hosts"], 3)
        self.assertFalse((directory / "lab.json").exists())
        self.assertIn("Pending", (directory / "ACCEPTANCE.md").read_text())
        with self.assertRaises(OSError):
            manage.load(directory)
        with self.assertRaisesRegex(ValueError, "never overwritten"):
            manage.draft(args)

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

    def test_policy_routes_are_checked_after_the_bridge_exists_not_before(self):
        manage.write_plan(self.args)
        _, manifest = manage.load(self.plan)
        bridge = manage.bridge_name(manifest)
        main_only = subprocess.CompletedProcess([], 0, json.dumps([{"dst": "10.0.0.0/8", "dev": "lan"}]), "")
        with patch.object(manage, "run", return_value=main_only) as run, patch.object(manage, "virsh") as virsh:
            virsh.return_value = subprocess.CompletedProcess([], 0, "", "")
            manage.check_subnet(manifest)
            self.assertEqual(run.call_args.args, ("ip", "-json", "route", "show"))
        captured = subprocess.CompletedProcess([], 0, json.dumps([{"dst": "192.168.254.11", "dev": "vpn0", "table": "65743"}]), "")
        with patch.object(manage, "run", return_value=captured):
            with self.assertRaisesRegex(ValueError, "routes via vpn0"):
                manage.check_routing(manifest)
        direct = subprocess.CompletedProcess([], 0, json.dumps([{"dst": "192.168.254.11", "dev": bridge}]), "")
        with patch.object(manage, "run", return_value=direct) as run:
            manage.check_routing(manifest)
            self.assertEqual(run.call_count, len(manifest["hosts"]))

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

    def test_domains_use_sata_disks_file_serial_logs_and_the_selected_installer(self):
        manage.write_plan(self.args)
        directory, manifest = manage.load(self.plan)
        host = manifest["hosts"][0]
        domain = ET.fromstring(manage.domain_xml(manifest, host, directory, install=True))
        disks = domain.findall("devices/disk[@device='disk']")
        self.assertEqual([d.find("target").attrib["bus"] for d in disks], ["sata", "sata"])
        self.assertEqual([d.find("target").attrib["dev"] for d in disks], ["sda", "sdb"])
        self.assertEqual(domain.find("devices/disk[@device='cdrom']/source").attrib["file"], str(self.iso))
        serial = domain.find("devices/serial")
        self.assertEqual(serial.attrib["type"], "file")
        self.assertEqual(serial.find("source").attrib["path"], str(directory / (host["name"] + "-serial.log")))
        manifest["unattended"] = {"iso": str(directory / "lab.iso"), "iso_sha256": "a" * 64, "port": 8000,
                                  "answerfile_url": "http://192.168.254.1:8000/answerfile.xml"}
        domain = ET.fromstring(manage.domain_xml(manifest, host, directory, install=True))
        self.assertEqual(domain.find("devices/disk[@device='cdrom']/source").attrib["file"], str(directory / "lab.iso"))

    def test_answer_files_are_static_hashed_and_scoped_to_each_host(self):
        manage.write_plan(self.args)
        _, manifest = manage.load(self.plan)
        host = manifest["hosts"][1]
        document = manage.answerfile_xml(manifest, host, "$6$fixture$hash")
        self.assertTrue(document.startswith('<?xml version="1.0"?>'))
        root = ET.fromstring(document)
        self.assertEqual(root.attrib, {"mode": "fresh", "sr-type": "ext"})
        self.assertEqual(root.findtext("primary-disk"), "sda")
        self.assertEqual(root.find("root-password").attrib["type"], "hash")
        self.assertEqual(root.findtext("root-password"), "$6$fixture$hash")
        self.assertEqual(root.find("source").attrib["type"], "local")
        admin = root.find("admin-interface")
        self.assertEqual(admin.attrib, {"hwaddr": host["mac"], "proto": "static"})
        self.assertEqual(admin.findtext("ipaddr"), "192.168.254.12")
        self.assertEqual(admin.findtext("subnet"), "255.255.255.0")
        self.assertEqual(admin.findtext("gateway"), "192.168.254.1")
        self.assertEqual(root.findtext("hostname"), host["name"])
        self.assertIsNone(root.find("ntp-server"))

    ISOLINUX = (
        "SERIAL 0 115200\nDEFAULT install\nPROMPT 1\nTIMEOUT 50\n\nLABEL install\n\tKERNEL mboot.c32\n"
        "\tAPPEND /boot/xen.gz dom0_mem=max:8192M console=com1,vga --- /boot/vmlinuz console=hvc0 console=tty0 --- /install.img\n"
        "LABEL shell\n\tKERNEL mboot.c32\n\tAPPEND /boot/xen.gz --- /boot/vmlinuz bash-shell --- /install.img\n")
    GRUB = (
        'set default="0"\nset timeout=5\n\nmenuentry "install" {\n    multiboot2 /boot/xen.gz console=com1,vga\n'
        '    module2 /boot/vmlinuz console=hvc0 console=tty0\n    module2 /install.img\n}\n\nmenuentry "shell" {\n'
        '    multiboot2 /boot/xen.gz\n    module2 /boot/vmlinuz bash-shell\n    module2 /install.img\n}\n')

    def test_boot_menus_gain_a_default_unattended_entry_and_keep_the_originals(self):
        url = "http://192.168.254.1:8000/answerfile.xml"
        isolinux = manage.unattended_isolinux(self.ISOLINUX, url)
        self.assertIn("DEFAULT unattended\nPROMPT 0\nTIMEOUT 10\n", isolinux)
        self.assertIn("LABEL unattended\n\tKERNEL mboot.c32\n\tAPPEND /boot/xen.gz dom0_mem=max:8192M console=com1,vga"
                      " --- /boot/vmlinuz console=hvc0 console=tty0 answerfile=" + url +
                      " install atexit=poweroff --- /install.img\nLABEL install\n", isolinux)
        self.assertIn("LABEL shell", isolinux)
        self.assertEqual(isolinux.count("answerfile="), 1)
        grub = manage.unattended_grub(self.GRUB, url)
        self.assertTrue(grub.startswith('set default="0"\nset timeout=3\n\nmenuentry "unattended" {\n'))
        self.assertIn("    module2 /boot/vmlinuz console=hvc0 console=tty0 answerfile=" + url + " install atexit=poweroff\n", grub)
        self.assertEqual(grub.count('menuentry "install" {'), 1)
        self.assertEqual(grub.count("answerfile="), 1)
        with self.assertRaisesRegex(ValueError, "install entry"):
            manage.unattended_isolinux("DEFAULT other\n", url)
        with self.assertRaisesRegex(ValueError, "module layout"):
            manage.unattended_isolinux("LABEL install\n\tKERNEL mboot.c32\n\tAPPEND /boot/xen.gz --- /x\n", url)

    def test_unattended_records_no_password_and_serves_only_known_hosts(self):
        manage.write_plan(self.args)
        directory, manifest = manage.load(self.plan)
        image = directory / (manifest["name"] + "-install.iso")

        def fake_run(*command, check=True, input=None):
            if command[:3] == ("openssl", "passwd", "-6"):
                self.assertEqual(input, "fixture-lab-secret\n")
                return subprocess.CompletedProcess(command, 0, "$6$salt$digest\n", "")
            if command[0] == "bsdtar":
                text = self.ISOLINUX if command[-1].endswith("isolinux.cfg") else self.GRUB
                return subprocess.CompletedProcess(command, 0, text, "")
            if command[0] == "xorriso":
                image.write_bytes(b"remastered")
                return subprocess.CompletedProcess(command, 0, "", "")
            raise AssertionError(command)

        with patch.object(manage, "run", side_effect=fake_run), patch.object(manage, "virsh") as virsh, \
                patch.dict(manage.os.environ, {"XENADMIN_LAB_ROOT_PASSWORD": "fixture-lab-secret"}):
            manage.unattended(argparse.Namespace(directory=str(self.plan), port=8000))
            virsh.assert_not_called()
        text = (directory / "lab.json").read_text()
        self.assertNotIn("fixture-lab-secret", text)
        self.assertNotIn("$6$", text)
        directory, manifest = manage.load(self.plan)
        self.assertEqual(manifest["unattended"]["iso"], str(image))
        self.assertEqual(manifest["unattended"]["iso_sha256"], hashlib.sha256(b"remastered").hexdigest())
        answerfiles = directory / "answerfiles"
        self.assertEqual(oct(answerfiles.stat().st_mode & 0o777), "0o700")
        for host in manifest["hosts"]:
            path = answerfiles / (host["ip"] + ".xml")
            self.assertEqual(oct(path.stat().st_mode & 0o777), "0o600")
            self.assertIn("$6$salt$digest", path.read_text())
            self.assertNotIn("fixture-lab-secret", path.read_text())
        self.assertIn("answerfile=http://192.168.254.1:8000/answerfile.xml install atexit=poweroff",
                      (directory / "unattended" / "isolinux.cfg").read_text())
        known = manifest["hosts"][0]["ip"]
        handler = manage.answerfile_handler({"hosts": [{"ip": "127.0.0.1"}]}, answerfiles)
        (answerfiles / "127.0.0.1.xml").write_text((answerfiles / (known + ".xml")).read_text())
        import http.client
        server = manage.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        import threading
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            client = http.client.HTTPConnection("127.0.0.1", server.server_address[1], timeout=5)
            client.request("GET", "/answerfile.xml")
            reply = client.getresponse()
            self.assertEqual(reply.status, 200)
            self.assertIn(b"$6$salt$digest", reply.read())
            client.request("GET", "/" + known + ".xml")
            self.assertEqual(client.getresponse().status, 404)
        finally:
            server.shutdown()
            server.server_close()
        stranger = manage.answerfile_handler({"hosts": [{"ip": "192.0.2.9"}]}, answerfiles)
        server = manage.ThreadingHTTPServer(("127.0.0.1", 0), stranger)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            client = http.client.HTTPConnection("127.0.0.1", server.server_address[1], timeout=5)
            client.request("GET", "/answerfile.xml")
            self.assertEqual(client.getresponse().status, 404)
        finally:
            server.shutdown()
            server.server_close()

    def test_tampered_unattended_record_is_rejected(self):
        manage.write_plan(self.args)
        path = self.plan / "lab.json"
        manifest = json.loads(path.read_text())
        manifest["unattended"] = {"iso": str(self.base / "elsewhere.iso"), "iso_sha256": "a" * 64, "port": 8000,
                                  "answerfile_url": "http://192.168.254.1:8000/answerfile.xml"}
        path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "unattended installer record"):
            manage.load(self.plan)


if __name__ == "__main__":
    unittest.main()
