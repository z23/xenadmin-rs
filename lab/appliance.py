"""Optional lab appliances built on the workstation without privileges or guest internet.

- An isolated storage network with extra virtio NICs on every lab host (bonds, secondary
  interfaces, a storage path to the appliance).
- An Alpine Linux NFS appliance booted from a remastered Alpine ISO whose root overlay and
  packages travel on the ISO itself, so nothing inside the isolated lab downloads anything.
- A tools-equipped guest image: the same Alpine ISO remastered with xe-guest-utilities, kept
  isohybrid so the client can import it as a bootable disk image.

Everything is recorded in the private lab manifest; no credentials are created or stored.
"""
import io
import ipaddress
import json
import secrets
import shutil
import tarfile
import tempfile
import time
import uuid
from pathlib import Path

import manage

COMMANDS = {"storage-network", "nas-image", "nas", "guest-image"}
NAS_IP_OFFSET = 20


def _mac():
    return "52:54:00:" + ":".join(f"{b:02x}" for b in secrets.token_bytes(3))


def save(directory, manifest):
    path = directory / "lab.json"
    tmp = directory / "lab.json.tmp"
    tmp.write_text(json.dumps(manifest, indent=2) + "\n")
    tmp.chmod(0o600)
    tmp.replace(path)


def storage_network_xml(manifest):
    storage = manifest["storage"]
    network = manage.ET.Element("network")
    manage.element(network, "name", storage["network"])
    manage.element(network, "uuid", storage["network_uuid"])
    metadata = manage.element(network, "metadata")
    manage.element(metadata, "{" + manage.NAMESPACE + "}owner", manifest["id"])
    # No <forward> and no DHCP: a plain isolated layer-2 segment for storage traffic.
    manage.element(network, "bridge", name=storage["bridge"], stp="on", delay="0")
    subnet = ipaddress.IPv4Network(storage["subnet"])
    manage.element(network, "ip", address=str(subnet.network_address + 1), netmask=str(subnet.netmask))
    return manage.render(network)


def ensure_storage(manifest, subnet):
    storage = manifest.get("storage")
    if storage is None:
        subnet_net = manage.private_network(subnet)
        if ipaddress.IPv4Network(subnet_net).overlaps(ipaddress.IPv4Network(manifest["subnet"])):
            raise ValueError("Storage subnet overlaps the lab subnet")
        storage = manifest["storage"] = {
            "subnet": subnet_net,
            "network": manifest["name"] + "-storage",
            "network_uuid": str(uuid.uuid4()),
            "bridge": "xas" + manifest["id"].replace("-", "")[:8],
            "host_nics": {},
        }
    return storage


def storage_network(args):
    directory, manifest = manage.load(args.directory)
    storage = ensure_storage(manifest, args.subnet)
    save(directory, manifest)
    defined = manage.virsh("net-list", "--all", "--name").stdout.splitlines()
    if storage["network"] not in defined:
        path = directory / "storage-network.xml"
        path.write_text(storage_network_xml(manifest))
        manage.virsh("net-define", str(path))
    if storage["network"] not in manage.virsh("net-list", "--name").stdout.splitlines():
        manage.virsh("net-start", storage["network"])
    for host in manifest["hosts"]:
        macs = storage["host_nics"].setdefault(host["name"], [])
        while len(macs) < args.nics:
            macs.append(_mac())
        save(directory, manifest)
        attached = {
            line.split()[-1].lower()
            for line in manage.virsh("domiflist", host["name"]).stdout.splitlines()[2:]
            if line.strip()
        }
        running = manage.virsh("domstate", host["name"]).stdout.strip() == "running"
        for mac in macs:
            if mac in attached:
                continue
            flags = ["--live", "--config"] if running else ["--config"]
            try:
                manage.virsh("attach-interface", host["name"], "network", storage["network"],
                             "--model", "virtio", "--mac", mac, *flags)
                print(host["name"], mac, "attached", "live" if running else "for the next start")
            except manage.subprocess.CalledProcessError as error:
                if not running:
                    raise
                manage.virsh("attach-interface", host["name"], "network", storage["network"],
                             "--model", "virtio", "--mac", mac, "--config")
                print(host["name"], mac, "attached for the next domain start only (a power cycle, not an OS reboot):",
                      error.stderr.strip().splitlines()[-1] if error.stderr else "live attach refused")


