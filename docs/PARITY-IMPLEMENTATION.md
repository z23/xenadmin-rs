# Retained parity implementation ledger

This ledger tracks every workflow in [PARITY.md](PARITY.md) sections 1–15 and 18,
plus the qualification gates in section 19. Sections 16 (specialized hardware and guests)
and 17 (disaster recovery, updates and legacy services) are excluded at the user's request.
No other `Assess` row is silently removed: it needs a supported capability implementation
or a documented unsupported result for the target XCP-ng version.

The status below is conservative. Backend data models and preparatory helpers are
foundations, not completed user workflows. A row becomes implemented only when the UI,
typed command, fresh checks, outcome reporting and local verification exist. It becomes
qualified only after a sanitized disposable-lab happy path and failure/interruption path.
This is an implementation ledger; [the original audit](PARITY.md) remains the reference
baseline and [PLAN.md](PLAN.md) provides the existing milestone gates.

This change adds saved profiles/pool ownership, optional keyring credentials, event recovery,
expanded device inventory and action diagnosis, console controls, VM property edits and
durable steps, chosen-host placement, memory checkpoints and safe first device actions.
Rows below record those concrete subsets; no broad row is complete merely because a
supporting type or method exists. Local automated checks do not establish live qualification.
The [nested lab harness](../lab/README.md) creates isolated, owned local resources; no
live-host qualification has yet been established by this change.

The [device lifecycle increment](reports/2026-11-08-device-lifecycle.md) adds the reviewed
halted-VM disk, CD-drive and NIC workflows recorded in section 7. It retains explicit
partial/unknown outcomes and does not establish live-host compatibility.

## 1. Connections, credentials and trust

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P01-01 | Direct host/pool login and logout without XO | Existing workflow; qualification open |
| P01-02 | Connect through a non-coordinator member | Existing workflow; qualification open |
| P01-03 | Session renewal and reconnect | Bounded reconnect/resync and jitter implemented; coordinator-outage qualification open |
| P01-04 | Try remembered verified pool members after endpoint failure | Verified-member candidates persisted and retried; live failover qualification open |
| P01-05 | Multiple saved connection profiles | Profile editor, persistence and legacy migration implemented; desktop/lab qualification open |
| P01-06 | Simultaneous pools and unified inventory | Concurrent connections, pool switcher and unified cross-pool search implemented; typed identities, alias deduplication and stale coverage fixture verified; live qualification open |
| P01-07 | Deduplicate aliases/member endpoints for the same pool | Authenticated owner sharing, authority-mismatch rejection and exclusive desktop state-directory lock implemented; lab qualification open |
| P01-08 | Connect/disconnect all; remove profiles; reconnect as another user | Profile edit/remove, per-connection and bulk controls implemented; bounded saved-credential loads, per-profile failures and pending cancellation fixture-verified; live qualification open |
| P01-09 | Session-only password | Existing workflow; qualification open |
| P01-10 | Save/forget credentials and protected session restore | Opt-in Secret Service save/load/forget and per-profile startup restore implemented; stale credential results, locked/missing keyring and no-retry behavior fixture-verified; desktop-service qualification open |
| P01-11 | System CA/name verification and endpoint pins | Existing workflow; qualification open |
| P01-12 | Desktop custom-CA file selection | Native portal file picker and manual path entry implemented; cancelled/stale editor results fixture verified, native portal qualification open |
| P01-13 | Inspect and accept unknown certificates | Queued |
| P01-14 | Review changed certificate and replace trust | Existing subset; remaining work queued |
| P01-15 | Easy certificate-validation opt-out | Existing workflow; qualification open |
| P01-16 | Proxy/timeout options and richer connection diagnostics | Request timeout and connection errors implemented; proxy configuration and richer diagnostics remain queued |

