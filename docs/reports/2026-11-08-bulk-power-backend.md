# Bulk VM power backend

The core accepts one typed `VmPowerBatch` through the existing observed-pool resource
command. A selection contains one existing power action and 1–32 distinct ordinary guest
UUIDs with their reviewed power states. Unknown states, incompatible actions, non-pool
targets and duplicate identities are refused. The public connection facade and actor retain
the existing read-only, current-generation and pool identity checks.

The entire selection is checked before the first write. Each guest is then resolved and
checked again immediately before its serial dispatch: UUID, ordinary guest kind, exact
reviewed state, advertised operation, blocked operations, current tasks and the host's
`VM.assert_operation_valid`. Start/resume use `start_paused=false` and `force=false`.
The existing [XAPI VM API](https://xapi-project.github.io/xen-api/classes/vm.html) defines
these methods and the assertion. No force fallback, placement, migration or guest shutdown
is added implicitly.

The batch reserves the whole pool against other VM and resource writes in both directions.
Each item has a durable dispatch marker, task receipt, task-success proof and verified
resulting power state. Public steps identify the individual VM by UUID. The private plan
is omitted from public serialization and inspection; the inspector exposes the typed
selection and per-VM steps, including when history is filtered to one selected VM.
Private-plan decoding checks the exact action/selection, bounded length, dispatch order
and evidence consistency before reservation decisions. Journal updates cannot change
reviewed identities or remove/replace existing task receipts.

A saved dispatch CAS immediately precedes each send. A lost reply or failed task stops the
batch with an unknown outcome and retains the pool reservation. Matching VM state alone
cannot replace a missing task receipt; reboot requires task-success proof even if the VM
is running. A later refused guest leaves a verified subset and untouched pending items as
partially completed. There is no compensation.

Stop requests let the current host task finish, then skip remaining guests. The worker does
not send `task.cancel`. Connection interruption stops local tracking and leaves dispatched
work uncertain. Recovery reads saved tasks and guest state only; it never submits pending
items. Late receipts/completion/verification can extend saved evidence after recovery or
acknowledgement, but cannot resume the original worker. Verified owned tasks are cleaned up
only after a terminal outcome is durable, within one three-second best-effort deadline;
unknown or acknowledged-unknown tasks remain available for inspection.

## Verification

Synthetic loopback fixtures cover ordered dispatch, whole-selection preflight refusal,
changed/blocked second guests, lost replies despite matching VM state, failed tasks,
incorrect resulting state, stop before/after the current task, and recovery/acknowledgement
during the asynchronous response, task-success response and final VM verification. They
assert no second dispatch during recovery, private receipt omission, retained reservations
and append-only plan evidence. Journal fixtures cover whole-pool conflicts in both
directions and malformed private-plan refusal. A public connection fixture covers read-only
and old-generation submission refusal.

All 453 core tests passed (290 unit, 154 direct protocol, five profiles and four TLS),
including the nine focused bulk/journal tests and public connection refusal fixture.
Strict workspace/all-target Clippy, formatting and diff checks passed. Desktop integration, interaction checks and native synthetic
renders are delivered separately. No production host or real guest was contacted. Actual
XCP-ng compatibility, power transitions and RBAC behavior remain for the prepared disposable
lab; the user has deferred live qualification. Cross-pool batches and bulk delete, properties,
move and export remain separate retained workflows.
