# Host recovery archive staging

Hosts → Host backup and support → Stage recovery accepts a trusted gzip-tar filesystem backup
from the captured host and product version. The native file picker inspects the complete archive
without extracting it. Review shows its SHA-256, entry count and logical size; replacing the backup
partition requires typing the captured host UUID. Changing host version/connection, closing the
review or switching mode invalidates stale results and cancels archive inspection.

The target XCP-ng 8.3 restore script first reads the running host's inventory, selects its configured
`BACKUP_PARTITION`, reformats it as ext3 while preserving its label, mounts it and extracts the
archive. It then reads the archived inventory and creates `.xen-backup-partition`. The app reports
this exact scope: the current backup partition device path is unavailable through this API;
staging does not restore running configuration, reboot, install the staged root filesystem or
restore the pool database. Those are separate installer/operator recovery steps. VM disks are
not in the host filesystem backup.

Before upload, the client checks the exact inventory UUID/version, source file identity and
compressed digest. It limits compressed input to 64 GiB, decoded/logical contents to 256 GiB,
entries to 500,000, path text to 16 MiB, individual paths to 4 KiB, extension/metadata reads to
1 MiB, and inspection to ten minutes with cancellation checks. Inventory assignments are restricted
to the known XCP-ng installer/XAPI keys and literal shell-free values. Unknown keys and executable
syntax are refused. Complete archive graph validation refuses path traversal, descendants beneath
links/non-directories, unresolved/cyclic hard links, special files, ambiguous/unsupported PAX
metadata and non-regular `.xen-backup-partition` markers. These checks do not authenticate a backup
or make arbitrary archived binaries/configuration trustworthy: the operator must trust its source.

Backup, support and restore HTTP now require HTTPS and a unique fresh selected-host certificate
among current pool members. This exact peer check composes with the existing CA/pin/skip policy
and captured proxy route. Missing/shared certificates are refused; mismatched peers receive no
cookie or archive body. Redirects remain refused. Certificate uniqueness cannot establish physical
host identity if an administrator deliberately copies the private key/certificate outside the
observed pool.

The durable operation reserves the pool, saves the owned task UUID and dispatch before the single
HTTP PUT, and hashes the actual upload stream. Success requires a complete matching stream receipt,
unchanged source and success from that exact task. Lost responses, task failure/cancellation or
missing evidence leave the backup partition outcome unknown. Recheck never creates a task or
repeats the upload. Interrupted/acknowledged rows retain their state when late stream evidence arrives.
Public operation history contains the bounded summary, not local paths, session cookies or opaque
RPC references. Source selection and transfer inspection use private file handles and no extraction.

Validation uses synthetic archives, loopback TLS/XAPI servers and Rune fixtures only. It covers
correct staging, wrong host/version, changed source, read-only/busy host, rejected redirects,
expired authentication, failed/lost results, recovery/acknowledgement, no replay, corrupt journal
plans, wrong/shared/missing certificates, shell/graph/PAX/marker checks, captured native file
selection and compact/wide review footers. Final bounded workspace run passed 792 tests (358 core unit, 219 direct protocol, five profile,
four TLS, 202 desktop and four probe), with `--test-threads=4`. Strict workspace/all-target Clippy,
formatting/diff checks and release desktop compilation passed. Final targeted host fixtures passed
after the step/receipt journal validation and visible UUID instruction adjustments. Native Vulkan
renders at 1440×900 and 513×652 were visually inspected using synthetic data.

The initial default-concurrency run exposed two pre-existing cloud-init fixture timing assumptions
(late attachment polling and acknowledgement before stream receipt). All 11 cloud fixtures pass
in isolation and in the bounded full run. The owning agent is correcting those fixtures separately;
this report does not claim a passing default-concurrency run.
Live staging and installer-assisted recovery remain deferred to the prepared disposable lab.

Primary evidence:

- [XCP-ng 8.3 xapi package](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
  selects upstream v26.1.21.
- [Restore script](https://github.com/xapi-project/xen-api/blob/v26.1.21/scripts/host-backup-restore/host-restore)
  defines partition formatting, extraction, inventory sourcing and the marker.
- [HTTP handler](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_host_backup.ml)
  defines `/host_restore` and server-task behaviour.
- [XCP-ng 8.3 host-installer package](https://github.com/xcp-ng-rpms/host-installer/blob/8.3/SPECS/host-installer.spec)
  selects [v10.10.38.xcpng.4 inventory generation](https://github.com/xcp-ng/host-installer/blob/v10.10.38.xcpng.4/backend.py).
- [XAPI inventory fields](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/libs/xapi-inventory/lib/inventory.ml)
  supplies the remaining known optional assignments.

## Integrated 0.12 verification

After integration with VM recovery, guest tools, creation cloud-init and automatic management addressing at `b5d88d4`, the default-concurrency workspace suite passed 869 tests (389 core unit, 240 direct-protocol, 227 Rune, 5 profile, 4 TLS and 4 probe). Strict workspace/all-target Clippy, formatting and diff checks passed. This successful run does not remove the separately recorded brittle cloud-fixture ordering; that deterministic-test follow-up remains tracked.
