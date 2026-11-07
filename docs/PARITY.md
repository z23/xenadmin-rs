# XenAdmin / XCP-ng Center parity audit

Audit date: 2026-11-08. Rust baseline: 0.2.0 (`2755a5b`) plus the 0.2.1 Operations
visibility fix. Reference: [xcp-ng/xenadmin revision
35dc091c5b62ab3b22449bb7a89cfcf84a6600d4][reference].

The application is a direct-host recovery alpha. Direct connection, inventory, basic
power commands, an embedded VM console, and basic snapshots exist. VM configuration,
device editing, provisioning, migration, and host/network/storage administration remain
largely missing.

This is a workflow-level catalogue from the reference's commands, context menus, property
pages, wizards, tabs, console controls and action families. Duplicate menu entries and
interaction variants are grouped. It is not a list of every XAPI method or individual
API field. Reference code does not establish support on every XCP-ng version, guest,
storage driver or license. Host-dependent and legacy features are included explicitly.

This audit does not expand the agreed release scope. [FEATURES.md](FEATURES.md) owns scope;
[PLAN.md](PLAN.md) owns sequencing and acceptance gates.

## Status and priority

- **Implemented:** the narrow stated workflow has code/local verification; not a claim of live-host qualification.
- **Partial:** a usable subset exists; the limitation is stated.
- **Missing:** no end-to-end client workflow. A parsed capability or enum does not count.
- **Unqualified:** real-system acceptance remains outstanding.

Priorities rank remaining work: **P1** outage recovery; **P2** routine VM administration;
**P3** infrastructure administration/monitoring; **P4** broader convenience and specialized
features. **Assess** means the reference contains it, but applicability/inclusion needs a
decision. **—** means no feature work identified for that narrow row; qualification may
still be outstanding. These are not severity labels or delivery commitments.

## 1. Connections, credentials and trust

Reference: `XenModel/Network/XenConnection.cs`, `XenAdmin/Dialogs/Network`,
`XenAdmin/Dialogs/RestoreSession`, `XenAdmin/Dialogs/OptionsPages/ConnectionOptionsPage.cs`.
Rust: [connection.rs](../crates/xenadmin-core/src/connection.rs),
[trust.rs](../crates/xenadmin-core/src/trust.rs),
[preferences.rs](../crates/xenadmin-desktop/src/preferences.rs).

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Direct host/pool login and logout without XO | Implemented | JSON-RPC; requires reachable XAPI. | — |
| Connect through a non-coordinator member | Implemented | Follows redirect; retains configured endpoint pins. | — |
| Session renewal and reconnect | Partial | Renewal/redirect handling exists; not a qualified failover implementation. | P1 |
| Try remembered verified pool members after endpoint failure | Missing | No alternative-member failover. | P1 |
| Multiple saved connection profiles | Missing | Only last endpoint/username remembered. | P1 |
| Simultaneous pools and unified inventory | Missing | One active desktop connection. | P1 |
| Deduplicate aliases/member endpoints for the same pool | Missing | No shared connection ownership. | P1 |
| Connect/disconnect all; remove profiles; reconnect as another user | Partial | Single connect/disconnect exists; profile/bulk workflows do not. | P2 |
| Session-only password | Implemented | Not persisted in preferences. | — |
| Save/forget credentials and protected session restore | Missing | No Linux keyring integration/profile-session restore. | P2 |
| System CA/name verification and endpoint pins | Implemented | Manual primary/additional-host pins cover redirects/consoles. | — |
| Desktop custom-CA file selection | Partial | Supported by core/probe; no desktop file chooser. | P2 |
| Inspect and accept unknown certificates | Missing | No certificate inspection/trust dialog; manual pins only. | P1 |
| Review changed certificate and replace trust | Partial | Validation can reject changes; no old/new comparison/retrust flow. | P1 |
| Easy certificate-validation opt-out | Implemented | Member/coordinator/console scope; visible status; editing endpoint clears choice. | — |
| Proxy/timeout options and richer connection diagnostics | Missing | No connection-options center. | P4 |

## 2. Inventory, navigation and inspection

