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
| P01-13 | Inspect and accept unknown certificates | Credential-free TLS inspection, bounded certificate details and explicit exact-endpoint SHA-256 review implemented; fixture verified, live qualification open ([report](reports/2026-11-08-connection-trust.md)) |
| P01-14 | Review changed certificate and replace trust | Previous/presented pin and stored certificate comparison, stale-review rejection and draft-only replacement implemented; no automatic save/login or trust propagation; fixture verified, live qualification open |
| P01-15 | Easy certificate-validation opt-out | Existing workflow; qualification open |
| P01-16 | Proxy/timeout options and richer connection diagnostics | Request timeout, Direct/captured Environment/HTTP CONNECT routing, transient proxy authentication, effective route and login/inventory/event failure stage implemented; unsupported HTTPS/SOCKS/PAC proxies refused explicitly; fixture verified, live/native proxy qualification open |

## 2. Inventory, navigation and inspection

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P02-01 | Pool/host/VM tree; VM and template tables | Existing workflow; qualification open |
| P02-02 | Network and SR tables | Dedicated SR/network columns, attachment/disk/interface details and entry points into storage/network administration implemented; fixture verified, live qualification open |
| P02-03 | Search by name/UUID/host; sort and virtualized rows | Existing workflow; qualification open |
| P02-04 | Persist sidebar/table column widths | Existing workflow; qualification open |
| P02-05 | Full VM general information | Captured General information view includes guest OS/tools/addresses, configured resources, runtime memory/vCPUs/flags, host UTC timestamps and clock-qualified elapsed boot time; fixture/native synthetic verified, live qualification open ([report](reports/2026-11-08-general-information.md)) |
| P02-06 | Full host/pool properties | Captured host hardware/software/runtime/authentication summaries and pool membership/storage/HA/network/security-policy summaries implemented; unreadable/stale data stays explicit, fixture verified; live qualification open ([report](reports/2026-11-08-general-information.md)) |
| P02-07 | VM disk/NIC and host/storage attachment graph | Typed VDI/VBD/VIF/PIF/PBD graph, VM devices, host interfaces/storage attachments and SR disk/VM/snapshot consumers implemented; fixture verified, live qualification open |
| P02-08 | Physical NIC/link/bond/VLAN inspection | Host/network details include PIF addressing/topology plus separately negotiated carrier, speed, duplex, NIC vendor/device/PCI path and advertised capabilities; missing/invalid values remain unknown, fixture verified; live qualification open |
| P02-09 | SR attachment/path and disk inspection | SR details display host attachments, disk capacity/flags, VM/snapshot consumers and bounded per-PBD multipath/path observations; fixture verified, actual path health and live qualification remain open |
| P02-10 | Live inventory/stale-state handling | Optional-class negotiation, isolated decode failures and bounded resync implemented; live compatibility/scale qualification open |
| P02-11 | Filtered/grouped/saved/cross-pool searches | Typed cross-pool text/type/power/host/pool/tag/folder/custom-field filters, grouped ordering and saved definitions implemented and fixture verified; [search report](reports/2026-11-08-cross-pool-search.md); live qualification open |
| P02-12 | Multi-object actions and context menus | VM multi-selection, reviewed same-pool power batches and whole-row/keyboard context menus implemented; Rune fixtures cover selection, virtual scrolling, stale targets and focus return ([menu report](reports/2026-11-08-inventory-context-menus.md)). Other bulk workflows and native qualification remain open ([power report](reports/2026-11-08-bulk-power-ui.md)) |

