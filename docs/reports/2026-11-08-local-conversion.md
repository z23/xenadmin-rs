# Offline local disk conversion

This increment adds an offline local conversion manager. A selected self-contained
QCOW2, monolithicSparse/streamOptimized VMDK, VHDX, VHD or raw image becomes a new
raw or dynamic VHD file. Importing that file is a separate existing reviewed XAPI
workflow. Conversion does not establish guest boot, firmware or driver compatibility.

Every QEMU image parser runs inside the same mandatory Bubblewrap sandbox,
including metadata inspection and final comparison. The sandbox has a new network,
PID, IPC and user namespace, a clear environment, no desktop sockets or home
mounts, read-only trusted runtime/tool mounts, a read-only staged input and only
an operation-owned output directory. Missing tools or sandbox support refuse the
operation; there is no unsandboxed fallback. Commands use typed argument vectors,
never a shell or user-supplied image options. No repair, rebase, force sharing or
salvage mode is offered.

The source is copied into a new private inode and hashed while its original file
identity is checked. A staged reflink must never be a hardlink. Source staging stays
read-only throughout inspection, conversion and comparison. Only a complete,
self-contained graph is supported initially: backing files, external QCOW2 data
files, multi-file VMDK extents, differencing formats and encryption are refused.
Metadata cannot authorize an extra host mount. Explicit backing-chain selection
and multi-file/appliance remapping remain retained follow-up work.

One worker owns a conversion at a time. Inspection output and diagnostics, memory,
CPU time, wall time, output size and conversion concurrency are bounded. Cancellation
kills and reaps the isolated worker. A child-only `close_range(CLOEXEC)` adapter
closes inherited descriptors above stderr before exec; the stdin policy pipe remains
available. This is isolated in `xenadmin-process`, preserving core's unsafe-code ban.
The sandbox also denies process creation while permitting QEMU worker threads.
Crash recovery never resumes conversion.
Private history captures owned file identity and paths; public summaries contain
formats, sizes and outcomes. Only exact operation-owned temporary files are eligible
for cleanup. Existing destination files are never overwritten. Successful output
must pass the existing raw/VHD validator, exact virtual-size checks and sandboxed
content comparison before fsync and atomic no-replace publication.

## Source evidence

