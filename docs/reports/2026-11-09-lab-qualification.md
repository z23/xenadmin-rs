# Disposable lab qualification, first live run

This report records the first live XCP-ng runs of the client. Each scenario names the
ledger rows it exercises and separates the observed host behaviour from what remains open.
Raw evidence (sanitized JSON written by the live harness) stays outside the repository.

## Identity

- Application: workspace version 0.17.0, development branch after `86d0565` (CI deadline
  scaling and lab harness changes merged to main).
- Lab: three nested KVM guests from the [harness manifest](../../lab/README.md), manifest
  `9e8e4f49`, isolated libvirt network without forwarding, hosts installed unattended
  from `xcp-ng-8.3.0-20260806.iso` (SHA-256
  `1d06e2d88a0fbf9a38384aa46637fd5d1ad568ba0df7ab3ed95fb047655f22e6`, publisher checksum
  list signature verified against the documented XCP-ng key fingerprint).
- Hosts report product version 8.3.0. Each host has a 64 GiB SATA system disk with a local
  EXT repository and an untouched 32 GiB SATA data disk.
- Workstation: Linux 7.2.15 (Arch-derived), libvirt 12.8.0, QEMU 11.1.2, AMD nested KVM.
- Live harness: `crates/xenadmin-core/tests/lab_live.rs`, run with the manifest and a
  lab-only password file; targets come only from the manifest.
- Native desktop run: the debug `xenadmin-rs` binary inside a nested Hyprland 0.56.2 session
  on the same workstation (1600x1800 headless output, 1558x1758 window), driven over AT-SPI
  through libatspi and the compositor's `sendkeystate` dispatcher, which delivers physical
  key events; screenshots through `grim`. The lab contains no Xen Orchestra service.

## Workstation prerequisites found during setup

- Cloudflare WARP captured every destination through a policy-routing table consulted
  before the main table and dropped non-tunnel traffic in its own nftables input and
  output chains. The lab could not fetch answer files or reach XAPI until WARP was
  disconnected. The harness now verifies that every lab address resolves to the lab
  bridge after the bridge exists.
- ufw's default policy dropped DHCP requests to the bridge without logging them (its
  after-rules skip DHCP ports). Rules allowing the bridge, DHCP and the answer-file port
  were required.

## Scenarios