## 3. VM power and recovery

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P03-01 | Start, clean shutdown and reboot | Existing workflow; qualification open |
| P03-02 | Force shutdown/reboot | Reviewed force shutdown/reboot implemented; separate running-task recovery is tracked in P03-07. Live qualification open |
| P03-03 | Pause/unpause and suspend/resume | Existing workflow; qualification open |
| P03-04 | Start/resume on a chosen eligible host/home server | Chosen-host start/resume and fresh compatibility preflight implemented; live placement qualification open |
| P03-05 | Explain unavailable actions | Host block messages preserved, current diagnostic context enforced and contradictory blocked/allowed operations refused; lab RBAC/state qualification open |
| P03-06 | Show current tasks and administrative action blocks | Current operations, administrative blocks, per-action diagnosis and pool-wide task Activity workspace implemented; fixture verified, live qualification open |
| P03-07 | Reviewed force recovery with running tasks | Captured task inspection, separately reviewed local operation cancellation, and fresh task/state/block/RBAC/assert-valid force admission implemented; unresolved reservations preserved and other-client tasks untouched. Protocol/Rune verified; live qualification open ([report](reports/2026-11-08-vm-recovery.md)) |
| P03-08 | Recovery-mode boot | Existing HVM guest CD/network boot with journaled exact boot-order restoration and separate provenance-checked interrupted-override restoration implemented; PV/firmware conversion refused. Protocol/Rune verified; guest/media and live-host qualification open ([report](reports/2026-11-08-vm-recovery.md)) |
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
| P04-10 | Full-screen, detached and multiple console windows | Fullscreen and up to eight independent native VM/host console windows implemented, sharing existing pool sessions with isolated input/frame/recovery state. Window manager, bounded reuse and stale-dialog guards fixture verified; live multi-monitor/guest qualification remains open ([report](reports/2026-11-08-independent-console-windows.md)) |
| P04-11 | Save console screenshot | Explicit native save dialog and background PNG export of the captured frame implemented; private atomic file output and console-switch fencing fixture verified, native qualification open |
| P04-12 | Host text console; guest SSH/RDP/external viewer | Direct host control-domain RFB plus reviewed guest SSH/RDP/custom local-client launch implemented. Captured VM/pool/generation/address, separate argv, bounded local declarations and independent guest authentication fixture verified; live guest/client qualification open ([host console](reports/2026-11-08-host-console.md), [guest tools](reports/2026-11-08-external-guest-tools.md)) |

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
| P05-11 | Export snapshot as template/backup | Reviewed halted disk-snapshot and memory-checkpoint XVA export implemented; explicit disks-only/memory choice, source/parent/memory identity and private atomic output fixture verified. Snapshot-chain coverage and live qualification remain open; see P11-07 |
| P05-12 | Ancestry tree, preview image, storage/size detail | Typed ancestry tree/current base, captured disk/SR/checkpoint allocation detail and bounded authenticated saved JPEG/PNG preview loading implemented; protocol/Rune/native synthetic checks passed. Opt-in pre-snapshot console capture and private, provenance-checked PNG upload with content readback and no-replay recovery are implemented ([capture report](reports/2026-11-08-snapshot-preview-capture.md)); live qualification remains open ([inspection report](reports/2026-11-08-snapshot-inspection.md)) |
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
| P06-07 | VM HA restart priority/protection | Reviewed restart/best-effort/disabled priorities and server eligibility checks implemented; fixture verified, startup order/delay controls also implemented ([HA policy report](reports/2026-11-08-ha-policies.md)); live qualification open ([report](reports/2026-11-08-pool-administration.md)) |
| P06-08 | BIOS/UEFI/Secure Boot provisioning choices | Reviewed creation modes, halted UEFI Secure Boot policy and read-only guest/pool readiness implemented; installed-image BIOS/UEFI switches refused, fixture verified, live enforcement qualification open ([evidence](reports/2026-11-08-firmware-templates.md)) |
| P06-09 | HVM shadow-memory settings | Reviewed halted/live shadow-memory multiplier editing implemented with modern HVM identity and advertised capability checks; fixture verified, live qualification open |
| P06-10 | Cloud-config parameters/config drive | Reviewed halted-VM NoCloud FAT drive with private user-data/metadata/network-config, chosen SR/slot, direct upload and read-only attachment implemented; no automatic boot or guest execution claim. Local protocol/Rune fixtures verified; live guest qualification open. See [NoCloud report](reports/2026-11-08-cloud-init.md) |
| P06-11 | Guest tools status/installation media | Guest-reported tools/OS/address display and reviewed host-advertised/legacy tools ISO mounting into a running or halted VM implemented; exact CD/media/storage checks, partial failure and no-replay recovery fixture verified. Installer execution remains inside the guest; live qualification open ([report](reports/2026-11-08-guest-tools-media.md)) |

## 7. VM disks, removable media and NICs

