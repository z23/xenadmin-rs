# Pool, host, storage and internal-network administration

This increment adds reviewed pool/host/SR/network name and description edits, default SR
selection, host scheduling enable/disable, asynchronous SR rescan, internal network creation,
and unused internal-network MTU changes/deletion. The desktop presents the exact target,
pool and requested values before submitting. Host scheduling controls do not evacuate or
power off guests. Network changes refuse interfaces that appear after review.

The backend uses typed pool-scoped observations and a revisioned resource journal beside
the VM journal. Admission locks both operation families against conflicting pool changes.
Each dispatch is persisted first. Lost replies and post-write verification failures retain
an unknown outcome; a later read can reconcile it without replaying writes. Metadata edits
can explicitly finish partially. Creation correlates a new network with its operation tag
and bounded pre-existing UUID baseline. Deleted targets remain visible in pool history.

The local inspector includes safe resource-operation summaries without private RPC/task
references. This continues using the running client's existing authenticated connections.

Local protocol fixtures cover successful commands, fresh conflicts, interface membership,
lost setter/create replies, wrong creation identity, partial edits, cancellation boundaries,
reconnect task recovery, read-only access, alias routing and generation fences. Journal tests
cover migration, cross-family admission and races involving cancellation and acknowledgement.
The inspector has a projection test for private evidence. Four Rune interaction tests cover
reviewed values, stable targets, disabled used-network actions, offline history and compact
controls. Synthetic network and pool dialogs were rendered at 513×652 and 1440×900 and
visually inspected. Headless scenes do not qualify native accessibility or desktop input.

No live host mutation was performed. The local lab handoff is prepared; virtualization and
live acceptance remain pending at the user's request. External/VLAN networks, bonds,
management addressing, SR creation/attachment, evacuation and full maintenance remain open
in the [retained parity ledger](../PARITY-IMPLEMENTATION.md).
