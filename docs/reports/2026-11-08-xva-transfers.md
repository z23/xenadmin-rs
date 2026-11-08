# Authenticated XVA transfers

This increment adds native file selection and reviewed XVA import/export to the direct-host desktop. It is implemented and verified against synthetic hosts and archives. No live host was changed, and production compatibility remains unqualified.

## Delivered workflows

- Export a halted VM, custom template, or halted disk snapshot to a new local XVA file. A running VM can first use the existing disk-snapshot workflow. Memory checkpoints and snapshot-chain archives remain outside this increment.
- Import ordinary halted guests/custom templates from plain or gzip XVA to a selected attached user SR. The review shows archive guests, disk count, virtual capacity and file size. XAPI matches existing networks by name/bridge or creates networks; guests remain halted for NIC review before starting.
- Native Open/Save dialogs capture the source/pool and selected file identity. Existing destinations, symlinks, changed source files, unsupported archive object classes, control domains and built-in templates are rejected.
- Transfer history shows streamed bytes, server-task outcome and confirmed retained guest/disk/network UUIDs. Stop closes the local stream and separately requests server-task cancellation when available. Recheck never uploads or downloads again.
- Read-only connections can explicitly export and recover their export history. Import and unrelated mutations retain the read-only guard. This requires server permission to create and manage the owned transfer task as well as export permission.

## Boundary and recovery

Transfers use only fixed `/export` GET and `/import` PUT endpoints. The existing endpoint trust policy applies to every HTTP client; session authentication uses a transient Cookie and is absent from the URL and journal. Export redirects are bounded and restricted to current pool hosts, HTTPS/default port for a different host, the same endpoint path and unchanged approved parameters. Upload selects an attached SR host before dispatch and refuses redirects rather than replaying bytes.

The resource journal reserves the pool before work and persists task ownership, pre-existing object UUID baselines, immutable local file identity and stream/task receipts. Task identity is checked against a fresh baseline and operation-specific label before transfer. Import success requires matching task-result references and fresh VM/disk/network provenance, with no pre-existing UUID accepted as newly created. Upload reply loss can reconcile through the existing successful task; it never resends the archive.

Failed or cancelled imports remain **Outcome unknown**. XAPI can create VM/network records before adding their `import_task` marker, so tagged inventory cannot prove complete cleanup. Confirmed retained UUIDs are shown, but untagged objects are never claimed or deleted. The pool reservation remains until inspection and explicit acknowledgement in transfer history. No automatic cleanup destroys guest data.

Export writes private mode-0600 files beside the destination, using bounded streaming and an owned inode. It publishes the final file with atomic no-replace rename only after a complete byte/hash receipt, selected-source metadata identity and successful server task. Persisted completion can recover even after the host task or temporary pathname disappears. File identity and paths remain in the private plan; Activity exports and the development inspector use explicit public projections, and raw public record serialization also omits the private plan.

Bounds are 8 TiB per archive, 64 guests, 256 data disks/networks, 8 MiB decoded metadata, bounded XML depth/node count, 64 KiB upload buffers, 60-second idle response reads and a 24-hour transfer deadline. Archives larger than these limits need a future reviewed extension. Export SHA-256 is local receipt evidence; XAPI remains responsible for XVA disk-chunk checksum validation during import.

## Verification

Synthetic HTTP fixtures cover successful private export, import provenance, reply loss without replay, foreign/changed redirects, session expiry, truncated/wrong archives, missing tasks, pre-existing guest identity, orphan and untagged resources, task cancellation, restart after atomic publication, disk-snapshot normalization, gzip metadata, source changes and symlinks. Native Rune tests cover Open/Save, cancellation, stale dialogs/pools and compact/wide review with demo writes disabled. All 327 workspace tests passed, followed by the final 18 core transfer and four Rune transfer cases after adding acknowledgement coverage (329 tests in the resulting workspace). Strict workspace Clippy, formatting and diff checks passed. Synthetic wide and compact renders were inspected under `/tmp`; no user screenshots or live inventory were copied into the repository.

## Follow-on scope

OVF/OVA, standalone disk-image import, conversion, zstd XVA, cross-pool mapping/copy, transfer-network selection for archive transfer, resumable/range transfer, overwrite of existing local exports, memory-checkpoint export and snapshot-chain archives remain pending. Host backup/restore/support-bundle transfer can reuse the fixed-endpoint streaming boundary in a separately typed workflow. Direct cleanup of orphan VDIs/networks still requires a future reviewed resource workflow; normal created guests can use the existing explicit lifecycle deletion.

## Semantic references

Behavior was implemented independently from the primary XAPI interface and reference behavior; no source implementation was copied. [XAPI import/export interface](https://xapi-project.github.io/xen-api/importexport.html) documents task-linked HTTP transfers. [Import implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/import.ml) establishes new-identity imports, task-result guest references and the timing of provenance tags. [Export implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/export.ml) clears snapshot lineage on normal exports; fixtures begin with a synthetic snapshot-shaped source and verify the normalized archive identity.
