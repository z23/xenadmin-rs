# Standalone disk-image transfers

This increment adds reviewed raw and VHD import/export through direct XAPI connections. It is implemented and tested with synthetic images and loopback servers; no live host was contacted or changed.

## Delivered workflows

- Import a complete raw, fixed VHD or dynamic VHD image into a **new unattached data disk** on a selected attached user SR. Native file selection captures file identity; review shows the new name, chosen storage, virtual capacity, file length and format. It never overwrites an existing VDI.
- Export an ordinary managed data disk as raw or VHD from its VM device card or the image-transfer disk picker. Selection captures disk UUID, SR UUID and capacity. All consumers must be halted and detached from the running device; live writable disks and memory-checkpoint consumers are refused. Snapshot disks can be selected where their current consumer graph meets these checks.
- Read-only clients may export with server permission for the owned transfer task. Import still requires a writable connection.
- New disk UUIDs appear in transfer/Activity history even when image upload fails, is cancelled or remains uncertain. Recheck reads existing task and ownership evidence; it never resends bytes or repeats disk creation. Cleanup is always explicit; this increment does not add standalone orphan-disk destruction.

## Transfer and ownership boundary

The HTTP boundary adds only fixed `GET /export_raw_vdi` and `PUT /import_raw_vdi` routes with typed format and captured disk parameters. Authentication remains transient Cookie data; the existing TLS trust policy applies. Export redirects are confined to fresh known pool hosts and the same fixed operation. Upload chooses an attached storage host first and refuses redirects. Upload explicitly supplies Content-Length, as the VHD importer does not accept HTTP chunked encoding.

Before creating a disk, the journal stores a bounded fresh VDI UUID baseline and unique operation marker. A returned or rediscovered VDI must be new, have the exact marker/name/SR, be writable and unattached, and provide at least the reviewed virtual capacity. The marker and UUID are rechecked before upload and after the host task. A misbound/pre-existing result never becomes success. An interrupted creation may retain unknown resources and keeps the pool reservation; positive recovery can report the newly created disk as partial without starting its upload. Failed/cancelled uploads target only that proven new disk and therefore report partial contents with its UUID retained.

Exports use private mode-0600 files, complete stream/hash receipts, format validation, successful task evidence and atomic no-replace publication. The source UUID, SR, capacity and halted consumer state are freshly checked before and after the transfer. Another independent administrator can still change the guest during transfer; use a disk snapshot for a stable point in time. A source file changed during an upload cannot produce a fully verified import even if the HTTP reply is lost and the host task reports success. Raw images have no embedded guest identity or disk-content checksum; success is based on selected source identity, byte count, host task and fresh target identity, not a claimed end-to-end disk hash.

## Format bounds and remaining scope

Raw images must be nonempty, sector aligned and at most 8 TiB. VHD virtual capacity is bounded at 2040 GiB. Footer/header checksums, fixed image length, dynamic block geometry, bounded BAT entry counts, metadata/data boundaries and nonoverlapping allocated blocks are checked before host work. Differencing VHD, external parents, saved-state VHD, VHDX and QCOW2 are refused. The dynamic parser accepts a conservative complete-image subset; it does not silently flatten parents or convert formats.

OVF/OVA, disk format conversion, cross-pool workflows, range/resumable transfer and live qualification remain pending. XVA archive behavior is documented separately in the [XVA transfer report](2026-11-08-xva-transfers.md).

## Verification

Five image-format tests exercise complete raw/fixed/dynamic input, checksums, parent refusal, virtual-size bounds, overlapping/out-of-bounds BAT blocks and changed selected files. Thirteen synthetic HTTP workflows cover raw/fixed/dynamic upload, private export, unchanged pre-existing disks, lost creation/upload replies without replay, wrong/pre-existing result identities, failed/cancelled partial disks, completion after restart and task removal, live/changed export refusal, read-only authorization, a changed source with a lost reply, and a delayed creation reply after unknown-outcome acknowledgement. Late proven UUIDs remain in history without reopening the acknowledged operation or starting its upload. Three additional Rune tests cover captured native import review at compact/wide widths, export identity and stale capacity, and format/name guards; existing XVA tests remain regression coverage. All 349 workspace tests passed before the final late-reply regression was added; the resulting workspace contains 350 tests. Strict workspace Clippy, formatting and diff checks passed. Synthetic compact (513×652) and wide (1440×900) disk-export renders were inspected from `/tmp`; no user screenshots or live inventories were copied.

## Semantic references

Implemented independently without copying upstream implementation. The [XAPI disk transfer interface](https://xapi-project.github.io/xen-api/snapshots.html) documents raw/VHD endpoints and the pre-created VDI contract. The [XAPI raw VDI importer](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/import_raw_vdi.ml) supplies the format, Content-Length and single-VDI task-result semantics; the [exporter](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/export_raw_vdi.ml) supplies the read attachment and size constraints. The [Microsoft VHD specification](https://www.microsoft.com/en-us/download/details.aspx?id=23850) and [libvhdi format documentation](https://github.com/libyal/libvhdi/blob/main/documentation/Virtual%20Hard%20Disk%20%28VHD%29%20image%20format.asciidoc) describe footer, dynamic-header and BAT layout.