Property editor evidence is in the [device property report](reports/2026-11-08-device-properties.md).

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P07-01 | List disks/attachments/device positions | VM disk/attachment/device-position view implemented; live storage qualification open |
| P07-02 | Create disk on selected SR | Reviewed halted-VM creation and attachment on a chosen eligible SR implemented; created disk UUID survives partial failure; fixture verified, lab qualification open |
| P07-03 | Attach existing disk | Reviewed halted-VM attachment of an unattached managed, non-shared user disk implemented, with explicit access mode and no ownership claim; fixture verified, lab qualification open |
| P07-04 | Detach disk without deleting data | Reviewed halted-VM detachment implemented; backing disk and data remain; fixture verified, lab qualification open |
| P07-05 | Activate/deactivate disk attachment | Reviewed existing disk attachment plug/unplug implemented with fresh ownership checks; lab qualification open |
| P07-06 | Resize disk | Grow-only ordinary VDI workflow supports detached disks and inactive/unreserved attachments on running or halted VMs, exact consumer review and driver-rounded size verification; attached online resize is not offered for the XCP-ng 8.3 API lineage; lab qualification open ([report](reports/2026-11-08-disk-administration.md)) |
| P07-07 | Delete disk | Reviewed halted-VM deletion of an exclusively owned, non-shared user disk implemented with current consumer checks and separate journaled attachment/data deletion; fixture verified, lab qualification open |
| P07-08 | Disk name/description, access mode, I/O priority and device position | Reviewed exclusive-disk metadata, halted access/position and advertised best-effort I/O priority implemented; per-setter recovery and map preservation fixture verified, live qualification open |
| P07-09 | Move/copy/migrate disk to another SR | Selected disks can relocate via reviewed same-pool VM migration, including current-host selection; standalone inactive ordinary VDI copy to chosen SR with verified new identity and separately journaled naming implemented; broader disk topologies and lab qualification remain open ([report](reports/2026-11-08-disk-administration.md)) |
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
| P08-03 | Installation source, copied BIOS and cloud-config options | ISO selection, inherited/BIOS/UEFI/Secure Boot modes and chosen-host BIOS identity copy with reviewed affinity side effect implemented for eligible templates; optional inline cloud-init inputs continue through an exact-created-VM configuration-drive review after verified creation; private draft, failed/unknown outcomes and selection/reconnect guards fixture verified; live qualification open ([report](reports/2026-11-08-creation-cloud-init.md)); legacy CoreOS plugin integration excluded |
| P08-04 | Instant/fast template creation | Fast clone to custom template implemented when the source/storage advertises support; fixture verified, lab qualification open |
| P08-05 | Clone versus full-copy VM | Reviewed halted source clone versus full copy to selected SR implemented, with VM/template result choice; fixture verified, lab qualification open |
| P08-06 | Convert VM to template | Reviewed halted ordinary-VM conversion implemented with fresh capability checks; fixture verified, lab qualification open |
| P08-07 | Copy/edit/delete custom templates | Copy, reviewed deletion and custom-template metadata/CPU/memory/topology/weight/shadow/boot/autostart/home-host/Secure Boot properties implemented; empty-template firmware changes require explicit review and built-in templates remain protected; fixture verified, lab qualification open |
| P08-08 | Delete VM record versus VM plus selected disks/snapshots | Record-only or exact selected owned-disk/halted-snapshot deletion implemented with current ownership checks; checkpoint deletion uses its existing separate flow; fixture verified, lab qualification open |
| P08-09 | Recover/cancel partial creation/deletion and clean owned resources | Durable steps, read-only recheck, stop controls, unknown acknowledgment and created UUIDs implemented; retained VMs use reviewed deletion and detached ordinary VDIs can be explicitly deleted using terminal local creation proof plus fresh consumer checks; unproven/shared/snapshot/CBT resources remain refused, lab qualification open ([report](reports/2026-11-08-disk-administration.md)) |

## 9. Storage repositories

