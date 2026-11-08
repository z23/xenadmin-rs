# Snapshot preview capture

This increment completes the capture/upload portion of retained P05-12. Live host and guest
qualification remains deferred to the prepared disposable lab.

## Delivered workflow

Snapshot creation offers an unchecked **Save a console preview with the snapshot** option
for a current running VM. Its explanation states that the image is stored on the pool and
may contain sensitive guest information. The client opens a separate authenticated console,
receives a frame without sending guest input, closes that console, and encodes the frame
before submitting the reviewed snapshot/checkpoint command. Capture failure stops before
creation; the operator can retry or turn off the option.

The image waits in memory for that exact creation operation. Upload requires its verified
success, exact created snapshot, original pool/VM/connection generation and current inventory.
Cancellation takes precedence even if creation subsequently succeeds. Closing the creation
dialog cancels pending capture/encoding; changing the connection prevents later submission.
Once snapshot creation has been submitted, its independent operation remains inspectable in
Activity. Capturing is bounded to 30 seconds; waiting for creation is bounded to 30 minutes,
then at most 30 seconds for its inventory identity. No pending image survives application restart.

Snapshot creation and preview attachment have separate operation results. An upload problem
cannot erase or misreport a successfully created snapshot. This workflow does not attach a
current console image to an arbitrary older snapshot, replace an existing preview, or retry
an uncertain upload automatically.

## Backend and recovery

The upload facade accepts a current `ObservedSnapshot`, a local source operation ID and a
validated private PNG. The source journal must contain completed snapshot-creation task and
result evidence, a new UUID outside its captured VM baseline, verified metadata completion,
and no cancellation request. Source, target and private plan are checked again before dispatch.

Before `VM.create_new_blob`, the client captures up to 16,384 existing blob UUIDs. It refuses
invalid/duplicate baselines, a pre-existing returned UUID, changed association/parent/UUID,
non-PNG or public blobs, and an existing inline or blob preview. The fixed association is
`XenCenter.VNCSnapshot`; the new blob is explicitly private. A positive creation reference
is required before upload. Lost creation replies remain unknown, even if an association later
appears: the client does not adopt that object, create another, or delete an uncertain orphan.

Upload uses the connection's current TLS/proxy policy and a session cookie, with redirects
disabled and no credentials in the URL. The host's early HTTP 200 is insufficient evidence.
Completion requires a bounded download with the exact captured byte length and SHA-256,
plus identity/association checks before and after. Lost upload replies may be resolved by
this read-only check; no upload or missing step is replayed. Journal merges retain late
positive receipts while preserving recovery, cancellation and acknowledgement decisions.
A verified empty placeholder with no submitted upload remains a partial outcome.

Image bytes are excluded from inventories, journals, public operation JSON and debug output.
The private journal retains identities, dispatch evidence and a hash, not pixels. Capture uses
one background encoder at a time and scales to fit 1600×900. The backend validates a complete
single-frame PNG, at most 8 MiB, 4096 per side and 4,194,304 pixels, with bounded decoder
allocation. Owned validation/encoded buffers are cleared when dropped; this is not a claim
that all image-library, transport or shared console memory copies are erased.

## Target source evidence

The reference XenAdmin `VMSnapshotCreateAction.cs` captures before snapshot creation to avoid
console changes during checkpointing, then stores the image through `VM.create_new_blob`.
The implementation here is original Rust and uses the PNG format already supported by the
saved-preview reader.

XCP-ng 8.3's pinned [XAPI v26.1.21 VM implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vm.ml)
allocates a blob and then associates the fixed map key. The
[database map implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/database/db_cache_types.ml)
refuses an existing key with a different value, including under idempotent-map behavior.
A concurrent insertion therefore cannot be silently overwritten by this method; allocation
may already have happened before refusal, so the client retains uncertainty and performs no
automatic cleanup. The [blob HTTP implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_blob.ml)
sends 200 before consuming the PUT body, which is why content readback is mandatory.

## Verification

The final full workspace suite passed **898 tests**: 405 core unit, 247 direct protocol,
233 Rune/desktop, five profile, four TLS and four probe cases. Strict workspace/all-target
Clippy, formatting, diff checks and native desktop build passed. A preceding loaded run
hit an existing home-host reconciliation fixture's timeout; that case passed alone and
the complete rerun passed without changing its behavior or deadline.

Eight new core suites cover valid and early-200 uploads, wrong source/parent, existing
preview and pre-existing returned blob, public blob, read-only/denied access, lost create,
lost upload, recovery/cancellation before PUT, acknowledgement after create/PUT, late
receipts, read-only recheck, redirects, truncation, content mismatch, PNG limits, journal
immutability and public serialization. A direct facade fixture covers write access,
connection generation and missing source evidence. Existing creation tests also verify
the source evidence needed for upload.

Three new desktop tests cover bounded background encoding, cancelled-success/uncertain
creation decisions, explicit opt-in and synthetic non-dispatch at compact/wide sizes.
Two native Vulkan synthetic renders at 1440×900 and 513×652 were inspected: the privacy
explanation, checkbox, description and footer controls are visible with wrapped text.
Independent reviews prompted the pre-existing blob baseline and cancellation precedence;
no further blocking source issue was identified.
Fixtures use only local synthetic servers and images. No production operation, user screenshot,
or private inventory is included. Actual RFB capture timing, host HTTP behavior and snapshot
compatibility still require the disposable lab; synthetic renders do not qualify real input,
Wayland/X11 or accessibility behavior.
