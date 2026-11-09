#!/usr/bin/env python3
"""Create and control a disposable, isolated nested-XCP-ng libvirt lab."""

import argparse
import getpass
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import uuid
import xml.etree.ElementTree as ET

URI = "qemu:///system"
NAMESPACE = "https://github.com/z23/xenadmin-rs/lab"
REPOSITORY = Path(__file__).resolve().parents[1]


def run(*args, check=True, input=None):
    return subprocess.run(args, check=check, text=True, capture_output=True, input=input,
                          env={**os.environ, "LC_ALL": "C"})


def virsh(*args, check=True):
    return run("virsh", "--connect", URI, *args, check=check)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def root_path(value):
    path = Path(value).expanduser().resolve()
    if path == REPOSITORY or REPOSITORY in path.parents:
        raise ValueError("Lab state must be outside the source repository")
    return path


def private_network(value):
    network = ipaddress.IPv4Network(value, strict=True)
    if not network.is_private or network.prefixlen != 24 or network.is_loopback or network.is_link_local:
        raise ValueError("Use a private /24 subnet reserved for this disposable lab")
    return str(network)


def gateway(manifest):
    return str(ipaddress.IPv4Network(manifest["subnet"]).network_address + 1)


def installer(manifest):
    """Image booted by `start --install` and the digest it must still match."""
    unattended = manifest.get("unattended")
    if unattended:
        return Path(unattended["iso"]), unattended["iso_sha256"]
    return Path(manifest["iso"]), manifest["iso_sha256"]


def serial_log(directory, host):
    return directory / f"{host['name']}-serial.log"


def element(parent, tag, text=None, **attributes):
    child = ET.SubElement(parent, tag, attributes)
    if text is not None:
        child.text = str(text)
    return child


def render(tree):
    ET.indent(tree)
    return ET.tostring(tree, encoding="unicode") + "\n"


def network_xml(manifest):
    network = ET.Element("network")
    element(network, "name", manifest["name"])
    element(network, "uuid", manifest["network_uuid"])
    metadata = element(network, "metadata")
    element(metadata, "{" + NAMESPACE + "}owner", manifest["id"])
    # No <forward>: guests can talk to each other and the workstation, not production.
    element(network, "bridge", name="xal" + manifest["id"].replace("-", "")[:8], stp="on", delay="0")
    subnet = ipaddress.IPv4Network(manifest["subnet"])
    ip = element(network, "ip", address=str(subnet.network_address + 1), netmask=str(subnet.netmask))
    dhcp = element(ip, "dhcp")
    for host in manifest["hosts"]:
        element(dhcp, "host", mac=host["mac"], name=host["name"], ip=host["ip"])
    return render(network)


