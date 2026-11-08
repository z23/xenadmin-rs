# Performance alarm and email configuration

This increment implements retained row P14-11 through direct XAPI. It adds reviewed performance-alarm thresholds, duration/repeat intervals, pool email settings and explicit monitoring-service refresh requests. It has synthetic protocol, journal and Rune verification; actual host evaluation and SMTP delivery remain lab qualification work.

## Operator workflow

Open **Alarms**, search the captured pool's VMs, hosts or storage repositories, and choose a resource. The editor exposes ordinary VM CPU/network/disk alarms, host CPU/network/free-memory alarms, the host's exact control-domain memory alarm, and storage throughput per host. CPU/memory fractions are displayed as percentages; other thresholds show their native units. Free host memory triggers below its threshold; the other supported metrics trigger above. Enable, edit or explicitly disable one alarm, then review its original pool/object identity before submission.

**Pool email** configures one destination address, SMTP relay host/optional port and one of the host's shipped English/Japanese/Simplified Chinese message languages. An empty destination disables delivery. Existing sender, SMTP authentication and other unrelated settings are retained. This workflow does not send test email or establish delivery.

**Refresh service** reviews the exact captured host set, then asks each live host's installed `perfmon` plugin to send its existing service a refresh notification. Positive acceptance is distinct from running service health, alarm evaluation and delivery. There is no install/start fallback. A failed host can leave earlier hosts notified; a lost response is not replayed. Without this request the target monitor normally reloads configuration periodically, up to 30 minutes later.

## State and privacy

`AlarmOperation` is a typed resource command. Observations bind the pool, object and connection generation; the UI keeps its captured review across selection changes and rejects stale confirmation. The worker performs fresh UUID/type/control-domain ownership checks and compares a digest of the reviewed settings before each setter. Unrelated metadata is taken from the latest record rather than from an old UI copy. Known session permission denials prevent the first write.

The bounded XML reader rejects DTDs, duplicate definitions, ambiguous nested host parameters and unsupported structure. Existing custom sources/directions cannot be overwritten as ordinary thresholds; explicit removal is supported when the definition is inspectable. Other variables, priority parameters and unknown extensions are preserved. Raw XML, custom source expressions and unrelated SMTP credentials are excluded from the public inventory and operation intent. Only the projected settings and revisions cross the facade.

XAPI's whole-map setters provide no compare-and-swap. A fresh pre-dispatch read narrows the external-client race but cannot make concurrent changes atomic. Readback verifies the chosen metric/configuration and rejects an unexpectedly customized source. It does not prove exclusive authorship, retention of a concurrent external edit, a running monitoring daemon or mail delivery. Lost setter replies remain unknown until explicit read-only reconciliation establishes the requested postcondition.

Refresh acceptance receipts are durable, monotonic, bound to reviewed hosts and dispatched step positions. Missing receipts cannot be inferred from matching configuration. Reconciliation never repeats the plugin call; positive late receipts preserve interrupted/acknowledged operation state.

## Source evidence

The implementation uses the primary XCP-ng 8.3 XAPI revision `v26.1.21` and the reference XenAdmin behavior:

- [Host perfmon implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/python3/bin/perfmon): metric names/defaults/units, descendant XML parameter lookup, evaluation and configuration reload intervals.
- [Refresh plugin](https://github.com/xapi-project/xen-api/blob/v26.1.21/python3/perfmon/perfmon): refresh notification and acceptance semantics; no service-start operation.
- [Alarm mail helper](https://github.com/xapi-project/xen-api/blob/v26.1.21/python3/libexec/mail-alarm): pool `mail-destination`, `ssmtp-mailhub`, `mail-language` and preserved SMTP overrides.
- Reference XenAdmin `PerfmonDefinition`, `PerfmonDefinitionAction` and `PerfmonAlertEditPage`: per-object alarm editing and control-domain ownership. The reference's hidden physical-utilization control is not represented as a supported ordinary alarm.

Peer review found two meaningful mismatches before integration: the initial reader did not account for the host's descendant XML lookup, and the initial readback did not reject an unexpectedly customized source. Both were corrected with nested-parameter and post-set source-change regressions.

## Verification

The source workspace passed 847 tests: 371 core unit, 242 direct-protocol, 221 Rune, 5 profile, 4 TLS and 4 probe. Strict workspace/all-target Clippy passed. New checks cover XML bounds/custom preservation/injection refusal, control-domain and storage projection, stale identity/type/permissions, unchanged SMTP authentication, no implicit email/refresh request, positive/negative/lost refresh responses, read-only reconciliation of a lost setter response, and changed alarm source at verification. Durable receipt tests reject rollback, unreviewed/duplicate hosts and receipts for an undispatched step. Rune tests exercise percentage conversion, captured pool review, disabled synthetic confirmation, custom-source refusal and explicit disable, plus delivery/refresh wording.

Wide and compact native synthetic renders of the threshold editor, captured review and pool email form were inspected. All screenshots use fabricated inventory. Native input/accessibility and actual alarm timing, service compatibility, RBAC, email delivery and concurrent-client behavior remain live qualification work in the prepared disposable lab; no production mutations were performed.