Reference: `XenAdmin/TabPages`, `XenAdmin/Controls/XenSearch`, `XenAdmin/SettingsPanels`.
Rust: [model.rs](../crates/xenadmin-core/src/model.rs),
[xapi.rs](../crates/xenadmin-core/src/xapi.rs),
[desktop](../crates/xenadmin-desktop/src/main.rs).

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Pool/host/VM tree; VM and template tables | Implemented | Basic hierarchy/stable identities. | — |
| Network and SR tables | Partial | Minimal properties; not connectivity/device views. | P2 |
| Search by name/UUID/host; sort and virtualized rows | Implemented | Basic navigation, not a query builder. | — |
| Persist sidebar/table column widths | Implemented | Local preferences. | — |
| Full VM general information | Partial | Basic identity/state/location/CPU/static max memory; missing guest OS, IPs, tools/driver state, uptime and runtime detail. | P1 |
| Full host/pool properties | Partial | Basic host identity/state/address/version; missing hardware/resource/capability/health detail. | P2 |
| VM disk/NIC and host/storage attachment graph | Missing | No VDI/VBD/VIF/PIF/PBD inventory and relationship views. | P2 |
| Physical NIC/link/bond/VLAN inspection | Missing | No link speed/state or management-interface detail. | P2 |
| SR attachment/path and disk inspection | Missing | No per-host connectivity or virtual-disk browser. | P2 |
| Live inventory/stale-state handling | Partial | Event-driven five-class inventory; malformed batches require reconnect rather than automatic resync; backoff refinement missing. | P1 |
| Filtered/grouped/saved/cross-pool searches | Missing | No query builder or search import/export. | P4 |
| Multi-object actions and context menus | Missing | No end-to-end bulk review/mutation workflow. | P3 |

## 3. VM power and recovery

Reference: `XenAdmin/Commands`, `XenModel/Actions/VM`.
Rust: [operations.rs](../crates/xenadmin-core/src/operations.rs).

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Start, clean shutdown and reboot | Implemented | Start uses host-selected placement. | — |
| Force shutdown/reboot | Partial | Confirmed ordinary force actions; task-blocked recovery below is missing. | P1 |
| Pause/unpause and suspend/resume | Implemented | All four now exist; no longer parity gaps. | — |
| Start/resume on a chosen eligible host/home server | Missing | No picker, start_on or resume_on. | P1 |
| Explain unavailable actions | Partial | 0.2.1 shows disabled buttons and empty-availability/stale explanation; not exact host reasons. | P1 |
| Show current tasks and administrative action blocks | Missing | No current_operations/blocked_operations view or per-action validity diagnosis. | P1 |
| Reviewed force recovery with running tasks | Missing | Reference warns and cancels tasks before forcing a Running VM; Rust has no equivalent. | P1 |
| Recovery-mode boot | Missing | Reference temporarily changes HVM boot policy/order. | P2 |
| Batch VM power actions | Missing | No per-target eligibility/progress/partial results. | P3 |

An empty allowed-operation list does not establish a particular fault. XAPI describes
`allowed_operations` as advisory; `current_operations`, `blocked_operations` and
`assert_operation_valid` provide different evidence. See the [VM API][vm-api]. The
[reference force-shutdown path][force] needs a separately reviewed recovery flow, not
blanket enablement of force buttons. Recovery-mode boot is a separate [reference action][boot].

## 4. Consoles

