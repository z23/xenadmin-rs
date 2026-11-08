# Storage lifecycle increment

This increment adds direct-host storage administration to the Rune desktop. It is a
fixture-verified development feature; no real storage provisioning, detach, destruction,
trim or multipath modification was performed during development.

## Implemented workflows

The storage page has a **New storage** entry point and a selected-repository **Storage
attachments and lifecycle** dialog. Typed forms cover NFS virtual disks, ISO over NFS
and unauthenticated LVM over iSCSI. They bind the reviewed host, repository, location,
backend and operation to the current pool connection generation.

Backend discovery reads `SM.get_all_records`, combining legacy capabilities and feature
names. Explicit probing uses `SR.probe` only when `SR_PROBE` is advertised. The result
shows existing SR UUIDs, names and sizes. Inputs are captured for each probe; changing
inputs makes its results unavailable for selection. The parser rejects DTDs, malformed
UUIDs and duplicate identities and bounds bytes, nodes and results.

Create and introduce are separate operations. Introduction never forgets an existing UUID.
PBD creation is a separate reviewed command; after it completes, the operator uses
**Attach / repair** to plug it. Existing PBDs may be plugged, unplugged or removed while
detached. Existing authenticated PBDs can be reused by the host without exposing their
credentials to the client view. New authenticated attachment provisioning is unavailable.

Forget and destroy have separate explanations. Both require the exact repository UUID to
be typed, show the captured disk UUID set and require detached attachments and no disk
consumers. The core re-reads current dependencies, including pool default/suspend/crash/HA
storage and host cache/suspend/crash references. The UI limits these reviews to 100 disks;
larger repositories need a future paged review rather than a truncated destructive approval.

Per-PBD inspection shows attached state, advertised multipathing state and bounded active /
maximum path counts. Attached means the host is maintaining a connection, not that storage
is healthy. Capability-gated reclaim uses only the `trim` plugin's `do_trim` function on an
explicitly reviewed attached host. A positive plugin result is retained as safe completion
evidence; arbitrary plugin result XML is not exposed in history.

## Tracking and recovery

Storage operations share the resource journal, controls, history and conservative pool-wide
admission guard with VM and infrastructure operations. Each command has one host mutation.
The journal records dispatch before submission and retains private task/result references.
Creation checks a pre-existing UUID baseline, returned identity, backend/metadata and
attachment topology; PBD creation also uses an operation marker and verifies host/SR/config.
Task result references survive a failed verification read. Rechecking uses saved evidence
without repeating a creation or completing an unsubmitted attachment action. A lost task
reply without sufficient result identity stays unknown and holds the local reservation.

No password, XAPI Secret value, session reference or raw device configuration is included
in the public action model or inspection output. The safe location projection is disabled
when the attachment uses authentication keys. Backend errors retain only safe error codes.

## Verification

Synthetic checks cover successful creation and typed result verification, wrong result
identity, failed verification followed by recheck, lost replies without replay, duplicate
host/UUID refusal, active-consumer and default-SR removal refusal, attach/detach identity,
read-only discovery and generation cancellation, redacted failures, secret-free attachment projection, bounded probe parsing and trim
capability/attachment/result requirements. Rune interaction fixtures cover immutable review
inputs, distinct forget/destroy review and visible compact confirmation controls that cannot
submit from demo mode. Synthetic desktop scenes use public example names only.

Final checks passed: `cargo test --workspace` (271 tests, including 17 added storage tests),
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` and
`git diff --check`. The `storage-create` (1440×900) and `storage-review` (513×652) scenes
were rendered through Rune/Vulkan and visually inspected. These renders do not establish
real keyboard, clipboard, screen-reader or Wayland interaction compatibility.

Live acceptance must use an explicitly authorized disposable storage lab and test reconnect/cancellation,
backend versions, task cleanup and actual data preservation/removal separately.

## Remaining scope

- NFS export enumeration and iSCSI IQN/LUN discovery; the current forms require explicit targets.
- CHAP/SMB provisioning with journalled XAPI Secret creation, duplication and cleanup.
- Non-default iSCSI ports and backend-specific advanced options.
- Automatic multi-host attachment/repair orchestration and authenticated missing-PBD recovery.
- Host multipath setting changes, maintenance sequencing and path failover qualification.
- Read caching, thin/thick provisioning options and additional backend wizards.
- Automatic orphan cleanup after server-side partial creation and full storage health diagnosis.
- Real XCP-ng version/backend compatibility and destructive lab qualification.

These gaps remain open in the [retained parity ledger](../PARITY-IMPLEMENTATION.md).

## Protocol sources

- [XAPI SR class](https://xapi-project.github.io/xen-api/classes/sr.html): create, introduce,
  probe, forget/destroy, advisory allowed operations and storage relationships.
- [XAPI PBD class](https://xapi-project.github.io/xen-api/classes/pbd.html): constructor fields,
  plug/unplug and the limits of `currently_attached` as a health indication.
- [XAPI SM class](https://xapi-project.github.io/xen-api/classes/sm.html): configuration
  metadata, feature versions, deprecated capabilities and coordinator/pool scope differences.
- [NFS backend](https://github.com/xapi-project/sm/blob/master/libs/sm/drivers/NFSSR.py) and
  [LVM-iSCSI backend](https://github.com/xapi-project/sm/blob/master/libs/sm/drivers/LVHDoISCSISR.py):
  typed device-config keys and optional capability names. Baseline create/attach/delete
  methods must not be gated on nonexistent SR_CREATE/SR_ATTACH/SR_DELETE capability flags.
- Reference XenAdmin `SrCreateAction.cs`, `SrIntroduceAction.cs`, `SrRepairAction.cs`,
  `SrTrimAction.cs` and `PBD.cs` were consulted for behavior. Their source was not copied.
