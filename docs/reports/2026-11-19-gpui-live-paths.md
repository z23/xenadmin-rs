# GPUI front end: the remaining live paths, headless against the lab (2026-11-19)

This report continues [the first live headless report](2026-11-19-gpui-live-headless.md). It
records the paths that gate G3, criterion 2, of [the migration plan](../GPUI-MIGRATION.md) names
and that the first run left out
([gate report G3](2026-11-19-gpui-gate-g3.md), criterion 2, "Missing"), run through the front
end against the project's disposable lab. The method is the one of the first report and has the
same limits: it is evidence for "live-host-verified, headless"; it is not evidence for a real
window.

## Method, and what it is not

As in the first report ("What headless against the lab is", "What it is not"): the application is
started in the test process as `app::start` wires it, on GPUI Kit's headless platform with the
offscreen renderer, on a fresh temporary state directory, against the real hosts of the lab. The
test acts as a user can (presses and typing on elements found by their keys, the menu bar, the
toolbar, rows of local actions, dialogs); what it reads of the model is for assertions and for
waiting. Changes "from outside" go through a second core connection of the test's own or through
`lab/other_admin.py` (another administrator's pinned JSON-RPC session). A power cut and a power
button of the member's machine are `virsh destroy` and `virsh start` of its libvirt domain.

What these runs do not establish, beside the list of the first report:

- **Fullscreen.** The scenario presses Fullscreen in a console window and on the Console tab and
  reads the window state the Kit's test platform holds (`false`, `true`, `false`). That shows the
  intent reaches the right window. No compositor made anything fullscreen.
- **The clipboard.** "Type Clipboard as Keys" read the clipboard of the Kit's test platform, into
  which the test had put the text. No desktop clipboard, no portal and no other application were
  involved.
- **A second window.** The console window is a second window of the headless platform. The test
  activates it before a press, as a press in a window of a desktop does; focus changes between
  real windows and the release of held keys when a real window is left are not shown.
- **Keys.** Keys typed at a canvas are the Kit's test key events, as in the first report; the keys
  of "Type Clipboard as Keys" are produced by the model and the core from the text, and their
  effect was checked in the guest.

Xen Orchestra was never stopped. The application has no code path to it; the appliance was moved
between the hosts by the core's own scenario around the host scenarios and is back on the member.

## The lab and its rules for this run

The lab of the first report: `…-h1` (coordinator, 192.168.254.11) and `…-h2` (member,
192.168.254.12), the NAS appliance, the guests `lab-guest-1794316526` (Alpine, no tools, local
repository of h1), `lab-tools-1794263807` (Alpine with tools, shared NFS repository) and `XOA`.
`lab-desktop-1794365323` was halted and was not touched.

Allowed in this run, unlike the first: a toolstack restart, a reboot, a shutdown and a power cut of
the member, and a toolstack restart of the coordinator by another administrator. Not done, by the
rules: nothing powered off or reset the coordinator; high availability stayed off; the accepted
path of a forced shutdown during a migration was not taken (only its refusal); no storage
repository, network, bond, management interface, pool membership, certificate or pool secret was
changed; nothing was done on the NAS; no guest or disk was deleted other than the snapshots a
scenario took itself.

## Scenarios

All are `#[ignore]` tests of `crates/xenadmin-desktop/tests/lab_live.rs`, run one at a time with
`cargo test -p xenadmin-desktop --features scenes --locked --test lab_live -- --ignored --exact NAME --nocapture --test-threads=1`
and the variables `XENADMIN_LAB_MANIFEST`, `XENADMIN_LAB_PASSWORD_FILE` and `XENADMIN_STATE_DIR`
(a fresh temporary directory for each run). Evidence is outside the repository, under
`/var/lib/libvirt/images/xenadmin-rs-lab/evidence/gpui/<unix time>-<scenario>/`: numbered PNG
frames, element trees and a `summary.json` with the steps, their times and the results. The table
gives the last run of each scenario, on 2026-11-19 local time (AEDT). Durations are of the whole
test, connection included.

@@TABLE@@

Utilities, not scenarios: `lab_state` (read-only; run after every scenario),
`lab_baseline_plain_guest_on_the_coordinator` (new: moves the plain guest home with the
application's own wizard when a failed run left it on the member; needed once, see finding 2), and
the core's `lab_move_xo_appliance` (moved the appliance to the coordinator before the host
scenarios and back to the member after them).

### 1. Power (P3.1)

`lab_p3_power_paths`, on `lab-guest`, with "review routine power actions" switched on in Options
for the routine actions:

- **Suspend and Resume** from the VM menu through "Confirm power operation": suspended in 13.7 s,
  resumed in 6.3 s; the guest's start time is the same before and after (it did not boot).
- **Reboot** through its confirmation: 8.4 s, with a newer start time.
- **Force Reboot and Force Shut Down** through "Confirm force operation", which is asked whatever
  the option says: 2.9 s (newer start time) and 7.1 s (halted).
- **Start on Server, refused.** The halted guest's disks are on the coordinator's local
  repository. Start on Server with the member chosen is submitted and the host refuses it: the
  record is `Start on host • Rejected` on the General tab and in the Events tab, whose outcome
  reads "XAPI refused the request: VM_REQUIRES_SR". The host's reason is shown to the user as the
  host gave it, by its error code.
- **Start on Server, accepted** with the coordinator chosen: running there after 11.9 s.

`lab_p3_power_batch_stops_on_a_failure`: Shut Down for `lab-guest` and `lab-tools` as a batch from
the Search tab. While the host worked on the first guest, another administrator forced that guest
off (second core connection), which cancelled the batch's own task. The batch stopped: its record
is `OutcomeUnknown`, "Shut down VM … task cancelled; remaining VMs were not submitted. Inspect
before acknowledging", with the second guest's step "Pending — Not submitted"; the tools guest
stayed running. The Events tab shows the record and its two steps; the unknown outcome was
acknowledged there (`AcknowledgedUnknown`) and the first guest started again.

### 2. Refused reviews

`lab_p3_refused_reviews` (nothing is confirmed in it; no operation of the client is recorded):

- **A move of a VM's disks while a snapshot exists.** A snapshot of `lab-guest` is taken from
  outside. The Migrate wizard, with the member and its local repository chosen for both disks,
  reaches its review with Apply migration disabled: "Storage-transport moves with snapshot chains
  are not supported yet".
- **A revert that is refused.** Two conditions of `crates/xenadmin-ui/src/areas/snapshots`: the
  review of a running guest cannot be confirmed until its clean shutdown is chosen; and with the
  shutdown chosen, the snapshot is deleted by another client while the review is open, after which
  the confirmation is refused again and the review says "The connection is not ready or the
  selected object is no longer current".
- **Reboot and Shut Down of an enabled server**: both items of the Server menu are disabled for
  the member with "Disable scheduling and evacuate guests first; a multi-host pool coordinator
  must be handed over separately".
- **The maintenance review after the set of resident guests changed.** The review of Enter
  Maintenance Mode is opened for the empty member; another client migrates the tools guest onto
  it; Confirm is refused: "The connection, host or prerequisites changed. Close and reopen this
  review before applying it." The member stays enabled, nothing is submitted, the guest is moved
  back.

**A forced shutdown during a migration** is in `lab_p3_migration_with_disks` (scenario 3): 3.1 s
into the migration, Force Shut Down from the VM menu opens the review "VM recovery controls" with
the caution "A migration of this VM is running. …", the acceptance unticked and Confirm disabled:
"This action is refused unless you accept below what it can leave behind." The review was closed.
The acceptance was never ticked: the accepted path was not run, by the lab's rule.

Not shown: the host's own refusal of an evacuation (it needs a guest that cannot be moved; none
was made).

