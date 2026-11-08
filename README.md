# XenAdmin RS

A Linux desktop client for direct management of XCP-ng hosts and pools.
The UI uses Rune. The backend uses Rust and XAPI.

The client must work when Xen Orchestra is unavailable. Connections, credentials,
inventory, VM consoles, and host discovery must work without an XO service.
The client requires a reachable XAPI service and the user's host permissions.

## Project status

Development alpha. The Cargo workspace builds a Rune desktop application and a read-only
host probe. It implements direct XAPI login, event inventory, VM power tasks, durable
operation tracking, snapshot listing/creation/restore, VM properties and device controls,
multiple saved connections, optional keyring credentials and an embedded RFB console.
No Xen Orchestra service is used.

Local protocol and TLS fixtures, Rune widget interactions, headless rendering, and a native
window smoke test pass. **Successful live-host qualification is pending.**
The first qualification target is XCP-ng 8.3 LTS on Linux x86-64.
See the [actual results and remaining gaps](docs/reports/2026-11-07-alpha.md).
Version 0.1.1 adds a connection-wide certificate-validation opt-out and fixes inventory
bootstrap with unknown storage sizes; see the [connection fix report](docs/reports/2026-11-07-connection-fixes.md).
Version 0.1.2 strengthens interrupted-operation recovery and adds resume/unpause controls;
see the [recovery foundation report](docs/reports/2026-11-07-recovery-foundation.md).
Version 0.2.0 adds per-VM snapshot views, disk-snapshot creation, and guarded restore;
see the [snapshot report](docs/reports/2026-11-07-snapshots.md) and [feature comparison](docs/PARITY.md).
Version 0.2.1 keeps unavailable power actions visible with an explanation; see the
[UX verification](docs/reports/2026-11-08-operations-ux.md). The
[full parity checklist](docs/PARITY.md) distinguishes implemented, partial, missing and
unqualified workflows, with priorities and reference-source evidence.

Version 0.3.0 adds connection/profile ownership, device inventory and actions, reviewed VM
properties, chosen-host start/resume, memory checkpoints and console controls. See the
[implementation and verification report](docs/reports/2026-11-08-parity-foundations.md)
and [retained parity ledger](docs/PARITY-IMPLEMENTATION.md). The complete parity plan is
still in progress.

Version 0.4.0 adds snapshot-to-VM/template cloning, reviewed snapshot deletion, richer
resource details and private live-client development inspection. It also preserves host
administrative block messages and removes the demonstration entry from normal navigation.
See the [verification report](docs/reports/2026-11-08-development-and-snapshots.md).

Version 0.6.0 adds reviewed VLAN/bond and storage administration, same-pool migration,
pool task/alert activity, and live VM resource settings. See the
[checkpoint report](docs/reports/2026-11-08-checkpoint-0.6.md) for verification and limitations.

## Run it

```sh
cargo build --locked --workspace
./target/debug/xenadmin-rs
```

Explore the interface using synthetic inventory:

```sh
./target/debug/xenadmin-rs -- --demo
```

Use **Add connection** to enter a host endpoint and session-only credentials. On narrow
windows, use **Connect** and **Details** in the compact controls. Self-signed installations
can use independently verified, endpoint-specific certificate fingerprints.
The demo cannot issue host actions.

The [running guide](docs/RUNNING.md) covers Linux dependencies, certificate trust,
the host probe, local state, checks, and packaging. `./scripts/check.sh` runs the local
checks; `./scripts/package.sh` builds a release archive under `artifacts/`.

## Start here

- [Implementation plan](docs/PLAN.md) defines the milestones and their acceptance gates.
- [Retained parity ledger](docs/PARITY-IMPLEMENTATION.md) tracks all 202 retained rows.
- [Disposable lab](lab/README.md) documents the isolated nested KVM harness.
- [Architecture](docs/ARCHITECTURE.md) defines ownership, state, and the UI contract.
- [Feature scope](docs/FEATURES.md) maps management workflows to milestones.
- [Verification](docs/VERIFICATION.md) defines the evidence required to ship.
- [Sources](docs/SOURCES.md) records upstream revisions and unresolved compatibility questions.
- [Design decision](docs/DECISION.md) compares the architecture alternatives.

The recovery release acceptance gate is to locate, start, and use the console of an XO VM
with XO unreachable. This requires a designated disposable lab and authorized test scope.
Later releases extend snapshot workflows and add VM creation, migration, storage and network administration,
metrics, and import and export.

## Development

The workspace pins Rust 1.99.1 and Rune
`f3d04f3f2069d177d04c963b0ba9f6b2065e46eb`; `Cargo.lock` pins the dependency graph.
`xenadmin-core` owns protocol and operation behavior without Rune dependencies.
`xenadmin-desktop` adapts snapshots and console frames to Rune. `host-probe` consumes the
same core through its public facade.

The project uses the MIT license for original work. Binary archives include supplied
dependency and embedded-font notices. This is an independent project, not an official
XCP-ng or Vates application.

Use `./scripts/dev.sh` to enable private local inspection of the running desktop, and
`./scripts/inspect.sh` to query it. See [development diagnostics](docs/DEVELOPMENT.md).
