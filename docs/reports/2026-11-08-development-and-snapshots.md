# Development inspection and snapshot lifecycle increment

Version 0.4.0 continues the retained parity plan. It does not complete the full ledger.
All examples, fixtures and renders used for this increment contain synthetic data.
No private screenshots or host inventory were added to the repository.

## User interface and diagnosis

- Removed demonstration inventory entry points from wide and compact normal navigation.
  Explicit `--demo` and headless scenes remain available for isolated development tests.
- Replaced the sidebar tagline with a clickable `z23 / xenadmin-rs` repository link.
- Preserved administrator text from `VM.blocked_operations` instead of treating it as
  an API error-code enum. The display bounds text, removes control/bidirectional formatting
  characters and gives an explicit fallback for empty messages. Real API error-code
  redaction is unchanged.
- Fresh action checks expire when the relevant VM state, operations, permissions evidence,
  pool or generation changes. They also expire when inventory becomes stale. Advisory
  checks do not enable an action that current inventory does not permit.
- Host, SR and network details now display more typed metadata and attachment relationships.
  Missing booleans and measurements stay unknown rather than becoming false or zero.

## Live development inspection

`scripts/dev.sh` enables a private read-only Unix socket. `scripts/inspect.sh` queries the
running desktop's existing connections without another login. Status, typed inventory,
operation/step history and fresh power diagnostics include their connection and freshness
context. The inspector enforces ownership, permissions, peer UID, message size, deadlines
and concurrency limits. It excludes credentials, raw RPC references and mutation commands.
The [development guide](../DEVELOPMENT.md) documents the commands and data handling.

## Snapshot workflows

- Snapshot-to-template cloning and snapshot-to-VM cloning have reviewed names and immutable
  source identities. VM conversion is separately journaled and the resulting VM stays halted.
- Clone task results must identify an object absent from a bounded, freshly read and persisted
  pre-existing UUID set. Existing unrelated VM/template results cannot trigger conversion or
  a false success. Recovery without that evidence fails closed.
- Snapshot deletion optionally cleans up explicitly reviewed owned snapshot disk UUIDs.
  Current ownership and consumers are checked before dispatch; shared/unowned disks remain.
- Memory-checkpoint deletion rechecks the saved-memory identity immediately before discarding
  it and verifies cleanup before continuing. A failed cleanup leaves an actionable outcome.
- Every destructive step persists before dispatch. Failure after earlier steps yields a
  partial or unknown result. Reconciliation observes the server without repeating writes.

The retained ledger records P05-08, P05-09 and P05-10 as implemented with fixture evidence.
Other snapshot workflows and the wider parity scope remain open where the ledger says so.

## Verification and limits

The repository check covers formatting, strict Clippy, workspace tests, vendored RFB tests,
lab-harness tests and synthetic Rune renders. Focused fixtures exercise administrator
messages, changed diagnostic context, private socket lifecycle/bounds, typed inspection,
stale connections, snapshot ownership changes, checkpoint replacement, clone-result identity,
lost replies and read-only reconciliation. Rune interaction tests cover the repository link,
absence of normal demo controls, snapshot reviews and compact layouts.

`./scripts/check.sh` passed: 144 workspace tests, 46 vendored RFB tests, seven lab-harness
tests, strict workspace Clippy and all 13 synthetic render scenes. The 144 workspace tests
include 12 snapshot lifecycle fixtures, 10 live-inspection fixtures and four probe tests.

No production-host changes were performed. Live destructive qualification still requires an
explicitly designated disposable lab. Headless UI checks do not establish real desktop
accessibility or guest-console compatibility. The inspector's protocol fixtures are distinct
from observing an authenticated production session.
