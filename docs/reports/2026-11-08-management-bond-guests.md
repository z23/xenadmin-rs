# Guest network interfaces during management bond changes

## Implemented increment

Management bond creation/removal now has a read-only guest-interface impact review. It lists
all NIC consumers on the affected networks, including interfaces the host will retain. Each
row captures the guest and NIC UUIDs, power state, residency or eligible hosts, MAC, device,
attachment state, and original/destination network. The review also shows each network's MTU
and default locking policy: a NIC using `network_default` inherits the destination's policy.

The supported management topology remains static IPv4 over physical interfaces or unique-tag
VLANs on physical/bond interfaces. The host transfers eligible local NICs within the existing
single bond task. The client sends no extra VIF mutations or VM power actions. Active local
VLAN NICs retain their network UUID and are reattached by the host to the recreated bridge.
Remote consumers and stopped guests eligible on other hosts retain their network.

Locality follows the pinned server implementation exactly: a VM resident on the reviewed
host is local; a VM resident elsewhere is not. An explicitly unresident VM is local only
when a successful `VM.get_possible_hosts` response returns that host alone or an empty set.
The empty set means the guest has no eligible host; the review states the server's behavior.
Missing, refused, malformed or incomplete placement evidence never becomes an empty set.
Control-domain interfaces are refused by this ordinary guest workflow.

## Identity, concurrency and recovery

The query is bounded to 1,024 affected interfaces and existing bounded inventory reads,
runs in the connection's discovery budget, and expires after 45 seconds. Closing the review,
changing connection generation or dropping the query cancels it. The UI binds returned data
to the captured form, host and pool before showing confirmation. Read-only connections can
inspect the impact but cannot submit the change.

Dispatch reads the complete affected consumer set again. VM/VIF/network forward and reverse
relationships, guest identity/state/residency, authoritative placement, MAC/device/attachment,
NIC settings and network policies must match the review. Canonical fingerprints retain
configuration equality without exposing raw NIC/network metadata. Set ordering is normalized.
New or changed consumers and changed guest/network settings require another review.

After positive task completion, verification requires the same interface and VM UUIDs,
unchanged guest power/residency and NIC settings, the expected network for every consumer,
and unchanged reviewed network policies. Eligible hosts are not recomputed after a network
move, since that result may legitimately change. Management recovery still requires the
existing certificate-bound host route proof. This verifies reported configuration and the
management route, not guest traffic delivery.

The captured consumer plan is immutable journal intent. Legacy plans without a consumer
review still require networks with no VIFs. Lost responses, partial moves, identity rebounds,
policy changes and other mismatches retain the uncertain outcome and pool reservation.
Recheck reads saved task/topology/consumer evidence and never submits another bond or NIC
change. Restart can finish from a saved task completion receipt even after task removal.
Public history retains the exact reviewed NIC/VM/network UUIDs; no raw references or metadata
are exposed. External clients can still change state between individual XAPI reads and the
host operation; the final checks report mismatches instead of claiming success or rolling back.

## Verification

Synthetic protocol cases cover create/remove, local and remote residents, paused/suspended
guests, empty/single/multiple eligible-host results, unchanged VLAN networks and attachment,
unknown placement and inconsistent graphs, changed consumers/settings/policies, lost replies,
partial moves and rebound IDs, restart without a task, read-only review and connection fencing.
Journal tests cover durable intent, immutability and corrupted destination/locality/policy
scope. Rune tests cover exact moved/retained reviews at compact and wide sizes, reachable
scrolled consumer rows, stale consumer/guest state and query/form/pool fencing.

The worktree full workspace passed 990 tests: 443 core unit, 273 protocol, 261 Rune,
5 profiles, 4 TLS and 4 probe tests. Final focused management checks passed 5 core unit,
40 protocol and 17 Rune tests. Strict workspace/all-target Clippy, formatting and diff checks
passed. Synthetic native Vulkan reviews were rendered and inspected at 1440×900 and
513×652, with a scrolling review and stable confirmation footer at both sizes. A source peer
review and its network-policy follow-up found no blocking issue.

No production host or guest was changed. Live XCP-ng interoperability, traffic continuity
and disposable-lab qualification remain deferred at the user's request.

## Remaining network work

IPv6 management bond handover, dynamic/custom VLAN topology migration, standalone same-IP
physical/VLAN transitions, management retagging, clustering and tunnel/SR-IOV-dependent
migration remain separate retained work. This increment does not close those ledger rows.

## Source basis

- [XAPI v26.1.21 bond implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_bond.ml)
  defines `get_local_vifs`, transfers local physical-network NICs during bond creation/removal,
  and reattaches resident VLAN consumers when a plugged VLAN is recreated.
- [XAPI v26.1.21 VIF implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vif.ml)
  implements `move_internal` without guest unplug/hotplug: the database network changes,
  and an attached, non-suspended interface is moved between host bridges.

No upstream implementation source was copied.
