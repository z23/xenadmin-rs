# Desktop options, offline help and compact console navigation

The Options dialog now persists system/light/dark appearance, reduced animation, optional
review for routine single-VM power actions, and fit/actual-size plus automatic-reconnect
defaults for newly opened consoles. Existing consoles retain their own choices. Force
power and configuration workflows keep their required target reviews. Cancel discards
an options draft; Apply preserves unrelated connection/preferences data. Older preference
files receive the existing behavior as defaults.

Help is available without a host connection or internet. Its topic pages cover direct
connections and trust, partial/unknown operations and cancellation, console controls,
snapshot/storage semantics and private development inspection. It links to the repository
only on explicit click. This adds integrated help; localization remains a separate open row.

Navigation now wraps above the central workspace at all widths, avoiding overlap with the
pool title as workflows accumulate. At compact sizes, Details shows the console's pinned
target and Back to console returns to the same stream. Opening an options/help overlay or
switching to Details releases held console input. Opening a new console returns to its view.

## Verification

- All 165 desktop tests passed after the implementation; five focused behavior tests cover
  options apply/cancel, legacy persistence/defaults and mandatory force review, help navigation,
  compact pinned-console return and newly opened console defaults. The last defaults test
  was added after the full run and separately verified.
- Strict workspace/all-target Clippy, formatting and native desktop build passed.
- Native synthetic captures at 1440×900 and 513×652 show Options and Help; additional
  compact console-details and wide navigation captures were inspected. Headings and footer
  controls fit; long help bodies scroll independently.

No live host operation or user inventory was used. These checks do not establish physical
keyboard, clipboard, Wayland/X11, Orca or multimonitor behavior. The parity ledger retains
those qualification gates.