Implementation boundaries and evidence: [storage lifecycle report](reports/2026-11-08-storage-lifecycle.md).

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P09-01 | SR name/type/capacity/usage | Existing workflow; qualification open |
| P09-02 | Probe/discover targets | Read-only backend capabilities, NFS export, iSCSI IQN/LUN and typed HBA device enumeration plus bounded SR discovery implemented; local EXT/LVM accepts an operator-provided by-id path and probes both repository types with captured inputs, without claiming physical device enumeration or blank/unused proof; fixture verified, lab qualification open |
| P09-03 | Create SR | Reviewed shared NFS, LVM-iSCSI (including CHAP/mutual CHAP) and LVM-HBA/Fibre Channel creation implemented. HBA requires discovered exact host/device identity and destructive LUN confirmation; fresh probe refuses known existing repositories/use. Tracked tasks, verified new identities and journalled temporary Secret cleanup; reviewed local EXT/LVM creation from an explicitly declared by-id device with data-loss acknowledgement and exact-path confirmation implemented; physical device identity/capacity/use are not fully established by XAPI; further backends pending; fixture verified, lab qualification open |
| P09-04 | Add ISO library | ISO over NFS and explicit SMB 3.0 creation with optional transient authentication implemented; no SMB1 fallback; fixture verified, lab qualification open |
| P09-05 | Introduce/attach existing SR | Reviewed introduce by exact UUID, separate selected-host PBD creation and attach implemented; existing UUIDs are never implicitly forgotten; authenticated new attachments require fresh CHAP/SMB credentials and verify host-owned Secret copies; local EXT/LVM introduction and single-host attachment checks implemented; fixture verified, lab qualification open |
| P09-06 | Detach/reattach SR | Reviewed selected-host PBD unplug/plug implemented with fresh SR/host identity and consumer checks; attached status explicitly does not imply healthy storage; fixture verified, lab qualification open |
| P09-07 | Repair SR connectivity | Reviewed existing-PBD multi-host repair plus missing-PBD reconstruction from a captured supported shared-storage source implemented; coordinator first, exact graph/configuration checks, provenance-linked server credential clones without value reads, durable partial/unknown outcomes and separately reviewed restoration. Custom/host-dependent configurations remain incomplete; fixture verified, live qualification open. [Report](reports/2026-11-08-storage-graph-repair.md) |
| P09-08 | Rescan/refresh SR | Reviewed asynchronous rescan with durable task tracking, cancellation and reconnect recovery implemented; synthetic fixtures verified, lab qualification open |
| P09-09 | Forget versus destroy SR | Separate exact-UUID reviews implemented with captured disk set, fresh detach/consumer/default/HA dependency checks and tracked tasks; UI removal limited to 100 reviewed disks; fixture verified, destructive lab qualification open |
| P09-10 | Choose default SR | Reviewed pool default SR selection with fresh prior-value and user-storage checks implemented; lab qualification open |
| P09-11 | SR name/description edits | Reviewed metadata editing with fresh conflict checks and explicit partial outcomes implemented; lab qualification open |
| P09-12 | Multipath settings/path inspection | Bounded path inspection and reviewed standard multipathing implemented with complete-host detach/settings/replug orchestration on disabled empty maintenance hosts, HA off and active storage consumers refused; originally detached subset preserved, no automatic compensation after uncertainty. Custom equipment/configuration and live path qualification remain open. [Report](reports/2026-11-08-storage-graph-repair.md) |
| P09-13 | Trim/reclaim | Explicit SR_TRIM-capable attached-host reclaim via fixed trim plugin implemented with tracked task and positive-result evidence; unavailable plugin is a reported host error; fixture verified, lab qualification open |
| P09-14 | Read caching/provisioning options | Reviewed read-cache policy editing for NFS/EXT/SMB drivers advertising VDI_READ_CACHING implemented, preserving unrelated settings; effective active caching is not claimed. Driver-advertised thick/thin choice for new LVM-iSCSI/HBA repositories and persisted allocation readback implemented, with legacy driver-default behavior preserved. Driver THIN_PROVISIONING inspection implemented; no existing-SR conversion. Further backend provisioning options retained. Fixture verified, lab qualification open |
| P09-15 | Backend-specific wizards | Typed NFS, ISO-NFS, SMB3, CHAP/mutual-CHAP LVM-iSCSI and HBA/Fibre Channel forms, custom iSCSI port, authenticated target enumeration and HBA device selection/destructive review implemented; local EXT/LVM operator-path forms with dual repository probe, identity declaration and destructive review implemented; automatic physical device enumeration and other backend wizards remain pending |

