# Management VLAN migration during bond creation and removal

## Implemented increment

The management-bond review now accepts a management VLAN over a physical interface or
an existing bond. It also captures other VLANs on the affected physical members or bond.
Creation requires the management VLAN's physical parent first in the member list; this
makes XAPI's primary-member choice explicit. Removal captures the existing bond's primary
member. The same single host operation transfers the VLANs and management address.

The review lists every recreated VLAN/interface UUID, tag, network, IPv4 configuration and
attachment state. Each tag must be unique across the affected component. Existing networks
are retained; new VLAN and interface UUIDs are identified and shown in operation history.
Unplugged VLANs stay unplugged. The management endpoint retains its static IPv4 address and
must pass the existing certificate-bound recovery connection before success is reported.

Supported VLAN settings are static IPv4 or no IP configuration. VLANs must have the normal
XAPI dummy MAC, no IPv6 configuration, no custom interface properties and empty VLAN/PIF
metadata: the target API recreates these objects and does not preserve those custom fields.
These conditions are rechecked before dispatch, not silently cleared by the desktop.
VIF consumers, clustering, storage/service purposes, protected interfaces, tunnels and
SR-IOV dependencies remain outside this increment's migration review.

## Provenance and recovery

The private management plan adds a bounded pre-existing VLAN UUID baseline alongside its
Bond/PIF baselines. New identities require positive task completion, old object absence,
new UUIDs excluded from those baselines, the exact target parent/host/network/tag and
unambiguous forward/reverse relationships. Creation also requires the positively identified
new bond/master result before VLAN replacements can be adopted.

Each old-to-new VLAN/interface pair is monotonic evidence. Positive pairs are retained
before full configuration verification; a partially migrated graph can therefore remain
uncertain while still identifying verified replacements. A later reference rebound cannot
replace saved identities. Success requires every expected pair and the exact resulting
configuration, including management role, address fields, VLAN attachment state and parent
properties. Missing task records after saved success receipts do not force another write.

Recheck remains read-only. Failed tasks, lost replies and partial/mismatched graphs retain
the pool reservation until reconciled or explicitly acknowledged. No automatic creation,
removal, IP reassignment or rollback occurs. Old journal entries for plain bonds remain
readable without a VLAN baseline. Raw references and private baselines remain outside
public serialization; public history exports only typed replacement UUIDs.

## Verification

The worktree workspace run passed 936 tests: 414 core unit, 265 protocol, 244 Rune desktop,
5 connection-profile, 4 TLS and 4 probe tests. New cases cover creation/removal with a
management VLAN and an unplugged sibling, metadata/property/consumer/tag refusal, partial
replacement retention, restart without a task, rebound identity refusal, private journal
corruption and compact/wide captured reviews. Final focused management checks passed 4 core
unit, 33 protocol and 14 Rune tests. Strict workspace/all-target Clippy, formatting and diff
checks passed. Synthetic native Vulkan reviews were rendered and inspected at 1440×900 and
513×652; the scrolling review and confirmation footer remained usable at both sizes.

A source peer review found no blocking issue in the reviewed component, provenance and
recovery paths. No production host, switch or guest was changed. Live XCP-ng interoperability,
traffic continuity and the agreed disposable-lab qualification remain deferred.

## Remaining topology work

Local guest-interface migration, IPv6 management topology handover, dynamic VLAN addressing,
custom VLAN metadata/property preservation and arbitrary same-IP physical/VLAN transitions
remain retained work. This increment does not claim that those server-supported workflows
are unsupported. Cross-host orchestration and other network features remain separately
tracked in the parity ledger.

## Primary source basis

- [XAPI v26.1.21 bond implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_bond.ml)
  recreates VLANs during bond changes. Unique tags preserve each network; a duplicate tag can
  select an existing VLAN and move local VIFs to its network. IPv4 configuration is copied,
  old records are destroyed, and the old attachment/management state is restored.
- [XAPI v26.1.21 VLAN implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vlan.ml)
  assigns new UUIDs, the fixed dummy MAC, parent device/MTU/family and empty metadata/properties.
  It does not preserve arbitrary old interface attributes. No upstream source was copied.
