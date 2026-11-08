# Pool-wide tunnel and SR-IOV coverage

This increment extends the [host primitives](2026-11-08-network-fabric-primitives.md) with
reviewed pool-wide GRE, VXLAN and SR-IOV creation/removal. It is synthetic-fixture verified;
no real hosts, controllers or NICs were changed. Traffic, physical VF and live-host qualification
remain deferred. P10-11/P10-12 retain the limitations listed below.

## Available workflows

On the selected network, **Complete pool coverage** asks for one explicit eligible input on each
host that lacks an interface. Existing target members remain captured and are preserved. Missing
host selections, duplicate hosts, incompatible hardware/protocol, unknown inventory or changed
coverage refuse the review. All current pool hosts must be live at execution. SR-IOV changes retain
the disabled/evacuated-host, disabled-HA and exact same-driver dependency guards of the primitives.

Creation runs serially, coordinator first when it needs a member. Removal captures every existing
member, removes its exact object/PIF pair and leaves the coordinator until last. A partially covered
network can be removed without pretending that it covered every pool host. The underlying inputs
and target network record remain. No automatic rollback, driver dependency migration or host reboot
is submitted. The review lists step order, preserved members, host/input/controller or hardware
identity and the potential for partial results.

GRE/VXLAN use an already configured controller and qualified transport inputs; no controller or
shell peer setup is inferred. Successful configuration does not establish inter-host traffic.
Controller activity and SR-IOV reboot/mode indicators remain visible separately in coverage.

## Durable step and identity contract

The existing topology worker supplies serial dispatch, expiry, cancellation, receipts and read-only
reconciliation. Its previous VLAN journal records decode unchanged. A new typed fabric operation
and optional baseline identify the new plan shape; legacy rows are never reinterpreted as fabric
operations. Every step has the exact captured host/input and a distinct task UUID. Created object,
PIF and task UUIDs must be absent from the bounded initial baseline, and created identities must
also be distinct across steps.

Before every dispatch, one fresh bounded inventory rereads the exact pool/host set, target network
policy and complete graph. The expected current membership is derived only from the immutable
review and earlier positively verified receipts. All reviewed input/controller/hardware settings
are rechecked, including hosts whose steps are already complete. External members, consumers,
host changes or input changes stop progress. Final completion also rereads the complete expected
graph, including when all receipts were verified before a restart. Recovery never promotes a submitted
topology plan directly from Preparing to success; it performs that fresh check first.

A new pair's UUIDs are retained before broader readback checks. Saved member configuration binds
later steps; a replacement object or interface cannot be silently adopted. Public history and
Activity exports include created/removed stable IDs, while raw references and baselines stay in
the private journal payload. Loading rejects mismatched public steps, missing positive completion,
changed scope, invalid baselines or a submitted suffix without a verified prefix.

A lost, failed or cancelled task stays unknown and retains the pool reservation. Recheck reads only
submitted steps and never repeats a write or starts an unsubmitted suffix. A positively verified
prefix with an untouched suffix is reported as partial; a fresh independent coverage review can
then fill only missing hosts. Positive receipts survive task removal and late replies after recovery
or acknowledgement. A stopped original worker cannot dispatch subsequent hosts.

The final dispatch persist and local publication are synchronous, so backpressure on the UI update
channel cannot suspend a committed dispatch before its RPC. Compare-and-swap still refuses a
recovery or cancellation that changed the row before that final persist. Cancellation requests are
persisted once; recheck never submits a new task cancellation.

## Source evidence

The underlying semantics remain pinned to XCP-ng 8.3 XAPI `v26.1.21` and XenAdmin
`35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`; no upstream implementation was copied.

- [XAPI tunnel implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_tunnel.ml)
  supplies host-scoped GRE/VXLAN objects; the [controller design](https://github.com/xapi-project/xen-api/blob/v26.1.21/doc/content/design/tunnelling.md)
  separates configured access bridges from controller-supplied peers and keys.
- [XAPI SR-IOV implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_network_sriov.ml)
  supplies host-scoped logical interfaces. [XenAdmin CreateSriovAction](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/Network/CreateSriovAction.cs)
  orders pool creation coordinator first. Coordinator-last removal is this client's conservative
  ordering; it is not presented as an additional server API prerequisite.

## Retained work

VF-capacity querying and logical-PIF VLAN lifecycle remain the next SR-IOV increment. Controller
setup, broader driver/guest dependency migration and actual cross-host traffic/physical hardware
qualification remain separate. Dynamic/multiple-address IPv6 management topology migration,
standalone management VLAN retagging/address-family switches and service-dependent transitions
remain tracked; this increment does not close those gaps.

## Verification

The increment adds ten protocol fixtures with protocol/hardware/recovery/corruption subcases,
three Rune tests and a full-publication-channel dispatch/CAS unit regression. The fixtures cover
both tunnel protocols and SR-IOV, creation/removal order, full pool input selection, changed hosts,
offline hosts, controller/PCI/consumer changes, lost/failed/cancelled tasks, saved completion after
task removal, fresh completion of partial coverage, late receipts after recovery/acknowledgement,
read-only admission and six corrupt journal shapes. Rune tests cover compact/wide ordered reviews,
exact selections, stale review invalidation and synthetic Apply refusal.

All 1,198 workspace tests passed: 524 core unit, 333 protocol, 328 desktop and 13
profile/TLS/probe tests. Strict workspace/all-target Clippy, formatting and diff checks passed.
Four synthetic Vulkan scenes were rendered and inspected: selector and captured review at
1440×900 and 513×652. Host choices and footer controls remain reachable; long review content
scrolls. A misleading protocol-error prefix on an unavailable selector input was replaced by
its plain prerequisite explanation, then the three Rune tests and strict Clippy were rerun.

Independent source review found the inherited publication await at the final dispatch boundary;
the synchronous helper and full-channel/CAS regression close it. A separate root review identified
the generic recovery shortcut for all-verified Preparing rows; the final fresh-graph regression
covers both unchanged and externally changed state. No source-review blocker remains. An existing
storage fixture's intermediate-Verifying observation race was fixed independently in prerequisite
commit `f2bec46`; the final complete suite includes that fix. The new form-to-review test also now
advances Rune frames after pointer events before asserting the callback result.

Images and logs remain in local temporary paths. No user screenshots or real host inventory are
included, and these checks do not establish native input/accessibility or physical network behavior.
