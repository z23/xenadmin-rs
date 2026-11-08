# Reviewed VM recovery controls

This increment implements a staged task-aware force workflow and HVM recovery boot for
P03-07/P03-08. It uses direct XAPI, the existing operation journal and a Rune review panel.
Production hosts were not contacted or changed. Actual guest recovery and XCP-ng host
compatibility remain disposable-lab qualification work.

## Supported behavior

The VM details panel opens **Recovery controls**. It captures the VM UUID, pool,
connection generation and current host task UUID/operation pairs. Tasks from any client
are shown separately from this client's unfinished journal operations. A local cancellation
request has its own review and targets an exact operation ID/revision. Cancellation may
stop only remaining workflow steps; it neither proves host cancellation nor releases an
unresolved reservation. The operator waits, refreshes, and reviews the force action
separately. There is no automatic cancel-and-force chain or cancellation of other clients'
tasks.

For a running ordinary VM with known current tasks, reviewed hard shutdown/reboot may
proceed even when absent from `allowed_operations`. Fresh preflight resolves every captured
task UUID, rereads the exact VM/current-operation map, preserves administrative blocks,
and calls `VM.assert_operation_valid`. Session write permissions are checked up front.
The server remains authoritative. A changed VM state/task set or known block refuses the
action; preflight refuses an already halted VM instead of using hard reboot as start. Ordinary idle
VMs retain their normal power actions. Existing VM/pool reservation admission is unchanged.

Recovery boot supports halted, idle HVM guests already using `BIOS order` and known BIOS
or UEFI firmware. The captured boot order is changed to lowercase `dn` (CD, then network),
the VM is started once, and the exact original order is restored after verified startup.
An originally absent order key is removed again, rather than replaced with an assumed
default. Existing `dn` requires no configuration write. Firmware, Secure Boot policy,
other boot-map keys and mounted media are preserved. PV conversion, installed-guest
BIOS/UEFI conversion, guest-side recovery execution proof, and automatic recovery-media
selection are not part of this operation.

Every write has a persisted step boundary. Task references/UUIDs, a bounded preexisting
task UUID baseline and completion receipts remain private journal data. Late responses
merge evidence without replacing recovered/acknowledged state or continuing later writes.
Rechecking never starts a pending write, repeats a force action, requests task cancellation,
or restores configuration. A lost response or failed task remains unknown with its
reservation; a received refusal after a verified change reports partial completion.

A separately reviewed **Restore saved boot order** is offered only for a terminal original
recovery with a positive override-setter receipt and an unsubmitted original restoration
step. It requires the exact VM/pool, original configuration and unchanged temporary order.
A dispatched restoration consumes that source claim, including an uncertain restoration;
a successful original recovery cannot later claim another client's matching `dn` value.
The action preserves guest power. If ownership cannot be proved, the operator must inspect
the VM and use ordinary reviewed properties instead.

## Protocol evidence

The [reference force command](https://github.com/xcp-ng/xenadmin/blob/master/XenAdmin/Commands/ForceVMShutDownCommand.cs)
and [VM lifecycle command](https://github.com/xcp-ng/xenadmin/blob/master/XenAdmin/Commands/VMLifeCycleCommand.cs)
allow the advertised-operation exception around this client's running actions and request
cancellation through client history. They do not reset all server tasks. The new client
keeps cancellation and force as separate explicit reviews so local uncertainty and
reservations remain intact.

The primary XAPI [VM lifecycle rules](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vm_lifecycle.ml)
check blocked operations and power state, and permit hard shutdown/reboot alongside some
other current operations. The [VM implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vm.ml)
can start a halted VM from hard reboot; our captured Running-state guard intentionally
prevents that unintended transition in this workflow.

The [reference recovery boot action](https://github.com/xcp-ng/xenadmin/blob/master/XenModel/Actions/VM/HVMBootAction.cs)
temporarily selects CD/network boot. Its model converts display order to lowercase on
wire. In modern XAPI, setting `HVM_boot_policy` also changes `domain_type`, so this
implementation requires an existing HVM configuration and never calls that setter.
Primary target remains XCP-ng 8.3; older host releases have not been qualified here.

## Verification

- Sixteen focused core protocol/journal/permission tests cover force with foreign-client
  tasks, explicit administrative blocks, changed task identities, changed power state,
  retained local reservations, task UUID baseline/rebinding, exact order/absence and
  firmware preservation, unsupported guest modes, received refusal, lost replies,
  recovery/acknowledgment during late replies/verification, restoration provenance/reuse,
  private serialization, corrupt public-step/private-receipt combinations, and upfront
  write permission requirements.
- Four Rune interaction tests cover wide/compact review, captured VM/task identity despite
  selection changes, admin/stale/unsupported state, explicit local cancellation review,
  visible confirmation controls and refusal to submit synthetic mutations.
- The full workspace passed **808 tests**: 365 core unit tests, 223 direct-connection
  tests, 207 desktop tests, five profile tests, four TLS tests and four host-probe tests.
- Strict workspace/all-target Clippy, formatting, diff checks and the workspace build passed.
- Four native Vulkan headless renders (recovery boot and task-aware force, each at
  1440×900 and 513×652) were inspected. The captured target, review text and fixed footer
  controls remain visible; synthetic confirmation stays disabled. Both scenes are in
  the target-aware check script. These renders do not establish native input behavior.
- No live guest power action, native keyboard interaction, actual recovery ISO execution
  or XO-down acceptance run is claimed. Concurrent external host changes can still occur
  after a client preflight; the host validates and executes the final request.
