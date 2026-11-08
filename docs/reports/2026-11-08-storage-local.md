# Reviewed local EXT/LVM provisioning

This increment adds a direct-XAPI workflow for local EXT and LVM storage using a single
operator-supplied persistent device path. It is fixture-verified implementation; no physical
device has been formatted or tested against a live host.

## Workflow and the API boundary

The storage form offers Local EXT and Local LVM, an exact selected host, and one
`/dev/disk/by-id/…` path. It probes both drivers for existing repositories. An existing result
selects nonformatting introduction by UUID; attachment creation and plugging remain separate
reviewed actions. Local repositories are nonshared and limited to one host attachment.

Creating local storage requires an operator statement identifying the physical disk or
partition, a data-loss acknowledgement, and the entire device path typed again on the captured
review. The statement is explicitly an operator declaration, not server-verified evidence.
Device lists, traversal, mapper/RAID aliases and malformed paths are refused. Host or path
changes invalidate the probe and acknowledgement. The backend independently repeats checks
before dispatch; a synthetic preview cannot submit writes.

This is not automatic physical disk discovery. The target EXT/LVM `SR.probe` implementations
report existing volume groups; their underlying scan catches per-device errors and can return
an empty list even when a path cannot be inspected. Consequently the UI never describes an
empty probe as proof that a disk exists, is readable, is blank or is unused. Device capacity,
partition/signature state, arbitrary aliases and external consumers are not fully established
by these APIs. A persistent path can still be retargeted outside this client. The operator must
verify the intended device independently before accepting permanent data loss.

## Fresh checks, ownership and recovery

The core validates the exact host UUID and idle host state, bounds driver and repository data,
requires both EXT/LVM drivers to advertise device probing, and refuses any existing repository
from either probe. It also checks registered PBD paths (including detached attachments),
whole-device/partition families, SCSI IDs and SR serial metadata for known use. Unmatched
aliases are not interpreted as proof of nonuse. The storage driver remains responsible for
its own current device/root/in-use checks and can reject the operation.

The captured declaration and typed location belong to the existing immutable action and
pool/generation observation. `Async.SR.create` uses `shared=false`; completion requires the
owned task and a new SR/attachment identity with the requested host, backend and exact path.
Wrong shared state or graph, failed tasks and lost responses remain unknown, with the existing
operation reservation and private reconciliation evidence. Recheck never formats again and
no unowned SR/PBD is automatically forgotten or destroyed. A host failure may occur after
some device writes; the review explicitly warns that an unknown result is not proof that
existing data survived.

For introduced local SRs, attachment creation checks both the SR attachment list and a fresh
PBD scan before creating the first host attachment. Plug/repair checks require one exact
attachment and nonshared state. This does not reconstruct a missing physical-device identity
or create attachments across multiple hosts. Inventory projects local paths only when the
associated SR type is known, because EXT and LVM use the same device-configuration key.

## Primary evidence

- [XCP-ng storage documentation](https://docs.xcp-ng.org/storage/) documents local EXT/LVM
  creation using a selected host and `device-config:device`.
- [XAPI SR API](https://xapi-project.github.io/xen-api/classes/sr.html) describes backend-specific
  probing; it does not establish a universal local block-device inventory contract.
- [XCP-ng 8.3 SM packaging](https://github.com/xcp-ng-rpms/sm/blob/8.3/SPECS/sm.spec) starts from
  SM v3.2.12 and applies downstream driver patches. The implementation requires the installed
  driver's advertised probing capability rather than assuming an unmodified upstream binary.
- [EXTSR](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/EXTSR.py) and
  [LVHDSR](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/LVHDSR.py) accept a device path
  and use different volume-group prefixes for probes.
- [LVM utilities](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/lvutil.py) show
  `scan_srlist` suppressing per-device inspection failures, and destructive volume-group
  creation zeroing the beginning of selected devices before PV/VG creation.
- [SM utilities](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/util.py) implement
  current PBD-device and serial-use checks. They are not comprehensive external ownership proof.
- The local XenAdmin `LocalStorageFrontend` accepts an operator-provided device string rather
  than exposing an authoritative physical disk inventory. Its behavior informed this flow;
  no reference source was copied.

## Verification and retained work

- Seven new synthetic protocol groups cover EXT/LVM creation, dual probes and changed or
  existing repositories, malformed/incomplete driver evidence, known path/serial use, read-only
  discovery, nonformatting introduction and first-host attachment, multi-host refusal, and
  failed/lost/mismatched task outcomes with no replay or automatic cleanup.
- A model test covers accepted persistent paths, lists/traversal/mapper refusal, conservative
  partition-family matching, declaration bounds and backend-aware location projection. Existing
  legacy create/location serialization tests continue to pass with the optional declaration absent.
- Five Rune interaction groups cover captured host/path/declaration, required probe and
  acknowledgement, exact confirmation and footer reachability at 1440×900 and 513×652,
  nonformatting existing-result selection, and focus versus actual text-edit invalidation.
  Starting another probe invalidates the old result even if the new request cannot start.
- The full workspace passed 1,003 tests: 445 core unit/protocol, 278 direct-connection,
  267 desktop, five profile, four TLS and four host-probe tests. After the final reprobe
  invalidation change, all five affected Rune groups passed again. Strict workspace/all-target
  Clippy, formatting and diff checks passed, as did the native workspace build.
- Four synthetic native Vulkan renders were visually inspected for form/review at both sizes.
  The long body scrolls while heading/footer controls stay reachable. These renders are private
  temporary artifacts; no screenshots or live inventory are committed.
- Independent source review found no blocking issue in the new local backend, fresh graph
  guards and existing durable worker integration; the reviewer did not rerun fixtures.

Automatic local physical inventory, arbitrary alias resolution, remote partition/signature
inspection, other supported/as-is backend wizards and complete missing-PBD/multipath
orchestration remain retained scope. A future richer discovery path needs an actual supported
host API; this increment introduces no shell execution, custom host agent or fabricated
capacity/ownership result. Real formatting, attachment, data integrity and recovery remain
qualification gates in the designated disposable lab. All fixtures and renders are synthetic.