| Scenario | Rows | Result | Observation |
| --- | --- | --- | --- |
| Direct recovery, read-only | P01-01, P01-11, P01-13, P19-01 | Passed | Credential-free certificate inspection, exact-endpoint pin, read-only login and current inventory on all three standalone hosts; wrong pins and the untrusted self-signed certificate were refused before any credential was sent |
| Reviewed pool join | P13-02 | Passed after fixes | First attempt was refused locally and the second left an unknown outcome (findings 1 and 2). After the fixes a standalone host joined the two-host pool in eleven seconds: `pool.join` receipt recorded, destination membership verified, member visible on the destination, the old standalone connection kept its original pool identity |
| Guest provisioning | P11-09, P08-02, P07-03, P07-02, P03-01, P04-01 | Passed after fix | A 68 MiB isohybrid Alpine image imported as a raw disk into the coordinator's local repository, a guest created from the generic template without a disk, the image attached as its first disk, a blank data disk added, the guest started, and the embedded console delivered 720x400 frames through the authenticated tunnel; the first attempt stopped at the memory setter (finding 3) |
| Console input, snapshot and reviewed restore | P04-02, P05-02, P05-03, P05-04, P19-04, P19-06 | Passed | Keyboard input through the authenticated tunnel logged into the guest and configured its network; the guest wrote marker A into its data disk's first sector and reported it to a listener on the isolated bridge; a running-guest disk snapshot succeeded; the guest changed the marker to B and reported it; the reviewed restore cleanly shut the guest down, reverted configuration and disks and started it; after the fresh boot a read-only pass reported A and never B. Earlier passes were invalid because the live guest image lacks filesystem tools, not because of the client |
| Guest power matrix | P03-01, P03-02, P03-03 | Passed | Clean shutdown, start, pause, unpause, hard reboot, hard shutdown and start each completed with the expected power state. The tool-less live guest advertises no clean reboot, suspend or resume, and the client recorded those as unavailable rather than attempting them |
| Reviewed deletions | P05-08, P08-08 | Passed | Six snapshots, each with its two exclusively owned disks, were deleted through the reviewed snapshot deletion and verified absent; the device-less guest left by the partial creation was deleted through the reviewed VM deletion with no disks selected |
| Console across a hard reboot | P04-07 | Partly | The core console tunnel reported closure within about 200 ms of the hard reboot; once the guest was running again a reopened console delivered frames. The desktop's bounded automatic reconnect lives above the core facade and still needs a desktop-level run |
| Live migration with storage relocation | P11-01, P11-03, P11-05 | Passed after fixes | The running guest moved to another member and back with both disks relocated to each member's local repository over the client's chosen transfer network; the first attempts were refused by the client (no transfer network given) and then by the host (finding 4). With every member's certificate pinned, the console opened on the destination host delivered frames; without the member pin the tunnel was refused, as designed |
| Guest and host metrics | P14-07, P14-08 | Passed after fix | Direct RRD subscriptions for the running guest (44 advertised sources) and its host (68) delivered 120-sample series at a five-second step for CPU, memory and disk-queue sources; the first attempt failed on the data-source listing (finding 5) |
| VM property edits | P06-01, P06-02, P06-04 | Passed | The description was edited on the running guest; maximum and startup vCPUs and all four memory limits were edited while halted; each setter was verified by a fresh read and the inventory read the new values back; the guest was started again |
| Reviewed maintenance and ejection | P12-03, P13-02 | Passed | Member entered maintenance (disabled observed), reviewed ejection task completed, member left the coordinator's inventory and returned as a standalone pool with a new identity |
| Native desktop session, no XO anywhere | P01-01, P04-02, P18-06, P19-02 | Passed, finding 6 | Cold start from the saved profile, session-password login through the connect dialog, inventory, UUID search, details and Open console in the Rune window; the embedded console rendered the guest, physical key events logged in as root, configured the guest's address and ran `wget` twice against a listener on the isolated bridge, which logged both requests from the guest including the data-disk marker A; shifted characters arrived correctly once Shift was sent as a physical key carrying its modifier mask. Pointer input was not exercised: the nested session has no pointer injection and a text-mode guest shows no pointer effect |
| Coordinator power loss under a live session | P01-03, P19-05 | Passed, finding 7 | With the lab guest halted on the coordinator, the coordinator domain lost power; the client left Ready after 31 s with "The host did not respond within the request timeout", refused a write while stale before sending anything, retried with increasing backoff through nine generations, and was Ready again 147 s after the domain started, under generation 10, with no replayed or added operation records; the guest, halted by the crash, started again through the recovered session. An ACPI shutdown request sent to the nested dom0 earlier was ignored, and the client correctly stayed Ready while the host kept serving |
| Snapshot clone to VM and template | P05-08, P05-09, P08-06, P08-08 | Passed after fix | A snapshot of the halted guest was cloned into a new VM and into a template, each with three disks of its own and none shared with the source; the new VM started, and its console typed commands that reported marker A from its copied data disk to the listener; the VM was shut down and converted to a template; both templates were deleted with their own disks and the source snapshot with its disks, leaving the guest's disks and the VM count as before. The first attempt stopped because the new VM never offered Start (finding 8). Leftovers of the interrupted attempts were removed first through the same reviewed deletions |
| Interrupted snapshot create across a restart | P05-02, P19-04 | Passed | A snapshot create was cut off by closing the client 150 ms after submission, while its record was Tracking a host task; a new client on the same state directory recovered the record as OutcomeUnknown ("The connection was closed") and did not replay it; an operator recheck moved it to Tracking and then Succeeded ("Snapshot created and identity verified") in two seconds through the saved task reference; the host held exactly one snapshot of that name, which the reviewed deletion then removed |
| Disk lifecycle on the halted guest | P07-04, P07-06, P07-07, P07-08, P07-09 | Passed | A 256 MiB scratch disk was created on the guest, renamed and described, moved to another device position, detached with its data kept, grown to 512 MiB while detached, copied to another member's local repository and the copy deleted through the owned-disk cleanup; after reattachment the deletion through the new, non-owning attachment was accepted for review and rejected at dispatch with the disk and attachment untouched, the disk was detached again and deleted through its creating operation; a second disk was deleted together with its owning attachment; the guest's own disks were unchanged and it was started again. The dispatch refusal reads "shared, attached elsewhere, or unsuitable" where the review screen would have said the attachment does not own the disk |
| Login through a member and pool sharing | P01-02, P01-07, P01-11 | Passed | A read-write login addressed to the second member followed the coordinator redirect and became Ready on the coordinator's endpoint with all three hosts, the member and coordinator recorded as verified members; a second connection to the coordinator in the same client saw the same pool and inventory revision; closing the first handle disconnected the shared pool session, as the profile editor states. With only the member's certificate pinned, the redirect to the coordinator was refused by TLS before any credential left the client; with both pinned, a wrong password was refused with SESSION_AUTHENTICATION_FAILED |
| Internal network and NIC lifecycle | P07-12, P07-13, P10-01, P10-02, P10-04 | Passed | An internal network was created on the pool (bridge `xapi0` observed), its MTU lowered to 1400 and its name and description changed, each read back; on the halted guest a NIC on that network was added with a generated MAC at device 1 and removed again; the network was deleted and the network count returned to its previous value; the guest's own NIC and network were untouched and it was started again |
| Cross-pool copy with both pools connected | P01-06, P11-04, P13-02 | Passed | The third host was ejected again through the reviewed ejection and returned as a standalone pool; one client held the two-host pool and the standalone pool at once; the halted guest was copied across pools over the lab network to the standalone host's local repository with its NIC mapped to the same-named network, in 18 s, with new VM, disk and MAC identities and the source guest retained; the copy started on the other pool and its console typed commands that reported marker A from the copied data disk; the copy was shut down and deleted with both disks, and the guest was started again on its own pool |
| Cross-pool move round trip | P11-04 | Passed after fixes | The halted guest was moved from its pool to the standalone host and back (and once more each way after an interrupted attempt left it on the standalone host), each leg in 25 to 35 s with both disks relocated, the VM UUID preserved by XAPI, the source pool verified empty of the VM, its attachments and its disks, and the destination verified with the reviewed placement; the guest started again on its home pool. The first attempts ended unknown or refused (findings 9 and 10) |
| XVA export and import across pools | P11-06 | Passed | The halted guest was exported uncompressed to a 74.6 MB XVA file on the workstation in 5.5 s (the allocated blocks of its 68 MiB image and 1 GiB data disks) with the export verified against the saved file; the file was imported into the standalone pool's local repository in 2.5 s as a new VM with two disks whose identities and storage were verified; the import was deleted with its disks, the file removed and the guest started again |
| Pool name and member enable state | P13-03, P12-03 | Passed after fix | The pool XCP-ng had left unnamed was renamed through the client once the review expected its empty label (finding 11); the client refuses to set an empty name by design, so the lab pool keeps a lasting name and later runs rename and restore it; the second member was disabled and enabled again with each state read back |
| Member toolstack restart and reboot | P12-03, P12-05, P12-06 | Passed after fixes | The empty second member entered maintenance; its toolstack restart was confirmed by a newer agent start time after 16 s once the client waited for it (finding 12); the restart had re-enabled the host with the marker still set, so maintenance was exited and entered again through the client; the reviewed reboot was confirmed by a newer boot time after 156 s; exiting maintenance then repeated the enable that XCP-ng refused with HOST_STILL_BOOTING until it succeeded, and the member was enabled with its marker cleared |
| Memory checkpoint and CD drive with guest tools media | P05-05, P05-08, P07-10 | Passed | XCP-ng 8.3.0 took a disk-and-memory checkpoint of the running, tool-less guest (the snapshot shows as Suspended and the guest kept running) and the reviewed deletion removed it with both disks and left no memory image behind; a CD drive was added to the halted guest, the host's advertised guest tools image inserted through the tools-media workflow and ejected, and the guest started again. An earlier attempt expected the checkpoint to be refused without tools; the host's behaviour, not the client, decided that |
| OVF export and import across pools | P06-05, P11-08 | Passed | The OVF writer represents only the `dc` boot order, so the halted guest's order was changed from `cd` through the reviewed boot-order editor; the guest was exported as a 73.4 MB OVA in 11 s with its package, disk formats and manifest verified; the package was inspected (one guest, two disks, one network) and imported into the standalone pool in 2.8 s with the network mapped by name and the new VM and disks verified; the import was deleted with its disks, the file removed, the boot order restored and the guest started again |
| Local storage lifecycle on the standalone host | P09-02, P09-03, P09-06, P09-08, P09-09 | Passed | The standalone host's untouched 32 GiB data disk was probed by its stable device path without formatting (no existing repository), a local EXT repository of 33.6 GB was created on it after the explicit data-loss declaration, rescanned, detached and reattached through its host attachment with each state read back, detached again and destroyed with no disks reviewed; a repository left by an interrupted attempt was detached and destroyed first through the same operations, since a destroy is refused while the repository is attached |
| Forget and introduce a local repository | P09-05, P09-09 | Passed | A local EXT repository on the standalone host's data disk was detached and forgotten, leaving its data on the disk; the non-formatting probe then found that repository by its UUID and size; it was introduced again by that exact UUID, given a host attachment, attached with the state read back, then detached and destroyed |
| Concurrent description edit refused | P06-01 | Passed | Two separate clients held the guest; the second changed its description, the first saw the change through events, and the first client's edit reviewed against the old description was rejected at dispatch with "VM properties changed since this editor opened. Reload them before saving" without touching the host; a fresh review by the first client then applied |
| Description edit interrupted before dispatch | P06-01, P19-04 | Passed | The client was closed immediately after submitting a description edit, while its record was still Submitting; a new client on the same state directory recovered the record as Rejected ("The connection was closed"), the host's description was unchanged and nothing was replayed. A setter whose reply is lost after dispatch could not be produced on the lab, because a single setter completes within milliseconds; that path stays covered by fixtures only |
| Snapshot restore interrupted mid-workflow | P05-03, P05-04, P19-04 | Passed, with an open observation | A reviewed restore (clean shutdown, revert, start) was cut off by closing the client while its shutdown step was being tracked; the restarted client recovered the workflow as OutcomeUnknown with the shutdown step unknown and the revert and start steps pending, and its read-only recheck repeated no write and submitted no remaining step; the unresolved workflow held the guest until the operator acknowledged it, after which the guest was started and the snapshot deleted. The recheck does not infer a finished clean shutdown from the guest being halted, so the step stays unknown; and in one of four attempts the restarted client's history did not show the record within five minutes and the guest had never been shut down, which could not be reproduced afterwards and remains open |
| Saved profiles for two pools | P01-05, P01-08 | Passed | Two profiles with reviewed certificate pins for every lab host were saved to a profile store and read back by a fresh store; both were connected at once from their saved options with session-only passwords and reached their distinct pools; both were disconnected; one profile was removed and a fresh store reloaded with the other |
| Chosen-host start, action diagnosis and home server | P03-04, P03-05, P03-06 | Passed | Every power action was diagnosed on the running guest (Start, Resume and Unpause unavailable, the rest available), once more while a clean shutdown was in flight (Start unavailable) and on the halted guest (only Start available); the server answered VM_BAD_POWER_STATE to the fresh validity check and the client reported the action as unavailable rather than attempting it. A chosen-host start on the other member was accepted, rechecked against the live host and refused by XAPI with VM_REQUIRES_SR because the guest's disks live on the resident host's local repository; the guest stayed halted and the refusal was recorded, not retried. A chosen-host start on the resident host succeeded and the guest ran there. The home server was set to the resident host and cleared again through the property editor, each verified on read-back |
| Suspend and chosen-host resume | P03-03, P03-04, P04-01 | Passed | The running tool-less guest advertised suspend (its first boot earlier in the day had not, so the power matrix recorded it as unavailable), the fresh diagnosis agreed and the suspend completed. A chosen-host resume on the other member was rechecked against the live host and refused with VM_REQUIRES_SR, leaving the guest suspended. The chosen-host resume on the resident host succeeded, the guest ran there and the console delivered a stable frame afterwards |
| Live property editors and snapshot metadata | P05-01, P05-07, P06-02, P06-03, P06-04, P06-09 | Passed after fixes, findings 13 and 14 | On the running guest the CPU scheduling weight and the dynamic memory range were changed and restored and read back; a live shadow multiplier edit was refused because XCP-ng 8.3.0 does not advertise it for this guest, and after finding 13 the refusal names the missing advertisement. While halted the vCPU maximum was raised, the cores-per-socket topology and the shadow multiplier were set, and after a start the host advertised the live vCPU change five seconds later and the live increase read back. A disk-only snapshot was created with a description, listed with it, renamed with a new description and deleted with its two disks. Restoring the vCPU maximum against the topology set earlier was refused by name (finding 14); the topology was reset, the counts and the multiplier restored, and the guest left running. An earlier attempt had left the guest halted with a topology its vCPU count could not satisfy (XAPI refused the start with VCPU_MAX_NOT_CORES_PER_SOCKET_MULTIPLE); the scenario now repairs that through the editor |
| Batch power actions over two VMs | P03-09, P05-10 | Passed | The guest's snapshot was cloned into a second VM and the guest halted. A reviewed batch start of both halted VMs, a batch clean shutdown of both running VMs and a batch start of the guest alone each completed with per-VM steps "Task success and resulting VM state verified" and the expected states read back. A batch naming a wrong expected state for the clone was refused at submission before any write and changed nothing. A batch submitted the moment both VMs showed Running was refused whole by the selection preflight ("The requested action is unavailable for this VM", no VM submitted) because XCP-ng 8.3.0 advertised `clean_shutdown` for the freshly started VMs only six seconds later; the scenario now waits for the advertisement as the UI does. The clone was deleted with its two disks and the source snapshot with its disks; a clone left running by the first attempt was halted and deleted through the same workflows |
| Other-client task inspection and cancellation | P14-04, P14-05, P14-02, P11-01 | Passed, observation 15 | One client started a reviewed live storage migration of the guest to the other member; a second client with its own state saw the `Async.VM.migrate_send` task in the pool-wide inventory 0.3 s after submission with its typed details (pending, progress 0, resident host, cancel advertised) and requested its cancellation through the reviewed task command. The host accepted the request, but the owning client destroyed the task on reading its result before the canceller's verification read, so the cancellation record stayed OutcomeUnknown ("task outcome is not yet confirmed"), a recheck left it unknown and the operator acknowledgement moved it to AcknowledgedUnknown and released the pool reservation. The first client's migration ended Cancelled ("Host task cancelled; original placement verified") after 32 s and the guest stayed running on its source host with both disks on the source repository. The synchronous `VM.assert_can_migrate` task was listed too; it advertises no cancellation and the client refused to cancel it. The client refuses a storage move while the guest has snapshots, so the scenario first deleted the checkpoint an earlier interrupted attempt had left |
| Recovery boot override and restore | P03-08, P04-01 | Passed | The halted guest was booted through the reviewed recovery override: the boot order "cd" was saved, "dn" (CD then network) written, the guest started and the original order restored while it ran, each step verified; the record states that execution of a recovery environment is not verified. The console delivered a frame of the VGA firmware banner while the firmware looked for media; with an empty CD drive and no network boot service the guest stayed in its firmware, did not advertise or honour a clean shutdown, and was hard-shut and started normally with the restored order "cd" read back |
| Disk attachment, HA priority, changed-block tracking and cloud-init drive | P07-05, P06-07, P07-11, P06-10 | Passed after a fix, finding 16 | On the running tool-less guest the data disk attachment advertised only attach, pause and unpause, and the reviewed unplug was refused without a host call. The HA restart priority was set to best-effort and restored to disabled through the pool workflow, each verified on read-back, with HA itself disabled on the pool. With the guest halted, changed-block tracking enabled on the data disk beforehand outside the client (XAPI accepted `VDI.enable_cbt` while the guest ran) was disabled through the device property editor and read back as disabled, after finding 16. A NoCloud drive with private user-data was built locally, uploaded as an 8 MiB disk on the resident host's local repository, attached read-only at slot 2 with the client's ownership marker and deleted again through the owning attachment (attachment removed, disk data deleted, both verified); the record states that guest execution is not verified and the guest was not started from it |
| Host and pool settings, host text console and Ctrl+Alt+Delete | P12-02, P12-13, P12-09, P13-07, P10-10, P04-12, P04-03 | Passed | The member's name and description, its syslog destination (set to the coordinator's address), the pool's VM autostart flag, migration compression and IGMP snooping were each changed through their reviewed workflows, verified on read-back and restored exactly. The coordinator's control-domain text console opened through the direct RFB path and, once the first frame had been painted in full, showed the XCP-ng console login screen with the host's address and certificate fingerprints (640x384). Ctrl+Alt+Delete sent as a key chord through the guest console rebooted the tool-less Alpine guest in place: the VM metrics start time changed within six seconds of the chord, the guest was Running again with its actions advertised, and the console showed a fresh OpenRC boot and login prompt |
| Coordinator handover round trip | P13-04, P01-02, P14-02 | Passed after a fix, finding 17 | The reviewed handover moved the coordinator role to the member and back. Each time the live session lost the host eleven seconds after dispatch while both toolstacks restarted, reconnected through two generations and was Ready on the new coordinator within fifteen seconds by following the host's coordinator redirect; a fresh login through the former coordinator's address was redirected the same way. The handover record was left OutcomeUnknown ("The connection was closed") because the session was lost during verification, and the operator's recheck on the new coordinator confirmed it ("Dispatched changes reconciled; unsubmitted steps were not replayed"). The guest kept running on its host and both hosts were enabled again before the next leg |
| Member certificate reset and trust replacement | P13-12, P01-14, P01-01 | Passed | The member's self-signed certificate was reset through the reviewed pool security operation and confirmed by a fresh host read; the fingerprint observed at the endpoint changed. A login pinned to the previous fingerprint failed within half a second with "TLS verification failed for https://192.168.254.12/. Verify this host's certificate and add its endpoint-specific fingerprint", before any credential was sent, and a login pinned to the new fingerprint reached Ready through the member's redirect to the coordinator. The lab's saved desktop profile still carries the previous pin for that member and will need the desktop trust review |
| Host archives and template lifecycle | P12-12, P12-14, P08-01, P08-04, P08-05, P08-07 | Passed after a fix, finding 18 | The member's support overview listed its version and 41 advertised categories with their privacy labels and no crash dumps. The private host configuration backup (740 MB gzip tar, 44,701 entries) streamed in 86 s and the support bundle of the default categories (43 MB) in 6 s; both were task-verified, inspected and saved privately, then removed once measured. The backup was first refused as malformed by the archive inspection (finding 18). With the guest halted, a fast clone created a custom template with its own two disks, the template's name and description were edited and read back, a full copy from the template produced a VM whose two disks lay on the chosen local repository and were shared with nothing, and the VM and the template were deleted with their disks; the guest's disks were unchanged and it was started again |
| VLAN lifecycle, interface rescan and management unplug refusal | P10-02, P10-03, P10-06, P10-07 | Passed | An internal network was created as the VLAN's target, a tag-100 VLAN interface was created on the coordinator's management NIC and appeared attached on that network, the host's physical interfaces were rescanned, a request to unplug the management interface was refused before submission with the interface still attached, the VLAN interface was unplugged and plugged with each state read back, the VLAN was deleted with its interface and the unused network was deleted |
| Scheduled-snapshot policy lifecycle | P05-13 | Passed after a fix, finding 19 | The pool's schedule support reported the coordinator's clock (UTC offset 0). A disk schedule was created disabled, the guest was added, the schedule enabled, run now through the synchronous plugin call (1.8 s; the schedule's last run time advanced and a snapshot named by the run appeared), disabled, emptied and deleted; the run's snapshot was deleted with its two disks. An enabled schedule with the guest left by the first attempt was disabled, emptied and deleted through the same workflows first |
| Inventory tables and certificate opt-out | P01-15, P02-02, P02-05, P02-06, P02-10, P09-01 | Passed | A read-only login with the explicit certificate-validation opt-out and no pin reached Ready. The inventory presented seven repositories with type, capacity, usage, sharing, attachment and disk counts (both 22 GiB local EXT repositories, the shared tools ISO library and the udev drives), two networks with bridge, MTU and interface counts, the guest's general information (vCPUs, memory limits and actual memory, firmware and boot policy, start time and clock-qualified elapsed boot time, no guest tools) and each host's product, build, Xen, CPU, memory, boot and agent start times and manufacturer, the pool's coordinator, default repository and policy flags, and 23 optional record classes negotiated as available |
| Force shutdown review with another client's running task | P03-07, P11-01, P14-02 | Passed, observation 20 | While a second client's storage live migration ran, the first client captured the VM's running `migrate_send` task in the force shutdown review and the host admitted the force: the task completed, the VM state was verified and no other server task was cancelled by the client. The host stopped the migration task and the owning client's record ended OutcomeUnknown ("placement is incomplete or cannot be verified"); its recheck left it unknown because the guest was halted on no host, so it remains for the operator to acknowledge. The guest's two disks stayed together on the source repository and it was started again |
| VM wizard with an ISO install source, UEFI firmware and Secure Boot policy | P08-02, P08-03, P06-08 | Passed after a fix, finding 21 | A UEFI VM was created from the generic template with the host's tools image as the installation source and a new 2 GiB disk on the coordinator's local repository: the halted VM carried the image in a CD drive at slot 1, the disk at slot 0, one NIC, one vCPU, 512 MiB and firmware uefi with Secure Boot off; its boot status read firmware uefi, policy off, guest readiness Disabled and pool readiness Ready. The Secure Boot policy was turned on and off through the halted-VM editor, each verified on read-back, and the VM was deleted with its disk while the shared tools image stayed. The creation was first refused because the client required the image's own type to be `iso` (finding 21) |
| Checkpoint restore resumes the memory image | P05-05, P05-03, P05-04, P04-02 | Passed | With the guest logged in through the console, a timed report was armed inside it (a 45 s sleep followed by a request to the workstation listener) and a disk-and-memory checkpoint was taken ten seconds later. The live guest delivered the report once, 51 s after arming. The reviewed restore of the checkpoint cleanly shut the guest down, restored its configuration and disks and resumed the restored memory, each step verified; 41 s after the resume the same report arrived a second time, because the sleep pending in the checkpointed memory fired again. The checkpoint was deleted with its two disks |
| Second native desktop session: tables, search, changed member certificate, host console, screenshot export | P01-14, P02-01, P02-02, P02-03, P03-05, P04-07, P04-11, P04-12, P18-06, P19-05 | Passed, finding 22 | Cold start from the saved profile and a session-password login reached Ready with the tree (pool, coordinator, guest, member), the VM table (name, state, host, vCPUs, memory limit), the details pane and per-action reasons ("The host does not currently allow this action" for Start, Resume and Unpause on the running guest). Searches by host, by name and by UUID prefix each matched the guest and a search for the empty member matched nothing; the Hosts, Storage (type, usage, capacity) and Networks (bridge) tables rendered. With the profile still carrying the member's pre-reset pin, the member's host text console was refused: "TLS verification failed for https://192.168.254.12/. Verify this host's certificate and add its endpoint-specific fingerprint", followed by bounded automatic reconnect attempts. The profile editor's certificate inspection of that endpoint showed "Certificate changed" with the previous and presented SHA-256, subject, issuer, names, validity, serial and the validation reason; typing the exact endpoint and ticking the trusted-channel confirmation enabled "Replace pin"; after reconnecting, the member's text console connected and its banner showed the same SHA-256. When the harness rebooted the member mid-session the console reported "Cannot reach the XCP-ng management endpoint" and automatic reconnect stopped after attempt 5 of 5. "Save screenshot…" on the guest console raised the native "Save console screenshot" dialog through xdg-desktop-portal-gtk on the host session; a file name typed there produced an 800x600 PNG of the console frame and the client reported the saved path. Evidence: desktop-run-2.json and screenshots in the desktop evidence directory outside the repository |
| Control-domain memory change with two member reboots | P12-08, P12-03, P12-05 | Passed on the fourth run, finding 23 | The member's control-domain memory was raised from 1440 MiB to 1568 MiB through the reviewed editor and read back from the limits; the member entered maintenance, rebooted (156 s to a new boot time) and came back live with the control domain's actual memory at the new allocation; the allocation was restored, maintenance was exited and re-entered because the reboot had re-enabled the host while the client's marker stayed (finding 12), the member rebooted again (156 s) with the original allocation read back after the boot, and maintenance was exited. The run began by clearing the marker that the third run had left behind. Earlier runs: the first was refused before the second reboot because the reboot had re-enabled scheduling, the second was refused re-entering maintenance because the marker was still set, and the third ended its final exit-maintenance OutcomeUnknown behind a journal conflict whose cause the worker discarded (finding 23). Evidence: 1794243324-dom0-memory-change-with-member-reboot.json |
| NIC lifecycle: internal network, hot unplug refusal, network and MAC change, rate limit | P07-14, P07-15, P07-16, P02-07 | Passed, observations 24 and 25 | With the guest halted, an internal network was created and a NIC on it added at device 1 with a host-assigned MAC, read back from the VIF graph with its VM and network. After the start the VIF reported currently_attached with only `attach` advertised, so the disconnect was refused by the client without a host call and with the missing advertisement named (observation 24; the first run had shown the generic unavailable-action text). Halted again, the NIC was moved to the management network with a replacement locally administered MAC through the client's destroy-and-recreate path and read back on device 1; the host recorded no supported VIF QoS algorithm, so the rate limit was skipped (observation 25); the NIC was removed, the network deleted and the guest restarted. Evidence: 1794243903-nic-plug-edit-and-rate-limit.json |
| Home-server placement with a diskless VM | P06-06, P03-04, P08-02, P08-08 | Passed | A diskless VM was created from the generic template on the pool, its home server set to the member and read back; a start without a chosen host placed it on the member. After a hard shutdown the home server was moved to the coordinator and the next start placed it there; the home server was cleared and the record deleted. Evidence: 1794243970-home-server-placement-with-a-diskless-vm.json |

## Findings

1. **`host.list_methods` is not implemented on XCP-ng 8.3.0.** The host answers
   NOT_IMPLEMENTED. The join and security-operation reviews treated the missing method
   catalogue as proof that the operation was unsupported and refused before dispatch.
   They now treat an unimplemented catalogue as "cannot pre-verify" and let the server's
   own explicit refusal of unknown methods remain authoritative; an explicitly empty
   catalogue still refuses. The security overview already reported the catalogue error
   rather than failing. Fixture coverage was added for both paths.
2. **A joined member is briefly not live.** After `pool.join` the member restarts its
   toolstack; the coordinator reports it disabled and not live for roughly half a minute.
   Post-join verification failed on the first read and left the operation unknown. It now
   keeps re-reading within its timeout; an operator recheck through the destination
   connection still resolves a late case. Fixture coverage was added.

3. **Halted guests do not advertise `changing_memory_limits` or `changing_VCPUs`.** XCP-ng
   8.3.0 lists `changing_static_range` and `changing_dynamic_range` for a halted guest and
   no vCPU operation at all, yet `VM.set_memory_limits`, `VM.set_VCPUs_max` and
   `VM.set_VCPUs_at_startup` succeed. VM creation gated its memory and vCPU steps on the
   absent names and stopped partially complete, leaving a device-less guest. The gate now
   accepts the range pair for the combined limits setter and lets a halted, idle guest's
   vCPU setters reach the server; a running guest still needs the advertised capability.
   Unit coverage was added.

4. **Same-pool migrations must not carry a VIF map.** `VM.assert_can_migrate` on XCP-ng
   8.3.0 answers OPERATION_NOT_ALLOWED with "VIF mapping is not allowed for intra-pool
   migration" for any VIF entry, including one that names the interface's current
   network. The client sent identity placements as a map and was refused. It now sends an
   empty VIF map for same-pool moves and refuses a real network change before dispatch;
   the disk map is still required and still sent. Unit coverage was added.

5. **JSON-RPC replies can contain bare non-finite numbers.** `VM.get_data_sources` and
   `host.get_data_sources` on XCP-ng 8.3.0 return `Infinity` and `-Infinity` as bare
   tokens, which strict JSON rejects, so every metrics subscription failed with "invalid
   JSON" before any RRD request. The client now replaces bare `NaN`, `Infinity` and
   `-Infinity` tokens outside string literals with `null` when a reply does not parse
   strictly; the metrics decoder already treated absent limits as unknown. Unit coverage
   was added.

6. **Children clipped out of and back into a scrolling container never return to the AT-SPI
   bus.** With the pinned Rune revision and AccessKit's Unix adapter (`accesskit_unix` 0.24.1
   and `accesskit_atspi_common` 0.21.1, the current releases), shrinking the window to 700 px
   made Rune resend 119 nodes and the adapter unregister the 28 details-column children that
   fell outside the column's clip (141 registered objects became 113). Growing the window
   back to 1758 px made Rune resend 26 nodes, none of them those children, because Rune keeps
   a scroll viewport's children in the viewport's content space and nothing about them
   changed. The adapter lists them as children again but re-evaluates clipping only for
   nodes whose own data changed, so an assistive client gets "Unknown object" for Open
   console, Migrate and the power controls until the column is rebuilt. Keyboard focus
   still reaches them, and a window built at its final size exposes every node. Both halves
   are upstream behaviour; the question is recorded in [sources](../SOURCES.md) and P18-06
   stays open.

