# GPUI spike S1: console input

Result: **requirements 3 to 5 passed on a real Hyprland seat. Eight windows mapped. Gate G0 did not pass.**

Route A, a second `wl_keyboard` on the GPUI window's display, delivered evdev codes and distinct left and right modifiers into `ConsoleInput::PhysicalKey`. Those values are the XT make codes and X keysyms the core sends as a QEMU extended key event. Route B was not vendored. Route C was not adopted. No guest was used. Requirements 1, 8, 9, and 11 were attempted on this seat and are not shown. Requirement 10 mapped eight windows; fullscreen did not hold.

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

## Seat surface run

`examples/s1_surface` is the probe for requirements 1, 8, 9, 10, and 11. It ran on the same Hyprland 0.56.2 seat. No guest was opened. Route B was not vendored. Route C was not adopted.

The window mapped on the focused workspace, workspace 37 on DP-2, floating, at 480 by 320. `hl.dsp.focus` by address and by pid returned ok, and the active window stayed the previously focused terminal. `bring_to_top` returned ok and did not change that.

Frames. Fit, a replaced solid frame, and actual size are painted in the probe. Pixels were not captured. `grim` binds `ext_image_copy_capture`, receives buffer size and damage, and never receives ready; a full-output capture and a toplevel capture both hung. `grim -o` and `grim -g` are mutually exclusive in this build. A direct `zwlr_screencopy` client got an 80 by 80 shm buffer for a 64 by 64 region and then no ready event within 3 seconds. That check stops there.

Pointer. A `zwlr_virtual_pointer_v1` moved the compositor cursor. `hyprctl cursorpos` matched an outside point and then the window center, 3142,2332, including the logical layout box of 8192 by 3168 that Hyprland uses for absolute motion. GPUI `mouse_position` stayed 0,0. No move, button, or wheel event was delivered. Shift+wheel was not observed. The same limit covers the hidden cursor: `CursorStyle` has no hidden variant, capture never completed, and the window never took pointer focus, so hide-on-typing was not observed. Requirement 9 stays the visible-cursor allowance in the plan, and it was not shown either way.

Fullscreen. One run started at hyprctl fullscreen 0. After `Window::toggle_fullscreen`, hyprctl reported 2. GPUI `is_fullscreen` stayed false, so a second call does not send unset, and the exit was not observed. A repeat of the same sequence stayed at 0 for the whole 2 second poll. Enter was seen once and did not repeat. Exit was not seen.

Clipboard. The probe built `TypedText::review` for a 19-character marker, zero line breaks, and `ConsoleInput::TypeText` of that marker. `read_from_clipboard` came back empty. `wl-paste -t text/plain` exited 1. GPUI's Wayland clipboard sets the selection only while one of its windows has keyboard or pointer focus and a press serial; this window never became the active window. The previous clipboard, an `image/png` offer, was restored.

Windows. Hyprland listed eight clients for this process, `xenadmin-s1-surface` and `xenadmin-s1-w2` through `xenadmin-s1-w8`, each mapped. That part of requirement 10 held on two runs.

Memory. A 1920 by 1080 frame every 33 milliseconds for 60 seconds produced VmRSS samples 183032, 207012, and 207012 KB. RSS rose about 24 MB and then stayed flat. That is not zero growth. GPU memory is not in RSS.

## Requirements

| Requirement | Result |
| --- | --- |
| 1. Fit and actual-size frames, no stale frame | Not shown. The probe paints both sizes and drops the replaced image. This seat's screencopy never completes, so the pixels were not read back. |
| 2. Key down and key up, including repeat | Shown for the injected set on the Hyprland seat. Not the whole US layout. |
| 3. Physical key identity as XT codes | Shown. The table above is what the seat delivered, as `ConsoleInput::PhysicalKey`. No guest received a code. |
| 4. Separate left and right modifier press and release | Shown for Shift, Ctrl, Alt, and Super. GPUI's keystrokes did not carry those keys. |
| 5. CapsLock, NumLock, ScrollLock, PrintScreen, Pause, ContextMenu | Shown. Menu arrived as evdev 139. |
| 6. Release held keys on focus loss, deactivation, and close | Leave released a held B. Deactivation and close were not separate cases. |
| 7. No application binding consumes a key while the console is focused | The probe calls `stop_propagation` from `intercept_keystrokes`. It has no menu bar, so a menu accelerator was not present to steal a key. |
| 8. Pointer, buttons, wheel | Not shown. The compositor cursor moved onto the window. GPUI received no motion, button, or wheel event, and the window never became active. |
| 9. Hidden local cursor and drawn guest cursor | Not shown. `CursorStyle` has no hidden variant. Capture and pointer focus were both unavailable, so a visible local cursor is the recorded allowance and was not itself observed. |
| 10. Fullscreen and up to eight windows | Eight mapped windows, twice. Fullscreen entered once, hyprctl 0 to 2, and a repeat stayed at 0. Exit was not observed because `is_fullscreen` stayed false. |
| 11. Clipboard and typing the clipboard | The `TypeText` value was built. The seat clipboard did not take the text. GPUI writes the selection only for a focused window with a press serial. |

## What this does not establish

The seat run is one compositor, Hyprland 0.56.2, on one US layout. A German or French layout was not applied to the operator's session. Evdev codes are what `wl_keyboard` delivers either way; that was not shown by changing the layout. No guest captured the codes, and no production host was contacted. Orca was not run. S2 still cannot write a PNG: `gpui-pre-linux` 0.3.12 does not export `current_headless_renderer`. S3's Wayland scroll, S4's native shell, and S5's clean install on both distributions were not run on this seat. Gate G0 does not pass. Route A is the route for requirements 3 to 5. It is not yet a ported console.
