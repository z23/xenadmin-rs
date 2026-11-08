# Bulk power and inventory context menus

VM inventory supports Shift/Ctrl multi-selection and a visible selection count. A power
review captures 1–32 visible ordinary guests from one pool, their UUIDs and observed power
states. Selection changes and refreshed table sorting do not retarget an open review.
Changing host scope, page or search clears the table selection and context menu.

All nine existing power actions are offered with per-selection availability. Any unavailable
guest prevents the whole review; the application never silently drops an incompatible guest.
The review lists every captured identity and explains serial execution, stop-on-uncertainty,
pool reservation and the absence of rollback. Force actions require a separate risk
acknowledgement. Submission checks the captured pool/generation and current guest eligibility
again; synthetic inventory cannot submit mutations.

The name cell has a context menu for details and copying the UUID. VM cells additionally
offer properties, devices, snapshots and the existing multi-selection power review. A
single-object action targets the clicked row and clears an unrelated pinned console target.
Non-VM menus omit VM-only operations. Menus currently open from the name cell; whole-row and
keyboard context-menu invocation remain open UX work.

The [backend report](2026-11-08-bulk-power-backend.md) describes durable per-VM receipts,
reservations and read-only recovery. Bulk properties, deletion, migration and export are
separate retained workflows.

## Verification

Three Rune interaction fixtures cover Shift multi-selection, selection persistence through
refresh, immutable review identity, changed-state refusal, synthetic submission refusal,
clicked-row context identity, non-VM menus and the 32-VM bound. Wide (1440×900) and compact
(513×652) review renders and a normal inventory render were inspected using synthetic data.
The source-worktree workspace suite passed 590 tests; strict workspace/all-target Clippy
passed. The integrated count is recorded separately after merging concurrent increments.

No production changes were made. Actual power transitions and native modifier-key behavior
remain for the prepared disposable lab and desktop qualification.
