# Reviewed same-pool VM moves

The desktop captures 1–32 visible ordinary VMs from the current pool and reviews a common destination host with individual disk and NIC mappings. Running VMs migrate live. Halted VMs remain halted and use the existing move workflow, including a reviewed home-host update. The editor retains each selected identity and reports incomplete, stale or unsupported targets; it does not omit them or infer a shutdown.

Disk repositories and NIC networks initially retain their current mappings. Each target exposes its device choices, and the final review lists VM/device UUIDs, old and new storage/network destinations, transfer network and mode. The current generation and the complete review are checked again before submission. Session write permissions are checked before receiver preparation. Existing restrictions on snapshot chains, shared disks, CBT, target visibility and host compatibility still apply.

## Execution and recovery

Moves use the same durable batch parent/child ownership introduced for [bulk deletion](2026-11-08-bulk-vm-deletion.md). Every selected VM has a preallocated child ID and exact typed migration action. The parent holds the pool reservation while children execute serially. Stop completes the current child workflow and skips the remainder; it does not request task cancellation.

Whole-selection preflight only reads current source graphs, destination hosts, storage and network visibility and compatibility. It never calls `host.migrate_receive`. That call prepares transient receiver credentials for the admitted child; it does not establish that a migration was submitted. The child refreshes its captured mappings before and after receiver preparation, journals migration dispatch, then sends the existing migration request once. Receiver values never enter public history or the private journal.

Migration tracking now has a dedicated evidence merge. Positive task replies, task outcome/result and individually verified replacement disk UUIDs survive concurrent recovery or acknowledgement. The immutable plan and its monotonic interruption flag prevent an old worker from continuing while an explicit recheck publishes a newer state. Home-host preparation performs all awaited reads before its final dispatch fence. Recovery never acquires another receiver token, replays migration, starts a remaining target, submits an unsent home-host update, or sends task cancellation/cleanup. Original execution retains its explicit single-VM cancellation behavior.

The existing single-VM home-host reconciliation contract remains: a positively completed migration task plus the exact desired affinity value can establish successful final placement even if the setter reply was lost. The history distinguishes a received setter reply from an observed value; it does not claim the read proves which actor set it. If another operator reaches that value while the batch is interrupted, the already-admitted child may become verified, but the parent remains fenced and cannot start another child. Remaining targets require a new review.

New journal validation binds migration intent, target, device graph, baseline and public step shape, rejects impossible completion without a task receipt, and prevents evidence from being removed or changed. Older migration plans decode with defaults; a recovered unknown migration acquires the interruption fence. Parent/child linkage continues to be validated against the actual child task column and exact action.

## Verification and remaining scope

Synthetic protocol tests cover live and halted serial moves, whole-selection rejection with zero receiver preparation, stop-after-current behavior, lost migration reply, recovery/acknowledgement during task receipt/success/affinity preparation, late replacement disk proof, immutable/corrupt journal plans, legacy payloads, pending read-only recheck and another actor reaching the desired affinity. Existing single-VM migration tests remain in the suite. Rune tests cover compact/wide captured reviews, per-target mappings, selection changes and stale/incomplete target refusal.

The workspace test run passed 1,130 tests: 503 core unit, 299 direct protocol, 315 Rune, five profile, four TLS and four host-probe tests. The final migration-focused run passed 26 cases and the batch-focused run passed 11 groups. Workspace strict Clippy and release all-target checking passed. Synthetic native `batch-move` reviews were rendered at 513×652 and 1440×900 and visually checked for readable identity/mapping details and a visible action footer. These renders use demonstration data.

No actual host migration or user inventory capture was used. Live XCP-ng compatibility, guest/device integrity and native accessibility qualification remain deferred to the disposable lab. Cross-pool bulk moves and bulk archive export remain open; this increment covers same-pool moves only.

## Integrated verification

Main at `86a56ce`, including host monitoring and dedicated VM workflow permission checks, passed 1,162 tests (516 core unit, 311 direct protocol, 322 Rune, five profile, four TLS and four host-probe tests). Workspace strict Clippy and whitespace checks also passed. Migration uses its exact shared requirement mapping in the central operation guard as well as the standalone and batch reviews. No live infrastructure changes were made.