Reference: `XenAdmin/ConsoleView`, `XenAdmin/Controls/ConsolePanel.cs`,
`XenAdmin/Controls/ConsoleTab`.
Rust: [console.rs](../crates/xenadmin-core/src/console.rs),
[console_view.rs](../crates/xenadmin-desktop/src/console_view.rs).

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Embedded direct authenticated RFB console | Implemented | Tunnel/framebuffer; operator has reported real use. | — |
| Keyboard and basic pointer input | Partial | Keys, left drag, right click, held-key release exist; full coverage unqualified. | P1 |
| Ctrl+Alt+Delete and deliberate special-key controls | Missing | No toolbar/menu. | P1 |
| Mouse wheel/middle button | Missing | Not mapped by desktop. | P1 |
| Extended QEMU keys, layouts and shortcut capture | Partial | Basic keysyms; extended/non-US behavior incomplete/unqualified. | P1 |
| Controlled clipboard send/receive | Missing | No guest clipboard workflow. | P1 |
| Reopen after reboot/migration/session replacement | Missing | Manual reopen; no relocation-aware recovery. | P1 |
| Keep console target aligned with Details/actions | Partial | Selecting another VM can update Details while old console remains open. | P1 |
| Scaling, framebuffer resize and cursor behavior | Partial | Fit scaling/coordinate mapping exist; richer controls and live qualification missing. | P2 |
| Full-screen, detached and multiple console windows | Missing | One embedded view. | P2 |
| Save console screenshot | Missing | No operator export control. | P4 |
| Host text console; guest SSH/RDP/external viewer | Missing | No integrations; Windows RDP embedding is not a Linux port target. | P3 |

## 5. Snapshots

Reference: `XenAdmin/TabPages/SnapshotsPage.cs`, `XenModel/Actions/VM`,
`XenModel/Actions/VMSS`, `XenAdmin/Wizards/NewPolicyWizard`.
Rust: [snapshot UI](../crates/xenadmin-desktop/src/vm_snapshots.rs),
[verification report](reports/2026-11-07-snapshots.md).

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| List per-VM snapshots and basic details | Implemented | Typed snapshot identities/parent index; flat list. | — |
| Create named disk snapshot | Implemented | Journaled task and returned-object verification. | — |
| Restore to existing VM | Partial | Parent must already be Halted; identity/relationship/capability checked. | P2 |
| Shutdown-before/boot-or-resume-after restore | Missing | No combined power-state orchestration. | P2 |
| Disk-and-memory checkpoint creation | Missing | No option. | P2 |
| Guest-quiesced snapshot creation | Missing | No option; host/guest support requires qualification. | P2 |
| Snapshot description at creation; edit name/description | Missing | Name-only creation; no property editor. | P2 |
| Delete snapshots with owned-disk cleanup | Missing | Parsed Delete capability is not a workflow. | P2 |
| New VM from snapshot | Missing | No clone/provision workflow. | P2 |
| New template from snapshot | Missing | Separate from restore; absent. | P2 |
| Export snapshot as template/backup | Missing | No transfers. | P3 |
| Ancestry tree, preview image, storage/size detail | Missing | Flat list only. | P4 |
| Scheduled-snapshot policies | Missing | No create/edit/delete, frequency/retention, assignment, enable/disable, run-now; assess host VMSS support first. | Assess |

## 6. VM properties/resources

Reference: `XenAdmin/SettingsPanels`, `XenModel/Actions/VM`,
`XenAdmin/Wizards/NewVMWizard`, `XenAdmin/Wizards/BootModesControl.cs`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Rename VM/template and description | Missing | Read-only properties. | P2 |
| Startup/max/live vCPU counts | Missing | No CPU editor/hot-change handling. | P2 |
| CPU topology and scheduling weight | Missing | No cores-per-socket/weight controls. | P2 |
| Static/dynamic memory limits, ballooning/live changes | Missing | Static max displayed; no editor. | P2 |
| Boot order, bootable devices, autostart | Missing | No startup editor. | P2 |
| Home server/host affinity | Missing | No placement preference editor. | P2 |
| VM HA restart priority/protection | Missing | No configure/unprotect workflow. | P3 |
| BIOS/UEFI/Secure Boot provisioning choices | Missing | No wizard; reference gates choices by version/capability. | P2 |
| HVM shadow-memory settings | Missing | No advanced editor. | P4 |
| Cloud-config parameters/config drive | Missing | No template-dependent configuration. | P4 |
| Guest tools status/installation media | Missing | No tools panel or Install PV Tools media action. | P2 |

## 7. VM disks, removable media and NICs

