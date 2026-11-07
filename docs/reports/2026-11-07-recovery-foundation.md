# Recovery foundation

The operator reported successful direct inventory loading and console display. This is
user-reported evidence, not an agent-run host qualification. No screenshots, private host
identifiers or console contents are included. No live host requests were made for these fixes.

The supplied review notes were checked against the source. A journal test reproduced stale
recovery terminalizing a newer dispatch before the fix. Conditional journal revisions and
checked affected-row counts now prevent that interleaving. A dispatch permit is returned
only after its durable marker succeeds. Late task references merge into the current row.
Legacy rows migrate without dropping task references or unresolved reservations.

Recovery inspection completes before mutation admission and returns records directly, so
large history does not fill an unconsumed bounded channel. Known task read failures retry
without replaying their mutation. Task progress no longer restarts the inventory long-poll.
Established-session coordinator redirects trigger fresh login and resynchronization.

Cancellation is a durable request routed by operation ID, and survives tracker replacement.
The UI selects the newest journal revision and keeps unresolved operations visible before
terminal history. Unknown outcomes have an explicit recheck/review flow. Acknowledgement
releases the local reservation without claiming success or server cancellation.

The original member certificate pin survives coordinator redirection and a console return.
Editing the host endpoint clears the remembered certificate-validation bypass. The bypass
remains available as an explicit per-connection choice. Read-only probe connections do not
open the operation journal or resume mutation/cancellation work.

Resume, unpause, pause and suspend join the power actions with host capability checks.
Failure displays a safe XAPI error code, and terminal task records are cleaned up after
their outcome is durable. Memory labels explicitly describe the static maximum. Force
review repeats the selected VM name and UUID.

The local check script passes all **41 tests**, formatting, Clippy with warnings denied,
workspace builds and four synthetic Rune render exports. Added fixtures cover recovery
ordering, late evidence, old-schema migration, more than 128 recovery records, transient
task reads, long-poll continuity, recovered cancellation, coordinator change, power recovery,
terminal-write failure and read-only admission. No live mutation is claimed by these results.

[Ubuntu 24.04 CI](https://github.com/z23/xenadmin-rs/actions/runs/42370080881) passed the same
checks for source `4f18f6aa8afc3e91553a27fdd31dae524085cab9`. The local 0.1.2 archive is
`artifacts/xenadmin-rs-0.1.2-linux-x86_64.tar.gz`, SHA-256
`ae0ead2ed649e1dfb367a96c46812a845f640c14262598f6b2914915806dbd21`.

The whole-batch inventory rejection remains intentional: silently dropping a malformed
required record would break the coherent-inventory contract. More schema qualification,
endpoint failover, full console input coverage and performance measurements remain open.