### 3. Moving a VM (P3.7)

- `lab_p3_migration_with_disks`: `lab-guest`, running, through the Migrate wizard with the other
  server, that server's local repository for both disks (the drop-downs driven by the keyboard)
  and the transfer network; the review names the server and both repositories by UUID. To the
  member in 24.0 s and back in 24.0 s; after each leg this client and a second one read the guest
  running on the destination with every disk on its local repository.
- `lab_p3_migration_cancelled`: the same migration, cancelled from the client 8.9 s after Apply
  with "Review cancellation request…" on the General tab and its confirmation. The record ended
  `Cancelled`, "Host task cancelled; original placement verified", 20.7 s after Apply; the guest
  ran on the coordinator with its disks on its local repository as read by this client and by
  another; the destination repository held no disk before and none after. One record, nothing
  sent twice. See finding 1 for the review that was refused first.
- `lab_p3_halted_disk_move`: `lab-guest` shut down, its two disks moved to the member's local
  repository and back (23.9 s and 24.0 s), the review saying "Remain halted; change home host
  after verified move"; the guest stayed halted through both and was started again on the
  coordinator. See finding 3 for the home server it is left with.

### 4. Snapshots (P3.4)

`lab_p3_snapshot_with_memory`: at a root shell on the Console tab a file is made in `/dev/shm`
(memory only) and a probe command paints the screen when the file exists. Take a Snapshot with
"Include guest memory": in the Snapshots tab after 12.2 s, the snapshot's power state
`Suspended`. The file is removed (the probe paints nothing). Revert through its review with the
shutdown and "Resume restored memory after restore" chosen: 16.4 s, records `Create memory
snapshot`, `Restore snapshot`, both succeeded. The console then shows the shell that was logged in
before the snapshot, no login is asked, and the probe paints again: the guest resumed with the
snapshot's memory. The snapshot was deleted with its disks (0.5 s); a second client reads no
snapshot of the guest.

