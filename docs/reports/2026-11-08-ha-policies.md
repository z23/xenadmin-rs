# HA startup and pool policy controls

Pool administration now reviews VM startup order, start delay and shutdown delay independently,
including VMs outside a vApp. Each review captures VM and optional vApp UUID, previous value
and desired value. Fresh checks reject replaced targets, changed membership, busy VM/vApp
records and changed fields. Order must be nonnegative; delays are bounded to one day. These
are the same VM fields used by vApps, not a separate HA scheduling system. The UI explains
that ordering and delays neither start a guest nor guarantee application readiness.

Pool controls also expose the HA overcommit policy and persistent heartbeat timeout default.
The overcommit review explains the effect on restart capacity and does not claim that disabling
overcommit repairs an already overcommitted pool. Timeout editing requires disabled HA,
readable maps and no higher-priority timeout in `ha_configuration`. Operators can set a
10–86400 second default or remove the override. Unrelated `other_config` entries are preserved;
whole-map writes retain the API's concurrent-client limitation. No automatic HA disable,
enable, restart or capacity adjustment occurs.

The existing pool journal owns each setter's positive completion receipt. Readback verifies
the resulting field and membership; missing replies remain unknown even if values match.
Recheck does not repeat the setter. Required method permissions are reviewed before dispatch,
and read-only connections cannot submit.

## Primary-source basis

- The pinned [XenAdmin VM HA settings page](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenAdmin/SettingsPanels/VMHAEditPage.cs) and [startup action](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Actions/VM/SetVMStartupOptionsAction.cs) edit the shared VM order/start-delay fields.
- Pinned [XAPI VM setters](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_vm.ml) reject negative order/delay values. The existing vApp validator now applies the same nonnegative-order rule.
- Pinned [HA timeout selection](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_ha.ml) prefers `ha_configuration.timeout`, then `other_config.default_ha_timeout`, then the host default; its minimum timeout is ten seconds.
- [XCP-ng HA documentation](https://docs.xcp-ng.org/management/ha/) describes protected-VM restart order and the persistent timeout setting; the [CLI reference](https://docs.xcp-ng.org/appendix/cli_reference/) documents HA overcommit policy.

No upstream implementation was copied. Independent source review found no blocking issue.

## Verification

The workspace passed 1,052 tests: 462 core unit, 289 direct-protocol, 288 Rune, 5 profile,
4 TLS and 4 probe. Four new protocol groups cover successful setters/map preservation,
active or conflicting timeout configuration, stale fields/membership/busy targets, read-only
and input-range refusals, lost replies and positive-receipt recheck. Three Rune groups cover
compact/wide immutable reviews, unknown membership, invalid input and timeout precedence.
The new core bound fixture also covers negative shared startup order. Strict workspace/all-target Clippy, formatting and diff checks passed. Native Vulkan review renders at 513×652 and 1440×900 were visually inspected with synthetic inventory; target identity, explanation and confirmation footer were readable and reachable.

No live infrastructure was changed. Actual HA failover, fencing timeouts, guest delays and
multi-host database propagation remain disposable-lab qualification gates.
