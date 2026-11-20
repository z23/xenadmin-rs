# Console key codes on GPUI

Status on 2026-11-19, after the cutover of the code and the runs of that day: the decision stands as
written below. Route A is in the client (`crates/xenadmin-desktop/src/wayland_keys.rs` and
`console/keys.rs`). It ran with a console for the first time on 2026-11-19, in a virtual machine
with injected input and a software renderer, on sway 1.12
([session report](../reports/2026-11-19-gpui-vm-session.md), section D); what that run says about
each accepted limit is recorded under "Accepted limits" below. It has not run with a person's
keyboard, and on no compositor other than Hyprland 0.56.2 (the spike, without a console) and sway
1.12. The listener itself has no test; its key repeat, which the session found missing (a held key
arrived in the guest as one press), has unit tests in `console/keys.rs` and was confirmed in the
same machine with a build of the fix and, on 2026-11-20, with the AppImage CI built of the merged
tree (commit `3493640`): the guest recorded the first repeat 0.60 s after the press and then sway's
25 a second, ending at the release, when the focus left the window and when the console was closed;
a held Shift did not repeat ([report](../reports/2026-11-20-gpui-merged-tree.md)). That run also
found that while a key is held and repeats, a pointer press on a button of the window does nothing:
the toolkit repeats the key as key events of its own and counts no element as hovered after one. In the headless lab runs of 2026-11-19 keys reached a guest
through the Kit's test key events, which is the path without the listener
([report](../reports/2026-11-19-gpui-live-headless.md)). Where the text below speaks of the Rune
console or the Rune binary in the present tense, it is of 2026-11-14: the Rune crate has since been
removed, and the crate `xenadmin-gpui` it names is `crates/xenadmin-desktop`.

Route A is the route for physical key codes. Gate G0 did not pass. Decision
D2 still stands: keysym-only input is not adopted.

## What GPUI 0.3.12 gives

`Keystroke` carries modifiers, a key name, and an optional typed character.
On Wayland, `wl_keyboard::Event::Key` includes the evdev code. `gpui-pre-linux`
0.3.12 uses that code only to ask XKB for a keysym. Modifier keysyms are not
turned into key events. Left and right are not distinguished after that
translation. `CursorStyle` has no hidden value. The details are in
`docs/reports/2026-11-14-gpui-spike-s1.md`.

## What was shown

On the operator's Hyprland 0.56.2 seat, a second `wl_keyboard` bound to the
GPUI window's `wl_display` received the evdev code for every injected key,
including Shift, Ctrl, Alt, and Super on both sides, and the lock, print,
pause, and menu keys. GPUI's own keystrokes did not include those modifiers.

The spike maps each code to `ConsoleInput::PhysicalKey`. `keycode` is the XT
make code the Rune console already sends. `down` stays a separate flag. That
is the value `xenadmin-core` turns into a QEMU extended key event once the
server has negotiated the extension. The mapping was checked against the
seat's events, not only against the unit vectors.

## Routes

Route A is the one that produced those events. The listener remains in
`xenadmin-gpui` for this spike. `xenadmin-keys` is still not a crate: the
plan adds it when a console view takes the route, and no view was ported.
There is no vendored patch to re-apply on a Kit upgrade. The cost is the
Wayland seat listener and the evdev-to-XT table, retested when the
compositor or GPUI's display handle changes. Only Hyprland 0.56.2 was run.

Route B, a vendored patch of `gpui-pre` and `gpui-pre-linux`, was not
applied. Route A met requirements 3 to 5 without it.

Route C would send keysyms from `Keystroke` and would drop extended key
events and left/right modifiers. It is not adopted.

## Accepted limits

The remaining seat failures are accepted limits. They do not stop the
migration, and they do not change route A. Route B stays unvendored. Route
C stays unused.

Still not shown on a real seat, and not a reason to wait:

- GPUI did not deliver pointer motion or buttons on the Hyprland probe.
- `CursorStyle` has no hidden value, so the local cursor stays visible.
- The guest cursor is not drawn. The Rune console still draws it, because
  that front end can hide the local cursor. The GPUI console will not draw
  a second cursor.
- Screencopy never completed, so fit and actual-size pixels were not read
  back.
- Clipboard text was not offered, because the probe window never became the
  active window. Clipboard without focus stays unsupported.
- Fullscreen was reported once and did not repeat. Exit was not observed.
- No guest recorded a key, and a second host layout was not applied.
  Ctrl+Alt+F2 was not sent.

Eight windows did map. The evidence is in
`docs/reports/2026-11-14-gpui-spike-s1.md`. The Rune binary remains the
released artifact until cutover.

Recorded on 2026-11-19, from the driven session in a virtual machine
([report](../reports/2026-11-19-gpui-vm-session.md), "What the session says about the gates
and about decision 0006"). It was sway 1.12 on the display of a QEMU machine, software
rendering, every key and pointer event injected at the machine's devices by a script, with the
AppImage CI built at commit `e808a2f`. None of it is a real seat with a person, and none of it
was on Hyprland, so each limit above stays open for that; what the session adds:

- Pointer: on sway the whole session was driven by pointer presses, and a lab guest recorded
  the button presses made on its console. Pointer motion inside a guest was not examined.
- The local cursor and the guest's cursor were not examined.
- Fit and actual size were looked at in pictures of the machine's display; pixels were not
  compared.
- Clipboard: with the window focused, text was copied out and pasted in. Without focus it was
  not tried.
- Fullscreen was entered and left twice, on the main window and on a console window, as the
  compositor reported it.
- A lab guest recorded every key of a test line from its own input device, and recorded the
  same key codes when the compositor's layout was German. Ctrl+Alt+F1 was sent through the
  console's button and arrived; no such chord was pressed at the keyboard.