### 5. Console (P3.3)

`lab_p3_console_window_and_typing`, on `lab-guest`:

- **Type Clipboard as Keys.** The clipboard (of the test platform) holds a shell command of 98
  characters that compares a variable with the text
  `Hi, World! (A+B)*C? <D> {E} ~F #G @H %I ^J &K _L |M :N` and ends with a line break. The
  variable had been set through the canvas from octal escapes, with keys that need no Shift. The
  review says "Type 98 characters, with 1 line break sent as Enter, into lab-guest-1794316526 as
  key presses? The text is not shown here. …" and does not show the text. After the confirmation
  the guest's screen turned green: the shell read exactly the text, capitals and punctuation
  included, and the line break ran it.
- **Open in Separate Window.** A second window titled "lab-guest-1794316526 · XenAdmin RS console"
  opened and drew the guest's picture at once; the tab said "No console is open for this VM."
  A command typed in the second window ran in the guest (its screen turned magenta in the window's
  own canvas; 50 keys forwarded by that canvas).
- **Fullscreen**: the window state of the headless platform went `false`, `true`, `false` for the
  console window (its Fullscreen button) and for the main window (Fullscreen on the Console tab's
  row). The intent and the window state only, as said above.
- **The window closed** with its "Close console" button: the window was gone, the tab said "No
  console is open for this VM.", Open Console on the tab's row brought the picture back in the
  tab and keys typed there reached the guest. Closing the window ends its console; it does not
  move back by itself.
- **Send Ctrl+Alt+Del** from the tab's row: the guest's start time changed 5.6 s later and it ran
  on the same server; no power record was added (the reboot is the guest's own). The tab said
  "Console disconnected: No RFB console is available for this VM. …" and "Waiting to reconnect to
  this console (attempt 1 of 5)", and showed pictures of the new boot 4.3 s after that without
  being asked.

### 6. Hosts (P3.6)

Before these, the appliance was moved to the coordinator; no guest ran on the member.

- `lab_p3_member_toolstack_restart`. With no console open the Server menu offers Restart
  Toolstack (its review was opened and closed). With the member's text console open in the
  Console tab the item is disabled: "Pool HA must be known disabled, a current toolstack start
  marker must be supplied and pool tasks must be finished" (finding 4). 4.4 s after Close Console
  it is offered again. Through its review and confirmation: the record succeeded after 18.1 s
  ("Restart toolstack: Verified — Confirmed by fresh host state"), the member's toolstack start
  marker is newer, and the member is enabled again as read by this client and by another. While
  it restarted, the member's row of the tree said "Disabled" and the status bar stayed
  "Connected directly".
- `lab_p3_member_reboot_through_maintenance`. Enter Maintenance Mode (0.6 s, nothing to
  evacuate), Reboot from the Server menu through its review: the record succeeded after 160.5 s
  with a newer boot time; the client read the member as not live 11 s after the confirmation and
  as live again 143 s later. Exit Maintenance Mode: 6.5 s; its step says "the host refused the
  call once while it was still booting and it was repeated". Enabled as read by both clients.
- `lab_p3_member_shutdown_through_maintenance`. Enter Maintenance Mode, Shut Down through its
  review: the record succeeded after 15.4 s, libvirt reported the machine "shut off" and the
  client read the member as not live. The machine was started (`virsh start`); the client read it
  live with a newer boot time 145.7 s later; Exit Maintenance Mode (6.4 s); enabled as read by
  both clients. Nothing had been halted by the power cycle.

In all three the tree marks the member "Disabled" from the moment it is disabled until it is
enabled again, whether it is in maintenance, restarting or switched off (finding 5).

@@RECOVERY@@

## Findings

@@FINDINGS@@

## What was not run

- **The accepted path of a forced shutdown during a migration**: forbidden on this lab (it wedged
  it twice, finding 63 of the lab report). Only the refusal is shown.
- **A coordinator change through the front end** (the handover to the member and back): the
  supported list of `docs/FEATURES.md` has "recovery after a lost session or a lost coordinator",
  not a handover started from the front end, and a lost coordinator means powering off h1, which
  the lab's rules forbid. Left out.
- **Maintenance of the coordinator**: it needs the handover. Left out with it.
- **The host's own refusal of an evacuation**: not produced (see scenario 2).
- **A reset certificate**: not done, by the rules. The changed-certificate path was produced with
  a pin that is not the host's (scenario 7).
- **A saved password in the desktop keyring and connection at startup**: there is no keyring on
  the headless platform, and the operator's keyring is not to be touched.
- **Remove Server, Reconnect As, a session renewed after a logout from outside**: outside this
  work list.
- Everything of "What it is not": a real window, a compositor, a seat, a desktop clipboard, a
  portal, a screen reader, the packaged binary.

## Checks

@@CHECKS@@

## State of the lab afterwards

@@STATE@@