Reference: `XenModel/Actions/VDI`, `VBD`, `VIF`, `VM`,
`XenAdmin/Dialogs/VIFDialog.cs`, `XenAdmin/SettingsPanels/VBDEditPage.cs`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| List disks/attachments/device positions | Missing | No VDI/VBD view. | P2 |
| Create disk on selected SR | Missing | No allocation workflow. | P2 |
| Attach existing disk | Missing | No chooser/sharing checks. | P2 |
| Detach disk without deleting data | Missing | No detach action. | P2 |
| Activate/deactivate disk attachment | Missing | No VBD plug/unplug. | P2 |
| Resize disk | Missing | No size editor/checks. | P2 |
| Delete disk | Missing | No affected-resource review/destruction. | P2 |
| Disk name/description, access mode, I/O priority and device position | Missing | No VDI/VBD editor. | P2 |
| Move/copy/migrate disk to another SR | Missing | No placement/transfer workflow. | P3 |
| Add CD drive; mount/change/eject ISO | Missing | No removable-media workflow. | P1 |
| Disable disk changed-block tracking | Missing | Reference has disable-CBT command; not a full backup feature. | P4 |
| List NICs/MACs/networks/attachment state | Missing | No VIF view. | P2 |
| Add/delete VM NIC | Missing | No create/destroy workflow. | P2 |
| Connect/disconnect VM NIC | Missing | No plug/unplug. | P2 |
| Change NIC network/MAC | Missing | No editor. | P2 |
| NIC rate limiting/QoS | Missing | No bandwidth controls. | P3 |

## 8. Creation, templates, clones and deletion

Reference: `XenAdmin/Wizards/NewVMWizard`, `XenModel/Actions/VM`, `XenAdmin/Commands`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Browse templates | Partial | Table only; no template management. | P2 |
| Create VM from template | Missing | No CPU/memory/disk/NIC/ISO/host/review/provision wizard. | P2 |
| Installation source, copied BIOS and cloud-config options | Missing | No provisioning pages. | P2 |
| Instant/fast template creation | Missing | No fast-clone path. | P4 |
| Clone versus full-copy VM | Missing | Neither; storage semantics must remain distinct. | P2 |
| Convert VM to template | Missing | No conversion. | P2 |
| Copy/edit/delete custom templates | Missing | No lifecycle workflow. | P2 |
| Delete VM record versus VM plus selected disks/snapshots | Missing | No ownership review/deletion workflow. | P2 |
| Recover/cancel partial creation/deletion and clean owned resources | Missing | Journal is single-task, not durable multi-step execution. | P2 |

## 9. Storage repositories

Reference: `XenModel/Actions/SR`, `XenAdmin/Wizards/NewSRWizard_Pages`,
`XenAdmin/TabPages/PhysicalStoragePage.cs`, `SrStoragePage.cs`,
`XenAdmin/SettingsPanels/HostMultipathPage.cs`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| SR name/type/capacity/usage | Implemented | Unknown sizes remain unknown. | — |
| Probe/discover targets | Missing | No NFS/iSCSI IQN/LUN/HBA discovery. | P3 |
| Create SR | Missing | Each storage backend needs qualification. | P3 |
| Add ISO library | Missing | No NFS/SMB ISO-repository workflow. | P2 |
| Introduce/attach existing SR | Missing | No existing-storage attach. | P3 |
| Detach/reattach SR | Missing | No host/pool attachment workflow. | P3 |
| Repair SR connectivity | Missing | No PBD/path diagnosis/repair. | P3 |
| Rescan/refresh SR | Missing | No scan action. | P2 |
| Forget versus destroy SR | Missing | Neither or distinct affected-resource review. | P3 |
| Choose default SR | Missing | No default-storage setting. | P3 |
| SR name/description edits | Missing | Read-only. | P3 |
| Multipath settings/path inspection | Missing | No host multipath editor. | P3 |
| Trim/reclaim | Missing | No action/capability presentation. | P4 |
| Read caching/provisioning options | Missing | Driver/version-dependent reference settings. | Assess |
| Backend-specific wizards | Missing | Reference includes local/NFS/SMB/iSCSI/HBA-FC/FCoE; not a promise to port all of them. | Assess |

## 10. Networks and host interfaces

