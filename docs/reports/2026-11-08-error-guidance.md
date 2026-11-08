# Contextual host-error and outcome guidance — 2026-11-08

Activity task results, VM operation history and infrastructure operation history now offer
**Explain outcome**. The read-only dialog captures the task or operation identity, the original
bounded host code when available, the recorded outcome and specific inspection steps. It
keeps that captured context when inventory selection or the connected pool changes.

The offline catalogue covers common permission/login, expired-session, current-task,
power-state, guest-tools/cooperation, CPU/memory, host reachability/maintenance, SR capacity,
attachment/path, disk-consumer, network/management, migration compatibility, HA protection,
archive and unsupported-API failures. Numeric `SR_BACKEND_FAILURE` subcodes receive backend
inspection guidance without pretending to identify an array-specific cause. Unrecognized
codes remain visible with an explicit generic explanation. `UNKNOWN_API_ERROR` does not
establish an administrative restriction.

## Outcome and privacy boundaries

Unknown results lead with read-only rechecking and no repetition of the uncertain workflow.
Acknowledgement is described as a local guard release, not success or rollback. Partial and
cancelled results retain the need to inspect completed steps and resources before any new
review. Guidance never changes eligibility, invokes an operation, reconnects, releases a
reservation or automatically modifies HA, storage or guest state.

Tasks provide a typed error code. Existing journal rows do not have a structured error field,
so the compatibility adapter accepts only an entire application-owned `XAPI refused the
request: CODE` or `Host task failed: CODE` detail. If the overall detail includes additional
prose, an exact failed/unknown step detail can supply the code. It does not search arbitrary
message bodies, task descriptions or administrative block text for code substrings. Invalid,
oversized, mixed-case or parameter-bearing strings are not displayed as codes. Successful or
active entries do not gain failure advice from an old detail. New guidance contains only
static text and existing public identity fields; opaque task references and private plans
are not projected into it. Existing private export behavior is unchanged.

## Evidence

Error meanings were checked against the [pinned XAPI v26.1.21 error definitions](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/datamodel_errors.ml),
the source lineage already used for XCP-ng 8.3. The advice is original client text and
makes no claim that every host/storage-driver error is understood.

- **1,071 workspace tests passed**: 471 core, 289 direct-protocol, 298 Rune desktop,
  five profile, four TLS and four host-probe tests.
- Two new core groups cover bounded exact codes, numeric backend-family matching,
  unrecognized codes and uncertainty-aware guidance.
- Four new Rune groups cover exact-envelope recognition, failed-step fallback, unknown and
  acknowledged precedence, both history families and host tasks, compact/wide controls,
  captured context after selection changes, exclusion of private references, and no dispatch.
  Successful history and arbitrary message bodies do not expose error help.
- Strict workspace/all-target Clippy, formatting, whitespace checks and native build passed.
- Independent source review found no blocking outcome, classification, privacy or modal
  lifetime issue. Native Vulkan renders at 513×652 and 1440×900 were visually inspected:
  the outcome precedes advice, text wraps, and the Close control remains visible. Only
  synthetic scenes were rendered; no screenshot is committed.

All verification uses generated inventory and local fixtures. No production host was contacted
or changed. Live host compatibility and real Wayland/X11/Orca qualification remain deferred.