## 10. Networks and host interfaces

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P10-01 | Network names/bridges | Existing workflow; qualification open |
| P10-02 | Create/delete internal network | Reviewed creation and unused internal-network deletion implemented; identity provenance, lost replies and fresh interface checks fixture verified; lab qualification open |
| P10-03 | Create/edit/remove external/VLAN network | Per-host VLAN creation/deletion, all-host VLAN creation and retagging of unused plain VLAN networks implemented, including tag 0 and preserved unplugged state. Exact topology/new UUIDs, partial completion and read-only recovery fixture verified; unique-tag static IPv4 management VLAN migration during bond creation/removal is now implemented ([report](reports/2026-11-08-management-vlan-bonds.md)); management retagging, broader IP-bearing topology migration and live qualification remain open ([report](reports/2026-11-08-network-topology.md)) |
| P10-04 | Name/description, MTU and automatic attachment | Names/descriptions, non-management attached-network MTU and automatic new-VM attachment settings implemented with fresh checks; nonstandard management MTU is unsupported by XCP-ng and explicitly refused |
| P10-05 | Create/destroy bond; change mode/hash settings | Create/destroy unused non-management bonds and mode/hash changes implemented; reviewed plain static IPv4 management-bond creation/removal now transfers the existing address with task/new-UUID and certificate-bound recovery checks ([report](reports/2026-11-08-management-bond-migration.md)). Unique-tag IPv4 management VLAN migration and replacement UUID review implemented ([report](reports/2026-11-08-management-vlan-bonds.md)); explicit local guest-NIC impact capture/migration/verification implemented ([report](reports/2026-11-08-management-bond-guests.md)); IPv6 and broader dependent topology migration remain open; fixture verification only |
| P10-06 | Scan/inspect physical NICs | Physical NIC inspection and asynchronous host rescan implemented; fixture verified, live qualification open |
| P10-07 | Plug/unplug host interfaces | Reviewed plug/unplug with fresh host/interface identity and management/protected-interface unplug refusal implemented; fixture verified, live qualification open |
| P10-08 | Change management interface and IP/netmask/gateway/DNS | Non-management IPv4/IPv6 mode/address/gateway/DNS editing implemented; IPv6 requires an already enabled host address family. Reviewed static management IPv4/IPv6 changes and moves between physical interfaces, existing bond masters and VLANs over physical/bond interfaces with authenticated recovery-route verification implemented; DHCP/IPv6 autoconf transitions with reviewed expected recovery IP and static return implemented ([transition report](reports/2026-11-08-management-address-transitions.md)); existing topology is captured and rechecked ([topology report](reports/2026-11-08-management-existing-topology.md)); plain static IPv4 management bond creation/removal now supports same-IP handover ([report](reports/2026-11-08-management-bond-migration.md)); unique-tag IPv4 management VLAN/bond handover now retains reviewed settings and replacement identities ([report](reports/2026-11-08-management-vlan-bonds.md)); reviewed local guest-NIC migration during these bond changes implemented ([report](reports/2026-11-08-management-bond-guests.md)); automatic address discovery, standalone physical/VLAN same-IP transitions and IPv6 topology migration remain open; fixture verification only |
| P10-09 | Additional management/storage interfaces and purposes | Secondary-interface IP configuration, purpose labels, explicit unplug protection and ordinary physical management moves implemented; existing bond/VLAN endpoint moves and configuration implemented; plain static IPv4 management bond creation/destruction and same-IP handover implemented; reviewed unique-tag management VLAN/bond migration implemented; reviewed local guest-NIC migration during those bond changes implemented; clustering, service-purpose and IPv6 topology migration remain open |
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
| P11-06 | XVA VM/template import/export | Native reviewed plain/gzip ordinary-guest import and halted VM/custom-template export implemented. Explicit checkpoint import verifies the entire memory/disk stream before dispatch and retains new suspended guests without resuming; identity/provenance/recovery fixtures verified ([checkpoint report](reports/2026-11-08-checkpoint-archives.md)). zstd, broader archive shapes and live qualification open |
| P11-07 | Snapshot export | Halted disk-snapshot and suspended memory-checkpoint XVA export implemented, with explicit disks-only/memory review and complete checksum/identity verification. Full-export endpoint omits ancestry; alternative snapshot-chain packaging and live qualification remain open ([evidence](reports/2026-11-08-checkpoint-archives.md)) |
| P11-08 | OVF/OVA appliance import/export | Reviewed ordinary HVM raw/VHD OVF/OVA import and VHD package export implemented with explicit SR/network mapping, private atomic publication and no-replay partial-resource recovery; parser/protocol/Rune fixtures verified. VMDK/conversion, advanced appliance settings and live qualification remain open; [report](reports/2026-11-08-ovf-ova-transfers.md) |
| P11-09 | Standalone disk-image import | Raw and fixed/dynamic VHD import into a new unattached disk implemented; matching disk export, private atomic files, captured size/storage and partial-identity recovery fixture verified. Differencing VHD/VHDX/QCOW2, conversions and live qualification remain open; [report](reports/2026-11-08-disk-image-transfers.md) |
| P11-10 | Conversion manager | Capability review and implementation queued |
| P11-11 | Transfer cancellation/expiry/partial-result recovery | XVA, checkpoint XVA, raw/VHD and ordinary OVF/OVA stream/task progress, cancellation, expiry and no-replay recovery implemented; failed/unknown imports retain proven UUIDs and unresolved guards. Checkpoint recheck never sends an unsent cancel or transfer, and late task/created-memory evidence preserves recovery/acknowledgement. Conversion formats and live qualification open |

See the [XVA transfer increment](reports/2026-11-08-xva-transfers.md) for format limits, trust and unresolved import cleanup.

## 12. Host administration

