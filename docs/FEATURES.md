# Feature scope

This table defines target workflow coverage. The [alpha report](reports/2026-11-07-alpha.md)
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
| Desktop integration | M3 | Wayland, X11, HiDPI, keyboard access, Orca, file picker, installed desktop entry |
| Release artifacts | M3 | Clean-system installation and offline startup with documented runtime dependencies |
| VM creation | M4 | Template, CPU, memory, disks, NICs, ISO, destination, preview, partial cleanup |
| VM properties | M4 | Rename, CPU and memory settings with power-state and capability checks |
| VM devices | M4 | Attach, detach, and configure disks and NICs within supported host capabilities |
| Snapshots and clones | M4 | Create, list, restore, clone, and delete with explicit affected-resource review |
| VM deletion | M4 | Distinguish removing a VM record from deleting its disks and shared resources |
| Same-pool migration | M5 | Eligible targets, storage/network checks, progress, endpoint and console relocation |
| Host operations | M5 | Maintenance, evacuation, shutdown, reboot, logs and relevant host properties |
| Storage administration | M5 | Attach, detach, rescan, inspect capacity and connectivity |
| Network administration | M5 | Create networks and supported bonds/VLANs, inspect physical and virtual interfaces |
| Bulk transfers | M6 | Streamed XVA import/export and qualified disk transfers with cancellation |
| Cross-pool migration | M6 | Qualified version/storage/network combinations, progress and partial-failure handling |
| Expanded compatibility | M6 | Each additional host version, distribution, or architecture has recorded evidence |

## Availability boundary

Direct access means the application can operate without an XO service, account, plugin,
database, proxy, or cached XO metadata. It requires reachable XAPI and console endpoints.
The workstation must have an authorized host account. Pool coordination, storage access,
and VM availability remain properties of the XCP-ng installation.

The application does not promise a generic console for guests that expose no RFB console.
Guest RDP and SSH integrations can be added later without becoming management dependencies.
