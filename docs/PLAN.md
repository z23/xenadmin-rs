# Implementation plan

## Outcome

Build an installable Linux desktop client that connects directly to XCP-ng hosts and pools.
An operator can find, start, and open the console of the Xen Orchestra VM while XO itself
is unreachable. All application connections use XAPI and the advertised host endpoints.

This is an authorized reimplementation. The reference client supplies behavioral evidence.
The new UI uses Rune and the backend uses Rust. [Feature scope](FEATURES.md) owns the
workflow list. [Architecture](ARCHITECTURE.md) owns the implementation boundaries.

## Retained parity delivery

The expanded scope retains every workflow in [the implementation ledger](PARITY-IMPLEMENTATION.md)
except the two excluded categories: Specialized hardware and guests; DR, updates and legacy
services. Their exclusions apply throughout the other workflows too. The ledger maps all
202 retained rows to their delivery status; a foundation is not a completed workflow.

The following phases extend the initial M0–M6 roadmap below. They are ordered by shared
backend dependencies, and each slice must leave the desktop usable.

| Phase | Delivery |
| --- | --- |
| P0 | Row-complete scope ledger and ownership-checked disposable nested KVM lab tooling |
| P1 | Saved/concurrent connections, one authenticated owner per pool, trust, credentials, failover and single instance |
| P2 | Typed device/host/task graph, capability explanations, reviewed operations and durable partial/unknown outcomes |
| P3 | Recovery controls, chosen-host power actions, reliable console input/clipboard/reconnect and independent console windows |
| P4 | VM properties, disks, ISO drives and virtual network interfaces |
| P5 | Snapshot modes, restore orchestration, deletion/clone/VMSS, VM creation/templates and ownership-aware deletion |
| P6 | Storage and network lifecycle, physical interfaces, VLANs/bonds, multipath and management addressing |
| P7 | Bounded import/export/conversion and same/cross-pool migrations |
| P8 | Host/pool administration, maintenance, HA, auth/security and host configuration backup/restore |
| P9 | Metrics, tasks, alerts, operation history and diagnostic reports |
| P10 | Organization, saved searches, tags/folders, vApps, bulk actions, native extensions and translation catalogs |
| P11 | AppImage/portable packaging and native Arch/Ubuntu 24.04 qualification under Wayland/X11 |

Current implementation work delivers parts of P0–P10. The prepared offline lab
handoff awaits virtualization; live qualification is deferred at the user's request.
All phases retain open workflows or qualification gates. The per-row ledger and verification report
are the completion authority. In particular, an API/data-model implementation does not
establish a usable end-to-end workflow or live-host compatibility.

Keep XCP-ng 8.3 as the primary host target; qualify 8.1+ explicitly, including XML-RPC
where needed. Quiesced/VSS snapshot APIs removed in XCP-ng 8.1 are reported as unsupported,
not emulated. VMSS schedules run on capable hosts, not in an unattended desktop scheduler.
Native extensions use versioned trusted local subprocess/declarative UI integration;
Windows plugin compatibility is excluded. English and translation catalogs are the initial
localization target. Ordinary guest tools, firmware/boot, HA and host configuration recovery
remain in scope.

The [lab harness](../lab/README.md) can generate and manage only its explicitly owned,
isolated resources. No production infrastructure test is implied by application development.

## Starting decisions

