# XVA compression

Reviewed XVA export now offers uncompressed, gzip and zstd output for ordinary guests,
custom templates, disk snapshots and memory checkpoints. Import detects those formats
from the file header, independently of its filename. This increment is implemented and
fixture verified; host compatibility remains a disposable-lab qualification gate.

## Behavior and evidence

The export choice is part of the immutable action. Changing it invalidates the selected
output and any outstanding file-picker response. Review displays the selected format;
the picker proposes `.xva`, `.xva.gz` or `.xva.zst`. OVF/OVA keep their existing packaging.
The client sends the exact `use_compression` value and refuses to publish a response in
another format. It does not silently downgrade a request when a host ignores compression.

All exports pass complete decompressed tar, metadata-identity, sparse disk/memory chunk,
checksum and archive-termination validation before atomic publication. Compressed frames
must finish correctly, including trailers and any concatenated frames. An appended second
archive is refused. The HTTP layer accepts small compressed archives while retaining its
separate minimum for disk-image transfers; the decoded archive checks remain authoritative.

Ordinary import inspects bounded metadata before review and streams the original compressed
bytes to the host. It does not claim to prevalidate the complete ordinary guest payload.
Memory-checkpoint selection and pre-dispatch validation inspect the full archive, preserving
the existing explicit RAM review, new-identity checks and suspended result. No start or resume
is sent. A lost upload reply retains partial evidence, and recheck never uploads again.

Decoder input reads are capped at 64 KiB and check cancellation, including when a compressed
stream consists of many empty frames. zstd history is limited to 128 MiB; larger custom
windows and external dictionaries are unsupported. Metadata is limited to 8 MiB with a
16 MiB compressed-input inspection bound. Existing object-count, disk-size and complete-stream
bounds also apply. Decoding and file work remain off the UI thread.

Old serialized export actions default to uncompressed. Old archive summaries have an unknown
compression value until inspected again; no format is invented for historical records.

## Sources and verification

The pinned [XAPI export handler](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/export.ml)
accepts `gzip` and `zstd` for `use_compression`. Its
[import handler](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/import.ml)
detects zstd magic and otherwise supports gzip decompression. The pinned Rust
[zstd decoder](https://docs.rs/zstd/0.14.3/zstd/stream/read/struct.Decoder.html)
provides the history-window limit and concatenated-frame decoding; the dependency is built
without its optional default features. The client uses no external compression command.

Focused regression groups cover both compressed formats, plain compatibility, corrupt and
truncated frames, appended archives, checksum failures, oversized history windows, cancellation,
ordinary and checkpoint import provenance, lost checkpoint-upload responses, exact export
query/format matching, and refused publication without a second download. Rune checks cover
both window sizes, immutable review choices, picker invalidation and OVF/OVA reset.

The workspace run passed 1,099 tests (490 core unit, 289 direct protocol, 307 Rune/desktop and
13 profile/TLS/probe tests). A final additional compressed-checkpoint lost-response case passed
alongside all six other compression groups. Strict workspace/all-target Clippy, formatting,
diff checks and the desktop build passed. Native Vulkan renders at 513×652 and 1440×900
were visually inspected; format choices, explanation and file/review controls are reachable.
These synthetic renders do not qualify a compositor or actual host behavior. Focused peer source
review found no blocker in decoder bounds, publication, no-replay behavior or persisted defaults.
No production mutation, credential, private inventory or user screenshot was used.
