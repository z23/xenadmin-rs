# GPUI spike S5: packaging and desktop integration

Result: **not passed**. The released archive still packages the Rune binary. No clean install was run.

## What was checked

`scripts/package.sh` still builds the workspace in release and stages executables through `packaging/artifacts.py`. That script copies only `xenadmin-rs` and `host-probe`. The GPUI hello window is an example, not a package binary, so a later package run would not replace the Rune artifact with it. `scripts/package.sh --appimage` was not run. Notices were not regenerated. `gpui-kit-assets` is in the lock and was not added to a package.

No native Wayland session, X11 startup, software-rendering launch of a GPUI window, AT-SPI activation, second-instance activation, client-side decorations, or display scale 1 and 2 was run. Arch was not available. Ubuntu 24.04 was the build machine only.

The Rune binary remains the released artifact. Offering a GPUI binary beside it is decision D6 and is not this phase.

## What this does not establish

A clean install on Arch or Ubuntu, an AppImage that contains GPUI, portal or AT-SPI behavior, or Orca. Headless output does not establish them. No lab or production host was used.
