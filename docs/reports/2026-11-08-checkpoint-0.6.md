# Version 0.6.0 development checkpoint

This checkpoint adds reviewed [host networking](2026-11-08-network-administration.md),
[storage provisioning and attachments](2026-11-08-storage-lifecycle.md),
[same-pool VM migration](2026-11-08-migration.md),
[pool activity and alert management](2026-11-08-activity.md), and
[live VM resource settings](2026-11-08-vm-resource-settings.md).
The [202-row ledger](../PARITY-IMPLEMENTATION.md) remains the completion authority.

`./scripts/check.sh` passed: 335 workspace tests, 46 vendored console protocol tests,
eight lab preparation tests, 26 synthetic Rune renders, formatting and strict Clippy.
These fixtures exercise stale reviews, response loss, partial outcomes and read-only
reconciliation. Failed or cancelled storage/network creation retains an unknown outcome
because partial resources may remain. Render checks do not establish native input,
clipboard, accessibility or cross-distribution qualification.

`./scripts/package.sh` built the local portable archive
`xenadmin-rs-0.6.0-linux-x86_64.tar.gz`, SHA-256
`68e8b80a4587d7282ba3a17bf59904d1e720c919b2be4c174912a0f603b16707`.
It includes dependency and font notices. Packaging did not restart the existing client.

Live infrastructure mutations remain deferred at the user's request until virtualization
is available for the prepared disposable local lab. All test inventory and generated
screenshots are synthetic; private user captures are excluded. Host/pool administration,
HA/security, authenticated file transfers and organization are in active implementation.
Other retained variants and platform gates remain open in the ledger; this is not a
feature-parity or production-qualification claim.
