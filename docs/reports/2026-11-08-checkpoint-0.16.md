# 0.16.0 development checkpoint

Local portable and AppImage packages freeze source at `b28a37a`. The retained parity scope remains in progress. Infrastructure mutation tests remain deferred to the prepared disposable lab until virtualization is available.

## Added since 0.15

- Reviewed bulk VM archive export with whole-selection preflight, immutable destinations, serial transfers and durable child ownership.
- Sandboxed local disk conversion to raw or dynamic VHD, immutable staged inputs, complete content comparison and atomic publication without overwriting existing files. Conversion tools are discovered separately and are not bundled.
- Coordinated pool dissolution with exact member review, whole-pool prerequisites, serial ejection and read-only recovery of interrupted work.
- Pool-wide tunnel and SR-IOV coverage orchestration, preserving existing members and recording exact created-object and interface evidence.
- Queued VM action rechecks during worker teardown, closing a race that could silently discard an immediately requested recheck.

The [parity ledger](../PARITY-IMPLEMENTATION.md) and linked increment reports retain the supported scope and outstanding work. Cross-pool bulk moves, conversion backing chains, further storage backends and advanced networking remain outside this freeze.

## Verification

The combined workspace passed **1,229 tests**: 545 core unit, 333 direct protocol, 338 Rune, five profile, four TLS and four probe cases. Five native conversion suites are ignored in the default run; all nine targeted conversion suites, including those five, passed explicitly using the locally built QEMU 10.1.2 tool. Strict workspace/all-target Clippy passed. The final documentation/UI wording update compiled in the locked release packaging build. All 14 packaging tests passed.

Portable and extracted AppImage installers ran successively in an isolated prefix containing a space. Both SHA-256 checks and probe launchers passed; two releases remained installed, and uninstall preserved a state sentinel. Packaged compact bulk-export, wide conversion-review, compact pool-fabric and compact pool-dissolution dialogs rendered successfully and were visually inspected using synthetic data.

No live host, user's installation or private screenshot was changed or published. Fixture and headless native rendering checks do not establish live device/data integrity, real compositor interaction, accessibility, multi-monitor behavior or clean-distribution compatibility.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.16.0-linux-x86_64.tar.gz`: SHA-256 `b94f83e71124837354d7ef0334bb8fe4b694ec336e4dc265d038e16086e17b6f`.
- `artifacts/xenadmin-rs-0.16.0-linux-x86_64.AppImage`: SHA-256 `308cc1671e21bb49701c46a797f49473b5039a00ca487c76d7d77ba1b089a741`.

Packages remain local. No release, tag or binary upload was created. Subsequent development uses version 0.17.0 so these verified artifacts remain immutable.
