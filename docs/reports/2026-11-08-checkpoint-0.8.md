# Version 0.8.0 development checkpoint

This checkpoint adds [standalone host joining](2026-11-08-pool-join.md),
[snapshot metadata and restore orchestration](2026-11-08-snapshot-workflows.md),
[host-side snapshot schedules](2026-11-08-snapshot-schedules.md),
[home-host affinity](2026-11-08-home-host.md),
[disk and NIC properties](2026-11-08-device-properties.md),
[OVF/OVA transfers](2026-11-08-ovf-ova-transfers.md),
[cross-pool saved searches](2026-11-08-cross-pool-search.md), and guided NFS/iSCSI
discovery with custom iSCSI ports. Session-only reconnect-as is included.

[Concurrent-operation recovery fixes](2026-11-08-concurrent-operation-recovery.md)
keep stopped workers unknown and retain late task receipts without continuing
pending changes. The host-specific follow-up exercises synchronous and asynchronous
late replies after recovery and acknowledgement. Release compilation also corrected
a debug-only join scene reference; that scene remains excluded from release builds.

`./scripts/check.sh` passed: **585 workspace tests**, **46 vendored console tests**,
**8 local lab preparation tests**, **58 synthetic Rune renders**, formatting and
strict workspace/all-target Clippy. The portable release build completed, and the
archive includes dependency/font notices. No private screenshots or inventory were
used in the repository, fixtures or package.

Package: `artifacts/xenadmin-rs-0.8.0-linux-x86_64.tar.gz`.
SHA-256: `eb7dac83e9d6239a2604beff0e442aabec9b0a4d8c4f40f9f6c524a5a6865cb9`.

This is a development checkpoint. The [retained parity ledger](../PARITY-IMPLEMENTATION.md)
still has outstanding workflows. In particular, batch controls, firmware/template
extensions, cloud-init media, authenticated storage, broader network configuration,
cross-pool movement/copy, conversion and desktop qualification are not completed
by this archive. OVF/OVA supports the documented ordinary-HVM raw/VHD subset;
VMDK conversion and advanced appliance settings remain open.

Live infrastructure-changing tests are deferred at the user's request until the
prepared disposable lab has virtualization. Native portal, keyboard, accessibility,
guest behavior and clean-distribution compatibility remain unqualified. Packaging
did not restart a client. A local process/socket check found no running development
client during this checkpoint; read-only inspection remains available through
`scripts/dev.sh` and `scripts/inspect.sh` when a client is running.