## 2. Inventory, navigation and inspection

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P02-01 | Pool/host/VM tree; VM and template tables | Existing workflow; qualification open |
| P02-02 | Network and SR tables | Typed SR/network metadata and device-related detail views added; full dedicated administration views remain queued |
| P02-03 | Search by name/UUID/host; sort and virtualized rows | Existing workflow; qualification open |
| P02-04 | Persist sidebar/table column widths | Existing workflow; qualification open |
| P02-05 | Full VM general information | Guest OS/tools/addresses and VM resource metadata added; full runtime/uptime presentation remains partial |
| P02-06 | Full host/pool properties | Host software/CPU metadata, metrics and attachment details displayed; full pool properties remain queued |
| P02-07 | VM disk/NIC and host/storage attachment graph | Typed VDI/VBD/VIF/PIF/PBD graph and VM devices view implemented; broader host/SR views and lab qualification remain open |
| P02-08 | Physical NIC/link/bond/VLAN inspection | Host/network details display PIF addressing, management/physical flags, VLAN, MTU and bond members/mode/link count; link-speed/path inspection remains partial |
| P02-09 | SR attachment/path and disk inspection | SR details display host attachments, disk capacity/flags, VM/snapshot consumers and bounded per-PBD multipath/path observations; fixture verified, actual path health and live qualification remain open |
| P02-10 | Live inventory/stale-state handling | Optional-class negotiation, isolated decode failures and bounded resync implemented; live compatibility/scale qualification open |
| P02-11 | Filtered/grouped/saved/cross-pool searches | Typed cross-pool text/type/power/host/pool/tag/folder/custom-field filters, grouped ordering and saved definitions implemented and fixture verified; [search report](reports/2026-11-08-cross-pool-search.md); live qualification open |
| P02-12 | Multi-object actions and context menus | VM multi-selection, reviewed same-pool power batches and name-cell context menus implemented; fixture verified. Keyboard/whole-row menus and other bulk workflows remain open ([report](reports/2026-11-08-bulk-power-ui.md)) |

## 3. VM power and recovery

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P03-01 | Start, clean shutdown and reboot | Existing workflow; qualification open |
| P03-02 | Force shutdown/reboot | Existing subset; remaining work queued |
| P03-03 | Pause/unpause and suspend/resume | Existing workflow; qualification open |
| P03-04 | Start/resume on a chosen eligible host/home server | Chosen-host start/resume and fresh compatibility preflight implemented; live placement qualification open |
| P03-05 | Explain unavailable actions | Host block messages preserved, current diagnostic context enforced and contradictory blocked/allowed operations refused; lab RBAC/state qualification open |
| P03-06 | Show current tasks and administrative action blocks | Current operations, administrative blocks, per-action diagnosis and pool-wide task Activity workspace implemented; fixture verified, live qualification open |
| P03-07 | Reviewed force recovery with running tasks | Queued |
| P03-08 | Recovery-mode boot | Queued |
| P03-09 | Batch VM power actions | Reviewed 1–32 VM same-pool serial power batches implemented with per-VM outcomes, whole-selection preflight and no-replay recovery; protocol/Rune fixtures verified, live qualification open ([report](reports/2026-11-08-bulk-power-backend.md)) |

## 4. Consoles

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P04-01 | Embedded direct authenticated RFB console | Existing workflow; qualification open |
| P04-02 | Keyboard and basic pointer input | Focus-aware keyboard/pointer controls extended; real guest input/layout matrix remains open |
| P04-03 | Ctrl+Alt+Delete and deliberate special-key controls | Deliberate key-chord toolbar implemented; guest/platform qualification open |
| P04-04 | Mouse wheel/middle button | Wheel and middle-button mapping implemented; real guest qualification open |
| P04-05 | Extended QEMU keys, layouts and shortcut capture | QEMU physical-key extension implemented with fallback; non-US/shortcut-capture qualification remains open |
| P04-06 | Controlled clipboard send/receive | Explicit guest clipboard send/receive with limits and protocol negotiation implemented; platform/guest qualification open |
| P04-07 | Reopen after reboot/migration/session replacement | Bounded automatic reconnect follows the pinned VM's current same-pool host, boot marker and session generation; stale/missing/offline targets release input, failures stop after five attempts, and manual/opt-out controls remain available. Model/Rune fixtures verified; cross-pool routing and live qualification remain open ([report](reports/2026-11-08-console-recovery.md)) |
| P04-08 | Keep console target aligned with Details/actions | Console target pinned to pool/VM identity; selection changes no longer silently retarget input; lab qualification open |
| P04-09 | Scaling, framebuffer resize and cursor behavior | Fit/actual-size/panning and cursor handling extended; live resize/display matrix remains open |
| P04-10 | Full-screen, detached and multiple console windows | Fullscreen implemented; detached/multiple windows require further Rune integration |
| P04-11 | Save console screenshot | Explicit native save dialog and background PNG export of the captured frame implemented; private atomic file output and console-switch fencing fixture verified, native qualification open |
| P04-12 | Host text console; guest SSH/RDP/external viewer | Direct authenticated host control-domain RFB console implemented with fresh host/VM/console identity checks and host-only details; protocol/Rune fixtures verified. Guest SSH/RDP/external viewer and live qualification remain open ([report](reports/2026-11-08-host-console.md)) |

