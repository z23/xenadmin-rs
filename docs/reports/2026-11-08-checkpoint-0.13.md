# 0.13.0 development checkpoint

The local portable and AppImage packages freeze source at `d7c5ec3`. The retained parity scope remains in progress, with live qualification deferred until the prepared disposable lab has virtualization available.

## Added since 0.12

- Reviewed HBA and local EXT/LVM provisioning, driver-advertised thick/thin allocation, and explicit destructive device reviews.
- Complete XVA stream/checksum verification before export publication; separate memory-checkpoint export and import that leaves new guests suspended.
- Reviewed guest SSH/RDP and trusted local executable/argument declarations with captured guest targets and local process status.
- Guest NIC impact review and verification during supported management bond changes.
- Whole-row and keyboard inventory context menus with stale-selection guards and keyboard focus return.
- Correct temporary desktop ownership release despite duplicated descriptors, while normal desktop ownership remains retained until process exit.

Individual feature reports and the [parity ledger](../PARITY-IMPLEMENTATION.md) define supported topologies, refusal conditions and remaining work. Bulk VM properties and further storage/network workflows are outside this source freeze.

## Verification

The combined workspace passed 1,044 tests: 461 core unit, 285 direct-protocol, 285 Rune, 5 profile, 4 TLS and 4 probe. Strict workspace/all-target Clippy, formatting and diff checks passed. The locked release packaging build and all 14 packaging tests passed.

Both artifact hashes were checked. Portable and extracted AppImage installers ran successively in an isolated temporary prefix containing a space. Both probe launchers worked, two releases remained available, and uninstall preserved a state sentinel. Packaged local-storage review at 513×652, checkpoint export at 1440×900 and independent console at 513×652 rendered successfully and were visually inspected with synthetic data.

No live host, user installation or private screenshot was changed or published. These checks do not establish real guest data integrity, Wayland/X11 interaction, accessibility, multi-monitor behavior or clean-distribution compatibility.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.13.0-linux-x86_64.tar.gz`: SHA-256 `f598a08cd8a63db1dce6d1afe71c09e712673ba840cbf5c465afee8c54ca60d3`.
- `artifacts/xenadmin-rs-0.13.0-linux-x86_64.AppImage`: SHA-256 `8c911b5a2d0e2742190316eaa085a941117ff15af91f25c6cbde61d2c1ffa14c`.

These artifacts remain local. No release, tag or binary upload was created.
