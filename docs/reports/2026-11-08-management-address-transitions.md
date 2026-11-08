# Management address transitions

This increment extends the [management handover workflow](2026-11-08-management-network.md)
with IPv4 DHCP, IPv6 DHCP and IPv6 autoconfiguration, return to static addressing,
and verified same-IP changes to static gateway, mask and DNS settings. All verification
uses synthetic hosts; live XCP-ng compatibility remains unqualified.

## Supported review and recovery

The management IP editor offers the automatic modes applicable to the installed
primary address family. The operator supplies an expected recovery IP, such as a
DHCP reservation or stable autoconfiguration address. Entering this value does not
create a reservation. The review identifies the current host/interface/settings,
chosen mode, expected address, disruption and existing certificate policy.

Only the requested mode and empty address/gateway/DNS arguments are sent for automatic
configuration. Success requires fresh readback of the chosen mode, assignment containing
the reviewed IP, host management address and unchanged secondary-family configuration,
then the existing certificate-bound authenticated recovery connection. An unexpected
lease, wrong mode or unreachable endpoint leaves an unresolved operation with its
pool reservation. Recheck reads existing evidence; it never repeats network settings.
Recorded completion can survive task removal and restart. Late replies preserve
positive task evidence without reviving an interrupted worker.

Static DNS verification follows XAPI's per-family merge: IPv4 changes preserve IPv6
DNS and IPv6 changes preserve IPv4 DNS. Changes consisting only of DNS entries from
the other family are rejected as no-ops. Ordinary physical PIFs, MTU 1500, HA disabled
and an unambiguous current primary address remain requirements.

## Source evidence and remaining scope

The [XCP-ng 8.3 package specification](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
identifies upstream v26.1.21 at the time of this review. Its
[PIF implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_pif.ml)
supports automatic modes, updates address observations after bring-up and preserves
the other DNS family. The [8.3 release documentation](https://docs.xcp-ng.org/releases/release-8-3/)
requires installation to change the host's address-family mode; this workflow does
not attempt such a conversion.

Recovery uses a reviewed IP. DNS aliases are neither created nor updated; hostname-based
recovery, automatic discovery of unexpected leases and ambiguous multiple global IPv6
source addresses remain open. Static same-IP settings changes are supported. Moving the
same IP between interfaces is a separate pending topology workflow.

Management bond/VLAN migration remains retained scope. The upstream
[bond implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_bond.ml)
and local XenAdmin CreateBondAction/DestroyBondAction show that creating or destroying
a management bond can also move IP configuration, VLANs and VIFs. These require explicit
topology plans and resource provenance; the current physical-interface checks were not
relaxed to imply support. Tunnel and SR-IOV networking likewise remain open.

## Verification

Protocol cases cover all three automatic modes, return to static, mismatched assignments,
mode and secondary-family changes, same-IP updates, DNS-family preservation, restart
after completed task removal and delayed receipts after recovery/acknowledgement.
Rune cases cover compact/wide reviews, captured mode/IP, invalid family/hostname input,
stale assignments, no-op refusal and demo non-dispatch.

The full workspace passed 821 tests: 355 core unit, 240 protocol, 213 Rune,
five profile, four TLS and four probe tests. The management subset includes
17 protocol and seven Rune cases. Strict workspace Clippy (`-D warnings`), formatting
and diff checks passed. Native Vulkan renders of the synthetic `management-dhcp`
review were inspected at 1440×900 and 513×652; the compact body scrolls and its
permission explanation and action footer remain visible. No production host calls
or private screenshots were used. A peer source review found no blocking issue in
the address-mode, observation, permission and recovery changes.
