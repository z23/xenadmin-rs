# Network administration increment

Implemented on the direct XAPI connection, using the durable resource journal:

- Host physical-interface inspection and asynchronous discovery scans.
- Reviewed interface plug/unplug; management interfaces cannot be unplugged here.
- Per-host VLAN creation on an existing network and deletion of unused VLAN interfaces.
- Bonds from two to four unused physical interfaces on one host, with active-backup,
  balance-slb or LACP mode and supported LACP hash choices. Destroying an unused bond
  requires exact master, members and primary-member review.
- Mode/hash changes for non-management bonds.
- Network automatic attachment to new VMs, preserving unrelated configuration keys.
- Network MTU changes with exact current interface membership and management guards.
- Non-management IPv4 configuration and pool IGMP snooping settings.

`Host networking…` is available from host and network Details. Forms capture the reviewed
configuration and identities. The compact dialog retains its review/confirm footer while
content scrolls. Management changes explain their current unavailable state.

## Outcome handling

Each command sends one asynchronous mutation after fresh preflight. The task identity is
persisted before polling. Creation captures bounded pre-existing VLAN, bond and PIF UUIDs;
task success is insufficient without verifying the returned new object's graph. Result
references stay private and persist before graph verification, permitting recovery after a
task disappears. If the initial task reply is lost, matching topology alone is not accepted
as proof: the outcome remains unknown and rechecking does not repeat the write.

Bond creation currently requires members without existing IP configuration, VLAN children
or VM attachments. Management IP handover, pool-wide VLAN/bond orchestration, retagging,
IPv6 editing, interface-purpose configuration, SR-IOV and cross-host internal networking
remain in the retained ledger. These are not implicitly marked complete by this increment.

## Verification

Ten direct-connection protocol tests cover VLAN and LACP dispatch/result identities,
changed interface identity, missing membership, old or wrong creation results, task-read
and post-write verification errors, lost replies/no replay, read-only refusal, IPv4/MTU/IGMP
configuration-key preservation, and refusing VLAN deletion when a VM is newly attached. Two core validation tests cover contiguous IPv4 masks,
address modes and management guards. Two Rune interaction tests cover compact/wide reviews,
captured prior values, reachable controls and refusal to submit synthetic previews.
Workspace strict Clippy passed. The compact synthetic VLAN dialog was rendered and visually
inspected. No production mutations or user screenshots were used.

Live XCP-ng qualification is deferred to the disposable lab; the user requested offline
lab preparation until virtualization becomes available. This increment is fixture verified,
not live-host verified.

## Primary/reference evidence

- [XAPI VLAN methods](https://xapi-project.github.io/xen-api/classes/vlan.html).
- [XAPI Bond modes and methods](https://xapi-project.github.io/xen-api/classes/bond.html).
- [XAPI PIF methods](https://xapi-project.github.io/xen-api/classes/pif.html).
- XenAdmin `XenModel/XenAPI-Extensions/Network.cs` establishes that a missing `automatic`
  key means enabled; `XenModel/Actions/Network/CreateBondAction.cs` and the Bond extension
  establish supported mode/hash values. No reference implementation source was copied.
