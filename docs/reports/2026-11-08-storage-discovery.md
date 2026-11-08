# Guided storage discovery

The storage administration form now offers NFS export discovery and staged iSCSI
portal → target IQN → LUN → existing-repository discovery. Each request uses a selected
host through the application's existing direct XAPI connection. Selecting a result fills
the form; it never creates or attaches storage. The existing separate provisioning review
still explains that initializing an iSCSI LUN can erase data.

The iSCSI port is explicit throughout discovery, creation, attachment and review. Saved
operations from before this increment default to 3260, and verification treats an absent
legacy PBD port as 3260 only. Other ports must match exactly. Discovered alternate portals
are displayed and require selection; the client does not silently follow or scan them.

Results are tied to the captured pool observation, host and exact form inputs. Changes to
those inputs or the connection invalidate result selection. Pending queries are cancelled
on handle drop or generation change and have a 45-second deadline. The UI shows 20 rows at
a time. Guided enumeration results are limited to 512 KiB, 16,000 XML nodes, depth eight and 1,000
rows. XML entities, mixed/split identifier text, ambiguous duplicate fields, invalid ports
and invalid identifiers are rejected. Only typed export paths/access descriptions,
IQN/portal/port and LUN identity/capacity/vendor/serial fields reach the desktop. Unknown
LUN number or capacity stays unknown.

## Protocol evidence

The [NFS driver](https://github.com/xapi-project/sm/blob/master/libs/sm/drivers/NFSSR.py)
uses a missing `serverpath` probe to enumerate exports; its
[NFS helper](https://github.com/xapi-project/sm/blob/master/libs/sm/nfs.py) emits the
`nfs-exports` schema. NFSv4 pseudofs children can be relative names and are presented as
root paths. The [iSCSI base driver](https://github.com/xapi-project/sm/blob/master/libs/sm/BaseISCSI.py)
emits target IQNs with advertised portals and ports; the
[LVM-iSCSI driver](https://github.com/xapi-project/sm/blob/master/libs/sm/drivers/LVHDoISCSISR.py)
emits LUN records when the SCSI identifier is omitted. The reference client actions
`ISCSIPopulateIQNsAction.cs` and `ISCSIPopulateLunsAction.cs` confirm these schemas and
missing-parameter codes. No upstream implementation was copied.

The private XAPI adapter recognizes only the expected missing-parameter error for each
query (101 for NFS exports, 96 for IQNs, 107 for LUNs), extracts its bounded XML stderr and
parses the expected schema. Ordinary errors retain their sanitized code; arbitrary error
payloads are never exposed in errors, logs or public history. Successful XML responses
are also accepted when their schema matches the requested stage. No create-based
fallback is used. Discovery requires the backend's advertised `SR_PROBE` capability.

A probe is not entirely free of host effects: drivers may temporarily mount an export or
open an iSCSI session while inspecting it. The UI states this. The client submits no
repository creation, task creation or attachment mutation during discovery. General
storage handling is described in the [storage lifecycle report](2026-11-08-storage-lifecycle.md).

## Verification and limits

Synthetic parser and loopback fixtures cover expected error payloads, successful result
schemas, malformed data, error sanitization, custom ports, old serialized intents, and
absence of provisioning calls. Existing generation-cancellation and read-only fixtures
remain in place. Rune interaction fixtures exercise result selection and review at
513×652 and 1440×900, input changes, invalid ports and retained destructive review.
The full workspace rerun passed 541 tests (249 core unit, 153 direct protocol, 126 desktop,
five profiles, four TLS and four host-probe). Strict workspace/all-target Clippy,
formatting, diff and shell syntax checks passed. After the final result-page layout change,
all four storage Rune tests and strict Clippy passed again. Native Vulkan renders of the
result page at both sizes were inspected; the target, advertised portal/port, selection and
back controls are visible. This does not qualify native keyboard/accessibility or a real
storage backend. Synthetic images remain local under `/tmp/xenadmin-storage-discovery-*`.

Integration with the OVF and search increments passed all 582 workspace tests (279 core
unit, 153 direct protocol, 137 desktop, five profiles, four TLS and four host-probe),
strict workspace/all-target Clippy, formatting, diff and shell checks.

The initial full run also exposed an existing transfer worker cancellation/CAS race: a
worker could exit while its durable state still said Tracking. Its focused rerun and the
complete rerun passed with unchanged assertions. That shared worker correction is tracked
separately; this storage discovery patch does not claim to fix it.

CHAP, SMB credentials, additional storage backends and automatic multi-host repair remain
retained work. They require a separate durable secret lifecycle; this increment accepts
no storage passwords. Authentication failures and unsupported legacy response formats
remain visible errors. No real storage target or production host was contacted; backend
compatibility and storage effects require the prepared disposable lab.
