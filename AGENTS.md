# Project instructions

## Product contract

- Build a standalone Linux XCP-ng management client with GPUI Kit and Rust.
- Connect directly to XAPI. Xen Orchestra must never be required to start, authenticate,
  discover a pool, run a command, or open a console.
- Keep the UI responsive through slow hosts, reconnects, and console traffic.
- This repository contains a beta: a supported core and experimental areas, as drawn in
  docs/FEATURES.md. Report implemented, fixture-verified, live-host-verified, and planned
  behavior separately; see docs/reports/ for evidence.

## Implementation boundaries

- Read docs/PLAN.md and docs/ARCHITECTURE.md before implementation.
- Keep GPUI, GPUI Kit and platform-window types inside the desktop adapter
  (`crates/xenadmin-desktop`). The interface's model (`crates/xenadmin-ui`) depends on no
  UI toolkit.
- Keep raw RPC responses, session references, and transport details inside the XAPI adapter.
- Give each pool inventory one writer. Scope object identities and results to their pool
  and connection generation.
- Track uncertain writes explicitly. Reconcile with the server before retrying a mutation
  whose response was lost. Local cancellation does not establish server cancellation.
- Keep console frames and metrics separate from reliable operation state.
- Pin `gpui-kit` at exactly `0.9.1` (`gpui-kit = "=0.9.1"`, with `Cargo.lock` holding the
  GPUI crates it brings) and keep generated bindings reproducible. A Kit upgrade is a
  reviewed change of its own, recorded in docs/SOURCES.md.
- Keep everything derived from the upstream XAPI description inside `crates/xenadmin-xapi`,
  the one crate under the LGPL. Do not copy generated output or the description into the
  MIT-licensed crates.
- Use the reference XenAdmin source to understand behavior. Preserve its license if copying
  source. Do not imply this project is an official XCP-ng or Vates application.

## Verification boundaries

- No production host changes are authorized by ordinary application development.
- Run infrastructure-changing checks only against an explicitly designated disposable lab
  within the scope authorized for that test. Read-only connection checks are distinct from
  power, storage, network, migration, and pool-management operations.
- Never commit host credentials, sessions, private host inventories, or unredacted captures.
- Confirm the application works with XO unreachable, using the application's own connections.
- Headless output of the GPUI front end (scene renders, tests on the Kit's test platform,
  runs of the shell against the lab without a window) does not establish real Wayland,
  clipboard, keyboard, or Orca behavior.
- Record actual checks and limitations. Do not mark a milestone complete based on planned tests.

## Planning updates

Keep scope in docs/FEATURES.md, milestone sequencing in docs/PLAN.md, and ownership rules
in docs/ARCHITECTURE.md. Link to the owning document instead of duplicating its tables.

## Front end

The front end is GPUI Kit. It was moved from Rune by the plan in
[docs/GPUI-MIGRATION.md](docs/GPUI-MIGRATION.md), whose "Status" section gives the state of
every gate; the Rune crate was removed at the cutover and the client binary `xenadmin-rs`
is built from `crates/xenadmin-desktop`. Gate G0 did not pass as written: what its seat run
left open is recorded as accepted limits in
[docs/decisions/0006-console-key-codes.md](docs/decisions/0006-console-key-codes.md), and on
2026-11-17 the operator directed that the migration be finished. A limit that a later run
closes or confirms is recorded where it was stated.

Standing rules:

- Selection, tabs, commands, drafts and scenes belong in the toolkit-free model,
  `crates/xenadmin-ui`. GPUI and Kit types stay in `crates/xenadmin-desktop`, which holds
  widget state only.
- Every action enters through the command table, which feeds the menu bar, the toolbar,
  the context menus and a tab's row of local actions. No top-level button outside it.
- A surface that only shows state is a tab. Only an action is a dialog.
- `gpui-kit` stays pinned at exactly `0.9.1` until a reviewed upgrade.
- Headless output does not establish Wayland, clipboard, keyboard or Orca behavior.
- While a gate of the plan is open, `xenadmin-core`, `xenadmin-xapi` and
  `xenadmin-process` do not change for the front end. Record a core gap under
  `docs/reports/` and continue elsewhere.
- No new product scope rides on the migration.

Open, and to be said wherever the front end's status is given (the plan's "Status" section
and the gate reports of 2026-11-19 under `docs/reports/` own the detail):

- Nothing of the GPUI front end has been shown in a real window on a compositor. The
  Wayland key path for console key codes (decision 0006, route A) compiles in the client
  and has no test and no seat run in it. Clipboard, the portal file chooser, activation of
  the window by a second start, fullscreen and client-side decorations are established on
  the Kit's test platform only.
- No screen reader has been run.
- No packaged build (archive, AppImage) of the GPUI front end has been installed on a
  clean system, and the recovery gate of README.md has not been repeated with one.
- Live evidence of the GPUI front end is the headless run of 2026-11-19 against the lab
  ([report](docs/reports/2026-11-19-gpui-live-headless.md)). Connection recovery through a
  lost host, a toolstack restart or a coordinator change was not rerun through it, and the
  experimental areas were verified on fixtures and by rendering only.