## 5. Snapshots

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P05-01 | List per-VM snapshots and basic details | Existing workflow; qualification open |
| P05-02 | Create named disk snapshot | Existing workflow; qualification open |
| P05-03 | Restore to existing VM | Reviewed exact-identity disk/checkpoint restore with task and postcondition verification; fixture verified, lab data-integrity qualification open. [Evidence](reports/2026-11-08-snapshot-workflows.md) |
| P05-04 | Shutdown-before/boot-or-resume-after restore | Explicit clean shutdown of a running VM and optional start/resume after verified restore; durable per-step partial/unknown outcomes and no replay. Fixture verified, live qualification open |
| P05-05 | Disk-and-memory checkpoint creation | Named memory checkpoint command, confirmation and result verification implemented; data-integrity qualification open |
| P05-06 | Guest-quiesced snapshot creation | Explicitly unsupported on XCP-ng 8.1+ because VSS/quiesced support was removed; no legacy API or consistency fallback offered. [Evidence](reports/2026-11-08-snapshot-workflows.md) |
| P05-07 | Snapshot description at creation; edit name/description | Create description after new snapshot identity verification; reviewed name/description edits with concurrent-change guards and partial outcomes. Fixture verified, live qualification open |
| P05-08 | Delete snapshots with owned-disk cleanup | Reviewed record deletion and explicit owned disk UUID cleanup implemented with checkpoint-memory checks and durable partial/unknown outcomes; fixture verified, lab qualification open |
| P05-09 | New VM from snapshot | Reviewed clone and separately journaled template conversion implemented; new halted VM identity verified against pre-existing UUID baseline; fixture verified, lab qualification open |
| P05-10 | New template from snapshot | Reviewed clone with new template identity/provenance checks implemented; fixture verified, lab qualification open |
| P05-11 | Export snapshot as template/backup | Reviewed halted disk-snapshot XVA export implemented; source/parent identity and private atomic output fixture verified. Memory-checkpoint export, snapshot-chain coverage and live qualification remain open; see P11-07 |
| P05-12 | Ancestry tree, preview image, storage/size detail | Queued |
| P05-13 | Scheduled-snapshot policies | Implemented/fixture-verified: reviewed host VMSS create/edit/assignment/enable/run/delete; XCP-ng 8.3 plugin contract and local clock checked. Live cron/DST/retention qualification open. [Evidence](reports/2026-11-08-snapshot-schedules.md) |

## 6. VM properties/resources

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P06-01 | Rename VM/template and description | VM and custom-template metadata/property editing implemented with explicit kind/default-template guards; fixture verified, live qualification open |
| P06-02 | Startup/max/live vCPU counts | Halted startup/max vCPU editing and reviewed live vCPU increases within the existing maximum implemented; exact host capability and current limit checks fixture verified, live qualification open |
| P06-03 | CPU topology and scheduling weight | Halted cores-per-socket topology and CPU scheduling weight editors implemented; unrelated platform/CPU keys preserved, live qualification open |
| P06-04 | Static/dynamic memory limits, ballooning/live changes | Halted static/dynamic limits and reviewed running-VM dynamic ranges implemented with fresh static-bound checks; fixture verified, live ballooning qualification open |
| P06-05 | Boot order, bootable devices, autostart | Boot-order/autostart editors and reviewed halted-VM bootable disk-attachment flags implemented; fixture verified, live qualification open |
| P06-06 | Home server/host affinity | Reviewed pool-host preference/set-clear editor implemented with fresh UUID resolution and lost-reply reconciliation; fixture verified, live placement qualification open ([evidence](reports/2026-11-08-home-host.md)) |
| P06-07 | VM HA restart priority/protection | Reviewed restart/best-effort/disabled priorities and server eligibility checks implemented; fixture verified, advanced HA policy and live qualification open ([report](reports/2026-11-08-pool-administration.md)) |
| P06-08 | BIOS/UEFI/Secure Boot provisioning choices | Reviewed creation modes, halted UEFI Secure Boot policy and read-only guest/pool readiness implemented; installed-image BIOS/UEFI switches refused, fixture verified, live enforcement qualification open ([evidence](reports/2026-11-08-firmware-templates.md)) |
| P06-09 | HVM shadow-memory settings | Reviewed halted/live shadow-memory multiplier editing implemented with modern HVM identity and advertised capability checks; fixture verified, live qualification open |
| P06-10 | Cloud-config parameters/config drive | Reviewed halted-VM NoCloud FAT drive with private user-data/metadata/network-config, chosen SR/slot, direct upload and read-only attachment implemented; no automatic boot or guest execution claim. Local protocol/Rune fixtures verified; live guest qualification open. See [NoCloud report](reports/2026-11-08-cloud-init.md) |
| P06-11 | Guest tools status/installation media | Guest-reported tools/OS/address display implemented; tools-install media workflow remains queued |

## 7. VM disks, removable media and NICs

