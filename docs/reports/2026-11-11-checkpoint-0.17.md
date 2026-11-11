# 0.17.0 development checkpoint

Local portable and AppImage packages freeze source at
`90e6355799e1a13b2b1fc8e99c8f4fbdaca5935a`. This is the first checkpoint with live-host
evidence: every earlier one rested on fixtures alone. No release, tag or binary upload was
created. The retained parity scope remains in progress, and the
[parity ledger](../PARITY-IMPLEMENTATION.md) remains the completion authority.

## Added since 0.16

The 114 commits since the 0.16 freeze are almost entirely qualification work and the fixes
it produced.

- A live harness (`crates/xenadmin-core/tests/lab_live.rs`, 87 ignored scenarios and
  utilities) that drives the core facade against the disposable nested XCP-ng 8.3.0 lab,
  and lab appliances built offline by the lab tooling: an NFS, iSCSI and SMB appliance, a
  tools-equipped guest image and a Samba domain controller.
- Fixes for the host behaviours the live runs exposed, each recorded as a numbered finding
  in the [live qualification report](2026-11-09-lab-qualification.md): unimplemented
  method catalogues, late liveness after a join, allowed operations that halted guests and
  disks never advertise, same-pool migrations refusing any VIF map, non-finite numbers in
  JSON-RPC replies, start markers that arrive after a restart call returns, source cleanup
  that lags a cross-pool move, hard links in the host backup, a snapshot schedule run with
  no asynchronous form, ISO library images typed as user disks, sparse appliance exports
  and an upload reply wait that outlived its idle limit.
- Client changes that came from the same runs: pinned pool members remembered as failover
  endpoints, HA refused when its heartbeat storage interface is not protected from unplug,
  a reviewed forget of a stale interface record, storage refusals named as storage
  prerequisites, local permission refusals worded as local, and refusals that name a
  missing host advertisement instead of a stale editor.
- A shared deadline scale for the protocol fixtures on slow runners, and cached, verified
  AppImage inputs in CI.

## Verification

[CI for this commit](https://github.com/z23/xenadmin-rs/actions/runs/42837355297) passed
formatting, Clippy with warnings denied and **1,255 workspace tests**: 562 core unit (five
native conversion suites ignored), 342 direct protocol, 338 Rune, five profile, four TLS
and four probe cases. The 46 vendored RFB tests, 22 lab tooling tests and 15 packaging tests
passed, the headless scenes rendered, and the job built the archive and AppImage.

The packages below were built on the development workstation from a pristine checkout of
that commit. Both SHA-256 files verified. The archive's installer and the AppImage's
`--install` ran in turn into one isolated prefix whose path contains a space, leaving two
retained releases; the installed probe printed its help, the installed desktop and the
AppImage each rendered the synthetic demo scene headlessly, and uninstall removed the owned
files and kept a state sentinel.

Live evidence, all on XCP-ng 8.3.0 in the disposable lab, is in the
[live qualification report](2026-11-09-lab-qualification.md) and per acceptance row in
[lab/ACCEPTANCE.md](../../lab/ACCEPTANCE.md). The AppImage itself then ran the recovery
gate in a nested compositor: cold start on an empty state directory, first-time certificate
review of a member and the coordinator, a session-only login, the Xen Orchestra VM found by
its UUID with XO shut down, started by a pointer click and consoled with keyboard input, and
connections observed to the two pool hosts only. The same run verified pointer position and
buttons from inside a guest, the release of a held key on focus loss, the console following
a guest that another client migrated, the client as an X11 application under Xwayland, and
saved credentials against a real GNOME keyring on a private session bus. The
[verification traceability report](2026-11-11-verification-traceability.md) maps the core
verification scenarios to this evidence.

## Known defects in this build

The packaged desktop run found three defects in 0.17.0. They are fixed in the 0.18.0
development tree and rechecked on the live desktop (findings 42 to 44 of the live report).

- The Clear button of the inventory search and of the two Activity filters does nothing.
  Select the text and delete it instead.
- The console's Actual size view shows an empty panel. Use the default fitted view.
- After the window loses focus while a key is held in a console, the guest can receive one
  extra press and release of that key. The key is not left held.

The package's README is the running guide as of this commit, which still says that no real
XCP-ng host has qualified the build; the live report supersedes that sentence.

## Limits

Everything live is XCP-ng 8.3.0 on nested virtualization. The desktop evidence comes from a
nested compositor with injected keyboard and pointer input on the development workstation:
it does not establish a real seat, a standalone X server with a window manager, clipboard
exchange with a guest, Orca, non-US layouts, software rendering or clean-distribution
installation. Most workflows have a recorded successful path and no recorded failure or
interruption path. No production host, user installation or private inventory was touched,
and no credential, session or unredacted capture is in the repository.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.17.0-linux-x86_64.tar.gz`: SHA-256 `b4ac38b11bd6647b0eac26fd1febdad60b07c759eb04ec6a80d0d94e6fd2d935`.
- `artifacts/xenadmin-rs-0.17.0-linux-x86_64.AppImage`: SHA-256 `78bf1a572d8af56f75957234c02fe0443c52b9538bbcee61c3a5717e28afbf0c`.

Packages remain local. Subsequent development uses version 0.18.0 so these verified
artifacts remain immutable.
