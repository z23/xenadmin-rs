# Snapshot metadata and restore workflows

This increment implements P05-03/04/07 in the Rune desktop and Rust core, with synthetic
protocol and interaction verification. Live-host and data-integrity qualification remain
open. No production hosts were contacted or changed, and no private screenshots were used.

## Delivered behavior

Snapshot creation accepts a name and description for disk snapshots and memory checkpoints.
The host creates the snapshot first; the client verifies its parent, new UUID and mode before
setting the description in a separate tracked step. A metadata failure retains the created
snapshot UUID and reports a partial result. A task result pointing at a pre-existing snapshot
cannot cause that object's metadata to be changed.

The snapshot page includes an editor for name and description. It captures the snapshot and
parent UUIDs, pool and connection generation, and the original metadata. Each setter checks
that the previous values still match. A concurrent edit stops remaining setters instead of
silently replacing the other operator's values. Names are limited to 1–255 characters and
descriptions to 4096; unsupported control characters are refused.

Restore reviews explicit power choices. A halted VM can be restored directly. A running VM
requires the operator to select a permitted clean shutdown. Paused or suspended parent VMs
must be stopped separately; this workflow does not force a shutdown or discard their current
memory implicitly. After restoration, an optional startup choice starts a disk snapshot or
resumes a memory checkpoint. By default the VM remains halted or suspended respectively.

The workflow separately journals shutdown, revert and startup. It checks the exact parent
and snapshot identities again before each mutation, honors allowed and blocked operations,
and calls the host's operation-validity check. Successful restore requires a successful host
task, the VM's ancestry pointing at the selected snapshot, and the expected restored power
state. A checkpoint also requires a distinct restored memory VDI; the original snapshot's
saved memory is retained. Startup capability is checked again after restore, so a refusal
leaves a verified restore with a partial result.

## Recovery and cancellation

Every async stage records its task receipt and completion evidence privately. Lost dispatch
responses, missing tasks, failure/cancellation of a host task, or inconsistent postconditions
remain unknown; matching power state alone cannot prove the operation completed. Recheck
reads task and object evidence without submitting pending steps or replaying earlier writes.
The snapshot page exposes recheck and stop-remaining-steps controls, alongside Activity.

Stopping the workflow does not cancel an in-flight host task. It prevents subsequent stages
once that task resolves; for example a completed shutdown can leave the VM halted without
restoring it. Failed restores never trigger an automatic start, force action or rollback.

A dedicated journal compare-and-save merge retains late task/result/created-UUID evidence
after recovery or explicit acknowledgment while preserving the operator's state. A recovered
or acknowledged workflow cannot dispatch another step. Private task references, identity
baselines and reconciliation plans do not appear in public serialized operation records.

## Supported host boundary

Guest-quiesced/VSS snapshots are explicitly unavailable on the supported XCP-ng 8.1+ lineage.
The [XCP-ng 8.1 release notes](https://docs.xcp-ng.org/releases/release-8-1/) document their
removal. The desktop offers no legacy quiesced API or automatic fallback that would pretend
to provide application consistency. Disk snapshots may require application preparation or
a prior guest shutdown; a memory checkpoint is a separate choice.

The restore implementation follows the upstream XAPI contract: `VM.revert` restores metadata
and disks and sets the parent VM to the snapshot's power state. The
[XCP-ng snapshot implementation](https://github.com/xcp-ng/xen-api/blob/master/ocaml/xapi/xapi_vm_snapshot.ml)
updates VM ancestry, clones checkpoint memory, and treats revert as non-cancellable. These
sources informed the implementation; they do not replace XCP-ng 8.3 lab qualification.
Reference-client behavior was also read in `VMSnapshotCreateAction.cs` and
`VMSnapshotRevertAction.cs`; no upstream implementation code was copied.

## Verification

The new coverage comprises 21 core tests and four Rune interaction tests. Protocol cases
cover disk and checkpoint success, explicit shutdown, retained stopped state, metadata
conflicts/refusal/lost replies, existing-object task results, changed parent and memory
identity, startup refusal, task failure/cancellation, missing dispatch receipts, restart with
a known task, stopping later steps, private serialization, and late receipts after recovery
or acknowledgment. Rune tests exercise name/description entry, captured selections, explicit
shutdown/start choices, checkpoint resume copy, and disabled demo submission at compact and
wide sizes. Existing snapshot clone/delete and memory-mode tests remain in place.

The full workspace suite passed 507 tests: 235 core unit, 142 direct protocol, 117 desktop,
five connection-profile, four TLS and four host-probe tests. Strict workspace/all-target
Clippy, workspace build, formatting, diff and shell syntax checks passed. The ten snapshot
Rune tests passed again after the final compact table layout adjustment. Eight synthetic
Vulkan renders cover listing, creation, edit and restore at 1440×900 and 513×652; the
resulting layouts were inspected, and the compact listing remains scrollable. These renders
do not establish native Wayland, keyboard, clipboard or accessibility qualification.

Integration onto `1fb4cb8` preserved pool joining, VMSS receipt handling, affinity and
device properties. The combined workspace passed 534 tests (245 core unit, 152 direct
protocol, 124 desktop, five profile, four TLS and four host-probe), all-target compilation
and strict workspace/all-target Clippy. Both sets of synthetic scene checks remain in
the standard check script.

Live shutdown, checkpoint resume, restore disk integrity, host permission behavior,
and failures across toolstack/host restart remain deferred to the authorized disposable lab.
Snapshot ancestry/preview detail remains separate retained work; this increment does not
claim that workflow complete.