Property editor evidence is in the [device property report](reports/2026-11-08-device-properties.md).

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P07-01 | List disks/attachments/device positions | VM disk/attachment/device-position view implemented; live storage qualification open |
| P07-02 | Create disk on selected SR | Reviewed halted-VM creation and attachment on a chosen eligible SR implemented; created disk UUID survives partial failure; fixture verified, lab qualification open |
| P07-03 | Attach existing disk | Reviewed halted-VM attachment of an unattached managed, non-shared user disk implemented, with explicit access mode and no ownership claim; fixture verified, lab qualification open |
| P07-04 | Detach disk without deleting data | Reviewed halted-VM detachment implemented; backing disk and data remain; fixture verified, lab qualification open |
| P07-05 | Activate/deactivate disk attachment | Reviewed existing disk attachment plug/unplug implemented with fresh ownership checks; lab qualification open |
| P07-06 | Resize disk | Halted VM grow-only disk workflow implemented with size/consumer checks; other resize modes remain queued |
| P07-07 | Delete disk | Reviewed halted-VM deletion of an exclusively owned, non-shared user disk implemented with current consumer checks and separate journaled attachment/data deletion; fixture verified, lab qualification open |
| P07-08 | Disk name/description, access mode, I/O priority and device position | Reviewed exclusive-disk metadata, halted access/position and advertised best-effort I/O priority implemented; per-setter recovery and map preservation fixture verified, live qualification open |
| P07-09 | Move/copy/migrate disk to another SR | Selected disks can relocate via reviewed same-pool VM migration, including current-host selection; standalone copy and broader disk topologies remain queued |
| P07-10 | Add CD drive; mount/change/eject ISO | Reviewed halted-VM empty CD-drive creation and existing-drive insert/eject implemented; replacement uses explicit eject then insert; fixture verified, lab qualification open |
| P07-11 | Disable disk changed-block tracking | Reviewed halted exclusive-disk disablement with exact disk confirmation and backup-chain warning implemented; lost-reply reconciliation fixture verified, live qualification open |
| P07-12 | List NICs/MACs/networks/attachment state | VM NIC/MAC/network/attachment view implemented; live network qualification open |
| P07-13 | Add/delete VM NIC | Reviewed halted-VM addition/deletion implemented with a chosen network, device slot and optional generated MAC; fixture verified, lab qualification open |
| P07-14 | Connect/disconnect VM NIC | Reviewed NIC plug/unplug implemented with fresh ownership/state checks; lab qualification open |
| P07-15 | Change NIC network/MAC | Reviewed halted-VM network move and MAC replacement implemented; supported MTU/QoS/locking/IP restrictions retained, unsupported extra configuration refused; fixture verified, lab qualification open |
| P07-16 | NIC rate limiting/QoS | Reviewed halted NIC ratelimit policy in KB/s implemented when advertised by the host; unrelated parameters retained, fixture verified, live throughput qualification open |

## 8. Creation, templates, clones and deletion

Implementation and verification details are in the [VM lifecycle report](reports/2026-11-08-vm-lifecycle.md).

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P08-01 | Browse templates | Template inventory and lifecycle action entry points implemented; lab qualification open |
| P08-02 | Create VM from template | Reviewed halted-VM wizard implemented with name/description, CPUs/memory, destination SR, network, optional disk/ISO and ordinary firmware choices; fixture verified, lab qualification open |
| P08-03 | Installation source, copied BIOS and cloud-config options | ISO selection, inherited/BIOS/UEFI/Secure Boot modes and chosen-host BIOS identity copy with reviewed affinity side effect implemented for eligible templates; ordinary cloud-init drive preparation is available as a separate halted-VM workflow after creation; inline creation-wizard cloud-config remains a follow-up; legacy CoreOS plugin integration excluded |
| P08-04 | Instant/fast template creation | Fast clone to custom template implemented when the source/storage advertises support; fixture verified, lab qualification open |
| P08-05 | Clone versus full-copy VM | Reviewed halted source clone versus full copy to selected SR implemented, with VM/template result choice; fixture verified, lab qualification open |
| P08-06 | Convert VM to template | Reviewed halted ordinary-VM conversion implemented with fresh capability checks; fixture verified, lab qualification open |
| P08-07 | Copy/edit/delete custom templates | Copy, reviewed deletion and custom-template metadata/CPU/memory/topology/weight/shadow/boot/autostart/home-host/Secure Boot properties implemented; empty-template firmware changes require explicit review and built-in templates remain protected; fixture verified, lab qualification open |
| P08-08 | Delete VM record versus VM plus selected disks/snapshots | Record-only or exact selected owned-disk/halted-snapshot deletion implemented with current ownership checks; checkpoint deletion uses its existing separate flow; fixture verified, lab qualification open |
| P08-09 | Recover/cancel partial creation/deletion and clean owned resources | Durable steps, read-only recheck, cancellation, unknown acknowledgment and created resource UUIDs implemented; retained created VMs can use reviewed deletion, standalone detached-orphan cleanup remains pending; lab qualification open |

## 9. Storage repositories

