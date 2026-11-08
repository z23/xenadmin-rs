# Cross-pool VM move and copy increment

The desktop can review a live move, halted move or halted copy between two connected pools.
It captures the source VM, destination pool/host and explicit storage/NIC mappings, then
uses direct XAPI host-to-host migration. Xen Orchestra and desktop file staging are not
involved. This is an implemented, synthetic-fixture-verified subset; no live hosts were
changed and no live-host compatibility claim is made.

## Supported workflow

Both connections must be current, writable and owned by the same backend. Ordinary HVM
VMs are accepted only with exclusive, owned, writable, persistent user disks, empty CD
drives, no snapshot chain, no snapshot schedule, no vApp membership, no HA protection and
no excluded specialized devices. A running guest can move live. A halted guest can move
or copy; copy leaves the source halted and creates new VM/device UUIDs and MAC addresses.

The three-stage Rune form selects the destination pool/host, maps every disk and NIC and
selects a destination PIF address. Re-selecting a pool clears all mappings and transport
acknowledgement. Host compatibility, exact source graph, source affinity/state, writable
storage visibility/capacity and destination network visibility are checked again before
submission. An unrelated destination identity arriving during preflight invalidates it.
Move refuses pre-existing destination VM/VBD/VIF UUIDs to avoid XAPI restore replacement.

The source console closes when a live move is submitted. Transfer history offers an
explicit destination-console reopen only after verified success and current destination
inventory confirms the exact pool, VM UUID, running state and host. Halted operations offer
Show destination VM. This path does not automatically follow external cross-pool moves.

## Transport boundary

Destination `host.migrate_receive` returns a transient receiver session and advertised
SM/xenops/master endpoints. Only the exact selected host, PIF IP, coordinator IP, fixed
paths, standard ports and matching session query fields are accepted. Unexpected fields,
credentials, fragments, alternate addresses or query parameters are refused. Receiver
credentials and URLs never enter the journal, public history or Activity exports.

The host-generated transport policy is preserved. XAPI may use HTTP or TLS without
certificate verification for host-to-host transfer. Desktop TLS trust does not establish
host-to-host confidentiality or server authentication. The final review requires explicit
acknowledgement of that fact and a trusted migration network; the client does not rewrite
URLs, force compatibility or silently downgrade/retry. Desktop calls to each pool retain
that connection's ordinary certificate policy. Destination coordinator errors cannot
redirect the source connection.

## Durable state and partial outcomes

The existing resource journal reserves both pools transactionally against VM and resource
mutations, in either admission direction. The private immutable plan records bounded
pre-existing destination VM/VDI/VBD/VIF UUID sets, exact graph/shape/mappings and a source
task baseline before dispatch. Credentials remain transient. The reviewed operation,
source pool and destination pool remain in public history; raw references and private
plan evidence do not serialize through the public record. Every journal load/save and
active-reservation check validates the private plan against its public intent, exact
baseline classes, device mapping/cardinality, receipt dependencies and completed-resource
proof. Corrupt or mismatched plans fail closed instead of reconciling another identity.

The worker persists intent before receiver preparation, compatibility checking and the
single `Async.VM.migrate_send` call. Submission/task receipts and positive new destination
identities are retained even if a concurrent recovery/acknowledgement ends the old worker.
A recovered old worker cannot poll onward or submit another write. Startup recovery
publishes cross-pool history without spawning a peer-less generic tracker; only explicit
two-connection reconciliation starts inspection, avoiding a race with destination startup. A successful task's
completion proof permits verification even after the host removes the task record.

Destination VM, disk, VBD and VIF identities/settings/placement are verified before source
disposition is checked. Those destination UUIDs remain recorded when source removal or
copy preservation cannot be established. The client never manually destroys source disks
or adopts unrelated objects merely because they appeared after the baseline. A failed or
cancelled server task can leave partial resources, so its outcome stays unknown and both
reservations remain until explicit acknowledgement. Missing task/result evidence is not
reconstructed by repeating the transfer. Task cleanup occurs only after terminal success
has been durably saved. Unconfirmed host receiver sessions are left to host expiry.

Cancellation requests the existing task's cancellation once; it does not establish that
host work stopped. Local tracking has a two-hour deadline. Recheck uses both currently
connected pools, reads the original receipts and verifies state, without calling receive,
preflight or migrate again. Activity and pool history route cross-pool rechecks through
that two-connection path. Acknowledgement explicitly releases both reservations without
claiming success or deleting resources.

## Verification

Focused protocol fixtures cover the three transfer modes; changed/shared/unowned/mounted
source resources and schedule/HA restrictions; receiver endpoint rejection; destination
identity collisions; explicit submission refusal versus lost/polling/failed-task replies;
wrong result/placement; retained source cleanup; restart after persisted success and
removed task; read-only/foreign-backend/stale observations; cancellation and tracking deadline; and delayed task
receipts arriving after recovery with and without acknowledgement. Journal tests cover
VM/resource admission in both pools and both directions. Receiver validation tests cover
unchanged HTTP/HTTPS policy and malformed endpoints/credentials.

Rune fixtures cover compact/wide review, explicit remapping after destination selection,
transport acknowledgement, halted-copy semantics, stale/shared/attachment refusal and
offline history with destination identities. The final full workspace suite passed 584 tests after the peer-review boundary fix and
startup-tracker race correction; strict all-target workspace Clippy and formatting checks
passed. This includes 10 cross-pool protocol tests, receiver validation, both-direction
admission and corrupted/missing private-plan fixtures, plus six Rune interaction cases. Native Vulkan renders at 1440×900 and 513×652 were inspected; the compact form
scrolls and its final controls are reached in Rune interaction tests. These checks establish
synthetic protocol and desktop behavior, not real host or accessibility qualification.

## Remaining scope

Snapshot chains, shared/unowned/CBT/reset-on-boot disks, mounted media, VMSS/vApp/HA
membership, PV guests and advanced migration topologies remain outside this increment.
They are not marked as completed parity. Broader cross-pool snapshot/media/shared-storage
workflows, externally initiated cross-pool console relocation, IPv6-only host combinations,
server/version qualification and live cancellation/recovery testing remain open. The
excluded specialized hardware/guest categories remain excluded.

## Behavioral references

- [XAPI VM API](https://xapi-project.github.io/xen-api/classes/vm.html)
- [XAPI host API](https://xapi-project.github.io/xen-api/classes/host.html)
- [Migration implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_vm_migrate.ml)
- [Host receiver implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_host.ml)
- [Import restore/copy identity behavior](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/import.ml)
- [XenAdmin cross-pool action](https://github.com/xcp-ng/xenadmin/blob/master/XenModel/Actions/VM/VMCrossPoolMigrateAction.cs)

The older six-argument VM migration call is retained for ordinary guests; later optional
vGPU/VDI-format maps have empty defaults in the versioned XAPI IDL. No reference-client
source was copied.
