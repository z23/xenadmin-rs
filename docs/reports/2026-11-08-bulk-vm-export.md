# Reviewed bulk archive exports

P15-06 now includes serial export of 1–32 captured halted VMs or templates from one
pool. The selection bar and inventory context menu open a review with each name,
UUID and independently selected new output path. Available formats are plain,
gzip or zstd XVA, an OVF directory, and an OVA file. The current ordinary-HVM OVF
subset still applies; every selected source must pass before the first transfer.
Read-only profiles may export, subject to server permissions. Review distinguishes
known RPC permissions from host-dependent HTTP authorization.

The existing journal-v4 batch owner now admits typed resource-transfer children as
well as VM children. Each private grant binds the parent, child ID, ordinal, pool,
journal kind, exact transfer action and selected output descriptor. Destination
paths stay in the private journal plan; public records and inspector output do not
contain them. All output parents, inode identities and path overlaps are checked
before any transfer. Existing files and directories are never overwritten.

Stop completes the current child and skips later VMs. Recovery and acknowledgement
fence parent and child together. Batch recheck only reads admitted task evidence
and verifies already-published files; it cannot create another child, download,
rename a new archive, cancel a task or destroy a task. A completed export describes
the verified archive at export time, not unchanged current guest configuration.
Parent history remains accessible when many children exist; child histories direct
the operator to parent controls. Unresolved children retain their parent reservation
and evidence, including across restart and pruning.

The shared OVF exporter now retains late stream/task receipts without reviving a
stopped worker, and performs short final file/directory publication under the same
journal/interruption guard as XVA. Expensive hashing and package construction stay
outside the transaction. Existing single-export reconciliation behavior is retained;
the new batch recheck has the stricter existing-artifact-only rule.

Validation:

- The full workspace run passed 1,172 tests: 523 core unit, 311 direct protocol,
  325 Rune, five profile, four TLS and four host-probe tests. A subsequently added
  parent/child-history interaction test passed with all four focused export Rune
  cases.
- New fixtures exercise two read-only XVA children, OVF and OVA manifests, whole-
  selection refusal, exact private grants, destination alias/parent replacement,
  Stop, late receipts after recovery/acknowledgement, and recheck without a second
  download or final publication. Existing delete/move batch tests also passed.
- Strict workspace Clippy, release all-target compilation, formatting and diff
  checks passed. Built-in synthetic review scenes were rendered and inspected at
  513×652 and 1440×900; the footer remains reachable. These renders do not prove
  actual desktop file portal, compositor or assistive-technology behavior.

The first nested export fixture exposed excessive debug stack usage from inline
private workflow plans. The separate [record-layout change](2026-11-08-resource-record-layout.md)
boxes those plans; the same default-stack fixture now passes. A full run also exposed
an existing storage test that treated the intermediate `Verifying` revision as a
finished recheck; separate commit `f2bec46` waits for the semantic final outcome and
retains its exact `OutcomeUnknown` assertion.

No real hosts, guests or private screenshots were used. Live compatibility and
archive restore qualification remain deferred to the disposable lab. Cross-pool
bulk moves, batch snapshot/checkpoint exports and full snapshot-ancestry archive
workflows remain open. Full `/export` XVA still lacks snapshot ancestry; memory
checkpoints and snapshots use their separate reviewed single-source workflows.
