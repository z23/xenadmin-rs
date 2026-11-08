# Host filesystem backup and support downloads

Implemented and verified with synthetic protocol, archive and Rune fixtures. No real host
request, production mutation, private screenshot, keyring access or support upload was used.

The host details view now offers **Host backup and support**. It captures the selected host,
pool connection generation and product version. The native save dialog selects a new file;
review keeps those choices fixed if inventory selection changes. Transfers use the existing
pool reservation, operation history, cancellation and explicit read-only reconciliation.

Filesystem backup downloads `/host_backup` directly from that host. This is a dom0 root
filesystem archive containing sensitive system files and a fresh pool database copy; VM
disks are excluded. The downloaded inventory must identify the exact reviewed host UUID and
product version. It is intended for installer-assisted recovery, not immediate configuration
replacement on a running host. Uploading/staging recovery is a separate unfinished workflow.

Support discovery reads `host.get_system_status_capabilities` and a bounded crashdump
catalogue. The view exposes advertised category keys, privacy classification, unknown or
bounded size/time estimates, and selected-host crashdump UUID/time/size. Discovery errors
remain distinct from empty catalogues. The core rechecks selected categories before requesting
`/system-status?entries=…&output=tar`. The XML inventory establishes bundle format; attribution
comes from the selected authenticated host route and owned task, not an embedded XML UUID.

The `host-crashdump-logs` category excludes raw `coredump.bin`. XAPI exposes host crashdump
upload and destroy methods, but no corresponding raw-download method; this view does not
mislabel logs as a raw memory dump. Raw dump transfer/deletion remains outside this increment.

All HTTP routes retain connection TLS and proxy policy, send the session in a cookie and
refuse redirects for host archives. An owned task UUID, successful task completion, complete
stream receipt and valid archive are required before publication. HTTP 200 alone is insufficient.
Files use mode 0600 and no-clobber publication under the journal's recovery/cancellation fence.
Interrupted streams are never repeated by reconciliation. Public history excludes local paths,
session tokens and private RPC references.

Inspection never extracts or executes archive content. The pinned `tar` 0.4.46 dependency
(MIT/Apache-2.0, default features disabled) reads GNU sparse/long-name archives. Inspection
bounds compressed download size to 64 GiB, decoded/logical size to 256 GiB, entry count to
500,000, accumulated path text to 16 MiB and extension/metadata reads to 1 MiB. It rejects
traversing paths, duplicate non-directory entries, unexpected trailing data and wrong inventory
identity. A connection cancellation or ten-minute inspection deadline stops the reader.

Synthetic coverage includes private successful backups/support bundles, capability and
version changes, wrong archive identity, truncation, expired authentication, rejected redirects,
server task failure, missing streams and no-replay rechecks; projection privacy; GNU sparse,
long-path and repeated-directory archives; extension bounds and cancellation; captured-target
review, stale file-dialog callbacks and narrow/wide modal footers. Final validation passed 781 workspace tests (349 core unit, 219 direct protocol, 5 profile,
4 TLS, 200 desktop and 4 probe), strict workspace/all-target Clippy, formatting and diff
checks, plus release-mode desktop compilation. Native Vulkan renders at 1440×900 and
513×652 were visually inspected using synthetic data. Independent source review found no
remaining blocker after the archive inspection cancellation/deadline improvement.

Primary sources for the XCP-ng 8.3 lineage:

- [XCP-ng xapi package](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
  pins upstream v26.1.21.
- [Host backup HTTP handler](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_host_backup.ml)
  sends headers before script completion, requiring task verification.
- [Backup script](https://github.com/xapi-project/xen-api/blob/v26.1.21/scripts/host-backup-restore/host-backup)
  creates the pool database dump and sparse gzip tar filesystem archive.
- [Restore script](https://github.com/xapi-project/xen-api/blob/v26.1.21/scripts/host-backup-restore/host-restore)
  reformats the configured backup partition and stages files for installer recovery.
- [System status handler](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/system_status.ml)
  supplies selected-category tar output.
- [Status report package](https://github.com/xcp-ng-rpms/xenserver-status-report/blob/8.3/SPECS/xenserver-status-report.spec)
  selects [bugtool v2.0.15](https://github.com/xenserver/status-report/blob/v2.0.15/xen-bugtool),
  including capability attributes and raw crashdump exclusions.
- [Crashdump implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_host_crashdump.ml)
  implements upload/destroy rather than a raw download endpoint.

Reference behavior was also checked in local XenAdmin `HostBackupRestoreAction.cs`,
`SingleHostStatusAction.cs` and `GetSystemStatusCapabilities.cs`. The implementation is original;
no upstream source was copied. Actual XCP-ng archive compatibility and installer recovery
remain qualification gates for the prepared disposable lab.