7. **A coordinator power loss discards recently acknowledged metadata.** XAPI answers
   `VM.set_name_description` from its in-memory database and writes that database to disk
   periodically, so a description change the coordinator had acknowledged about seventy
   seconds before its power was cut was gone when the host returned, back at the previous
   value. The client reported Succeeded at the time, which was true, and after reconnecting
   it showed the host's current value rather than its own memory of the write. Nothing in
   the client can prevent this; operators should know that metadata written shortly before
   a coordinator crash may need to be repeated, and the client never repeats it on its own.

8. **A template turned into a VM keeps the template's allowed operations.** After
   `VM.clone` of a snapshot and `VM.set_is_a_template(false)`, XCP-ng 8.3.0 still lists
   `provision` and no `start` for the new VM until `VM.update_allowed_operations` is called;
   the client derives its Start control from that list and so never offered it. The snapshot
   clone and the lifecycle clone-to-VM workflows now ask the host to recompute the list
   right after the conversion, including on the recheck path that finds a conversion whose
   reply was lost. A host that refuses the recompute is named in the step detail rather than
   failing the clone. Unit coverage was added for both workflows.

9. **A cross-pool move's source cleanup lags the migration task.** `VM.migrate_send`
   reports success while the source VM, its attachments and above all its disk records are
   still being removed; the disks go only on the source storage manager's next scan. The
   client verified the source immediately and left every ordinary move as OutcomeUnknown
   ("source device cleanup is not yet verified"). It now waits for the source VM and
   attachments within the connection's cleanup bound (sixty seconds by default) and, once
   they are gone, accepts lingering disk records after a ten-second grace, naming their
   count in the outcome. Unit coverage was added.

