# Host administration increment — 2026-11-08

This increment adds reviewed direct-XAPI host maintenance, evacuation, host lifecycle and
selected settings in the Rune desktop. It is fixture-verified development software; no
production host was contacted or changed and live XCP-ng qualification remains open.

## Implemented workflows

The Hosts details pane opens Host administration. Each review retains the original pool,
connection generation, host UUID, prior settings and affected guest UUIDs. Confirmation is
unavailable after a connection/context change or when current prerequisites no longer match.
The backend rereads identities and prerequisites before every mutation.

- Enter maintenance disables scheduling when necessary, evacuates the exact reviewed running
  or paused guests and then records maintenance. Exit enables scheduling before clearing the
  marker. Evacuation reads host blockers, checks live enabled possible destinations for every
  guest and invokes the authoritative host feasibility check before disabling scheduling and
  again before evacuation. The host chooses destinations. No guest shutdown, automatic HA
  reduction, coordinator handover or rollback is performed.
- Shutdown and reboot require disabled scheduling, no active resident guests and the reviewed
  boot marker. Multi-host pool coordinators require a separate handover first. Reboot needs a
  newer boot marker; shutdown needs both a durable successful task receipt and a fresh offline
  report. A disconnected host alone does not establish shutdown.
- Toolstack restart requires HA known disabled, no pending pool tasks, and the reviewed agent
  start marker. A newer agent marker establishes completion after a disruption. XCP-ng warns
  that restarting the toolstack under HA may fence the host, so this workflow refuses it.
  See [XCP-ng HA documentation](https://docs.xcp-ng.org/management/ha/).
- Power-on uses the host's existing configured method and verifies fresh liveness. The settings
  editor supports disabled/Wake-on-LAN modes only when no BMC configuration is present.
- dom0 memory targets the exact host control-domain UUID and current memory tuple. Positive
  whole-MiB input is checked against static minimum and physical host memory. Static maximum
  and both dynamic bounds change together; static minimum is preserved. Guest VMs are never
  substituted when control-domain identity is absent or changes.
- Remote syslog accepts one DNS name/IP or local-only logging. Saving the destination and
  requesting logging reconfiguration are separate steps. Lost reconfiguration acceptance is
  unresolved even if the saved map matches.
- Standard multipathing can be toggled only on an empty maintenance host whose PBDs are all
  already detached. The standard `dmp` handler is updated separately and unrelated map keys
  are retained. No automatic detach/replug or custom-handler replacement is performed.
- Pool administration exposes an explicitly pool-wide VM autostart review. It changes only
  the pool gate and preserves individual VM flags; it does not immediately start guests.
- Forget dead host requires independent power-off acknowledgement, fresh offline/disabled/
  empty state, a non-coordinator target and HA disabled. It removes the host record only;
  storage repository records and data are retained.

The existing host inspection now also displays maintenance, multipathing, remote logging and
power-on mode alongside CPU/software metadata, liveness, memory and attachments.

API behavior was checked against [XAPI host documentation](https://xapi-project.github.io/xen-api/classes/host.html),
[XAPI VM documentation](https://xapi-project.github.io/xen-api/classes/vm.html), and the local
reference XenAdmin host actions. This is original implementation code; no upstream source
was copied. Modern/newer host fields are negotiated as known/unknown, never guessed.

## Durability and uncertainty

Host work uses the existing pool-wide resource reservation. Ordered steps persist dispatch,
accepted replies and task-success receipts. Late evidence merges into the latest revision,
retaining cancellation and never resurrecting acknowledged operations. Private host plans
are encoded only by the journal; serializing the public operation record omits task refs.

“Stop remaining steps” waits for in-flight work to resolve, then prevents later steps. It
neither cancels the server task nor undoes completed changes. Explicit task cancellation
remains a separate Activity action. Failed later steps preserve partial results. Recovery
reads existing task/object evidence and never submits unstarted steps or repeats writes.
For asynchronous work other than independently verified restart epochs, matching final state
without durable task completion stays unknown; this includes a lost deletion response even
when the host record is already absent. Operator acknowledgement remains explicit.

Shared map setters merge one key into a fresh map; XAPI does not offer compare-and-swap for
these fields, so another client can race between the final read and setter.

## Verification

- `cargo test --workspace`: **307 passed** — 139 core unit, 77 direct-protocol,
  5 profiles, 4 TLS, 4 host-probe and 78 desktop tests. This includes 18 host protocol
  fixtures, the private-plan/late-receipt journal regression and five host Rune tests.
- `cargo clippy --workspace --all-targets -- -D warnings`, formatting and diff checks passed.
- Protocol coverage includes changed membership/identity, absent destinations, refused
  evacuation after scheduling disable, HA-enabled/unknown restart refusal, lost replies,
  restart recovery without replay, stopping remaining steps, missing async completion,
  exact dom0, saved-versus-applied syslog, attached-storage refusal, readonly/generation
  admission and record-only dead-host removal. UI checks cover exact review capture at both
  widths, numeric bounds, invalid logging input, pool-wide autostart and power-off guards.
- Independent peer review prompted HA fencing protection, stronger async-completion evidence
  and private journal-only task serialization; the regression suite passed afterward.

An initial duplicate-cache build exhausted the local temporary-filesystem quota; only that
worktree's generated cache was removed and verification reused the existing shared cache.
One initial parallel run also returned a journal startup error before host dispatch; the
focused rerun and final complete suite passed. No host write was sent in that failed case.

Synthetic host-maintenance dialogs were rendered with Vulkan and visually inspected at
1440×900 and 513×652. The guest list scrolls while review controls stay visible. Only generated
fixture inventory was used; user screenshots and private inventories are not repository
artifacts. Headless checks do not establish native Wayland/X11 input, clipboard, accessibility,
portal behavior or real host compatibility.

## Retained work and qualification

The [parity ledger](../PARITY-IMPLEMENTATION.md) remains the row-level completion authority.
These are scoped workflows, not a claim that all host administration is complete.

- BMC/Secret-backed credential configuration and custom power-on plugins remain open.
- Host/root password change retains the requirement for volatile secrets and coordinated
  reconnect credential updates; no passwords may enter the serialized operation journal.
- Host configuration backup/restore and crash-dump/support-bundle downloads retain bounded
  authenticated transfer, private-file and exact-host recovery review dependencies.
- Multipath detach/replug orchestration, additional monitoring configuration and remaining
  host hardware property coverage remain open.
- Real XCP-ng 8.3/8.1+ qualification, RBAC, HA/storage constraints, coordinator interruptions,
  shutdown task timing and native desktop testing require the designated disposable lab.
