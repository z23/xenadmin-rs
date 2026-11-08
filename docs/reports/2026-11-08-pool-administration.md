# Pool policies, availability and membership

This increment implements reviewed direct-XAPI pool operations in the Rune desktop.
It is fixture verified, not live-host qualified. The user has deferred infrastructure
tests to the prepared local lab. No production hosts or real credentials were used.

## Implemented workflows

Open **Manage pool → Pool policies, HA and membership**. The workspace captures
its pool and review values; navigating the main inventory cannot retarget it.
Connection generation changes invalidate submission. Every mutation uses the
existing durable resource history and the pool-wide write reservation.

- Migration compression and HTTPS-only policy edits use observed before values.
- Planned coordinator handover checks the current coordinator, destination liveness
  and enabled state. HA must already be disabled. No forced failover is offered.
- HA enable accepts explicit shared heartbeat SRs. It checks each SR with
  `SR.assert_can_host_ha_statefile`, verifies every member has an attached PBD, and
  asks the server whether a failover plan exists for the configured tolerance.
- Failure tolerance is a separate reviewed edit; disabling HA is explicit and
  requires entering the pool UUID. These changes are never automatic prerequisites.
- VM restart priorities support restart, best effort and disabled. Restart requires
  `VM.assert_agile`. An empty priority is the known disabled value, not missing data.
- Member ejection requires an exact host UUID confirmation, a disabled live member,
  no resident/scheduled guests and no guest disks in its local user SRs. It refuses
  the coordinator and HA-enabled pools. The review explains the host reboot/reset.
- TLS verification enable distinguishes pool-internal certificates from desktop
  trust. Pool-secret rotation requires disabled HA and reachable members. No secret
  value is retrieved or serialized.

Status includes the coordinator, HA enabled state, configured and planned failure
tolerance, overcommit, heartbeat repositories and advertised policy fields. Unknown
fields remain unknown. Controls are absent for unavailable policy fields.

## Recovery and verification

Synchronous replies and successful asynchronous tasks create a durable monotonic
completion receipt before verification. Saved receipts allow read-only rechecks
after verification failure without resubmission. Lost dispatch replies and missing
task identities stay unknown even if a desired field already matches. Failed or
cancelled pool tasks also stay unknown: partial changes may have occurred. Their
task evidence and pool reservation are retained until explicit operator resolution.

Tests cover the policy methods and argument shapes, HA heartbeat checks and final
VDI/SR identity, handover/ejection, fresh conflicts, detached heartbeat storage,
occupied/local-data hosts, HA protection, read-only connections, lost replies,
failed/cancelled tasks, secret-safe history, receipt durability and late controls.
Rune fixtures cover compact/wide review footers and immutable targets. Native
synthetic renders are inspected separately; they do not qualify Wayland input,
clipboard, accessibility or infrastructure compatibility.

Validation: 337 workspace tests passed (4 probe, 163 core unit, 5 connection
profiles, 78 direct RPC, 4 TLS and 83 desktop). Strict workspace/all-target Clippy,
formatting and diff whitespace checks passed. Wide pool-policy and compact HA-review
native synthetic scenes were rendered and visually inspected. These files remain
under `/tmp/xenadmin-pool-review-artifacts/`, outside the repository.

## Remaining P13 work

Joining pools needs transient destination credentials, independent destination
identity/trust review and coordinated ownership transition. It is not implemented
by accepting passwords in serializable resource actions. Directory join/leave,
RBAC subject/role editing, reconnect-as/elevation, host certificate install/reset,
advanced HA timeout/order/delay and coordinated dissolve remain follow-up work.
No generic raw RPC, credential/configuration-map editor or forced offline removal
is substituted for these workflows.

Pool clustering is unsupported on XCP-ng because its upstream implementation uses
unavailable proprietary components; the [XCP-ng upgrade guide](https://docs.xcp-ng.org/installation/upgrade/)
explicitly requires it to be disabled before migration. Legacy SSL and TLS 1.0/1.1
were removed in [XCP-ng 8.2](https://docs.xcp-ng.org/releases/release-8-2/).
GPU placement belongs to the user's excluded specialized hardware category.

## API and reference evidence

The implementation uses the signatures and semantics in the primary
[pool API](https://xapi-project.github.io/xen-api/classes/pool.html),
[SR heartbeat assertion](https://xapi-project.github.io/xen-api/classes/sr.html)
and [VM API](https://xapi-project.github.io/xen-api/classes/vm.html).
The current [XCP-ng host/pool guide](https://docs.xcp-ng.org/management/hosts-pools/)
documents single-host pools, migration compression, HTTPS-only access and pool
secret rotation. [HA guidance](https://docs.xcp-ng.org/management/ha/) documents
heartbeat storage, restart policies and fencing behavior. The
[XCP-ng certificate guide](https://docs.xcp-ng.org/guides/TLS-certificates-xcpng/)
establishes the 8.3 pool-verification requirement.

The upstream [pool implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_pool.ml)
establishes ejection/reset and HA handover restrictions. The
[secret-rotation implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_psr.ml)
returns after rotation/cleanup; a successful task and cleared pending flag together
are therefore useful completion evidence. The flag alone never establishes rotation.

Reference XenAdmin `EnableHAAction`, `EjectHostFromPoolAction`, `EvacuateHostAction`,
`PoolAdvancedEditPage` and generated Pool/VM method declarations were consulted for
behavior and synchronous/asynchronous call availability. No source was copied.