10. **Records the destination releases after the review refused the next move.** The
   preflight compared the destination's full VM, disk and attachment identity sets with the
   reviewed baseline and refused on any difference; after a move, the previous move's
   released source disks on that pool changed the set and the move back was refused with
   "reopen this review". Only identities that appear after the review can be mistaken for a
   transfer's result, so the preflight now refuses on arrivals alone. Unit coverage was added.

11. **A pool that was never named could not be renamed or dissolved.** XCP-ng leaves a
   fresh pool's `name_label` empty, and the inventory shows "Unnamed" in its place. The
   desktop's rename editor and the dissolution review carried that placeholder as the
   expected name, so the host's fresh-record check refused the edit as "changed since
   review". The inventory now also carries the pool's own label, the reviews expect that,
   and the editor keeps showing the placeholder. Unit coverage was added for the empty
   label and for the placeholder being refused.

12. **A toolstack restart's new start marker arrives after the call returns.**
   `host.restart_agent` returns at once and the host publishes its new `agent_start_time`
   some twenty seconds later; a host reboot publishes its new `boot_time` minutes later on
   the nested lab. The client checked the marker immediately and left the operation
   OutcomeUnknown ("not confirmed by a newer start marker") although the restart had
   happened. It now waits for the newer marker within the connection's host restart bound
   (ten minutes by default), treating an unreachable host as still restarting, and the
   fixtures bound that wait so a marker that never changes still settles quickly. A
   toolstack restart also re-enables scheduling on a host that carries the client's
   maintenance marker, so the host is then enabled and marked at once; the client refused
   both entering maintenance again and exiting it from that state, leaving the marker
   stuck. Exiting maintenance now accepts an already enabled host that still carries the
   marker, repeats the harmless enable call and clears the marker. A host that has just
   rebooted also refuses `host.enable` with HOST_STILL_BOOTING for a while after its new
   boot time is visible; the enable is now repeated every few seconds within the same
   restart bound instead of failing the exit from maintenance. Unit coverage was added for
   both.
