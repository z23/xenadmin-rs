# Disposable nested XCP-ng acceptance lab

This harness prepares one to three local KVM guests, each with a system disk and a
separate disposable data disk. The guests share a libvirt network with no forwarding.
They can reach each other and the workstation. No existing pool is a test target.

`manage.py plan` checks a caller-supplied installer hash and writes a manifest and
reviewable domain/network XML. It does not download software, create virtual machines,
change host networking or install dependencies. Later lifecycle commands operate only
on names, UUIDs and ownership metadata matching that manifest. Unrelated resources with
matching names are rejected. `undefine` retains disk images and evidence.

## Requirements

- An x86-64 workstation with accessible `/dev/kvm`, Intel VMX or AMD SVM exposed and
  nested virtualization enabled. The harness reports this state and does not change it.
- QEMU, libvirt's system daemon, `virsh`, `qemu-img`, `ip`, and optionally `virt-viewer`;
  unattended installation also needs `xorriso`, `bsdtar` and `openssl`.
  The operator must already have permission to manage system libvirt resources.
- At least 16 GiB available RAM and eight schedulable vCPUs for the default two-host lab;
  each host gets 8 GiB, four vCPUs, a sparse 64 GiB system disk and a 32 GiB data disk.
- A locally downloaded XCP-ng installer ISO and a SHA-256 digest independently verified
  against the publisher. Use an XCP-ng release supported by the intended compatibility matrix.
- A libvirt-accessible storage directory outside the source tree. Parent directory
  traversal and SELinux/AppArmor storage policy must permit QEMU access. Disk images
  start mode 0600; libvirt applies its normal DAC/security driver policy.
- A dedicated unused private /24. `prepare` and `start` reject overlaps with workstation
  routes or other libvirt networks, and `start`/`serve` verify that each lab address
  resolves to the lab bridge once it exists. A VPN whose policy-routing table captures
  all destinations needs an exception such as
  `ip rule add to SUBNET lookup main priority 5000` before the WARP-style rule. Nothing
  enables NAT, bridging to a physical NIC, guest internet access, autostart, or storage
  forwarding.

## Prepare and install

Before virtualization or the installer is available, prepare an offline handoff:

```sh
python3 lab/manage.py draft --directory ~/src/xenadmin-rs-lab-prep
```

This records a three-host resource plan, current prerequisite availability, a candidate
isolated subnet and the [acceptance scenarios](ACCEPTANCE.md). It creates no hypervisor
resources and cannot be passed to lifecycle commands as an active manifest. Live tests
take their targets from a manifest created by the lifecycle commands below; the first
recorded run is in the [live qualification report](../docs/reports/2026-11-09-lab-qualification.md)
and the per-scenario results are in the [acceptance record](ACCEPTANCE.md).

Run `python3 lab/manage.py doctor` first. Replace the following placeholders explicitly:

```sh
python3 lab/manage.py plan \
  --directory /var/lib/libvirt/images/xenadmin-rs-lab \
  --iso /path/to/xcp-ng-installer.iso \
  --sha256 PUBLISHER_VERIFIED_SHA256 \
  --subnet 192.168.254.0/24
python3 lab/manage.py prepare --directory /var/lib/libvirt/images/xenadmin-rs-lab
python3 lab/manage.py start --directory /var/lib/libvirt/images/xenadmin-rs-lab --install
```

Inspect the generated XML before `prepare`. The manifest lists host names and DHCP
reservations. Open each installer with `virt-viewer --connect qemu:///system HOST_NAME`.
Install on the first disk, leaving the second disk disposable for SR tests. Use lab-only
credentials. DHCP reservations assign `.11`, `.12` and `.13`; static addresses matching
those reservations are also suitable. The workstation is `.1` on the isolated subnet.

After installation, shut down the guests, then start without `--install` to boot from
the system disks. Join the second host to the first using XCP-ng's normal pool-join
workflow. This manual installer/pool setup is intentional: no root passwords or pool
join credentials are persisted in the manifest or passed through command-line arguments.

### Unattended installation

The installer can instead answer itself from a per-host XML file served only on the
isolated bridge. After `plan`, run:

```sh
python3 lab/manage.py unattended --directory /var/lib/libvirt/images/xenadmin-rs-lab
```

