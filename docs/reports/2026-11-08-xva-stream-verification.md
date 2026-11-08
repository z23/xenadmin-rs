# Complete native XVA export verification

The export workflow now verifies every native XVA disk chunk before publishing the private output file. Previously it required the host task, complete HTTP stream receipt and matching VM/snapshot metadata; those checks did not independently validate payload checksums or disk boundaries.

## Behavior

Validation runs off the UI/runtime thread and accepts plain or gzip archives. It requires the captured metadata digest, regular tar entries with valid headers, known data-disk prefixes, increasing eight-digit chunk positions, exact first/last chunks and lengths, and an immediately following matching XXH64 or SHA-1 checksum for each chunk. Omitted interior chunks represent sparse zeros. ISO SR references remain metadata only. Unknown disks, duplicate/reordered chunks, changed checksums, missing end records, appended data and oversized padding are refused. The parser holds at most bounded metadata and one 1 MiB disk chunk; cancellation is checked between chunks and on every bounded compressed-input read, including scans through empty gzip members.

The existing task/stream/journal publication guard still applies. An invalid export is never published or automatically downloaded again, even when the server task succeeded. This increment does not extend archive import formats or claim checkpoint/ancestry support.

## Primary-source contract

- [XAPI v26.1.21 stream_vdi.ml](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/stream_vdi.ml) defines 1 MiB chunks, sparse omission with mandatory first/last chunks, lexicographic disk ordering, `.xxhash` XXH64 and older `.checksum` SHA-1 records.
- [XAPI v26.1.21 export.ml](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/export.ml) writes metadata followed by disk streams and two tar end blocks. Full export fixes `with_snapshot_metadata` to false; the separate metadata endpoint exposes ancestry but contains no disk backup.
- The small [xxhash-rust 0.8.15 implementation](https://docs.rs/xxhash-rust/0.8.15/xxhash_rust/xxh64/index.html) is pinned with only the XXH64 feature. Existing SHA-1 support is reused. These format checks detect corruption; they do not authenticate an archive.

## Verification

Focused tests cover sparse disks, both checksum formats, gzip, altered/missing payload and checksum records, invalid ordering and bounds, extra data, truncated tar records, metadata mismatch and cancellation. The local HTTP fixture now supplies real checksummed disk content; a deliberately corrupted chunk with a successful host task cannot produce a final file. The isolated workspace passed 983 tests before the final cancellation hardening. The final five archive-stream tests and strict workspace/all-target Clippy passed after that change; formatting and diff checks passed. Peer review identified the compressed-input cancellation gap, which was fixed and covered by a deterministic short-read test.

No live host or private inventory was used. Real XCP-ng export/import round trips remain deferred to the disposable lab. Legacy exports without inline checksums, additional formats and memory-checkpoint handling remain separate work.