13. **A change the host does not advertise was reported as a stale editor.** The live
   shadow multiplier and live vCPU guards failed when `changing_shadow_memory_live` or
   `changing_VCPUs_live` was absent from the VM's allowed operations, and the refusal read
   "VM properties changed since this editor opened", which sends the operator to reload an
   editor that will refuse again. XCP-ng 8.3.0 never advertised the live shadow change for
   the lab guest and advertised the live vCPU change only a few seconds after boot. The
   guards now return a distinct refusal that names the missing advertisement; a changed
   limit or count still reports the stale-editor conflict. Unit coverage was added.
14. **The vCPU editor accepted a maximum the topology cannot boot.** `VM.set_VCPUs_max`
   accepts any count, and a VM whose maximum is not a multiple of its `cores-per-socket`
   platform value fails to start with VCPU_MAX_NOT_CORES_PER_SOCKET_MULTIPLE; an earlier
   run of the scenario left the lab guest in that state. The vCPU maximum step now checks
   the current topology, or the topology planned in the same edit, and refuses with a
   message that points at the topology editor. Unit coverage was added.
15. **Allowed operations lag the power state after a start.** For about five seconds after
   a VM reaches Running, XCP-ng 8.3.0 still publishes the halted set of allowed operations:
   `clean_shutdown`, `migrate_send` and `changing_VCPUs_live` are absent, and the client's
   whole-selection and per-operation preflights refuse the action without sending anything
   ("The requested action is unavailable for this VM"). That is the correct refusal for what
   the host advertises; the harness now waits for the advertisement as the editors do, and
   no client change was made.
