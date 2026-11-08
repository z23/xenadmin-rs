# Snapshot ancestry, storage and saved previews

Implemented in the 0.10 development source. Live-host qualification remains deferred for the prepared disposable lab.

The snapshot page now defaults to a stable ancestry tree and can switch to the chronological list without losing selection. It marks the VM's current snapshot base. Missing or foreign parents and cycles remain visible with an explanation; every snapshot appears once. Parent links use typed UUIDs and the graph walk is iterative.

Inspect snapshot opens a captured, independently scrollable view of the original pool, VM and snapshot. It lists distinct attached disks, storage repositories, virtual capacity, reported allocation, VDI parentage and disk flags, plus the checkpoint memory disk when resolved. Missing classes/records remain explicit. Reported allocation is not described as exclusive usage or reclaimable space: shared blocks and storage snapshot chains prevent that inference.

Load saved preview reads an existing XenCenter snapshot JPEG/PNG blob or its legacy inline base64 image. It does not create or upload images. The query checks fresh pool, VM, snapshot and parent identities, uses the existing connection's certificate/proxy policy and a session cookie on the fixed blob endpoint, refuses redirects, and rechecks blob metadata and the snapshot's image association after the read. The query permits read-only connections and is fenced by connection generation; closing its view cancels pending reads. Errors omit request URLs and credentials.

Preview images stay in memory, outside the operation journal and inspector JSON. The private image type's Debug output contains only type and byte count. Compressed payloads are limited to 8 MiB; the query has a 30-second deadline and download a 20-second timeout. A shared decoder permit remains held until decoding exits, including after a view closes; repeated open/load/close cannot accumulate decoder threads. A background decoder limits dimensions to 4096 pixels per side and 4,194,304 pixels total, with an explicit decoder allocation budget. The native window receives only the resulting bounded RGBA image. Selection/workspace changes cannot retarget an outstanding read or apply an old result to a new panel.

The preview is a saved image, not a live console or a guarantee of the guest's state at the exact disk snapshot instant. Capturing/uploading a new image when taking a snapshot remains a follow-up. No user screenshots or host inventories were used in fixtures or committed.

## Evidence

- Full workspace sweep: 749 tests passed on the source base (342 core unit, 202 direct protocol, 192 Rune, 5 profiles, 4 TLS and 4 probe tests).
- Two additional direct-protocol facade tests passed after that sweep: read-only access/foreign-generation refusal and cancellation when the connection retires.
- New checks cover branch/current-base resolution, unavailable parents/cycles, checkpoint disk references, UUID-only public metadata, blob and inline formats, changed parent/blob/metadata, oversized/truncated/redirected downloads, query cancellation, PNG/JPEG decoding and excessive dimensions, captured UI identity and tree/list selection.
- A further decoder-slot regression and all five focused snapshot Rune tests passed after peer review caught a possible accumulation of decoders when closing/reopening the panel. The slot now survives panel closure until the worker exits.
- Strict workspace/all-target Clippy, formatting and diff checks passed after that fix; the desktop build and native renders preceded the slot-only change.
- Independent source review found no remaining blocker after rechecking the decoder permit.
- Four native synthetic renders at 1440×900 and 513×652 inspected: ancestry page and information dialog. These do not establish real Wayland input, accessibility, or live-host image compatibility.

## Protocol evidence

The pinned [XAPI blob handler](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_blob.ml) accepts a UUID selector, and its [HTTP authentication layer](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_http.ml) accepts the session cookie. The reference client's `SnapshotsPage.cs` and `VMSnapshotCreateAction.cs` establish the `XenCenter.VNCSnapshot` association and legacy inline fallback. This implementation reads those formats without copying the reference client source.
