# GPUI spike S5: packaging and desktop integration

Result: **not passed**. The GitHub job built the existing archive and AppImage, and that archive still packages the Rune binary. No clean install and no native session were run.

## What was checked

`scripts/package.sh` still builds the workspace in release and stages executables through `packaging/artifacts.py`. That script copies only `xenadmin-rs` and `host-probe`. The GPUI hello window is an example, not a package binary, so the package does not replace the Rune artifact with it. This machine did not run `scripts/package.sh`. The GitHub `check` job on `1dae709` did, including `--appimage`, and that job succeeded. See the S0 report for the two run URLs. Notices for the GPUI crates were collected only as part of that CI package. `gpui-kit-assets` is in the lock. No separate notice review was done here.

No native Wayland session, X11 startup, software-rendering launch of a GPUI window, AT-SPI activation, second-instance activation, client-side decorations, or display scale 1 and 2 was run. Arch was not available. Ubuntu 24.04 was the build machine only.

The Rune binary remains the released artifact. Offering a GPUI binary beside it is decision D6 and is not this phase.

## What this does not establish

A clean install on Arch or Ubuntu, an AppImage that contains GPUI, portal or AT-SPI behavior, or Orca. Headless output does not establish them. No lab or production host was used.
