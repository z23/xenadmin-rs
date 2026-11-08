# Version 0.5.0 development checkpoint

This is an implementation checkpoint, not completion of the retained parity plan or live
qualification. The [202-row ledger](../PARITY-IMPLEMENTATION.md) remains the scope authority.

## Included work

- [Bulk connections and protected startup restoration](2026-11-08-connection-restore.md).
- [Disk and NIC lifecycle operations](2026-11-08-device-lifecycle.md).
- [Pool, host, network and storage administration foundation](2026-11-08-resource-administration.md).
- [Native CA selection and private console PNG export](2026-11-08-desktop-files.md).
- [VM creation, clone/copy, template editing and reviewed deletion](2026-11-08-vm-lifecycle.md).
- [Direct host and VM performance charts](2026-11-08-performance-metrics.md).

## Verification

`./scripts/check.sh` passed: 254 workspace tests, 46 vendored VNC tests, eight lab preparation
tests, 18 synthetic UI renders, workspace formatting and strict Clippy. The protocol tests
exercise identity checks, conflicting operations, lost replies, reconciliation and bounds.
Headless render checks do not establish native keyboard, clipboard, accessibility or all
Wayland/X11 behavior. All fixtures and rendered inventory are synthetic.

`./scripts/package.sh` built `xenadmin-rs-0.5.0-linux-x86_64.tar.gz` with SHA-256
`ffa0863bce95d91200c15374418627e6e660724c0090b97f2a9152dc70ab2216`.
The build uses the pinned lockfile and includes dependency/font notices.

## Remaining qualification and scope

No live infrastructure mutations were used to qualify these increments. The user chose
local disposable-lab preparation and deferred live tests until virtualization is available.
The offline three-host lab draft is ready; no lab machines or virtual networks have been
created. Follow [lab preparation](../../lab/README.md) when KVM/tooling become available.

Migration, broader storage/network management and task/alert administration are in active
implementation. Further retained rows include host/pool administration, security/HA,
transfers, organization, advanced VM settings and desktop qualification. Individual slice
reports distinguish implemented behavior from remaining variants. The existing running
client was not restarted as part of packaging this checkpoint.
