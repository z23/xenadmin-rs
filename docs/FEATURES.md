# Feature scope

The row-complete [retained parity ledger](PARITY-IMPLEMENTATION.md) defines the expanded
scope and records each workflow separately. It excludes only Specialized hardware and
guests, and DR, updates and legacy services. The table below describes the original
vertical slices; it is not an additional scope restriction. The [alpha report](reports/2026-11-07-alpha.md)
records the initial implemented subset and its limitations; the
[snapshot report](reports/2026-11-07-snapshots.md) records the 0.2.0 extension. No milestone
is complete. M2's outage workflow has been run on the disposable XCP-ng 8.3.0 lab through
the core facade and through the packaged desktop client
([live report](reports/2026-11-09-lab-qualification.md)), with its remaining gaps listed in
the [verification traceability report](reports/2026-11-11-verification-traceability.md).
A milestone becomes complete only when the acceptance evidence
[verification](VERIFICATION.md) requires is recorded.
Milestone order and dependencies are defined in the [implementation plan](PLAN.md).

| Workflow | Milestone | Required behavior |
| --- | --- | --- |
| Desktop and protocol qualification | M0 | Real Rune window, direct XAPI probe, interactive console, dependency pins |
| Connection profiles | M1 | Multiple endpoints, credential lookup, session-only credentials, edit and remove |
| TLS trust | M1 | System or custom CA verification, explicit per-endpoint trust, changed-certificate handling |
| Pool discovery | M1 | Connect through a member, discover coordinator, remember verified members |
| Live inventory | M1 | Pools, hosts, VMs, templates, snapshots, storage, networks, disks, and interfaces |
| Inventory navigation | M1 | Tree, search, sort, multiselect, properties, stable selection |
| Connection recovery | M1 | Session renewal, stale-state display, coordinator changes, full resynchronization |
| VM power | M2 | Start, clean shutdown, reboot, explicit force shutdown/reset, eligible host choice |
| Operations | M2 | Progress, errors, partial results, unknown outcomes, task reconciliation, cancellation status |
| VM console | M2 | Direct authenticated RFB tunnel, resize, keyboard, pointer, reconnect, controlled clipboard |
| Outage recovery | M2 | Locate and start the XO VM and use its console while XO is unreachable |
| Metrics | M3 | Bounded CPU, memory, disk, and network chart history from RRD endpoints |
| Activity | M3 | Pool tasks and reviewed cancellation, messages and reviewed dismissal, searchable local history, terminal-only hide/restore and private JSON export; [fixture evidence](reports/2026-11-08-activity.md) |
| Desktop integration | M3 | Wayland, X11, HiDPI, keyboard access, Orca, file picker, installed desktop entry |
| Release artifacts | M3 | Clean-system installation and offline startup with documented runtime dependencies |
| VM creation | M4 | Template, CPU, memory, disks, NICs, ISO, destination, preview, partial cleanup |
| VM properties | M4 | Rename, CPU and memory settings with power-state and capability checks |
| VM devices | M4 | Attach, detach, and configure disks and NICs within supported host capabilities |
| Snapshots and clones | M4 | Create, list, restore, clone, and delete with explicit affected-resource review |
| VM deletion | M4 | Distinguish removing a VM record from deleting its disks and shared resources |
| Same-pool migration | M5 | Eligible targets, storage/network checks, progress, endpoint and console relocation |
| Host operations | M5 | Maintenance, evacuation, shutdown, reboot, logs and relevant host properties |
| Storage administration | M5 | Attach, detach, rescan, reviewed multi-host attachment repair, read-cache policy and provisioning-capability inspection |
| Network administration | M5 | Create networks and supported bonds/VLANs, inspect physical and virtual interfaces |
| Local disk conversion | M6 | Reviewed offline self-contained images to raw/VHD, mandatory sandbox, private output and separate import review; [evidence](reports/2026-11-08-local-conversion.md) |
| Bulk transfers | M6 | Streamed XVA including explicitly reviewed checkpoint memory, raw/VHD disk and ordinary OVF/OVA transfers with cancellation |
| Cross-pool migration | M6 | Qualified version/storage/network combinations, progress and partial-failure handling |
| Expanded compatibility | M6 | Each additional host version, distribution, or architecture has recorded evidence |

## Supported and experimental

Decided by the operator on 2026-11-11, for XCP-ng 8.3 with the client running as a Wayland
application.

Supported means that a workflow has recorded live evidence of its successful path and of a
failure or interruption path, and that a defect in it holds a release back:

- connection profiles, certificate trust, saved credentials, pool discovery through a
  member, and recovery after a lost session or a lost coordinator;
- the live inventory with its navigation and search;
- VM power actions, singly and as a batch;
- the VM and host console: display, keyboard and pointer;
- snapshots: create, inspect, restore and delete;
- performance charts;
- activity: progress, unknown outcomes, recheck, acknowledgement and history.

Everything else in the [ledger](PARITY-IMPLEMENTATION.md) is experimental. It is
implemented, in most cases with a recorded successful run on the lab, but how it fails is
mostly unrecorded, and it may change or be withdrawn. The desktop shows a notice above
every dialog of an experimental area. Inside the supported areas two things are
experimental as well: clipboard exchange with a guest, which has not worked with the Linux
guests tried and is unverified with a guest whose tools support it, and clones and exports
made from a snapshot.

