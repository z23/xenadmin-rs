# Bulk VM and custom-template property editor

The desktop adds **Edit selection…** to VM and template selection bars and to the existing
whole-row/keyboard context menu. Between 1 and 32 visible targets from one pool can be
captured. One common property is chosen per review: name, description, home-host preference,
halted vCPU counts, halted memory limits, live vCPU count, live dynamic memory, topology,
CPU weight, shadow multiplier, boot order, autostart, empty-template firmware or Secure Boot
policy. Common names are literal; no automatic numbering or template interpolation occurs.

The review shows every captured target's name, UUID, power state and old → new values.
Targets already at the requested value remain listed as **Already desired — excluded from
changes**. All-no-op batches cannot be submitted. Missing data, unknown states, built-in
or unknown template types and unsupported combinations produce named per-target reasons
and block the whole review; they are not silently dropped. The property builder shares
memory parsing and change summaries with the single-target editor.

Halted/live limits, advertised live capabilities and explicit blocks are checked before
review. Firmware requires a halted, empty, custom HVM template with complete VBD inventory
and disabled Secure Boot; installed guests cannot switch firmware. Secure Boot changes
require halted UEFI targets, preserve certificates and make no enforcement claim. The core
checks enabling support again before writing. Home-host preference never implies migration.

Reviewed commands are immutable. Submission re-observes the pool generation, checks every
captured identity/state, reconstructs the exact changed and unchanged rows from current
fields and refuses any mismatch. Demo, read-only/stale connections, known RBAC denials and
the active-operation limit cannot submit. Changing the table selection after review cannot
retarget it. Context menu callbacks separately compare their captured selection and scope.

The existing resource-operation history shows the core's per-target/setter progress.
Changes run serially, stop on failure/uncertainty/cancellation and retain already completed
changes; there is no automatic rollback. Explicit reconciliation does not send pending
setters. The private journal stores immutable target patches and monotonic dispatch, positive-reply and value-verification receipts. Whole-selection preflight happens before the first write; each setter repeats fresh identity, state, field and permission checks. Missing replies stay unknown even when values match, and late evidence cannot resume an interrupted worker. The local inspector exposes target UUIDs and setter counts without property values.

Six new synthetic desktop test groups passed, covering all property edit types, mixed
no-op/changed selections, all-no-op refusal, missing/unknown/unsupported targets, live
limits and advertised capabilities, immutable/stale review, compact/wide footer reachability,
custom-template entry and keyboard context-menu selection fencing. Independent source review
found no blocking issue. The complete desktop suite passed all **284 tests**. Strict desktop all-target Clippy,
release-mode all-target checking, debug build, owned-file formatting and diff checks passed.
Native Vulkan review renders at 513×652 and 1440×900 were visually inspected; per-target
values and the permission/action footer are readable and reachable. The compact underlying
inventory emits its pre-existing zero-height viewport warning while the modal is open; the
modal itself is bounded and scrollable. Real compositor and host qualification remain open.
No live host or user screenshot was used.


The core suite passed 458 unit tests, 278 direct-protocol tests, 5 profile tests and 4 TLS
tests at this branch baseline. Seven new bulk-property groups include whole-selection and
RBAC refusals, later target/field changes, lost replies despite matching values, cancellation,
late receipt/recheck races, forged journal state and immutable plan validation. Strict core
all-target Clippy passed. Full integration verification is recorded separately after merge.

This increment implements bulk properties in P15-06. Bulk deletion, movement and export
remain separate retained workflows. There is no live-host qualification claim.
