# Required permissions before reviewed changes — 2026-11-08

The ordinary VM properties, disk/NIC actions and configuration, storage, network, host,
pool and basic resource review dialogs now show required write-permission eligibility.
A definite denial lists the missing methods and disables confirmation. The backend checks
these same requirements before the first write, so a known denial on a later required step
does not leave an earlier step applied. Host state and server rejection remain authoritative.

## Captured requirements

VM settings use their existing ordered SettingsStep plan. Device metadata and QoS changes
share the actual changed-field plan and dispatch method mapping: unchanged descriptions,
retained QoS parameters and case-only MAC spelling do not invent required writes. A NIC
network move requires VIF.move; a MAC replacement checks both VIF.destroy and VIF.create
before deleting the original interface. Disk creation checks both VDI.create and VBD.create.

Host administration shares its existing step selection and dispatch mapping. Entering
maintenance requires disabling scheduling or evacuation only when those steps were captured
in review; the maintenance marker still requires its setter. Logging checks both configuration
and application before either write. Storage, network and pool requirements reuse their
existing dispatch method mappings. Basic resource metadata requirements omit unchanged fields.
The generic resource worker additionally checks every freshly prepared step before dispatch.

Requirements are assessed against the current connection session, so stale/disconnected
snapshots show unknown information and new logins replace previous decisions. A proven denial
wins over other unknown requirements. Unknown information permits the normal reviewed flow;
it never creates an authority grant. Read-only, object-capability, identity, generation and
fresh-server checks remain in place, and recovery still only observes uncertain writes.
No session references, raw records, credentials or private host data enter this projection.

## Scope retained

This is a required-write check, not proof that every auxiliary read, protected-key operation,
HTTP transfer or fresh host prerequisite is authorized. Protected map add/remove methods
remain unknown. Authenticated storage shows the known storage requirement plus unresolved
credential steps. Pool join has destination/conditional requirements beyond its source RPC.
Dedicated VM creation/clone/delete, snapshot, migration, transfer, cloud-init, organization,
vApps, VMSS, host BMC and newly added network-topology workflows still need complete upfront
review maps. Their existing per-RPC authorization and outcome tracking remain effective.
No permission is inferred from a role-name hierarchy, and no automatic elevation is attempted.

The [session-permission report](2026-11-08-session-permissions.md#primary-semantics) contains
the primary XAPI v26.1.21/XCP-ng 8.3 and reference-client evidence for exact effective permission
matching, Async normalization, local-superuser authority and protected-key uncertainty.
This increment reuses those semantics without claiming role administration is universally
available on XCP-ng installations.

## Verification

- 746 workspace tests passed: 340 core, 204 direct connection, 189 desktop, five profile,
  four TLS and four host-probe tests.
- Four new core tests cover changed-field/conditional step sets, required denial precedence
  over unknown data, and exclusion of unnecessary NIC/QoS/maintenance methods.
- Two new protocol tests cover settings and disk-attachment later-step denial with zero writes,
  and logging-apply denial before changing the destination. Existing grant/refusal, session
  renewal and device/configuration success/recovery cases also pass.
- One new Rune interaction test exercises four review dialogs at compact/wide widths,
  checks permission-notice reachability and confirms unknown data cannot bypass the existing
  synthetic-target submission guard. The 189 desktop tests passed again after moving notices
  beside the fixed confirmation controls in dialogs with a separate footer.
- Strict workspace Clippy, workspace build, formatting and whitespace checks passed.
- Synthetic Vulkan host/storage renders at wide/compact sizes were inspected. Headless
  rendering does not establish native Wayland/X11 accessibility or real host qualification.

All verification uses synthetic inventory and local loopback fixtures. No production host
was contacted, no infrastructure mutation was performed, and no private screenshot was copied
or published. Restricted-role behavior and native host operation compatibility remain lab gates.