def domain_xml(manifest, host, directory, install=False):
    domain = ET.Element("domain", {"type": "kvm"})
    element(domain, "name", host["name"])
    element(domain, "uuid", host["uuid"])
    metadata = element(domain, "metadata")
    element(metadata, "{" + NAMESPACE + "}owner", manifest["id"])
    element(domain, "memory", manifest["memory_mib"], unit="MiB")
    element(domain, "vcpu", manifest["vcpus"])
    os_node = element(domain, "os")
    element(os_node, "type", "hvm", arch="x86_64", machine="q35")
    element(os_node, "boot", dev="cdrom" if install else "hd")
    if install:
        element(os_node, "boot", dev="hd")
    features = element(domain, "features")
    element(features, "acpi")
    element(features, "apic")
    element(domain, "cpu", mode="host-passthrough", check="none", migratable="off")
    element(domain, "clock", offset="utc")
    element(domain, "on_poweroff", "destroy")
    element(domain, "on_reboot", "restart")
    element(domain, "on_crash", "preserve")
    devices = element(domain, "devices")
    # SATA keeps the installer's disk discovery and /dev/disk/by-id names conventional.
    for suffix, target in [("system", "sda"), ("data", "sdb")]:
        disk = element(devices, "disk", type="file", device="disk")
        element(disk, "driver", name="qemu", type="qcow2", cache="none", discard="unmap")
        element(disk, "source", file=str(directory / f"{host['name']}-{suffix}.qcow2"))
        element(disk, "target", dev=target, bus="sata")
        element(disk, "serial", f"lab-{suffix}")
    if install:
        cdrom = element(devices, "disk", type="file", device="cdrom")
        element(cdrom, "driver", name="qemu", type="raw")
        element(cdrom, "source", file=str(installer(manifest)[0]))
        element(cdrom, "target", dev="sdc", bus="sata")
        element(cdrom, "readonly")
    nic = element(devices, "interface", type="network")
    element(nic, "mac", address=host["mac"])
    element(nic, "source", network=manifest["name"])
    element(nic, "model", type="virtio")
    element(devices, "graphics", type="spice", autoport="yes", listen="127.0.0.1")
    video = element(devices, "video")
    element(video, "model", type="vga")
    # Xen and dom0 both log to com1; keep that text in the lab directory for review.
    serial = element(devices, "serial", type="file")
    element(serial, "source", path=str(serial_log(directory, host)), append="on")
    element(serial, "target", port="0")
    return render(domain)


