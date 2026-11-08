# 0.10.0 development checkpoint

This local Linux checkpoint includes source through `8a58220`. It advances retained parity; it does not claim complete XenAdmin parity or live-host qualification. The disposable lab remains prepared offline, with live tests deferred until virtualization is available.

## Added since 0.9.1

- Captured VM/host/pool general information and separately negotiated physical-link metrics/capabilities.
- Reviewed host IPMI/Wake-on-LAN/disabled power configuration, with transient secrets and ownership-aware cleanup.
- Effective session permissions, ordinary RPC denial guards and upfront required-write checks for covered property/device/host/storage/network/pool reviews.
- Reviewed all-host VLAN creation and retagging of unused plain VLAN networks, including tag 0, exact topology and unplugged-state preservation.
- Pool root-password changes with verified coordinator login, generation-bound reconnect credential replacement and explicit optional update of the unchanged saved profile. Other members' password synchronization remains explicitly unverified.
- Snapshot ancestry/current-base display, disk/storage/checkpoint allocation inspection and bounded authenticated saved-image preview loading.
- Export publication fencing against recovery, acknowledgement, cancellation and changed local file identity.

Each feature's report records its limits. Management-interface handover, guest-tools media mounting, host backup/support/staged restore and VM recovery controls under development after this checkpoint are not included. Snapshot preview capture/upload, broader storage/migration/bulk workflows, desktop extensions/localization and other ledger rows remain open.

## Verification

The integrated workspace passed 789 tests (350 core unit, 223 direct-protocol, 203 Rune, 5 profile, 4 TLS and 4 probe), followed by strict workspace/all-target Clippy. Formatting and diff checks passed. The release build caught an unconditional fixture-scene registration whose function had been gated to debug builds; the two inconsistent gates were removed, and the release workspace then built successfully.

The reviewed source increments have synthetic wide/compact native renders recorded in their reports. The packaged portable client additionally rendered the compact connection dialog, and the AppImage-installed client rendered the synthetic root-password review at 1440×900. Both were inspected. No private user screenshot, inventory or credentials were used or published.

All 14 packaging Python tests passed. Archive and AppImage checksums were verified; both were installed successively into a temporary prefix containing a space. The installed helper resolved that prefix, retained both verified releases during upgrade, and removed only owned files on uninstall while preserving a private test-state sentinel. The packaged probe's help command succeeded. An attempted unsupported desktop `--version` flag launched the isolated empty client; that exact temporary process was closed, and the smoke resumed using the installer manifest version and supported rendering options. No user installation or host connection was changed.

These are fixture and local native checks. Actual guest data integrity, host/storage/network compatibility, failure recovery, XO-down acceptance, clean Ubuntu/Arch installation and real input/accessibility matrices remain qualification work.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.10.0-linux-x86_64.tar.gz`
  SHA-256 `fb784f2c965a048566ad4ca8aaf44aa93751da505e68de765e376ef3f4e59d2c`
- `artifacts/xenadmin-rs-0.10.0-linux-x86_64.AppImage`
  SHA-256 `2876b1b4dec99fde62cf3d6e93bb35b55ac0a8ca6f9e922b8db4ee87fd3ef4b1`

These files were built locally. No GitHub release, tag or binary upload was created.
