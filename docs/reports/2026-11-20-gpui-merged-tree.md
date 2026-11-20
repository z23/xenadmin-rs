# GPUI front end: the package of the merged tree, run where the earlier one ran

2026-11-20. The merged tree (`gpui-migration` at commit `3493640`) is pushed and CI has built
it. This report says what that package did in the virtual machine of the
[driven session of 2026-11-19](2026-11-19-gpui-vm-session.md), step by step, with the same
methods, so that the two compare. It looks at what changed in the client since the package of
`e808a2f`: the second start, key repeat on the Wayland key path, the start without a display
or a driver, "Reduce animation", a tab body that ends in a short table, and the close. It
then records two things of the same day that were not made in the machine: the live headless
scenarios run again on the merged tree, and the upgrade test.

The words are those of the session report: **injected** (pressed at the machine's keyboard or
pointer device through QEMU's monitor), **frame** (a picture of the machine's display, looked
at unless a row says it was compared by a program), **compositor** (sway's own report),
**AT-SPI** (read from the accessibility bus, to find a control or read a state, never to
act), **guest effect** (what a lab guest itself recorded and reported).

Everything in the first part is "verified in a virtual machine with injected input": a QEMU
machine, sway 1.12, a software renderer, a script at the keys. No person looked at a screen.
The last section says what that leaves open.

## The package

| | |
| --- | --- |
| Built by | workflow `ci.yml`, run 44658600102, job `check`, for commit `3493640aa790fc821a82c8db69989178e25d3dba` (the head of `gpui-migration` on GitHub); conclusion success, every step successful |
| AppImage | `xenadmin-rs-0.20.0-linux-x86_64.AppImage`, 47,327,736 bytes, SHA-256 `2933ec2d7553c5212211a853d477cb954a7388ebff4725c646cf83a01f6f77e4` |
| Archive | `xenadmin-rs-0.20.0-linux-x86_64.tar.gz`, 46,554,594 bytes, SHA-256 `86e0e77d07c1851f3876f0cb80f1cc0e3dcc08b82e9094a9026ec699b48317a2` |
| Checksums | both files match their `.sha256` files on the workstation and again in the machine |
| The run of the packaged client on CI | the step "Build portable archive and AppImage" ran `./scripts/package.sh --appimage` and succeeded. That script runs `packaging/smoke.py` on the staged client and, after the image is written, on the AppImage (extracted, so that no FUSE is needed): the client is asked for `--scene demo --snap FILE` with no display variable and must refuse in one line with status 1. The script prints nothing when the client answers as it should, so the step's log has no line of it; that it ran is read from the script at this commit, which ends the packaging when it fails, and from the step's success. The step "Check, test, and render" ran the script's own tests, whose lines are in the log. What the smoke test shows is that the client loads and reads its command line on Ubuntu 24.04; it opens no window. |

`--version` prints `xenadmin-rs 0.20.0`, as the package of `e808a2f` did: the version does not
tell the two apart, the checksums do. The archive was checked against its checksum and not
run. Unless a row says otherwise, every observation below was made with this AppImage.

## The machine

The machine of the session report, started from its kept disk and not updated: QEMU with KVM,
`-display none`, `virtio-vga` 1440x900, a USB keyboard and an absolute pointer, Arch Linux
(kernel 7.3.4-arch1-2), sway 1.12-4 with its software renderer on the machine's display,
Mesa 26.2.7 (llvmpipe), no Xwayland, no keyring, no sound. One package was added in the
guest (`bc`, for the timing arithmetic). The accessibility bus was off for steps 1 to 3 and
on from step 4, because the driver finds controls through it.

Frames and logs are outside the repository, under the lab's evidence directory
(`evidence/gpui-vm2/`): `frames/001` to `044`, `anim/` (the bursts of step 6), `logs/`.

## Steps

| Step | Result | Observed by |
| --- | --- | --- |
| 1. The package in the machine | checksums verified; `--version` answers in 0.02 s with status 0 and no display variable set; `--scene demo --snap FILE` is refused in one line, status 1 | the shell's output (`logs/01-package.log`) |
| 2. Start with neither `WAYLAND_DISPLAY` nor `DISPLAY` | one line, `xenadmin-rs: no display: neither WAYLAND_DISPLAY nor DISPLAY is set. Start the client from a Wayland or X11 session.`, status 1, the process ended after 0.02 s; the same with `--demo` | the shell's output and status, timed (`logs/02-start-failures.log`) |
| 2. Start with a `WAYLAND_DISPLAY` that names no socket | one line, `xenadmin-rs: the display could not be reached: failed to connect to Wayland compositor at /run/user/1000/wayland-nope: No such file or directory (os error 2)`, status 1, ended after 0.2 s (0.06 s with `--demo`). With a `DISPLAY` that names no X server: one line of the same kind, status 1, 0.06 s. The failed start leaves `desktop-activate.sock` and `desktop.lock` in the state directory; a later start on that directory was not misled by them (it gave the no-display line, status 1). | the same |
| 2. Start with no Vulkan driver usable | Two cases. With the driver's description hidden (`/usr/share/vulkan/icd.d` covered in a private mount namespace, or `VK_DRIVER_FILES` naming a missing file) and Mesa's EGL and GL libraries present: **the window opens** and is drawn, by another path (frame 001, looked at: the demo's first view); the log has Mesa's warnings only. With those libraries hidden as well (the clean-install method), or the Vulkan loader and those libraries hidden: **two lines**, `XenAdmin RS: The main window did not open: Failed to create surface: Failed to create surface for any enabled backend: {}` and `No Vulkan driver could be used. Install the Vulkan driver of the graphics card, or Mesa's software driver.`, status 1, no window, the process ended after 0.26 s. With the package of `e808a2f` that process stayed until it was killed (clean-install report). | the shell's output and status, timed; compositor (window count); frame 001 (`logs/02-start-no-vulkan.log`) |
| 2. Start with no font | not done | |
| 3. Start on an empty state directory | the main window is drawn: menu bar, toolbar, the empty tree, Home (frame 002, looked at). 0.27 s from the start of the process to sway's new-window event: one measurement, software rendering, after the image had been run several times (the first report's 1.7 s was a first run). The client logs Mesa's five lines about the missing GPU and nothing of its own. | frame; compositor (window events, timed); log (`logs/03-first-start.log`) |
| 3. A second start, nothing pressed in the first client since its start, another window has the keyboard | the second process prints nothing and exits with status 0 after 0.03 s; sway marks the first window urgent | compositor (`get_tree`, window event `urgent`) (`logs/03-second-start.log`) |
| 3. A second start after a pointer press in the client, the pointer then over another window | status 0; sway marks the first window **urgent** (one of one, and again in the next row, which was made in the same situation). With the package of `e808a2f` this was the case in which nothing happened, eleven of eleven. | the same; frame 003 |
| 3. A second start from a shell without `WAYLAND_DISPLAY` and `DISPLAY` | forwarded: status 0, no output, 0.03 s, and the first window is marked urgent. It is not refused for want of a display. | the same |
| 3. A second start while the client has the keyboard | status 0; nothing changes (the window is focused already) | the same |
| 4. Connect from the saved connection | on the state directory the first session left (one connection, two pins, no password): the connection is listed as "Disconnected" (frame 004). "Connect…" in the row's context menu; a press on Password: focus ring and AT-SPI state `focused`, role `password text` (frame 006); the password typed from its file by injected keys, drawn as dots (frame 007, looked at); Enter: connected with no certificate question, and the tree shows the pool, both servers, the four guests and the repositories (frame 008, looked at) | injected; frames; AT-SPI |
| 4. Console of `lab-guest-…`: the keyboard by a pointer press | the console is drawn (frame 009); a press on the picture, `root`, Enter: logged in (frame 010). The guest's network was down after its last boot and was brought up by a typed command, after which the guest reached the listener. | injected; frame; guest effect |
| 4a. A line with capitals, digits and every US punctuation sign | the line of the first session, typed again: the guest's file holds exactly its bytes; its key events show Left Shift down and up as a key of its own around each shifted character | injected; guest effect (`logs/04-console-keys-decoded.txt`) |
| 4b. A key held for two seconds, focus kept | `b` down, first repeat 0.60 s later, then one every 30 to 60 ms (40 ms mostly: sway announces 25 a second after 600 ms), release 2.05 s after the press; the guest's file holds 37 `b`, and nothing follows the release | injected; guest effect (timed) |
| 4c. A key held while the focus leaves the window | `a` held; about 1.5 s later another window (a terminal started through sway) took the focus; the key stayed down at the machine for about three more seconds. The guest recorded the repeats and then the release 1.55 s after the press, and nothing more until the next key 6.5 s later; its file holds 25 `a`. | injected; compositor (the other window focused); guest effect (timed) |
| 4d. A key held while the console is closed with the pointer | **Not as expected with an ordinary press: finding 1.** `c` held, the pointer on "Close Console" in the tab's row, a press of 80 ms 1.3 s after the key went down: the console did **not** close, and `c` repeated for the whole 4.1 s the key was down (two of two). With a press whose release follows at once (1 ms apart, two monitor commands back to back), in the same situation: the console closed, and the guest recorded the release of `c` 1.47 s after its press and nothing more, while the key stayed down at the machine for 2.5 more seconds; its file holds 23 `c`. With Left Shift held instead of `c` (a key that does not repeat) the ordinary press closed the console, and the guest recorded Shift's release at the close, 1.60 s after its press. | injected; AT-SPI (whether the tab still has "Close Console"); guest effect (timed) |
| 4e. Shift held | Left Shift down and, 1.54 s later, up; no repeat between | injected; guest effect (timed) |
| 4. Afterwards | `exit` typed; the guest's console shows its login prompt (frames 020 and 044, looked at) | injected; frame |
| 5. The Snapshots tab at 800 by 600 | floated and resized through sway to a content of exactly 800 by 600 (sway's `window_rect`). The row of local actions (Take a Snapshot, Revert, Edit properties, Create VM, Create template, Export selection…, Delete) is on **one** line at this width in this machine (frame 021, looked at); the toolbar is on two lines and the sidebar is folded behind its toggle. The tab's body shows the two paragraphs, "Show list" and the table; what follows the table (the selected snapshot's information, its disks, the saved preview, the results of operations) is below the window's edge and is reached by scrolling the body with the wheel, the row of local actions staying in place (frame 029). | compositor; frames |
| 5. Take a snapshot, press its row | "Take a Snapshot" in the row, a name typed, "Create snapshot": the row `v5-snap · current base` appears and the result is shown (frames 022, 023). Frames before and after a press on the row (024, 025), compared by a program: no pixel of the band that holds the table's header differs; the row is highlighted, the buttons of the row of local actions are enabled, the information appears below. The table did not move. | injected; frames (compared) |
| 5. The same where the row of local actions wraps | tiled by sway beside a terminal, the client's content is 716 wide, which is below the 800 it asks for. There the row is on **two** lines (frame 026, looked at). The row unselected by Ctrl and a press, then selected again (frames 027, 028): no pixel of the table's header band differs. | compositor; injected; frames (compared) |
| 5. Delete the snapshot | "Delete" in the row; the review names the snapshot and its two disks; "Also delete 2 exclusively owned snapshot disk(s)" ticked; "Delete snapshot": "No snapshots for this VM." and the result "Snapshot and reviewed disks deleted" (frames 030 to 032, 036). The lab's own reading afterwards: 0 snapshots. | injected; frames; the `lab_state` test |
| 6. "Reduce animation" | Tools > Options: the check box can be ticked; Apply writes `"reduced_motion": true` to `preferences.json` at once; after the window was closed and the client started again the box is ticked (frame 038, looked at). **An effect on drawing was observed**, on the one animation looked for: ticking another check box of the same dialog while the display was captured every 30 ms. With the option off the tick went on changing for about 200 ms after it first appeared (six successive captures differ, four of four tries); with the option on it was complete in the capture in which it first appeared and no later capture differed (three of three tries, one of them after the restart). Switching the option off again and applying brought the first behaviour back without a restart. No other animation was looked for; the dialog itself appears in one step either way. | injected; AT-SPI; the file; captures compared by a program (`anim/`) |
| 7. Close through the compositor | `swaymsg kill`: the process ended 0.5 s after the request; its two connections to the lab are gone; `preferences.json` is rewritten at that moment; the activation socket is removed (`logs/07-close.log`) | compositor; `ss`; the files |
| 7. No password on disk | the script that reads the password from its file searched every file of the state directory for it (raw, base64, hex, percent-encoded, UTF-16): **no**; again at the end: no | script |
| 8. The lab and the machine | the lab at its baseline, read by the `lab_state` test: both servers enabled, `XOA` running on h2, the two Alpine guests running on h1, the desktop guest halted, 0 snapshots. The machine shut down, its disk kept. | the `lab_state` test |

## Findings

### 1. While a key is held and repeats, a pointer press on a button does nothing

What was done: in a guest's console that has the keyboard, a letter key is held; the pointer
presses "Close Console" in the tab's row (down, 80 ms, up).

What was seen: nothing. The console stays open and the key goes on repeating in the guest
until it is released at the keyboard (two of two; `logs/04-console-keys-decoded.txt`, d1 and
d2). The same press with no key held closes the console, also after the pointer has rested on
the button; the same press with Shift held closes it; a press whose release follows within a
millisecond closes it with the letter key held.

Cause, read in the toolkit's source and consistent with the four observations
(`gpui-pre` 0.3.12, `src/window.rs`): the window remembers whether the last input was a key or
the pointer; a key-down event sets "keyboard", a pointer press or move sets "pointer", and a
pointer release sets nothing. While the last input is "keyboard", no element counts as
hovered, and a click is delivered only if the element is hovered at the release. The toolkit
repeats the held key as key-down events of its own, 25 a second here, so one of them falls
between the press and the release of any click a hand makes. The client's own key path
(decision 0006, route A) is not involved: it takes the keys from a second listener, but the
toolkit still sees them.

This is the toolkit's behaviour in any window, not a change of this package; in this client it
means that while a key is held in a console no button, tab or menu answers the pointer. Not
fixed: there is no small change in the client that makes the toolkit forget the key, and the
toolkit is pinned. It is listed for the owner.

What the step was for was still shown: when the console closes under a held key, the repeat
ends at the close and nothing more arrives (the immediate press and the Shift case of row 4d).

### 2. An open console can keep the software renderer busy, and its picture can fall behind

What was seen: with the Console tab of the Alpine guest open and the guest idle at a shell
prompt after some output had scrolled its screen, the client used about 170 % of one CPU
(llvmpipe's threads; 167 to 172 % in seven measurements of five seconds); at the guest's
login prompt after a boot, 27 to 31 %; on the General tab 1 to 2 %; with the console closed,
none. In the first of these states, some three and a half minutes into the key tests, the
console's picture was behind the guest: frames 011 to 014 show
the guest's screen as it had been up to about 70 seconds earlier (the guest's reports, which
do not pass through the picture, were on time, and so were the keys). Closing and opening the
console brought a current picture.

The package of `e808a2f` was started in the same machine for comparison: 27 to 30 % at the
login prompt, 28 % at a shell prompt the first time and 96 % the second, and 149 to 166 %
after the same thirty lines of output. So the load is not new with this package, and what
sets it off was not found. Whether the old package's picture also falls behind was not measured.

Not investigated further. It was seen with a software renderer in a virtual machine only;
whether a hardware GPU shows any of it is not known.

### 3. Smaller observations

- A start that fails because the display cannot be reached leaves `desktop-activate.sock` and
  `desktop.lock` in the state directory. The next start was not misled by them.
- Without a Vulkan driver the client says two lines, not one, and where Mesa's EGL and GL
  libraries are installed it opens its window all the same.
- The row of local actions of the Snapshots tab wraps only below the minimum width here; a
  tiling compositor gives a window less than its minimum without asking.

## Live, headless, on the merged tree

Not made in the machine. On 2026-11-20 from 13:08 local time, the ignored tests of
`crates/xenadmin-desktop/tests/lab_live.rs`, which run the real shell and core without a
window, were run on the lab at commit `3493640`: every scenario of the
[first live report](2026-11-19-gpui-live-headless.md), one at a time, each on a fresh state
directory, and of the [second](2026-11-19-gpui-live-paths.md) the member's toolstack restart,
in the stricter form the review of 2026-11-20 gave it (with the member's console open Restart
Toolstack was not offered, its menu item being disabled; it was offered once the console was
closed, and the restart then ran and was followed to its end). The lines are those of the
run's summary file; the evidence directories are under `evidence/gpui/` on the lab's storage.

| Test | Exit | Seconds | Evidence directory |
| --- | --- | --- | --- |
| `lab_state` | 0 | 1 | |
| `lab_g2_add_server_and_follow_the_host` | 0 | 17 | `1795140509-g2-add-server-and-follow` |
| `lab_p3_power_and_events` | 0 | 39 | `1795140527-p3-1-power-and-events` |
| `lab_p3_console` | 0 | 127 | `1795140565-p3-3-console` |
| `lab_p3_console_save_screenshot` | 0 | 18 | `1795140692-p3-3-console-save-screenshot` |
| `lab_p3_snapshots` | 0 | 46 | `1795140710-p3-4-snapshots` |
| `lab_p3_performance` | 0 | 20 | `1795140757-p3-5-performance` |
| `lab_move_xo_appliance` (the core's own scenario: the appliance to the coordinator) | 0 | 50 | |
| `lab_place_tools_guest_on_the_member` | 0 | 23 | `1795140826-place-tools-guest-on-the-member` |
| `lab_p3_host_maintenance` | 0 | 37 | `1795140850-p3-6-host-maintenance` |
| `lab_baseline_tools_guest_on_the_coordinator` | 0 | 24 | `1795140886-baseline-tools-guest-on-the-coordinator` |
| `lab_p3_member_toolstack_restart` | 0 | 48 | `1795140910-p3-6-member-toolstack-restart` |
| `lab_move_xo_appliance` (back to the member) | 0 | 12 | |
| `lab_p3_migration` | 0 | 32 | `1795141156-p3-7-migration` |
| `lab_p3_connection_recovery` | 0 | 27 | `1795141189-p3-8-connection-recovery` |
| `lab_client_closed_in_the_middle_of_an_operation` | 0 | 34 | `1795141215-closed-in-the-middle-of-an-operation` |
| `lab_p3_power_batch` | 0 | 99 | `1795141249-p3-1-power-batch` |
| `lab_state` | 0 | 1 | |

Every line says exit 0 and one test passed, and the file has no line of a failed attempt. The other scenarios of the second report
were not run again: among them the reboot and the shutdown of the member, the restart of the
coordinator's toolstack, the lost member and the certificate that is not the pinned one.

## The upgrade test

Not made in the machine. The ignored test
`a_state_directory_of_the_released_client_loads_and_closes_without_loss`
([report of 2026-11-19](2026-11-19-gpui-upgrade-state.md) for what it does) passed at commit
`3493640` on the two state directories of the old client (that report says who wrote them). It was run for this report in
a worktree at that commit; its lines:

Directory written by 0.19:

```text
upgrade: files 3; saved connections 1; preference keys 14; journal absent
upgrade: saved connections listed with endpoint, user and pins as in the file: yes (1 connection(s), 1 certificate pin(s))
upgrade: preferences read as in the file: yes (13 key(s); the file has inspector_width: yes)
upgrade: connections asked to restore at start: 0; connected: 0
upgrade: the close wrote preferences.json without inspector_width and nothing else: yes (profiles.json and the journal are byte for byte the same)
upgrade: no journal in this directory, nothing to open
upgrade: the original directory is unchanged: yes
```

Directory written by 0.17:

```text
upgrade: files 4; saved connections 1; preference keys 14; journal format 4 with 1 VM operation(s), 0 resource operation(s), 0 batch member(s)
upgrade: saved connections listed with endpoint, user and pins as in the file: yes (1 connection(s), 2 certificate pin(s))
upgrade: preferences read as in the file: yes (13 key(s); the file has inspector_width: yes)
upgrade: connections asked to restore at start: 1; connected: 0
upgrade: the close wrote preferences.json without inspector_width and nothing else: yes (profiles.json and the journal are byte for byte the same)
upgrade: VM operations of the journal listed by the model after a session opened: 1 of 1
upgrade: journal after the core opened it: format 5 (was 4), 1 VM operation(s), 0 resource operation(s), 0 batch member(s); payloads byte for byte the same: yes
upgrade: the original directory is unchanged: yes
```

A third directory at hand, older than 0.17 (its options have no interface size), was given to
the test as well. The test fails on it where it compares the preferences read with the file,
at the key `options`: the file lacks a field that the model holds with its default. The test
was not written for such a directory and the failure was not investigated; the upgrade report
already says that a directory older than 0.17 is not shown.

## What none of this shows

- **No person, no desk.** Every key and pointer event of the first part was injected at the
  machine's devices by a script, and every picture is a capture read afterwards. Nothing here
  says how the client feels to use. Key repeat was measured for a key held by a script; a
  hand, a real keyboard with its own repeat and quirks, an input method and a compose key
  were not there.
- **No hardware GPU.** The client drew with Mesa's software driver and sway with its software
  renderer, on one output of 1440x900. The times, the load of an open console and the
  observation of "Reduce animation" are of that renderer.
- **One compositor.** sway 1.12. Not Hyprland, which is the operator's, not GNOME, not KDE.
  What sway does with a second start (it marks the window urgent and never focuses it) is its
  policy; what another compositor does with the same requests was not seen.
- **X11 not at all**, and no keyring: a saved password and a connection at start were not
  tried.
- **No screen reader.** The accessibility bus was used by the driver only.
- **Not the whole of the first session.** The certificate reviews, the recovery gate with Xen
  Orchestra shut down, a host's console, the clipboard, the file chooser, the separate
  console window and fullscreen were not repeated with this package. The recovery gate has
  been run with the package of `e808a2f` only.
- **The archive was not run**, and neither package was installed on a clean system: the
  clean-install check is of the packages of `e808a2f`, and the corrected package lists have
  still not been installed alone on an untouched image.
- **A start without a font** was not tried. A start without a seat was not tried.
- **Not a release artifact.** The AppImage is a CI build of a branch.
- **The live rerun was headless**, and of the first report's scenarios; the upgrade test is a
  model test on copies.

## The lab afterwards

Returned to its baseline and read back with the `lab_state` test (the last row of the steps).
The description of `lab-guest-…` was not changed. Inside that guest, until its next boot, its
network interface holds the address it was given for the reports; it is logged out. The
snapshot taken in step 5 was deleted with its disks through the client.

No Rust source was changed for this report.
