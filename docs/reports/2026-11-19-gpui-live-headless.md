# GPUI front end: live headless runs against the lab (2026-11-19)

Note added after the cutover of the code, later the same day: the crate this report calls
`crates/xenadmin-gpui` is `crates/xenadmin-desktop` now, and the tests are run with
`cargo test -p xenadmin-desktop --features scenes --locked --test lab_live -- --ignored --exact NAME --nocapture --test-threads=1`.
The runs described below were made at commit `e2d4572`, before the last round of fixes to the model
and the renderer (branches `port/f1` and `port/f2`) and before the cutover. Every scenario was run
again after the cutover, at commit `56fedbd`, and passed: see
[Repeated after the cutover](#repeated-after-the-cutover) at the end. The text between is unchanged.

This report records what was run of the GPUI front end against the project's disposable
lab for the gates G2 and G3 of [the migration plan](../GPUI-MIGRATION.md), with what
method, and what that method cannot show. It is evidence for "live-host-verified, headless";
it is not evidence for a real window.

## What "headless against the lab" is

The tests of `crates/xenadmin-gpui/tests/lab_live.rs` (helpers in
`tests/lab_support/mod.rs`) start the application as `app::start` wires it, in the test
process:

- the model is made as the application makes it (`launch::prepare`, `Model::live` with the
  owner of the state directory and default preferences), with the real core on its own
  runtime, the bridge that wakes the store, the real `AppStore`, `ShellView`, intents and
  console windows;
- the window is opened on the Kit's headless platform with the offscreen renderer the scene
  tool uses, so every step can be captured as a PNG of what the renderer drew, beside the
  element tree of the window and the description the model gave;
- the hosts are the real hosts of the lab: every connection, task, console stream and
  metrics request goes to them over HTTPS from the application's own connections. Waiting
  is real time (the test sleeps, advances the platform clock by what passed, and lets the
  application run until it is idle);
- the test acts as a user can: presses and typing on elements found by their keys, commands
  through the menu bar, the toolbar and the rows of local actions. What it reads of the
  model is used for assertions and for waiting only. Changes "from outside" and independent
  reads use a second core connection of the test's own.

The state directory of every run is a fresh temporary directory given through
`XENADMIN_STATE_DIR`; the test refuses a directory that is not under the temporary
directory or is not empty. The lab's password is read from its file inside the test process
and typed into the application's password field; the evidence writer refuses to write a
file that contains it, and scenarios 1 and 8 check that no file of the state directory
holds it. The password field is drawn masked (`02-connect-form.png` of the G2 run, looked at).

## What it is not

No compositor, no seat and no real window were involved. These runs do not establish, and
each remains to be shown in a real window on Wayland:

- the Wayland keyboard path: keys reached the guest through the Kit's test key events and
  the canvas, not through a seat, a keymap or the key-code path of
  [decision 0006](../decisions/0006-console-key-codes.md);
- pointer capture, focus changes between real windows, fullscreen, window activation and
  the release of held keys when a real window is left;
- the clipboard (no clipboard command was used);
- the portal file chooser: Save Screenshot was answered by the test platform's path prompt,
  on the Kit's test platform, which has no renderer (no PNG of the window in that scenario);
- a screen reader: the element tree was captured, Orca was not run;
- frame pacing, scaling and fonts of a real display: the PNGs are 1440 by 900 offscreen
  frames;
- the released binary: the tests link the same crates and call `app::start`, the client
  binary itself was not run.

Nothing here was run with Xen Orchestra stopped; the application has no code path to it,
and the appliance stayed running on the lab throughout, as the lab's rules ask.

## The lab

The libvirt lab of this workstation, from its manifest
(`/var/lib/libvirt/images/xenadmin-rs-lab/lab.json`, lab id `9e8e4f49-…`): two nested
XCP-ng 8.3 hosts in one pool (`…-h1`, the coordinator, 192.168.254.11; `…-h2`,
192.168.254.12) on an isolated network, with a NAS appliance that serves a shared NFS
repository. Guests used: `lab-guest-1794316526` (Alpine, no guest tools, on h1's local
repository), `lab-tools-1794263807` (Alpine with guest tools, on the shared repository) and
the Xen Orchestra appliance `XOA`, which was only moved between the hosts by the core's own
scenario `lab_move_xo_appliance` around the maintenance scenario. A fourth guest of the
pool, `lab-desktop-1794365323`, was halted when the runs began and was not touched.

Not done, by the lab's rules: no host or toolstack restart, no change of pool membership,
high availability, storage repositories, networks or bonds, nothing on the NAS, and no
guest or disk deleted other than the snapshot a scenario took itself.

## Scenarios

All are `#[ignore]` tests, run one at a time with
`cargo test -p xenadmin-gpui --features scenes --locked --test lab_live -- --ignored --exact NAME --nocapture --test-threads=1`
and the variables `XENADMIN_LAB_MANIFEST`, `XENADMIN_LAB_PASSWORD_FILE` and
`XENADMIN_STATE_DIR`. Evidence is outside the repository, under
`/var/lib/libvirt/images/xenadmin-rs-lab/evidence/gpui/<unix time>-<scenario>/`: numbered
`NN-<step>.png` frames, `NN-<step>.tree.json` element trees and a `summary.json` with the
steps, their times and the results. The table gives the last run of each scenario, all
between 23:35 and 23:57 UTC on 2026-11-18 (10:35 to 10:57 local on the 19th), in the order
they were run, except that scenario 2 was run once more at the end (its run in sequence,
`1795044958-p3-1-power-and-events`, also passed, in 36.2 s). Durations are of the whole test, connection included.

| # | Scenario | Test | Result | Duration | Evidence directory |
| --- | --- | --- | --- | --- | --- |
| 1 | G2: Add Server, inventory follows the host | `lab_g2_add_server_and_follow_the_host` | passed | 15.9 s | `1795044929-g2-add-server-and-follow` |
| 2 | P3.1, P3.2: power and Events | `lab_p3_power_and_events` | passed | 40.5 s | `1795046213-p3-1-power-and-events` |
| 3 | P3.3: console | `lab_p3_console` | passed | 116.5 s | `1795044995-p3-3-console` |
| 3 | P3.3: Save Screenshot | `lab_p3_console_save_screenshot` | passed | 25.9 s | `1795045111-p3-3-console-save-screenshot` |
| 4 | P3.4: snapshots | `lab_p3_snapshots` | passed | 59.6 s | `1795045138-p3-4-snapshots` |
| 5 | P3.5: performance | `lab_p3_performance` | passed | 30.5 s | `1795045198-p3-5-performance` |
| 6 | P3.6: host maintenance | `lab_p3_host_maintenance` | passed | 76.2 s | `1795045290-p3-6-host-maintenance` |
| 7 | P3.7: migration | `lab_p3_migration` | passed | 35.9 s | `1795045406-p3-7-migration` |
| 8 | P3.8: connection recovery | `lab_p3_connection_recovery` | passed | 26.1 s | `1795045442-p3-8-connection-recovery` |
| 9 | A client closed in the middle of an operation | `lab_client_closed_in_the_middle_of_an_operation` | passed | 33.8 s | `1795045468-closed-in-the-middle-of-an-operation` |
| 2 | P3.1: the power batch | `lab_p3_power_batch` | passed | 170.5 s | `1795045743-p3-1-power-batch` |

Two more tests are not scenarios: `lab_state` reads the pool through a core connection and
prints the servers, the guests and the snapshots; `lab_baseline_tools_guest_on_the_coordinator`
moves the tools guest to the coordinator with the application's Migrate to Server wizard
when it is found on the member (run once, between scenarios 6 and 7: 2.3 s for the move,
`1795045385-baseline-tools-guest-on-the-coordinator`).

### 1. Gate G2

Add Server from the Home tab's row, the coordinator's address, `root` and the password.
Under Certificate options the certificate of each of the two servers is inspected; the
review shows the SHA-256 fingerprint that an inspection through the core made by the test
reports (asserted on the description and on the element tree of the frame), the pin is
refused until the exact endpoint is typed and the confirmation is checked, then accepted.
Connect: the tree shows the pool, both servers, the guests and the repositories
(`06-connected.png`), the status bar says "Connected directly • https://192.168.254.11/".
With `lab-guest` selected, its description is changed through the second core connection;
the General tab showed the new text 0.2 s later with the selection on the same object
(`08-general-after-outside-change.png`), and the description was put back.

### 2. Power and Events

"Review routine VM power actions" is turned on in Tools > Options so that a routine action
asks for its confirmation. Start is disabled for the running guest in the VM menu with the
reason "The host does not currently allow this action", and the toolbar does not offer it.
Shut Down from the VM menu, through its confirmation: the record appears on the General tab
while the host works, ends "Shut down • Succeeded" after 8.3 s with the guest halted; the
Events tab of the Notifications view has the row ("Shut down", "Succeeded") and, with the
row chosen, the outcome with the host task's result (`08-events-history.png`). Start from
the toolbar through its confirmation: running after 6.1 s.

The power batch (its own test): the pool's Search tab narrowed to VMs by its Filters, a
press on the row of `lab-guest` and a press with the secondary modifier on the row of
`lab-tools`; Shut Down from the VM menu opens the batch on its review with both guests
listed (`07-shut-down-batch-review.png`); Run batch: "2 of 2 VM actions verified" after
12.3 s, both halted. The same selection of two halted guests, Start, the review, Run batch:
both running after 7.2 s. The Events tab has both batches as "Power actions on selected
VMs", "Succeeded". The host started the tools guest on h2; the scenario moved it back to
h1 through the wizard (7.4 s). The tools guest was halted for about 20 s.

### 3. Console

The Console tab of `lab-guest` opens its console: an 800 by 600 frame arrived and the
canvas drew it (`05-console-first-frame.png`; the test also checks that the pixels of the
canvas in the frame are not one colour). A press on the canvas gives it the keyboard;
Return changes the guest's screen. `root` is typed at the login (this guest has no
password), then a command that paints the screen red, clears it and echoes a line: the
frame shows the red screen with the echoed line and the prompt
(`09-console-command-echo.png`; the red share of the canvas is asserted, 0.49 with the
guest's 4:3 picture letterboxed). The colours are put back and the shell is left. Send
Ctrl+Alt+Del is on the row, enabled, and was not pressed. Release Input was pressed (see
the findings). The host text console of h2 opens with a 640 by 384 frame showing the
host's login screen (`11-host-console.png`); nobody logged in.

Save Screenshot (its own test, on the Kit's test platform, which answers path prompts and
has no renderer): a cancelled prompt writes nothing; an answered prompt writes an 800 by
600 PNG of the guest's picture with mode 600. The file is kept as
`guest-console-screenshot.png`.

### 4. Snapshots

Take a Snapshot from the VM menu (1.8 s), the row is in the Snapshots tab. The guest's
description is then changed from outside, as something the snapshot does not have. Revert
through its review with "shut down first" and "start afterwards": 16.0 s, the description
is the snapshot's again and the guest is running. Delete through its review (0.5 s); no
snapshot of the run is left.

### 5. Performance

The Performance tab of `lab-guest`: 15 charts, all with samples 0.3 s after the tab opened;
the first chart (`cpu0`) has 120 valid samples at 5 second steps over ten minutes, and what
the chart element painted (one stroke, the labels of the range and of the scale) is the
geometry of the model's plot at the size of the element in the frame
(`06-performance-guest.png`). The same for h1: 26 charts, all with samples
(`08-performance-server.png`).

### 6. Host maintenance

Before: `XOA` moved to h1 with the core scenario. `lab-tools` was on h2. Enter Maintenance
Mode from the Server menu: the evacuation review names the guest to move; confirmed, the
operation succeeded in 9.7 s with h2 disabled, marked "Disabled" in the tree, "Maintenance:
Yes" on its General tab (`07-in-maintenance.png`), no guest left on it, and the menu item
now "Exit Maintenance Mode". Exit through its review: enabled again after 1.1 s. The
scenario then migrated `lab-tools` back to h2 through the wizard (9.0 s), and `XOA` was
moved back to h2 with the core scenario.

### 7. Migration

Migrate to Server from the VM menu for `lab-tools`, on h1 at that point: the destination
h2 chosen, storage and networks as offered, the review, Apply: running on h2 after 4.2 s,
the General tab names h2. Then back to h1 the same way (3.3 s).

### 8. Connection recovery

Only what the lab's rules allow (no host or toolstack restart). Disconnect from the Home
tab's row: the row says "Disconnected", the status bar "No host connected", and the tree
keeps the pool as last seen under the word "Disconnected". Connect… for the saved
connection: the form has the saved endpoint and user and no password; Connect without one
is refused with a reason; with the password typed the connection is current again without a
new certificate review (the pins were saved). The application is then closed and a second
one started on the same state directory: one saved connection, shown as "Disconnected"
(`01-second-start.png`), nothing connects by itself, and Connect… asks for the password and
connects. No file of the state directory holds the password.

### 9. A client closed in the middle of an operation

Shut Down of `lab-guest` submitted (no confirmation, the default), and the whole
application (window, store, model, backend) dropped while the record was in the state
"Tracking". The host halted the guest with no client connected (read through the second
connection). A new application on the same state directory, connected with the saved
connection: the Events tab has one record of the shutdown, "Shut down", "Succeeded"
(`03-events-after-restart.png`); the guest has exactly one operation record, so nothing was
sent a second time (a second request would be a second record, or a failure of this one on
a halted guest). The guest was then started from the toolbar.

## Findings

No run exposed a defect of the product that needed a change: the diff of this work in
`crates/xenadmin-ui` and `crates/xenadmin-gpui` is the two test files only. What the runs
showed that is worth knowing:

1. **A member's console needs the member's own certificate.** With only the coordinator's
   certificate pinned, the Console tab of h2 says "Console disconnected: TLS verification
   failed for https://192.168.254.12/. Verify this host's certificate and add its
   endpoint-specific fingerprint. Reconnect when the guest or host is available." and shows
   no picture (`1795030097-p3-3-console/10-host-console.png` and its `summary.json`). This
   is the core's per-endpoint trust rule, the same for the Rune front end, and the message
   says what to do; the scenarios therefore inspect and pin both servers' certificates in
   the connect form before connecting. Nothing was changed. Whether the way from that
   message to the certificate review is short enough for a connected user was not examined.
2. **Release Input leaves the keyboard with the canvas.** It releases what is held in the
   guest and does not move the focus, as in the Rune front end. Recorded
   (`canvas_has_keyboard_after_release_input: true`), not asserted either way.
3. **A started guest runs where the host puts it.** The Start batch brought the tools guest
   up on h2 although it had run on h1; that is the host's placement, not the client's. The
   batch scenario moves it back.
4. **The Search text also matches the pool's name**, as its label says ("Name, UUID,
   description, host or pool"): in a pool named `lab-pool-…` the text `lab-` keeps all 71
   objects (`1795045703-p3-1-power-batch/05-…-grid.png`). The batch scenario narrows the
   grid with the Filters instead. The grid draws only the rows in view, so a row further
   down cannot be pressed before it is scrolled to; that is a matter for the harness.
5. **Earlier failed runs are in the evidence directory.** The scenarios were written
   against the lab between 19:14 and 19:45 UTC on 2026-11-18 by an engineer whose session
   ended before its notes were written: one failed run of scenario 2, seven of scenario 3
   and three of scenario 8 precede the first passing run of each, and two failed runs of
   the power batch (a row not drawn, see 4) precede its passing run. From their summaries:
   the last failed console run is finding 1; the others stop at a wait or a lookup of the
   test (the login sequence at the guest's console, the row of the Events list, the Home
   tab's row actions). No product change followed from any of them, and their causes
   cannot be stated more precisely than that.

## What was not run

- Anything in a real window: see "What it is not".
- Connection recovery through a lost host, a toolstack restart or a coordinator change:
  the lab's rules for these runs do not allow them. The core's own live scenarios cover
  them at the core's level; the GPUI shell was not shown through them.
- The console in a separate window, fullscreen, the clipboard commands, the key chords of
  the Console row (Ctrl+Alt+Del was deliberately not sent), SSH, RDP or a local tool.
- Suspend, resume, pause, reboot and the force actions; Start on Server; the recovery
  reviews; a batch that stops on a failure.
- Snapshots with memory, snapshot export, a revert that is refused.
- Maintenance of the coordinator; a migration with a storage move or between pools.
- A saved password in the desktop keyring and automatic connection at startup: the runs use
  no keyring, and the password is typed each time.
- Every other area of the application: storage, networks, pool membership, high
  availability, templates, import and export, schedules, users and roles.

## Checks

Run at the commit that holds the last change to the tests (`e2d4572`; only this report
changed afterwards):

- `cargo fmt --all -- --check`: clean.
- `cargo clippy -p xenadmin-gpui --all-targets --features scenes --locked -- -D warnings`: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test -p xenadmin-ui --locked`: 965 passed, 2 ignored, and 1 passed, 2 ignored in the
  documentation tests.
- `cargo test -p xenadmin-gpui --features scenes --locked`: 204 passed; the 13 tests of
  `lab_live` and one other test stay ignored.

## State of the lab afterwards

Read with `lab_state` after the last scenario: both servers enabled; `XOA` running on h2;
`lab-tools-1794263807` and `lab-guest-1794316526` running on h1; `lab-desktop-1794365323`
halted as it was found; no snapshot in the pool; the description of `lab-guest` as it was.
The tools guest was found on h2 when the first runs began and is left on h1, the baseline
named for this work.

## Repeated after the cutover

On 2026-11-19 between 01:24 and 01:36 UTC (12:24 to 12:36 local), after the two rounds of fixes
and the cutover had been merged, every test above was run again at commit `56fedbd`: the tree of
the merge `e808a2f` with the one test file changed that adds the utility named below. Each ran on
a fresh state directory, one at a time, with no display (`DISPLAY` and `WAYLAND_DISPLAY` unset).
All passed at the first attempt, and no scenario was changed for it.

| Order | Test | Result | Duration | Evidence directory |
| --- | --- | --- | --- | --- |
| 1 | `lab_g2_add_server_and_follow_the_host` | passed | 18.7 s | `1795051461-g2-add-server-and-follow` |
| 2 | `lab_p3_power_and_events` | passed | 44.5 s | `1795051480-p3-1-power-and-events` |
| 3 | `lab_p3_console` | passed | 127.8 s | `1795051525-p3-3-console` |
| 4 | `lab_p3_console_save_screenshot` | passed | 16.3 s | `1795051653-p3-3-console-save-screenshot` |
| 5 | `lab_p3_snapshots` | passed | 42.8 s | `1795051670-p3-4-snapshots` |
| 6 | `lab_p3_performance` | passed | 17.6 s | `1795051713-p3-5-performance` |
| 7 | `lab_place_tools_guest_on_the_member` (new utility) | passed | 23.3 s | `1795051785-place-tools-guest-on-the-member` |
| 8 | `lab_p3_host_maintenance` | passed | 34.9 s | `1795051808-p3-6-host-maintenance` |
| 9 | `lab_baseline_tools_guest_on_the_coordinator` | passed | 22.6 s | `1795051854-baseline-tools-guest-on-the-coordinator` |
| 10 | `lab_p3_migration` | passed | 33.5 s | `1795051877-p3-7-migration` |
| 11 | `lab_p3_connection_recovery` | passed | 26.8 s | `1795051911-p3-8-connection-recovery` |
| 12 | `lab_client_closed_in_the_middle_of_an_operation` | passed | 34.8 s | `1795051938-closed-in-the-middle-of-an-operation` |
| 13 | `lab_p3_power_batch` | passed | 186.4 s | `1795051973-p3-1-power-batch` |

`lab_state` was read before the first and after the last. Around the maintenance scenario the Xen
Orchestra appliance was moved to the coordinator and back with the core's own scenario
`lab_move_xo_appliance` (25.6 s and 10.6 s), as in the first run.

One test is new. In the first run the tools guest happened to be on the member when the
maintenance scenario began, which is what made its evacuation a real one; the baseline since then
has the guest on the coordinator, where an evacuation of the member would have nothing to move.
`lab_place_tools_guest_on_the_member` moves the guest to the member with the application's own
Migrate to Server wizard (3.3 s here) when it is found on the coordinator, and the baseline test
moves it back afterwards. In this run the member held `lab-tools-1794263807` before maintenance,
the evacuation review named it, entering maintenance took 4.6 s with the guest moved to the
coordinator and the member marked "Disabled" in the tree, leaving took 1.0 s, and the scenario
moved the guest back (3.4 s).

Other values of this run, from its summaries: Shut Down 10.5 s and Start 5.8 s in scenario 2, with
Start refused for the running guest with the same reason as before; an 800 by 600 frame of the
guest and a 640 by 384 frame of the host's text console, and the typed command painting 0.49 of
the canvas red; the screenshot an 800 by 600 PNG with mode 600; the snapshot taken in 1.8 s,
reverted to in 15.8 s and deleted in 0.3 s; 15 charts of the guest with samples 0.1 s after the tab
opened, the first with 120 valid samples; the migration 3.4 s there and 3.2 s back; the reconnection
and the connection of the second start 0.2 s each with no password in the state directory; the
closed client's shutdown reconciled as one record, "Shut down", "Succeeded"; the two batches 11.5 s
and 7.2 s, each one "Succeeded" entry on the Events tab. Three frames were looked at
(`07-in-maintenance.png`, `09-console-command-echo.png` and `15-events-history.png` of their
scenarios).

What the repeat is and is not: the same as the first run. It shows the scenarios above for the
tree after the cutover; it was headless, with no window, no compositor and no seat, and it adds no
path that the first run left out (see "What was not run").

The lab afterwards, read with `lab_state`: both servers enabled; `XOA` running on h2;
`lab-tools-1794263807` and `lab-guest-1794316526` running on h1; `lab-desktop-1794365323` halted;
no snapshot in the pool.
