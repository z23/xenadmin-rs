# GPUI Kit as a front-end candidate

Gate G0 has not passed. This record states the evaluation and the alternatives.
It does not switch the released front end. The Rune binary in `xenadmin-desktop`
remains the artifact that `scripts/package.sh` ships.

## Context

The desktop crate is one Rune application: 119 source files, a single `State`,
and scene checks that render through `xenadmin-rs --snap`. The migration plan
in `docs/GPUI-MIGRATION.md` proposes rebuilding that interface on GPUI Kit
0.9.1, with a toolkit-free model in a later phase. Phase 0 had to show that
the pinned Kit can carry the console, the headless checks, the core bridge,
the shell, and packaging before any view is ported.

The plan was checked against `origin/main` (`bc2e7e4`). Desktop layout matches
the plan's baseline at `d6294bd`. Two later commits touch only the core's
migration receiver URL. The plan's accent colour does not match `theme.rs`.

## What was tried

`gpui-kit` is pinned at exactly `0.9.1`. A spike crate, `xenadmin-gpui`,
builds beside the Rune crate. Reports `docs/reports/2026-11-14-gpui-spike-s0.md`
through `s5.md` say what ran on Ubuntu 24.04.5 and what did not.

The console spike is the one that blocked the cloud run. GPUI's Wayland
backend does not forward evdev codes or left and right modifiers. A second
`wl_keyboard` was written there and was not attached, because that machine
had no compositor. On the later Hyprland seat run the same listener did
deliver those codes as `ConsoleInput::PhysicalKey`. That does not finish S1
or G0: frames, pointer, clipboard, a guest, and S2 through S5 are still
open. Route B was not vendored. Route C, keysyms only, was not taken.

Headless element trees of a two-element scene matched. PNG capture is
unavailable: `gpui-pre-linux` 0.3.12 does not export `current_headless_renderer`.
The scale spike kept a VM id selected across 5,450 headless frames in 60
seconds. Shell tests passed in the Kit's test platform. Packaging and a native
session were not run.

## Alternatives

- Stay on Rune. The released client already builds, renders scenes, and is
  what the lab reports describe. The information-architecture problems in the
  plan remain.
- egui. Not spiked. The plan names its test harness as the fallback if G0
  fails. That choice is not made here.
- GTK 4 or Qt. Not spiked. Either would be a new toolkit and a new set of
  platform checks, and neither was needed to see that G0 failed.
- GPUI Kit 0.9.1. Spiked. The pin builds. Route A delivered console key
  codes on a later Hyprland seat. The named headless renderer and the rest
  of the gate were not met.

## Consequence

Phase 1 does not start from this branch. The spike crate, the pin, and these
reports stay so the next attempt does not repeat the source reading. Views
are not ported. `xenadmin-core`, `xenadmin-xapi`, and `xenadmin-process` are
unchanged.

This note is an engineering record, not legal advice. The Kit and `gpui-pre`
are Apache-2.0. That does not change the licence of the existing crates.
