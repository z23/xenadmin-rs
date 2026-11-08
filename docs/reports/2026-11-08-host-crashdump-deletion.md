# Reviewed retained host crashdump deletion

Implemented with synthetic protocol, journal and Rune fixtures. No real host request, live
infrastructure mutation, keyring access, private screenshot or support upload was used.

Host backup and support lists the captured host's retained crashdumps with UUID, timestamp
and size. Review deletion fixes that host and dump independently of later inventory selection,
explains the retention loss and requires the exact dump UUID before submission. Previously
downloaded local files and other retained dump records are outside the operation.

The core rechecks the captured pool and host, live host metrics, idle host operations and
exact crashdump UUID/owner/time/size before persisting dispatch and calling
`host_crashdump.destroy`. It uses the existing pool-wide resource reservation. A positive
synchronous reply is persisted separately from dispatch; successful completion additionally
requires a fresh check that the same UUID is absent. It reports API completion and record
absence, without claiming independent inspection of the host filesystem. A failed or lost
reply remains OutcomeUnknown even when the database record disappears; Recheck only reads
existing evidence and never repeats destruction. Late positive receipts preserve interruption
and acknowledgement while retaining evidence for later inspection.

The crashdump filename is internal-only in the target API model. The client does not invent
filename access or claim to inspect the host's `/var/crash` tree. A support log archive excludes
raw `coredump.bin`, and the API provides upload and destroy but no direct raw-dump download
method. Raw dump transfer remains an open retained workflow; deletion does not silently
upload diagnostic evidence to another service.

Primary evidence for XCP-ng 8.3:

- [XCP-ng xapi package](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
  pins the upstream v26.1.21 lineage.
- [Host crashdump implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_host_crashdump.ml)
  deletes a retained directory and destroys its database record in a finally block, even
  after filesystem deletion raises an error. A non-directory path can also leave no deleted
  directory despite API success. These semantics require the qualified outcome above.
- [API data model](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/datamodel.ml)
  defines UUID, host, timestamp and size, marks filename internal-only, and exposes destroy
  and upload rather than raw download.
- [Target status-report implementation](https://github.com/xenserver/status-report/blob/v2.0.15/xen-bugtool)
  excludes raw core content from the `host-crashdump-logs` category.

Synthetic checks cover successful completion, changed identity/owner/time/size, missing dumps,
offline or busy hosts, read-only/denied permissions, failed/lost replies after record removal,
retained records after positive replies, no-replay reconciliation, immutable journal identity,
fabricated completion refusal and late receipts after recovery or acknowledgement. Rune
checks cover captured targets, unadvertised selections, changed hosts and compact/wide review
footers. Live compatibility and actual host filesystem behavior remain disposable-lab gates.

Final validation passed all 877 workspace tests at default concurrency: 391 core unit,
243 direct protocol, 5 profile, 4 TLS, 230 Rune and 4 probe tests. Strict workspace/all-target
Clippy, release desktop compilation, formatting and diff checks passed. Native Vulkan renders
at 1440×900 and 513×652 were visually inspected using synthetic data. The catalogue pagination
fixture reaches record 65; private evidence tests reject fabricated completion and preserve
late receipts after recovery/acknowledgement. Independent source review found no remaining
backend blocker. UI review prompted the active-operation cap and access to every captured dump.

## Integrated verification

At combined source `56c789b`, all **921 workspace tests** passed: 412 core unit,
256 direct protocol, 240 Rune/desktop, five profile, four TLS and four probe cases.
Strict workspace/all-target Clippy passed. This includes existing management-topology,
alarm, crashdump, disk and deterministic cloud-upload recovery fixtures. No live host
changes were performed; disposable-lab qualification remains deferred.