Reference: `XenModel/Actions/Network`, `XenAdmin/Wizards/NewNetworkWizard_Pages`,
`XenAdmin/Dialogs/NetworkingProperties.cs`, `XenAdmin/SettingsPanels/EditNetworkPage.cs`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Network names/bridges | Implemented | Minimal read-only view. | — |
| Create/delete internal network | Missing | No lifecycle workflow. | P3 |
| Create/edit/remove external/VLAN network | Missing | No VLAN/uplink chooser. | P3 |
| Name/description, MTU and automatic attachment | Missing | No property editor. | P3 |
| Create/destroy bond; change mode/hash settings | Missing | No active-backup/LACP/load-balancing configuration. | P3 |
| Scan/inspect physical NICs | Missing | No PIF rescan or NIC tab. | P2 |
| Plug/unplug host interfaces | Missing | No attachment controls. | P3 |
| Change management interface and IP/netmask/gateway/DNS | Missing | No network-change/reconnect plan. | P3 |
| Additional management/storage interfaces and purposes | Missing | No secondary-interface editor. | P3 |
| Pool multicast/IGMP snooping | Missing | Backend/capability dependent. | P4 |
| SR-IOV networks | Missing | No physical/virtual-function workflow; hardware dependent. | P4 |
| Cross-host internal networking | Missing | Reference CHIN path; assess target semantics before porting. | Assess |

## 11. Migration and transfers

Reference: `XenAdmin/Wizards/CrossPoolMigrateWizard`, `ImportWizard`, `ExportWizard`,
`XenModel/Actions/VM`, `XenModel/Actions/OvfActions`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Same-pool live migration | Missing | No target/CPU/storage/network preflight or task. | P3 |
| Move halted VM/change storage placement | Missing | No offline move. | P3 |
| Storage live migration | Missing | No disk destination mapping. | P3 |
| Cross-pool migration/move/copy | Missing | No destination credentials or resource mapping. | P3 |
| Transfer-network choice and console relocation | Missing | No migration continuity. | P3 |
| XVA VM/template import/export | Missing | No streamed transfer/file workflow. | P3 |
| Snapshot export | Missing | Same absent transfer foundation as section 5. | P3 |
| OVF/OVA appliance import/export | Missing | No EULA/manifest/security/resource mapping. | P4 |
| Standalone disk-image import | Missing | No image-to-VM workflow; formats need qualification. | P4 |
| Conversion manager | Missing | Reference appliance integration; assess Linux/XCP-ng applicability. | Assess |
| Transfer cancellation/expiry/partial-result recovery | Missing | Command journal is not a bulk-transfer implementation. | P3 |

## 12. Host administration

Reference: `XenModel/Actions/Host`, `XenAdmin/Commands`, `XenAdmin/SettingsPanels`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Host hardware/software properties | Partial | Basic identity/version/state only. | P2 |
| Rename/description | Missing | No editor. | P3 |
| Enable/disable scheduling; enter/exit maintenance | Missing | No maintenance workflow. | P3 |
| Evacuate host with destination checks | Missing | No orchestration. | P3 |
| Shutdown/reboot host | Missing | No host power commands. | P3 |
| Restart toolstack | Missing | No recovery command/reconnect workflow. | P1 |
| Host power-on/configuration | Missing | No configured out-of-band integration. | P4 |
| dom0 memory | Missing | No editor. | P3 |
| Host autostart | Missing | No policy editor. | P3 |
| Change host/root password | Missing | No server-password workflow. | P3 |
| Forget/remove dead host | Missing | Distinct from client disconnect. | P3 |
| Host configuration backup/restore | Missing | Not VM backup; no workflow. | P4 |
| Logging destination/monitoring settings | Missing | No remote-log/NRPE editor; assess NRPE applicability. | P4 |
| Crash dumps and support bundles | Missing | No inspect/remove/export workflow. | P3 |

## 13. Pools, HA, identity and server security

