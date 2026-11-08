# Pool-wide VLAN topology increment

The Linux client can create a VLAN interface on every host in the connected pool and
retag an unused VLAN network through the Host networking view. The review captures the
network, every host/uplink identity, old VLAN and child interface UUIDs, tag and attachment
state. Tag 0 is supported by the native XAPI VLAN API; tags 1–4094 remain available.

Creation requires an empty target network and one managed untagged uplink on the same
underlay network for every current pool host. Every host must be online. Retagging requires
an exact VLAN interface set, no VM attachments, no management/IP configuration, no purpose
or custom metadata, no unplug protection and no nested interface consumers. Each old VLAN
is deleted and replaced on its host; an originally unplugged interface is unplugged again
after creation. No automatic rollback or network-wide atomicity is claimed.

Before each write, the worker checks the pool UUID, exact host coverage, live host state,
network membership and relevant interface configuration again. It records intent before
submission and records each task result before verification. New VLAN/PIF UUIDs must be
absent from the original bounded baseline and remain bound to the saved result throughout
the operation. A reference rebound to a different new object cannot redirect a later
unplug. Detaching a replacement repeats the full configuration and topology guard.

A lost reply or failed server task keeps the result uncertain and the pool reserved.
Late positive receipts survive restart recovery and acknowledgement, without reviving the
original worker. Recheck only inspects dispatched work: it never creates/deletes another
interface, starts remaining steps, or dispatches a previously pending cancellation request.
A previously recorded successful task can be verified even after the task record disappears.
Unsubmitted steps produce an explicit partial outcome. Created and removed VLAN/PIF UUIDs
are visible in resource history and Activity exports; opaque references remain private to
the local journal. Known owned tasks are cleaned after terminal persistence under one
five-second cleanup deadline.

## Verification

All fixtures use synthetic loopback hosts and generated identities. No production host
operations, credentials, inventory or user screenshots were used.

- Twelve topology protocol cases cover two-host create/retag, native tag 0, full host and
  pool identity coverage, offline/changed members, VM consumers, partial completion,
  lost/failed/misbound replies, restart with removed task records, protection/IP/topology
  changes, late receipt recovery/acknowledgement, replacement identity changes before
  unplug, pending cancellation intent during read-only recheck, and expiry before remaining host steps.
- A journal corruption fixture checks private plan/intent identity, missing baselines,
  changed steps, invalid completion/result evidence and premature success. Recovery and
  conflicting admission fail closed.
- Rune review tests cover compact/wide layouts, captured identities, changed pool and
  interface graph, incomplete inventory, protection, tag validation, preview mutation
  refusal and public created/removed identity exports.

The full workspace suite passed 710 tests (324 core unit, 199 protocol, 5 profile, 4 TLS,
4 CLI and 174 Rune cases). Final focused checks also cover the added expiry case and
exact-pool/result guards. Strict workspace Clippy, formatting and diff checks pass.
Native Vulkan synthetic reviews were rendered and inspected at 1440×900 and 513×652.
Live XCP-ng compatibility and disruption/recovery qualification remain open.

## Remaining network scope

Management interface/IP handover with recovery endpoints, management/bond/VLAN topology
migration, SR-IOV capability-qualified networking and cross-host internal tunnels remain
separate workflows. This increment does not retag a network carrying management, storage
IP traffic or VM attachments. Nonstandard management MTU remains unsupported by XCP-ng.

## Behavioral sources

- [XAPI VLAN.create implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_vlan.ml)
  validates tags 0–4094, creates the VLAN/PIF records, and plugs the resulting interface.
  An error can therefore follow partial creation, so failed tasks remain uncertain.
- [XAPI pool API](https://xapi-project.github.io/xen-api/classes/pool.html) documents the
  pool-wide VLAN API and the same tag range. This client uses separately journaled host
  steps to retain exact partial results.
- [XCP-ng networking documentation](https://docs.xcp-ng.org/networking/) describes VLAN,
  bond and management-network constraints, including unsupported management jumbo MTU.

The Rust implementation is original; no upstream source was copied.
