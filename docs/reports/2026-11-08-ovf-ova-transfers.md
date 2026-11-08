# OVF/OVA transfers

This increment adds an ordinary HVM OVF/OVA subset to the existing authenticated
transfer boundary. It does not complete general OVF interoperability or the
conversion manager. All checks below use synthetic data and loopback HTTP fixtures;
no production host was changed and no live-host qualification is claimed.

## Implemented

- Native OVF/OVA import selection, background inspection with cancellation, captured
  file identities, guest CPU/memory/BIOS-or-UEFI and disk review, one explicitly
  selected destination SR, and an explicit mapping for every declared network.
- Plain descriptor packages and uncompressed regular-file OVA archives. Complete
  raw and fixed/dynamic VHD images are accepted. A bounded OVA member is streamed
  directly, without extracting untrusted paths or reading the entire image into RAM.
- Optional SHA-256 or legacy SHA-1 manifest integrity verification. A supplied
  manifest must cover the descriptor and every disk. This is integrity checking,
  not publisher authentication; certificate/signature packages are refused.
- New halted VMs, private per-guest VDI copies, owned disk attachments and mapped
  NICs with generated MAC addresses. CPU/memory, firmware and device slots are
  checked through fresh records; compatible hosts are checked before completion.
  No guest power action is submitted.
- Halted ordinary VM, custom-template and disk-snapshot export to an OVA file or a
  new OVF directory with `guest.ovf`, `guest.mf` and VHD disks. Snapshot lineage is
  normalized to an ordinary guest configuration. Disk UUIDs, storage, capacity,
  attachment graph and guest configuration are rechecked before publication.
- Read-only connection export remains supported, subject to host permissions for
  the owned transfer task and fixed disk-export endpoint.
- Private staging and no-clobber publication: files use mode 0600, package directories
  use mode 0700, and the final name appears through an atomic rename after format,
  task and manifest verification. An existing destination is preserved.

## Recovery and ownership

Every creation stores a bounded pre-existing UUID baseline and a unique ownership
marker before dispatch. Returned references and verified VM/VDI/VBD/VIF UUIDs are
persisted individually. No result can adopt an existing identity or silently claim
ownership of an imported shared disk. Positive late evidence is merged even after
recovery or acknowledgement, without reopening a terminal operation.

Each disk stream has a separately owned task, immutable private file identity,
expiry, dispatch marker, task receipt and local stream receipt. Stream completion
and host completion are independent. Stopping the stream does not establish host
cancellation. Recheck reads saved tasks or completion proof; it never repeats a
creation, upload or download, and never applies unsubmitted remaining steps.
Uncertain work keeps the pool reservation. Definite stopped imports retain their
identified guests, disks and devices as partial results; no automatic rollback or
deletion is attempted. Retained device UUIDs appear in transfer history and Activity
exports. Private paths, raw references and ownership plans remain outside public
record serialization and diagnostic projections.

## Explicit limits

- VMDK, VHDX, QCOW2, differencing/external-parent images and format conversion remain
  pending. This does not yet import the full range of VMware/XenCenter appliances.
- Conditional hardware/deployment profiles, resource constraints, guest properties,
  unsupported required extensions, custom controller models/SCSI/SATA controllers,
  multiple IDE controllers, missing/invalid disk-parent references, mounted installation
  media and specialized devices are refused instead of silently discarded.
- Export currently supports ordinary HVM guests with `dc` boot order and Secure
  Boot disabled. It captures basic resources, firmware, data disks and NIC placement;
  advanced Xen-specific settings, guest customizations and appliance startup ordering
  are not represented. Imported collection members are ordinary individual VMs,
  not a newly created XAPI appliance group. General cloud-init and conversion remain
  separate outstanding work.
- Export does not hold a server-side guest lock across all disk downloads. An
  independent administrator can still change a guest during transfer; export a disk
  snapshot for a stable point in time. Fresh checks reject observed changes.
- New output names are required; package overwrite, resumable byte ranges and
  automatic retry/cleanup of unknown server-side changes are not implemented.
  Failed or interrupted exports can retain private staging evidence until the
  operation is reconciled; successful OVA export removes its owned staging files.
- Native window-system dialogs, a real pool with XO unavailable, large image
  throughput and imported guest boot compatibility still require lab qualification.

## Evidence

Core fixtures cover parser/path/format bounds, manifest changes, regular-file-only
OVA handling, bounded VHD member reads, positive base-256 large TAR sizes, explicit
network mapping, complete OVF/OVA imports, collections with private disk copies,
wrong/pre-existing IDs, ownership changes, each lost creation reply, failed and
pending uploads, explicit cancellation, changed local sources, read-only refusal,
late identity after acknowledgement, and restart with removed task records.
Export fixtures cover private file/directory modes, escaped descriptors and exact
NIC slots, manifest-validated round trips, normalized snapshots, read-only export,
unsupported/running guests, changed or truncated disks, no-overwrite races and
saved completion proof after publication, and a late package receipt after
acknowledgement/cancellation without final publication. Rune fixtures exercise native open/save,
explicit SR/network mappings, captured pool/source, format changes and demo write
blocking at compact and wide sizes. Final validation passed all 492 workspace tests (237 core unit, 141 core
integration, 110 desktop and four CLI tests), including 26 focused OVF tests and
three new Rune workflows. Strict workspace Clippy, formatting and diff checks
passed. Synthetic compact and wide import/export scenes were rendered with native
Vulkan and inspected; this establishes layout coverage, not real desktop-dialog
or live-host qualification. Peer review identified and resolved controller-parent
validation and late-publication stop handling before this checkpoint.

Integration onto the current main branch also passed `scripts/check.sh`: 563
workspace tests, 46 vendored VNC tests, eight lab-helper tests, strict workspace
Clippy, formatting, build and all synthetic native scene renders. OVF import and
export are now included in that script at 1440 × 900 and 513 × 652. The compact
integration renders were inspected. Existing compact device/snapshot scene
diagnostics about zero-height scroll viewports remain outside this transfer
change; no live-host qualification was performed.

## Primary references

- [DMTF OVF 2.1 specification](https://www.dmtf.org/sites/default/files/standards/documents/DSP0243_2.1.0.pdf)
- [XAPI VM API](https://xapi-project.github.io/new-docs/xen-api/classes/vm/index.html)
- [XAPI raw-VDI import](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/import_raw_vdi.ml)
- [XAPI raw-VDI export](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/export_raw_vdi.ml)
- Local XenAdmin reference: `XenModel/Actions/OvfActions/Import.cs`. Its workflow was
  studied; upstream implementation source was not copied.