Reference: `XenModel/Actions/Pool`, `XenModel/Actions/AD`, `XenAdmin/Commands`,
`XenAdmin/Wizards/HAWizard_Pages`, `XenAdmin/SettingsPanels`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Create/dissolve pool | Missing | No lifecycle workflow. | P3 |
| Join/eject/remove pool member | Missing | No compatibility review/join/eject. | P3 |
| Pool name/description | Missing | No editor. | P3 |
| Planned coordinator handover during evacuation | Missing | Reference evacuation can designate a coordinator; absent here. | P3 |
| Enable/configure/disable HA; heartbeat SR selection | Missing | No wizard/failure-capacity checks. | P3 |
| HA status and VM restart priorities | Missing | No HA/protection overview. | P3 |
| Migration compression/GPU placement policy | Missing | No advanced pool settings. | P4 |
| Pool clustering | Missing | Capability/storage dependent; assess support. | Assess |
| Join/leave directory authentication | Missing | No AD workflow; qualify host support. | Assess |
| User/group membership and role assignment | Missing | No subject/role management. | P3 |
| RBAC-aware eligibility and credential elevation/reconnect-as | Partial | Server enforces permissions; UI does not explain/preflight them fully. | P1 |
| Install/reset host certificate | Missing | Server operation, distinct from client trust; version/HA gates apply. | P3 |
| Enable pool TLS verification/rotate pool secret | Missing | No server-side security workflow. | P3 |
| Legacy SSL mode | Missing | Reference compatibility switch; do not mechanically port obsolete modes. | Assess |

Forced coordinator promotion, quorum repair and fencing overrides remain outside initial
scope. Redirect support cannot make an unreachable host toolstack operable.

## 14. Tasks, metrics, alerts and diagnostics

Reference: `XenAdmin/TabPages/HistoryPage.cs`, `PerformancePage.cs`, `AlertSummaryPage.cs`,
`XenAdmin/Alerts`, `XenModel/Actions/Perfmon`, `XenModel/Actions/StatusReport`.
Rust: [operation journal](../crates/xenadmin-core/src/operations.rs).

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Track local power/snapshot progress/completion | Implemented | Durable intents/task evidence; recent per-VM history. | — |
| Unknown outcomes, recheck/acknowledge and cancellation | Implemented | Reconnect reconciliation; no automatic primary mutation replay. | — |
| Contextual error explanations/remediation | Partial | Sanitized host error codes exist; richer explanations missing. | P1 |
| Pool-wide tasks from other clients | Missing | Current history contains local application operations only. | P1 |
| Search/filter/dismiss history and inspect full task details | Missing | No history center. | P3 |
| Multi-step progress/partial outcomes/owned cleanup | Missing | Needed for creation/deletion/migration. | P2 |
| VM/host CPU/memory graphs | Missing | No RRD worker/chart views. | P3 |
| Disk/network throughput/latency graphs | Missing | No metrics/history. | P3 |
| Custom graphs/data-source selection/historical ranges | Missing | No performance workspace. | P4 |
| Host/pool messages/alerts: list/filter/dismiss/export | Missing | No message inventory or alert UI. | P1 |
| Alarm thresholds/intervals/notification-email settings | Missing | No configuration. | P3 |
| Resource reports/support-status export | Missing | No report workflow. | P3 |

## 15. Organization and grouped workflows

Reference: `XenAdmin/Commands`, `XenModel/Actions/CustomFields`, `Folders`,
`VMAppliances`, `XenAdmin/Wizards/NewVMApplianceWizard`, `XenAdmin/Controls/XenSearch`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Tags: create/rename/delete/assign/remove/group/search | Missing | No tag inventory/editor. | P4 |
| Folders: create/rename/delete/move/remove objects | Missing | No folder or drag/drop equivalent. | P4 |
| Custom field definitions/values | Missing | No view/editor. | P4 |
| vApps: create/edit/delete/membership | Missing | No appliance inventory/editor. | P4 |
| vApp ordered startup/shutdown and delays | Missing | No multi-VM sequence. | P4 |
| Bulk properties/delete/move/export | Missing | No affected-object/partial-outcome workflow. | P3 |
| Search definition import/export/resource export | Missing | No file workflows. | P4 |

## 16. Specialized hardware and guests

