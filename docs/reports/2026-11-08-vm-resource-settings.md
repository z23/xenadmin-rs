# VM resource settings increment

The existing Properties editor now supports live vCPU increases within the current maximum,
running-VM dynamic memory ranges, halted cores-per-socket topology, CPU scheduling weight,
and halted/live HVM shadow-memory multipliers. Static memory and maximum CPU changes still
use the halted-VM workflow. Live CPU reduction is refused.

Reviews capture previous values. The backend rechecks state, advertised live capabilities,
current static memory bounds and CPU maximum before writing. CPU topology must divide the
planned maximum; initial preflight projects earlier CPU edits so one reviewed patch can grow
CPUs and set its new topology. Every actual step still reads fresh host state. Map edits keep
unrelated keys. Shadow multipliers accept bounded finite input and use numeric RPC arguments;
modern `domain_type=hvm` works without the deprecated HVM boot policy.

The operation journal records dispatch and verification per step. Lost replies remain
unknown. Recheck only observes desired values and never repeats CPU hotplug or another setter.
Applied steps remain visible if a later change fails. This does not implement guest-driven
memory demand policy, live CPU removal, HA restart priorities, firmware conversion or config
drives; those retained rows remain separate work.

## Verification

Six new direct-connection tests cover combined live CPU/memory/weight edits, map preservation,
fresh changed-limit/capability refusal, lost-response CPU reconciliation with one dispatch,
modern HVM shadow configuration and invalid/oversized settings. A new core test checks coupled
CPU/topology preflight and fresh per-step guards. A Rune editor test verifies a running guest
produces live-CPU and dynamic-range commands, and rejects CPU reduction; the existing compact
review/captured-target tests also pass. Strict all-target workspace Clippy passed.

A parallel review caught deprecated HVM detection and unbounded multiplier strings; both
were corrected with coverage. All protocol fixtures are synthetic. Live qualification waits
for the user's disposable local lab and virtualization tooling.

## Primary/reference evidence

[XAPI VM methods](https://xapi-project.github.io/xen-api/classes/vm.html) define live CPU,
dynamic memory range and shadow multiplier setters. XenAdmin `ChangeVCPUSettingsAction.cs`
uses `VCPUs_at_startup` for live counts and rejects live reduction;
`ChangeMemorySettingsAction.cs` uses the dynamic-range setter for live memory changes.
No reference implementation source was copied.
