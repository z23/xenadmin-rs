# Reviewed checkpoint XVA archives

Memory-checkpoint export and import are implemented with synthetic protocol, archive and Rune
verification. Live XCP-ng compatibility remains a disposable-lab qualification gate.

## User workflow

A suspended checkpoint's export panel starts with **Disks only**. Selecting **Include guest
memory** explicitly captures the snapshot/parent UUID, suspend-image UUID, storage and image
size. The review explains that RAM may contain credentials and application secrets. A private
temporary output is published to a new filename only after the complete stream, source identity
and host task verify. Existing files are never overwritten. Disk-only export normalizes the
archive to a halted guest without a memory image.

Selecting a plain or gzip XVA containing memory inspects its entire disk/memory payload before
review. A distinct checkpoint-import action shows the memory images and destination storage.
Before creating a host task, it reopens the exact selected file and repeats full verification.
The host creates new guest/disk UUIDs; the client verifies task provenance, destination storage,
memory type/size, unique suspend ownership and the captured guest shape. Imported memory guests
remain **Suspended**. No start or resume RPC is sent. CPU compatibility, hypervisor compatibility
and eventual resume success are not established by archive import.

The existing ordinary import action remains closed to memory-bearing archives. Checkpoints are
not offered as OVF/OVA exports. Unknown or unavailable snapshot export capability is refused.

## Evidence and interruption boundaries

The parser requires each suspend image to belong to exactly one suspended guest, forbids VBD
or ISO use of that image, rejects orphan/shared/missing memory records and bounds object counts,
sizes and streams. Plain and gzip payloads use the existing full sparse-chunk, XXH64/SHA-1,
tar-header and archive-termination validation. Selection and pre-dispatch checks run off the
UI thread and honor cancellation.

Fresh preflight binds the actual snapshot inventory entry, its parent and exact current memory
VDI/SR/size. Import uses a private UUID baseline and the owned task result/marker; existing
guest or memory identities cannot be adopted. Proven suspend-image UUIDs are retained separately
from ordinary disks for inspection after partial or uncertain results.

Late HTTP receipts, task outcomes and verified resource identities merge monotonically without
reviving recovered or acknowledged operations. Missing upload receipts remain partial even when
the host task succeeds. Explicit checkpoint recheck reads existing task/resource evidence; it
never repeats an upload or sends a previously unsent `task.cancel`. Unknown outcomes retain the
existing reservation until the operator follows the normal reconciliation/acknowledgement flow.

## Target source and remaining scope

The pinned [XAPI v26.1.21 export handler](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/export.ml)
includes `suspend_VDI` for a suspended guest when `preserve_power_state=true`; otherwise it
normalizes the guest to halted and clears the memory reference. The
[import implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/import.ml)
maps the imported suspend image and sets the new guest to suspended without resuming it.

Full `/export` explicitly constructs metadata with `with_snapshot_metadata=false`. Passing an
ancestry option cannot turn that endpoint into a complete snapshot-chain backup. The metadata-only
export path is different, and a separate multi-object package/reconstruction workflow remains
retained work. This increment therefore exports one independent guest/checkpoint, not its entire
snapshot ancestry. It does not declare every native ancestry-backup design unsupported.

No production host, guest, credential store or user screenshot was used. Actual checkpoint
creation/export/import/resume round trips and host-version/CPU combinations await the prepared lab.

## Verification

Focused cases cover complete plain/gzip memory archives; missing/shared/orphan/ISO/VBD memory;
truncation and checksum failure; cancellation; source mutation; memory/disks-only export output;
new suspended import provenance and wrong type/size/owner/baseline; no start/resume calls; lost
HTTP replies and missing receipts; private-journal corruption; recovery/acknowledgement during
task and resource reads; and read-only pending-task recheck. Rune fixtures exercise both compact
and wide reviews, immutable choices, pool switching and demo refusal. A synthetic inventory-event
fixture proves snapshot capture uses the snapshot map rather than ordinary VM details.

The final default-concurrency workspace run passed **1,007 tests**: 459 core unit, 271
direct protocol, 264 Rune/desktop, five profile, four TLS and four probe cases. This includes
nine new checkpoint core/archive/recovery cases and two new Rune cases. Strict workspace
all-target Clippy, release-mode workspace/all-target checking, debug build, formatting and
diff checks passed. Native Vulkan renders of the synthetic checkpoint export panel at
513×652 and 1440×900 were visually inspected; the memory choice and review controls are
reachable at both sizes. These renders do not qualify real compositor or host behavior.

An earlier complete run exposed an unrelated temporary desktop-owner reacquisition failure.
The separate [lock-release report](2026-11-08-desktop-lock-release.md) records its deterministic
regression and fix; the final workspace run includes that fix. Peer review also found late
created-identity evidence and read-only-recheck cancellation edges; gated fixtures cover both
accepted corrections.
