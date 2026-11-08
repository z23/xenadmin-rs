# Device property increment — 8 November 2026

Implemented direct-XAPI disk name/description, access mode, guest device position,
bootable attachment flag, best-effort I/O priority, changed-block tracking disablement,
and NIC rate limiting in the Rune device page. Each editor captures its VM, pool,
connection generation and device UUIDs; review cannot follow a later selection or reconnect.

Disk metadata supports running or halted guests. Other changes require a halted VM
and disconnected device. Disk edits require an ordinary managed, nonmissing,
nonshared disk with exactly the captured attachment. No implicit shutdown, unplug,
slot swap, NIC replacement or disk recreation is performed. Position changes require
an unused host-advertised slot. CBT disablement requires the disk UUID and describes
the incremental-backup impact.

QoS retains unrelated map entries, refuses unknown algorithms, requires advertised
support before enabling a policy, and separates parameter and algorithm steps.
Rate limits use kilobytes per second. Disk priority uses best-effort `ionice`, with
0 highest and 7 lowest, following [XCP-ng storage documentation](https://docs.xcp-ng.org/storage/manage-srs/#disk-io-tuning).
The reference XenAdmin display labels reverse those endpoints; this implementation
uses the documented wire semantics. Actual scheduling effects still require a
supporting storage scheduler and live qualification.

The existing durable device journal records each setter independently. Lost replies
or API errors after dispatch remain unknown; reconciliation only reads the same
objects and does not replay unsent setters. A read-back with the expected QoS map
but a conflicting algorithm cannot be marked successful. Partial metadata/QoS
changes remain visible in operation history. Private plan fields and opaque
references remain excluded from public operation serialization.

## Verification

- Full isolated workspace suite: 453 tests passed (201 core unit, 132 direct
  protocol, 107 desktop, 5 profile, 4 TLS, 4 host-probe).
- Eight new protocol fixtures cover property writes, map preservation, identity,
  running metadata, halted configuration, stale/shared/attached/unsupported/occupied
  refusals, lost replies, partial setter failure, CBT reconciliation and concurrent
  algorithm change. The concurrent algorithm case was added after peer review.
- Three Rune interaction tests cover compact/wide review, captured identities,
  disabled synthetic submission, stale observation, correct units, invalid/no-op
  values and exact CBT confirmation. Rerun after final wording/validation fixes.
- Strict workspace Clippy, formatting and diff checks passed.
- Native Vulkan compact (513×652) and wide (1440×900) synthetic renders inspected.
  The dialog labels, input and action controls are visible. Rune emits an existing
  background device-page scroll sizing warning in the compact scene; this does not
  establish native keyboard, screen reader or clipboard qualification.

No live device writes were performed. Storage/guest behavior, priority effects,
NIC throughput and backup-chain effects remain deferred to the disposable lab.
