# Shared storage graph repair and multipath maintenance

This increment implements two reviewed direct-XAPI workflows: reconstructing missing
shared-storage host connections, and changing standard host multipathing while disconnecting
and reconnecting the captured attachment set. No production host was contacted or changed.
Live qualification remains deferred to the disposable virtualized lab.

## Implemented behavior

From Storage administration, **Repair missing or detached host connections** captures one
existing PBD as a configuration source and 1–32 selected hosts. Existing detached PBDs are
reused; a missing host/SR pair receives a new PBD before it is plugged. The selected coordinator
runs first. Source and target host/SR UUIDs, attachment states, safe locations and private
configuration fingerprints are checked again before each write. Duplicate/newly appearing
pairs, changed coordinator, unavailable hosts, and unknown/custom host-dependent options
refuse the workflow. Supported source shapes are the existing typed shared NFS, ISO-NFS,
SMB, iSCSI and HBA locations; local repositories are not reconstructed across hosts.

Authenticated sources pass existing Secret UUID references to XAPI's PBD constructor. The
client never fetches Secret values. Bounded pre-existing PBD, task and Secret UUID sets and
the returned exact PBD graph establish provenance. Each password reference must become a
new, distinct Secret UUID with matching nonsecret configuration and no other PBD, SR or
host-power consumer. Unknown or failed creation is not cleaned up automatically. A positively
identified new PBD remains visible even if later configuration verification fails; history
explicitly distinguishes this retained identity from a verified connection. No plug follows
an unverified creation.

From Host administration, **Multipathing with storage reconnection** captures every PBD on
one already-disabled, empty maintenance host (maximum 64). HA must be off and the pool must
have no operation in progress; a statefile that XCP-ng leaves listed after HA is disabled
does not count (lab finding 32). The backend checks the complete host/PBD/SR/VDI/VBD/VM graph
and configuration fingerprints before the first detach and again through the sequence.
Active/reserved consumers of affected iSCSI or HBA storage anywhere in the pool refuse
maintenance. Metadata-replication repositories are refused. The host's local-cache
repository (XCP-ng's thin-provisioned local storage by default) is reviewed as kept attached
rather than refused since 2026-11-10 (lab finding 33): multipathing only affects block-storage
replugs, and the refusal had blocked every stock XCP-ng host. Only the other originally
attached PBDs are unplugged, the modern `host.set_multipathing` policy and standard `dmp`
handler are changed, and that same subset is plugged again. Originally detached PBDs remain
detached. Other host settings are preserved. On the replug the iSCSI driver persists the
portal list it discovered as `multihomelist`; the configuration hash check accepts an iSCSI
attachment that matches the reviewed configuration without that one key (lab finding 34).

Each mutation has a durable dispatch marker and positive receipt. Async task UUIDs must be
new relative to the baseline and unique within the workflow. Lost replies, task failure,
identity changes and ambiguous postconditions remain unknown. A known later refusal retains
earlier verified results as partial. Stop prevents remaining steps after current work resolves;
it does not cancel storage I/O. Explicit recheck reads only and never submits pending steps,
retries a mutation, cancels a task or performs cleanup. Late receipts and identified resources
merge without changing recovered/acknowledged state. A 30-minute preparation expiry stops
further dispatch; it is not a promise that a server task finishes within that time.

History lists per-host PBD identities, verified states and every step. For a known partial
result, **Restore this connection** opens a separate existing-attachment review against the
same pool/host/SR/PBD and only while the PBD is currently detached. It cannot turn into a
detach if state changes. Unknown outcomes must first be inspected/resolved. No automatic
rollback, unowned Secret deletion, PBD deletion or data deletion is introduced.

## Source evidence

- [Pinned XAPI PBD implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_pbd.ml): construction rejects an existing host/SR pair and creates a detached PBD; plug/unplug drive storage attachment and enforce HA/consumer constraints. Constructor-supplied `other_config` is not an ownership marker.
- [Pinned XAPI Secret implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_secret.ml): `duplicate_passwds` creates new Secrets for password-reference keys inside the server, so the client need not read clear values.
- [Pinned host implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_host.ml): `set_multipathing` updates host policy and configuration; attachment orchestration is a separate client responsibility.
- [XCP-ng multipathing guidance](https://docs.xcp-ng.org/storage/multipathing/): active iSCSI/HBA I/O and storage equipment configuration constrain changes. This client does not modify array zoning, switches or `multipath.conf`.
- Reference-only comparison: local XenAdmin `SrRepairAction.cs` reconstructs missing shared PBDs and orders the coordinator first; `EditMultipathAction.cs` captures attached PBDs and reconnects them after changing policy. This implementation journals each step and deliberately avoids compensation following uncertain outcomes.

## Verification and remaining boundaries

Thirteen synthetic protocol tests cover shared-PBD creation with/without credentials,
existing-PBD reuse/coordinator ordering, source and graph changes, unshared/custom refusals,
pre-existing or wrong returned identities, reused/cloned credential consumers, task UUID
reuse/failure/loss, late receipt and late identity after recovery/acknowledgement, both
multipath directions, originally detached preservation, HA/resident/active-storage refusal,
missing/duplicate/rebound disk graphs, and malformed private-plan admission/save.
Seven Rune tests exercise exact target capture, compact/wide reachable controls, selection,
changed pool/attachment state, unknown handler/maintenance/HA refusal, synthetic no-write
behavior and separately reviewed partial restoration. Final isolated workspace validation passed **1,023 tests** (458 core unit,
278 direct-connection, 274 desktop, 5 profile, 4 TLS and 4 host-probe tests). Strict
workspace/all-target Clippy, formatting, diff whitespace checks and the native desktop
build passed. Four synthetic Vulkan renders at 1440×900 and 513×652 were visually inspected:
headings and confirmation controls stay visible and the body scrolls. No screenshot is
committed. The independent source review identified the retained-identity and reverse-graph
gaps; both were corrected with regression coverage.

Custom or host-dependent device configurations remain an explicitly incomplete reconstruction
scope; unsupported options are not silently copied. The server remains authoritative for
permissions, driver availability, host failures and attachment success. A policy readback or
`currently_attached=true` does not establish usable I/O, path redundancy, storage health or
array compatibility. No live paths, authenticated-array credentials, multipath failover,
Wayland interactions or hardware were qualified by these fixtures.
