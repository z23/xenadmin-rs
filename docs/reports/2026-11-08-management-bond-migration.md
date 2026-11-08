# Management bond creation and removal

## Implemented workflow

Host networking now has a reviewed management-bond workflow. Creation captures the
current static IPv4 management interface, 2–4 physical members, an existing empty
destination network and the selected bond mode (including LACP hashing). Removal captures
an existing management bond and its exact primary member. Both retain the existing
management address; XAPI transfers the configuration as part of its bond operation.
The desktop lists each affected member, network, address and recovery endpoint before apply.

This increment supports plain IPv4 management bonds at MTU 1500 with HA disabled.
Creation requires unconfigured secondary members; removal requires a complete captured
bond component. VLANs, VM interfaces on affected networks, host clustering, separate
interface purposes, protected interfaces, tunnels and SR-IOV dependencies are refused
before the write. Migration of those consumers remains open. IPv6 bond handover also
remains open: the target implementation's bond configuration transfer copies IPv4 fields,
so supporting IPv6 requires a separate qualified sequence. Existing already-configured
IPv6 bond/VLAN management endpoints remain supported by the prior handover workflow.

The destination network must already exist and contain no PIFs or VIFs. Creating a suitable
internal network is a separate existing action. Network objects are retained when a bond
is removed. Switch configuration is external to this client and must match the reviewed mode.

## Backend and recovery

The existing management worker executes one asynchronous `Bond.create` or `Bond.destroy`.
It registers the exact recovery endpoint, records dispatch before calling XAPI, checks a
new task UUID, and requires a successful completion receipt before verifying the outcome.
A failed task can have changed topology, so failure and lost replies remain uncertain.
Recheck inspects evidence and never repeats creation, removal, address changes or rollback.

Fresh reads check exact pool/host identity, HA state, the captured interfaces, both directions
of bond/VLAN references, interface properties, network consumers and clustering. A private
pre-existing Bond/PIF UUID baseline prevents an existing object from being reported as a
new result. Once a task result identifies a new bond/master pair with the reviewed members,
that pair is retained independently of subsequent configuration or route verification.
Saved identities cannot be replaced by a later rebound reference.

Completion requires the expected management role, static address/netmask/gateway/DNS,
member identities, transferred configuration and topology. The recovery connection binds
the reviewed host's exact TLS leaf before credentials, while retaining the connection's
normal CA/pin/explicit-skip and proxy policy. An authenticated coordinator redirect does
not itself establish first-hop host identity. Public history distinguishes configuration
observed from recovery endpoint verified, and shows identified new bond/master UUIDs.
Raw task references, property baselines and private plans stay out of public serialization.

## Evidence and limitations

Synthetic protocol cases cover create/remove success, changed identities/properties and
consumers, failed/pending/lost replies, unexpected results, wrong final configuration,
restart after task removal and delayed replies after recovery/acknowledgement. Journal
cases exercise baseline corruption and monotonic late-created identity evidence. Rune
review cases cover exact captured members, stale inputs, pool changes, compact/wide layouts
and demo mutation refusal. The worktree workspace run passed 904 tests (398 core unit,
258 protocol, 235 desktop, 5 profile, 4 TLS and 4 probe); an additional removal-review
Rune case subsequently passed, taking the desktop count to 236. The final focused checks,
strict all-target workspace Clippy, format/diff checks and native Vulkan renders at
1440×900 and 513×652 are recorded in the integration handoff. A source peer review found
no remaining blocker in the reviewed bond/provenance/recovery paths.

No real host, switch or guest was changed. Fixtures establish client behavior, not live bond
convergence, DHCP behavior, packet loss, storage continuity or actual host interoperability.
VLAN/guest-consumer management bond migration and broader network topology workflows remain
retained work, not unsupported feature declarations.

## Primary behavioral sources

- [XAPI v26.1.21 bond implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_bond.ml)
  selects the management member as primary, inherits its MAC when none is supplied,
  transfers IPv4 configuration, migrates management and can move VLANs/VIFs/tunnels.
  Removal transfers configuration back to the primary member and destroys the bond/master.
- [XCP-ng 8.3 xapi package specification](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
  identifies the target upstream version. The local XenAdmin `CreateBondAction` and
  `DestroyBondAction` provide reference workflow context. No upstream source was copied.
