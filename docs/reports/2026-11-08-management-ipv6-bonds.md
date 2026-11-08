# Staged static IPv6 management bond/VLAN handover

Implemented a reviewed five-stage create/remove workflow for ordinary management bonds,
including management VLANs and captured local VM NIC consumers. The application talks directly
to XAPI. This report describes synthetic verification only; no live host was changed or
qualified, and the user has deferred lab work.

## User workflow

Host networking offers **Staged IPv6 management bond…** for IPv6 management interfaces.
The user selects the exact physical members or existing bond, destination network, bond mode,
and a separate, already configured static IPv6 temporary interface. The guest-impact query
captures the full relevant NIC/network graph and authoritative VM locality before review.

The stages are:

1. Authenticate the independent temporary path and move host management to it.
2. Clear IPv6 on the original interface after it is no longer management.
3. Create/remove the reviewed bond and move its reviewed VLANs and local guest interfaces.
4. Restore the original static IPv6 settings on the positively identified target interface.
5. Authenticate the target path and move management back.

Only one stage is submitted per operation. Each requires positive task completion, exact
configuration/topology readback and certificate-bound route verification before the next
**Continue** review becomes available. The temporary interface remains configured after the
handover. During the intermediate stages its address is the explicit recovery point.

Closing the window keeps the unfinished pool reservation. **Leave partial configuration…** is
a separate deliberate review that releases the local guard without host calls, rollback or a
claim of completed topology. It records the achieved stages and retained route, disables further
continuation, and requires an independent review for later changes. Active or uncertain stages
cannot use this release path; their existing read-only recheck/acknowledgement rules remain.

## Source basis

The target is the XCP-ng 8.3 XAPI source version v26.1.21. In
[`xapi_bond.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_bond.ml),
`move_configuration` transfers IPv4 settings and DNS, while newly created bond PIFs start without
IPv6 configuration. VLAN recreation also starts without IPv6 settings in
[`xapi_vlan.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vlan.ml).
The bond primary-member selection and common primary-address-family checks require explicitly
reviewed member ordering and an already configured IPv6 primary family.

[`xapi_pif.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_pif.ml)
refuses removal of an active primary IPv6 management configuration. Therefore this workflow
moves management to the temporary interface before clearing the old address, instead of assuming
that `Bond.create` or `Bond.destroy` preserves IPv6. The reference source was inspected to derive
behavior; upstream implementation code was not copied.

## Recovery and ownership

- Original and temporary topology components are disjoint, exact and bounded. Protected/service
  interfaces, host clustering, HA, tunnels and SR-IOV dependencies are refused in this workflow.
- The actual temporary login TLS connection must present the reviewed host leaf, composing with
  existing CA, explicit pin or skip-validation and proxy policy. Pool identity, host role/address
  and configuration are also checked; coordinator redirection alone is not host-route proof.
- Bond/VLAN/PIF and per-stage task baselines exclude pre-existing identities. New UUIDs are
  retained independently of full configuration success. Rebound results cannot change saved IDs.
- Each continuation atomically claims one immutable predecessor and reserves the pool before
  dispatch. Stale/duplicate, active-child and unknown-prefix continuation attempts cannot replay
  a write. Previous positive completion proof remains usable after task-record removal.
- Lost replies, failed/cancelled tasks and unresolved pending tasks remain uncertain. Recheck
  never creates a bond, reconfigures an interface, cancels a server task or advances an unsubmitted
  stage. Acknowledgement is not permission to continue.
- Public history/activity export includes stage count, deliberate partial-release status,
  recovery endpoints and created/replacement UUIDs. Raw task references and plans remain private
  to the local journal; credentials remain transient.

## Verification

`cargo test --workspace` passed all 1,030 tests: 453 core unit, 288 direct-protocol,
276 desktop and 13 profile/TLS/probe tests. The new coverage comprises ten staged protocol
fixtures, two journal tests and three Rune tests. Strict workspace/all-target Clippy,
formatting and diff checks passed. Native Vulkan renders at 1440×900 and 513×652 were
inspected; the review scrolls within the compact dialog and its footer remains visible.
All screenshots and logs remain local temporary artifacts.

Focused checks cover both create/remove directions, VLAN recreation with plugged/unplugged
siblings and guest consumers, every uncertain stage, exact temporary TLS/HA/protection/service
checks, concurrent predecessor claims, pre-dispatch graph changes, late receipts, restart with
removed tasks, deliberate partial release and private-plan corruption. Rune tests cover compact
and wide review layouts, stale members/temporary interfaces and captured continuation intent.

Independent source review covered topology/provenance, route proof, interruption fencing,
predecessor claims and deliberate guard release; its identified detached-member lookup and
acknowledged-reservation projection issues were corrected before the final checks. An initial
full run encountered the existing temporary desktop ownership fixture failure. The final run
includes the already integrated ownership-unlock fix (source commit `69505c2`) and passes that
fixture and the full suite. The new networking commit does not duplicate that prerequisite.

## Remaining scope

This increment supports one static IPv6 management address, members already using IPv6 as their
primary family, ordinary unique-tag VLANs, and the existing exact guest-consumer subset. Other
affected VLANs may use static IPv4 or no IP. It does not yet stage DHCP/autoconfiguration or
multiple IPv6 addresses through a topology replacement, switch primary address families,
retag management VLANs, or implement arbitrary same-IP physical/VLAN transitions. Tunnels,
SR-IOV and cross-host internal networking remain retained separate work. No broader feature-parity
completion or live-host qualification is claimed.
