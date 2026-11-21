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

- On 2026-11-21 the operator ran a local build of the GPUI front end, reported that it looks
  good, and directed that this tree become the main version. That run names no commit,
  compositor, renderer, host or scenario, and no hardware GPU is recorded as having drawn it.
  The recorded window runs are in virtual machines, on 2026-11-19 and 2026-11-20, with a
  software renderer and nobody at them: a driven session with keys and pointer injected at the machine's
  devices, on sway 1.12
  ([report](docs/reports/2026-11-19-gpui-vm-session.md)), and the window opened on clean
  Ubuntu 24.04 and Arch systems
  ([report](docs/reports/2026-11-19-gpui-clean-install.md)). In that session the Wayland
  key path for console key codes (decision 0006, route A) carried injected keys to a lab
  guest, and the clipboard with the window focused, the portal file chooser, fullscreen
  and server-side decorations were observed. A second start often showed nothing with
  that package; with a build of the fix it marked the window urgent on sway and focused it
  only where sway's policy allows, and with the CI package of the merged tree (commit
  `3493640`), run in the same machine on 2026-11-20, it marked the window urgent
  ([report](docs/reports/2026-11-20-gpui-merged-tree.md)). In that run a held key repeated
  in a lab guest's console, and a pointer press on a button did nothing while a key was
  held and repeating.
  None of that is established for a real keyboard, for Hyprland, GNOME or KDE, or for X11
  with a console; the listener of route A has no test.
- No person has used a screen reader with it. In the same virtual machine the client was on
  the accessibility bus and Orca wrote names and roles for the demo's views to its log;
  nobody heard it. The accessibility statement stays "not supported".
- The packaged builds that were installed and run are the archive and the AppImage CI built
  at commit `e808a2f`, in those virtual machines only. The recovery gate of README.md was
  repeated with that AppImage in the driven session, not by a person and not on a release
  artifact. The fixes made on 2026-11-19 and 2026-11-20 are in the packages CI built
  at commit `3493640`; of those only the AppImage was run, in the driven session's machine,
  not on a clean system and without the recovery gate.
- Live evidence of the GPUI front end is headless: the runs of 2026-11-19 against the lab
  ([successful paths](docs/reports/2026-11-19-gpui-live-headless.md),
  [refusals, interruptions and recoveries](docs/reports/2026-11-19-gpui-live-paths.md)),
  the first of them repeated on the merged tree on 2026-11-20.
  A coordinator handover to the member and back, and maintenance of that coordinator
  with no resident guest, were run headless on 2026-11-21
  ([report](docs/reports/2026-11-21-gpui-coordinator.md)). The host's refusal to
  evacuate a running diskless VM was run headless the same day
  ([report](docs/reports/2026-11-21-gpui-evacuation-refusal.md)). A memory snapshot
  in flight during a restart of the coordinator's toolstack was run headless
  the same day
  ([report](docs/reports/2026-11-21-gpui-inflight-toolstack.md)). A saved password
  in the keyring was not run through it, and the experimental areas were verified
  on fixtures and by rendering only.
