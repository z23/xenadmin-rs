# Direct host performance metrics

The core's `Connection::subscribe_metrics` reads VM or host performance through the
existing authenticated connection. It accepts an observation minted by the current
inventory and stops when the view drops its handle, the connection closes, or the
connection generation changes. It also works on read-only connections. Metrics never
open the mutation journal or enable a datasource on the server.

`VM.get_data_sources` and `host.get_data_sources` supply names, descriptions, units and
recording state. The backend retains raw values and units. An explicit selection accepts
up to 32 distinct source names; the default selects up to eight enabled sources in each
CPU, memory, disk and network category. Other sources remain available in the metadata.
Missing or disabled selections produce unknown samples rather than zeroes.

## Windows and sampling

| Window | Requested consolidation interval |
| --- | --- |
| Ten minutes | 5 seconds |
| Two hours | 60 seconds |
| One week | 3,600 seconds |
| One year | 86,400 seconds |

Every request selects `cf=AVERAGE`. These intervals follow the documented archive
resolutions; the actual returned step is exposed as `sample_step_seconds`. A sample's
timestamp marks the end of its consolidation interval, not a continuously observed
instant. The backend does not combine sources with different units or convert fractions
to percentages.

The initial `start` is the negative window duration, interpreted by RRDD relative to
its own most recent update. This avoids assuming the workstation clock equals the
host clock. Later requests overlap the previous end by one requested interval and merge
by timestamp. A changed host or sampling step replaces the previous window. Returned
samples determine `range_start` and `range_end`; shorter retention stays shorter, and
missing rows are not filled with fabricated values. NaN, infinity and null remain gaps.
`updated_at_ms` records local retrieval time, separately from sample timestamps.

## Transport and limits

The backend requests `/rrd_updates` with both `Accept: application/json` and `json=true`.
VM requests use `vm_uuid=<selected UUID>` and `host=false`; host requests use
`vm_uuid=none` and `host=true`. Both exclude storage-repository streams with
`sr_uuid=none`. The older generated XenAdmin HTTP wrapper's `uuid` parameter does not
filter this endpoint; the actual RRDD handler uses `vm_uuid`.

VM residency is resolved before each request and checked again before publishing the
response. A migration during a fetch discards that response and retries against the
new verified member. TLS policy, custom CA and per-endpoint pins come from the current
connection. Redirects must stay on the verified pool's origins, retain the scheme and
port, and use the metrics path with no user information or fragment. Redirect query
parameters are discarded; the original target and session parameters are rebuilt.
Transport errors expose sanitized application errors, not response bodies or token URLs.

Limits are four live subscriptions per authenticated pool owner; 512 datasource records;
32 selected series; an 8 MiB HTTP body; 2,048 response columns and rows; and 2,048 retained
samples per series. Each HTTP request has a 15-second deadline through body completion,
with at most four vetted hops. Polling is every 5–60 seconds according to the selected
window. Transient failures preserve previous data with a stale state and back off up to
30 seconds. Protocol and authorization errors stop the subscription with an explanation.

This increment reads archives available through RRDD's update export. It does not
implement full `/vm_rrd` or `/host_rrd` archive download/unarchive fallback. If a halted
VM's archive is not available in the coordinator's update export, that historical view
can be unavailable. Datasource recording configuration, alert configuration and chart
export are separate workflows.

## Primary references

- [XAPI metrics guide](https://xapi-project.github.io/xen-api/metrics.html): archive
  granularity, host placement, delta requests, consolidation and JSON negotiation.
- [RRDD HTTP handler](https://github.com/xapi-project/xcp-rrdd/blob/master/bin/rrdd/rrdd_http_handler.ml):
  `vm_uuid`, `host` and `sr_uuid` filters and explicit exclusions.
- [RRD update implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/libs/xapi-rrd/lib/rrd_updates.ml):
  JSON object rows, string-valued samples, consolidation timestamps and relative starts.
- [XenAdmin MetricUpdater](https://github.com/xcp-ng/xenadmin/blob/master/XenModel/XenSearch/MetricUpdater.cs):
  resident-host/coordinator selection and delta sampling behavior.

Protocol, authorization, bounds, redirect, cancellation, migration and generation checks
use synthetic loopback fixtures. They do not establish compatibility with every live
XCP-ng release or authorize changes to any host.