def ensure_nas(manifest):
    nas = manifest.get("nas")
    if nas is None:
        subnet = ipaddress.IPv4Network(manifest["subnet"])
        nas = manifest["nas"] = {
            "name": manifest["name"] + "-nas",
            "uuid": str(uuid.uuid4()),
            "mac": _mac(),
            "storage_mac": _mac(),
            "ip": str(subnet.network_address + NAS_IP_OFFSET),
            "memory_mib": 1024,
            "data_gib": 64,
            "export": "/srv/nfs",
        }
    if "storage" in manifest and "storage_ip" not in nas:
        nas["storage_ip"] = str(ipaddress.IPv4Network(manifest["storage"]["subnet"]).network_address + NAS_IP_OFFSET)
    return nas


def _tar_entry(tar, name, data=None, mode=0o644, link=None):
    info = tarfile.TarInfo(name)
    info.mtime = int(time.time())
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    if link is not None:
        info.type = tarfile.SYMTYPE
        info.linkname = link
        info.mode = 0o777
        tar.addfile(info)
        return
    payload = data.encode() if isinstance(data, str) else data
    info.size = len(payload)
    info.mode = mode
    tar.addfile(info, io.BytesIO(payload))


def _interfaces(entries):
    lines = ["auto lo", "iface lo inet loopback", ""]
    for name, config in entries:
        lines += [f"auto {name}", f"iface {name} inet {config['mode']}"]
        if config["mode"] == "static":
            lines += [f"    address {config['address']}", f"    netmask {config['netmask']}"]
        lines.append("")
    return "\n".join(lines)


def nas_overlay(manifest):
    """Alpine diskless overlay for the NFS appliance; applied by the initramfs at every boot."""
    nas = manifest["nas"]
    lab = ipaddress.IPv4Network(manifest["subnet"])
    interfaces = [("eth0", {"mode": "static", "address": nas["ip"], "netmask": str(lab.netmask)})]
    networks = [str(lab)]
    if "storage_ip" in nas:
        storage = ipaddress.IPv4Network(manifest["storage"]["subnet"])
        interfaces.append(("eth1", {"mode": "static", "address": nas["storage_ip"], "netmask": str(storage.netmask)}))
        networks.append(str(storage))
    export = nas["export"]
    options = "rw,sync,no_subtree_check,no_root_squash"
    exports = "".join(f"{export} {net}({options})\n" for net in networks)
    for sub in ("sr", "iso", "heartbeat"):
        exports += "".join(f"{export}/{sub} {net}({options})\n" for net in networks)
    start = f"""#!/bin/sh
# Runs on every boot of the diskless appliance: install the packages shipped on the ISO,
# prepare the data disk and export it. Progress goes to the serial console for the lab log.
log() {{ echo "lab-nas: $*" > /dev/console; logger -t lab-nas "$*" 2>/dev/null; }}
EXTRA=$(ls -d /media/*/extra 2>/dev/null | head -n 1)
log "media: $(ls /media | tr '\n' ' ') extra: ${{EXTRA:-none}} ($(ls "$EXTRA" 2>/dev/null | wc -l) files)"
if out=$(apk add --allow-untrusted --force-non-repository "$EXTRA"/*.apk 2>&1); then
    log "packages installed"
else
    log "package installation failed: $(echo "$out" | grep -v '^(' | tail -n 4 | tr '\n' ' ')"
fi
DEV=$(findfs LABEL=labnfs 2>/dev/null || echo /dev/vda)
if ! blkid "$DEV" | grep -q 'TYPE="ext4"'; then
    mkfs.ext4 -q -F -L labnfs "$DEV" && log "formatted $DEV"
fi
mkdir -p {export}
if mount -t ext4 "$DEV" {export}; then
    log "mounted $DEV at {export}"
    for d in sr iso heartbeat; do mkdir -p {export}/$d; chmod 1777 {export}/$d; done
    log "nfsd module: $(modprobe nfsd 2>&1 && echo loaded)"
    log "rpcbind: $(rc-service rpcbind start 2>&1 | tr '\\n' ' ' | cut -c1-300)"
    rc-service rpc.statd start >/dev/null 2>&1
    if ! out=$(rc-service nfs start 2>&1); then
        log "nfs service: $(echo "$out" | tr '\\n' ' ' | cut -c1-200)"
        log "statd: $(rc-service rpc.statd start 2>&1 | tr '\\n' ' ' | cut -c1-200) $(grep -i statd /var/log/messages 2>/dev/null | tail -n 2 | tr '\\n' ' ' | cut -c1-300)"
        # NFSv4 and plain v3 mounts need nfsd and mountd only; lock recovery notification is
        # not needed for the lab repositories, so bring the daemons up directly.
        exportfs -r && rpc.nfsd 8 && rpc.mountd && log "nfsd and mountd started directly"
    else
        log "nfs service started"
    fi
    log "rpc: $(rpcinfo -p 2>/dev/null | awk '{{print $5}}' | sort -u | tr '\\n' ' ')"
    log "exports: $(exportfs 2>&1 | tr '\\n' ' ' | cut -c1-300)"
else
    log "mount failed for $DEV"
fi
log "ready {nas['ip']}{' ' + nas['storage_ip'] if 'storage_ip' in nas else ''}"
"""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        _tar_entry(tar, "etc/.default_boot_services", "")
        _tar_entry(tar, "etc/hostname", nas["name"] + "\n")
        _tar_entry(tar, "etc/network/interfaces", _interfaces(interfaces))
        _tar_entry(tar, "etc/apk/world", "alpine-base\ne2fsprogs\n")
        _tar_entry(tar, "etc/exports", exports)
        _tar_entry(tar, "etc/local.d/nas.start", start, mode=0o755)
        _tar_entry(tar, "etc/runlevels/boot/networking", link="/etc/init.d/networking")
        _tar_entry(tar, "etc/runlevels/default/local", link="/etc/init.d/local")
    return buffer.getvalue()


