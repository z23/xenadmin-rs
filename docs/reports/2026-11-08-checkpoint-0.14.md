# 0.14.0 development checkpoint

Local portable and AppImage packages freeze source at `a27aae0`. The retained parity scope
remains in progress. The prepared disposable lab will be used when virtualization is available;
all checks in this checkpoint used synthetic fixtures or isolated local installations.

## Added since 0.13

- Reviewed bulk VM/custom-template properties and deletion, with per-target outcomes, durable
  child ownership and interruption that does not start remaining targets.
- HA failure tolerance, overcommit and timeout policies with positive setter receipts.
- Shared-storage missing-PBD reconstruction and reviewed multipath maintenance/restoration,
  preserving credential ownership and exact attachment identities.
- Staged single-address static IPv6 management bond/VLAN migration through a separately
  authenticated temporary route, with retained reservations between reviewed stages.
- Contextual error guidance that distinguishes failed, partial and unknown outcomes.
- Reviewed NBD TLS/plaintext service policy with exact interface impact and pool-wide exclusion.
- gzip/zstd XVA import and export, exact output-format verification, bounded decoding and
  checkpoint-memory provenance without automatic guest resume.

The [parity ledger](../PARITY-IMPLEMENTATION.md) and linked increment reports specify supported
shapes, exclusions, recovery rules and unfinished implementation. Bulk move/export, tunnel and
SR-IOV lifecycle, remaining advanced transfer/network workflows and other retained gaps are
outside this source freeze.

## Verification

The integrated workspace passed **1,138 tests**: 505 core unit, 304 direct protocol, 316 Rune,
5 profile, 4 TLS and 4 probe cases. Strict workspace/all-target Clippy, formatting and diff
checks passed. The locked release packaging build and all 14 packaging tests passed.

Earlier integration checkpoints passed 1,107 tests for the combined storage/IPv6/error changes
and 1,120 after bulk deletion. The storage-auth observer-order fixture was corrected using an
explicit worker completion fence; the original broad-run timeout was not reproduced, and no
production behavior was changed to conceal a test failure.

Portable and extracted AppImage installers ran successively in an isolated prefix containing a
space. SHA-256 verification and both probe launchers passed; two releases remained installed,
and uninstall preserved a state sentinel. Packaged compact bulk deletion, wide compressed XVA
export and compact network service review rendered successfully and were visually inspected.
One smoke-script attempt used an incorrect scene name and exited with status 2; the corrected
script then passed in a fresh prefix.

No live host, user installation or private screenshot was changed or published. These checks
do not establish real guest data integrity, compositor interaction, accessibility, multi-monitor
behavior or clean-distribution compatibility.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.14.0-linux-x86_64.tar.gz`: SHA-256 `f3d9d500f4914731cd994911d5eb8e7233bddc2468166bcbcbad85fd191e8909`.
- `artifacts/xenadmin-rs-0.14.0-linux-x86_64.AppImage`: SHA-256 `a28ce89ee6cbe7a6649d815ab01c1ef434da3272d64942482d6a7a33d87ee5c5`.

The packages remain local. No release, tag or binary upload was created.
