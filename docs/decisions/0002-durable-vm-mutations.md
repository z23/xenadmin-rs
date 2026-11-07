# Durable VM mutations

## Problem

Snapshots add parent relationships, returned object identities and disk cleanup to the
existing power-task journal. Configuration edits can require several setters. A lost
response must not cause either workflow to replay a mutation or discard known task evidence.

The user-provided review identified recovery/dispatch races and stale observers in the
initial implementation. The recovery foundation now uses journal revisions, conditional
writes with checked row counts, a private noncloneable dispatch permit, and evidence merges.
Inventory polling is independent of task progress. Cancellation intent survives tracker
replacement. Read-only connections do not reconcile mutation workflows.

## Caller's view

The intended interface is one semantic submission route returning a watched operation.
Read-only snapshot listing comes from coherent inventory. Fresh observations bind each
VM and snapshot to its pool and connection generation. UI review captures those observations,
then submission revalidates them.

```text
view = connection.latest()
vm = view.observe_vm(vm_id)
snapshot = view.observe_snapshot(vm_id, snapshot_id)
operation = connection.submit(id, CreateSnapshot(vm, validated_name))
operation = connection.submit(id, RevertSnapshot(snapshot))
operation = connection.submit(id, CloneSnapshot(snapshot, validated_name))
```

These are design sketches until the corresponding APIs land. No caller builds RPC methods,
resolves opaque references, sequences cleanup or interprets task-result XML.

## Shape

Use a closed semantic command enum and private closed domain steps. The operation engine
owns durable per-step dispatch markers, task references, results, partial outcomes and
uncertainty. Its journal compares the entire normalized intent and reserves affected stable
identities. Only a successful conditional dispatch transition creates a permit to send a
mutation. Unknown dispatches cannot acquire another permit.

The connection actor remains the sole inventory writer. Workers publish operation evidence,
not inventory changes. Task results resolve to stable UUIDs before dependent steps. Snapshot
and parent VM identifiers are distinct types and relationships are rechecked before writes.

Deletion captures a durable ownership manifest before removing a snapshot record. Cleanup
includes only proven owned disks and checks for foreign attachments again. Successful record
deletion followed by failed disk cleanup is partial completion. Clone-to-VM includes a
separately tracked provision step; a returned template is not presented as a runnable VM.

Preserve old power records and their separate SQLite task column. New fields have explicit
defaults or a transactional migration. An undecodable journal fails mutation admission
visibly while inventory remains usable. A terminal result is published after persistence.

## Synthesis decision

Two independent candidates compared typed semantic workflows against immutable private
step plans. The latter is the base because multi-step recovery retains the decisions made
before an object disappears. The former contributes the small semantic public API, strict
observations and full-intent equality. Both agreed on ownership and unknown-outcome rules.
No generic public workflow language is introduced.

The session could not enforce a Task readonly flag, so reviewers were instructed to keep
source read-only and write isolated design artifacts. All configured judge models share
the parent's model family, so a cross-family judge was unavailable; the parent compared
both complete candidates. The parent model settings were unchanged.

## Tradeoffs

We accept conservative parent-VM reservations in exchange for predictable local conflicts.
External clients can still change infrastructure, so current server checks remain authoritative.
We accept explicit partial outcomes in exchange for avoiding unsafe automatic compensation.
We accept an internal step schema in exchange for inspectable restart behavior, without
exposing step ordering to callers.

## Initial scope and verification

Start with standard disk snapshots, a halted parent for revert, guarded snapshot/disk
deletion, and clone/provision. Extend the same executor to independent name, CPU and memory
edits. Additional snapshot modes, automatic power restoration and advanced device workflows
need their own capability and failure fixtures.

Tests must observe wire mutation counts, preserved reservations, legacy task recovery,
late evidence, stale/reparented selections, partial disk cleanup, returned object identity,
and failures at each dispatch/persistence boundary. Live writes require a designated
disposable target and authorized scope; application development does not authorize them.
