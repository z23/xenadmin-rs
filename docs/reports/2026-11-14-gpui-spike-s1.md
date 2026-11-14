# GPUI spike S1: console input

Result: **requirements 3 to 5 passed on a real Hyprland seat. The full S1 matrix did not, and gate G0 did not.**

Route A, a second `wl_keyboard` on the GPUI window's display, delivered evdev codes and distinct left and right modifiers into `ConsoleInput::PhysicalKey`. Those values are the XT make codes and X keysyms the core sends as a QEMU extended key event. Route B was not vendored. Route C was not adopted. No guest was used. Requirements 1, 8, 9, 10, and 11 were not run.

## What the pinned source does

Read from the crates locked by `gpui-kit` 0.9.1:

- `gpui-pre` 0.3.12 `Keystroke` has `modifiers`, `key`, and `key_char`. It has no evdev code.
- `gpui-pre-linux` 0.3.12 `wl_keyboard::Event::Key` converts the evdev code to an XKB keycode and then a keystroke. The match arms for press and release both require `!keysym.is_modifier_key()`, so modifier keys are not delivered as key events. The raw code is not forwarded.
- `CursorStyle` has no hidden variant.
- `Window` implements `HasDisplayHandle`. The test platform's display handle returns `HandleError::NotSupported`.

That matches the plan's reading of GPUI 0.3.12.

## What was built

`crates/xenadmin-gpui/src/wayland_keys.rs` binds a second `wl_keyboard`.

- `from_foreign_display` wraps a `wl_display` the caller already owns. `from_gpui_display` takes that pointer from the GPUI window. Neither reads the socket. `dispatch_pending` drains the private queue. The `unsafe` block is documented on that function. `xenadmin-keys` was not created. The listener stays in this spike until a console view takes it.
- `connect_own` opens `WAYLAND_DISPLAY` as its own client. That is not route A. It is the probe in `examples/s1_probe.rs`.
- `scancode.rs` re-keys the Rune console XT table by evdev code. The vectors from `console_view.rs` match: KEY_A is `0x1e`, Right is `0xcd`, Right Ctrl is `0x9d`, SysRq is `0x54`, Pause is `0xc6`. Left and right modifiers stay distinct. The table stays in this spike. `xenadmin-ui` is Phase 1 and was not created.

Unit tests cover the table, a missing `WAYLAND_DISPLAY`, and a null foreign pointer. The null pointer is rejected before any Wayland call.

## Cloud run, no compositor

Ubuntu 24.04.5, kernel 6.12.113+. `WAYLAND_DISPLAY` was unset. `examples/s1_probe` exited 2:

```text
wayland_display=
own_connection=error
WAYLAND_DISPLAY connection failed: Could not find wayland compositor
```

No compositor was started on that run. No lab guest was used. No physical key injection was run. Route B was not applied.

## Seat run

Linux 7.2.15-1-omarchy, Hyprland 0.56.2 (`efb50993780079460b0cbed1363e2166a2de1d9f`), `XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-1`. This is the operator's real seat, not a headless or nested compositor. The keyboard layout stayed `us` with the existing options. Caps Lock stayed off and Num Lock stayed on.

`examples/s1_seat` opened two GPUI windows and bound the second keyboard with `ForeignKeyboard::from_gpui_display` on the first window's `wl_display`. Hyprland's `send_key_state` dispatcher injected `code:` values, which the compositor sends as evdev codes (`xkb keycode - 8`). The probe drained the private queue from a foreground timer. A frame callback stops being scheduled once the window is idle, so the drain is not hooked to that callback.

The second keyboard's enter surface pointer matched the GPUI window handle: window A `0x558925095bb0`, window B `0x55892586d490`. Keys aimed at A were entered on A. Z, aimed at B, was entered on B.

GPUI's own keystrokes, from `intercept_keystrokes`, were `a`, `c`, `scroll_lock`, `print`, `pause`, `xf86menukb`, `f1`, `right`, `tab`, `escape`, `delete`, `b`, and `z`. There was no keystroke for Shift, Ctrl, Alt, or Super. `gpui_modifier_key=false`.

