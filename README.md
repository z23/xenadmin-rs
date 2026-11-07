# XenAdmin RS

A planned Linux desktop client for direct management of XCP-ng hosts and pools.
The UI uses Rune. The backend uses Rust and XAPI.

The client must work when Xen Orchestra is unavailable. Connections, credentials,
inventory, VM consoles, and host discovery must work without an XO service.
The client requires a reachable XAPI service and the user's host permissions.

## Project status

Planning. This repository contains the implementation plan and architecture decisions.
There is no working application, Cargo workspace, or claim of host compatibility yet.
The first support target is XCP-ng 8.3 LTS on a Linux x86-64 workstation.

## Start here

- [Implementation plan](docs/PLAN.md) defines the milestones and their acceptance gates.
- [Architecture](docs/ARCHITECTURE.md) defines ownership, state, and the UI contract.
- [Feature scope](docs/FEATURES.md) maps management workflows to milestones.
- [Verification](docs/VERIFICATION.md) defines the evidence required to ship.
- [Sources](docs/SOURCES.md) records upstream revisions and unresolved compatibility questions.
- [Design decision](docs/DECISION.md) compares the architecture alternatives.

The first usable release targets direct connection, live inventory, VM power controls,
operation progress, and an embedded VM console. Later releases add creation, snapshots,
migration, storage, networks, and import and export.

## Development

Begin with milestone M0 in the implementation plan. The Rune revision reviewed during
planning is `f3d04f3f2069d177d04c963b0ba9f6b2065e46eb`. M0 must build and run that revision
before accepting it as the application's dependency. Commit `Cargo.lock` once the Rust
workspace exists.

The project uses the MIT license for original work. Preserve upstream notices if later
implementation reuses source from another project.
