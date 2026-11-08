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
- QEMU, libvirt's system daemon, `virsh`, `qemu-img`, `ip`, and optionally `virt-viewer`.
  The operator must already have permission to manage system libvirt resources.
- At least 16 GiB available RAM and eight schedulable vCPUs for the default two-host lab;
  each host gets 8 GiB, four vCPUs, a sparse 64 GiB system disk and a 32 GiB data disk.
- A locally downloaded XCP-ng installer ISO and a SHA-256 digest independently verified
  against the publisher. Use an XCP-ng release supported by the intended compatibility matrix.
- A libvirt-accessible storage directory outside the source tree. Parent directory
  traversal and SELinux/AppArmor storage policy must permit QEMU access. Disk images
  start mode 0600; libvirt applies its normal DAC/security driver policy.
- A dedicated unused private /24. `prepare` and `start` reject overlaps with workstation
  routes or other libvirt networks. Nothing enables NAT, bridging to a physical NIC,
  guest internet access, autostart, or storage forwarding.

## Prepare and install

Before virtualization or the installer is available, prepare an offline handoff:

```sh
python3 lab/manage.py draft --directory ~/src/xenadmin-rs-lab-prep
```

This records a three-host resource plan, current prerequisite availability, a candidate
isolated subnet and the [acceptance scenarios](ACCEPTANCE.md). It creates no hypervisor
resources and cannot be passed to lifecycle commands as an active manifest. Live tests
remain pending until virtualization and a verified installer are available.

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