GUEST_SERVICE = """#!/sbin/openrc-run
description="XCP-ng guest agent"
command="/usr/sbin/xe-daemon"
command_args="-p /run/xe-daemon.pid"
command_background=true
pidfile="/run/xe-daemon.pid"

depend() {
    need localmount
    after net
}

start_pre() {
    modprobe xenfs 2>/dev/null
    mountpoint -q /proc/xen || mount -t xenfs xenfs /proc/xen 2>/dev/null
}
"""


def guest_overlay(hostname="lab-tools"):
    """Alpine diskless overlay for the tools-equipped guest: DHCP, guest agent, console login."""
    start = """#!/bin/sh
log() { echo "lab-tools: $*" > /dev/console; }
EXTRA=$(ls -d /media/*/extra 2>/dev/null | head -n 1)
if out=$(apk add --allow-untrusted --force-non-repository "$EXTRA"/*.apk 2>&1); then
    log "guest tools installed"
    rc-service xe-guest-utilities start && log "guest agent running"
else
    log "guest tools installation failed: $(echo "$out" | tail -n 3 | tr '\n' ' ')"
fi
"""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        _tar_entry(tar, "etc/.default_boot_services", "")
        _tar_entry(tar, "etc/hostname", hostname + "\n")
        _tar_entry(tar, "etc/network/interfaces", _interfaces([("eth0", {"mode": "dhcp"})]))
        _tar_entry(tar, "etc/apk/world", "alpine-base\ne2fsprogs\n")
        _tar_entry(tar, "etc/init.d/xe-guest-utilities", GUEST_SERVICE, mode=0o755)
        _tar_entry(tar, "etc/local.d/tools.start", start, mode=0o755)
        _tar_entry(tar, "etc/runlevels/boot/networking", link="/etc/init.d/networking")
        _tar_entry(tar, "etc/runlevels/default/local", link="/etc/init.d/local")
    return buffer.getvalue()


def _console_cfg(text):
    return text.replace("quiet", "quiet console=tty0 console=ttyS0,115200", 1)