16. **Disks never advertise `disable_cbt`.** XCP-ng 8.3.0 lists only `clone` and `snapshot`
   among a disk's allowed operations even with changed-block tracking enabled, and it has no
   `VDI.update_allowed_operations` to recompute them (MESSAGE_METHOD_UNKNOWN). The client
   required the advertisement before offering or sending the disable, so the editor could
   never be used on this host. It now decides from the tracking state, the blocked
   operations and an idle disk, and leaves the host to refuse a disable it does not accept.
   Unit coverage was added.
17. **A handover is verified before the previous coordinator steps down.** The
   `pool.designate_new_master` task completes on the previous coordinator, which keeps
   reporting itself as the coordinator until its toolstack restarts as a member some ten
   seconds later. The client verified the pool record at once, found the coordinator
   unchanged and left the operation OutcomeUnknown with "The resource changed since review",
   although the handover was under way. The verification now keeps reading the pool record
   within the connection's host restart bound while the previous coordinator is still
   reported; the session is normally lost to the redirect in between, which leaves the
   record unknown with that reason, and the operator's recheck on the new coordinator then
   confirms it. A fixture fault that delays the switch covers the wait.
18. **The host backup repeats EFI file names as hard links.** XCP-ng 8.3.0's `host_backup`
   archive lists three files on the vfat EFI partition twice: once as the file and once as a
   hard link entry with the same name pointing at itself. The archive inspection treated any
   repeated non-directory name as malformed, so the download ended OutcomeUnknown and the
   unresolved operation blocked every other pool action until acknowledged. A hard link whose
   target is a name already in the archive is now accepted (it adds nothing on restore); a
   repeated ordinary file is still refused. Unit coverage was added.