The same injection produced `ConsoleInput::PhysicalKey` on the second keyboard. `keycode` is the XT make code. `down` is separate from that code, which is how `xenadmin-core` builds `X11Event::ExtendedKeyEvent` after the server negotiates the extension.

| Key | Evdev | XT make | X keysym |
| --- | --- | --- | --- |
| A | 30 | `0x1e` | `0x61` |
| Shift left / right | 42 / 54 | `0x2a` / `0x36` | `0xffe1` / `0xffe2` |
| Ctrl left / right | 29 / 97 | `0x1d` / `0x9d` | `0xffe3` / `0xffe4` |
| Alt left / right | 56 / 100 | `0x38` / `0xb8` | `0xffe9` / `0xffea` |
| Super left / right | 125 / 126 | `0xdb` / `0xdc` | `0xffeb` / `0xffec` |
| Caps Lock, Num Lock, Scroll Lock | 58, 69, 70 | `0x3a`, `0x45`, `0x46` | `0xffe5`, `0xff7f`, `0xff14` |
| Print, Pause, Menu | 99, 119, 139 | `0x54`, `0xc6`, `0xdd` | `0xff61`, `0xff13`, `0xff67` |
| F1, Right, Tab, Escape, Delete | 59, 106, 15, 1, 111 | `0x3b`, `0xcd`, `0x0f`, `0x01`, `0xd3` | `0xffbe`, `0xff53`, `0xff09`, `0xff1b`, `0xffff` |

A repeat injection of A produced more than one down. Shift+A stayed evdev 30 / XT `0x1e` while Shift was its own press. Ctrl+C and Ctrl+Alt+Delete arrived as separate key transitions. Ctrl+Alt+F2 was not sent. On Leave, the probe emitted a release for each key it still held; Hyprland's dispatcher also restores the previous focus after every injected key, so a real up followed that synthesized release. A console that keeps focus would not see Leave between a down and its up.

`xenadmin-keys` was not created. The listener stays in the spike crate. `xenadmin-core`, `xenadmin-xapi`, and `xenadmin-process` were not changed.

## Requirements

| Requirement | Result |
| --- | --- |
| 1. Fit and actual-size frames, no stale frame | Not run. No console view was ported. |
| 2. Key down and key up, including repeat | Shown for the injected set on the Hyprland seat. Not the whole US layout. |
| 3. Physical key identity as XT codes | Shown. The table above is what the seat delivered, as `ConsoleInput::PhysicalKey`. No guest received a code. |
| 4. Separate left and right modifier press and release | Shown for Shift, Ctrl, Alt, and Super. GPUI's keystrokes did not carry those keys. |
| 5. CapsLock, NumLock, ScrollLock, PrintScreen, Pause, ContextMenu | Shown. Menu arrived as evdev 139. |
| 6. Release held keys on focus loss, deactivation, and close | Leave released a held B. Deactivation and close were not separate cases. |
| 7. No application binding consumes a key while the console is focused | The probe calls `stop_propagation` from `intercept_keystrokes`. It has no menu bar, so a menu accelerator was not present to steal a key. |
| 8. Pointer, buttons, wheel | Not run. |
| 9. Hidden local cursor and drawn guest cursor | Not run. `CursorStyle` has no hidden variant. |
| 10. Fullscreen and up to eight windows | Not run. Two windows were opened for key routing only. |
| 11. Clipboard and typing the clipboard | Not run. |

## What this does not establish

The seat run is one compositor, Hyprland 0.56.2, on one US layout. A German or French layout was not applied to the operator's session. Evdev codes are what `wl_keyboard` delivers either way; that was not shown by changing the layout. No guest captured the codes, and no production host was contacted. Frames, the pointer, the hidden cursor, fullscreen, the clipboard, and Orca were not run. Headless output still does not establish those. S2 through S5 are unchanged, so gate G0 does not pass. Route A is the route for requirements 3 to 5 on this seat. It is not yet a ported console.
