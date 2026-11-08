# Virtual disk growth, copy and owned cleanup

This increment adds a virtual-disk workspace from storage details and VM disk cards. It talks directly to the connected pool. Reviews capture pool/generation, disk UUID, storage, size and each ordinary VM attachment; changing table selection does not retarget a review.

## Implemented behavior

- Grow an ordinary managed, writable, nonshared disk with no active or reserved attachments. Detached disks and unplugged attachments on running or halted ordinary VMs are supported. Every attachment and the disk's advertised `resize` capability are rechecked. Growth is positive, sector aligned and bounded by the signed XAPI size range. A driver may round upward: completion shows the actual backing size. Guest partitions and filesystems remain the operator's separate task.
- Copy an inactive ordinary disk to explicitly chosen storage with `Async.VDI.copy`. The original is preserved and the copy stays detached. A fresh task and disk UUID baseline excludes pre-existing results. Naming is a separate durable step, so an interrupted or refused rename retains the positively identified new disk. Existing `other_config` is not rewritten merely to add an ownership tag.
- Delete a fully detached owned disk after a review naming the disk, origin operation and exact UUID confirmation. Provenance comes from terminal local device-creation, raw/VHD import, NoCloud creation, verified VM-creation/copy graphs or this new disk-copy workflow. Workflows that use an ownership marker must still have that exact marker. Cleanup refuses unknown originating operations, arbitrary untracked disks, all VBD consumers (including consumers missing from the VDI's list), suspend images, snapshot parents, HA statefiles, shared/read-only disks and CBT state. Unsupported provenance is explained rather than inferred from a familiar name or an empty attachment list.
- Operation history exposes recheck, stop remaining steps and explicit unknown acknowledgement. Local stop does not cancel in-flight host disk I/O. A recheck reads existing receipts and current state; it never submits a missing resize, copy, rename or delete step. Lost responses remain uncertain even if a current field happens to match the requested result.

Private plans store task identity, result and baseline evidence separately from public serialization. Journal validation binds those plans to the action, target, pool and public step shape. Monotonic evidence merges preserve recovered and acknowledged states and stop later writes. The public inspection report includes the observed backing size, never raw references.

## Target API evidence

[XCP-ng 8.3's xapi package](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec) identifies the v26.1.21 source lineage. Its [VDI implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vdi.ml) rejects active/reserved attachments for ordinary resize and returns the storage driver's resulting size. Copy creates a new disk and inherits source metadata. Destroy also removes inactive VBD records and suspend-image references; the client therefore explicitly refuses those consumers before orphan cleanup.

The [current VDI API documentation](https://xapi-project.github.io/xen-api/classes/vdi.html) records removal of `resize_online` in the XenServer 7.3 lineage and later reintroduction in a newer API lineage. Class presence alone is not evidence that XCP-ng 8.3 supports it. This increment uses ordinary `VDI.resize`, never an invented online fallback. [XCP-ng storage guidance](https://docs.xcp-ng.org/storage/manage-srs/) distinguishes growth from shrinking and documents format-dependent size limits; the client does not assume every disk uses the VHD limit.

Reference behavior was checked in XenAdmin's `VDISizeLocationPage.cs`, generated `VDI.cs`, `MoveVirtualDiskAction.cs` and `DestroyDiskAction.cs`. No reference implementation was copied.

## Verification

- The full workspace suite passed **868 tests**: 393 core unit/protocol cases, 236 direct-XAPI integration cases, five connection-profile cases, four TLS cases, four host-probe cases and 226 Rune/desktop cases.
- The final focused disk run passed **13 protocol/journal suites**, including a subsequently added upfront permission-denial case. They cover driver rounding, inactive running-VM attachments, changed/reserved/active consumers, existing copy results, rename refusal, lost replies, recovery and acknowledgement races, imported ownership markers, HA UUID dependencies, malformed private plans and public serialization. The added case brings the current unique test count to 869; the last complete workspace run preceded that test-only addition.
- Four new Rune interaction tests passed at 1440×900 and 513×652, covering immutable selection, compact controls, input validation, unsupported cleanup provenance, explicit deletion confirmation, demo non-dispatch and access to operation history.
- Strict workspace/all-target Clippy, formatting, diff checks and the workspace build passed. Six native Vulkan synthetic renders (grow, copy and cleanup at both sizes) were visually inspected; titles and footer controls remained visible, body text wrapped, and synthetic confirmation remained disabled. The scenes are included in the target-aware check script.
- Independent source review found no blocking ownership, identity, journal or recovery issue. All inputs and protocol servers were synthetic. No production host or user screenshot was used.

The separate cloud fixture correction (`e8cf5eb`) replaces timing assumptions with explicit response gates. Its 15 focused tests and five repeated 32-thread runs passed; that fix is already integrated independently.

## Remaining boundaries

Live XCP-ng/SR-driver compatibility and guest data integrity remain deferred to the disposable lab. Online resizing of attached disks, shrinking, suspended/paused/template/snapshot consumers, CBT/shared/special-purpose disks and unproven orphan provenance are not supported by this workspace. Existing reviewed VM migration remains the path for supported disk relocation; standalone copy does not detach or delete the source. Cleanup discovery uses the locally available operation history; losing that history does not grant authority to delete an untracked disk.

## Integrated verification

At combined source `56c789b`, all **921 workspace tests** passed: 412 core unit,
256 direct protocol, 240 Rune/desktop, five profile, four TLS and four probe cases.
Strict workspace/all-target Clippy passed. This includes existing management-topology,
alarm, crashdump, disk and deterministic cloud-upload recovery fixtures. No live host
changes were performed; disposable-lab qualification remains deferred.