- [QEMU disk image utility](https://www.qemu.org/docs/master/tools/qemu-img.html)
  documents offline operation, explicit input/output formats, JSON inspection,
  conversion and content comparison, and warns about concurrently modified images.
- Pinned [QEMU v10.1.2 qemu-img.c](https://github.com/qemu/qemu/blob/v10.1.2/qemu-img.c)
  uses `BDRV_O_NO_BACKING` for ordinary image inspection. This is not permission
  to ignore backing data during conversion.
- [QAPI block metadata](https://github.com/qemu/qemu/blob/v10.1.2/qapi/block-core.json)
  exposes backing filenames, QCOW2 external data files and encryption, and VMDK
  parent/extent metadata. [VHDX support](https://github.com/qemu/qemu/blob/v10.1.2/block/vhdx.c)
  explicitly refuses parent/differencing images in this pinned implementation.
- [Bubblewrap](https://github.com/containers/bubblewrap/blob/v0.11.0/README.md)
  provides filesystem/namespaces primitives; the invoking application is responsible
  for the actual policy. Its presence alone does not establish a safe sandbox.
- The pinned XenAdmin `35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`
  `XenAdmin/Wizards/ConversionWizard/ConversionWizard.cs` uses a separate
  `ConversionClient` for VMware server/VM mappings. Local image conversion is not
  a claim of that remote service's parity; its audited scope remains separate.

## Using the workflow

Open **Convert local disk…**, select the trusted local QEMU executable, explicit input
format, source file and a new destination. **Inspect and review** stages a private copy
and checks tool capabilities before showing the immutable review. **Convert reviewed
image** performs conversion and comparison. A successful result offers **Review disk
import…**, which opens the existing import workflow with a fresh storage/pool review.
It never automatically uploads, attaches or starts a VM.

Closing a prepared review discards its owned temporary files. Hiding active work leaves
it running; **Cancel local work** stops it. Interrupted work is shown in local history
and can only receive a separately reviewed cleanup of recorded temporary identities.
History has explicit pages and refuses an over-limit directory instead of silently
omitting jobs. Published outputs and unrecognized files are retained.

The application installs no tools. Linux x86-64, working user namespaces, Bubblewrap,
util-linux `prlimit`, a kernel supporting `close_range(CLOEXEC)`, and a trusted QEMU 8+
with the requested formats are required. Capability checks include exact-size dynamic
VHD output options. This qualification used QEMU 10.1.2 built under the project-local
`.xenadmin-tools` directory, Bubblewrap 0.13.1 and util-linux 2.42.4. The downloaded
QEMU source archive SHA-256 was
`9d75f331c1a5cb9b6eb8fd9f64f563ec2eab346c822cb97f8b35cd82d3f11479`.
This records the tested source; it is not a signature-verification claim.

## Limits and verification

Inputs are regular files, at most 8 TiB; virtual output size is limited by the existing
raw/VHD importer. Each image-tool process has a 2 GiB address-space limit, 64 descriptors,
no core dumps, 1 MiB stdout/stderr bounds, and a 30-second inspection or four-hour
conversion/comparison deadline. Conversion has two QEMU workers and a 128 MiB/s rate cap.
Staging and hashing check cancellation and a four-hour deadline between bounded reads;
a kernel-blocked filesystem operation cannot be forcibly interrupted by that loop.
Reflinks are distinct owned inodes; the fallback preserves sparse zero regions.

The descriptor canary initially demonstrated that Bubblewrap intentionally forwards
inherited descriptors to its payload. Its source closes extra descriptors only in the
namespace reaper, not the payload. The new adapter is tested with a deliberately
non-CLOEXEC descriptor above 100. A gated payload also proves cancellation terminates
the new-session process, closes its pipes and releases an acquired file lock. The
canary uses only synthetic local files. No parent descriptor flags are changed.

Nine focused core suites cover metadata/format refusal, source immutability, private
journal binding, no-overwrite publication, exclusive worker ownership, history bounds,
recovery cleanup, cancellation, environment/filesystem/descriptor isolation and real
QEMU byte-equivalent conversions. Five require a real namespace sandbox and trusted
local tools and are explicitly run with `--include-ignored`; they are not silently
counted by the default suite. Four Rune groups cover compact/wide control reachability,
file-dialog cancellation/stale callbacks, busy target capture, cancellation and the
separate import handoff. The complete workspace passed 1,156 tests: 509 core unit, 311 direct protocol, 323
desktop, five profile, four TLS and four host-probe tests; five environment-dependent
conversion suites were ignored by that default run and then explicitly passed in
the nine-suite conversion qualification. Those real-QEMU cases cover fixed and
dynamic VHD, QCOW2, monolithicSparse and streamOptimized VMDK, and VHDX. Strict
workspace/all-target Clippy, formatting and whitespace checks passed. Four synthetic
native Vulkan renders at 1440×900 and 513×652 were visually inspected; both input
and immutable-review headings/footers remain reachable. Rendering does not qualify
real native file dialogs, accessibility or guest behavior.

No production host was contacted or changed. Real guest boot/firmware compatibility,
large-image performance, additional QEMU versions, backing chains, multi-file VMDKs,
internal snapshot selection and appliance remapping remain unqualified or retained.
The existing VMware remote conversion service is a distinct audited scope; this local
workflow does not claim to replace it.



Reproduction (with a trusted local QEMU path selected explicitly):

```sh
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
XENADMIN_TEST_QEMU=/absolute/trusted/qemu-img cargo test -p xenadmin-core local_conversion -- --include-ignored
```

`XENADMIN_TEST_QEMU` is used only by the explicitly opted-in synthetic tests. The
application captures its executable choice in the reviewed workflow and does not
read that test variable. No binaries, source archives or synthetic render files are
committed with this increment.