Reference: `XenModel/Actions/GPU`, `USB`, `VtpmAction.cs`, `DockerContainer`, `PVS`;
`XenAdmin/TabPages/GpuPage.cs`, `UsbPage.cs`, `XenAdmin/Dialogs/VtpmManagementDialog.cs`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Host GPU inventory/VM GPU passthrough assignment | Missing | No tab/assignment; hardware qualification needed. | P4 |
| vGPU profiles/allocation/placement | Missing | Vendor/driver/host dependent. | Assess |
| Integrated GPU passthrough host setting | Missing | No setting/reboot consequence review. | P4 |
| Physical USB inventory/passthrough enablement | Missing | No host controls. | P4 |
| Attach/detach VM USB devices | Missing | No VUSB workflow. | P4 |
| vTPM create/remove/manage | Missing | No dialog or copy/restore preservation review; qualify host support. | P4 |
| Container discovery/process/lifecycle controls | Missing | Reference Docker enlightenment/plugin paths; not required for hypervisor recovery. | Assess |
| PVS site/proxy/read-cache integration | Missing | Citrix-specific integration; not ordinary XCP-ng parity. | Assess |

Generic PCI-device administration is not claimed as verified reference GUI coverage.
GPU, USB, SR-IOV and API-level PCI capabilities are different scopes.

## 17. Disaster recovery, updates and legacy services

Reference: `XenModel/Actions/DR`, `Updates`, `Pool_Patch`, `Pool_update`,
`SupplementalPack`, `WLB`, `Healthcheck`, `XenAdmin/Wizards/DRWizards`,
`XenAdmin/Diagnostics`, `XenAdmin/SettingsPanels/LivePatchingEditPage.cs`.

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| DR metadata/recovery-storage configuration | Missing | Compatible storage/host support must be established. | Assess |
| DR failover/failback/dry run/reports | Missing | No VM/appliance recovery wizard. | Assess |
| Applicable/installed update lists and prechecks | Missing | No update UI; reference vendor/version-specific. | P3 |
| Upload/apply/clean patches/updates | Missing | Needs XCP-ng-specific maintenance/update design. | P3 |
| Repository/CDN sync/proxy/token configuration | Missing | Reference services may not apply to XCP-ng. | Assess |
| Supplemental-pack install | Missing | No upload/install workflow. | P4 |
| Live-patching policy/restart guidance | Missing | Host-lineage dependent. | Assess |
| Licensing/edition changes | Missing | Legacy XenServer flows; outside initial XCP-ng target. | Assess |
| Workload Balancing recommendations/automation/reports | Missing | External WLB service integration; not direct-host recovery. | Assess |
| Legacy health-check/cloud-service controls | Missing | Include only if a relevant supported service exists. | Assess |

The source contains upgrade diagnostics/evacuation checks. That alone does not establish
that every legacy rolling-upgrade wizard is usable in this checkout; this report does
not claim a verified upstream end-to-end rolling OS-upgrade workflow.

XO backup schedules/repositories/chains and cloud orchestration are outside this
comparison. Snapshot creation/XVA export are not an XO backup replacement. Reference
VMSS and DR functionality is listed separately because it is distinct from XO backup.

## 18. Linux desktop and extensibility

Reference: `XenAdmin/Dialogs/OptionsPages`, `XenAdmin/Plugins`, `XenAdmin/Controls/ConsoleTab`.
Rust: [running guide](RUNNING.md), [verification](VERIFICATION.md).