19. **`VMSS.snapshot_now` has no asynchronous form.** XCP-ng 8.3.0 answers
   `Async.VMSS.snapshot_now` with "No async mode for this operation" (a plain Fault the
   client reports as UNKNOWN_API_ERROR), so the schedule's run-now ended OutcomeUnknown and
   the unresolved operation blocked the schedule's other edits. The run is now the
   synchronous call, whose string reply carries the plugin result; completion is still
   verified by the advanced last-run time and a new snapshot on every member. The fixtures
   follow the synchronous call.
20. **A force shutdown is admitted during another client's migration.** XCP-ng 8.3.0 accepts
   `VM.hard_shutdown` while a `migrate_send` task runs, halts the guest on its source and
   cancels the migration itself. The migrating client's record is left unknown ("placement
   is incomplete or cannot be verified") and its recheck cannot resolve it because the guest
   is no longer running anywhere, so the operator acknowledges it. The client cancelled no
   other client's task. No client change was made.
21. **ISO library images are recorded as `user` disks.** XCP-ng 8.3.0 gives the disks of its
   ISO repositories, including the host's tools image, the VDI type `user`; only the
   repository's content type says `iso`. The creation wizard required the image's own type
   to be `iso` and refused every ISO on the lab, while the CD-drive and tools-media paths
   already judged by the repository. The wizard now accepts a read-only, present image whose
   type is `iso` or whose repository content type is `iso`. Unit coverage was added.
