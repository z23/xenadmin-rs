# Same-pool migration and storage placement increment

This increment adds a reviewed ordinary-VM migration workflow to the Rune desktop.
It implements the bounded subsets recorded in [the P11 ledger](../PARITY-IMPLEMENTATION.md),
with synthetic protocol and UI verification. No production or disposable-lab hosts were used.

## Implemented behavior

The three-step review captures the source pool, connection generation, VM UUID, running or
halted state, resident host, home-host affinity, every data-disk UUID/source SR, and every
NIC UUID/source network. The user chooses a destination host, per-disk SR placement,
per-NIC destination network, and a transfer network when required. A live host-only move
uses `Async.VM.pool_migrate`; storage/network changes and halted moves use the same-pool
`host.migrate_receive` / `VM.assert_can_migrate` / `Async.VM.migrate_send` path. Selecting the
current host with a changed disk SR supports disk-only relocation through the same review.
A halted move separately sets the reviewed home host after verifying migration. It never
starts or resumes the guest.

Fresh checks resolve UUIDs again, verify the complete device attachment graph and advertised
capabilities (including blocked operations), and require an enabled, live destination with
attached PBDs and appropriate network PIFs. Inserted ISO media remains in its original SR
and must be accessible from the destination. Internal guest networks can lack PIFs, while
the selected transfer network requires an attached destination PIF with a usable IP address.
Server compatibility assertions validate additional CPU, memory and platform constraints.

Running migration preserves VM, VBD and VIF identity and verifies the destination host.
Moved data disks must have new UUIDs absent from the persisted pre-dispatch baseline, the
selected destination SR, matching type/size and the expected exclusive attachment.
Replacement disk UUIDs remain visible in operation history, including individually verified
disks after a partial result. Attachment ownership is preserved; the client does not claim
ownership of previously unowned data or issue separate disk-destruction/rollback calls.

## Durable outcomes and console behavior

The existing journal and task tracker own dispatch, task progress and cancellation.
Receiver tokens contain credentials and exist only in transient dispatch memory: they are
not stored in plans, histories, inspector projections or error text. VM/device/host/SR/network
reservations compose with existing pool-wide creation and resource-administration guards.

A lost submission response remains unknown, even if the destination is later observed.
Read-only reconciliation never repeats migration or a home-host setter. Persisted task
completion permits recovery after the host removes its task. A task polling API failure
remains unknown, whereas explicit submission rejection is a definite failure. A task that
fails after changing placement remains partial or unknown with the known replacement disks.
The client destroys only its own task after terminal outcome persistence.

A console follows a successful local live migration once current inventory reports its
reviewed destination. The console's pool/VM target is independent of table selection.
Older migration results and late source-host events cannot reopen the old location; a
failed reconnect leaves the ordinary manual reconnect control available. The one-shot
tracking set is bounded. Unknown migrations and externally initiated migrations still
require manual reconnect or outcome reconciliation.

## Limits and pending work

Storage-transport moves currently refuse snapshot/checkpoint chains, shared or multiply
attached source disks, CBT disks, missing/unmanaged/read-only data disks, and unsupported
VM states. A host-only shared-storage move may preserve snapshots. Paused/suspended guests,
cross-pool migration, compression settings, IPv6-only transfer selection, standalone disk
copy, and migration of snapshot disk chains remain pending. Existing disk copies and
legacy halted-VM copy/delete implementations were not substituted for migration.

The ordinary-VM workflow uses XAPI's older compatible argument set, leaving specialized
GPU/vGPU mappings outside this slice. Actual host-version, storage-driver and network
compatibility, live console continuity and failure recovery still need explicit lab
qualification. Synthetic Rune checks do not establish native Wayland/X11 behavior.

## Evidence

The implementation follows the official [VM API](https://xapi-project.github.io/xen-api/classes/vm.html),
[host API](https://xapi-project.github.io/xen-api/classes/host.html) and the XAPI
[migration implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_vm_migrate.ml).
The [VM lifecycle rules](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_vm_lifecycle.ml)
permit `migrate_send` for halted and running guests, despite older generated method prose
that describes running guests only. The local reference XenAdmin `VMMigrateAction` and
`VMCrossPoolMigrateAction` supplied behavioral context; no source was copied.

Eighteen new core cases cover host-only and storage migration, halted home-host changes,
NIC mappings, stale topology, shared/CBT/snapshot refusal, incompatible destinations,
malformed receiver host identity, existing-disk/wrong-VM results, partial disk retention,
lost replies, task removal, read-only restart reconciliation and admission guards. The
existing read-only connection integration case now explicitly rejects a migration command.
Six new desktop cases exercise compact/wide reviews, captured choices, stale source/home-host
checks, and console handling of selection changes, multiple migrations and late events.
The workspace run passed 262 tests (140 core, 68 desktop, 41 direct-host integration,
five profile, four TLS and four host-probe tests). A subsequent additional disk-only
relocation case passed in the focused rerun, bringing the verified total to 263; all
18 migration and six desktop migration cases passed after the final changes. Workspace
strict Clippy, formatting and diff checks passed. Synthetic Vulkan review renders at
1440×900 and 513×652 were inspected locally; buttons and review content fit both sizes.
The generated synthetic images stay outside the repository. No supplied private screenshots
or host inventory were copied into artifacts.