def remaster(alpine_iso, digest, apks, overlay, overlay_name, output):
    """Add an overlay, an /extra package directory and serial console parameters to the
    Alpine ISO, keeping its El Torito and isohybrid boot records."""
    if manage.sha256(Path(alpine_iso)) != digest:
        raise ValueError("Alpine ISO does not match the trusted digest")
    apks = Path(apks)
    files = sorted(p for p in apks.iterdir() if p.suffix == ".apk")
    if not files:
        raise ValueError("No .apk files in the package directory")
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        (work / "overlay.tar.gz").write_bytes(overlay)
        extra = work / "extra"
        extra.mkdir()
        for file in files:
            shutil.copy2(file, extra / file.name)
        for name, target in (("syslinux.cfg", "/boot/syslinux/syslinux.cfg"), ("grub.cfg", "/boot/grub/grub.cfg")):
            manage.run("xorriso", "-osirrox", "on", "-indev", str(alpine_iso), "-extract", target, str(work / name))
            (work / name).chmod(0o644)
            (work / name).write_text(_console_cfg((work / name).read_text()))
        output = Path(output)
        if output.exists():
            output.unlink()
        manage.run(
            "xorriso", "-indev", str(alpine_iso), "-outdev", str(output), "-boot_image", "any", "replay",
            "-overwrite", "on",
            "-map", str(work / "overlay.tar.gz"), f"/{overlay_name}.apkovl.tar.gz",
            "-map", str(extra), "/extra",
            "-map", str(work / "syslinux.cfg"), "/boot/syslinux/syslinux.cfg",
            "-map", str(work / "grub.cfg"), "/boot/grub/grub.cfg",
        )
        output.chmod(0o600)
    return manage.sha256(output)


def nas_image(args):
    directory, manifest = manage.load(args.directory)
    nas = ensure_nas(manifest)
    output = directory / "nas.iso"
    digest = remaster(args.alpine_iso, args.sha256, args.apks, nas_overlay(manifest), nas["name"], output)
    nas["iso"] = str(output)
    nas["iso_sha256"] = digest
    nas["alpine_iso_sha256"] = args.sha256
    save(directory, manifest)
    print("Built", output, digest)


def guest_image(args):
    directory, manifest = manage.load(args.directory)
    output = directory / "guest-tools.iso"
    digest = remaster(args.alpine_iso, args.sha256, args.apks, guest_overlay(), "lab-tools", output)
    manifest["guest_tools_image"] = {"path": str(output), "sha256": digest, "alpine_iso_sha256": args.sha256}
    save(directory, manifest)
    print("Built", output, digest)


def nas_domain_xml(manifest, directory):
    nas = manifest["nas"]
    domain = manage.ET.Element("domain", {"type": "kvm"})
    manage.element(domain, "name", nas["name"])
    manage.element(domain, "uuid", nas["uuid"])
    metadata = manage.element(domain, "metadata")
    manage.element(metadata, "{" + manage.NAMESPACE + "}owner", manifest["id"])
    manage.element(domain, "memory", nas["memory_mib"], unit="MiB")
    manage.element(domain, "vcpu", 1)
    os_node = manage.element(domain, "os")
    manage.element(os_node, "type", "hvm", arch="x86_64", machine="q35")
    manage.element(os_node, "boot", dev="cdrom")
    features = manage.element(domain, "features")
    manage.element(features, "acpi")
    manage.element(features, "apic")
    manage.element(domain, "cpu", mode="host-passthrough", check="none", migratable="off")
    manage.element(domain, "clock", offset="utc")
    manage.element(domain, "on_poweroff", "destroy")
    manage.element(domain, "on_reboot", "restart")
    manage.element(domain, "on_crash", "preserve")
    devices = manage.element(domain, "devices")
    cdrom = manage.element(devices, "disk", type="file", device="cdrom")
    manage.element(cdrom, "driver", name="qemu", type="raw")
    manage.element(cdrom, "source", file=nas["iso"])
    manage.element(cdrom, "target", dev="sda", bus="sata")
    manage.element(cdrom, "readonly")
    data = manage.element(devices, "disk", type="file", device="disk")
    manage.element(data, "driver", name="qemu", type="qcow2", cache="none", discard="unmap")
    manage.element(data, "source", file=str(directory / f"{nas['name']}-data.qcow2"))
    manage.element(data, "target", dev="vda", bus="virtio")
    manage.element(data, "serial", "lab-nfs")
    for mac, network in [(nas["mac"], manifest["name"])] + (
        [(nas["storage_mac"], manifest["storage"]["network"])] if "storage" in manifest else []
    ):
        nic = manage.element(devices, "interface", type="network")
        manage.element(nic, "mac", address=mac)
        manage.element(nic, "source", network=network)
        manage.element(nic, "model", type="virtio")
    manage.element(devices, "graphics", type="spice", autoport="yes", listen="127.0.0.1")
    video = manage.element(devices, "video")
    manage.element(video, "model", type="vga")
    serial = manage.element(devices, "serial", type="file")
    manage.element(serial, "source", path=str(directory / f"{nas['name']}-serial.log"), append="on")
    manage.element(serial, "target", port="0")
    return manage.render(domain)