Implementation boundaries and evidence: [storage lifecycle report](reports/2026-11-08-storage-lifecycle.md).

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P09-01 | SR name/type/capacity/usage | Existing workflow; qualification open |
| P09-02 | Probe/discover targets | Read-only backend capabilities, NFS export and iSCSI IQN/LUN enumeration plus bounded SR discovery implemented with explicit selection and captured inputs; fixture verified, lab qualification open |
| P09-03 | Create SR | Reviewed shared NFS and LVM-iSCSI creation, including CHAP/mutual CHAP, implemented with transient credentials, tracked tasks, verified new identities and journalled temporary Secret cleanup; other backends pending; fixture verified, lab qualification open |
| P09-04 | Add ISO library | ISO over NFS and explicit SMB 3.0 creation with optional transient authentication implemented; no SMB1 fallback; fixture verified, lab qualification open |
| P09-05 | Introduce/attach existing SR | Reviewed introduce by exact UUID, separate selected-host PBD creation and attach implemented; existing UUIDs are never implicitly forgotten; authenticated new attachments require fresh CHAP/SMB credentials and verify host-owned Secret copies; fixture verified, lab qualification open |
| P09-06 | Detach/reattach SR | Reviewed selected-host PBD unplug/plug implemented with fresh SR/host identity and consumer checks; attached status explicitly does not imply healthy storage; fixture verified, lab qualification open |
| P09-07 | Repair SR connectivity | Per-host attach retry and explicit missing-PBD creation implemented; authenticated missing-PBD creation from safe projected location and fresh credentials implemented; automatic multi-host repair remains pending; lab qualification open |
| P09-08 | Rescan/refresh SR | Reviewed asynchronous rescan with durable task tracking, cancellation and reconnect recovery implemented; synthetic fixtures verified, lab qualification open |
| P09-09 | Forget versus destroy SR | Separate exact-UUID reviews implemented with captured disk set, fresh detach/consumer/default/HA dependency checks and tracked tasks; UI removal limited to 100 reviewed disks; fixture verified, destructive lab qualification open |
| P09-10 | Choose default SR | Reviewed pool default SR selection with fresh prior-value and user-storage checks implemented; lab qualification open |
| P09-11 | SR name/description edits | Reviewed metadata editing with fresh conflict checks and explicit partial outcomes implemented; lab qualification open |
| P09-12 | Multipath settings/path inspection | Bounded per-PBD path inspection and reviewed standard multipathing changes on empty maintenance hosts with all PBDs detached implemented; automatic detach/replug orchestration remains pending; fixture verified, live path qualification open |
| P09-13 | Trim/reclaim | Explicit SR_TRIM-capable attached-host reclaim via fixed trim plugin implemented with tracked task and positive-result evidence; unavailable plugin is a reported host error; fixture verified, lab qualification open |
| P09-14 | Read caching/provisioning options | Capability review and implementation queued |
| P09-15 | Backend-specific wizards | Typed NFS, ISO-NFS, SMB3 and CHAP/mutual-CHAP LVM-iSCSI forms, custom iSCSI port and authenticated guided target enumeration implemented; other backends remain pending |

## 10. Networks and host interfaces

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P10-01 | Network names/bridges | Existing workflow; qualification open |
| P10-02 | Create/delete internal network | Reviewed creation and unused internal-network deletion implemented; identity provenance, lost replies and fresh interface checks fixture verified; lab qualification open |
| P10-03 | Create/edit/remove external/VLAN network | Per-host VLAN creation/deletion on reviewed networks implemented; fresh topology and new-identity verification fixture tested. Pool-wide orchestration and VLAN retagging remain queued |
| P10-04 | Name/description, MTU and automatic attachment | Names/descriptions, non-management attached-network MTU and automatic new-VM attachment settings implemented with fresh checks; management-network MTU workflow remains queued |
| P10-05 | Create/destroy bond; change mode/hash settings | Create/destroy unused non-management bonds and mode/hash changes implemented with exact member review; management/VLAN/IP migration during bond creation remains queued |
| P10-06 | Scan/inspect physical NICs | Physical NIC inspection and asynchronous host rescan implemented; fixture verified, live qualification open |
| P10-07 | Plug/unplug host interfaces | Reviewed plug/unplug with fresh host/interface identity and management-unplug refusal implemented; fixture verified, live qualification open |
| P10-08 | Change management interface and IP/netmask/gateway/DNS | Non-management IPv4 mode/IP/netmask/gateway/DNS editing implemented; management interface handover, recovery endpoints and IPv6 editing remain queued |
| P10-09 | Additional management/storage interfaces and purposes | Queued |
| P10-10 | Pool multicast/IGMP snooping | Reviewed pool IGMP snooping setting implemented and fixture verified; live qualification open |
| P10-11 | SR-IOV networks | Queued |
| P10-12 | Cross-host internal networking | Capability review and implementation queued |

