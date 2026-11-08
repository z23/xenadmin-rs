# NoCloud configuration drives — 2026-11-08

A halted ordinary VM can now receive a reviewed cloud-init drive directly through XAPI.
The workflow accepts a guest hostname, UTF-8 user-data and optional network-config, requires
a storage repository and unused disk slot, prepares private local media in the background,
then reviews the exact VM/pool, storage UUID, slot and configuration ID before submission.
The VM remains halted. The drive stays attached and contains user-data; its later removal
uses the existing reviewed device workflow.

## Implemented boundary

The image is an 8 MiB FAT16 filesystem labelled `CIDATA`, with `meta-data`, `user-data` and
optional `network-config` in its root. Metadata carries a stable instance ID for that
prepared artifact and the requested hostname. User-data is limited to 2 MiB and network
configuration to 1 MiB. UTF-8 text is retained unchanged; the guest remains responsible
for interpreting scripts, cloud-config or other supported user-data. The application does
not validate cloud-init schemas, install cloud-init, boot the VM, or prove guest execution.
A new configuration ID is a new NoCloud instance identity, so existing cloud-init guests
may run first-boot configuration again when that media is used.

The backend creates one new ordinary user VDI, uploads through `/import_raw_vdi`, then
creates one nonbootable, read-only VBD. These steps share one durable pool reservation.
No existing disk is overwritten, replaced or deleted. Known app-created seed drives are
refused until separately removed; external seed media can affect guest selection and is
called out in review. The new disk and attachment UUIDs appear in Activity, inspection and
public history export. The creation wizard itself does not yet embed cloud-init inputs;
newly created halted guests use this separate workflow.

Fresh pool, VM UUID/kind/power state/current operations, SR identity, server-advertised
available disk slot, and new disk provenance are checked before relevant writes. Read-only
connections, stale observations and media prepared for another VM are refused. Verification
requires the exact new VM/VDI/VBD graph, ownership marker, chosen slot and RO/nonbootable
settings. It establishes the completed upload task and attachment configuration, not a
server-side disk hash or guest provisioning outcome.

## Confidentiality and interruption handling

Runtime inputs and prepared media have redacted Debug implementations. Core user-data,
network-config and FAT image buffers use zeroizing storage. The local artifact uses a new
0700 directory and 0600 file. The private journal contains its path, file identity and
SHA-256 plus reconciliation references, never inline user-data. Public record serialization,
Activity exports and inspection exclude the private plan, paths, hashes and transport refs.
Unused/cancelled local drafts are removed; rejected operations without disk creation and
successfully attached media remove their unchanged local artifact. Partial or uncertain
operations can retain a private local artifact for inspection.

Hash, artifact identity, baseline UUIDs and received references are immutable; upload and
attachment evidence is monotonic. A recovery latch prevents later attachment even if a
transfer task subsequently completes. Recheck reads existing task/resource evidence and
never uploads or creates an attachment. A proven upload interrupted before attachment is
reported as a retained unattached disk. Lost attachment replies can resolve by bounded
new-UUID/marker discovery and exact graph verification. Late upload/attachment receipts
retain recovered or acknowledged states and cannot resume subsequent writes. Acknowledging
an unknown outcome does not establish completion or remove server resources.

## Verification

- Workspace tests: 621 passed (305 core units/protocol fixtures, 159 direct connection,
  144 desktop, five profiles, four TLS and four host-probe).
- New coverage: three FAT/input/privacy tests, nine upload/attachment/journal protocol tests,
  one facade identity/read-only test and four Rune interaction tests.
- Protocol cases include lost upload/attachment responses, misbound results, changing VM
  state, existing seed refusal, modified local files, pool/generation binding, immutable
  journal evidence, recovery during upload and recovery/acknowledgement during late replies.
- Rune tests cover compact/wide target capture, validation preserving entered data,
  selection/pool changes, synthetic no-write gating and cancelled background preparation.
- `fsck.fat -n` accepted a generated synthetic image; independent `blkid -p` identified
  `CIDATA`, `vfat`, `FAT16`, and 8 MiB. FAT readback tests verify exact root filenames/content.
- Strict workspace Clippy, formatting, whitespace and shell syntax checks passed.
- Four synthetic Vulkan editor/review renders at 1440×900 and 513×652 were visually
  inspected; their scenes are included in `scripts/check.sh`. Compact controls are reachable
  by scrolling. These scenes contain generated fixture data only.

All infrastructure mutations remain fixture-only. XCP-ng/guest qualification, real boot
consumption, Wayland/X11 input/accessibility and distribution packaging for this increment
remain open. No private screenshots, live inventory or credentials were used or published.

## Primary evidence and dependency

[Cloud-init's NoCloud documentation](https://docs.cloud-init.io/en/latest/reference/datasources/nocloud.html)
specifies the labelled VFAT/ISO9660 media and root file names. The implementation uses the
existing direct-XAPI VDI create/import and VBD create path; it needs no XO service, SSH or
host plugin. It does not use the excluded legacy CoreOS plugin integration.

[`fatfs` 0.3.6](https://github.com/rafalh/rust-fatfs/tree/v0.3.6) is pinned with only `std`
and `alloc` features; its MIT license covers the dependency. The
[formatting API](https://docs.rs/fatfs/0.3.6/fatfs/struct.FormatVolumeOptions.html) supplies
FAT16 formatting and the volume label. `Cargo.lock` records the exact dependency graph.