22. **Controls below a dialog's fold are outside the keyboard focus cycle.** The certificate
   review dialog places its exact-endpoint confirmation, trusted-channel checkbox and
   "Replace pin" below the visible area; until End scrolled the content, Tab cycled only
   through the visible endpoint field, the inspect button and Back, so a keyboard-only
   operator has no hint that the confirmation exists. The same clipped controls were
   unresolvable on the accessibility bus (finding 6). The inventory search field's Clear
   button also exposes no accessible action. Recorded for the desktop; no client change yet.
23. **A journal conflict while recording a worker error discarded that error.** The third
   dom0 run's final exit-maintenance operation ended OutcomeUnknown with only the generic
   "Local tracking stopped after a concurrent update" detail on its Enable scheduling step,
   although the host had enabled scheduling and the RPC trace recorded no failed call. The
   worker's conflict path reloaded the row and replaced the detail, so whatever stopped the
   worker was lost. The conflict detail now carries that error (unit covered). The conflict
   did not recur in the fourth run, which passed end to end; the concurrent writer is not
   identified and this remains a client observation rather than a host finding.
24. **VIF hot plug is not advertised on a running guest without PV drivers.** After the
   tool-less guest started, its new VIF reported currently_attached with `attach` as its only
   allowed operation, so the host offered neither plug nor unplug. The client refused the
   disconnect without a host call. Its refusal had used the generic unavailable-action text;
   a device operation the host does not advertise is now refused with that reason and the
   guest-tools prerequisite named (unit covered). Hot plug with a tools-equipped guest
   remains open.
25. **XCP-ng 8.3 records no supported VIF QoS algorithm.** The new VIF's
   qos_supported_algorithms was empty, so the client's rate-limit editor, which requires the
   host to advertise `ratelimit`, was not offered and the scenario skipped it. XenCenter sets
   the ratelimit policy without that advertisement; whether the client should do the same is
   an open product decision, not a host fault.

## Console observation

The host console applied no implicit shift to keysym events: `:` arrived as `;` and
uppercase as lowercase until the harness sent Shift around shifted characters, as a
physical keyboard and the desktop's key forwarding do. A blinking text cursor alternates
two frames, so frame-settling checks must tolerate that.

The desktop forwards QEMU extended keys as physical scancodes, so an injected Shift must be
a physical key event for the guest and must also carry the xkb modifier mask for the
desktop's own text fields; the compositor's key injection satisfied both, while a virtual
keyboard tool with its own keymap garbled every character. Rune buttons activate with Space,
and Enter on a focused link opened the workstation's browser outside the nested session.

## Limitations

The harness scenarios used the core facade; the desktop scenario used the Rune window under
a nested compositor, so real seat input, X11, clipboard, Orca speech, non-US layouts, pointer
input and the desktop's own console reconnect across a reboot remain unqualified. The M2 gate
names an XO-managed VM; this lab has no XO at all, so the run shows the client working with
no XO service rather than with a specific XO VM. Interruption paths for the passed workflows,
memory checkpoints, storage and network administration and the remaining ledger rows are
recorded as they are exercised. The lab now holds one running guest, its snapshots and one
device-less guest left by the partial creation; they stay for later deletion scenarios.