| Workflow | Status | Gap / boundary | Priority |
| --- | --- | --- | --- |
| Linux executable/desktop-entry archive | Implemented | Local packaged build, not installer qualification. | — |
| Clean-system installation/offline cold start | Unqualified | Runtime dependencies and XO/internet-unavailable installation checks needed. | P1 |
| Portable/distro package, upgrade and uninstall | Missing | No selected/qualified AppImage/Flatpak/deb/rpm lifecycle. | P3 |
| Native Wayland/X11/GPU compatibility | Partial | Prior Wayland smoke; X11 workaround and broader GPU/software-renderer qualification outstanding. | P1 |
| HiDPI/multimonitor/compact windows | Partial | Responsive/synthetic layout checks; real display matrix incomplete. | P2 |
| Keyboard navigation/focus/Orca | Unqualified | Semantic trees are not AT-SPI/Orca acceptance. | P2 |
| Native file selection/clipboard integration | Missing | Needed for transfers, CA files and clipboard features. | P2 |
| General/confirmation/console/display options | Partial | Endpoint/width preferences; no full options center. | P4 |
| External tools/plugins | Missing | No menu/tab plugins/command templates; Windows/PowerShell compatibility not promised. | Assess |
| Localization/integrated help | Missing | No reference-scale translations/help. | P4 |
| Multiple client instances/shared-pool ownership | Partial | Journal conflict protection exists; multiprocess recovery ownership/profile deduplication needs further design/qualification. | P1 |

## 19. Qualification gaps

These remain even where feature code exists. They are release acceptance gaps.

| Acceptance | Evidence so far | Remaining | Priority |
| --- | --- | --- | --- |
| Actual XCP-ng inventory/console | Operator reported use; local fixtures. | Sanitized exact host/API/console compatibility record. | P1 |
| XO-down recovery | No XO dependency in architecture. | Disposable-lab connect/find/start-or-resume/console run while XO unreachable. | P1 |
| VM power correctness | Local fixtures. | Disposable VM lifecycle, blocked-task and lost-response cases. | P1 |
| Snapshot data integrity | Create/revert/result-verification fixtures. | Disposable-data restore verification and interruptions/checkpoint behavior. | P2 |
| Member/coordinator/TLS recovery | Local fixtures. | Actual multihost/certificate/outage scenarios. | P1 |
| Console input/relocation | RFB fixture/operator display evidence. | Guest input/layout/resize/focus/clipboard/reboot/migration matrix. | P1 |
| Host/version/storage/permission compatibility | XCP-ng 8.3 initial target; no full live gate passed. | Record supported combinations and exclusions. | P1 |
| Scale/responsiveness/long-running use | Virtualization/bounded transports. | Measured large inventories/event bursts/console/journal load on stated hardware. | P2 |

0.2.1 local checks passed formatting, strict Clippy, 52 tests and nine synthetic Rune
renders. The unavailable-action scene exposes all nine power buttons as disabled with
status text. Earlier [0.2.0 CI/package evidence](reports/2026-11-07-snapshots.md) is
version-specific, not live-host acceptance. No production changes were made for this audit.

## Recommended order

1. Complete recovery diagnostics and console usability: actual action reasons, running/
   blocked tasks, console target identity, special keys, reconnect, verified-member
   failover and connection profiles. Qualify the XO-down path.
2. Add device inventory/durable multi-step execution, then VM properties, ISO/NIC/disks,
   snapshot deletion/clone and VM creation/deletion.
3. Add migration/host maintenance, SR/network/pool administration, metrics/alerts and transfers.
4. Decide the Assess rows individually. Their presence in the reference does not make
   them prerequisites for the recovery release.

There is no percentage-complete estimate: these workflows differ greatly in size, and
some are intentionally outside initial scope.

## Source coverage

The pinned local reference was inspected across commands (including
`Controls/ContextMenuBuilder.cs`), wizards, dialogs, settings, tabs, console controls,
GUI actions, plugins, alerts and model actions. Action families covered: AD, custom
fields, DR, Docker, folders, GPU, host, network, NRPE, OVF, performance alarms, pool,
patches/updates, PVS, SR, status reports, supplemental packs, USB, VBD/VDI/VIF, VM,
VM appliances, VMSS, vTPM, WLB and conversion. Shared action infrastructure and generated
form resources are not separate user features. The section source paths are relative
to the pinned reference linked below; Rust source links point to this repository.

No private inventories, operator screenshots, console contents, credentials or endpoint
identifiers are included as report examples or attachments.

[reference]: https://github.com/xcp-ng/xenadmin/tree/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4
[force]: https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenAdmin/Commands/ForceVMShutDownCommand.cs
[boot]: https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/VM/HVMBootAction.cs
[vm-api]: https://xapi-project.github.io/new-docs/xen-api/classes/vm/index.html
