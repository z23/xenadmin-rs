# Version 0.7.0 development checkpoint

This checkpoint integrates reviewed [host administration](2026-11-08-host-administration.md),
[pool policies, HA and membership controls](2026-11-08-pool-administration.md),
[directory, role and certificate administration](2026-11-08-pool-security.md),
[XVA transfers](2026-11-08-xva-transfers.md),
[raw/VHD disk transfers](2026-11-08-disk-image-transfers.md),
[tags, folders and custom fields](2026-11-08-organization.md), and
[vApp membership and ordered power controls](2026-11-08-vapps.md).
The [202-row ledger](../PARITY-IMPLEMENTATION.md) remains the completion authority.

The [journal initialization fix](2026-11-08-journal-initialization.md) preserves SQLite's
process locks and avoids redundant WAL setup under concurrent opens. The deterministic lock
fixture and 100 repeated organization fixture groups passed. Public VM operation serialization
now [excludes private reconciliation evidence](2026-11-08-vm-journal-privacy.md), while the
journal preserves backward-compatible recovery data. Development inspection remains explicitly
opt-in, local and read-only, using the client's existing authenticated connections.

`./scripts/check.sh` passed: 463 workspace tests, 46 vendored console protocol tests,
eight lab preparation tests, 40 synthetic Rune renders, formatting and strict Clippy.
New workflows cover stale reviews, partial writes, lost replies, unknown outcomes, read-only
reconciliation and private transient credentials. These results do not establish live XCP-ng
behavior, native keyboard/clipboard/accessibility or clean-distribution compatibility.

Integrated testing also corrected two fixture races: cancellation history is delivered
asynchronously after the operation notification, and privacy checks must query SQLite rather
than opening its live database/WAL files with ordinary file I/O. Neither correction bypasses
an application outcome assertion; the history check retains a bounded deadline, and privacy
checks inspect stored payload and reconciliation-reference columns.

Live infrastructure mutations remain deferred at the user's request until virtualization is
available for the prepared disposable local lab. All generated inventory and screenshots are
synthetic. Private user screenshots and inventories are excluded from the repository and
packages. Pool join/elevation, VMSS schedules, OVF/OVA, remaining device/property workflows,
network/storage variants, search/bulk controls and desktop qualification remain in progress.
This is a development checkpoint, not a feature-parity or production-qualification claim.

`./scripts/package.sh` built `xenadmin-rs-0.7.0-linux-x86_64.tar.gz` locally,
SHA-256 `667ca8f69e6e7fb835cbb433b4300266626ea528cb98f884d17875d93ef7d00e`.
The archive includes dependency/font notices. Packaging did not restart the existing client.
