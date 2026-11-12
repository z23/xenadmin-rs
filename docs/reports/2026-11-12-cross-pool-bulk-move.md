# Reviewed batches of cross-pool moves

P15-06 now includes a serial move of 1–32 VMs from one connected pool to another. With
VMs selected in the inventory, "Move selection to another pool…" sits beside the
same-pool "Move selection…" in the bar above the table and after it in the row menu. It
opens a review in the three stages of the
[single cross-pool move](2026-11-08-cross-pool-transfers.md): the destination pool and
host, the mappings and the migration address, and a review of both pools with the
transport acknowledgement. This is implemented and verified by fixtures. Nothing of it
has run against a host.

## What is decided

One destination pool, one destination host and one migration address take the whole
selection.

Mappings are chosen once for the selection. One destination repository takes every disk,
and each source network that a selected VM has a NIC on maps to one destination network.
Nothing is mapped by default in another pool: the repository, each network and the
address are asked for. The review then lists the mapping this gives each VM, with every
disk's source and destination repository and every NIC's source and destination network,
and each VM is submitted with exactly that mapping. A VM that needs a mapping of its own
is moved by itself.

A batch may hold running and halted guests together. A running guest moves live and
stays running; a halted guest stays halted. Each is the single workflow's own mode, and
the moves are serial, so one does not depend on another. A suspended or paused guest, a
template, or a VM the single move would refuse makes the whole selection unavailable
with the VM named; none is left out. A guest that changes state between the review and
its turn is refused as changed when its turn comes, and the batch ends there.

A batch moves and does not copy. The core refuses a copy as an item.

The transport acknowledgement is given once for the batch, on the review, in the single
form's words. It is recorded on every VM's move, and a batch in which any move lacks it
is refused.

## How it is built

A batch of cross-pool moves is the existing workflow batch with a new kind of item, not
a second mechanism. `VmWorkflow::MoveToPool` holds one reviewed `CrossPoolVmOperation`:
the guest, the destination pool and host, the resident and home host that were reviewed,
the migration network and address, the mode, every disk's repositories, every NIC's
networks and the acknowledgement. `Connection::move_vms_to_pool` takes the batch with
both pools as they were observed (`CrossPoolBatchSelection`) under the single move's
rules: both connections writable, current and owned by one backend. The ordinary batch
submissions refuse such a batch, because they carry no second connection.

The child of an item is the single cross-pool move of exactly that operation, run by the
existing cross-pool worker. Its operation ID is allocated in the parent's private plan
when the batch is made. The journal admits it only with the parent's permit and only if
its action equals the item's operation, so the child is bound to its guest, its
destination and every mapping. A stored child that differs fails the batch closed.

The parent reserves both pools. It is refused at admission while either pool has another
operation, it blocks VM and resource operations in both while it runs, and it is listed
in both pools' histories. A child reserves the same two pools, and its permit excludes
its parent alone from the conflict check, in both.

Before the first child the parent checks the whole selection. It refuses a session role
that is known not to allow `VM.migrate_send` on the source pool or `host.migrate_receive`
on the destination. It then runs the single move's own preparation for every VM with
both sessions: the source pool and the guest's shape, devices and membership, the
destination pool, host, repository, networks and migration address, that VM's disks
against the repository's free space, and the destination's identities. Last it asks each
destination repository for the space of all the disks mapped to it, where the single
move asks for its own. This reads only: no receiver is prepared and nothing is sent. Each
child then repeats its own preparation when its turn comes, prepares its own receiver,
asks the hosts for compatibility, checks again and submits one transfer.

Stop finishes the current VM's move and skips the rest. It does not ask the hosts to
cancel a transfer. A child takes no control of its own, so a transfer that is under way
in a batch cannot be cancelled from the client, which a single transfer can ask for.

The fences are the batch's and the cross-pool worker's together. Recovery and
acknowledgement fence the parent and its active child in one transaction, as for every
batch. The cross-pool worker's saves before `host.migrate_receive` and before
`Async.VM.migrate_send` are, for a child, also refused once its parent is fenced,
whatever the child's own row says. A move of its own keeps the save it had.

## After an interruption or a restart

The receiver a move is prepared with lives only in the worker that prepared it. It is
never stored, so nothing can continue a move after a restart.

When a pool of an unfinished batch connects, its recovery fences the batch and its
active child as unknown. Either pool may be the first to do so. No worker is started for
the batch on either pool, because one connection cannot read a move between two, and the
ordinary recheck is refused for it. Both pools stay reserved.

The operator reconnects both pools and uses Recheck on the batch, from the pool's
history, Activity or the cross-pool transfers view. The recheck reads the children that
were admitted, with the receipts they recorded. It prepares no receiver, sends no
transfer and starts no remaining VM. What it finds for the child that was active:

- The transfer's task is recorded and succeeded: the destination and the source are
  verified as for a single move, the child succeeds, and the batch ends with the
  remaining VMs not started. A new review is needed for them.
- The task is still pending, or failed, or was cancelled: the child stays unknown, with
  that said on it.
- The transfer was submitted and no task was recorded: the child stays unknown. Nothing
  is replayed.
- The child had stored no plan: it prepared no receiver and sent nothing, and it ends as
  not sent. The plan is stored by the save that precedes the first call that changes a
  host, so this is read from the journal and is not an inference about the hosts. The
  same now holds for a single cross-pool transfer in that state, which used to stay
  unknown.
- The child had prepared its receiver and had not submitted the transfer: it stays
  unknown, as a single transfer in that state does. The receiver session is left to
  expire on the host.

An unknown batch is ended by Acknowledge, on either pool's connection. That releases the
batch and its unknown child together and both pools with them, proves nothing about the
hosts and removes nothing. A child that was already unknown when its batch is fenced
again keeps what was found about it, so the batch shows why a recheck did not settle it.

## Desktop

The entry is in the selection bar and the row menu of the VM page. The dialog has the
bulk dialogs' frame: a heading, a scrolling body and a footer that stays in the window.
It carries the experimental notice. The review shows the destination, each VM with its
mode and mapping, what the selection's disks need of the repository beside what the
inventory reports free, the single form's transport warning with the one
acknowledgement, what Stop and recovery do, and the permission assessment of the session
on each pool. The review is refused with its reason when a VM or the destination no
longer fits, when the inventory already shows the repository as too small, or when
either session's role is known not to allow its part. The reviewed batch is compared
with the pools again when it is submitted.

The console of a running guest in the selection closes when the batch is submitted, as
for a single live move. The batch is then shown in the pool's history, in Activity and in
the cross-pool transfers view, where a move that belongs to a batch is listed for what
it created and is rechecked, stopped and acknowledged through its batch.

The bar above the VM table has one more entry. At 513×652 it takes three rows where it
took two, and the table shows one row fewer. At 1440×900 the table keeps its place.

## Stored format

The operation journal stays at format 5. A batch of cross-pool moves is stored as a
workflow variant, in the parent's action and in its private plan, that 0.19.0 does not
have. 0.19.0 stops at format 4 and refuses a format 5 journal whole, and no published
version reads format 5, so the number is not raised again. No table is added and no row
is rewritten. A child is a cross-pool transfer row as a single transfer is.

## Validation

- The full workspace run passed 1,344 tests, 20 of them new: 593 core unit (4 new), 365
  direct protocol (10 new), 365 Rune (6 new), five profile, four TLS, four host-probe
  and seven binding tests, and the one unignored harness test. 104 are ignored, one more
  than before: the new lab scenario.
- Core fixtures, on two fixture pools through two connections of one backend: a batch
  of a halted and a running guest that completes in order, each child the single move
  of its reviewed operation under the parent, with a receiver of its own; refusal of the
  whole selection, before a receiver, a compatibility request or a transfer, when the
  second guest's disk became shared, it gained HA protection or a snapshot, it was
  halted after a review as running, the two disks fit the repository only one at a
  time, or neither fits; refusal for a role that may not send or may not receive; the
  acknowledgement required on every move, with a second pool, host, address or
  migration network, a copy, a disk named twice and a suspended guest refused, and the
  ordinary submissions closed to such a batch; Stop during the first move, which
  finishes it and never starts the second; both pools refusing another change while the
  batch runs and admitting one when it has ended; a restart while the first transfer is
  with the hosts, after which the fenced worker keeps its receipt and does nothing
  else, no worker is started on either pool, and a recheck with both pools verifies the
  first move by reads alone and leaves the second not started; a restart while the
  first move was still reading, which a recheck ends as not sent; a lost reply, which
  leaves the batch unknown through a recheck until an acknowledgement on the
  destination pool's connection releases both pools; and a destination that is another
  connection, another backend's, read-only, or whose repository, host or address
  changed after the review.
- Journal and worker tests: the closed set of valid batches; both pools reserved against
  VM and resource operations and released by acknowledgement; a child admitted only
  with the permit, in order and with the reviewed mapping; a stored child with another
  mapping failing the batch closed; recovery and acknowledgement from the destination
  pool, and none from a third; an unknown move keeping its finding; and the child's
  dispatch save refused under a fenced parent whose child row was not touched.