It prompts for a lab-only root password (or reads `XENADMIN_LAB_ROOT_PASSWORD`), stores
only its SHA-512 crypt hash in `answerfiles/ADDRESS.xml` (mode `0600`), and writes a
`NAME-install.iso` whose only change is a default boot entry carrying
`answerfile=http://GATEWAY:PORT/answerfile.xml install atexit=poweroff`. Each answer file
selects the host's reserved static address by MAC, installs on the first SATA disk with a
local EXT repository and leaves the second disk untouched. The manifest records the
derived image path, digest and URL, never the password. Requires `xorriso`, `bsdtar` and
`openssl`; the original publisher-verified ISO is still checked on every install start.

Serve the answer files from the bridge address, then boot the installers:

```sh
python3 lab/manage.py serve --directory /var/lib/libvirt/images/xenadmin-rs-lab &
python3 lab/manage.py start --directory /var/lib/libvirt/images/xenadmin-rs-lab --install
```

The server answers only the manifest's host addresses. A workstation firewall must admit
TCP on that port from the lab subnet to the bridge address. Each host powers off when its
installation completes; `status` shows every host shut off. Stop the server, then `start`
without `--install`. Installer and Xen console output is appended to `NAME-serial.log` in
the lab directory for review.

```sh
python3 lab/manage.py stop --directory /var/lib/libvirt/images/xenadmin-rs-lab
python3 lab/manage.py status --directory /var/lib/libvirt/images/xenadmin-rs-lab
python3 lab/manage.py start --directory /var/lib/libvirt/images/xenadmin-rs-lab
```

Graceful shutdown is asynchronous; `status` must show every host shut off before `start`
or `undefine`. `stop --force` cuts power only to owned lab guests and is intended for
explicit outage testing. Do not use these guest images for production data.

## Acceptance sequence

Record application commit, installer SHA-256, host/API versions, storage driver, kernel,
libvirt/QEMU versions and the outcome for each scenario. Use synthetic guest names and
content. A completed fixture test is not evidence of a completed live scenario.

1. Connect directly through each member while no XO endpoint is configured or reachable.
   Confirm one pool identity and current inventory, including device relationships.
2. Use a disposable VM to verify start, chosen-host start, pause/unpause, suspend/resume,
   clean/forced shutdown and console input. Compare observed host task/result state.
3. Edit a VM's name/description, CPU, memory and boot settings. Change a property from
   another client before saving and confirm the conflict is reported.
4. Create a marker file in a disposable guest, snapshot it, change the file, restore the
   snapshot, and verify the content. Check interrupted operations and original ownership.
5. Exercise CD/ISO, VBD, VIF and disk growth on disposable devices. Try a shared VDI,
   stale size and foreign VM device; confirm rejection without changing the other VM.
6. Stop the coordinator guest, interrupt a local management session and test certificate
   changes. Record unknown mutation outcomes and avoid replaying a primary write.
7. For every later retained workflow, add a happy path and an interrupted/rejected path
   from [the implementation ledger](../docs/PARITY-IMPLEMENTATION.md). Record unsupported
   host capabilities as unsupported rather than successful or silently absent.
8. Export only sanitized reports. Private screenshots, inventories, passwords, private
   keys, packet captures and guest disk images stay outside the repository.

Recovery utilities in the harness (ignored tests like the scenarios) put the lab back
together through the client after a failed experiment: `lab_forget_dead_coordinator_through_survivor`
(disable HA, delete the guests whose disks were on the dead host, forget it and its
repositories through the surviving coordinator), `lab_designate_first_host_coordinator`,
`lab_reattach_shared_repositories`, `lab_shut_down_shared_storage_guests` (with
`XENADMIN_LAB_SHUTDOWN_ALL=1` for every guest, before a host power cycle) and
`lab_start_lab_guests`. A host that must be reinstalled boots from a definition built from
its current domain with the unattended installer attached, while `manage.py serve` answers
its installer; the join scenario then brings it back.

