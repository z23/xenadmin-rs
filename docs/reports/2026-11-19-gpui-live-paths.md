# GPUI front end: the remaining live paths, headless against the lab (2026-11-19)

Note added later the same day, after this branch was merged with the day's others: three of the
scenarios below (`lab_p3_refused_reviews`, `lab_p3_snapshot_with_memory` and
`lab_p3_console_window_and_typing`) were run once more on the merged tree and passed; see
"Repeated on the merged tree" in [the first report](2026-11-19-gpui-live-headless.md). The gate
reports and the plan's Status section now use this report. The text below is unchanged.

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

| # | Path | Test | Result | Duration | Evidence directory |
| --- | --- | --- | --- | --- | --- |
| 1 | P3.1: reboot, suspend and resume, the force actions, Start on Server | `lab_p3_power_paths` | passed | 75.5 s | `1795066158-p3-1-power-paths` |
| 1 | P3.1: a batch that stops on a failure | `lab_p3_power_batch_stops_on_a_failure` | passed | 90.1 s | `1795066234-p3-1-power-batch-stops-on-a-failure` |
| 2 | Refused reviews | `lab_p3_refused_reviews` | passed | 52.1 s | `1795065756-p3-refused-reviews` |
| 3 | P3.7: live migration with disks, there and back; forced shutdown during it refused | `lab_p3_migration_with_disks` | passed | 126.8 s | `1795066432-p3-7-migration-with-disks` |
| 3 | P3.7: the same migration cancelled from the client | `lab_p3_migration_cancelled` | passed | 43.6 s | `1795066559-p3-7-migration-cancelled` |
| 3 | P3.7: a halted VM's disks moved and back | `lab_p3_halted_disk_move` | passed | 109.6 s | `1795066603-p3-7-halted-disk-move` |
| 4 | P3.4: snapshot with memory | `lab_p3_snapshot_with_memory` | passed | 202.2 s | `1795066713-p3-4-snapshot-with-memory` |
| 5 | P3.3: separate window, Fullscreen, Type Clipboard as Keys, Ctrl+Alt+Del | `lab_p3_console_window_and_typing` | passed | 174.1 s | `1795066915-p3-3-console-window-and-typing` |
| 6 | P3.6: Restart Toolstack on the member and its refusal | `lab_p3_member_toolstack_restart` | passed | 65.2 s | `1795064283-p3-6-member-toolstack-restart` |
| 6 | P3.6: Reboot of the member through maintenance | `lab_p3_member_reboot_through_maintenance` | passed | 213.2 s | `1795064355-p3-6-member-reboot-through-maintenance` |
| 6 | P3.6: Shut Down of the member through maintenance, and its return | `lab_p3_member_shutdown_through_maintenance` | passed | 195.1 s | `1795064584-p3-6-member-shutdown-through-maintenance` |
| 7 | P3.8: the coordinator's toolstack restarted by another administrator | `lab_p3_recovery_coordinator_toolstack_restart` | passed | 144.3 s (130 s of it the wait the lab asks for after such a restart) | `1795066014-p3-8-coordinator-toolstack-restart` |
| 7 | P3.8: a member lost and returned | `lab_p3_recovery_member_lost_and_returned` | passed | 783.5 s | `1795064787-p3-8-member-lost-and-returned` |
| 7 | P3.8: a certificate that is not the pinned one | `lab_p3_recovery_changed_certificate` | passed | 15.5 s | `1795067090-p3-8-changed-certificate` |
| base | G2: Add Server, inventory follows the host (run last) | `lab_g2_add_server_and_follow_the_host` | passed | 17.3 s | `1795067112-g2-add-server-and-follow` |
| base | P3.1, P3.2: power and Events (run last) | `lab_p3_power_and_events` | passed | 37.7 s | `1795067130-p3-1-power-and-events` |

The rows of paths 1 to 5 and 7 (except the lost member) and the two base rows are runs made
between 16:23 and 16:46 at commit `a87d6b6`, the last commit that changed the tests. The three
host scenarios and the lost member are runs made between 15:59 and 16:19, at the commits at
which they were written; the commits after them changed only the code of the recovery scenario of
the coordinator's toolstack. The reboot, the shutdown and the lost member were run once each and
not repeated: each is a reboot or a power cycle of the member. The member's toolstack scenario
was run three times and passed at the third (its two failed runs are among those listed below);
the coordinator's was run three times as well: passed, failed, passed.

