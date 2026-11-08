# Direct-host performance metrics

VM and host Details now offer **Performance**. The view presents CPU, memory, disk and
network sources reported by the host, with selectable data sources and four historical
ranges. Each graph labels its units, actual returned time range, sampling interval and
missing samples. Fraction units are displayed as percentages; missing or non-finite values
remain gaps. The source picker allows at most 32 series. Closing the view or navigating to
another object drops the subscription.

Metrics use the existing authenticated pool session and its endpoint trust policy. The
backend resolves the target host, filters `/rrd_updates` to the requested identity, and
rechecks residency before publishing a response. Redirects must name known pool endpoints;
session-bearing URLs are not exposed in diagnostics. Subscriptions are separate from the
inventory writer and operation journal. Errors affect the performance view rather than
blocking management. Sample history, response size, series counts and subscriptions are bounded.

The transport follows the [XAPI metrics documentation](https://xapi-project.github.io/xen-api/metrics.html)
and the upstream RRDD JSON format. Host retention determines actual historical coverage;
requesting a year does not imply a year of samples exists. This increment reads existing
data sources and does not enable host recording or modify alarm settings.

Three Rune tests exercise historical controls/source selection, UTC date formatting,
percentage scaling, true zero values, gaps and finite coordinates for extreme values.
Synthetic views at 1440×900 and 513×652 were rendered and visually inspected. Backend
fixtures cover the RRD decoder, target routing and subscription lifecycle; their final
integrated totals are recorded with the release checkpoint.

Live host compatibility, retention behavior, large-pool performance and native accessibility
remain unqualified. Saved custom graph presets, alarm configuration and report exports
remain open in the [retained parity ledger](../PARITY-IMPLEMENTATION.md).
