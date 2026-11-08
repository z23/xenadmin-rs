# Network service policy and remaining network capability audit

This increment adds a complete reviewed NBD service-policy workflow. The retained tunnel and
SR-IOV workflows below have been audited and designed; their lifecycle implementation remains
pending. Verification uses synthetic protocol fixtures and native synthetic scenes only. No
host, controller, physical interface or guest network was changed; live qualification remains
deferred at the user's request.

## Implemented service policy

Host networking on a selected managed network offers separate enable/remove actions for NBD with TLS
and NBD without TLS. The plaintext action has an explicit review explaining the absence of
transport confidentiality/integrity. There is no automatic mode switch, downgrade, or removal
of another network's purpose. Interface purpose labels and unplug protection remain separate
settings: they are not interpreted as host service policies.

The captured command contains the exact network, current purpose set and current interface
UUIDs, host/network identities, attachment states and IPv4/IPv6 addresses. The review states
that policy also applies to interfaces added later. Before dispatch the backend reads the exact
pool and one bounded network map, validates the selected UUID/reference and every interface in
both directions, and checks pool-wide exclusion of conflicting TLS/plaintext purposes. Unknown
or unavailable policy data fails closed. An interface added, removed, rebound or readdressed
since review prevents dispatch.

One asynchronous `network.add_purpose` or `network.remove_purpose` call runs through the existing
durable resource journal and permission review. A task receipt and matching fresh policy/graph
are required for completion. Lost responses remain unknown, retain the pool reservation and
cannot be replayed by Recheck. Read-only sessions cannot submit. Verification reports stored
policy, not NBD listener reachability, firewall propagation or actual traffic success.

The policy field is optional in typed inventory: missing/invalid data is distinct from an empty
set. Unknown future purpose names remain visible to decoding but cannot be silently overwritten.
All other network properties and interface addressing are preserved.

## Pinned source audit

The primary target is XCP-ng 8.3's XAPI `v26.1.21`, with local XenAdmin reference
`35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`. Source was read for behavior; no implementation code
was copied.

- [`datamodel.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/datamodel.ml)
  defines `network.add_purpose`/`remove_purpose`, with `nbd` and `insecure_nbd` as the supported
  service purposes. [`xapi_network.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_network.ml)
  rejects TLS/plaintext coexistence across the pool. There is no generic `storage` or `management`
  service-purpose enum; XenAdmin uses `PIF.other_config[management_purpose]` as a descriptive
  secondary-interface label.
- [`Tunnel.create`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_tunnel.ml)
  supports GRE/VXLAN objects and requires Open vSwitch on every pool host. It creates the access
  PIF and plugs it, but initializes controller activity as false. The
  [tunnel design](https://github.com/xapi-project/xen-api/blob/v26.1.21/doc/content/design/tunnelling.md)
  assigns peer/key programming to a controller; without it, only the local bridge exists.
  [`nm.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/nm.ml)
  retains that local-bridge behavior and handles the VXLAN firewall port.
- The pinned [vSwitch plugin](https://github.com/xapi-project/xen-api/blob/v26.1.21/python3/plugins/openvswitch-config-update)
  consumes configured `SDN_controller` records. XenAdmin's `CreateChinAction` creates one tunnel
  per selected management transport; its wizard required a configured controller and hides that
  legacy choice on Stockholm-or-later servers. A configured controller is not proof it is working.
- [`Network_sriov`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_network_sriov.ml)
  supports create/destroy and free-VF queries. The selected managed physical PIF must advertise
  `sriov`; pool feature restrictions also apply. A network's SR-IOV PIFs must use matching PCI
  device types. Creation adds a logical PIF and attempts activation. Its
  [activation helper](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_network_sriov_helpers.ml)
  exposes configuration mode and whether a host reboot is required. Manual configuration cannot
  be assumed to have been disabled merely because the XAPI objects were removed.

These APIs are present in the pinned stock source. Missing controllers, feature restrictions,
driver/VF availability, reboot requirements and absent hardware qualification are different
limits; they must not be described as nonexistent APIs.

## Next lifecycle contracts

Tunnel inventory will negotiate typed tunnel/access/transport/controller identities and bounded
controller-reported status. An unused target network can be configured across an explicitly
reviewed host set with GRE or VXLAN. Each transport must have usable IPv4 configuration (the
pinned `PIF.plug` gate specifically requires IPv4 mode), supported topology and a configured
controller; every pool host must advertise Open vSwitch. Per-step task/tunnel/access-PIF baselines
will exclude pre-existing results. Fresh host/controller/transport and reverse graph checks will
run before every write; exact identities will be verified afterward. Unused-only deletion will
name the exact tunnel/PIF set. Configured objects, controller-reported activity and traffic
qualification will remain distinct. No controller installation, XO requirement, ad hoc shell
peer programming or speculative route probing is planned.

SR-IOV inventory will expose physical/logical PIF linkage, PCI identity, advertised capability,
feature restriction, configuration mode, reboot requirement and separately queried available VF
count. Creation will review each physical PIF and its host, process the coordinator first, require
matching hardware types, and refuse management/protected/service dependencies or unreviewed
consumers. Logical-PIF VLAN creation/deletion and unused exact SR-IOV removal remain part of the
workflow. Each creation/deletion step will use existing durable receipt and no-replay rules;
partial resources will be retained and identified for separate cleanup review. Host maintenance
and reboot prerequisites will be visible; no host reboot will be implicit.

## Verification

Focused protocol tests cover both policy modes and enable/remove directions; preservation of
other settings; conflicting/unknown pool policy; changed interface/address/membership; duplicate
identities; stale pool; lost replies, retained reservations and no replay; task/readback failures;
read-only admission; and changed/missing/rebound selected-network data between reads. Rune tests
cover compact/wide review and stale policy/interface/pool guards.

All 1,116 workspace tests passed: 489 core unit, 304 direct-protocol, 310 desktop and 13
profile/TLS/probe tests. The increment adds two boundary tests, five protocol tests with multiple
mode/fault subcases, and two Rune tests. Strict workspace/all-target Clippy, formatting and diff
checks passed. Native Vulkan renders at 1440×900 and 513×652 were inspected; review text and
footer controls remain usable. Synthetic images and logs stay in temporary local files.

An independent source reviewer identified a selected-network freshness gap between separate
reads. Policy verification and the pool conflict scan now use the same bounded network map,
with exact UUID/reference checks; changed, missing and rebound selected rows have regression
fixtures. No review blocker remains. The full suite includes the already integrated deterministic
storage-auth observer fix (`e01efbc`, source `3dba9df`); this networking commit does not duplicate it.

## Retained gaps

Controller-backed cross-host internal/tunnel lifecycle and SR-IOV lifecycle remain pending after
this service-policy increment. Controllerless cross-host transport requires a different supported
control plane; creating XAPI Tunnel objects alone is insufficient. Clustering/service-dependent
management moves, dynamic/multiple-address IPv6 topology changes, primary-family changes,
standalone management VLAN retagging and same-IP physical/VLAN handover remain open. None is
removed from scope by this audit.
