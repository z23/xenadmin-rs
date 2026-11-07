# XCP-ng Center feature comparison

Reference source: XenAdmin revision `35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`.
This compares implemented client workflows, not a claim that every upstream feature works
on every host version or storage backend. [Feature scope](FEATURES.md) owns the roadmap.

| Area | Reference client implements | Current implementation |
| --- | --- | --- |
| Connection | Direct host/pool connections, saved servers, certificate decisions | Direct XAPI, member/coordinator redirects, endpoint pins, explicit bypass; one remembered endpoint, session-only password |
| Inventory | Full VM, device, host, pool, storage and network views | Basic pools, hosts, VMs/templates, SRs and networks; device details still missing |
| Power | Start on a chosen host, shutdown/reboot, force actions, pause/unpause, suspend/resume | Basic host-placed power actions, pause/unpause and suspend/resume; destination choice missing |
| Snapshots | Create, revert, delete, new VM/template from snapshot, export | 0.2.0: list, disk-snapshot creation, halted-parent restore; deletion, clone/provision, export and additional creation modes remain missing |
| VM configuration | Names/descriptions, CPU, memory, boot and startup settings | Read-only properties; edits remain planned |
| VM devices | Create/attach/detach/resize/move disks, ISO changes, NIC changes | Missing |
| Provisioning | New VM wizard, templates, clones, deletion | Missing |
| Networks | Create/remove networks, VLANs, bonds and host-interface/IP changes | Read-only name and bridge |
| Storage | Create/attach/detach/repair/rescan SRs and manage virtual disks | Read-only type, usage and capacity |
| Migration/transfers | Same/cross-pool migration, import/export | Missing |
| Host/pool management | Maintenance/evacuation, reboot/shutdown, toolstack actions and pool settings | Read-only host details |
| Monitoring and desktop | Metrics, alerts/events, richer task diagnostics, console integrations | Durable task history and basic embedded console; charts, alerts and several console integrations missing |

Snapshot support is explicit in the reference
[create action](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/VM/VMSnapshotCreateAction.cs),
[revert action](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/VM/VMSnapshotRevertAction.cs),
and [delete action](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/VM/VMSnapshotDeleteAction.cs).
Revert restores the existing VM; creating a VM from a snapshot is a separate workflow.
Disk-only, quiesced and memory-checkpoint paths have different host/guest/storage requirements.

Configuration is implemented in the reference's
[VM actions](https://github.com/xcp-ng/xenadmin/tree/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/VM),
[network actions](https://github.com/xcp-ng/xenadmin/tree/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/Network),
and [storage actions](https://github.com/xcp-ng/xenadmin/tree/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/SR).
These use XAPI directly and do not inherently require Xen Orchestra.

Implementation order after the recovery fixes: snapshots; editable VM properties/CPU/memory;
disk/NIC/ISO management; VM creation/templates; then broader network/storage/host workflows
and migration. Live mutation checks require a designated disposable target and scope.

Operator screenshots and their private contents are excluded from this comparison.
