# Project instructions

## Product contract

- Build a standalone Linux XCP-ng management client with Rune and Rust.
- Connect directly to XAPI. Xen Orchestra must never be required to start, authenticate,
  discover a pool, run a command, or open a console.
- Keep the UI responsive through slow hosts, reconnects, and console traffic.
- This repository contains a development alpha. Report implemented, fixture-verified,
  live-host-verified, and planned behavior separately; see docs/reports/ for evidence.

## Implementation boundaries

- Read docs/PLAN.md and docs/ARCHITECTURE.md before implementation.
- Keep Rune and platform-window types inside the desktop adapter.
- Keep raw RPC responses, session references, and transport details inside the XAPI adapter.
- Give each pool inventory one writer. Scope object identities and results to their pool
  and connection generation.
- Track uncertain writes explicitly. Reconcile with the server before retrying a mutation
  whose response was lost. Local cancellation does not establish server cancellation.
- Keep console frames and metrics separate from reliable operation state.
- Pin Rune to a reviewed full commit and keep generated bindings reproducible.
- Use the reference XenAdmin source to understand behavior. Preserve its license if copying
  source. Do not imply this project is an official XCP-ng or Vates application.

## Verification boundaries

- No production host changes are authorized by ordinary application development.
- Run infrastructure-changing checks only against an explicitly designated disposable lab
  within the scope authorized for that test. Read-only connection checks are distinct from
  power, storage, network, migration, and pool-management operations.
- Never commit host credentials, sessions, private host inventories, or unredacted captures.
- Confirm the application works with XO unreachable, using the application's own connections.
- Rune headless screenshots do not establish real Wayland, clipboard, keyboard, or Orca behavior.
- Record actual checks and limitations. Do not mark a milestone complete based on planned tests.

## Planning updates

Keep scope in docs/FEATURES.md, milestone sequencing in docs/PLAN.md, and ownership rules
in docs/ARCHITECTURE.md. Link to the owning document instead of duplicating its tables.
