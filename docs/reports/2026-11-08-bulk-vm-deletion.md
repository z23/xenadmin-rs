# Reviewed bulk VM deletion

The desktop can capture 1–32 visible VMs or custom templates from one pool and review their deletion together. Each must be halted and satisfy the existing single-VM deletion restrictions. Missing, changed, busy, default-template or unsupported targets refuse the whole review. The final confirmation names the captured count; each row lists its VM UUID and retained or selected disk/snapshot data.

The default deletes VM records and their device attachments while retaining disk data and snapshots. Ordinary snapshots and exclusively owned disks can be selected individually. Memory checkpoints use their separate existing deletion workflow. The batch never silently adds disks or snapshots. Before submission the desktop rebuilds the exact review against the captured pool/generation and current inventory, and core checks every selected workflow before the first mutation.

## Durable execution and interruption

The batch is a pool-scoped resource operation. It allocates immutable child operation IDs and delegates to the existing VM deletion worker, one target at a time. Journal version 4 retains earlier VM/resource rows and adds normalized child ownership links. An internal admission permit binds parent, ordinal, pool, VM and the complete typed deletion action. It excludes only that exact parent from conflict checks. Public callers cannot submit or control owned children independently.

Stop allows the current child's entire reviewed deletion workflow to finish and skips all remaining targets. It does not cancel a host task or roll back completed steps. Changed state before a later child stops the batch with the verified subset retained.

Recovery latches the parent and active child together. Explicit recheck only inspects already-admitted children; it cannot submit another VM or an unsent child step. Acknowledging an unknown batch acknowledges its unknown child in the same transaction. This releases the local reservation without claiming the host operation succeeded. Late evidence remains attached to those identities without reviving execution. Pruning removes terminal parent, child and ownership links as a group; unresolved children retain their parent evidence.

Existing synchronous deletion reconciliation is preserved. After a lost `VM.destroy` reply, explicit read-only recheck can establish that the exact captured VM UUID is absent. That establishes record absence, not an attributable completion receipt, and does not imply unselected disk cleanup. No next target is started after recovery even if that child becomes verified.

Dispatch is journaled before sending a write. Its immediate progress notification cannot wait behind UI backpressure after the final dispatch fence; later results remain durably queryable and reliably published.

## Verification

Synthetic tests cover successful serial deletion, whole-selection refusal, stop after the first child, changed later target, lost replies, recovery and acknowledgement during a reply, late evidence while recheck is active, exact permit binding, public-child control refusal, conflicting unrelated writes, immutable/corrupt links, and grouped pruning with an unresolved child. A bounded full progress queue exercises the final dispatch boundary. Migration fixtures verify v2 and v3 journal evidence survives v4 installation and reopening.

Rune interaction tests exercise compact/wide review and confirmation, immutable selection, changed/missing target refusal, explicit disk selection and sharing changes, default templates and read-only refusal. The `batch-delete` synthetic scene is included in `scripts/check.sh`.

The complete workspace passed 1,057 tests: 470 core unit tests, 285 direct protocol tests, 289 Rune tests, five profile tests, four TLS tests and four probe tests. Strict all-target workspace Clippy and release all-target compilation passed. Native synthetic renders at 513×652 and 1440×900 were visually inspected: the review body scrolls and the action footer remains visible. The initial full run caught two stale version-3 migration assertions; they were updated for the actual version-4 schema, and a separate v3 resource-evidence migration test was added before the passing run.

No production host, guest, private screenshot or live deletion was used. Live XCP-ng compatibility, deletion integrity and native accessibility qualification remain deferred to the prepared disposable lab.

## Remaining scope

This increment implements batch deletion only. Reviewed bulk properties are a separate increment. Same-pool moves, cross-pool moves and batch archive exports remain open; the private ownership mechanism is designed to support their distinct existing child workflows without replacing their evidence or transport rules. A terminal partial batch is never resumed automatically: remaining targets require a new review.