Runs that failed on the way, all kept under the evidence directory with `"outcome": "failed"`:
11 runs of 8 scenarios, every one a matter of the test or of the harness and none a wrong result
of the application (an operation refused by the host has the state `Rejected`, not `Failed`; a
review is a group whose lines are described under it; a window of the test platform is active
only when told; the toolbar has no power buttons while the inventory is stale; one run of the
certificate scenario was an exploration that ends in a panic by design). Two of them left
something to learn about the product: findings 1 and 2.

Not restored when a host scenario fails half way: the reboot and the shutdown through maintenance
bring the member back only where their helpers return. A panic inside them (a review that does
not open, a record that is not listed in time) leaves the member disabled, or powered off and
still in maintenance; no utility of the tests puts it back, and it is done by hand (`virsh
domstate` and `virsh start` for the member's machine, then Server > Exit Maintenance Mode). The
two scenarios passed in the one run each had. (Found by the review of 2026-11-20, which also made
the toolstack scenario stricter: it now fails, confirming nothing, if Restart Toolstack is offered
while the member's console is open, where it used to accept a review that cannot be confirmed or
a request the host refuses as the refusal. The stricter scenario has not been run.)

Utilities, not scenarios: `lab_state` (read-only; run after every scenario),
`lab_baseline_plain_guest_on_the_coordinator` (new: moves the plain guest home with the
application's own wizard when a failed run left it on the member; needed once, see finding 2), and
the core's `lab_move_xo_appliance` (moved the appliance to the coordinator before the host
scenarios and back to the member after them).

### 1. Power (P3.1)

`lab_p3_power_paths`, on `lab-guest`, with "review routine power actions" switched on in Options
for the routine actions:

- **Suspend and Resume** from the VM menu through "Confirm power operation": suspended in 6.1 s,
  resumed in 3.1 s; the guest's start time is the same before and after (it did not boot).
- **Reboot** through its confirmation: 4.3 s, with a newer start time.
- **Force Reboot and Force Shut Down** through "Confirm force operation", which is asked whatever
  the option says: 2.1 s (newer start time) and 6.2 s (halted).
- **Start on Server, refused.** The halted guest's disks are on the coordinator's local
  repository. Start on Server with the member chosen is submitted and the host refuses it: the
  record is `Start on host • Rejected` on the General tab and in the Events tab, whose outcome
  reads "XAPI refused the request: VM_REQUIRES_SR". The host's reason is shown to the user as the
  host gave it, by its error code.
- **Start on Server, accepted** with the coordinator chosen: running there after 11.0 s.

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
  member in 22.9 s and back in 22.9 s; after each leg this client and a second one read the guest
  running on the destination with every disk on its local repository.
- `lab_p3_migration_cancelled`: the same migration, cancelled from the client 8.4 s after Apply
  with "Review cancellation request…" on the General tab and its confirmation. The record ended
  `Cancelled`, "Host task cancelled; original placement verified", 11.7 s after Apply; the guest
  ran on the coordinator with its disks on its local repository as read by this client and by
  another; the destination repository held after the cancellation what it held before
  (finding 9). One record, nothing sent twice. See finding 1 for the review that was refused first.
- `lab_p3_halted_disk_move`: `lab-guest` shut down, its two disks moved to the member's local
  repository and back (22.1 s and 22.9 s), the review saying "Remain halted; change home host
  after verified move"; the guest stayed halted through both and was started again on the
  coordinator. See finding 3 for the home server it is left with.

### 4. Snapshots (P3.4)

`lab_p3_snapshot_with_memory`: at a root shell on the Console tab a file is made in `/dev/shm`
(memory only) and a probe command paints the screen when the file exists. Take a Snapshot with
"Include guest memory": in the Snapshots tab after 10.9 s, the snapshot's power state
`Suspended`. The file is removed (the probe paints nothing). Revert through its review with the
shutdown and "Resume restored memory after restore" chosen: 14.9 s, records `Create memory
snapshot`, `Restore snapshot`, both succeeded. The console then shows the shell that was logged in
before the snapshot, no login is asked, and the probe paints again: the guest resumed with the
snapshot's memory. The snapshot was deleted with its two disks ("Also delete 2 exclusively owned
snapshot disk(s)", 0.4 s); a second client reads no snapshot of the guest.

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
- **Send Ctrl+Alt+Del** from the tab's row: the guest's start time changed 5.3 s later and it ran
  on the same server; no power record was added (the reboot is the guest's own). The tab said
  "Console disconnected: No RFB console is available for this VM. …" and "Waiting to reconnect to
  this console (attempt 1 of 5)", and its picture was changing again 2.2 s after that without
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

### 7. Connection recovery in the interface (P3.8)

- `lab_p3_recovery_coordinator_toolstack_restart`. `lab-guest` is selected with its General tab
  open; another administrator restarts the coordinator's toolstack (`lab/other_admin.py
  restart-toolstack`). What the window showed, in seconds after the request:
  - 1.1 s: the pool's row of the tree says "Reconnecting…";
  - 1.3 s: the status bar says "Connection lost • inventory is stale • reconnecting…" (the
    connection's state is `Reconnecting`, "Cannot reach the XCP-ng management endpoint");
  - while it is away (frame `06-while-away.png`): the header and the General tab still show the
    guest from the last inventory, its row still says "Running", and the toolbar has no VM power
    button at all (Back, Forward, Add Server, New Storage and a disabled New VM remain);
  - 8.5 s: the pool's row says "Loading inventory…";
  - 8.8 s: the status bar says "Connected directly • https://192.168.254.11/" and the connection
    is current again, with nothing asked of the user.

  That is the design of `docs/ARCHITECTURE.md` (keep the last snapshot visibly stale, build a
  replacement inventory, then swap it). The selection and the tab were kept. No record of an
  operation appeared and the guest's start time did not change. Then a description changed by a
  second client appeared in the General tab 0.2 s later, and was put back: the connection follows
  the host again. "Nothing is sent twice" is shown here only in this sense: nothing was in flight
  when the toolstack went away, and the reconnection submitted nothing. An operation in flight
  across a toolstack restart was not produced through the front end (the first report's "client
  closed in the middle of an operation" and the core's own scenarios are the nearest evidence).
- `lab_p3_recovery_member_lost_and_returned`. The member, with no guest on it and its General
  tab open, loses power (`virsh destroy`). For 616 s the window showed no change at all: the
  member's row unmarked, the header "Enabled", the status bar "Connected directly". That is the
  time the pool takes to declare a silent member dead; the client shows what the pool says. After
  it the General tab's property reads "Host live = No"; the row of the tree and the header still
  show an enabled server (finding 5). The connection to the pool stayed current throughout. The
  machine was started: 138 s later the member's row said "Disabled" (the host answers again and
  is not yet enabled), at 141 s it had a newer boot time and at 150 s it was enabled and
  unmarked, by itself. The selection was kept; a second client reads the member enabled.
- `lab_p3_recovery_changed_certificate`. No host's certificate was reset. Add Server is filled in
  with a pin that is not the coordinator's (the member's certificate fingerprint) and with a
  password that is deliberately not the lab's. Connect: the attempt ended at once with "TLS
  verification failed for https://192.168.254.11/. Verify this host's certificate and add its
  endpoint-specific fingerprint", the connection saved and shown as "Connection failed". The
  refusal is the certificate's and not a refused login, although the password was wrong too:
  nothing was authenticated before the certificate was checked. (That no credential left the
  client is the core's order of work, TLS before login; the test sees its effect, not the wire.)
  "Review this endpoint's certificate…" on the Home tab opens the form with no password in it and
  the inspection endpoint preset; Inspect certificate gives the review headed "Certificate
  changed" with "Previous SHA-256" the saved pin and "Presented SHA-256" the fingerprint the core
  inspected. The replacement is refused ("Type the exact endpoint to update only its certificate
  pin.") until the exact endpoint is typed and the verification is confirmed; then the pin field
  holds the presented fingerprint, and with the lab's password the connection came up. No file of
  the state directory holds the password.
- **A coordinator change** was not run: see "What was not run".

## Findings

No defect of the product was found that had to be changed, and no product code was changed: the
branch differs from `gpui-migration` in `tests/lab_live.rs`, `tests/lab_support/mod.rs` and two
documents. What the runs showed that an owner of the product may want to decide on:

1. **The review of a cancellation is overtaken by the progress of what it cancels.** "Review
   cancellation request…" binds its review to the record's revision
   (`areas/power/recovery.rs`, `cancellable`). The record of a running storage migration changed
   about once a second (4 changes in 4 s, sampled in both runs). A review left open for about a
   second (the time of one capture) refused its confirmation: "The operation or connection
   changed. Close this dialog and review its current state before requesting cancellation."
   Opened again and confirmed at once, it was accepted, in both runs at the second attempt. A
   person who reads the review before confirming will usually be refused and has to be quicker
   than the next progress report. The rule came with the port from Rune; it was not changed here,
   because how tightly a review binds is a safety decision.
2. **A migration goes on when its client is gone.** Once a test ended (a panic of the test)
   seconds after Apply; the host finished the migration without a client, and the guest was found
   on the member with its disks there. That agrees with the first report's closed-client scenario
   and is not a defect; `lab_baseline_plain_guest_on_the_coordinator` moved the guest back
   through the wizard.
3. **A halted move leaves a home server.** `lab-guest` had no home server before the first run of
   the halted move; the move there set it to the member and the move back to the coordinator, as
   the review says ("change home host after verified move"). The guest is left with the
   coordinator as its home server.
4. **Why Restart Toolstack is refused is not said.** With a console open the item's reason is one
   sentence for three conditions and does not name the console. It is the core's sentence:
   recorded as gap 13 in [the core gaps](2026-11-19-gpui-core-gaps.md).
5. **The tree has one word for a server that is not in service, and none for one that is lost.**
   A member in maintenance, one whose toolstack restarts, one that reboots and one that is
   switched off all show "Disabled". A member that lost power shows nothing for ten minutes (the
   pool's own delay) and after that still nothing in the tree or the header: only the property
   "Host live = No" on its General tab says it.
6. **"inventory is stale" for a connection that never had one.** After the refused certificate
   the status bar says "Connection failed • inventory is stale"; no inventory was ever loaded.
7. **One sentence for two refusals of a revert.** When the snapshot under an open revert review
   is deleted by another client, the disabled Confirm gives the same reason as for a missing
   choice ("… explicitly select a permitted clean shutdown …") although the shutdown is chosen;
   the review itself adds the notice that the object is no longer current.
8. **A host's refusal is shown by its code.** Start on Server refused by the host reads "XAPI
   refused the request: VM_REQUIRES_SR": the host's actual reason, untranslated.
9. **A storage "base copy" stays for a while on the repository a VM was migrated away from.** The
   member's local repository held none at 15:41 and one unmanaged "base copy" at 16:36, 40 s
   after the round trip of the migration with disks; the cancelled migration left the count as
   it found it both times. The host's storage layer removes these on its own schedule (it had,
   by 15:41, after the first round trip of 15:36). Not looked at again after 16:36.

Harness: `Client::activate`, `try_press_in`, `key_in`, `type_keys_in`, `pixels_in`,
`is_fullscreen` and `copy_to_clipboard` were added to `tests/lab_support/mod.rs` for a second
window and the clipboard; a drop-down is driven by the keyboard (`pick`); `review_lines` reads a
review's lines (the first report's migration steps recorded an empty review text for that reason;
nothing was asserted on it then).

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

At the last commit that changed code (`a87d6b6`; the commits after it change this report only):

- `cargo fmt --all -- --check`: passed.
- `cargo clippy -p xenadmin-desktop --all-targets --features scenes --locked -- -D warnings`:
  passed.
- Not run, because nothing outside `tests/lab_live.rs` and `tests/lab_support` was changed but
  two documents: the workspace clippy, `cargo test -p xenadmin-ui` and
  `cargo test -p xenadmin-desktop --features scenes`.
- `lab_state` after every scenario; the two base scenarios last (the table).

The PNG frames of these runs were not looked at by a person. Every statement above rests on an
assertion or a recorded value: the model's description, the element tree of the frame, or pixels
of the captured frame (the colours a typed command painted in the guest).

## State of the lab afterwards

Read with `lab_state` at 16:46 AEDT, after the last scenario: both servers enabled, h1 the
coordinator; `XOA` running on h2; `lab-tools-1794263807` and `lab-guest-1794316526` running on
h1; `lab-desktop-1794365323` halted, not touched; no snapshot in the pool; no pending task
(`lab/other_admin.py pending-tasks`); high availability not touched. libvirt: h1, h2 and the NAS
running, h3 and dc shut off as before.

What differs from how the lab was found:

- `lab-guest` has the coordinator as its home server (finding 3); it had none.
- The member was rebooted once, shut down and started once and lost power once; its toolstack
  was restarted twice by the application (one of them in a run that failed afterwards on the
  test's own check) and the coordinator's three times by `other_admin.py` (the last at 16:27).
  Both guests were shut down, rebooted and started many times.
- The appliance was moved to the coordinator at 15:55 and back to the member at 16:22 by the
  core's `lab_move_xo_appliance`.
- A "base copy" may still be on the member's local repository (finding 9).

Evidence of every run, failed ones included, is under
`/var/lib/libvirt/images/xenadmin-rs-lab/evidence/gpui/`, directories `1795052692-…` to
`1795067130-…`. Nothing of it is in the repository.