def write_plan(args):
    directory = root_path(args.directory)
    iso = Path(args.iso).expanduser().resolve(strict=True)
    expected = args.sha256.lower()
    if not re.fullmatch(r"[0-9a-f]{64}", expected) or sha256(iso) != expected:
        raise ValueError("Installer SHA-256 does not match the supplied trusted digest")
    if not 1 <= args.hosts <= 3 or args.memory_mib < 8192 or args.vcpus < 2 or args.disk_gib < 64:
        raise ValueError("Use 1–3 hosts, at least 8192 MiB/2 vCPUs/64 GiB per host")
    directory.mkdir(parents=True, mode=0o755, exist_ok=True)
    if any(directory.iterdir()):
        raise ValueError("Plan directory must be empty; existing lab state is never overwritten")
    lab_id = str(uuid.uuid4())
    name = "xenadmin-lab-" + lab_id[:8]
    subnet = ipaddress.IPv4Network(private_network(args.subnet))
    hosts = []
    for index in range(args.hosts):
        mac = "52:54:00:" + ":".join(f"{byte:02x}" for byte in uuid.uuid4().bytes[:3])
        hosts.append({"name":f"{name}-h{index+1}", "uuid":str(uuid.uuid4()),
                      "mac":mac, "ip":str(subnet.network_address + 11 + index)})
    manifest = {"schema":1, "id":lab_id, "name":name, "network_uuid":str(uuid.uuid4()),
                "subnet":str(subnet), "iso":str(iso), "iso_sha256":expected,
                "memory_mib":args.memory_mib, "vcpus":args.vcpus, "disk_gib":args.disk_gib,
                "hosts":hosts}
    (directory / "lab.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (directory / "network.xml").write_text(network_xml(manifest))
    for host in hosts:
        (directory / f"{host['name']}.xml").write_text(domain_xml(manifest, host, directory))
        (directory / f"{host['name']}-install.xml").write_text(domain_xml(manifest, host, directory, True))
    print(f"Prepared {len(hosts)} hosts in {directory}; no hypervisor state changed")


def load(directory):
    directory = root_path(directory)
    manifest = json.loads((directory / "lab.json").read_text())
    if manifest["schema"] != 1:
        raise ValueError("Unsupported lab manifest schema")
    lab_id = str(uuid.UUID(manifest["id"]))
    if manifest["name"] != "xenadmin-lab-" + lab_id[:8]:
        raise ValueError("Invalid lab identity")
    uuid.UUID(manifest["network_uuid"])
    private_network(manifest["subnet"])
    if not 1 <= len(manifest["hosts"]) <= 3:
        raise ValueError("Invalid host count")
    for index, host in enumerate(manifest["hosts"]):
        if host["name"] != f"{manifest['name']}-h{index+1}":
            raise ValueError("Invalid managed host name")
        uuid.UUID(host["uuid"])
        if not re.fullmatch(r"52:54:00(:[0-9a-f]{2}){3}", host["mac"]):
            raise ValueError("Invalid host MAC")
        if ipaddress.IPv4Address(host["ip"]) not in ipaddress.IPv4Network(manifest["subnet"]):
            raise ValueError("Host address is outside the isolated lab network")
    unattended = manifest.get("unattended")
    if unattended is not None:
        iso = Path(unattended["iso"])
        if iso.parent != directory or not re.fullmatch(r"[0-9a-f]{64}", unattended["iso_sha256"]):
            raise ValueError("Invalid unattended installer record")
        if not 1024 <= int(unattended["port"]) <= 65535:
            raise ValueError("Invalid answer file port")
        if set(unattended) - {"iso", "iso_sha256", "port", "answerfile_url"}:
            raise ValueError("Unexpected unattended installer fields")
    return directory, manifest


def check_owner(manifest, host=None):
    name = host["name"] if host else manifest["name"]
    identifier = host["uuid"] if host else manifest["network_uuid"]
    query = "dumpxml" if host else "net-dumpxml"
    existing = virsh(query, name, check=False)
    if existing.returncode:
        # A failed query could also mean a permission/service error; never treat it as proof of absence.
        names = virsh("list" if host else "net-list", "--all", "--name").stdout.splitlines()
        if name in names:
            raise ValueError(f"Cannot inspect existing lab resource {name}")
        return False
    tree = ET.fromstring(existing.stdout)
    owner = tree.find("metadata/{" + NAMESPACE + "}owner")
    if tree.findtext("uuid") != identifier or owner is None or owner.text != manifest["id"]:
        raise ValueError(f"Refusing to operate on {name}: UUID/ownership differs from this lab")
    return True


def stopped(host):
    state = virsh("domstate", host["name"]).stdout.strip()
    if state not in {"shut off", "shutoff"}:
        raise ValueError(f"{host['name']} must be shut off before this operation")


def check_subnet(manifest):
    subnet = ipaddress.IPv4Network(manifest["subnet"])
    bridge = "xal" + manifest["id"].replace("-", "")[:8]
    for route in json.loads(run("ip", "-json", "route", "show", "table", "all").stdout):
        destination = route.get("dst", "default")
        if destination == "default" or route.get("dev") == bridge:
            continue
        if subnet.overlaps(ipaddress.IPv4Network(destination, strict=False)):
            raise ValueError("Lab subnet overlaps an existing workstation route; choose another /24")
    for name in virsh("net-list", "--all", "--name").stdout.splitlines():
        if not name or name == manifest["name"]:
            continue
        network = ET.fromstring(virsh("net-dumpxml", name).stdout)
        for ip in network.findall("ip"):
            if ip.attrib.get("family", "ipv4") != "ipv4":
                continue
            address = ip.attrib["address"]
            mask = ip.attrib.get("netmask", ip.attrib.get("prefix", "24"))
            if subnet.overlaps(ipaddress.IPv4Network(f"{address}/{mask}", strict=False)):
                raise ValueError("Lab subnet overlaps another libvirt network; choose another /24")


def prepare(directory, manifest):
    if sha256(Path(manifest["iso"])) != manifest["iso_sha256"]:
        raise ValueError("Installer changed since plan generation")
    network_exists = check_owner(manifest)
    check_subnet(manifest)
    for host in manifest["hosts"]:
        if check_owner(manifest, host):
            stopped(host)
    for host in manifest["hosts"]:
        for suffix, size in [("system", manifest["disk_gib"]), ("data", 32)]:
            disk = directory / f"{host['name']}-{suffix}.qcow2"
            if disk.is_symlink():
                raise ValueError("Managed disks must not be symlinks")
            if not disk.exists():
                run("qemu-img", "create", "-f", "qcow2", str(disk), f"{size}G")
                disk.chmod(0o600)
            else:
                image = json.loads(run("qemu-img", "info", "--output=json", str(disk)).stdout)
                if image.get("format") != "qcow2" or image.get("backing-filename"):
                    raise ValueError("Managed disks must be standalone qcow2 images")
    if not network_exists:
        (directory / "network.xml").write_text(network_xml(manifest))
        virsh("net-define", str(directory / "network.xml"))
    for host in manifest["hosts"]:
        path = directory / f"{host['name']}.xml"
        path.write_text(domain_xml(manifest, host, directory))
        virsh("define", str(path))
    print("Defined isolated network and powered-off hosts; disks retained on reruns")


def control(args):
    directory, manifest = load(args.directory)
    if args.command == "prepare":
        prepare(directory, manifest)
        return
    # Validate every target before changing any of them.
    if not check_owner(manifest):
        raise ValueError("Lab network has not been defined; run prepare first")
    for host in manifest["hosts"]:
        if not check_owner(manifest, host):
            raise ValueError("Lab host has not been defined; run prepare first")
    if args.command == "start":
        check_subnet(manifest)
        for host in manifest["hosts"]:
            stopped(host)
        if args.install:
            if sha256(Path(manifest["iso"])) != manifest["iso_sha256"]:
                raise ValueError("Installer changed since plan generation")
            image, digest = installer(manifest)
            if sha256(image) != digest:
                raise ValueError("Unattended installer changed since it was generated")
        active = virsh("net-list", "--name").stdout.splitlines()
        if manifest["name"] not in active:
            virsh("net-start", manifest["name"])
        for host in manifest["hosts"]:
            log = serial_log(directory, host)
            if log.is_symlink():
                raise ValueError("Serial logs must not be symlinks")
            log.touch(mode=0o600)
            path = directory / f"{host['name']}.xml"
            path.write_text(domain_xml(manifest, host, directory, args.install))
            virsh("define", str(path))
            virsh("start", host["name"])
    elif args.command == "stop":
        for host in manifest["hosts"]:
            if virsh("domstate", host["name"]).stdout.strip() not in {"shut off", "shutoff"}:
                virsh("destroy" if args.force else "shutdown", host["name"])
    elif args.command == "undefine":
        for host in manifest["hosts"]:
            stopped(host)
        for host in manifest["hosts"]:
            virsh("undefine", host["name"])
        if manifest["name"] in virsh("net-list", "--name").stdout.splitlines():
            virsh("net-destroy", manifest["name"])
        virsh("net-undefine", manifest["name"])
        print("Undefined owned lab resources; disk images and evidence were retained")
    elif args.command == "status":
        for host in manifest["hosts"]:
            print(host["name"], host["ip"], virsh("domstate", host["name"]).stdout.strip())


def answerfile_xml(manifest, host, password_hash):
    """Static-address fresh installation on the first SATA disk, answered without a console."""
    subnet = ipaddress.IPv4Network(manifest["subnet"])
    installation = ET.Element("installation", {"mode": "fresh", "sr-type": "ext"})
    element(installation, "primary-disk", "sda", **{"guest-storage": "true"})
    element(installation, "keymap", "us")
    element(installation, "root-password", password_hash, type="hash")
    element(installation, "source", type="local")
    admin = element(installation, "admin-interface", hwaddr=host["mac"], proto="static")
    element(admin, "ipaddr", host["ip"])
    element(admin, "subnet", str(subnet.netmask))
    element(admin, "gateway", gateway(manifest))
    element(installation, "name-server", gateway(manifest))
    element(installation, "hostname", host["name"])
    element(installation, "timezone", "UTC")
    return '<?xml version="1.0"?>\n' + render(installation)


UNATTENDED_PARAMETERS = "answerfile={url} install atexit=poweroff"


def unattended_isolinux(original, url):
    """Add a default, prompt-free entry that fetches the answer file and powers off afterwards."""
    lines = original.splitlines()
    try:
        label = lines.index("LABEL install")
        append = next(line for line in lines[label + 1:] if line.strip().startswith("APPEND "))
    except (ValueError, StopIteration):
        raise ValueError("Installer boot configuration no longer has the expected install entry")
    marker = " --- /install.img"
    if not append.rstrip().endswith(marker):
        raise ValueError("Installer boot configuration has an unexpected module layout")
    entry = append.rstrip()[: -len(marker)] + " " + UNATTENDED_PARAMETERS.format(url=url) + marker
    kernel = lines[label + 1]
    header = []
    for line in lines[:label]:
        if line.startswith("DEFAULT "):
            line = "DEFAULT unattended"
        elif line.startswith("PROMPT "):
            line = "PROMPT 0"
        elif line.startswith("TIMEOUT "):
            line = "TIMEOUT 10"
        header.append(line)
    return "\n".join(header + ["LABEL unattended", kernel, entry] + lines[label:]) + "\n"


def unattended_grub(original, url):
    lines = original.splitlines()
    try:
        start = lines.index('menuentry "install" {')
        end = lines.index("}", start)
    except ValueError:
        raise ValueError("Installer UEFI configuration no longer has the expected install entry")
    body = []
    for line in lines[start + 1:end]:
        if line.strip().startswith("module2 /boot/vmlinuz"):
            line = line + " " + UNATTENDED_PARAMETERS.format(url=url)
        body.append(line)
    header = ["set timeout=3" if line.startswith("set timeout=") else line for line in lines[:start]]
    return "\n".join(header + ['menuentry "unattended" {'] + body + ["}", ""] + lines[start:]) + "\n"


def remaster(manifest, directory, url):
    """Write a lab-only installer image whose only change is the boot menu."""
    source = Path(manifest["iso"])
    work = directory / "unattended"
    work.mkdir(mode=0o700, exist_ok=True)
    isolinux = run("bsdtar", "-xOf", str(source), "boot/isolinux/isolinux.cfg").stdout
    grub = run("bsdtar", "-xOf", str(source), "EFI/xenserver/grub.cfg").stdout
    (work / "isolinux.cfg").write_text(unattended_isolinux(isolinux, url))
    (work / "grub.cfg").write_text(unattended_grub(grub, url))
    target = directory / f"{manifest['name']}-install.iso"
    if target.is_symlink():
        raise ValueError("Unattended installer path must not be a symlink")
    if target.exists():
        target.unlink()
    run("xorriso", "-indev", str(source), "-outdev", str(target), "-boot_image", "any", "replay",
        "-map", str(work / "isolinux.cfg"), "/boot/isolinux/isolinux.cfg",
        "-map", str(work / "grub.cfg"), "/EFI/xenserver/grub.cfg", "-end")
    target.chmod(0o644)
    return target


def unattended(args):
    """Generate per-host answer files and a boot-menu-only installer variant; no hypervisor change."""
    directory, manifest = load(args.directory)
    if sha256(Path(manifest["iso"])) != manifest["iso_sha256"]:
        raise ValueError("Installer changed since plan generation")
    password = os.environ.get("XENADMIN_LAB_ROOT_PASSWORD") or getpass.getpass("Lab-only root password: ")
    if len(password) < 8 or password != password.strip():
        raise ValueError("Use a lab-only root password of at least eight characters")
    digest = run("openssl", "passwd", "-6", "-stdin", input=password + "\n").stdout.strip()
    if not digest.startswith("$6$"):
        raise ValueError("Could not hash the lab root password")
    answerfiles = directory / "answerfiles"
    answerfiles.mkdir(mode=0o700, exist_ok=True)
    for host in manifest["hosts"]:
        path = answerfiles / f"{host['ip']}.xml"
        if path.is_symlink():
            raise ValueError("Answer files must not be symlinks")
        path.touch(mode=0o600)
        path.write_text(answerfile_xml(manifest, host, digest))
    url = f"http://{gateway(manifest)}:{args.port}/answerfile.xml"
    image = remaster(manifest, directory, url)
    manifest["unattended"] = {"iso": str(image), "iso_sha256": sha256(image), "port": args.port,
                              "answerfile_url": url}
    (directory / "lab.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Wrote {len(manifest['hosts'])} answer files and {image.name}; serve them during start --install")


def answerfile_handler(manifest, answerfiles):
    hosts = {host["ip"] for host in manifest["hosts"]}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            client = self.client_address[0]
            path = answerfiles / f"{client}.xml"
            if client in hosts and self.path == "/answerfile.xml" and path.is_file():
                body = path.read_bytes()
                self.send_response(200)
                self.send_header("Content-Type", "application/xml")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self.send_error(404)

        def log_message(self, format, *args):
            print(self.client_address[0], format % args, flush=True)

    return Handler


def serve(args):
    """Answer each lab host's installer from the isolated bridge address only."""
    directory, manifest = load(args.directory)
    if "unattended" not in manifest:
        raise ValueError("Run unattended first")
    if not check_owner(manifest):
        raise ValueError("Lab network has not been defined; run prepare first")
    if manifest["name"] not in virsh("net-list", "--name").stdout.splitlines():
        virsh("net-start", manifest["name"])
    address = (gateway(manifest), int(manifest["unattended"]["port"]))
    server = ThreadingHTTPServer(address, answerfile_handler(manifest, directory / "answerfiles"))
    print(f"Serving answer files at {manifest['unattended']['answerfile_url']}; stop with Ctrl-C", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


def doctor():
    for tool in ["virsh", "qemu-img", "virt-viewer", "xorriso", "bsdtar", "openssl"]:
        print(f"{tool}: {shutil.which(tool) or 'not installed'}")
    print(f"/dev/kvm accessible: {os.access('/dev/kvm', os.R_OK | os.W_OK)}")
    for vendor in ["intel", "amd"]:
        nested = Path(f"/sys/module/kvm_{vendor}/parameters/nested")
        if nested.exists():
            print(f"kvm_{vendor} nested: {nested.read_text().strip()}")
    if shutil.which("virsh"):
        print(f"libvirt system connection: {'available' if virsh('uri', check=False).returncode == 0 else 'unavailable'}")


def draft(args):
    """Prepare an offline handoff, never a manifest accepted by lifecycle commands."""
    directory = root_path(args.directory)
    if not 1 <= args.hosts <= 3:
        raise ValueError("Use 1–3 disposable hosts")
    subnet = private_network(args.subnet)
    directory.mkdir(parents=True, mode=0o700, exist_ok=True)
    if any(directory.iterdir()):
        raise ValueError("Preparation directory must be empty; existing evidence is never overwritten")
    preparation = {
        "schema": "xenadmin-rs-lab-preparation-v1",
        "state": "awaiting-virtualization-and-verified-installer",
        "target_release": "XCP-ng 8.3",
        "hosts": args.hosts,
        "memory_mib_per_host": 8192,
        "vcpus_per_host": 4,
        "system_disk_gib_per_host": 64,
        "disposable_data_disk_gib_per_host": 32,
        "subnet_candidate": subnet,
        "forwarding": False,
        "autostart": False,
        "installer": None,
        "installer_sha256": None,
        "prerequisites": {
            "kvm_accessible": os.access("/dev/kvm", os.R_OK | os.W_OK),
            "tools": {tool: bool(shutil.which(tool)) for tool in ["virsh", "qemu-img", "qemu-system-x86_64", "ip", "virt-viewer"]},
        },
        "pending_checks": ["KVM access and nested virtualization", "libvirt system permissions", "free RAM and disk space",
                           "subnet overlap against workstation and libvirt routes", "publisher-verified installer SHA-256",
                           "QEMU storage-directory access"],
    }
    (directory / "preparation.json").write_text(json.dumps(preparation, indent=2) + "\n")
    source = REPOSITORY / "lab" / "ACCEPTANCE.md"
    (directory / "ACCEPTANCE.md").write_text(source.read_text())
    (directory / "NEXT.md").write_text(
        "# Local lab preparation\n\n"
        f"Prepared {args.hosts} isolated hosts: 8 GiB RAM, four vCPUs, a 64 GiB system disk and a 32 GiB disposable data disk each.\n\n"
        "No virtual machines, network interfaces, disks or services were created. This preparation file is not an active lab manifest.\n\n"
        "When virtualization is available, run `python3 lab/manage.py doctor` from the application checkout. Obtain the XCP-ng 8.3 installer and independently verify its publisher SHA-256.\n\n"
        "Choose a libvirt-accessible, empty storage directory and run:\n\n```sh\n"
        f"python3 lab/manage.py plan --directory /var/lib/libvirt/images/xenadmin-rs-lab --hosts {args.hosts} --subnet {subnet} --iso /path/to/verified-installer.iso --sha256 VERIFIED_SHA256\n"
        "```\n\nInspect the generated XML, then follow `lab/README.md` for prepare/install/start. The subnet is only a candidate until the overlap check runs.\n\n"
        "Record scenario evidence in ACCEPTANCE.md using synthetic data. Keep credentials and real inventories outside Git.\n"
    )
    print(f"Offline lab preparation written to {directory}; virtualization and installer checks remain pending")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor")
    preparation = commands.add_parser("draft", help="Prepare an offline lab handoff before virtualization/installer are available")
    preparation.add_argument("--directory", required=True)
    preparation.add_argument("--hosts", type=int, default=3)
    preparation.add_argument("--subnet", type=private_network, default="192.168.254.0/24")
    plan = commands.add_parser("plan", help="Write reviewable manifest/XML without creating VMs")
    plan.add_argument("--directory", required=True)
    plan.add_argument("--iso", required=True)
    plan.add_argument("--sha256", required=True, help="Trusted installer checksum from the publisher")
    plan.add_argument("--subnet", type=private_network, default="192.168.254.0/24")
    plan.add_argument("--hosts", type=int, default=2)
    plan.add_argument("--memory-mib", type=int, default=8192)
    plan.add_argument("--vcpus", type=int, default=4)
    plan.add_argument("--disk-gib", type=int, default=64)
    automatic = commands.add_parser("unattended", help="Write answer files and a boot-menu-only installer variant")
    automatic.add_argument("--directory", required=True)
    automatic.add_argument("--port", type=int, default=8000)
    answer = commands.add_parser("serve", help="Serve the answer files on the lab bridge during start --install")
    answer.add_argument("--directory", required=True)
    for name in ["prepare", "start", "stop", "status", "undefine"]:
        command = commands.add_parser(name)
        command.add_argument("--directory", required=True)
        if name == "start":
            command.add_argument("--install", action="store_true", help="Boot the checked installer ISO")
        if name == "stop":
            command.add_argument("--force", action="store_true", help="Cut power to these owned disposable hosts")
    args = parser.parse_args()
    try:
        if args.command == "doctor":
            doctor()
        elif args.command == "draft":
            draft(args)
        elif args.command == "plan":
            write_plan(args)
        elif args.command == "unattended":
            unattended(args)
        elif args.command == "serve":
            serve(args)
        else:
            control(args)
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError, ET.ParseError) as error:
        parser.exit(1, f"Lab operation failed: {error}\n")


if __name__ == "__main__":
    main()
