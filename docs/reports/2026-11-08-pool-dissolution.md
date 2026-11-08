# Coordinated pool dissolution

The pool administration dialog now offers an explicit review of the current pool,
its retained coordinator and every member to eject, followed by single-host pool
metadata clearing. Confirmation requires the exact pool UUID. This completes the
coordinated workflow retained in P13-01; live qualification is still deferred.

## Reference behavior and boundaries

The local XenAdmin reference at `35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`
(`XenAdmin/Commands/DeletePoolCommand.cs` and
`XenModel/Actions/Pool/DestroyPoolAction.cs`) requires a single-host pool with HA
disabled, then clears its name and description. It asks the operator to eject
members first. The coordinated sequence here combines those separately reviewed
operations; it does not claim the reference automatically ejects a multi-host pool.

The captured coordinator is never ejected. Members must already be disabled,
evacuated and free of local guest-disk data. The workflow does not disable HA,
evacuate guests, delete guest disks or force-remove offline members. XAPI ejection
reboots the selected member and resets its pool configuration; the review states
those effects. Independent connectivity after reboot needs separate verification.

## Admission and recovery

Before any write, the backend checks the exact pool/coordinator/member identities,
name, description and HA state, all member prerequisites, and all known later
setter permissions. It captures the existing task UUIDs. Each ejection rechecks
the remaining host set and the selected host; metadata changes only follow
verified member ejections. A new or missing member, coordinator change, enabled
HA or changed reviewed metadata stops the sequence.

Each ejection requires a newly returned task reference, a non-baseline task UUID,
positive completion and exact expected remaining membership. Disappearance alone
does not prove success. Metadata setters need positive replies as well as matching
readback. Private references stay in the journal; public results expose only
verified ejected host UUIDs and metadata completion flags.

Stop allows an already submitted ejection to settle, then skips later changes.
Recovery inspects submitted steps without replaying writes, cancelling tasks or
cleaning task records. A proven partial result can release the pool reservation
for a separately reviewed remaining sequence; uncertain outcomes retain it.
Late receipts can enrich evidence without reversing interruption or acknowledgment.
Normal completion cleans only task records whose stored UUID still matches the
owned, ended task. It never removes other clients' task records.

Peer review found and corrected two race conditions. A bounded UI update queue
cannot insert an await between the final dispatch CAS and RPC submission. Also,
restart recovery cannot promote previously verified steps directly to success:
even an entirely verified interrupted plan must reread current pool membership,
coordinator, HA and metadata before completing. Lost replies remain uncertain.

## Verification

Eleven core fixture groups exercise standalone and multi-member success,
whole-selection refusal, later host/membership changes, lost replies, failed and
pending tasks, pre-existing/reused/rebound task identities, monotonic journal
evidence, late receipts after stop/recovery/acknowledgment, a denied final setter,
backpressured publication, and fresh final-state checking after restart.
Two Rune groups cover captured identities, prerequisite refusal, synthetic/stale
submission guards and compact/wide confirmation-control reachability.

The isolated branch passed all 1,180 workspace tests: 531 core unit, 312 direct
protocol, 324 Rune, five profile, four TLS and four probe tests. Strict all-target
Clippy, formatting and whitespace checks passed. The final fixed confirmation
footer passed both Rune tests again and native Vulkan renders at 513×652 and
1440×900 were visually inspected. The exact UUID field, permission notice and
buttons remain visible while the member review scrolls. This does not establish
real compositor, accessibility or live pool behavior.

One full run also exposed the existing storage fixture reading an intermediate
Verifying publication; the separate fixture-only correction waits for completed
evidence and retains the exact OutcomeUnknown assertion. The rerun above passed.

No live host was accessed or changed by these fixtures.
