# GPUI spike S0: build and policy

Result: **not passed**. The spike crate builds on this Ubuntu 24.04 machine. Arch was not available, and the pull request's CI run is the remaining check of the same Ubuntu job. `gpui-fast` is not enabled.

## What was reviewed

The plan was read against `origin/main` at `bc2e7e4`. The plan's desktop facts were taken at `d6294bd`. The only commits since then are a core check that a migration receiver URL stays inside the pool. `crates/xenadmin-desktop/src` is still 119 files. The accent in `theme.rs` is OKLCH lightness about 0.52–0.55, chroma 0.17, hue 256, not `oklch(0.73, 0.15, 55)` as the plan states.

## What ran

- Host: Ubuntu 24.04.5, kernel 6.12.113+, `rustc 1.99.1 (58cf8f9c3 2026-10-13)`. No Arch machine.
- `gpui-kit` is pinned `=0.9.1` in the workspace. `cargo tree -p xenadmin-gpui --locked` shows `gpui-kit v0.9.1` and `gpui-pre v0.3.12`. `gpui-fast` is in `Cargo.lock` because the Kit declares it optional. It is not in the normal dependency tree of `xenadmin-gpui`.
- `Cargo.lock` gained 344 package names and lost none. The diff is about 5,700 lines.
- `cargo clippy -p xenadmin-gpui --all-targets --features scenes --locked -- -D warnings` completed with no warnings. The workspace `unsafe_code = "forbid"` cannot be overridden while `lints.workspace = true`, so this crate lists its own lints: `unsafe_code = "deny"` and `clippy::dbg_macro = "deny"`. The only `unsafe` is the foreign Wayland display wrapper. `xenadmin-core` is unchanged and still forbids unsafe.
- Cold compile of the new graph, from the first `cargo test -p xenadmin-gpui` log: about 2 minutes 30 seconds until `gpui-kit` and `xenadmin-gpui` had been compiled (that invocation then stopped on two type errors in the spike). Dependencies were not already in `target/`.
- After `cargo clean -p xenadmin-gpui`, with those dependencies still built: 1.3 seconds.
- One-line rebuild of `xenadmin-gpui` after that: 0.53 seconds.
- Debug `libxenadmin_gpui.rlib`: 1.1 MiB. That is the spike library, not a GPUI application binary.
- `cargo build -p xenadmin-gpui --example s0_hello --release --locked` took 5 minutes 21 seconds and produced `target/release/examples/s0_hello` at 52 MiB. That compile built the release graph, including crates the Rune binary already uses. The example was not opened. It is not `xenadmin-rs`.

## Packages

The build succeeded with the CI set plus the development packages this machine needed for X11 and xcb:

`build-essential pkg-config libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libfontconfig1-dev libxcb1-dev libxcb-xfixes0-dev libxcb-shape0-dev libxcb-xkb-dev libvulkan1 mesa-vulkan-drivers`

`.github/workflows/ci.yml` installs that set. Arch package names were not installed or tested.

## What this does not establish

- A clean install or build on Arch.
- That the existing 60-minute GitHub job still finishes once it compiles this graph. Local compile of the new graph was a few minutes; the shared runner is slower, and the Rune scene renders are unchanged.
- Wayland, clipboard, keyboard, or Orca behavior. The hello example was not opened. Headless checks are in the other spike reports.