def nas(args):
    directory, manifest = manage.load(args.directory)
    if "nas" not in manifest or "iso" not in manifest["nas"]:
        raise ValueError("Build the appliance image first with nas-image")
    nas = manifest["nas"]
    defined = manage.virsh("list", "--all", "--name").stdout.splitlines()
    if args.action == "start":
        # libvirt chowns an in-use image to the QEMU user, so the check belongs to start only.
        if manage.sha256(Path(nas["iso"])) != nas["iso_sha256"]:
            raise ValueError("Appliance image changed since it was built")
        disk = directory / f"{nas['name']}-data.qcow2"
        if not disk.exists():
            manage.run("qemu-img", "create", "-f", "qcow2", str(disk), f"{nas['data_gib']}G")
            disk.chmod(0o600)
        log = directory / f"{nas['name']}-serial.log"
        log.touch(mode=0o600)
        path = directory / f"{nas['name']}.xml"
        path.write_text(nas_domain_xml(manifest, directory))
        manage.virsh("define", str(path))
        if manage.virsh("domstate", nas["name"]).stdout.strip() != "running":
            manage.virsh("start", nas["name"])
        print("Started", nas["name"], nas["ip"], nas.get("storage_ip", ""))
    elif args.action == "stop":
        if nas["name"] in defined and manage.virsh("domstate", nas["name"]).stdout.strip() == "running":
            manage.virsh("destroy", nas["name"])
    elif args.action == "undefine":
        if nas["name"] in defined:
            if manage.virsh("domstate", nas["name"]).stdout.strip() == "running":
                manage.virsh("destroy", nas["name"])
            manage.virsh("undefine", nas["name"])
    elif args.action == "status":
        state = manage.virsh("domstate", nas["name"]).stdout.strip() if nas["name"] in defined else "undefined"
        print(nas["name"], nas["ip"], nas.get("storage_ip", ""), state)
        log = directory / f"{nas['name']}-serial.log"
        if log.exists():
            lines = [l for l in log.read_text(errors="replace").splitlines() if "lab-nas:" in l]
            for line in lines[-8:]:
                print("  ", line.strip())


def add_commands(commands):
    storage = commands.add_parser("storage-network", help="Define an isolated storage network and add virtio NICs to every lab host")
    storage.add_argument("--directory", required=True)
    storage.add_argument("--subnet", type=manage.private_network, default="192.168.253.0/24")
    storage.add_argument("--nics", type=int, default=2)
    for name, helptext in (("nas-image", "Build the Alpine NFS appliance ISO from the Alpine virt ISO and a directory of its packages"),
                           ("guest-image", "Build the tools-equipped Alpine guest image from the Alpine virt ISO and the xe-guest-utilities package")):
        image = commands.add_parser(name, help=helptext)
        image.add_argument("--directory", required=True)
        image.add_argument("--alpine-iso", required=True)
        image.add_argument("--sha256", required=True, help="Trusted Alpine ISO checksum from the publisher")
        image.add_argument("--apks", required=True, help="Directory of .apk files downloaded on the workstation")
    appliance = commands.add_parser("nas", help="Start, stop, inspect or undefine the NFS appliance")
    appliance.add_argument("action", choices=["start", "stop", "status", "undefine"])
    appliance.add_argument("--directory", required=True)


def run(args):
    if args.command == "storage-network":
        storage_network(args)
    elif args.command == "nas-image":
        nas_image(args)
    elif args.command == "guest-image":
        guest_image(args)
    elif args.command == "nas":
        nas(args)