## 11. Migration and transfers

See the [same-pool migration increment](reports/2026-11-08-migration.md) for the supported topology, identity verification and recovery limits.

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P11-01 | Same-pool live migration | Reviewed running-VM host moves implemented and fixture-tested with fresh compatibility/topology checks; live qualification open |
| P11-02 | Move halted VM/change storage placement | Reviewed same-pool move and separately verified home host implemented with partial/unknown recovery; snapshot chains and live qualification open |
| P11-03 | Storage live migration | Ordinary exclusive non-CBT disk relocation and NIC mapping implemented with replacement UUID verification; snapshot/shared-disk topologies and live qualification open |
| P11-04 | Cross-pool migration/move/copy | Reviewed ordinary exclusive-disk live/halted move and halted copy implemented with both-pool reservations, exact mapping/identity verification and no-replay recovery; synthetic fixtures verified. Snapshot/shared/media/HA/vApp topologies and live qualification remain open; [report](reports/2026-11-08-cross-pool-transfers.md) |
| P11-05 | Transfer-network choice and console relocation | Explicit IP-configured destination transfer network and one-shot console follow after verified local same-pool migration implemented; cross-pool moves now offer explicit destination-console reopen after verified success; external/IPv6-only paths and live qualification open |
| P11-06 | XVA VM/template import/export | Native reviewed plain/gzip ordinary-guest import and halted VM/custom-template export implemented; bounded identity/provenance fixtures verified; zstd, broader archive shapes and live qualification open |
| P11-07 | Snapshot export | Halted disk-snapshot XVA export implemented with parent/source identity and normalized archive verification; memory checkpoints, snapshot chains and live qualification open |
| P11-08 | OVF/OVA appliance import/export | Reviewed ordinary HVM raw/VHD OVF/OVA import and VHD package export implemented with explicit SR/network mapping, private atomic publication and no-replay partial-resource recovery; parser/protocol/Rune fixtures verified. VMDK/conversion, advanced appliance settings and live qualification remain open; [report](reports/2026-11-08-ovf-ova-transfers.md) |
| P11-09 | Standalone disk-image import | Raw and fixed/dynamic VHD import into a new unattached disk implemented; matching disk export, private atomic files, captured size/storage and partial-identity recovery fixture verified. Differencing VHD/VHDX/QCOW2, conversions and live qualification remain open; [report](reports/2026-11-08-disk-image-transfers.md) |
| P11-10 | Conversion manager | Capability review and implementation queued |
| P11-11 | Transfer cancellation/expiry/partial-result recovery | XVA, raw/VHD and ordinary OVF/OVA stream/task progress, explicit cancellation, expiry and no-replay recovery implemented; failed/unknown imports retain proven UUIDs and unresolved guards; conversion formats and live qualification open |

See the [XVA transfer increment](reports/2026-11-08-xva-transfers.md) for format limits, trust and unresolved import cleanup.

## 12. Host administration

Implementation and qualification evidence: [host administration report](reports/2026-11-08-host-administration.md).

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P12-01 | Host hardware/software properties | Host software/CPU metadata, live memory, attachments, maintenance, multipathing, logging and power-on mode displayed; broader hardware fields and live qualification remain open |
| P12-02 | Rename/description | Reviewed host metadata editing implemented with fresh conflict checks and explicit partial outcomes; lab qualification open |
| P12-03 | Enable/disable scheduling; enter/exit maintenance | Reviewed disable/evacuate/maintenance marker and enable/exit workflows implemented with exact guest review and per-step outcomes; fixture-verified, lab open |
| P12-04 | Evacuate host with destination checks | Reviewed guest UUID set, fresh blockers/possible-host checks and authoritative assert_can_evacuate before disable and evacuation; no automatic HA reduction or guest shutdown; fixture-verified, lab open |
| P12-05 | Shutdown/reboot host | Reviewed shutdown/reboot for disabled empty hosts; separate handover required for multi-host coordinators; task success/offline or newer boot marker verifies outcome; fixture-verified, lab open |
| P12-06 | Restart toolstack | Reviewed toolstack restart with pending-task preflight and newer agent-start marker recovery; fixture-verified, lab open |
| P12-07 | Host power-on/configuration | Power-on using configured method and disabled/Wake-on-LAN configuration implemented; BMC credential/Secret-backed editing remains queued; fixture-verified, lab open |
| P12-08 | dom0 memory | Reviewed exact-control-domain memory change with current bounds/physical limit checks and fresh verification; fixture-verified, lab open |
| P12-09 | Host autostart | Explicit pool-wide VM autostart review implemented; preserves per-VM flags and does not start VMs immediately; fixture-verified, lab open |
| P12-10 | Change host/root password | Retained dependency: volatile old/new credentials, session password-change and reconnect credential update workflow queued |
| P12-11 | Forget/remove dead host | Reviewed offline/disabled/empty non-coordinator removal with independent power-off acknowledgement and HA disabled; retains SR records/data; fixture-verified, lab open |
| P12-12 | Host configuration backup/restore | Retained dependency: authenticated bounded host backup/restore transfers and exact-host recovery review queued |
| P12-13 | Logging destination/monitoring settings | Reviewed remote syslog destination and separately journaled reconfigure implemented; further monitoring settings remain queued; fixture-verified, lab open |
| P12-14 | Crash dumps and support bundles | Retained dependency: crash-dump inventory and private support-bundle download workflow queued |

