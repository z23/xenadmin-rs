# Snapshot increment — 0.2.0

This increment adds per-VM snapshot listing, standard disk-snapshot creation and restore
through direct XAPI. The Rune view includes a virtualized snapshot table, captured-target
review dialogs and operation feedback. Non-VM pages do not show console/power controls;
their tables use object-appropriate columns. No Xen Orchestra service is involved.

The core separates `SnapshotId` from `VmId`, indexes parent relationships in the coherent
inventory batch, and excludes snapshot records from guest/template tables. Orphan snapshots
remain representable but cannot mint a parent-bound mutation observation. Commands carry
fresh pool/generation observations, not raw opaque references from the UI.

Power and snapshot commands use `Connection::submit(id, VmCommand)` and the same durable
journal. Intent equality includes snapshot names and selected snapshot UUIDs. Admission
reserves both the parent and selected snapshot identity. Restore freshly checks both
records and requires a halted normal VM. It sends `Async.VM.revert` the snapshot reference,
then leaves starting or resuming the restored guest to the operator.

Create sends `Async.VM.snapshot(vm, name)`, following the pinned reference client's API
shape. It parses the bounded XML task result without DTD expansion, saves the resulting
object reference, and verifies its UUID, snapshot kind and parent before reporting success.
A saved result can be verified after reconnect without fetching the task again. A missing
reply or mismatched result remains unknown, preserving the reservation and preventing
automatic duplicate writes. Legacy power journal rows remain readable.

## Local evidence

The final check script passed formatting, Clippy with warnings denied, all **52 tests**,
workspace builds and eight synthetic Rune exports. Snapshot fixtures cover successful create/revert, exact wire
arguments, full-intent deduplication, stale observations, read-only rejection, changed
power/parent preconditions, lost replies, mismatched returned objects and saved-result
recovery. Reducer tests cover snapshot-before-parent ordering, orphan/deletion handling and
index reconstruction. The Rune interaction check opens the view, edits/cancels create,
selects a snapshot and opens restore review at both 1440×900 and 513×652; demo
confirmations cannot mutate infrastructure. The Snapshots entry point is directly below
the VM name so it remains reachable in compact Details.

All protocol servers run locally with synthetic identities and isolated state. No live host
requests or production mutations were made for this increment. No operator screenshots,
private inventories, endpoint identifiers or console text are included in source, docs,
fixtures or package artifacts.

## Boundaries

Snapshot deletion and clone-to-VM are still planned: they need ownership-reviewed disk
cleanup and separately tracked provisioning. VM property/CPU/memory edits, device editing,
network/storage administration and broader milestone qualification remain open. Creation
does not offer guest-memory or quiesced modes. Restore can use an existing snapshot whose
host advertises revert, but actual disk/checkpoint behavior still needs live qualification.

These fixtures do not establish XCP-ng 8.3 compatibility or data-integrity acceptance.
The M0–M4 live gates remain open. Designate a disposable pool and disposable data before
running snapshot writes or recovery interruption scenarios against a real host.

## Build artifact

Source: `2755a5bc1e83ea6141003a3b7f93f3ddbabb98ec` (Rust 1.99.1, pinned Rune and
`Cargo.lock`). The local Linux x86-64 archive is
`artifacts/xenadmin-rs-0.2.0-linux-x86_64.tar.gz`; SHA-256:
`30cf18269651c2a2d500b361c6b0929fed5be5c3d22c6ae78a5f994cc14ace41`.
Its checksum, executable equality with the release build, extraction and snapshot-view
rendering from the extracted executable passed with isolated synthetic state. The archive
contains the two executables, desktop assets, running guide, license and dependency notices.

[Ubuntu 24.04 CI](https://github.com/z23/xenadmin-rs/actions/runs/42372297031) passed
formatting, strict Clippy, all 52 tests, workspace builds and the eight headless render
exports for that source commit. This is CI/fixture evidence, not a live-host acceptance run.
