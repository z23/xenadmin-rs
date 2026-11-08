# Saved graph presets and captured resource reports

Performance views can save, apply and remove up to 32 named graph presets on this
computer. Each stores the VM/host target type, time window and selected data-source
names. Applying a preset retains the current target and refuses missing advertised
sources instead of silently substituting a graph. Invalid definitions cannot replace
existing presets; older preference files retain their existing defaults.

Search all pools can export its captured filtered results as versioned JSON. The
report includes pool-scoped resource UUIDs, names, descriptions, host/power details,
tags, folders and custom fields, plus filter and freshness/coverage metadata.
Unavailable connections and incomplete metadata remain explicit. It is a private
inventory report, not a support bundle or a complete account of inaccessible objects.

Exports refuse more than 10,000 rows or 8 MiB rather than truncating. The native Save
dialog captures the report before opening, so later selection/inventory changes cannot
retarget the export. Cancelled or obsolete workspace callbacks do not write files.
Publication uses the existing background private-file writer: mode 0600, no overwrite
and no symlink replacement. No credentials, sessions or raw RPC records are exported.

## Verification

- All 172 desktop tests passed on this increment's base; six new behavior tests cover
  preset persistence/validation, missing-source refusal, target retention, report scope
  and limits, native picker cancellation and obsolete callbacks, captured results and
  private file permissions.
- Formatting and strict workspace/all-target Clippy passed; native desktop build passed.
- Synthetic 1440×900 and 513×652 graph-preset and cross-pool search renders were
  generated and inspected. Compact views retain visible controls and scrolling results.

No live host operation, private inventory or user screenshot was used. Native portal,
real metrics compatibility and clean-system qualification remain open.
