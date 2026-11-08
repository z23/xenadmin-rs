# Feature scope

The row-complete [retained parity ledger](PARITY-IMPLEMENTATION.md) defines the expanded
scope and records each workflow separately. It excludes only Specialized hardware and
guests, and DR, updates and legacy services. The table below describes the original
vertical slices; it is not an additional scope restriction. The [alpha report](reports/2026-11-07-alpha.md)
records the initial implemented subset and its limitations; the
[snapshot report](reports/2026-11-07-snapshots.md) records the 0.2.0 extension. No complete milestone has passed its
live-host acceptance gate. A milestone becomes complete only when its acceptance evidence
is recorded in [verification](VERIFICATION.md).
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
| Bulk transfers | M6 | Streamed XVA, raw/VHD disk and ordinary OVF/OVA transfers with cancellation |
| Cross-pool migration | M6 | Qualified version/storage/network combinations, progress and partial-failure handling |
| Expanded compatibility | M6 | Each additional host version, distribution, or architecture has recorded evidence |

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
Guest RDP and SSH integrations can be added later without becoming management dependencies.

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
repositories. Local EXT/LVM device creation and additional backend wizards remain retained.