Implementation and qualification evidence: [host administration report](reports/2026-11-08-host-administration.md).

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P12-01 | Host hardware/software properties | Host General information adds BIOS/chipset, authentication and boot/toolstack elapsed observations to software/CPU/memory and attachment details; fixture/native synthetic verified, live qualification open |
| P12-02 | Rename/description | Reviewed host metadata editing implemented with fresh conflict checks and explicit partial outcomes; lab qualification open |
| P12-03 | Enable/disable scheduling; enter/exit maintenance | Reviewed disable/evacuate/maintenance marker and enable/exit workflows implemented with exact guest review and per-step outcomes; fixture-verified, lab open |
| P12-04 | Evacuate host with destination checks | Reviewed guest UUID set, fresh blockers/possible-host checks and authoritative assert_can_evacuate before disable and evacuation; no automatic HA reduction or guest shutdown; fixture-verified, lab open |
| P12-05 | Shutdown/reboot host | Reviewed shutdown/reboot for disabled empty hosts; separate handover required for multi-host coordinators; task success/offline or newer boot marker verifies outcome; fixture-verified, lab open |
| P12-06 | Restart toolstack | Reviewed toolstack restart with pending-task preflight and newer agent-start marker recovery; fixture-verified, lab open |
| P12-07 | Host power-on/configuration | Power-on using configured method, disabled/Wake-on-LAN and reviewed IPMI Secret-backed configuration with ownership-aware cleanup implemented; legacy iLO/DRAC absent from audited XCP-ng 8.3 lineage, custom plugins not edited; fixture-verified, lab open ([report](reports/2026-11-08-host-power-credentials.md)) |
| P12-08 | dom0 memory | Reviewed exact-control-domain memory change with current bounds/physical limit checks and fresh verification; fixture-verified, lab open |
| P12-09 | Host autostart | Explicit pool-wide VM autostart review implemented; preserves per-VM flags and does not start VMs immediately; fixture-verified, lab open |
| P12-10 | Change host/root password | Fixture implementation: reviewed pool-wide local-root change, verified coordinator login, actor reconnect-credential rotation/old-alias invalidation, optional exact-profile SecretService update; other member propagation explicitly unverified. [Evidence](reports/2026-11-08-root-password.md) |
| P12-11 | Forget/remove dead host | Reviewed offline/disabled/empty non-coordinator removal with independent power-off acknowledgement and HA disabled; retains SR records/data; fixture-verified, lab open |
| P12-12 | Host configuration backup/restore | Implemented: exact-host private filesystem backup and reviewed backup-partition staging for installer-assisted recovery. This does not restore the running host or pool database. Strict trusted-archive inspection, UUID/version/SHA-256 binding, exact TLS peer and owned-task evidence; disposable-lab qualification remains. [Evidence](reports/2026-11-08-host-recovery-staging.md) |
| P12-13 | Logging destination/monitoring settings | Reviewed remote syslog destination and separately journaled reconfigure implemented; further monitoring settings remain queued; fixture-verified, lab open |
| P12-14 | Crash dumps and support bundles | Partial: advertised support categories, selected-host crashdump metadata, private support/log bundles and reviewed retained-crashdump deletion with durable completion evidence. Raw dump transfer remains open; target API has upload/destroy, no direct raw-download endpoint, and logs exclude raw cores. [Downloads](reports/2026-11-08-host-backup-support.md), [deletion](reports/2026-11-08-host-crashdump-deletion.md) |

