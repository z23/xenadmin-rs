# Console key codes on GPUI

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
