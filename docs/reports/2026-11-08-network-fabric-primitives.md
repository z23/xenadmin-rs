# Host tunnel and SR-IOV lifecycle

This increment implements reviewed creation and removal on one host, with pool coverage shown
separately. It is part of P10-11/P10-12, not completion of those rows. Tests use synthetic
protocol fixtures and Rune scenes; no real host, controller or NIC was changed. Physical VF,
controller and traffic qualification remains deferred at the user's request.

## Available workflows

Host networking on a selected network shows every pool host and the interfaces reported on that
network. It offers GRE or VXLAN creation using an already attached IPv4 transport, and SR-IOV
creation using an eligible physical interface. Removal reviews an exact network object and its
access/logical PIF; it preserves the network record and underlying interface. Both directions
require the target network to have no VM consumers or host service purposes.

Tunnel creation requires an existing, understood `SDN_controller` configuration and Open vSwitch
on all pool hosts. The client does not install a controller, program peers through a shell or
require Xen Orchestra. Configuration, controller-reported activity/error and actual traffic
qualification are separate states. Removal remains available without a configured controller or
an attached transport when its exact unused graph can still be checked.

SR-IOV creation requires the pool feature to be explicitly available, advertised PIF capability,
known PCI vendor/device/driver identity, matching hardware types on the target network, and a
live, disabled, evacuated host with pool HA disabled. The review identifies all physical interfaces
using the same driver. Management, IP, service, VM, bond, VLAN, tunnel or other SR-IOV dependencies
in that driver group prevent submission. A new network starts with the coordinator. The server's
configuration mode and reboot requirement remain visible; this operation never submits a reboot.
Removal retains driver/consumer guards but does not require the creation feature flag or capability
to remain enabled. Removing manually configured SR-IOV does not establish that hardware VFs were
disabled.

## Identity, persistence and uncertainty

Commands capture the pool, target network policy, selected host/input PIF, complete current target
membership, pool host set, controller configuration where needed, PCI identity and driver impact.
Fresh bounded class maps revalidate those identities, both sides of the PIF/object relations and
absence of unreviewed consumers immediately before dispatch. Unknown, unsupported, denied and
empty optional-class results remain distinct in inventory.

Each operation uses one fixed asynchronous XAPI method through the existing resource worker.
Before dispatch, a private journal plan captures pre-existing object/PIF/task UUID sets. Returned
task UUIDs must be new and remain stable. A successful task receipt is persisted before readback;
new object/PIF identities are retained before broader configuration verification. Result references
must resolve consistently by UUID, the new pair must not occur in the baseline, and subsequent
reads must match any saved pair. Public records and Activity exports expose stable UUIDs, not raw
references, baselines or private plans.

Task failure, cancellation, lost replies and failed verification retain an unknown outcome and the
pool reservation. Recheck never repeats creation or removal, and observed absence alone cannot
prove a lost deletion succeeded. Persisted positive completion survives task removal. Late positive
evidence can add the created identities after recovery or acknowledgement without reviving the
original worker or changing an acknowledged state. Read-only connections cannot submit changes.

The private plan validator binds the public action, exact step label/shape, pool and network,
required bounded baseline classes, captured existing identities, task receipt and created pair.
A successful or verified public row without its positive evidence is rejected on history loading.

## Source boundary

Behavior was checked against XCP-ng 8.3 XAPI `v26.1.21` and local XenAdmin
`35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`. No implementation source was copied.

- [`xapi_tunnel.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_tunnel.ml)
  creates a new access PIF, plugs it and initializes controller activity as false. The
  [tunnel design](https://github.com/xapi-project/xen-api/blob/v26.1.21/doc/content/design/tunnelling.md)
  assigns peer/key programming to the controller. `SDN_controller` objects are consumed by the
  [vSwitch plugin](https://github.com/xapi-project/xen-api/blob/v26.1.21/python3/plugins/openvswitch-config-update).
- [`xapi_network_sriov.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_network_sriov.ml)
  provides create/destroy/capacity methods and creates the logical PIF. Its
  [helpers](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_network_sriov_helpers.ml)
  report sysfs/modprobe/manual mode, reboot requirements and driver-group behavior. Creation and
  removal have different feature prerequisites; manual hardware configuration is not automatically
  reversed. XenAdmin's `CreateSriovAction` provides the coordinator-first ordering reference.

## Verification

The focused increment adds three decoder tests, eleven protocol tests with multiple mode/fault
subcases, and three Rune tests. Cases include both tunnel protocols; SR-IOV with and without reboot;
exact removal; stale identities, controller and consumer changes; protected driver impact; failed,
cancelled, lost or misbound task responses; read-only admission; retained creation IDs after failed
verification; removed-task recovery; late receipts after recovery/acknowledgement; corrupt public
step/private receipt combinations; and cleanup after creation-only prerequisites disappear.

Compact and wide review tests cover captured host/transport/controller/PCI details, explicit
reboot/traffic limits, stale review refusal and disabled synthetic Apply. Independent source review
found a missing public-step/private-proof binding; the validator and six corruption subcases now
cover it. No source-review blocker remains.

All 1,146 workspace tests passed: 501 core unit, 315 protocol, 317 desktop and 13
profile/TLS/probe tests. Strict workspace/all-target Clippy, formatting and diff checks passed.
Native Vulkan tunnel and SR-IOV review scenes were rendered and inspected at 1440×900 and
513×652; the review content scrolls while action controls remain visible. A raw controller-enum
label found during inspection was replaced with a readable TLS endpoint description. Synthetic
images and logs remain local temporary files; no live screenshots are included.

## Retained work

- Pool-wide controller-backed tunnel and SR-IOV orchestration with explicit complete coverage,
  per-host continuation and retained partial results.
- Read-only VF-capacity queries and SR-IOV logical-PIF VLAN lifecycle/mapping.
- Controller setup and transport qualification are separate capabilities; local bridge creation
  must never be presented as a working controllerless cross-host network.
- Broader same-driver dependency migration, guest-consumer migration and hardware qualification
  remain open; this initial SR-IOV workflow deliberately requires reviewed empty interfaces.
- Earlier management gaps remain retained: automatic/multiple-address IPv6 topology migration,
  standalone management retagging/family switch, clustering and service-dependent transitions.
