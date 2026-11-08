# Export publication after recovery

The existing generic XVA/raw export path could finish archive validation after a
recovery, acknowledgement or cancellation, then publish the final file without
rechecking the operation. The separate OVF publication path already had a stop check.
The host-transfer reuse audit found this generic gap before extending that path.

Format and SHA-256 validation now prepare a publication with pinned file and parent
identities outside the journal lock. Immediately before publication, a short SQLite
immediate transaction verifies the exact operation revision, intent, private transfer
plan, successful task/stream evidence and allowed state. Original interrupted workers,
cancelled operations and revoked connections cannot publish. Explicit reconciliation
may publish only after entering its own current verification state.

Inside that fence, file/path identities are checked again and the final name is created
by an atomic rename that refuses replacement. File hashing does not hold the database
lock. An already-published matching file can still be recognized after a crash; a commit
failure after rename remains an uncertain local result that can be reconciled through
the retained receipt. Files remain private and no host operation is repeated.

Verification: the new deterministic fixture pauses after validation and injects
recovery, acknowledgement, cancellation, connection revocation, source modification or
an existing destination. No stopped case publishes a new final file, and existing
operator data is preserved. The successful control retains mode 0600. All 568 core
tests passed (340 unit, 219 direct protocol, 5 profiles, 4 TLS), including the existing
XVA/raw/OVF fixtures. Strict workspace/all-target Clippy, formatting and diff checks
passed. Independent source review found no remaining blocker. No live host was used.
