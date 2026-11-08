# Host data-source recording

Hosts now offer **Host administration → Monitoring recording**. The panel reads
the actual host data-source catalog, filters and pages it, and displays each
source's name, description, units, bounds, standard/default flag and observed
recording state. Recording changes are separate from selecting a performance
graph. The catalog remains available on read-only connections.

**Start recording** enables an inactive source. **Forget recorded history** has a
separate captured review and explicit history-loss acknowledgement. Forgetting
removes that source's samples from the host's active RRD archives; it is not a
pause. Standard/default sources may resume collection immediately. The panel
therefore never promises a permanent disabled state, and successful deletion
history explicitly distinguishes the positive host receipt from subsequent
recording. Downloaded reports and other copies of historical data are unaffected.

The review stays bound to the original pool, host UUID, connection generation and
complete source metadata/state. The backend rereads exact host identity, live/idle
state and the real catalog before the setter. Missing, duplicate or malformed
catalog entries fail closed. Unknown synthetic metric placeholders cannot be
submitted as catalog selections. Existing session permission discovery checks the
exact method, and server authorization remains authoritative.

Both setters reuse the durable Host operation engine. A positive synchronous
receipt is persisted before readback. Start additionally requires enabled
readback; Forget permits an absent archival-only source or a source that already
resumed collection. Both verify the exact host and any remaining source metadata.
A lost reply stays unknown even if enabled state happens to match; rechecking
does not repeat a setter. A saved receipt can be verified after restart. Existing
interruption/acknowledgement fences preserve late receipts without continuing
mutations. Public operation serialization does not expose transport references.

The Rune modal keeps its heading and footer outside the scrollable body at wide
and compact sizes. Refresh discards the old catalog/review. History opens for the
captured pool even if the visible inventory has since switched pools. Unknown
outcomes use the existing Activity recheck/acknowledgement controls.

## Source evidence and target boundary

The primary target remains XCP-ng 8.3 / XAPI `v26.1.21`:

- [`datamodel_host.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/datamodel_host.ml)
  defines `host.get_data_sources`, `host.record_data_source` and
  `host.forget_data_source_archives`; catalog reads permit read-only roles and the
  two setters use pool-operator authorization.
- [`message_forwarding.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/message_forwarding.ml)
  routes all three methods to the explicitly selected host. The coordinator does
  not silently substitute its own RRD data.
- [`xapi_host.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_host.ml)
  forwards recording and forgetting to RRDD. [`rrdd_server.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xcp-rrdd/bin/rrdd/rrdd_server.ml)
  supplies the complete live/archival catalog and calls `Rrd.rrd_remove_ds` for
  forgetting. [`rrd.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/libs/xapi-rrd/lib/rrd.ml)
  removes the source and its associated sample/consolidation data from every RRA.
- [`rrdd_monitor.ml`](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xcp-rrdd/bin/rrdd/rrdd_monitor.ml)
  automatically adds default sources missing from the active RRD, and can add all
  sources when its global enable-all setting is active. A successful Forget
  receipt must not be rejected merely because recording reappeared.
- The pinned XenAdmin reference `35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`
  uses these recording methods in `XenModel/Actions/GetDataSourcesAction.cs`.
  Its NRPE properties invoke a separate `nrpe` host plugin in
  `XenModel/Actions/NRPE/NRPEHostConfiguration.cs`; a client-side version check does
  not establish plugin installation. Official [XCP-ng monitoring documentation](https://docs.xcp-ng.org/management/monitoring/)
  explicitly states that XCP-ng does not ship Nagios NRPE integration. This client
  reports that stock-target limitation; it does not invent plugin support or
  install dom0 monitoring packages.

VM/SR recording controls, custom NRPE deployments and external monitoring-agent
installation are not delivered by this host-only increment. Existing remote
syslog configuration remains separate. Persistence through real RRDD/host restarts
and actual host compatibility await the deferred disposable lab.

## Verification

Seven loopback protocol suites verify success, automatic standard-source
reappearance, archival-source disappearance, captured metadata/state changes,
read-only and RBAC refusal, malformed catalogs, stale observations, lost replies
without replay, private serialization and saved-receipt recovery after restart.
Three Rune tests exercise wide/compact review controls and history-loss
acknowledgement, immutable targets, refresh invalidation and captured-pool history.
An independent source review found no blocker in the backend receipt/identity
paths. The complete workspace passed 1,116 tests (486 core unit, 306 direct
protocol, 311 desktop, five profile, four TLS and four host-probe tests). Strict
workspace/all-target Clippy, formatting and whitespace checks passed. Four native
Vulkan renders cover catalog and deletion review at 1440×900 and 513×652; visual
inspection confirms reachable headings, controls and scrollable content. The
final synthetic scene refinements were followed by the three Rune tests and
strict Clippy again. These checks do not establish real-host or assistive-technology
qualification. No production host was contacted or mutated.

## Integrated verification

After merging this increment onto the 0.14 source checkpoint and advancing development to
0.15.0, the combined workspace passed **1,148 tests**: 505 core unit, 311 direct protocol,
319 Rune/desktop and 13 profile/TLS/probe cases. Strict workspace/all-target Clippy passed.
This integration does not change the live-host and desktop qualification limits above.
