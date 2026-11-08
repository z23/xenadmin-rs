# 0.3.0: Connection, console and VM administration foundations

This increment implements parts of the approved retained-parity plan. It does not complete
that plan. [The 202-row ledger](../PARITY-IMPLEMENTATION.md) records the remaining scope;
the specialized-hardware/guests and DR/updates/legacy-services categories stay excluded.

## Delivered workflows

- Save, edit and remove multiple connection profiles; keep independent pools connected and
  switch between them. Authenticate aliases before sharing one pool owner. Reject different
  credential/trust/access contexts instead of borrowing an existing session's authority.
- Persist expected pool identity and discovered member candidates for endpoint fallback.
  Expose custom CA, endpoint pins, timeout and read-only settings. Migrate the previous
  single-endpoint settings without saving credentials in the profile file.
- Save/load/forget passwords through optional Secret Service integration, with session-only
  credentials as the default and a manual fallback for locked/unavailable keyrings.
- Enforce one mutating desktop per state directory; a second window cannot modify profiles,
  preferences, keyring entries or operations through the application.
- Decode the VM/device/host/storage/network/guest-metrics/task graph and distinguish missing,
  unsupported, permission-denied and malformed optional data. Inspect VM disks, ISO drives,
  NICs/uplinks, guest tools and guest addresses.
- Explain power availability and request fresh host validity diagnostics. Review chosen-host
  start/resume, with fresh VM/host identity, state and boot-compatibility checks.
- Edit VM name, description, autostart, halted CPU counts, four memory limits and boot order.
  Review old/new values, verify each saved step and retain partial/unknown outcomes. A lost
  response is never handled by silently repeating a setter. Existing unrelated map keys
  are preserved from a fresh read.
- Insert/eject ISO media, activate/deactivate existing disk attachments, connect/disconnect
  existing NICs and grow a halted VM's exclusive writable disk. Re-resolve UUIDs, ownership,
  capabilities and consumers before dispatching exactly one tracked asynchronous task.
- Create host-advertised memory checkpoints as well as disk snapshots. Existing halted
  snapshot restore remains available. Snapshot creation verifies the returned identity.
- Pin console and device-page targets independently of table selection. Add deliberate key
  chords, negotiated QEMU physical keys, middle button/wheel, fit/actual-size, fullscreen,
  cursor rendering, explicit clipboard send/copy and reconnect to the captured VM.
- Add a disposable nested KVM/libvirt harness with hash-checked installer input, owned resource
  identities, isolated networking, collision checks and offline tests. It was not deployed.

The journal migration preserves old task references and unresolved reservations, adds
per-step settings outcomes, and refuses future versions. Regression fixtures cover a setter
that succeeds before verification fails and a later-step preflight failure after a verified
change: the former remains unknown; the latter reports partial completion.

## Verification

`./scripts/check.sh` passed on the development workstation: strict Clippy, the workspace
suite, the vendored RFB wire suite, offline lab-harness tests, a debug build and synthetic
Rune renders. Fixtures use loopback mock hosts and a private fake D-Bus Secret Service.
They do not contact real hosts or the user's password store. The protocol tests exercise
lost responses, task deduplication, ownership changes, stale settings, cancellation,
profile authority mismatches, fallback endpoints and malformed optional records.

Synthetic renders were inspected for compact connection/profile/property/device views.
The provided user screenshots and private inventory values were not added to the repository,
tests, documentation, package or generated scenes.

Final checks: **105 workspace tests**, **46 vendored RFB tests**, and **7 offline lab tests**
passed; strict workspace Clippy and formatting passed. Thirteen synthetic scenes rendered,
with final property/connection modal footers and device views inspected. The final desktop
suite was rerun after the last UI fixes (21 of the 105 workspace tests). UI fixtures initialize
without opening user preferences, keyrings, profile stores or a host backend.

Release: `artifacts/xenadmin-rs-0.3.0-linux-x86_64.tar.gz` (portable archive, not AppImage).
SHA-256: `4773785e125a9013ba014974d76716a81ed95d4cb823b6c02fd6d86fdb4d9af9`.
The packaged executable matches the final release binary. Package inspection confirmed
connection instructions and vnc-rs MIT/Apache notices; no captures or local state are included.

## Remaining boundaries

This increment has no new live-host qualification. The environment did not expose `/dev/kvm`
or QEMU/libvirt tools, so actual nested hosts and infrastructure-changing acceptance tests
were not run. No production host or guest was changed. Native Wayland/X11 clipboard,
keyboard layouts, accessibility, real guest console protocols and clean Ubuntu installation
still need the platform/lab matrix.

The larger VM/template/snapshot lifecycle, storage/network administration, migrations,
imports/exports, host/pool/HA/auth workflows, metrics/alerts/reporting, organization/vApps,
plugins/localization and AppImage qualification remain in the ledger. The new graph or
backend helpers do not count as implementation of those whole workflows.

Within this slice, connection import/export, unified cross-pool searches, automatic saved
session restore, certificate-inspection dialogs, XML-RPC compatibility and automatic console
relocation remain open. Snapshot deletion/clone, VMSS schedules and restore orchestration
are not implemented. Disk growth does not resize guest partitions/filesystems. Property
editing does not yet cover every XenAdmin property. XAPI map setters have no compare-and-swap
transaction, so another client's concurrent map edit can still race the fresh-read merge.

Rune's current secondary-button API lacks a release callback: right-button dragging uses
**Hold right button** plus **Release input**; focus loss releases held input. Separate console
windows remain open. See the [console decision](../decisions/0003-console-protocol-adapter.md)
for the bounded local vnc-rs patch, protocol behavior and preserved dependency licenses.
