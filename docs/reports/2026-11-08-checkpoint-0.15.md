# 0.15.0 development checkpoint

Local portable and AppImage packages freeze source at `f0dc41b`. The retained parity scope remains in progress. Live infrastructure mutation tests remain deferred to the prepared disposable lab until virtualization is available.

## Added since 0.14

- Reviewed host data-source recording controls and explicit recorded-history deletion semantics.
- Same-pool bulk VM moves with whole-selection preflight, per-VM host/disk/network mapping, durable child ownership and no automatic continuation after interruption.
- Dedicated snapshot, VM lifecycle, migration and resource workflow permission reviews. Known later write denials prevent the first workflow change; unchanged fields do not invent permission requirements, and HTTP or other unavailable authority remains unknown.
- Host-scoped GRE/VXLAN tunnel and SR-IOV creation/removal, optional controller/PCI inventory, explicit pool coverage, exact created-object/PIF evidence and no-replay recovery.

The [parity ledger](../PARITY-IMPLEMENTATION.md) and linked increment reports retain the exact supported scope and outstanding work. Bulk archive export, cross-pool batches, local disk conversion, coordinated pool dissolution, pool-wide fabric orchestration and further advanced network/transfer features are outside this freeze.

## Verification

The combined workspace passed **1,183 tests**: 522 core unit, 323 direct protocol, 325 Rune, five profile, four TLS and four probe cases. Strict workspace/all-target Clippy, formatting and whitespace checks passed. The locked release packaging build and all 14 packaging tests passed.

Portable and extracted AppImage installers ran successively in an isolated prefix containing a space. Both SHA-256 checks and probe launchers passed; two releases remained installed, and uninstall preserved a state sentinel. Packaged compact bulk-move, wide permission-review and compact SR-IOV dialogs rendered successfully and were visually inspected using synthetic data.

No live host, user's installation or private screenshot was changed or published. Fixture and headless native rendering checks do not establish live device/data integrity, real compositor interaction, accessibility, multi-monitor behavior or clean-distribution compatibility.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.15.0-linux-x86_64.tar.gz`: SHA-256 `f56ac691430ad7b04c702064c47bca41b27a4315305b3a1586aa1414c695198b`.
- `artifacts/xenadmin-rs-0.15.0-linux-x86_64.AppImage`: SHA-256 `215a93224c65d7cd47d0ffe7d2b5d7c7eee806f4c27b5e78f99859a8004012f0`.

Packages remain local. No release, tag or binary upload was created. Subsequent development uses version 0.16.0 so these verified artifacts remain immutable.