- Repository: [z23/xenadmin-rs](https://github.com/z23/xenadmin-rs), initially private.
- Workspace: `~/src/xenadmin-rs`.
- Initial host target: XCP-ng 8.3 LTS. Additional versions need their own compatibility evidence.
- Initial desktop target: Linux x86-64, with real Wayland and X11 checks. Test Arch on the
  development workstation and Ubuntu 24.04 LTS as a distribution baseline.
- Distribution: AppImage and a portable archive for x86-64, with installed desktop metadata.
  Qualify runtime dependencies and software rendering on both target distributions.
- Original work: MIT license. Reused upstream code retains its notices.
- Delivery: small commits that leave a working vertical slice. Later milestones extend that
  slice. Dates and effort estimates follow the M0 compatibility results.

## Current state

The development client includes direct concurrent pool connections, inventory and consoles,
reviewed VM/device/snapshot administration, host/pool/storage/network workflows, metrics,
activity, organization and bounded XVA/raw/VHD/OVF transfers. The
[0.8 checkpoint](reports/2026-11-08-checkpoint-0.8.md) records its packaged subset; subsequent
firmware, bulk power and console-recovery increments are tracked in the per-row ledger and
their reports. Implemented behavior has protocol, journal, Rune and native synthetic-render
checks, with remaining workflows explicitly open.

The user requested a prepared local disposable lab and deferred live tests until virtualization
is available. No active lab has been provisioned. Actual host compatibility, guest data
integrity, failover and the complete XO-down acceptance run remain qualification gates.
Milestones remain open until their retained workflows and relevant acceptance checks pass.

## M0: Qualify the dependencies and host protocols

Build the smallest real Rune application and direct-host probes before creating the full UI.
M0 resolves the uncertainties that could force a redesign.

1. Create the Cargo workspace, pin a Rust toolchain compatible with the selected Rune
   revision, and commit `Cargo.lock`. Begin with Rune commit
   `f3d04f3f2069d177d04c963b0ba9f6b2065e46eb` as the reviewed candidate.
2. Show a Rune window with a virtualized VM table, tree, details pane, and operation list.
   Exercise keyboard selection and persisted column widths with synthetic inventory.
3. Connect a CLI probe to a designated XCP-ng 8.3 lab endpoint. Verify TLS trust,
   JSON-RPC `/jsonrpc`, login, pool identity, version fields, coordinator redirection,
   `event.from` bootstrap and continuation, task queries, and logout.
4. Qualify one RFB implementation against the actual lab console tunnel. Check a stream-based
   API, compatible license, supported encodings, resize/cursor behavior, key release, and
   QEMU extended-key negotiation. Prefer a Rust implementation that passes these checks.
   If none passes, use a contained libvncclient binding rather than expanding scope into a
   general-purpose VNC library. Record the selected library and revision in an ADR.
5. Decode a console frame into a Rune window and send keyboard and pointer input back through
   the authenticated tunnel. Confirm the running VM's response from its console.
6. Verify Secret Service availability, a session-only credential mode, native file dialogs,
   AT-SPI exposure, and software Vulkan rendering. Pin any upstream fixes needed for the target.
7. Export an API description matching the lab's API lineage. Inspect the upstream SDK
   generator's machine-readable output. Choose a reproducible binding generator input,
   without writing a parser for OCaml source. Start with the classes used by M1 and M2.

Acceptance gate: record exact dependencies, host API version, negotiated console features,
and actual desktop results in a compatibility report. The window, read-only host probe,
and interactive console run from the local build. Supply sanitized protocol fixtures.
Keep a written list of any unverified platform behavior.

If a Rune platform gap blocks the required workflow, first test an application adapter or
a pinned upstream fix. A framework change needs an updated design decision. If JSON-RPC is
missing on a supported target, add XML-RPC behind the existing transport boundary and prove
the same behavior there.

## M1: Connect and maintain live inventory

Implement saved endpoints, explicit certificate trust, credential lookup, authentication,
pool discovery, and the live object store. Implement the full connection lifecycle before
adding actions that change host state.

- Accept any reachable pool member as the initial endpoint. Remember verified member
  addresses and follow XAPI coordinator discovery.
- Bootstrap event-covered records with `event.from` and its initial token. Fetch required
  non-event classes separately. Publish the initial inventory only after the batch applies.
- Deduplicate connection profiles that resolve to the same pool without merging unrelated
  pools or trusting object names as identities.
- Handle session expiry, toolstack disruption, endpoint changes, authentication refusal,
  certificate changes, invalid event tokens, and a manually disconnected profile.
- Show connection state, stale inventory age, the current coordinator, and reachable members.
  Disable actions whose prerequisites require current inventory.
- Render the inventory tree, searchable tables, details, networks, and storage in read-only
  form. Preserve selection by stable identity as records update.

Acceptance gate: the inventory follows changes made by another lab client. Session expiry
and connection loss leave an explicit stale view. Reconnect replaces the inventory
coherently. Delayed results from the old connection cannot alter the new view. XO is
unreachable throughout the direct-access acceptance run.

## M2: Deliver the recovery workflow

Add typed commands for VM start, clean shutdown, reboot, and explicit force actions.
Use server permissions and capabilities to explain unavailable actions. Track operations
through submission, task progress, completion, and uncertain outcomes.

- Open the embedded console from the selected VM using its current advertised endpoint.
- Handle console replacement after reboot and migration, remote resolution changes,
  non-US keyboard layouts, focus loss, pointer coordinates, and release of held input.
- Keep credentials and console input out of logs. Make console clipboard sharing an
  explicit user-controlled action and support capability refusal.
- Preserve operation history across transient disconnections. Reconcile submitted operations
  after reconnect instead of replaying writes whose acceptance is unknown.
- Query tasks periodically when event coverage is missing or unreliable. Represent a
  missing task as unresolved until task or object evidence establishes an outcome.
- Add a bounded local operation journal for accepted operation IDs, target identities,
  known server task references, and observed outcomes. Store no credentials in the journal.
  On restart, reconcile unfinished entries. Never automatically replay them.

Acceptance gate: with XO unavailable, connect directly, locate the lab XO VM, start it if
stopped, and interact with its console. Run the transport-failure and uncertain-write cases
in [verification](VERIFICATION.md). A lost response cannot silently duplicate a command.
Deliver the local executable and its runtime-dependency instructions for this alpha.

## M3: Package a usable Linux release

Add CPU, memory, disk, and network charts from RRD endpoints. Fetch according to visible
views and keep bounded sample history. Separate metrics failure from management availability.

Finish connection editing, search, keyboard shortcuts, task details, error recovery,
empty states, theme choices, HiDPI behavior, and accessible labels. Verify the file picker
under Wayland and X11. A missing drag-and-drop capability must leave a working picker.

Build CI jobs for formatting, Clippy, unit and protocol tests, Rune headless fixtures,
and Linux release artifacts. Hardware and lab jobs remain opt-in, use isolated fixtures,
and report their target identity. Keep CI infrastructure separate from managed hosts.

Acceptance gate: install the release on clean target systems, cold-start it without
internet or XO, authenticate with saved or session-only credentials, and complete M2.
Exercise both real display protocols, a software GPU path, and Orca. Record measured
performance with workload and hardware details before setting release budgets.

## M4: Add routine VM administration

Deliver template-based VM creation, CPU and memory editing, disk and NIC attachment,
ISO selection, snapshots, clones, rename, and explicit deletion. Break complex wizards
into a reviewable plan and tracked execution steps.

Creation and deletion are multi-object workflows. Track which VM, VDI, VBD, and VIF records
the operation created. Cleanup affects only resources owned by that attempt. Preserve and
show partial outcomes when another operator intervenes or rollback fails.

Acceptance gate: each workflow has one successful lab run, one partial-failure run,
and a verified resulting inventory. Snapshot restore and disk deletion require disposable
data. Cancellation reports whether server work actually stopped.

## M5: Add host, storage, and network management

Add host maintenance and evacuation, host shutdown and reboot, storage attachment and
rescan, network creation, and supported NIC/bond/VLAN operations. Add same-pool migration
with destination capability and storage/network checks.

Show the effects and connectivity risks in each operation's review screen. Changing the
management network can sever the client's path. Capture the recovery endpoints and
represent loss of contact as an unresolved outcome until verified.

Acceptance gate: complete these operations in a disposable multi-host lab. Verify migration
and coordinator changes with live events and console relocation. Confirm that the client
does not try to repair quorum, override fencing, or force promotion after ordinary failure.

## M6: Extend portability and transfer workflows

Add streamed XVA import and export, disk transfers where supported, cross-pool migration,
and additional host versions or Linux architectures as separate increments. Each supported
format needs interoperability fixtures and a real import/export round trip.

Acceptance gate: transfer cancellation, truncated streams, disk exhaustion, credential expiry,
and endpoint redirection produce attributable outcomes and no unowned cleanup. Large payloads
stream with bounded buffers. Release notes state the tested combinations.

## Outside retained scope

The two excluded parity categories, XO backup repository management, cloud orchestration,
Windows ActiveX/plugin compatibility, host agents and legacy XenServer compatibility are
outside the retained scope. The client recovers access to XCP-ng management while XO is down.
It cannot operate XAPI when the host toolstack itself is unavailable.

## Next acceptance task

Designate a disposable XCP-ng 8.3 pool, a credential reference, and permitted test scope.
Run the read-only probe first, qualify the console and API schema against that exact host
lineage, then perform the M2 outage workflow within the authorized scope. Resolve the
remaining M0–M2 gaps recorded in the alpha report before claiming a recovery release.
