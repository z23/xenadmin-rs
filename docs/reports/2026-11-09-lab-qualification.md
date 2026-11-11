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
- Lab appliances added on 2026-11-10 (see the [lab README](../../lab/README.md)): an
  isolated storage network with one live-attached virtio NIC per host (a second appears
  only after the host domain's next power cycle), an Alpine Linux NFS appliance booted from a remastered Alpine
  `virt` ISO that carries its packages and configuration offline and exports `/srv/nfs`
  with `sr`, `iso` and `heartbeat` directories on both lab networks, and a tools-equipped
  guest image (the same ISO with `xe-guest-utilities`) imported through the client. The
  workstation granted no privileges for this; everything runs through libvirt as the
  operator and nothing inside the lab reaches the internet.
- Native desktop run: the debug `xenadmin-rs` binary inside a nested Hyprland 0.56.2 session
  on the same workstation (1600x1800 headless output, 1558x1758 window), driven over AT-SPI
  through libatspi and the compositor's `sendkeystate` dispatcher, which delivers physical
  key events; screenshots through `grim`. During the desktop sessions of 2026-11-09 and
  2026-11-10 the lab contained no Xen Orchestra service; the appliance was imported on
  2026-11-11 for the recovery gate recorded below.
- Packaged desktop run of 2026-11-11: the 0.17.0 AppImage built from `90e6355`
  (SHA-256 `78bf1a572d8af56f75957234c02fe0443c52b9538bbcee61c3a5717e28afbf0c`), started on an
  empty private state directory in the same nested compositor. Keys came from the
  compositor's dispatcher as before; the pointer came from a small client of the
  compositor's virtual-pointer protocol, which gives the session a real pointer device for
  motion, buttons and the wheel. Guest-side evidence came from the guest itself: commands
  typed through the console made it capture its own input devices and report the bytes to a
  listener on the isolated bridge. A debug build of the fixed tree was used only for the
  recheck row.

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
| Snapshot restore interrupted mid-workflow | P05-03, P05-04, P19-04 | Passed, with an open observation | A reviewed restore (clean shutdown, revert, start) was cut off by closing the client while its shutdown step was being tracked; the restarted client recovered the workflow as OutcomeUnknown with the shutdown step unknown and the revert and start steps pending, and its read-only recheck repeated no write and submitted no remaining step; the unresolved workflow held the guest until the operator acknowledged it, after which the guest was started and the snapshot deleted. The recheck does not infer a finished clean shutdown from the guest being halted, so the step stays unknown; and in one of four attempts the restarted client's history did not show the record within five minutes and the guest had never been shut down. That attempt is explained by the scenario, not by the journal: its wait for the restore to be under way was already satisfied by a record still waiting for admission and by one refused before admission, and such a record exists only on its handle, so the client was closed with nothing admitted and nothing sent. The scenario now waits for a tracked host task and records the states it saw (2026-11-11); a restore refused before admission leaving no history entry is as designed |
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
| Endpoint failure with a remembered verified member | P01-04, P01-03 | Passed | A first connection through the coordinator reported its verified members; a second connection was given a dead address in the lab subnet as its endpoint together with the pool identity and that member list, as the desktop profile editor persists them. The dead endpoint failed after 3.1 s with "Cannot reach the XCP-ng management endpoint", the connection went Reconnecting and reached Ready through the remembered coordinator endpoint 4.2 s after the start with the expected pool. The same options without the pool identity stayed Connecting and Reconnecting on the dead endpoint for 45 s and never tried the member. Evidence: 1794244334-remembered-member-fallback-after-endpoint-failure.json |
| Default repository and repository metadata round trip | P09-10, P09-11, P09-08 | Passed | The pool's default repository was moved from the coordinator's local repository to the member's and read back from the pool details; a review that still believed the previous default was current and asked for a change was refused with "The resource changed since review. Refresh it before retrying" and nothing applied; the default was restored. The member's local repository was renamed with a new description and both were restored, each read back; a rescan of it completed with the host confirming the task. Evidence: 1794244607-default-storage-and-repository-metadata-round-trip.json |
| Halted guest move between the local repositories | P11-02, P11-03, P14-01 | Passed | With the guest halted and no snapshots present, a plan whose expected host equalled its destination was refused as invalid before any write; the guest and both of its disks were moved from the coordinator's local repository to the member's in 19 s and back in 21 s through the reviewed halted move, each verified by the disks' repository, the guest's cleared residency and its home host; the guest then started on the coordinator. A rerun sampled each move's reported progress, which went from 0 through 0.003 to 0.06 during the disk copies to 0.95 and 1.0 (P14-01). Evidence: 1794245098-halted-guest-move-between-local-repositories.json and 1794246069-halted-guest-move-between-local-repositories.json |
| Member evacuation review with a diskless VM | P12-04, P12-03 | Passed as a refusal, observation 26 | A diskless VM pinned to the member was started there. A maintenance review that believed the member empty was refused with "Resident VMs changed since review" and left the member enabled and the VM running on it. The review that named the VM as the member's resident guest was refused after the host's own blocker report, which the client now relays: "The host reports VMs that prevent evacuation (lab-evac-…: VM_LACKS_FEATURE)". The member stayed enabled and unchanged. The refusal arrived two minutes after submission in both runs; the client performs no wait in that review, so the time was spent in the host's blocker evaluation. The VM, which has nothing to boot, had powered itself off by then and its record was deleted. Evidence: 1794245573-member-evacuation-of-a-diskless-vm.json |
| Tags, custom fields and a vApp round trip | P15-01, P15-03, P15-04, P15-05 | Passed | On the lab guest a tag was assigned and removed; a custom field definition was added to the pool, its value set on the guest and cleared, and the definition removed; an empty vApp was created, the guest added with order 1 and 5 s start and shutdown delays, the vApp shut down cleanly and started again in order (6.3 s to the start task's completion), renamed, emptied and deleted. Every change was read back from the organisation inventory, the VM details and the appliance list. Evidence: 1794245809-tags-custom-fields-and-vapp-round-trip.json |
| Pool secret rotation and TLS verification state | P13-13 | Passed, enable refused as already on | The lab pool already had TLS verification enabled, so the reviewed enable was refused by the fresh pool read with "The resource changed since review" and nothing was sent. With HA disabled and both hosts live, the pool secret was rotated; the host confirmed the task in 1.3 s, the pool then reported no pending rotation, and the connection stayed current with its inventory revision advancing afterwards. Evidence: 1794246012-pool-secret-rotation-and-tls-verification-state.json |
| Folders, message dismissal and a two-VM description batch | P15-02, P14-10, P15-06 | Passed | The lab guest was placed in a folder, moved to another and taken out, and an empty folder was registered in the pool and removed, each read back from the organisation inventory. Of the pool's 163 server messages (thirteen distinct kinds), a dismissal with a stale review digest was refused with the message left in place, and the oldest message (a VM_STARTED record) was dismissed and left the inventory. A description batch over the running guest and a halted diskless VM verified two of two setters, was read back on both and restored by a second batch; the diskless record was deleted. Evidence: 1794246236-folders-message-dismissal-and-bulk-description-batch.json |
| Storage migration refused while a snapshot exists | P11-03, P05-01 | Passed as a refusal | With the guest running, a disk snapshot was taken and listed; a live migration with storage relocation to the member was then refused by the client with "This migration topology or resource configuration is not supported", leaving the guest running on its host with both disks in place and the pool's task count unchanged; the snapshot was deleted through the reviewed deletion. Evidence: 1794246386-storage-migration-refused-while-a-snapshot-exists.json |
| Snapshot XVA export | P11-07, P05-01 | Passed | A disk snapshot of the running guest was taken and exported as an uncompressed XVA into the lab's private transfer directory: the complete stream receipt and the server result were verified and the 71 MiB archive saved in 4.8 s; the archive was removed and the snapshot deleted through the reviewed deletion. Evidence: 1794246504-snapshot-xva-export.json |
| HA with the shared heartbeat repository and a protected guest | P13-05, P06-07 | Passed | With the shared NFS repository on the appliance as heartbeat store, HA was enabled (the host confirmed the task in 29 s and the pool listed the repository among its heartbeat stores), the tools guest was given the restart priority and read back, the tolerated host failures were raised to one with the pool reporting a plan for one failure and restored, the priority was removed and HA was disabled in 9 s. Evidence: 1794266333-ha-enable-with-shared-heartbeat-and-protected-guest.json |
| Member evacuation of the tools guest on shared storage | P12-04, P12-03, P11-01 | Passed | The tools guest was live-migrated to the member without moving storage, the member entered maintenance with the guest reviewed as its resident so the client evacuated it, the guest was running on the coordinator 4.3 s after submission with the member disabled, and the member left maintenance. Evidence: 1794266343-member-evacuation-of-the-tools-guest.json |
| Member power cycle: reviewed shutdown, libvirt start, return with the second storage NIC | P12-05, P12-03, P19-05, P10-04 | Passed with finding 27 | The empty member entered maintenance and the reviewed shutdown was accepted; the client's verification ended OutcomeUnknown after 11 s because the coordinator still reported the host live, although the domain powered off moments later (finding 27, fixed). The lab tooling started the domain again; the member reported live 127 s later with scheduling disabled and the client's maintenance marker still set, the exit from maintenance cleared both, and the rescan introduced the NIC that libvirt had held for the next start as `eth2`. On this run the exit from maintenance ended in the journal conflict of finding 23 and blocked the remaining steps; a second pass through the return scenario completed them. Evidence: 1794264878-member-shutdown-through-maintenance.json, 1794265327-member-return-and-second-storage-nic.json and 1794265729-member-return-and-second-storage-nic.json |
| Storage interface repair: forget a stale record and rescan | P10-04, P10-09, P09-02, P09-06 | Passed, observation 28 | The member's storage interface record still named the hot-plugged device `side-7679-eth1` and reported attached with its address, yet the NFS export probe through the member failed with SR_BACKEND_FAILURE_108. The new reviewed forget removed the record, the rescan re-introduced the same MAC as `eth1` (alongside `eth2`), the static address was configured, the attach was refused because the rescan had already attached the interface, the probe then listed the appliance's exports, and the shared repository attached on the member. Evidence: 1794265792-member-storage-interface-repair.json |
| Storage network and shared NFS repository on both hosts | P10-04, P10-09, P09-02, P09-03, P09-06, P09-08 | Passed, observation 30 | Each pool host's storage NIC was rescanned into the inventory, given a static address on the isolated storage network and attached; the appliance's four exports were probed through the coordinator; the shared NFS repository created in the first attempt was reused and verified attached on both hosts; a detach on the member was refused by the client because a running guest uses the repository elsewhere in the pool (observation 30); the repository was rescanned. Evidence: 1794268045-storage-interfaces-and-shared-nfs-repository.json |
| Tools guest on shared storage: import, agent, hot plug, live migration | P11-09, P08-02, P06-11, P07-14, P11-01, P14-01 | Passed | The remastered Alpine image with xe-guest-utilities was imported as a raw disk onto the shared repository, a VM was created there and booted from it, and the guest agent reported its tools version to the pool (no address yet: the guest booted before the lab network served leases). With PV drivers present the host advertised NIC unplug, the client disconnected and reconnected the running guest's NIC, and the guest live-migrated to the member and back without storage transfer in 4.3 s each way with the task's progress sampled. Evidence: 1794268116-tools-guest-on-shared-storage.json |
| Member storage bond round trip | P10-05, P10-09, P09-06 | Passed | On the member both storage NICs were freed (the repository detach was refused by the in-use guard and left attached), a bond network was created and an active-backup bond of the two NICs formed on it (`bond0`), switched to balance-slb and back, given the static storage address and left attached with the repository still reachable over it; then the bond's address was cleared, the bond detached and destroyed, the bond network deleted, and the first NIC's address restored and re-attached. Evidence: 1794267920-member-storage-bond-round-trip.json |
| Tools guest clean reboot and reported address | P03-01, P06-11 | Passed | With the guest agent present the client advertised the clean reboot, which completed in 4.3 s with a new start time; after the restart the agent reported its tools version again and, now that the lab network serves leases, the guest's IPv4 and IPv6 addresses. Evidence: 1794268322-tools-guest-clean-reboot-and-address.json |
| iSCSI discovery and LVM-over-iSCSI repository | P09-02, P09-03, P09-06, P09-08 | Passed | Through the coordinator the appliance's two targets were discovered on both of its portals (four target records), the open target's single 20 GiB LUN was enumerated with its SCSI identity and vendor, and an unauthenticated LUN probe of the CHAP-protected target was refused by the host (SR_BACKEND_FAILURE_68); a shared LVM-over-iSCSI repository was created on the open LUN, verified attached on both hosts, detached and reattached on the member through its attachment, and rescanned; it stays for the multipath scenario. Evidence: 1794277021-iscsi-targets-and-lvm-repository.json |
| CHAP-protected iSCSI repository with transient credentials | P09-02, P09-03, P09-06, P09-09 | Passed on the second attempt, finding 31 | The CHAP target's LUN was discovered through the coordinator with credentials supplied only for that request; a shared LVM-over-iSCSI repository was created on it through the authenticated flow (temporary Secret created, storage provisioned, Secret removed, 13 s), verified attached on both hosts with attachments that require credentials, detached on the member and then on the coordinator once the host advertised unplug, and destroyed. The first attempt's coordinator detach, submitted right after creation, was refused by the client's repository guard with a VM-worded message (finding 31); the repository it left behind was removed by the rerun's leftover cleanup. Evidence: 1794277742-iscsi-chap-repository.json (first attempt 1794277360-iscsi-chap-repository.json) |
| Member multipath round trip | P09-12, P12-03, P09-06 | Passed on the fourth attempt, findings 32, 33, 34 | With the member empty and in maintenance, standard multipathing was enabled through the reviewed complete-host flow: five attachments detached, the host policy and dmp handler set and the same five replugged (11.6 s) while the local-cache attachment stayed attached; the member read back multipathing with the dmp handler and its LVM-over-iSCSI attachment reported two active paths of two across the appliance's two portals; multipathing was then disabled the same way (11.0 s, paths no longer reported) and the member left maintenance. The first three attempts were refused by the HA guard, refused by the local-cache guard and left outcome-unknown by the configuration check (findings 32, 33, 34); the final run first reattached the two attachments and reverted the enable that the third attempt had left behind. Evidence: 1794280183-member-multipath-round-trip.json (earlier attempts 1794278122, 1794278659 and 1794279839) |
| ISO library over NFS | P09-04, P09-06, P09-08, P09-09 | Passed | An ISO library on the appliance's `/srv/nfs/iso` export was created through the coordinator, read back as a shared `iso` repository attached on every host, rescanned, and listed the image the appliance seeds (`guest-tools.iso`); it was then detached on the member and the coordinator once unplug was advertised and forgotten with its one image reviewed. The first run, before the appliance was rebuilt with a seeded export, listed no images. Evidence: 1794316192-iso-library-over-nfs.json (empty export: 1794280427-iso-library-over-nfs.json) |
| ISO library over SMB 3 | P09-04, P09-06, P09-08, P09-09 | Passed | The same round trip over the appliance's guest-readable SMB 3 share `//appliance/iso`, created through the explicit SMB 3.0 path without credentials (54 s), attached on both hosts, rescanned with the seeded image listed, detached everywhere and forgotten. Evidence: 1794316268-iso-library-over-smb.json |
| Coordinator loss with HA: member takeover | P01-04, P13-05, P13-06, P19-05 | Failed on the client side, findings 35 and 36 | With HA on the shared NFS repository, the tools guest protected on the coordinator and one failure tolerated, the coordinator's domain was destroyed. The connection went stale in 31 s, but its remembered-member list held only its own endpoint, so it never followed the member; XCP-ng elected the member within two minutes and HA restarted the protected guest there within three (the pool's own messages). The old coordinator, started again with HA armed, never rejoined: its storage interface was not brought up before XAPI, so the heartbeat repository was unreachable and XAPI never started, through two power cycles. Evidence: the pool messages and hypervisor counters recorded in findings 35 and 36; the run produced no record because its restore phase unwrapped a non-current connection, which the scenario now records instead |
| Pool recovery from a dead coordinator through the survivor | P12-11, P13-05, P09-09 | Passed with lab repair, observations 37 and 38 | Through the surviving coordinator the client disabled HA, deleted the record of the guest whose disks were on the dead host's local storage (no disks, no snapshots left), and forgot the dead host after its domain was verified off at the hypervisor. The client's forget of the dead host's three repositories stayed refused because XAPI never advertised forget for them; `SR.forget` was issued directly as a lab repair after moving the pool default repository to the shared NFS one. Evidence: 1794314951-forget-dead-coordinator-through-survivor.json |
| Reinstalled host rejoins by reviewed join | P13-02, P19-05 | Passed | The forgotten host was reinstalled unattended by the lab tooling (answer file fetched once, fresh host identity, XCP-ng 8.3.0 standalone) and joined into the survivor's pool through the client's reviewed join from its own connection; the member appeared on the destination and the source connection reconnected as it restarted its toolstack. Its storage interface was then configured and the shared NFS repository attached through the storage scenario, and the coordinator role was handed back to it through the reviewed handover, whose reply was lost in the toolstack restart and rechecked from the durable record. Evidence: 1794315795-form-pool-reviewed-join.json, 1794316007-designate-first-host-coordinator.json |
| Recovery gate: the XO VM located, started and consoled with XO unreachable | M2 gate, P11-06, P03-01, P04-01, P06-03 | Passed on the fourth attempt, findings 39 and 40 | The Xen Orchestra appliance image (1.9 GB, downloaded from the vendor) was imported through the client onto the shared NFS repository (173 s), sized to 2 GiB for the lab host while halted, and started; its agent reported an address and XO answered over HTTP from the workstation 25 s later. A clean shutdown through the agent made XO unreachable; with XO down the VM was located by name and description through the direct host connection, started, its console opened with a frame saved, and XO answered again 28 s after the start. The first three attempts failed in the client's XVA verifier and upload reply wait (findings 39 and 40). The appliance stays in the lab; the lab tooling then connected it to the pool over its own API, so it manages the pool and every lab guest, itself included, and the gate was run again in that state: XO reachable, shut down and unreachable, located, started and consoled, XO back 28 s later (44 s in all). Evidence: 1794320144-xo-vm-recovery-gate.json (first pass 1794317428), frames 1794320139- and 1794317423-console-xo-vm-console.ppm |
| Coordinator loss with HA: member takeover, rerun | P01-04, P13-05, P13-06, P19-05, P10-09, P13-04 | Passed on the fourth run (two harness refinements) | With both hosts' heartbeat storage interfaces protected from unplug first, HA enabled on the shared NFS repository (33 s) with the tools guest protected and resident on the coordinator, the coordinator's domain was destroyed. The connection went stale in 31 s, followed the member as coordinator through its remembered member endpoint 158 s after the cut, and found the protected guest already restarted there by HA. The old coordinator, started again, rejoined as a member in 176 s with no further reset. HA was disabled, the coordinator role handed back (the reply lost in the toolstack restart; once both hosts were enabled again the recheck confirmed the durable record without replaying anything), the unplug protection removed, the tools guest moved back and the local-storage guest the power cut had halted started again. The two earlier runs with the client fixes ended the same way on the host side and differed only in the harness, which now rechecks the handover and waits for migrate to be advertised again. Evidence: 1794320065-coordinator-loss-with-ha-member-takeover.json (earlier runs 1794318589 and 1794319366) |
| Directory service: join, subjects, roles, directory login and leave | P13-09, P13-10, P13-11, P10-08 | Passed on the third attempt (harness fixes) | With the Samba domain controller appliance up, both hosts' management interfaces were pointed at it for DNS through the management-network reconfiguration (addresses retained), the pool joined the lab domain with transient administrator credentials (6 s, both hosts reporting AD and the realm), the test user and group were resolved by their domain names and granted access (XCP-ng 8.3 created both subjects with no roles), the user was given the read-only role, and a connection as that user from a second client instance became Ready with the read-only role's 1348 permissions while its description edit was refused by the client's own permission guard before any request reached the host, worded as a host refusal (observation 41), and the guest stayed unchanged. The subjects were removed, the pool left the domain removing its machine accounts (1.3 s), and the DNS was restored. The first attempt reused the root connection's backend for the directory login, which the one-owner-per-pool rule refused; the second checked the refusal at submission rather than at the operation's outcome. Evidence: 1794318832-directory-join-subjects-and-roles.json |
| Recovery gate through the packaged desktop client | M2 gate, P19-02, P01-01, P01-02, P01-09, P01-13, P02-03, P03-01, P04-01, P04-02 | Passed, findings 42 and 43 | Lab tooling cleanly shut the Xen Orchestra VM down, and XO refused connections from the workstation. The 0.17.0 AppImage (built from `90e6355`) was cold-started on an empty state directory inside the nested compositor. A member's endpoint was typed by hand; the certificate inspection of the member and then of the coordinator showed no saved pin and the same SHA-256 values an independent TLS read over the isolated bridge had given, and typing each exact endpoint and ticking the trusted-channel check enabled Trust certificate. A session-only password reached Ready through the member's redirect, with the pool, both hosts and the three guests. A search for the first eight characters of the XO VM's UUID left that one halted VM; a pointer click selected it, a pointer click on Start submitted one Start operation (the journal holds exactly one, Succeeded with "Host task completed"), the VM ran on the member, and XO answered over HTTPS again within 40 s. Its console opened as "Connected directly to the host console" on the Debian login prompt, a typed user name appeared there and an empty password ended in "Login incorrect". A watch of the client's sockets, sampled four times a second for the 21 minutes the client ran, saw connections to the two pool hosts on port 443 and nothing else; checks as short as the certificate inspections fall between samples. Evidence: desktop-run-3.json, its screenshots and connections.json in the desktop evidence directory outside the repository |
| Pointer input verified from inside a guest | P04-02, P04-04, P04-09 | Passed for position and all three buttons; wheel direction unverified | The plain Alpine guest's kernel binds QEMU's USB tablet with its boot-protocol mouse driver, so the guest sees the tablet's absolute report as relative bytes: X in full and the low byte of Y. While the guest captured its input device and reported the bytes to a listener on the isolated bridge, a virtual pointer moved to three console positions and clicked left, middle and right. With the 800 by 600 frame fitted at a scale of 1.1975 the guest read X as pixels 199, 400 and 600 for targets at 199.6, 400.0 and 600.4, the low byte of Y matched rows 150, 300 and 449, and each button arrived as a press and a release (the right button as one complete click on release, as the adapter decision describes). The same three targets were exact after the window was resized to a scale of 0.75, and again in the same AppImage running as an X11 client. Two more reports arrived when the wheel turned, but that driver does not expose the tablet's wheel byte. Evidence: pointer-fit.json, pointer-resized.json and pointer-x11.json |
| Held key released when the window loses focus | P04-02 | Passed, finding 44 | With the guest capturing its keyboard device, a key was held down in the console and another window then took the compositor's focus while the key stayed down at the injector for 7.8 s. The guest saw one press, 36 repeats and the release 2.06 s after the press, the moment focus moved, and no repeat after it. One more press and release of the same key followed 9 ms later (finding 44). Evidence: held-key-focus-loss.json |
| Console follows a guest migrated by another client | P04-07, P19-06 | Passed | With the tools guest's console open in the desktop, lab tooling acting as another client live-migrated the guest to the member (3.4 s) and back (3.2 s). Each time the console reported "Console disconnected" about 2 s after the migration began and "Connected directly to the host console" again by 5 s and 4 s, the details pane showed the new host, the socket watch showed the tunnel on the member's address, and text typed afterwards appeared on the relocated console. The reviewed migration started from this client, with its one-shot console follow, was not run here. Evidence: migrate-1.json, migrate-2.json |
| Packaged client as an X11 application | P18-04, P04-02 | Passed under Xwayland | The same AppImage started with no Wayland display on the nested compositor's Xwayland server, with automatic adapter selection and no surface error (the first alpha report had recorded one). It loaded the saved profile and both pins, logged in with a session password, opened the guest console, ran a command whose text needed shifted characters and that the guest reported over the network, and passed the same pointer check. This is Xwayland under a nested compositor, not a standalone X server with a window manager |
| Saved credentials with a real Secret Service provider | P01-10, V18 | Passed for save, automatic restore and a locked store; forget not exercised | A throwaway GNOME keyring ran on a private session bus. With "Remember password in desktop keyring" and "Connect automatically when XenAdmin RS starts" ticked, the packaged client connected and the keyring then held one item labelled with the application, user and endpoint and carrying the application, endpoint, profile and username attributes. A restarted client was connected again with nothing typed. With the collection locked, a restarted client stayed disconnected and said "The desktop password store is locked. Unlock it in your password manager, then retry, or enter a password for this session"; a typed session password then connected with that notice still shown. This instance was driven by pointer coordinates and screenshots because the accessibility bus was not reachable from the private session. Removing the saved password through the client was not run |
| Fixes rechecked on the live desktop | P02-03, P04-09, P04-02 | Passed | A debug build of the tree with the three fixes, in the same nested session: a search that left one object returned to three after a pointer click on Clear, which now also exposes a click action; Actual size showed the console at 800 by 600 from the view's origin, typing reached the guest and the three pointer targets read back exactly at a scale of one; and the held-key capture showed one press, 35 repeats and one release with nothing after it. Evidence: pointer-actual-size-fixed.json and the run record's recheck section |
| Operator session on a real seat | P18-04, P18-05, P02-01, P02-03, P02-04, P04-02, P14-07, P19-08 | Passed, findings 46 to 50 | The operator ran the 0.18.0 release build as a native Wayland client in their own desktop session, first against a pool of theirs outside the lab, also XCP-ng 8.3.0, to look at its inventory, and then against the lab. They reported the tree, tables and search responsive on that pool's real inventory, the search and its Clear button working, and window resizing and the narrow layout working. On the lab they opened a guest's console and typed into it from their own keyboard; the lab has no graphical guest, so they could not try the pointer. They also reported that buttons were hard to tell from labels and showed no order of importance, that text was too small with no way to enlarge it, that a changed column width was not kept, and that the performance view showed an error once it refreshed (findings 46 to 50). That client's journal holds no operation from the session. Nothing from that pool (names, addresses or inventory) is recorded in this repository, and its size was not measured |
| Member reboot with the exit from maintenance repeated, rerun | P12-03, P12-05, P12-06 | Passed after the finding 23 fix | With the Xen Orchestra appliance moved to the coordinator first so the member was empty, the member entered maintenance, had its toolstack restarted, was rebooted through the reviewed operation and left maintenance, each step succeeding. On the exit the host refused the enable three times while it was still booting; the client repeated it, kept the three refused tasks in the record and verified the host enabled with its marker cleared. The appliance was moved back afterwards. Evidence: 1794360620-member-toolstack-restart-and-reboot.json (an earlier pass the same day, 1794360314, did not record the step details) |
| Clean shutdown and start interrupted by a client restart | P03-01, P14-02, P19-03, V12 | Passed | A clean shutdown and then a start of the plain guest were each submitted and the client closed the moment the request was on its way; a new client on the same state directory recovered each as OutcomeUnknown and sent nothing again. In the first run the shutdown had its task reference saved ("The connection was closed", then the recheck followed the task to Succeeded), while the start was cut between its dispatch and the saved reference: it recovered as "Interrupted operation without a saved task reference. Inspect the host before acknowledging its unknown outcome", the recheck left it unknown, and the operator's acknowledgement released the guest, which was in fact running. In the second run both were cut after the reference was saved and both rechecks reached Succeeded. Each time the guest reached the state asked for and the history held one record per request. Evidence: 1794360938- and 1794360982-interrupted-shutdown-and-start.json |
| XO unreachable through a network fault, its VM still running | M2 gate, P19-02, P07-03, P04-01 | Passed | With XO answering from the workstation, the appliance's only NIC was disconnected while it ran, through the client's own NIC operation on the direct connection. XO stopped answering at its address (the connection timed out) while the VM stayed Running. The same connection stayed Ready and showed the XO VM by name and description as Running with its NIC detached and no reported address, and its console opened and showed the appliance's login prompt. Reconnecting the NIC through the client succeeded, and XO answered at the same address 1.3 s after the request; the reboot fallback was not needed. The fault was a detached virtual NIC, injected by the client under test: a fault in the network between the workstation and XO, with the NIC attached, was not produced. Evidence: 1794362710-xo-network-fault.json |
| Session closed on the host with no outage | P01-03, P13-10, V05 | Passed after the finding 51 fix | Inside the directory scenario, with the read-only directory user connected from a second client instance. First, lab tooling acting as another administrator closed the user's sessions on the host (`session.logout_subject_identifier`) while the user kept its access: the user's client reported the host's SESSION_INVALID, signed in again by itself and was Ready under the next generation 1.5 s later with the read-only role, and it then showed a description edit made by the administrator and its restoration, so events arrived on the new session. Second, the user's subject was removed through the client, the user's own subject last so that its group no longer gave it access. In the first run that removal left the user's client Ready (finding 51) and only the tooling's logout stopped it. In the rerun with the fix the removal itself closed the sessions: the user's client reported SESSION_INVALID, tried to sign in again and stopped at Failed with the host's SESSION_AUTHORIZATION_FAILED 10.5 s later, and stayed there for the 30 s it was watched. Both times the subjects were removed, the pool left the domain and the DNS was restored. The session was closed by an administrator's call; a session that times out by itself was not waited for. Evidence: 1794363295- and 1794363655-directory-join-subjects-and-roles.json |
| Console connection lost while a modifier key is held | P04-02, P04-07, V19 | Passed in five runs; one earlier run with a cruder check is unexplained | At the tools guest's login prompt the harness read the character the screen drew for a plain `d` and for the same key with Shift, pressed Shift without releasing it and live-migrated the guest through the client. The console connection failed under the held key when the migration completed (4.3 s), and on a new console to the migrated guest a plain `d` read as lower case. With Shift held again, lab tooling acting as another administrator restarted the toolstack of the guest's host; the console failed 1.3 s later, the toolstack was back after 14 s, and a plain `d` on a new console again read as lower case. The guest was left holding the key in neither case, in every one of the five runs, and was migrated back each time. Before that check existed, three runs used a cruder sign of a held Ctrl, a restart of the login program on a plain `d`: two showed none, and one showed a restart after the migration, which did not recur once Ctrl had been released by hand. That sign cannot tell a held Ctrl from a login program that restarted for another reason, and the result was not reproduced; its cause is undetermined. The client sends nothing on a new console to release keys that a lost one left held, as the reference client does not, so a guest that did keep one would keep it until that key is pressed again. Only a Linux guest with guest tools was tried. Evidence: 1794364927-, 1794365011-, 1794365084-, 1794365155- and 1794365234-console-lost-while-a-modifier-is-held.json; the unexplained run is 1794364650- |
| Graphical guest and pointer presses | P11-09, P08-02, P03-01, P04-02, P04-04 | Passed through the core facade; pointer movement and clicks then confirmed by the operator on a real seat | The lab had no guest with a graphical desktop. GParted Live 1.8.1-6 (an isohybrid live image; its SHA-256 matched the publisher's checksum file fetched over HTTPS, the signature was not checked) was imported through the client as a raw disk (3.8 s), a guest `lab-desktop-1794365323` with 1 GiB was created from the generic template with that disk alone and started, and Return sent at intervals accepted the image's keymap, language and mode prompts: the console showed the GParted window on a desktop at 800 by 600. The operator opened the same console from the desktop client during that boot, so some pointer input in that run was theirs. Two presses were then sent through the client's console, each aimed by a person at a control in a saved frame: on the Close button of an information dialog at (592, 455), after which the frame showed the guest's own pointer arrow at that point and the dialog gone; and on the View menu title at (132, 32), after which the arrow was on the title and the menu open beneath it. The guest draws its pointer into the frame, so the frames show where the guest believes the pointer is. This shows the console's pointer coordinates and button reaching a desktop guest; it does not show the desktop client's own mapping from a window to those coordinates for this guest, which the earlier pointer row covers for another guest. The operator then used this guest's console from the 0.18.0 desktop client in their own Wayland session and reported that the pointer moved and that clicks worked; they did not report on the wheel, on dragging or on how exactly the pointer sat under their hand. The guest is left running. Evidence: 1794365524-desktop-guest-provisioning.json, 1794365577- and 1794365608-desktop-guest-pointer-press.json with their frames |
| Operator follow-up on a real seat | P14-07, P04-06, P01-10, P18-06 | Charts confirmed; findings 53 and 54 | From the 0.18.0 desktop client in their own Wayland session, after the earlier fixes, the operator reported that the performance charts draw and update without error, which closes the recheck that findings 49 and 50 had left open; that clipboard exchange with a guest does not appear to work (finding 54); and, after restarting their session's keyring daemon to repair an earlier side effect of this qualification, that a saved password could not be read because the store was locked (finding 53, fixed). They decided that screen readers are not supported in this release and agreed the supported and experimental boundary now recorded in the feature scope. The guests, pools and text involved were not recorded |
| Clipboard text typed as key presses | P04-06, P04-02 | Passed | In the plain guest's shell, one request through the new console input typed three lines: a `read` that takes the next line as it comes, a line of all 95 printable ASCII characters, and a command that reports that line's bytes in hexadecimal to the workstation. The 198 characters, three of them line breaks sent as Enter, were typed in 6.0 s, and the guest's report was exactly the bytes typed. The guest uses a US layout; another layout was not tried. This went through the core facade: the desktop's button, its review and its stopping are covered by unit and fixture tests and have not been used by a person. Evidence: 1794369693-typed-text-reaches-the-guest.json |
| Pool HTTPS-only setting round trip | P13-13 | Passed after the finding 55 fix | The client showed the pool's hosts as accepting plain HTTP, which a connection from the workstation to the coordinator's port 80 confirmed (200). The reviewed pool operation turned HTTPS-only access on: the step was verified on the hosts, the client showed the new value, and the same plain connection was refused. A second operation turned it off again, verified, and the plain connection answered. Before the fix this control was never offered on a real pool. Evidence: 1794385350-pool-https-only-round-trip.json |
| Supported core rerun after its decoder moved onto generated bindings | P01-01, P03-01, P03-04, P04-01, P05-02, P05-03, P05-04 | Passed | The event decoder's VM, host and task reads and the power and snapshot checks were moved from reads by key onto record views, first hand-written and then, by the operator's decision, generated from the upstream API description in the separately licensed `xenadmin-xapi` crate, with the VM power states compared through its generated constants ([decision](../decisions/0004-generated-bindings.md)). The inventory decoder followed, and then the network fabric decoder, which every inventory load runs, together with a change of the shared field readers to take a field type that a whole record cannot be passed as. After each of the four moves five scenarios were run again: read-only recovery through each host, the power matrix, a chosen-host start with the action diagnosis, console input with a snapshot and its reviewed restore, and the deletion of the snapshots. All passed each time and the lab was left at its baseline. The chosen-host scenario failed once in the first round, when it began seconds after the power matrix had started the guest: the client refused a clean shutdown that the host did not advertise yet, as it should, and the scenario now waits for the advertisement. The management topology decoder moved in the same change; it is read only when a management interface on a bond or VLAN is reconfigured, which none of the five does, so since its move it has fixture tests alone. Evidence, for the last round: 1794386886-direct-recovery-read-only.json, 1794386901-guest-power-matrix.json, 1794386916-chosen-host-start-and-action-diagnosis.json, 1794387156-guest-console-and-snapshot-restore.json and 1794387157-cleanup-snapshots-and-partial-guest.json |

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
   identified and this remains a client observation rather than a host finding. It recurred
   once more on 2026-11-10, again on the exit from maintenance right after a member came back
   (this time from a power cycle), and left the pool reservation unresolved so the run's
   later operations were refused as "another operation is running or unresolved". With
   `XENADMIN_DEV_RPC_TRACE=1` the journal now names the check that refused the save; the
   next recurrence will show it.

   Resolved on 2026-11-11. There was no second writer. A host that has just booted fails
   `host.enable` with HOST_STILL_BOOTING, and the worker repeats the call (finding 12); each
   repeat returns a new task, and the journal, which never lets a stored task reference
   change, refused the worker's own record as a concurrent update. The repeated task then
   succeeded with nobody tracking it, which is why the host ended enabled and no call had
   failed. The fixture had answered every call with the same task reference, so its test of
   the repeat passed; answering as a host does, it reproduced the reported outcome exactly.
   A step now keeps the tasks the host definitely refused, oldest first, and a new task may
   take the place of the current one only by moving it to that list; any other change of a
   task reference is still a conflict, now traced as its own check (unit and fixture
   covered). The verified step says how often the call was repeated. Rerun live: the
   member's exit from maintenance after its reboot recorded "the host refused the call 3
   times while it was still booting and it was repeated" and succeeded. Evidence:
   1794360620-member-toolstack-restart-and-reboot.json.
24. **VIF hot plug is not advertised on a running guest without PV drivers.** After the
   tool-less guest started, its new VIF reported currently_attached with `attach` as its only
   allowed operation, so the host offered neither plug nor unplug. The client refused the
   disconnect without a host call. Its refusal had used the generic unavailable-action text;
   a device operation the host does not advertise is now refused with that reason and the
   guest-tools prerequisite named (unit covered). Hot plug with a tools-equipped guest was
   recorded later in the tools guest on shared storage scenario.
25. **XCP-ng 8.3 records no supported VIF QoS algorithm.** The new VIF's
   qos_supported_algorithms was empty, so the client's rate-limit editor, which requires the
   host to advertise `ratelimit`, was not offered and the scenario skipped it. XenCenter sets
   the ratelimit policy without that advertisement. The operator decided on 2026-11-11 to
   leave this as it is while VM devices are experimental; it is not a host fault.
26. **A guest without PV drivers cannot be evacuated.** XCP-ng 8.3 reported the running
   diskless tool-less VM as preventing evacuation with VM_LACKS_FEATURE, so the member could
   not be evacuated through the pool. The client refused before any change; it had reported
   only that blockers existed, and now names the blocking VMs and the host's code in the
   refusal (unit covered). An evacuation that actually moves a guest needs shared storage or
   a tools-equipped guest, neither of which the lab had then; the later tools-guest
   evacuation on the shared NFS repository is recorded above.
27. **Host shutdown verification gave up before the coordinator noticed the member was
   down.** The reviewed shutdown of the empty member ended OutcomeUnknown 11 s after the
   host accepted the task, with "The host's fresh liveness does not match the reviewed
   operation", although the domain powered off a moment later. The coordinator marks a
   member offline only when its heartbeat lapses, so the verification now waits within the
   connection's host-restart bound for liveness to drop, as the reboot verification already
   waits for the new boot marker.
28. **A hot-plugged NIC's interface record keeps its temporary device name.** XCP-ng named
   the NIC added to a running host `side-7679-eth1`; the first attach of its interface
   failed with PIF_CONFIGURATION_ERROR, a reviewed reboot made the attach succeed and the
   toolstack then reported the interface attached with its static address, yet the
   hypervisor counted no transmitted frames and the host could not reach the appliance
   (SR_BACKEND_FAILURE_108). After a power cycle the second NIC was introduced as `eth2`,
   which shows the kernel had renamed the first to `eth1` while its PIF record still said
   `side-7679-eth1`, so the toolstack configured a device that no longer existed. The
   client gained a reviewed "forget physical interface record" operation (non-management
   physical interfaces without VLANs, bonds or tunnels; verified by the record's absence)
   so that a rescan re-introduces the device under its current name.
29. **Configuring an address does not attach a secondary interface.** PIF.reconfigure_ip
   left the storage interface unattached on both hosts and the appliance unreachable until
   the separate attach action ran. XenCenter plugs the interface after configuring it. The
   operator decided on 2026-11-11 to keep the explicit attach while network administration
   is experimental. The scenarios attach explicitly.
30. **The repository detach guard is pool-wide.** The client refuses to detach a shared
   repository on one host while any running guest in the pool uses a disk on it, even when
   that guest runs elsewhere; XAPI itself permits unplugging the attachment of a host that
   runs none of them. The guard is conservative rather than wrong, and the scenarios record
   the refusal. The operator decided on 2026-11-11 to keep the pool-wide guard while
   storage administration is experimental.
31. **A repository refusal spoke of a VM and did not say why.** Submitted a few seconds
   after the CHAP repository was created, the coordinator detach was refused by the client's
   own repository guard, which rejects an attachment change while the repository reports an
   operation in progress or when the host no longer advertises the action; both paths
   returned the generic action error whose text reads "unavailable for this VM", so the
   record did not say which. The repository guards now return a storage prerequisite error
   naming the reason (unit covered), the scenario waits for the host to advertise unplug and
   detaches the coordinator last, and the rerun passed in 38 s.
32. **Disabling HA leaves the statefile listed.** After `pool.disable_ha` XCP-ng 8.3 reports
   `ha_enabled` false and advertises `ha_enable` again, but `pool.ha_statefiles` still names
   the "Statefile for HA" disk on the heartbeat repository. The multipath maintenance guard
   required both HA off and an empty statefile list, so it refused the member with HA
   disabled, and it reported that refusal as an invalid XAPI response. The guard now requires
   HA off and no pool operation in progress, which is where an HA enable or disable in flight
   actually shows, and every storage-graph precondition now reports as a storage prerequisite
   (unit covered). Destroying or forgetting the heartbeat repository, and deleting the
   leftover statefile disk, stay refused by the client's dependency guards while the pool
   lists it. The operator decided on 2026-11-11 not to offer that cleanup while storage
   administration is experimental.
33. **A stock XCP-ng host's local storage is its local-cache repository.** Both pool hosts
   report `local_cache_sr` pointing at their thin-provisioned local EXT repository with
   `local_cache_enabled` set, as the installer leaves it. The multipath flow refused any host
   with such an attachment, so it could not run on either host. The reference XenAdmin action
   unplugs every attached PBD including that one; this client instead marks the attachment as
   the host's local cache in the review and keeps it attached, because multipathing only
   affects block-storage replugs and detaching the cache repository is not needed. The host
   must still report the same local-cache binding at execution (unit covered).
34. **The iSCSI driver rewrites the attachment configuration on a multipath plug.** When the
   member's LVM-over-iSCSI attachment was plugged with multipathing on, the driver persisted
   the portal list it discovered as `multihomelist` ("192.168.253.20:3260,192.168.254.20:3260")
   in `PBD.device_config`; the coordinator's attachment, plugged without multipathing, has no
   such key. The flow's configuration hash check treated that as a reviewed-configuration
   change and left the operation outcome-unknown after two of five replugs, with the member
   in maintenance and its NFS and tools-ISO attachments detached; nothing was compensated, as
   designed, and the pool-wide unresolved-operation guard then refused the disable and the
   exit from maintenance in the same session. The check now also accepts an iSCSI attachment
   whose configuration matches the reviewed one without `multihomelist` (unit covered), and
   the scenario reattaches anything an earlier attempt left detached before it starts.
35. **The client remembered no member endpoint for failover.** A Ready connection recorded
   only the endpoints it had actually used as verified members, so when the coordinator lost
   power with HA armed the reconnect loop had nothing but the dead endpoint to try, although
   the member's certificate was pinned and the member took over as coordinator within three
   minutes (the host's own messages: master transition, host failed, protected guest restarted
   on the survivor). The connection now remembers every pool host whose certificate it can
   verify through its pinned host certificates (or the connection-wide opt-out) as a
   verified member and failover endpoint; on the reruns the connection followed the member
   about 155 s after the cut.
36. **A heartbeat repository on a storage network strands a rebooted host.** The HA
   heartbeat repository was the NFS repository reached over each host's storage interface,
   which XAPI configures only after it starts. When the old coordinator booted again with HA
   armed, its storage interface never came up (zero transmitted packets at the hypervisor),
   the HA daemon could not reach the statefile, and XAPI never finished starting: the host sat
   at its configuration console with no network and never rejoined, through two power
   cycles. Enabling HA now refuses a heartbeat repository that a host reaches over a storage
   interface without unplug protection, naming the fix, and the takeover scenario protects
   those interfaces first. The stranded host was recovered by forgetting it through the
   survivor, reinstalling it unattended and rejoining it; on the rerun, with the interfaces
   protected, the old coordinator rejoined in about 175 s.
37. **A dead host's local-cache repository could not be forgotten.** The storage forget and
   destroy guards refused any repository flagged as a local cache, even after the host that
   used it had been forgotten and no host bound it. The guard now refuses only while a host
   still names the repository as its cache (unit covered).
38. **Forgetting a host leaves its repositories' advertised operations stale.** After
   `host.destroy` the dead host's local repositories kept advertising attach and detach
   operations and never advertised forget, so the client's reviewed forget stayed refused;
   XAPI's `SR.forget` accepted them directly. The recovery utility waits for the
   advertisement and records it; the lab repair used the direct call and is noted as such.
39. **A sparse appliance export ends a disk's chunk sequence early.** The Xen Orchestra
   appliance image omits every all-zero chunk, including the trailing ones of its 20 GiB disk,
   so the client's XVA stream verifier reached the end of the archive while still expecting
   chunks and left the import outcome-unknown after streaming 1.9 GB. The verifier now ends a
   disk's sequence when the next member belongs to another disk or the archive ends, and
   accepts an omitted first chunk (unit covered).
40. **An upload's reply wait aborted a long import.** The host answers an import request's
   headers at once and sends nothing more until it has consumed the whole body and finished
   the import. The client read that reply with a one-minute idle limit, so a 1.9 GB appliance
   import stopped after 180 s with about 900 MB sent, the client reported its own timeout and
   the host reported a premature end of stream; three attempts, including one after the sparse
   chunk fix, ended the same way. The reply wait is now bounded by the transfer request
   timeout and by cancellation rather than by an idle limit.
41. **A local permission refusal was worded as a host refusal.** The read-only directory
   session's write was stopped by the client's own permission guard, which never sent the
   request (no such call in the RPC trace), but the outcome read "XAPI refused the request:
   RBAC_PERMISSION_DENIED" as if the host had answered. The guard now reports that the
   signed-in role does not allow the method and that nothing was sent. XCP-ng 8.3 also creates
   directory subjects with no roles, so a new subject can do nothing until a role is granted.
42. **The search fields' Clear buttons did nothing.** The inventory search and the two
   Activity filters draw the kit's clear button, and the kit leaves that button's press to
   the application; none of the three handled it, so a pointer click changed nothing and the
   button exposed no accessible action (the second desktop session had noted the missing
   action). Each field now clears itself and refilters, with a Rune test per view; the fix
   was rechecked on the live desktop.
43. **Actual size showed an empty console panel.** The canvas kept its full-width request
   when it was given the frame's own size, and inside the panning view, whose content has no
   definite width, that resolved to zero width: no frame, no input and no canvas on the
   accessibility bus until Fit to panel was chosen again. No headless scene had ever drawn a
   console frame, so the checks could not see it. Each mode now sets its own sizing, two new
   scenes render a synthetic frame in both modes in the check script, and a Rune test asserts
   the canvas rectangle; the fix was rechecked on the live desktop with exact pointer mapping.
44. **A key repeat in flight pressed a released key again.** When the window lost focus the
   client released the held key in the guest at once, and 9 ms later the guest saw one more
   press and release of it: a repeat already queued arrived after the release and was
   forwarded as a press, and a second release then followed. The key was never left held. The
   physical-key path now drops a repeat for a key this console does not hold (unit covered),
   and the rerun showed one press, the repeats and one release.
45. **Smaller desktop observations, unchanged.** Selecting a guest in the tree while the
   search hides it shows no details and no hint that the filter is the reason. The object
   count reads "1 objects". The certificate endpoint field's placeholder wraps onto the
   Inspect certificate button below it. Controls below a dialog's fold were again absent from
   the accessibility bus until the dialog was scrolled (finding 6).
46. **Controls were hard to tell from labels, and nothing ranked them.** The kit's neutral
   theme fills a standard button with the colour of the card behind it and edges it with
   five percent white, outlines a secondary button about as faintly and draws an unchecked
   box the same way; the application used the text-only variant for 78 buttons and the
   emphasised variant for none. The desktop now builds its theme in one module that gives a
   standard button its own fill and a visible edge, takes an outlined control's edge to three
   to one against its surface, and gives the emphasised button the accent colour with white
   text above 4.5 to one (unit covered). The levels are applied by role: outlined buttons
   navigate or dismiss, filled buttons act on the selected object, red labels mark
   destructive and forced actions, and the accent marks the one action a view is for (Open
   console, Connect). Tabs sit in one bordered strip with the current tab raised; other
   choices are filled when current and outlined otherwise. No button is text-only outside a
   tab strip. Verified in headless renders of both themes, and the operator confirmed the
   improvement on their own seat.
47. **Text was too small, with no adjustment.** The interface now has a size setting from 80
   to 200 percent, in Options and on Ctrl with plus, minus and 0. The view is laid out at
   the window's size divided by the zoom and drawn that much larger, so text, controls and
   spacing scale together and the narrow layouts follow the space that is left; a focused
   console still receives those keys itself. Detached console windows are not scaled yet.
   Rune test and headless render at 125 percent, and the operator confirmed it working on
   their own seat.
48. **Column widths were written only as the window closed, and lost when a connection was
   activated.** Widths lived in the table's state until the close handler copied them out,
   and activating a connection rebuilt that state from the last saved values, so a session
   that ended any other way, or a width changed before connecting, was lost. The sidebar
   and column widths are now copied out as they change and written 0.6 s after they rest,
   and are kept across the rebuild (unit covered). The operator confirmed the sidebar and
   column widths now persist; the details pane's width is not saved, which they set aside.
49. **Every metrics refresh was refused.** The first answer drew the charts and each
   refresh after it failed with "invalid or duplicate metric timestamp", so the view showed
   "Performance data unavailable" over charts that had stopped advancing; the earlier live
   scenario only ever read the first answer. Asked from an exact sample time, XCP-ng 8.3.0
   reports the range as starting one step later and still returns the sample at the time
   asked, and when nothing newer exists it reports a start one step past its end. The
   decoder held rows to the reported range. It now allows one step of slack either way and
   refuses anything further (unit covered with the recorded shapes), and the live scenario
   follows three refreshes of a guest and of a host with no error.
50. **A host that has run HA had no charts at all.** Such a coordinator lists data sources
   with path-like names (`network/latency`, `statefile/<uuid>/latency`,
   `xapi_healthcheck/latency`); the client's name rule refused the slash and with it the
   whole source list ("invalid or duplicate metric source name"). The rule now allows a
   slash and still refuses anything that could extend a query string, and a source or
   column the client cannot name is left out instead of refusing the rest (unit covered);
   the live scenario lists the coordinator's 93 sources and charts them.
51. **A user whose access was removed stayed connected.** Removing a directory subject
   destroyed the subject and left its open sessions alone, as the review text said
   ("Existing sessions are managed by the host"): the removed user's client stayed Ready
   with its role until something else closed the session. The reference client closes the
   subject's sessions before it removes the subject, which a client that signs in again at
   once survives. The client now closes them after the removal has been confirmed, so there
   is nothing left to sign in with; a refusal of that call leaves the operation "outcome
   unknown" with the reason, and a recheck repeats the session close and never the removal
   (fixture covered). The live rerun shows the removed user's client stopping with the
   host's authorization refusal. As in the reference client, the sessions of a removed
   group's members are not closed this way: the host identifies a session by its user.
   XAPI's source rechecks directory sessions every five minutes by default and ends those
   that no longer have access; the lab run did not wait for that.
52. **A toolstack restart is refused while a console is open, without saying so.** The host
   keeps an unfinished task, "Connection to VM console", for every open console, and the
   client refuses to restart a toolstack while any task is unfinished. The refusal read
   "Finish pending pool tasks before restarting the toolstack", which sent the operator
   looking for a task. It now adds that an open VM or host console counts as one. The
   guard itself is unchanged.
53. **A password store with no password of its own stayed "locked".** The operator's
   desktop keeps its keyring without a password. Such a keyring reports itself locked after
   its daemon starts and opens at once, with no prompt, for the first application that
   asks. The client never asked: it read the locked state and told the operator to unlock
   the store in a password manager, which that desktop does not have. The client now asks
   the store to unlock whatever it found locked and reads the state again. A store that
   wants its password answers with a prompt of its own, which the client dismisses and
   never starts, so that case still ends in the locked notice and no dialog opens that the
   operator did not ask for (fixture covered). Both cases were then run against a real
   GNOME keyring daemon on a private bus with private data: a passwordless keyring that
   began locked was written, read and cleared, and a password-protected one that had been
   locked refused all three while the daemon went on answering. The keyring's own unlock
   dialog is not offered from the client.
54. **Clipboard exchange with a guest did not work for the operator.** The client sends and
   receives clipboard text as console cut-text messages, as the reference client does. A
   guest receives them only through guest tools that implement clipboard exchange; the
   lab's Linux guests have none, and which guests the operator tried was not recorded. The
   console had said "Clipboard text sent to the console", which read as success. It now
   says that the guest receives the text only if its tools support the exchange, and the
   feature is recorded as experimental. Whether it works with a guest that has such tools
   is unverified. At the operator's request the client then gained a way to type clipboard
   text as key presses, which needs nothing from the guest (see the scenario table).

55. **The pool's HTTPS-only setting could never appear.** The client read `https_only`
   from the pool record, and XAPI has no such field: the setting is kept on each host,
   and only the call that changes it is on the pool. Against a real pool the value was
   therefore always unknown, the control to change it was never offered, and the reviewed
   operation would have refused itself. The fixture's pool record carried the field, so
   every test passed. The generated bindings refused to compile the read, which is how it
   was found. The client now takes the value the hosts agree on, checks it on the hosts
   before the change and verifies it there afterwards, and the fixture has the API's
   shape (fixture covered). On the lab the setting was shown, turned on and turned off
   again, with plain HTTP to the coordinator refused in between.

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

The harness scenarios used the core facade. The desktop scenarios used the Rune window under
a nested compositor with injected input: keys through the compositor's dispatcher and, from
2026-11-11, the pointer through a virtual pointer device. That establishes what the client
does with Wayland and Xwayland input events; it does not establish a real seat, a standalone
X server with a window manager, clipboard exchange with a guest, Orca speech, non-US layouts,
software rendering (the workstation has no software Vulkan driver installed) or the wheel's
direction. The packaged client ran on the development workstation, not on a clean system.

The recovery gate has been run through the core facade with Xen Orchestra managing the pool
and through the packaged desktop client; in both, XO was made unavailable by shutting its VM
down. A third run, through the core facade only, left the VM running and detached its NIC;
a fault in the network between the workstation and a running, attached XO has not been
produced, and the desktop client has not been used for that case. A session closed on the
host with no outage has been followed through the core facade for a directory user; the
local root account's session cannot be closed that way. Interruption paths
for the passed workflows, storage and network failure cases and the remaining ledger rows
are recorded as they are exercised; most are still open. Everything here is XCP-ng 8.3.0.

The lab's steady state after the last run: the two-host pool with both hosts enabled, the
Xen Orchestra appliance running on the member and answering, the plain guest, the tools
guest and the desktop guest running on the coordinator, the shared NFS and iSCSI repositories attached, HA off, and
the third host standing alone for cross-pool scenarios.
