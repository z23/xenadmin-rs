# For upstream: a forced shutdown during a storage migration that the host does not unwind

This is finding 63 of the [live qualification report](2026-11-09-lab-qualification.md),
written so that it can be handed to the XCP-ng project. It has not been filed anywhere.
It describes host behaviour seen on a nested lab, with nothing inside the hosts inspected,
so it is a report of symptoms and of how to produce them, not a diagnosis.

## Environment

- XCP-ng 8.3.0, installed from `xcp-ng-8.3.0-20260806.iso`, toolstack (xapi) 26.1.11.
- A pool of two hosts, nested under KVM, 4 vCPUs and 8 GiB each. Each host has a local
  `ext` repository. HA off.
- The guest: an HVM Linux guest without guest tools, two disks on the coordinator's local
  repository (68 MiB and 1 GiB), running.
- Two API clients with separate sessions, both as the local root account.

## Steps

1. Client A starts a storage live migration of the running guest to the other host of
   the same pool: `host.migrate_receive` on the destination, then
   `Async.VM.migrate_send` with `live = true` and a `vdi_map` that places both disks on
   the destination's local repository.
2. While that task is at 4 to 6 % progress, client B calls
   `VM.assert_operation_valid` for `hard_shutdown`, which succeeds, and then
   `Async.VM.hard_shutdown`. The host admits it and the guest is halted.
3. Client A calls `task.cancel` on its `migrate_send` task.

## What happened

In two runs of two on 2026-11-12:

- The `Async.VM.migrate_send` task stayed `pending` at the progress it had reached, with
  the cancellation listed in its `current_operations`, for as long as it was watched
  (fifteen minutes in the first run).
- The destination's local repository held unattached copies of the guest's first disk
  and a snapshot of it. `VDI.destroy` of one of them did not return: the task stayed
  pending at no progress, beside a pending `host.call_plugin` on the destination and a
  "Garbage Collection" task. A toolstack restart on the destination removed the pending
  tasks; a second `VDI.destroy` then hung the same way.
- In the first run, a `VM.pool_migrate` of another guest, on shared NFS storage, away
  from the destination host then stopped at 91 % with that guest no longer answering on
  the network.

One run two days earlier, on the same lab, went differently: the host stopped the
migration task itself after the forced shutdown and left nothing behind. So the outcome
depends on something that was not controlled, perhaps how far the mirror had got.

When the migration is cancelled first, with the guest left running, it is unwound
cleanly: the task ends as cancelled and the destination repository is empty afterwards.

## What it took to clear

- A toolstack restart on the coordinator removed the `migrate_send` task and the
  migration's snapshot on the source repository.
- The copies on the destination could be deleted only after the destination host was
  power-cycled. Each delete then took two seconds.
- After that power cycle, which cut off the `pool_migrate` above, the next
  `pool_migrate` of the same guest to the coordinator stayed at 0 % until the
  coordinator's toolstack was restarted once more.
- A "Garbage Collection" task record created just after a coordinator toolstack restart
  stayed pending for hours and had to be destroyed by hand.

## What would be expected

Either `hard_shutdown` is refused while a `migrate_send` of that VM is in progress, as
other operations are, or, if it is admitted, the migration is torn down with it: the task
ends, the destination's partial copies are removed or at least can be removed, and the
repository's lock is released.

## What was not looked at

No log or process on either host was read: the lab keeps the control domains off limits.
Whether this happens on hardware, with other repository types, or with a guest that has
guest tools is not known. The lab scenario that produces it is
`lab_force_shutdown_review_with_a_running_task` with
`XENADMIN_LAB_FORCE_DURING_MIGRATION=accept`
([lab instructions](../../lab/README.md)); without that variable the scenario records the
client's refusal instead, which is what the client now does by default.
