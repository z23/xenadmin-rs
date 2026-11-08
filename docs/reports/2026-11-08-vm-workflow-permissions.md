# Permissions before dedicated VM workflows

Snapshot, VM lifecycle and chosen-host power reviews now assess the required write permissions
before submission. Known denial disables confirmation; the backend repeats assessment before
any write. Known preflight denials are journaled as Rejected because execution has not begun.
Unknown discovery remains explicit and defers to the host, while object state,
connection generation and ordinary per-RPC authorization checks remain independent.

Snapshot workflows share one ordered step selection and method mapping between execution and
permission assessment. Snapshot creation includes the description setter; restore includes only
the reviewed shutdown, revert and optional start/resume steps. Editing unchanged metadata does
not introduce an extra setter. Snapshot clone checks conversion to an ordinary VM before the
initial clone when requested. Explicit selected-disk deletion checks the disk destroy permission
before removing its VM record.

Memory-checkpoint deletion additionally requires `VM.hard_shutdown` to discard saved memory.
The generic deletion action does not contain power state, so it reports conditional requirements.
The UI uses its captured checkpoint flag, and the backend resolves the requirement from the fresh
deletion plan before any step. Ordinary disk snapshots do not acquire this power requirement.

VM creation checks its known resource, firmware, BIOS, disk and ISO setters before copying. Its
read-only source preflight resolves clone versus full copy, NIC creation versus a changed-network
move, and inherited CD creation/ejection. These exact requirements are checked again against the
fresh source before copy dispatch. An unchanged inherited network does not require `VIF.move`.
Template conversion and metadata editing use the actual setters; deletion includes only reviewed
disk cleanup. The source-dependent clone result/template conversion and protected-key operations
remain explicitly conditional rather than granting authority or inventing mandatory writes.

The review notice appears beside confirmation controls; snapshot and placement dialogs retain
scrolling, and VM lifecycle keeps its notice/footer reachable in compact windows. Applying through
another UI event still checks permissions; a stale or synthetic target cannot bypass its existing
guards. Recovery observes existing evidence and does not run these checks as permission to replay.

## Verification

Four added core regression groups cover later-step denial with zero host RPCs, optional/unchanged
requirements, fresh checkpoint versus ordinary-snapshot deletion, and source-dependent NIC denial
before copying with an unchanged-network success case. The existing snapshot and lifecycle suites
also pass. Rune review coverage now includes creation and snapshot edit/restore at both compact and
wide sizes, with permission notices reachable and synthetic confirmation unable to submit writes.
Peer review identified the checkpoint discard distinction; the implementation and regression cover
that correction. The final workspace run passed **1,142 tests**: 509 core unit, 304 direct protocol, 316 Rune
and 13 profile/TLS/probe cases. Strict workspace/all-target Clippy, formatting, diff checks
and the desktop build passed. Native synthetic lifecycle reviews at 513×652 and 1440×900
were visually inspected; the notice and fixed confirmation controls remain reachable.
An initial full run caught an older test expecting Failed for power permission denial;
it now checks Rejected to match refusal before execution, while retaining its zero-write assertion.

This increment reuses the effective-session permission semantics documented in the
[initial permissions report](2026-11-08-session-permissions.md) and
[configuration review report](2026-11-08-permission-reviews.md). It does not infer permissions from
role names or automatically elevate a session. No production host or private inventory was used.
Migration and remaining dedicated resource-wizard upfront maps are retained work; actual restricted
accounts and native desktop behavior await the disposable lab.