An X session without a Wayland compositor is experimental and is not being pursued, by
the operator's decision of 2026-11-11. A scripted check under the X.Org server with a
window manager passed for the window, input, clipboard, resizing and closing
([report](reports/2026-11-11-clean-install.md)), but nobody has used the client there and
no console was tried.

Not supported: screen readers. The accessibility tree has known gaps (findings 6 and 22 of
the [live report](reports/2026-11-09-lab-qualification.md)) and no screen reader has been
used with the client. Hosts other than XCP-ng 8.3 are not qualified, and the client says so
when it meets one.

The four product decisions the live report had left open (findings 25, 29, 30 and 32) all
concern experimental areas and stay as they are: no VIF rate limit unless the host
advertises it, an explicit attach after a secondary interface is given an address, a
pool-wide guard on detaching a shared repository, and no clean-up of a heartbeat
repository's leftovers. Each is to be looked at again when its area is proposed for
support.

## Availability boundary

Direct access means the application can operate without an XO service, account, plugin,
database, proxy, or cached XO metadata. It requires reachable XAPI and console endpoints.
The workstation must have an authorized host account. Pool coordination, storage access,
and VM availability remain properties of the XCP-ng installation.

The [connection trust increment](reports/2026-11-08-connection-trust.md) adds credential-free
certificate inspection, exact-endpoint pin acceptance/replacement and Direct, captured
Environment or explicit HTTP CONNECT routes. Proxy authentication stays in the application
session. Certificate review changes an unsaved profile draft and never automatically logs in.
HTTPS/SOCKS/PAC proxy configurations are explicitly unsupported in this increment.

Console sessions can move into up to eight independent native windows while the main window
browses other guests or pools. Each retains its original pool/VM and connection, with separate
input, clipboard, screenshot and reconnect state. The Console windows manager shows or closes
any session; closing the main application closes them all. See the
[independent console windows report](reports/2026-11-08-independent-console-windows.md).

The application does not promise a generic console for guests that expose no RFB console.
Reviewed guest SSH/RDP and configured local tools run independently of XAPI. Choose a reported
or manually verified guest IP and review the exact local executable/arguments. Authentication
happens in the external client; no XAPI password or session is supplied. Local declarations
are trusted programs, not sandboxed plugins. See the
[external guest tools report](reports/2026-11-08-external-guest-tools.md).

The [firmware/template increment](reports/2026-11-08-firmware-templates.md) adds reviewed
BIOS/UEFI/Secure Boot provisioning, eligible chosen-host BIOS identity with explicit
affinity, halted UEFI policy editing and read-only certificate readiness. Installed guest
firmware switches remain refused. Custom-template properties have explicit kind and
built-in-template guards. The [NoCloud increment](reports/2026-11-08-cloud-init.md) adds
private local FAT media, chosen-SR upload and reviewed read-only attachment to halted VMs.
It requires guest cloud-init/NoCloud support and does not claim guest execution.

Host backup and support also provides reviewed retained-crashdump deletion. It captures the
host and exact dump UUID/time/size, requires the dump UUID as confirmation, and checks fresh
ownership and host availability before dispatch. Completion requires a durable positive host
response plus record absence; this is not independent filesystem inspection. Lost or failed
responses remain uncertain and are never replayed by Recheck. Support logs exclude raw memory
dumps, and direct raw-dump download remains unavailable for the target API. See the
[crashdump deletion report](reports/2026-11-08-host-crashdump-deletion.md).

The [HBA/allocation increment](reports/2026-11-08-storage-provisioning.md) adds discovered
HBA/Fibre Channel LUN creation and driver-advertised thick/thin allocation for new iSCSI/HBA
repositories. The [local storage increment](reports/2026-11-08-storage-local.md) adds reviewed
EXT/LVM creation from an operator-declared persistent device path, with a dual repository
probe and explicit data-loss confirmation. It does not claim automatic physical device
discovery or fully verified capacity, signatures and ownership; additional backend wizards
remain retained.

The [shared storage graph increment](reports/2026-11-08-storage-graph-repair.md) adds reviewed missing-PBD reconstruction for supported shared locations, server-side credential cloning without reading values, and complete-host multipath detach/settings/replug orchestration. Known partial results offer separately reviewed reconnection; custom host-dependent reconstruction and live path qualification remain open.

Host monitoring includes a read-only data-source catalog and reviewed recording/history-deletion
controls. Forgetting deletes recorded RRD samples and may be followed immediately by automatic
collection; it is never presented as a harmless pause. Stock XCP-ng NRPE integration is explicitly
unsupported. [Supported semantics and verification](reports/2026-11-08-host-monitoring-recording.md).
The [network fabric increment](reports/2026-11-08-network-fabric-primitives.md) adds reviewed GRE/VXLAN and SR-IOV creation/removal on one host, optional controller/PCI inventory, explicit pool coverage and stable created-object/interface history. Controller activity, reboot requirements and actual traffic/VF qualification remain separate. [Pool-wide coverage operations](reports/2026-11-08-pool-network-fabric.md) preserve existing members, order exact host steps and retain partial results without replay. SR-IOV VLAN/capacity workflows and controller/traffic qualification remain retained.

- Reviewed coordinated pool dissolution: exact member order, disabled/evacuated-host checks, durable ejection receipts and final single-host metadata clearing; live qualification remains open.
