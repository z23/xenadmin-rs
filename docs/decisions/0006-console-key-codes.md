# Console key codes on GPUI

No route is selected. Gate G0 did not pass, and decision D2 says keysym-only
input is not adopted unless an operator accepts that regression.

## What GPUI 0.3.12 gives

`Keystroke` carries modifiers, a key name, and an optional typed character.
On Wayland, `wl_keyboard::Event::Key` includes the evdev code. `gpui-pre-linux`
0.3.12 uses that code only to ask XKB for a keysym. Modifier keysyms are not
turned into key events. Left and right are not distinguished after that
translation. `CursorStyle` has no hidden value. The details and the file
locations are in `docs/reports/2026-11-14-gpui-spike-s1.md`.

## Routes

Route A, a second `wl_keyboard` on the console window's display, is compiled
in `xenadmin-gpui`. Its XT table matches the Rune console vectors. It was not
run against a compositor, and no guest recorded the codes. The plan allows a
separate `xenadmin-keys` crate only after this route is selected. That crate
was not added.

Route B, a vendored patch of `gpui-pre` and `gpui-pre-linux`, was not applied.
This machine has no Wayland session in which the new callback could be shown
to fire for modifiers and for Print Screen, Pause, and the lock keys. A patch
that cannot be run does not meet requirements 3 to 5.

Route C would send keysyms from `Keystroke` and would drop extended key events
and left/right modifiers. That is a regression of a supported console path.
It is not adopted.

## Maintenance

There is no patch to re-apply. If a later session shows that route A or route
B meets the S1 matrix on a real compositor and a guest, this record has to be
replaced with that route and its maintenance cost. Until then the Rune console
path remains the one the application ships.
