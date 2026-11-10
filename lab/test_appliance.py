import ipaddress
import json
import tarfile
import io
import unittest
import xml.etree.ElementTree as ET

import appliance


def manifest():
    return {
        "schema": 1, "id": "9e8e4f49-b358-4c76-a189-d9f5e19c1061", "name": "xenadmin-lab-9e8e4f49",
        "network_uuid": "113c696c-2f29-444a-ba7e-39f379fa5094", "subnet": "192.168.254.0/24",
        "hosts": [{"name": "xenadmin-lab-9e8e4f49-h1", "uuid": "00000000-0000-4000-8000-000000000001",
                   "mac": "52:54:00:00:00:01", "ip": "192.168.254.11"}],
    }


class ApplianceTests(unittest.TestCase):
    def test_storage_network_is_isolated_and_addressed_from_its_subnet(self):
        m = manifest()
        appliance.ensure_storage(m, "192.168.253.0/24")
        root = ET.fromstring(appliance.storage_network_xml(m))
        self.assertIsNone(root.find("forward"))
        self.assertIsNone(root.find("ip/dhcp"))
        self.assertEqual(root.find("ip").get("address"), "192.168.253.1")
        self.assertEqual(root.find("bridge").get("name"), "xas9e8e4f49")
        with self.assertRaises(ValueError):
            appliance.ensure_storage(manifest(), "192.168.254.0/24")

    def test_nas_overlay_configures_static_addresses_exports_and_boot_services(self):
        m = manifest()
        appliance.ensure_storage(m, "192.168.253.0/24")
        nas = appliance.ensure_nas(m)
        self.assertEqual(nas["ip"], "192.168.254.20")
        self.assertEqual(nas["storage_ip"], "192.168.253.20")
        with tarfile.open(fileobj=io.BytesIO(appliance.nas_overlay(m)), mode="r:gz") as tar:
            names = tar.getnames()
            interfaces = tar.extractfile("etc/network/interfaces").read().decode()
            exports = tar.extractfile("etc/exports").read().decode()
            start = tar.getmember("etc/local.d/nas.start")
            link = tar.getmember("etc/runlevels/default/local")
        self.assertIn("etc/.default_boot_services", names)
        self.assertIn("address 192.168.254.20", interfaces)
        self.assertIn("address 192.168.253.20", interfaces)
        self.assertIn("/srv/nfs 192.168.254.0/24(rw,sync,no_subtree_check,no_root_squash)", exports)
        self.assertIn("/srv/nfs/heartbeat 192.168.253.0/24(", exports)
        self.assertEqual(start.mode & 0o111, 0o111)
        self.assertTrue(link.issym())
        self.assertEqual(link.linkname, "/etc/init.d/local")

    def test_nas_overlay_brings_up_iscsi_targets_and_keeps_the_chap_secret_private(self):
        m = manifest()
        nas = appliance.ensure_nas(m)
        self.assertTrue(nas["iscsi"]["open_iqn"].endswith(".open"))
        with tarfile.open(fileobj=io.BytesIO(appliance.nas_overlay(m, ("labchap", "s3cret"))), mode="r:gz") as tar:
            start = tar.extractfile("etc/local.d/nas.start").read().decode()
            chap = tar.extractfile("etc/lab-chap").read().decode()
            mode = tar.getmember("etc/lab-chap").mode
        self.assertIn(f"/iscsi create {nas['iscsi']['open_iqn']}", start)
        self.assertIn("authentication=1", start)
        self.assertIn("truncate -s 20G", start)
        self.assertEqual(chap, "labchap s3cret\n")
        self.assertEqual(mode & 0o777, 0o600)
        self.assertNotIn("s3cret", json.dumps(m))

    def test_guest_overlay_ships_the_agent_service_and_dhcp(self):
        with tarfile.open(fileobj=io.BytesIO(appliance.guest_overlay()), mode="r:gz") as tar:
            service = tar.extractfile("etc/init.d/xe-guest-utilities").read().decode()
            interfaces = tar.extractfile("etc/network/interfaces").read().decode()
        self.assertIn("/usr/sbin/xe-daemon", service)
        self.assertIn("iface eth0 inet dhcp", interfaces)

    def test_console_parameters_are_added_once(self):
        text = "APPEND modules=loop,squashfs quiet \n"
        self.assertEqual(appliance._console_cfg(text).count("console=ttyS0"), 1)
        self.assertTrue(appliance._console_cfg(text).rstrip().endswith("console=ttyS0,115200"))

    def test_nas_domain_has_cdrom_data_disk_and_both_nics(self):
        m = manifest()
        appliance.ensure_storage(m, "192.168.253.0/24")
        nas = appliance.ensure_nas(m)
        nas["iso"] = "/lab/nas.iso"
        root = ET.fromstring(appliance.nas_domain_xml(m, appliance.Path("/lab")))
        disks = root.findall("devices/disk")
        self.assertEqual([d.get("device") for d in disks], ["cdrom", "disk"])
        self.assertEqual(disks[1].find("source").get("file"), "/lab/xenadmin-lab-9e8e4f49-nas-data.qcow2")
        nics = root.findall("devices/interface")
        self.assertEqual([n.find("source").get("network") for n in nics],
                         ["xenadmin-lab-9e8e4f49", "xenadmin-lab-9e8e4f49-storage"])
        self.assertEqual(root.find("devices/serial/source").get("path"), "/lab/xenadmin-lab-9e8e4f49-nas-serial.log")


if __name__ == "__main__":
    unittest.main()
