# GPUI spike S1: console input

Result: **not passed**. Route A is implemented and not selected. Route B was not applied. Route C was not adopted. Requirements 3 to 5 were not shown on a guest.

## What the pinned source does

Read from the crates locked by `gpui-kit` 0.9.1:

- `gpui-pre` 0.3.12 `Keystroke` has `modifiers`, `key`, and `key_char`. It has no evdev code.
- `gpui-pre-linux` 0.3.12 `wl_keyboard::Event::Key` converts the evdev code to an XKB keycode and then a keystroke. The match arms for press and release both require `!keysym.is_modifier_key()`, so modifier keys are not delivered as key events. The raw code is not forwarded.
- `CursorStyle` has no hidden variant.
- `Window` implements `HasDisplayHandle`. The test platform's display handle returns `HandleError::NotSupported`.

That matches the plan's reading of GPUI 0.3.12.

## What was built

`crates/xenadmin-gpui/src/wayland_keys.rs` binds a second `wl_keyboard`.

- `from_foreign_display` wraps a `wl_display` the caller already owns. It does not read that socket. `dispatch_pending` drains the private queue. The `unsafe` block is documented on that function. `xenadmin-keys` was not created: the plan adds that crate only if route A is selected.
- `connect_own` opens `WAYLAND_DISPLAY` as its own client. That is not route A. It is the probe in `examples/s1_probe.rs`.
- `scancode.rs` re-keys the Rune console XT table by evdev code. The vectors from `console_view.rs` match: KEY_A is `0x1e`, Right is `0xcd`, Right Ctrl is `0x9d`, SysRq is `0x54`, Pause is `0xc6`. Left and right modifiers stay distinct. The table stays in this spike. `xenadmin-ui` is Phase 1 and was not created.

Unit tests cover the table, a missing `WAYLAND_DISPLAY`, and a null foreign pointer. The null pointer is rejected before any Wayland call.

## What ran

Ubuntu 24.04.5, kernel 6.12.113+. `WAYLAND_DISPLAY` was unset. `examples/s1_probe` exited 2:

```text
wayland_display=
own_connection=error
WAYLAND_DISPLAY connection failed: Could not find wayland compositor
```

No compositor was started. No lab guest was used. No physical key injection was run. Route B (a vendored patch of `gpui-pre` and `gpui-pre-linux`) was not applied: there is no session here in which a patched callback could be shown to deliver evdev codes, and an untested vendor tree would not meet requirements 3 to 5.

## Requirements

| Requirement | Result |
| --- | --- |
| 1. Fit and actual-size frames, no stale frame | Not run. No console view was ported. |
| 2. Key down and key up, including repeat | Not run. |
| 3. Physical key identity as XT codes | Not run. The table is unit-tested. No guest received a code. |
| 4. Separate left and right modifier press and release | Not run. GPUI's own path does not emit them. The second keyboard was not attached to a live display. |
| 5. CapsLock, NumLock, ScrollLock, PrintScreen, Pause, ContextMenu | Not run. |
| 6. Release held keys on focus loss, deactivation, and close | Not run. |
| 7. No application binding consumes a key while the console is focused | Not run. |
| 8. Pointer, buttons, wheel | Not run. |
| 9. Hidden local cursor and drawn guest cursor | Not run. `CursorStyle` has no hidden variant. |
| 10. Fullscreen and up to eight windows | Not run. |
| 11. Clipboard and typing the clipboard | Not run. |

## What this does not establish

Headless tests do not establish Wayland input, physical keys, the system clipboard, or Orca. This spike did not use a lab or a production host. Requirements 1 to 8, 10, and 11 do not hold on evidence. The migration stops at gate G0.