- Seven rules of the core were taken out of the code one at a time while it was
  written, and the tests of each were seen to fail: the whole-selection space check,
  the whole-selection preparation, the destination pool's right to fence, the exact
  match of a child's mapping, the dispatch fence, the settlement of a move without a
  plan, and recovery starting no worker. With the space check out the fixture batch
  went through, because the fixture repository reports the same free space after a
  move. The Rune tests were not checked this way.
- Rune tests: the entry in the selection bar and the row menu and the editor opening on
  exactly the selected VMs at 513×652 and 1440×900; the review at both sizes with each
  VM's mode, disks and NICs, the destination, the space line, the transport warning,
  the experimental notice, both sessions' assessments and the three footer buttons
  inside the window; each stage asking for its choices, nothing mapped by default, and
  the acknowledgement given on the review, recorded on every move and ended by going
  back or by another host or pool; refusal of the whole selection, with the VM named
  and none left out, for a changed, missing or suspended VM, a shared disk, a detached
  repository, a repository too small and another pool in front; no submission for a
  review that no longer matches or without the captured connections, and no review
  opened for a session that cannot change the pool; and the history
  of an unknown batch with each VM, the controls on the batch only, and Stop's wording.
- `cargo fmt --all --check`, strict workspace Clippy and `scripts/check.sh` passed on
  the committed tree; the script also ran the vendored protocol crate's 46 tests, the
  53 Python tests of the lab and packaging scripts and its headless renders.
- The synthetic `batch-cross-pool` scene was rendered with `--snap` at 513×652 and
  1440×900 and looked at; the footer is reachable in both. The inventory was rendered
  at both sizes before and after the new entry.

## What this does not show

When this was written nothing had run against a host. Later that day
`lab_cross_pool_batch_move_round_trip` was run on the disposable lab
([live report](2026-11-09-lab-qualification.md)). It halts the lab guest and the tools
guest, moves them to the standalone pool in one batch and back in a second, each disk
to the repository it came from and each NIC to its network, and checks placement and
power state after each batch. It reviews each guest's home host as the pool reports it.

- Its first run found no eligible guest and stopped, rightly: the lab guest had a
  snapshot record that a power cut had brought back (finding 65 of the live report),
  and the tools guest carried an HA restart priority.
- With the snapshot record removed, the tools guest was still left out, because its
  disk attachment was not marked as owned by the guest, and the batch was a batch of
  one: the lab guest moved to the standalone pool in 46 s and back in 29 s, verified.
- With the tools guest's attachment marked as owned and its restart priority cleared
  for the run, both guests moved in one batch in 72 s and back in a second in 61 s:
  "2 of 2 VM workflows verified" each way, every disk on the repository it was mapped
  to, the tools guest's back on the shared NFS repository, and every NIC on its
  network (evidence 1794478141-cross-pool-batch-move-round-trip.json).

Two further modes of the scenario were run afterwards. With Stop asked for during the
first of two moves, the first guest arrived and the second was never started and stayed
where it was: "1 of 2 VM workflows verified; 1 not started", the batch partly
completed; a batch of one brought the first guest back (evidence
1794486340-cross-pool-batch-move-round-trip.json). With both guests left running, they
moved live to the standalone pool in one batch (68 s) and back in a second (61 s), each
arriving running with its disks and NICs as reviewed (evidence
1794486601-cross-pool-batch-move-round-trip.json). Those are the first live moves
between pools this client has made on the lab. Before it, the scenario's own review of a
running guest as a halted move was refused by the client before anything was sent ("The
ordinary guest's state or resident host changed; reopen this review").

All of it went through the core and not through the window. An interruption of a
cross-pool batch, and the recheck after it, have not been tried against a host; the
fixtures are the only evidence for them.

The fixtures enter at `Connection::move_vms_to_pool` and
`Connection::recheck_cross_pool_batch`. The desktop's submission and its recheck through
them are not exercised: the Rune tests stop at a review that a synthetic session cannot
submit and at a recheck that has no connection to use, and they build the review from
synthetic inventories of two pools. A recheck that leaves a move pending, failed or
cancelled is tested for the single transfer and not again inside a batch. Headless renders
and Rune tests do not establish the behaviour of a real desktop: keyboard focus,
assistive technology and selecting with a pointer were not checked on a seat, and
nobody has used the dialog.

The fixture repository reports the same free space after a move, so the fixtures do not
show a host refusing the second move for space the first one took. The whole-selection
check exists for that case and is tested by itself.

The whole-selection check runs the single move's preparation once per VM, and each
preparation lists the destination's guests, disks and attachments. For 32 VMs on a large
destination that is many reads before the first transfer; its duration on a host is not
measured.

Open: mappings chosen per VM in the batch dialog, copies in a batch, cancelling a
transfer that is under way in a batch, and settling a move that was interrupted between
its receiver and its submission, which the single transfer does not settle either.
