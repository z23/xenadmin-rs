# Management on existing VLANs and bonds

The management editor now handles physical interfaces, existing bond masters and
VLANs over physical interfaces or bonds. Operators can move management to a different,
already configured static endpoint, or edit the current endpoint's static/automatic
IP settings. This extends the [management handover](2026-11-08-management-network.md)
and [address transitions](2026-11-08-management-address-transitions.md) workflows.
It does not create, destroy or retag topology.

## Review and execution

The review captures the complete connected VLAN/bond component: exact PIF, network,
host, bond and VLAN UUIDs, devices/MACs, membership, primary bond member, mode/hash,
VLAN tags, IP settings, attachment, MTU, purpose and protection. The UI lists the bond
members and VLAN relationships alongside the source/destination and recovery address.
Link counters and metric identifiers are excluded so a routine counter update does
not invalidate a review.

Before dispatch, bounded fresh XAPI reads validate both directions of the relationships
and reject duplicate or unresolved references. These checks precede the normal typed
inventory projection, which otherwise tolerates incomplete optional classes. The fresh
component must match the reviewed one. Endpoint IP settings and management flags are
checked separately using the same raw PIF observations; ancestor settings remain exact.
The same graph verification runs after completion and through the authenticated recovery
connection. A change after dispatch remains unresolved rather than claiming success.

Existing journal records without a topology capture keep the original strict physical
interface restrictions. New captures use the existing single-task worker, durable task
receipts, interruption fence, pool reservation and host-bound TLS recovery. Recheck does
not repeat configuration or perform rollback. This increment creates no network resources.

## Eligibility and remaining work

The connected component must fit the bounded capture (256 PIFs, 16 bonds and 128 VLANs),
have complete readable metadata, and use managed MTU-1500 PIFs and networks. HA must
be disabled. Bond members cannot be management endpoints. Tunnels, SR-IOV relationships
and nested VLANs are outside this workflow; their retained network work remains open.
The destination must already be attached and statically configured with a different
address and network. Old addressing remains intact after a move.

Creating/destroying a management bond and moving the same address between interfaces
remain separate pending workflows. XAPI's
[bond implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_bond.ml)
can move IP configuration, replace VLAN/PIF records and relocate local VIFs. The reviewed
8.3 package source's configuration-copy helper covers IPv4 fields; IPv6 creation/deletion
handover needs a separately qualified sequence. This does not prevent moves between
already configured IPv6 endpoints. The
[PIF management eligibility check](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_pif.ml)
accepts configured VLAN and bond-master endpoints while rejecting bond members.
No upstream source was copied.

## Verification

Synthetic protocol cases exercise physical↔VLAN/bond moves, VLAN over bond, moves
between VLANs on the same bond, static/dynamic settings, unchanged ancestor configuration,
stale or malformed graphs, missing reverse references, unsupported topology, changed
graph after completion and restart with a removed task. Rune cases cover compact/wide
reviews, exact member/tag capture, stale protection/identity/class data, counter updates,
bond-member refusal and demo non-dispatch.

The complete workspace passed 868 tests: 380 core unit, 247 protocol, 228 Rune,
five profile, four TLS and four probe tests. After replacing repeated reverse-reference
scans with indexes and boxing the optional capture, the focused management suite passed
again (24 protocol and 10 Rune cases, plus related core checks). Strict workspace Clippy
with `-D warnings`, formatting and diff checks passed. A peer source review found no
blocking issue. Native Vulkan renders of `management-vlan` were inspected at 1440×900
and 513×652; the compact review scrolls with its permissions and action footer visible.
No production host calls or private screenshots were used; live compatibility remains
unqualified.
