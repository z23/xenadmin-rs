# Management interface and address handover

Host networking now offers reviewed management IPv4/IPv6 settings and movement of host
management to a different, already plugged and statically configured physical interface.
The review captures exact pool/host/PIF/network identities, the installed address family,
current settings and the recovery address. The old interface retains its address after a
move; clearing it is a separate reviewed secondary-interface operation.

The implemented subset requires HA disabled, ordinary managed physical interfaces at
MTU 1500, static addressing in the installed primary family, no bond/VLAN/tunnel children,
and no separate storage/service purpose. The destination of a move must use a different
network and address. Fresh reads reject changed settings, ambiguous management membership
or an address already configured on another pool interface. Known API permission denial
is checked before dispatch; the server remains the authority for access checks.

A durable intent and bounded existing-task baseline precede the single asynchronous XAPI
request. The source actor records the exact recovery candidate before dispatch under its
current generation and pool. This adds a fallback candidate, not a trusted certificate or
an immediately verified pool member. Pending candidates can be loaded from the private
journal when reconnecting with a known expected pool. No session, password or trust override
is persisted in the management plan.

Successful task completion is saved before state verification. Public history separates
**configuration observed** from **recovery endpoint verified**. The latter requires a
separate authenticated login at the recovery address and exact pool/host/configuration
readback. The target host's certificate must be unique among the bounded current pool host
set; this is checked before dispatch and again around recovery verification. Its leaf
fingerprint is an additional TLS handshake constraint on the actual first login connection,
before credentials or coordinator redirection. The existing CA/pin/validation-opt-out and
proxy policy remains in effect. A first-hop identity check is not replaced by a successful
login to a redirected coordinator, and pins are not copied to the new address.

A lost reply, failed task, missing completion proof or unverified route remains uncertain
and holds the local pool reservation. Recheck reads existing evidence and never repeats a
configuration call, dispatches cancellation, rolls back, or resumes another write. Saved
completion can be verified after the task record disappears. Late receipts survive
recovery/acknowledgement without reviving the old worker. Only owned tasks are cleaned up,
after terminal journal persistence. Other workspace sessions are not deliberately restarted
by isolated verification; genuine network loss can still cause normal connection recovery.

## Verification

All checks use synthetic loopback XAPI/TLS hosts, generated certificates/UUIDs and local
journals. No production host, secret, private inventory or user screenshot was used.

- Thirteen protocol fixtures cover interface move, IPv4 and IPv6 changes, stale HA/IP/MAC/
  MTU/topology, address collisions, read-only refusal, shared or changed certificates,
  CA/pin-policy preservation, wrong first-hop identity withholding credentials even with
  validation disabled, member-to-coordinator redirect verification, lost/failed/rebound
  task receipts, pending-task interruption, saved completion with removed task records,
  late receipt recovery/acknowledgement without another write, and rebound foreign-task cleanup refusal.
- A journal corruption fixture rejects changed plan scope, route, intent, task identity,
  completion, expiry and public evidence, including during conflicting admission.
- Four Rune cases cover compact/wide review, captured identities, stale pool/HA/MTU/IP,
  destination ownership changes, IPv6 input and preview mutation refusal.
- Native Vulkan synthetic reviews were rendered and inspected at 1440×900 and 513×652.

Full workspace verification passed 774 tests (338 core unit, 226 protocol, 197 Rune,
5 profile, 4 TLS and 4 CLI). Final focused checks also cover the added foreign-task cleanup
case and strengthened private-plan invariants. Strict workspace Clippy, formatting and diff
checks pass.
These checks do not establish live XCP-ng disruption/recovery behavior or Wayland/X11
input/accessibility qualification. Live testing remains deferred.

## Remaining network scope

This is a bounded management workflow, not completion of all P10 networking. DHCP/autoconf
management discovery, changing the installed primary address family, identical-address
moves, VLAN/bond management topology migration and automatic clearing of old addresses
remain separate work. Shared-certificate pools or unavailable certificate discovery cannot
use this route-verification workflow. Nonstandard management MTU remains unsupported by
XCP-ng. Broader SR-IOV capability-qualified networking and cross-host internal tunnels
remain open in the retained ledger.

## Behavioral sources

- [XAPI host API](https://xapi-project.github.io/xen-api/classes/host.html) defines
  `management_reconfigure` using the destination PIF.
- [XAPI PIF API](https://xapi-project.github.io/xen-api/classes/pif.html) defines
  IPv4/IPv6 reconfiguration and primary address type.
- [XAPI host implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_host.ml)
  checks HA and destination management eligibility, then changes the management interface.
- [XCP-ng networking documentation](https://docs.xcp-ng.org/networking/) describes management
  networking and the unsupported management jumbo-MTU constraint.
- Reference XenAdmin `ChangeNetworkingAction.cs` and `NetworkingActionHelpers.cs` separate
  address configuration, management switching and optional old-address clearing. This
  increment retains the old address explicitly instead of silently executing that cleanup.

The Rust implementation is original; no upstream source was copied.
