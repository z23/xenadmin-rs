# Captured general information and NIC link details

VM and host Details now open a General information dialog. Pool administration exposes
the corresponding pool summary. Each dialog captures the exact object and pool together
with its inventory revision and freshness. Selection or pool changes do not retarget it;
Refresh from client only replaces the observation from the same pool. It does not issue a
second host login or claim that refreshing stale client inventory made it current.

VM information separates configured memory/CPU limits from reported actual memory and
current vCPU count. It includes runtime flags/domain type, install/boot/metrics timestamps,
boot policy, home-host affinity, autostart/HA and guest tools/OS/addresses. Elapsed time is
explicitly measured from the reported last boot to the client's captured clock, includes
paused time and is not guest-measured uptime. Missing, invalid, epoch-sentinel and future
boot times produce an unknown duration. Halted/suspended VMs do not show running elapsed time.

Host information adds BIOS/chipset values, authentication service and boot/toolstack ages
to CPU/software/memory details. Pool information covers member identities, readable counts,
default/heartbeat storage, HA tolerance/plan/overcommit and the existing pool policies.
It deliberately does not expose raw configuration maps, credentials or license internals.

The optional `PIF_metrics` class projects link carrier, Mbit/s speed, duplex, vendor/device
and PCI bus path. PIF capabilities distinguish missing/invalid from an explicitly empty
advertised set, with bounded entries. Host/network Details show these values independently
of attachment state. Unknown or invalid metrics never imply link health or poison the
ordinary PIF/VM inventory.

## Verification

- **704 workspace tests** passed on this increment's base (325 core, 189 direct protocol,
  177 desktop, five profile, four TLS and four probe tests); strict workspace/all-target
  Clippy, formatting and native desktop build passed.
- New fixtures cover link-metrics association/update, speed sentinel/invalid class isolation,
  missing versus empty capabilities, runtime field projection and elapsed-time boundaries.
- Rune tests cover captured identity across selection/pool changes, same-pool refresh,
  pool/host entry points and wide/compact dialogs. Synthetic 1440×900 and 513×652 native
  renders were inspected; headers/actions stay visible while detailed fields scroll.

Behavioral fields were checked against the pinned XAPI v26.1.21
[PIF/VM metrics schema](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/datamodel.ml)
and [host schema](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/datamodel_host.ml).
No source was copied. No live host was contacted and no private inventory or user image was
published. These observations still depend on the host's field/event coverage; current
connection state is not a guarantee that each guest agent or metrics field is recent.
