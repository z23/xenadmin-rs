# Reviewed batch snapshot and checkpoint exports

P15-06 now includes serial export of 1–32 snapshots of one pool, each to its own new
output path. The Snapshots view of a VM takes a selection of several of that VM's
snapshots (Shift or Ctrl with a click, or Shift with the arrow keys) and "Export
selection…" opens a review with each snapshot's name and UUID, whether it is a disk
snapshot or a memory checkpoint, its output path, the format and the permission
assessment. While several snapshots are selected, restore, edit, copy, inspect, delete and
the single export have no target and are unavailable.

Formats and their limits are those of the single export. A disk snapshot goes out as
plain, gzip or zstd XVA, an OVF directory or an OVA file, under the ordinary-HVM OVF
subset. A memory checkpoint goes out as XVA only: the package formats are not offered
for a selection that holds a checkpoint, and the core refuses such an item. The review
shows the checkpoint's memory image (its UUID, size and repository) and one choice per
checkpoint, "Disks only" or "Include guest memory". Memory is left out until it is chosen
for that checkpoint, and changing the choice ends a review that was made before it. An
archive with memory imports as a suspended guest; one without imports as a halted guest.

## How it is built

A snapshot batch is the existing workflow batch with a new kind of item, not a second
mechanism. `VmWorkflow::ExportSnapshot` names the snapshot and, for a checkpoint, the
reviewed memory image and the choice; the item's VM is the snapshot's parent. The child
of an item is the single snapshot or checkpoint export of exactly that source
(`ExportSource::Snapshot` or `ExportSource::Checkpoint`), run by the existing transfer
worker. The parent, the private grants, the pool reservation, Stop, recovery,
acknowledgement, pruning and the recheck that only verifies files already published are
the ones [bulk VM export](2026-11-08-bulk-vm-export.md) has; no worker or journal rule
was added. The whole-selection preflight runs the single export's fresh checks on every
item before the first child: the snapshot is still a snapshot of that VM, it is halted
or, for a checkpoint, suspended with the reviewed memory image unchanged, idle, of the
reviewed size and owned by that snapshot alone.

A checkpoint is admitted only as an item with its memory image reviewed. One named as an
ordinary snapshot fails the preflight, because the host reports it suspended. After the
transfer the archive is read whole and must hold one guest under the snapshot's UUID,
with the reviewed memory image when memory was chosen and with none when it was not.

Outputs are bound to snapshots. Several snapshots of one VM share that VM's UUID, so the
VM-keyed submission a batch of VMs uses refuses a snapshot batch, and
`Connection::submit_snapshot_export_batch` takes one output per snapshot UUID.
Destination paths stay in the private plan as before.

## Stored format

The operation journal goes from format 4 to format 5. Format 5 adds no table and rewrites
no row: opening a format 4 journal only raises its number, in the same transaction that
earlier migrations use. The number is raised because a snapshot batch is stored as a
workflow variant that 0.19.0 does not have, in the parent's action and in its private
plan, and 0.19.0 fails to read a journal that holds a row it cannot parse. With the
number raised it refuses the file whole, says it was written by a newer application and
leaves it as it is. 0.19.0 therefore cannot submit changes from a state directory that
this version has used; the changelog entry says so. The number is raised when the
journal is opened, as the earlier migrations do, and not only when the first snapshot
batch is stored, so this holds for someone who never uses the batch.

## Validation

- The full workspace run passed 1,323 tests, 14 of them new: 588 core unit (8 new), 355
  direct protocol, 359 Rune (6 new), five profile, four TLS, four host-probe and seven
  binding tests, and the one unignored harness test. 103 are ignored, one more than
  before: the new lab scenario.
- Core fixtures: a two-snapshot XVA batch on a read-only session, one disk snapshot and
  one checkpoint with its memory chosen, with both archives read back and each child
  bound to the parent; refusal of the whole selection before any task or download when
  the second snapshot's memory image changed size or is busy, the snapshot was
  reparented or is gone, or its destination exists; a checkpoint whose memory was not
  chosen, exported without memory, with a host that returns memory anyway not believed
  and a checkpoint named as an ordinary snapshot refused; Stop after the first child;
  recovery and acknowledgement during the first child, and a recheck that only reads;
  an interruption between the two children, after which two rechecks download nothing
  and leave the first archive as it was; the closed set of valid items; the binding of
  outputs to snapshots; and the migration of a format 4 journal that holds a VM export
  batch and its admitted child.
- Rune tests: selection of two snapshots in the view at 513×652 and 1440×900, made
  with a click and then Shift with an arrow key, and what it disables; the review at
  both sizes with names, UUIDs, kinds, memory image, memory choice, output paths,
  format, permission assessment, the experimental notice and the three footer buttons
  inside the window; the per-checkpoint memory choice with two checkpoints; the formats
  offered with and without a checkpoint; refusal of the whole review after a change of
  snapshot, memory image or destination; and the history lines and Stop wording of a
  snapshot batch.
- `cargo fmt --all --check`, strict workspace Clippy and `scripts/check.sh` passed on
  the committed tree; the script also ran the vendored protocol crate's 46 tests, the
  53 Python tests of the lab and packaging scripts and its headless renders. One earlier
  run of the script stopped on an existing test of desktop activation,
  `desktop_activation_reclaims_only_stale_owned_socket_and_preserves_replacements`,
  which found its stale socket still answering. That test passed alone twelve times and
  in the three full runs of the script after; it is not touched by this change and was
  not looked into further.
- The synthetic `snapshot-batch-export` scene was rendered with `--snap` at 513×652 and
  1440×900 and looked at; the footer is reachable in both.

## What this does not show

When this was written nothing had run against a host. Later that day
`lab_snapshot_batch_xva_export` was run on the disposable lab and passed on its first
run: a disk snapshot and a memory checkpoint of the running lab guest were exported in
one batch as uncompressed XVA, the checkpoint with its memory chosen for it, in 45 s.
The archives were 74.6 MB and 756 MB, and the client's own inspection read each back as
one guest under its snapshot's UUID, with one memory image in the checkpoint's archive
and none in the disk snapshot's. The archives, the snapshot and the checkpoint were then
removed ([live report](2026-11-09-lab-qualification.md), evidence
1794466851-snapshot-batch-xva-export.json). That is one run, of two snapshots of one
guest, through the core and not through the window. It is also the first time a
checkpoint was exported with its memory on the lab at all (P11-07). The other formats,
Stop and recovery have not been tried against a host.

The fixture tests enter at the batch request, as the bulk VM export tests do. The
desktop's submission through `Connection::submit_snapshot_export_batch` is exercised
only by that lab scenario; the Rune tests stop at a review that a synthetic
session cannot submit. Headless renders and Rune tests do not establish the behaviour of
a real desktop: the file portal, keyboard focus and assistive technology were not
checked, and selecting with Shift or Ctrl and a pointer click was not exercised at all,
in a test or on a seat. The core accepts snapshots of different VMs of one pool in one
batch; the desktop offers those of one VM, and nothing else was reviewed on screen.

When this was written, an interruption after a child's task was created and before
anything was streamed left that child, and so the batch, unknown until it was
acknowledged. That was changed the same day for every transfer: a recheck ends such a
child as not sent, because the journal marks a transfer as sent before its request goes
out, and the batch then ends as partly completed (fixture covered for a snapshot batch;
the code is the one bulk VM export runs). Restoring an exported
archive, compressed checkpoint archives against a host, snapshot ancestry in an archive
and cross-pool bulk moves remain open.