The harness reads its inputs from the environment: `XENADMIN_LAB_MANIFEST` (the manifest),
`XENADMIN_LAB_PASSWORD_FILE` (the lab-only root password), `XENADMIN_LAB_EVIDENCE_DIR`
(defaults to `evidence/` beside the manifest), `XENADMIN_LAB_HOST_CONTROL=libvirt` (lets a
scenario cut and restore a host's power through libvirt), `XENADMIN_LAB_CHAP_FILE` and
`XENADMIN_LAB_DC_FILE` (the private appliance secrets), `XENADMIN_LAB_GUEST_IMAGE` (the
isohybrid Alpine image the provisioning scenario imports), `XENADMIN_LAB_XOA_XVA` (a downloaded
Xen Orchestra appliance image for the recovery gate), `XENADMIN_LAB_ENDPOINT_INDEX` (sends
the default first-host endpoint to another manifest host, for example the surviving
coordinator while the first host is reinstalled) and `XENADMIN_LAB_JOIN_SOURCE` (the host
the join scenario joins into the default endpoint's pool). Secrets are read from files, never
from the command line, and never recorded.

## Optional appliances

The isolated lab has no internet access and the workstation needs no privileges beyond
libvirt, so the optional pieces below are built on the workstation and carried into the lab
as images. `python3 lab/alpine_closure.py --iso ALPINE.iso --branch v3.24 --out DIR ROOTS...`
downloads the requested Alpine packages and their dependency closure once (skipping what the
ISO already bundles) into a directory outside the source tree; the appliance needs
`nfs-utils nfs-utils-openrc rpcbind rpcbind-openrc e2fsprogs util-linux-misc` and the guest
image needs `xe-guest-utilities`. The Alpine ISO is checked against the publisher's SHA-256
before every build.

- `storage-network --directory DIR --subnet 192.168.253.0/24 --nics 2` defines an isolated
  storage network (no DHCP, no forwarding) and adds virtio NICs to every lab host. One NIC
  hot-plugs into a running host; further NICs wait for the domain's next start because the
  q35 machine has no spare PCIe slots, and the command says so. A reboot of the XCP-ng host
  from inside is not enough: the device appears only after the libvirt domain is powered off
  (for example through the client's reviewed host shutdown) and started again with `virsh
  start`. The MACs are recorded in the manifest so reruns are idempotent.
- `nas-image --directory DIR --alpine-iso ISO --sha256 DIGEST --apks DIR` remasters the Alpine
  `virt` ISO into `nas.iso`: a root overlay (static addresses on both lab networks, exports,
  boot services, a start script) and an `/extra` directory holding `nfs-utils` and its
  dependency closure. At every boot the diskless appliance installs those packages from the
  ISO, formats its data disk on first use, mounts it at `/srv/nfs` and exports `/srv/nfs`,
  `/srv/nfs/sr`, `/srv/nfs/iso` and `/srv/nfs/heartbeat` to both lab subnets. Progress is
  written to the serial console, which libvirt logs next to the host serial logs.
- The same image offers two iSCSI targets through the kernel target when built from the
  Alpine `standard` ISO (its `lts` kernel has the LIO modules; the `virt` kernel does not):
  an open target and a CHAP-protected one, each a 20 GiB sparse file on the data disk, on
  every appliance address. `--chap-file FILE` names a private one-line `user secret` file
  that is baked into the image only; the manifest records the target names, never the
  secret. The package set then also needs `targetcli targetcli-openrc`. Each LUN's backstore
  unit serial is pinned from the manifest (`nas.iscsi.serials`), because the kernel target
  would otherwise generate a new serial, and so a new SCSI identity, at every boot of the
  diskless appliance and strand every repository built on the LUN.
- `--seed-iso FILE` (repeatable) ships ISO images on the appliance ISO; at boot they are
  copied once into `/srv/nfs/iso`, so the library exports and the SMB share below list them.
  The same export is served as the guest-readable SMB 3 share `//APPLIANCE/iso` (no accounts,
  `map to guest`, SMB 3.0 minimum) for the SMB ISO-library rows; the package set then also
  needs `samba-server samba-server-openrc`.
- `nas start|status|stop|undefine --directory DIR` manages the appliance VM (1 GiB, one
  vCPU, a 64 GiB sparse data disk, NICs on the lab and storage networks). `status` prints the
  last appliance log lines.
- `guest-image --directory DIR --alpine-iso ISO --sha256 DIGEST --apks DIR` remasters the same
  ISO into `guest-tools.iso` with `xe-guest-utilities`, a guest-agent OpenRC service and DHCP
  on `eth0`. The image stays isohybrid so the client can import it as a bootable disk; a guest
  built from it reports PV drivers, its address and clean shutdown support to the pool.
  The lab network's DHCP serves guests from `.200` to `.249`; an existing network gets that
  range with `virsh net-update NETWORK add ip-dhcp-range RANGE_XML --live --config`.

- `dc-image --directory DIR --alpine-iso ISO --sha256 DIGEST --apks DIR --dc-file FILE`
  remasters the Alpine `standard` ISO into `dc.iso`, a Samba Active Directory domain
  controller for the directory-service rows: realm `lab.xenadmin.test`, NetBIOS domain
  `XENADMINLAB`, host name `dc` at `.21` on the lab network with Samba's internal DNS. On
  first boot it provisions the domain onto its 8 GiB data disk and creates the test account
  `labuser` in the group `labadmins`; later boots reuse the provisioned domain. The package
  set needs `samba-dc samba-server-openrc samba-client samba-winbind krb5 bind-tools
  e2fsprogs util-linux-misc`. `--dc-file` names a private one-line file holding the
  administrator password and the test user's password (alphanumeric, with upper and lower
  case and a digit for the domain's complexity policy); it is baked into the image only and
  the manifest records the realm and account names, never the passwords. The harness reads
  the same file through `XENADMIN_LAB_DC_FILE`. Pool hosts must use the controller as their
  DNS server to join; the directory scenario sets and restores that through the client.
- `dc start|status|stop|undefine --directory DIR` manages the controller VM (1 GiB, one
  vCPU, one NIC on the lab network).
- `python3 lab/other_admin.py COORDINATOR SHA256_FINGERPRINT ACTION` does what another
  administrator might do to the pool while the client under test is connected, for the
  cases the client cannot produce itself: `logout-subject SUBJECT_IDENTIFIER` closes every
  session of one directory subject (the directory scenario ends the test user's session
  with it), `restart-toolstack HOST_UUID` restarts one host's toolstack (the console
  scenario uses it, because the client refuses that restart while a console is open), and
  `pending-tasks` lists the pool's unfinished tasks without changing anything. It reads the
  root password from `XENADMIN_LAB_PASSWORD_FILE` and sends nothing to a host whose
  certificate does not match the fingerprint. The host never closes the root account's own
  sessions by a subject logout.
- `python3 lab/description_check.py COORDINATOR SHA256_FINGERPRINT [DESCRIPTION_JSON]`
  compares the fields of the pool's live records with the API description the client's
  bindings are generated from (or another description, named as the third argument), and
  prints the fields that hosts send and the description lacks, the described fields no
  record carries, the classes without records and the classes the host refuses. It only
  reads, prints field names and never record contents, and takes the password and the
  fingerprint as `other_admin.py` does. A class without records on the lab cannot be
  compared.
- A guest with a graphical desktop, for pointer checks, is made by the harness scenario
  `lab_desktop_guest_provisioning` from the isohybrid live image named by
  `XENADMIN_LAB_DESKTOP_IMAGE`. The lab uses GParted Live (`gparted-live-1.8.1-6-amd64.iso`,
  688 MiB, from the project's download page, checked against its published SHA-256): it is
  imported through the client as a raw disk, booted in a 1 GiB guest named `lab-desktop-…`
  and reaches its desktop after three Return presses, which the scenario sends.
  `lab_desktop_guest_pointer_press_lands_on_its_target` then presses at the point given in
  `XENADMIN_LAB_POINTER_TARGET=x,y` and keeps the frames before and after.
- The Xen Orchestra appliance for the recovery gate is the vendor's own image (downloaded
  from `https://xoa.io/xva`, 1.9 GB) imported through the client by the gate scenario, named
  `XOA`, sized to 2 GiB and left running on the shared NFS repository. `node lab/xo_connect.mjs
  XOA_ADDRESS COORDINATOR_ADDRESS` then makes it manage the pool over its API: it rotates the
  factory account to the lab account in the private file named by `XENADMIN_LAB_XO_FILE`
  (one line `email password`) on first use, adds the coordinator with the lab root password
  from `XENADMIN_LAB_PASSWORD_FILE`, and waits for the connection. The appliance presents its
  own self-signed certificate, so the script is run with `NODE_TLS_REJECT_UNAUTHORIZED=0` on
  this isolated network only. Nothing in the client depends on XO; this only makes the gate's
  XO VM a VM that XO itself manages.

Apart from the domain controller's lab-only accounts, none of this creates accounts or
credentials: the NFS appliance and the guest have no passwords, and everything is reachable
only on the isolated lab networks.

## Cleanup and local tests

```sh
python3 lab/manage.py stop --directory /var/lib/libvirt/images/xenadmin-rs-lab
python3 lab/manage.py undefine --directory /var/lib/libvirt/images/xenadmin-rs-lab
python3 -m unittest discover -s lab
```

Wait for shutdown before `undefine`. Disk images remain for deliberate manual deletion
or forensics; the harness does not recursively remove a directory.

At implementation time, this environment exposed neither QEMU/libvirt commands nor
`/dev/kvm`. Manifest/XML and ownership tests ran locally; no nested host was installed
or qualified. See [VERIFICATION.md](../docs/VERIFICATION.md) for the full release gate.
