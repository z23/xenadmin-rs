# Integrated bulk properties and HA policies — 2026-11-08

The 0.14 development tree integrates the reviewed bulk VM/template property editor and
HA startup-delay, overcommit and persistent timeout controls. Their exact workflow boundaries
and focused evidence are in the [bulk properties](2026-11-08-bulk-properties.md) and
[HA policy](2026-11-08-ha-policies.md) reports. This is a source checkpoint, not a new package
or a claim of complete retained parity.

## Integrated verification

At source commit `2dabd52`, the locked workspace suite passed **1,065 tests**: 469 core,
289 direct-protocol, 294 Rune desktop, five profile, four TLS and four host-probe tests.
Strict workspace/all-target Clippy, formatting and whitespace checks passed.
All host responses and mutations were local synthetic fixtures.

The first combined run exposed an ordering assumption in the delayed storage-Secret receipt
fixture. Startup publishes an unknown row before its asynchronous read-only reconciliation
finishes. Reconciliation can legitimately classify a retained Secret as a known partial result
if it sees the positive creation receipt. The corrected fixture waits for the initial
reconciliation to finish before releasing the reply, checks preserved unknown/acknowledged
state, then deliberately rechecks after the receipt and verifies the partial result. Both paths
retain the same Secret identity and send neither `SR.create` nor `secret.destroy`. Product
recovery policy was unchanged; the final complete suite passed without a retry.

## Ledger reconciliation

Code inspection also corrected four stale delivery descriptions: dedicated SR/network tables,
host/SR attachment graphs, offline non-coordinator removal and proven owned-resource cleanup
already have usable workflows. Their remaining restrictions and live qualification stay open.
The ledger still distinguishes these implemented subsets from broader unimplemented workflows.

The user's local lab preparation remains offline until virtualization is available. No
production host was contacted or changed, and no private inventory or screenshot was published.
