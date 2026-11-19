# XenAdmin RS

A Linux desktop client for direct management of XCP-ng hosts and pools.
The interface is built with GPUI Kit. The backend uses Rust and XAPI.

The client must work when Xen Orchestra is unavailable. Connections, credentials,
inventory, VM consoles, and host discovery must work without an XO service.
The client requires a reachable XAPI service and the user's host permissions.

![XenAdmin RS connected to a two-host XCP-ng pool: the pool, its servers and guests in the navigation tree, and the General tab of the selected guest](docs/images/lab-inventory.png)

*A pool the client is connected to directly: the pool, its two servers and their guests in
the tree, the tabs of the selected guest, and its General tab. The menu bar and the
toolbar offer the commands that apply to the selection.*

![The Console tab of a guest, showing the guest's text console after a login](docs/images/lab-console.png)

*A guest's console, opened through the host that runs the guest. Keyboard and pointer go
to the guest, and text on the clipboard can be typed into it as key presses.*

![The Performance tab of a guest, with a CPU chart of the last ten minutes](docs/images/lab-performance.png)

*A guest's performance, read from its host every five seconds, with longer ranges and a
choice of data sources.*

The pictures are headless frames, not photographs of a window: on 2026-11-19 the client's
interface was run without a display against the project's
[disposable lab](lab/README.md), a pool of two nested XCP-ng 8.3 hosts, and these are
frames its renderer drew off screen during those runs
([report](docs/reports/2026-11-19-gpui-live-headless.md)). They are of the development
tree after the front end moved to GPUI Kit, which no person has yet used in a window
("Project status" says where it has run).

## Project status

Beta. Version 0.19.0 is the current release
([packages](https://github.com/z23/xenadmin-rs/releases/tag/v0.19.0),
[changelog](CHANGELOG.md), [release report](docs/reports/2026-11-12-release-0.19.md)), for
XCP-ng 8.3 with the client running as a Wayland application. Its supported part is
connections and certificate trust, the live inventory, VM power, the console, snapshots,
performance charts and activity; on the main branch, towards 0.20.0, host maintenance
and moving a VM within its pool have joined it. Everything else below is experimental: implemented, in
most cases with one recorded successful run, and marked with a notice in the client. The
[feature scope](docs/FEATURES.md) draws that line. The Cargo workspace builds the desktop
application, a read-only host probe and a small sandbox helper. No Xen Orchestra
service is used.

The main branch differs from 0.19.0 in its whole front end. The released versions were
built with the Rune toolkit; the main branch is built with GPUI Kit, with a new layout
(a menu bar and toolbar, navigation views, tabs per object, Properties dialogs and
wizards), by the [migration plan](docs/GPUI-MIGRATION.md), whose "Status" section says
what is established. In short: every area of the old interface is ported; the model's
tests, the interface's tests on the toolkit's headless test platform and a render of
every scene pass; on 2026-11-19 the new interface was driven headless (without a window)
against the lab through the supported areas, their successful paths
([report](docs/reports/2026-11-19-gpui-live-headless.md)) and their refusals,
interruptions and recoveries
([report](docs/reports/2026-11-19-gpui-live-paths.md)); and on the same day the packages
CI built from the branch were run in windows for the first time, in virtual machines with
software rendering and nobody at them: installed on clean Ubuntu 24.04 and Arch systems
([report](docs/reports/2026-11-19-gpui-clean-install.md)), and driven in a Wayland session
(sway) with keys and pointer injected at the machine's devices, where the client connected
to the lab, passed the recovery gate below and typed into consoles through its Wayland
keyboard path ([report](docs/reports/2026-11-19-gpui-vm-session.md)). Not established: no
person has used the new front end in a window; it has not drawn on a hardware GPU; it has
not run on Hyprland, GNOME or KDE, nor with a console on X11; no person has used a screen
reader with it; no keyring was present in any run; and the fixes those runs led to are in
no package yet. The qualification described in the rest of this section was obtained with
the released versions, through the Rune interface or through the core's own lab
scenarios.

Implemented scope, as reviewed operations with durable partial and unknown outcomes:
saved and concurrent connections with certificate trust, credentials and endpoint failover;
event-driven inventory; VM power, properties, devices, consoles, snapshots, checkpoints and
schedules; VM creation from templates, cloning and deletion; storage and network lifecycle
including local, NFS, iSCSI and CHAP repositories, ISO libraries, VLANs, bonds, multipathing
and management addressing; import, export, conversion and same- or cross-pool migration;
host and pool administration including maintenance, HA, pool join and dissolution, security
and host backups; metrics, tasks and alerts; organization, saved searches, tags, folders,
vApps and bulk actions; a portable archive and an AppImage. The
[retained parity ledger](docs/PARITY-IMPLEMENTATION.md) tracks all 202 retained rows and is
the completion authority; an implemented row is not a qualified workflow until its live,
failure and interruption paths are recorded.

Verification so far. The offline suite (core, protocol and TLS fixtures, the interface's
model and its headless window tests, and a render of every scene) runs in CI.
Live qualification runs against a disposable nested XCP-ng 8.3.0 lab:
the [live qualification report](docs/reports/2026-11-09-lab-qualification.md) records
a hundred and eleven scenarios, including three driven desktop sessions, the last with the packaged
AppImage, and the operator's sessions on a real seat (all of them with the Rune interface
of the released versions), covering connections and recovery, power and consoles, snapshots and checkpoints,
devices, storage and networking, shared NFS and iSCSI repositories with HA, live migration
and evacuation, host maintenance, reboots and power cycles, coordinator handover and
endpoint failover, together with sixty-six numbered findings and observations: host
behaviours the client now handles, host observations, client fixes and four product
decisions, since settled. Per-row results are in [lab/ACCEPTANCE.md](lab/ACCEPTANCE.md), and the
[verification traceability report](docs/reports/2026-11-11-verification-traceability.md)
maps the core verification scenarios to their evidence.

Known limits of this release are listed in the [changelog](CHANGELOG.md): the failure
and interruption paths of the experimental areas are mostly unrecorded, only XCP-ng 8.3.0
has been exercised, clipboard exchange with a guest is experimental, and screen readers
are not supported. The [implementation plan](docs/PLAN.md) records how the supported part
was qualified and what comes next.

## Run it

```sh
cargo build --locked --workspace
./target/debug/xenadmin-rs
```

Explore the interface using synthetic inventory:

```sh
./target/debug/xenadmin-rs --demo
```

Use **Add Server…** (toolbar, Server menu, or the Home tab) to enter a host endpoint and
session-only credentials. Self-signed installations can use independently verified,
endpoint-specific certificate fingerprints, under **Certificate options**. The window is
at least 800 by 600; below 1100 wide the navigation pane is behind its toggle.
The demo cannot issue host actions.

The [running guide](docs/RUNNING.md) covers Linux dependencies, certificate trust,
the host probe, local state, checks, and packaging. `./scripts/check.sh` runs the local
checks; `./scripts/package.sh` builds a release archive under `artifacts/`. The
[installation guide](docs/INSTALLATION.md) covers the portable archive, the AppImage,
the optional installer and removal.

## Start here

- [Changelog](CHANGELOG.md) lists what changed, the known issues of each build and the
  stored formats it reads.
- [Implementation plan](docs/PLAN.md) defines the milestones and their acceptance gates.
- [Retained parity ledger](docs/PARITY-IMPLEMENTATION.md) tracks all 202 retained rows.
- [Live qualification report](docs/reports/2026-11-09-lab-qualification.md) and
  [lab acceptance](lab/ACCEPTANCE.md) record what the disposable lab has verified.
- [Clean-install report](docs/reports/2026-11-11-clean-install.md) and
  [performance baseline](docs/reports/2026-11-11-performance-baseline.md) record the first
  installs outside the workstation and a first synthetic measurement.
- [Disposable lab](lab/README.md) documents the isolated nested KVM harness and its appliances.
- [Architecture](docs/ARCHITECTURE.md) defines ownership, state, and the UI contract.
- [Feature scope](docs/FEATURES.md) maps management workflows to milestones.
- [Verification](docs/VERIFICATION.md) defines the evidence required to ship.
- [Sources](docs/SOURCES.md) records upstream revisions and unresolved compatibility questions.
- [Design decision](docs/DECISION.md) compares the architecture alternatives.

The recovery release acceptance gate is to locate, start, and use the console of an XO VM
with XO unreachable. It was recorded on the disposable lab on 2026-11-11 with the Xen
Orchestra appliance imported through the client and managing the pool: XO shut down and
unreachable, the VM located, started and consoled through the direct host connection, and XO
answering again afterwards. It was run through the core facade and then through the packaged
0.17.0 AppImage started on an empty state directory, with keyboard input in the appliance's
console and the client's connections observed going to the two pool hosts only. That
AppImage carried the Rune interface. With the GPUI front end the gate was repeated once, on
2026-11-19, with the AppImage CI built from the branch at commit `e808a2f`, started on an
empty state directory in a virtual machine: a Wayland session with software rendering,
every key and pointer press injected at the machine's devices by a script, nobody at it.
Xen Orchestra was shut down and unreachable; the VM was found with the search box,
started, and typed into at its login prompt; the client's connections went to the pool's
coordinator only; XO answered again afterwards
([report](docs/reports/2026-11-19-gpui-vm-session.md), section E). It has not been
repeated by a person at a desk or with a release artifact, which is what a release still
needs (gates G3 and G5 of the [migration plan](docs/GPUI-MIGRATION.md)).

## Release history

0.18.0 was the first published version and 0.19.0 is the second, both betas, with their
packages on the project's releases page and their notes in the [changelog](CHANGELOG.md).
The versions before them were development checkpoints that froze source locally and
recorded their verification:

- 0.1 to 0.2: [alpha results](docs/reports/2026-11-07-alpha.md),
  [connection fixes](docs/reports/2026-11-07-connection-fixes.md),
  [recovery foundation](docs/reports/2026-11-07-recovery-foundation.md),
  [snapshots](docs/reports/2026-11-07-snapshots.md) and
  [operations UX](docs/reports/2026-11-08-operations-ux.md).
- 0.3 and 0.4: [parity foundations](docs/reports/2026-11-08-parity-foundations.md) and
  [development inspection and snapshot cloning](docs/reports/2026-11-08-development-and-snapshots.md).
- 0.5 to 0.9.1: checkpoints [0.5](docs/reports/2026-11-08-checkpoint-0.5.md),
  [0.6](docs/reports/2026-11-08-checkpoint-0.6.md), [0.7](docs/reports/2026-11-08-checkpoint-0.7.md),
  [0.8](docs/reports/2026-11-08-checkpoint-0.8.md) and [0.9.1](docs/reports/2026-11-08-checkpoint-0.9.1.md),
  the last adding the ownership-checked installer and AppImage.
- 0.10 to 0.16: checkpoints [0.10](docs/reports/2026-11-08-checkpoint-0.10.md),
  [0.11](docs/reports/2026-11-08-checkpoint-0.11.md), [0.12](docs/reports/2026-11-08-checkpoint-0.12.md),
  [0.13](docs/reports/2026-11-08-checkpoint-0.13.md), [0.14](docs/reports/2026-11-08-checkpoint-0.14.md),
  [0.15](docs/reports/2026-11-08-checkpoint-0.15.md) and [0.16](docs/reports/2026-11-08-checkpoint-0.16.md).
- 0.17: live lab qualification and the recovery gate; see the
  [0.17 checkpoint](docs/reports/2026-11-11-checkpoint-0.17.md) and the
  [live qualification report](docs/reports/2026-11-09-lab-qualification.md).
- 0.18, the first release: the supported part drawn and the rest marked experimental;
  fixes from the packaged desktop run and from the operator's sessions on a real seat; the
  failure paths of the supported part recorded on the lab; clipboard text typed as key
  presses. See the [changelog](CHANGELOG.md) and the
  [release report](docs/reports/2026-11-11-release-0.18.md).
- 0.19: host records read through bindings generated from the API description of the
  qualified toolstack, kept in one crate under upstream's LGPL terms; the defects that
  move found, fixed; the operation areas whose code moved run again on the lab; console
  windows drawn at the interface size and the details pane's width kept. See the
  [changelog](CHANGELOG.md) and the
  [release report](docs/reports/2026-11-12-release-0.19.md).

The per-increment reports under `docs/reports/` are linked from the ledger rows they verify.

## Development

The workspace pins Rust 1.99.1 and `gpui-kit` at exactly 0.9.1 (which brings the GPUI
crates `gpui-pre` 0.3.12); `Cargo.lock` pins the dependency graph and
[sources](docs/SOURCES.md) lists the reviewed versions.
`xenadmin-core` owns protocol and operation behavior without any UI toolkit.
`xenadmin-ui` is the model of the interface, also without a toolkit: selection, tabs, the
command table, editors, reviews and scenes, described as plain data.
`xenadmin-desktop` is the GPUI Kit front end: it draws what the model describes, shows
console frames and charts, and builds the client `xenadmin-rs` and the headless scene tool
`xenadmin-scenes`. `xenadmin-xapi` holds the
generated XAPI record views, enumerations and catalogue. `xenadmin-process` is the
post-fork helper for the offline image sandbox. `host-probe` consumes the same core
through its public facade.

The project uses the MIT license for original work, with one exception:
`crates/xenadmin-xapi`, the XAPI bindings generated from the upstream API description, is
under the LGPL 2.1 with a linking exception, as its `LICENSE` and `NOTICE` say. Binary
archives include that crate's licence and notice with the supplied dependency
notices. This is an independent project, not an official
XCP-ng or Vates application.

Use `./scripts/dev.sh` to enable private local inspection of the running desktop, and
`./scripts/inspect.sh` to query it. See [development diagnostics](docs/DEVELOPMENT.md),
which also covers the scene tool and the interface's tests. The released versions up to
0.19.0 were built with the Rune toolkit; the [migration plan](docs/GPUI-MIGRATION.md) and
its [ledger](docs/GPUI-MIGRATION-LEDGER.md) record the move and what became of every
command, scene and test.