## 13. Pools, HA, identity and server security

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P13-01 | Create/dissolve pool | A standalone host is already a pool; naming, individually reviewed ejection and joining a standalone host to an independently connected pool implemented. Coordinated dissolve remains queued; live qualification open ([join report](reports/2026-11-08-pool-join.md)) |
| P13-02 | Join/eject/remove pool member | Reviewed standalone-host join and ejection of disabled, evacuated non-coordinator members with empty local guest storage implemented; offline removal remains queued; live qualification open ([join report](reports/2026-11-08-pool-join.md)) |
| P13-03 | Pool name/description | Reviewed metadata editing with fresh conflict checks and explicit partial outcomes implemented; lab qualification open |
| P13-04 | Planned coordinator handover during evacuation | Explicit reviewed handover to a live enabled member implemented; HA must already be disabled. Separate from evacuation; live qualification open |
| P13-05 | Enable/configure/disable HA; heartbeat SR selection | Reviewed enable/disable, heartbeat SR selection and failure-tolerance setting implemented with fresh server eligibility/failover-plan checks; advanced HA settings and live qualification open |
| P13-06 | HA status and VM restart priorities | Typed HA status, heartbeat storage, current plan/overcommit and three VM restart priorities implemented; start-order/delay configuration and live qualification open |
| P13-07 | Migration compression/GPU placement policy | Reviewed migration compression implemented for advertised fields; GPU placement is excluded specialized hardware scope; live qualification open |
| P13-08 | Pool clustering | Unsupported on XCP-ng: upstream clustering relies on unavailable proprietary software ([primary evidence](https://docs.xcp-ng.org/installation/upgrade/)); no unsupported RPC workflow is offered |
| P13-09 | Join/leave directory authentication | Reviewed AD join and local-only/credentialed leave implemented with transient credentials, exact member checks and partial/unknown recovery; fixture verified, live qualification open ([report](reports/2026-11-08-pool-security.md)) |
| P13-10 | User/group membership and role assignment | Directory resolution, subject creation/removal and exact reviewed role edits implemented for explicit RBAC support; restricted/unknown RBAC creation is refused to avoid automatic administrator grants; live qualification open |
| P13-11 | RBAC-aware eligibility and credential elevation/reconnect-as | Security discovery distinguishes methods, local superuser and RBAC support; security mutations require local superuser. Session-only reconnect-as is implemented without changing saved account/keyring/trust; general per-method RBAC eligibility remains queued |
| P13-12 | Install/reset host certificate | Reviewed PEM/PKCS8 installation and self-signed reset implemented with SHA-256 checks, transient private keys, native file pickers and no automatic trust change; fixture verified, live qualification open |
| P13-13 | Enable pool TLS verification/rotate pool secret | Explicit reviewed TLS enable and pool-secret rotation implemented with durable completion receipts; rotation refuses enabled HA or unreachable members; live qualification open |
| P13-14 | Legacy SSL mode | Unsupported on primary target: legacy SSL and TLS 1.0/1.1 were removed in XCP-ng 8.2 ([release notes](https://docs.xcp-ng.org/releases/release-8-2/)) |

## 14. Tasks, metrics, alerts and diagnostics

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P14-01 | Track local power/snapshot progress/completion | Durable task tracking extended to placement, checkpoints and device actions; lab qualification open |
| P14-02 | Unknown outcomes, recheck/acknowledge and cancellation | Existing workflow; qualification open |
| P14-03 | Contextual error explanations/remediation | Administrative messages, fresh action diagnostics and private live-client inspector added; broader remediation catalogue remains queued |
| P14-04 | Pool-wide tasks from other clients | Activity task workspace with filtering, typed details and reviewed cancellation; fixture-verified, live qualification open ([report](reports/2026-11-08-activity.md)) |
| P14-05 | Search/filter/dismiss history and inspect full task details | Searchable VM/resource history, terminal-only local hide/restore, typed task details and private JSON export implemented; raw task/debug payloads excluded, native/lab qualification open ([report](reports/2026-11-08-activity.md)) |
| P14-06 | Multi-step progress/partial outcomes/owned cleanup | Durable ordered VM property steps and partial/unknown outcomes implemented; owned-resource cleanup/provisioning orchestration remains queued |
| P14-07 | VM/host CPU/memory graphs | Direct RRD subscriptions and VM/host performance charts implemented; synthetic transport/render checks, live qualification open |
| P14-08 | Disk/network throughput/latency graphs | Host-advertised disk/network sources rendered with units and explicit sample gaps; source availability and live qualification remain host-dependent |
| P14-09 | Custom graphs/data-source selection/historical ranges | Per-view source selection and bounded 10-minute/2-hour/week/year ranges implemented with actual returned resolution; saved graph presets remain queued |
| P14-10 | Host/pool messages/alerts: list/filter/dismiss/export | Activity list/filter, content-bound reviewed server dismissal and private native-dialog JSON export implemented; fixture-verified, native/lab qualification open ([report](reports/2026-11-08-activity.md)) |
| P14-11 | Alarm thresholds/intervals/notification-email settings | Queued |
| P14-12 | Resource reports/support-status export | Activity task/message/public-operation JSON export implemented; broader resource reports and support-status bundles remain queued |

## 15. Organization and grouped workflows

See the [organization increment](reports/2026-11-08-organization.md) for captured-object scope and shared-map concurrency limits.

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P15-01 | Tags: create/rename/delete/assign/remove/group/search | Reviewed tag assignment/removal and captured-pool rename/delete, tag group filters and metadata search implemented; synthetic fixtures verified, lab qualification open |
| P15-02 | Folders: create/rename/delete/move/remove objects | Reviewed compatible folder registry, subtree rename/removal and selected object moves implemented; exact captured objects and per-step partial outcomes, lab qualification open |
| P15-03 | Custom field definitions/values | Bounded compatible text/date definitions and selected values implemented; definition deletion reviews value cleanup and refuses unreviewed consumers; fixture verified, lab qualification open |
| P15-04 | vApps: create/edit/delete/membership | Reviewed empty-group creation, metadata edits, per-VM membership and empty-group deletion implemented; fresh identity/graph and lost-reply fixtures verified, lab qualification open |
| P15-05 | vApp ordered startup/shutdown and delays | Per-member order/delay edits and explicit ordered start, clean or force shutdown implemented; exact captured members, durable completion and partial-child fixtures verified, lab qualification open |
| P15-06 | Bulk properties/delete/move/export | Queued |
| P15-07 | Search definition import/export/resource export | Reviewed bounded JSON search-definition import/export implemented with native pickers; legacy XenCenter XML interoperability and resource-result export remain open |

## 18. Linux desktop and extensibility

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P18-01 | Linux executable/desktop-entry archive | Existing workflow; qualification open |
| P18-02 | Clean-system installation/offline cold start | Qualification open |
| P18-03 | Portable/distro package, upgrade and uninstall | Versioned portable archive with desktop/icon metadata and dependency notices implemented; automated installation/upgrade/uninstall, AppImage/distro packaging and clean-system qualification remain open |
| P18-04 | Native Wayland/X11/GPU compatibility | Existing subset; remaining work queued |
| P18-05 | HiDPI/multimonitor/compact windows | Existing subset; remaining work queued |
| P18-06 | Keyboard navigation/focus/Orca | Qualification open |
| P18-07 | Native file selection/clipboard integration | Explicit console clipboard and native CA/screenshot/transfer/search/activity pickers implemented with cancelled/stale-callback fixtures; real portal/clipboard/desktop qualification remains open |
| P18-08 | General/confirmation/console/display options | Connection-profile and console options extended; general options center remains partial |
| P18-09 | External tools/plugins | Capability review and implementation queued |
| P18-10 | Localization/integrated help | Queued |
| P18-11 | Multiple client instances/shared-pool ownership | Authenticated pool alias ownership and exclusive desktop state-directory lock implemented; cross-process IPC/focus handoff remains queued |

## 19. Qualification gaps

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P19-01 | Actual XCP-ng inventory/console | Lab qualification open |
| P19-02 | XO-down recovery | Lab qualification open |
| P19-03 | VM power correctness | Lab qualification open |
| P19-04 | Snapshot data integrity | Lab qualification open |
| P19-05 | Member/coordinator/TLS recovery | Lab qualification open |
| P19-06 | Console input/relocation | Lab qualification open |
| P19-07 | Host/version/storage/permission compatibility | Lab qualification open |
| P19-08 | Scale/responsiveness/long-running use | Lab qualification open |

This ledger contains 202 retained workflow/qualification rows. IDs are stable
within the pinned audit and are intended for tests, lab evidence and follow-up changes.