## 13. Pools, HA, identity and server security

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P13-01 | Create/dissolve pool | A standalone host is already a pool; naming, individually reviewed ejection and joining a standalone host to an independently connected pool implemented. Coordinated dissolve remains queued; live qualification open ([join report](reports/2026-11-08-pool-join.md)) |
| P13-02 | Join/eject/remove pool member | Reviewed standalone-host join and ejection of disabled, evacuated non-coordinator members with empty local guest storage implemented; offline removal is implemented with the P12-11 disabled/empty/non-coordinator and independent power-off checks; live qualification open ([join report](reports/2026-11-08-pool-join.md), [removal report](reports/2026-11-08-host-administration.md)) |
| P13-03 | Pool name/description | Reviewed metadata editing with fresh conflict checks and explicit partial outcomes implemented; lab qualification open |
| P13-04 | Planned coordinator handover during evacuation | Explicit reviewed handover to a live enabled member implemented; HA must already be disabled. Separate from evacuation; live qualification open |
| P13-05 | Enable/configure/disable HA; heartbeat SR selection | Reviewed enable/disable, heartbeat SR selection, failure tolerance, overcommit policy and persistent timeout default implemented with fresh server checks and durable positive setter receipts. Timeout edits require disabled HA and respect configuration precedence; fixture verified, live qualification open ([report](reports/2026-11-08-ha-policies.md)) |
| P13-06 | HA status and VM restart priorities | Typed HA status, heartbeat storage, current plan/overcommit, three VM restart priorities and shared startup order/start/shutdown delays implemented. Exact VM/vApp membership and old/new value reviews are fixture verified; actual failover and delay behavior remain live qualification ([report](reports/2026-11-08-ha-policies.md)) |
| P13-07 | Migration compression/GPU placement policy | Reviewed migration compression implemented for advertised fields; GPU placement is excluded specialized hardware scope; live qualification open |
| P13-08 | Pool clustering | Unsupported on XCP-ng: upstream clustering relies on unavailable proprietary software ([primary evidence](https://docs.xcp-ng.org/installation/upgrade/)); no unsupported RPC workflow is offered |
| P13-09 | Join/leave directory authentication | Reviewed AD join and local-only/credentialed leave implemented with transient credentials, exact member checks and partial/unknown recovery; fixture verified, live qualification open ([report](reports/2026-11-08-pool-security.md)) |
| P13-10 | User/group membership and role assignment | Directory resolution, subject creation/removal and exact reviewed role edits implemented for explicit RBAC support; restricted/unknown RBAC creation is refused to avoid automatic administrator grants; live qualification open |
| P13-11 | RBAC-aware eligibility and credential elevation/reconnect-as | Effective session permissions, ordinary RPC denial guard, power eligibility, and upfront required-write review/first-write guards for VM settings/devices and storage/network/host/pool administration implemented. Exact captured steps avoid false denials on unchanged fields. Protected keys, conditional specialized workflows and HTTP eligibility remain explicit unknown; full VM lifecycle/snapshot/migration and other dedicated wizard requirements remain open. Reconnect-as remains session-only; fixtures verified, live qualification open ([session report](reports/2026-11-08-session-permissions.md), [review requirements](reports/2026-11-08-permission-reviews.md)) |
| P13-12 | Install/reset host certificate | Reviewed PEM/PKCS8 installation and self-signed reset implemented with SHA-256 checks, transient private keys, native file pickers and no automatic trust change; fixture verified, live qualification open |
| P13-13 | Enable pool TLS verification/rotate pool secret | Explicit reviewed TLS enable and pool-secret rotation implemented with durable completion receipts; rotation refuses enabled HA or unreachable members; live qualification open |
| P13-14 | Legacy SSL mode | Unsupported on primary target: legacy SSL and TLS 1.0/1.1 were removed in XCP-ng 8.2 ([release notes](https://docs.xcp-ng.org/releases/release-8-2/)) |

## 14. Tasks, metrics, alerts and diagnostics

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P14-01 | Track local power/snapshot progress/completion | Durable task tracking extended to placement, checkpoints and device actions; lab qualification open |
| P14-02 | Unknown outcomes, recheck/acknowledge and cancellation | Existing workflow; qualification open |
| P14-03 | Contextual error explanations/remediation | Administrative messages, fresh action diagnostics, private live-client inspection and offline error-specific explanations in Activity/VM/resource history implemented. Captured outcome context keeps unknown, acknowledged and partial results distinct; unrecognized codes remain explicit. Synthetic/Rune checks verified, live qualification open ([report](reports/2026-11-08-error-guidance.md)) |
| P14-04 | Pool-wide tasks from other clients | Activity task workspace with filtering, typed details and reviewed cancellation; fixture-verified, live qualification open ([report](reports/2026-11-08-activity.md)) |
| P14-05 | Search/filter/dismiss history and inspect full task details | Searchable VM/resource history, terminal-only local hide/restore, typed task details and private JSON export implemented; raw task/debug payloads excluded, native/lab qualification open ([report](reports/2026-11-08-activity.md)) |
| P14-06 | Multi-step progress/partial outcomes/owned cleanup | Durable ordered property, device, provisioning, deletion and infrastructure steps with partial/unknown outcomes implemented. Reviewed retained-VM deletion and detached ordinary-disk cleanup use local creation proof and fresh consumer checks (P08-09); unsupported/unproven/shared/special disks remain refused. Fixture verified, live qualification open |
| P14-07 | VM/host CPU/memory graphs | Direct RRD subscriptions and VM/host performance charts implemented; synthetic transport/render checks, live qualification open |
| P14-08 | Disk/network throughput/latency graphs | Host-advertised disk/network sources rendered with units and explicit sample gaps; source availability and live qualification remain host-dependent |
| P14-09 | Custom graphs/data-source selection/historical ranges | Per-view source selection, bounded 10-minute/2-hour/week/year ranges with actual resolution, and validated VM/host graph presets implemented; unavailable preset sources are explicitly refused, fixture/native synthetic checks passed; live qualification open ([report](reports/2026-11-08-graph-presets-reports.md)) |
| P14-10 | Host/pool messages/alerts: list/filter/dismiss/export | Activity list/filter, content-bound reviewed server dismissal and private native-dialog JSON export implemented; fixture-verified, native/lab qualification open ([report](reports/2026-11-08-activity.md)) |
| P14-11 | Alarm thresholds/intervals/notification-email settings | Reviewed VM/host/control-domain/SR thresholds, duration/repeat intervals, pool email/relay/language and explicit host perfmon refresh implemented. Bounded XML preserves unrelated/custom settings; typed revisions, fresh identity/RBAC checks and durable no-replay refresh receipts are fixture verified. Actual evaluation/service/SMTP delivery remain live qualification ([report](reports/2026-11-08-performance-alarms.md)) |
| P14-12 | Resource reports/support-status export | Activity JSON export, captured filtered cross-pool resource reports with freshness/coverage metadata, and reviewed selected-host support/log bundle downloads implemented; private bounded file publication fixture verified. Live qualification open ([reports](reports/2026-11-08-graph-presets-reports.md), [support bundles](reports/2026-11-08-host-backup-support.md)) |

## 15. Organization and grouped workflows

See the [organization increment](reports/2026-11-08-organization.md) for captured-object scope and shared-map concurrency limits.

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P15-01 | Tags: create/rename/delete/assign/remove/group/search | Reviewed tag assignment/removal and captured-pool rename/delete, tag group filters and metadata search implemented; synthetic fixtures verified, lab qualification open |
| P15-02 | Folders: create/rename/delete/move/remove objects | Reviewed compatible folder registry, subtree rename/removal and selected object moves implemented; exact captured objects and per-step partial outcomes, lab qualification open |
| P15-03 | Custom field definitions/values | Bounded compatible text/date definitions and selected values implemented; definition deletion reviews value cleanup and refuses unreviewed consumers; fixture verified, lab qualification open |
| P15-04 | vApps: create/edit/delete/membership | Reviewed empty-group creation, metadata edits, per-VM membership and empty-group deletion implemented; fresh identity/graph and lost-reply fixtures verified, lab qualification open |
| P15-05 | vApp ordered startup/shutdown and delays | Per-member order/delay edits and explicit ordered start, clean or force shutdown implemented; exact captured members, durable completion and partial-child fixtures verified, lab qualification open |
| P15-06 | Bulk properties/delete/move/export | Reviewed 1–32 same-pool VM/custom-template property batches implemented with exact per-target changes, explicit unchanged omissions and durable serial setter outcomes. Whole-selection/fresh-field checks and read-only no-replay recovery fixture verified; bulk delete/move/export and live qualification remain open ([report](reports/2026-11-08-bulk-properties.md)) |
| P15-07 | Search definition import/export/resource export | Reviewed bounded JSON search-definition import/export and captured resource-result export implemented with native pickers; legacy XenCenter XML interoperability remains open ([report](reports/2026-11-08-graph-presets-reports.md)) |

## 18. Linux desktop and extensibility

| ID | Workflow | Delivery state |
| --- | --- | --- |
| P18-01 | Linux executable/desktop-entry archive | Manifest-verified portable archive, installer and pinned-runtime AppImage implemented; local synthetic native render/installation smoke passed, clean distribution qualification open ([evidence](reports/2026-11-08-linux-packaging.md)) |
| P18-02 | Clean-system installation/offline cold start | Qualification open |
| P18-03 | Portable/distro package, upgrade and uninstall | User-prefix installation, retained releases, atomic active-version selection and ownership-checked removal implemented; AppImage with source/notices and Ubuntu CI packaging added; local fixture/native package smoke passed, clean-system qualification open ([evidence](reports/2026-11-08-linux-packaging.md)) |
| P18-04 | Native Wayland/X11/GPU compatibility | Existing subset; remaining work queued |
| P18-05 | HiDPI/multimonitor/compact windows | Existing subset; remaining work queued |
| P18-06 | Keyboard navigation/focus/Orca | Qualification open |
| P18-07 | Native file selection/clipboard integration | Explicit console clipboard and native CA/screenshot/transfer/search/activity pickers implemented with cancelled/stale-callback fixtures; real portal/clipboard/desktop qualification remains open |
| P18-08 | General/confirmation/console/display options | Persistent system/light/dark appearance, reduced motion, optional routine-power review and new-console fit/reconnect defaults implemented alongside per-profile options; compact/wide Rune fixtures and native synthetic renders verified, real desktop qualification open ([evidence](reports/2026-11-08-desktop-options.md)) |
| P18-09 | External tools/plugins | Versioned bounded trusted local executable/argv declarations, editable SSH/RDP presets, per-launch command/target review and local process status implemented and fixture verified. Automatic discovery, declarative extension views and richer plugin APIs remain open; Windows plugin compatibility excluded ([report](reports/2026-11-08-external-guest-tools.md)) |
| P18-10 | Localization/integrated help | Offline contextual-topic Help implemented for connections, recovery, consoles, storage and private diagnostics; translation catalogs/localization remain queued ([evidence](reports/2026-11-08-desktop-options.md)) |
| P18-11 | Multiple client instances/shared-pool ownership | Authenticated pool alias ownership, exclusive state-directory process ownership and private same-user activation handoff with Wayland/X11 tokens implemented; startup isolation and cross-process fixtures verified, compositor focus qualification remains open ([report](reports/2026-11-08-desktop-activation.md)) |

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
