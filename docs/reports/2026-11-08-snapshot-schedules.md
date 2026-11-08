# Host snapshot schedules — 2026-11-08

The Rune desktop now manages direct-XAPI VMSS snapshot policies. This is fixture-verified
software; no real XCP-ng host was contacted. Live scheduler, clock/DST, retention and
checkpoint interoperability remain lab qualification work.

The Snapshot schedules workspace creates a disabled, empty policy; edits a disabled
policy's name, description, snapshot/checkpoint mode, retention and schedule; reviews VM
assignment/removal; enables or disables scheduling; runs an enabled nonempty policy now;
and deletes a disabled empty policy after its assignments are separately removed.
Deleting a policy retains guests and snapshots, but XAPI removes associated policy
messages. Configuration changes are journaled individually and can end partially applied.
The existing Activity workspace exposes operation outcomes, recovery and acknowledgement.

Hourly, daily and weekly schedules accept only minute 0, 15, 30 or 45. Weekly schedules
select explicit weekdays, and retention is 1–10 scheduled snapshots per VM. Quiesced/VSS
mode is unsupported. Enabling and Run now explicitly warn that retention may remove the
oldest VMSS-created snapshot, including one made by a previous policy; manual snapshots
are excluded. Enabling may cause missed work to run on the next cron check. Disabling
future runs does not cancel an in-flight job.

## Host execution and clock evidence

The implementation is based on the official [VMSS API](https://xapi-project.github.io/xen-api/classes/vmss.html)
and the [XCP-ng VMSS implementation](https://github.com/xcp-ng/xen-api/blob/master/ocaml/xapi/xapi_vmss.ml).
XAPI delegates execution to a VMSS plugin. The
[XCP-ng 8.3 package specification](https://github.com/xcp-ng-rpms/vmss/blob/8.3/SPECS/vmss.spec)
ships that plugin and `/etc/cron.d/vmss.cron`. The public source archive
[`vmss-1.2.1.tar.gz`](https://github.com/xcp-ng-rpms/vmss/blob/8.3/SOURCES/vmss-1.2.1.tar.gz)
was inspected outside the repository and verified against SHA256
`2a3f53605648bd9d18acd83170196a59a4d4eaa3e237f0e72e784f313940826b`.
Its cron checks every 15 minutes on the coordinator, and schedule evaluation uses local
wall-clock time. No GPL implementation source was copied into the application.

Support requires an explicitly unrestricted VMSS pool feature, the VMSS API, an XCP-ng 8.3
coordinator, and readable UTC/local host clocks. A background read-only query displays both
clocks and the sampled UTC offset. Configuration/enable review captures coordinator UUID
and offset, which are reread before dispatch. Unknown clocks, excessive query skew and
changed coordinator/offset block those changes. The view states that schedule hours follow
the coordinator's timezone and future DST changes; it does not pretend that a current
offset is an IANA timezone. The supported host APIs are documented in the
[host reference](https://xapi-project.github.io/xen-api/classes/host.html).

Package/API evidence establishes the supported software contract, not the state of the
cron service on a particular host. The UI explicitly distinguishes policy configuration
and last-run evidence from service health. Other host releases fail closed for creation,
configuration, enabling and Run now until their VMSS execution contract is qualified;
disabling/removing assignments/deleting existing supported policies remain available.

## Recovery and validation

Every mutation retains its observed pool/generation, policy and VM UUIDs and reviewed
settings. Membership is complete or unavailable, never silently truncated. Creation keeps
a private preexisting-UUID baseline and rejects a task result naming an old policy. Private
baselines and completion evidence are durable but excluded from public serialization.

Run now requires fresh enabled/nonempty membership and valid snapshot/checkpoint capability
for each member. Success needs a durable successful host task, the plugin's explicit `True`
result, a newer policy last-run time and a fresh VMSS snapshot for every reviewed member.
Failure/cancellation, a false plugin result, missing member snapshots or a lost reply retain
an unknown outcome and pool reservation. No operation is replayed or automatically cleaned
up. Recheck/restart can finish from saved completion evidence when a task has disappeared.

Validation: all 426 workspace tests passed (193 core unit, 120 direct-XAPI,
5 profiles, 4 TLS, 4 host-probe and 100 desktop). Ten new VMSS protocol fixtures
cover successful workflows, clock/runtime refusals, exact membership, read-only admission,
partial/failed/cancelled runs, lost replies, preexisting result rejection and restart recovery.
Three Rune tests cover both review widths, invalid schedule/retention/clock input and weekly
checkpoint selection. Strict workspace Clippy, formatting and diff checks passed.
Synthetic Vulkan editors were rendered and visually inspected at 1440×900 and 513×652;
visible field captions were corrected after inspection and the Rune tests and Clippy were
rerun. Only synthetic inventory was rendered, outside the repository. Native Wayland/X11
and live host qualification are not established by these checks.
