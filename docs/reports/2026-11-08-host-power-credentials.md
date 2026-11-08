# Host power-on credentials

This increment adds a reviewed IPMI configuration workflow to Host administration. It also
replaces known IPMI settings with disabled or Wake-on-LAN mode. The existing power-on action
uses the saved host method; saving settings does not power on a host or test a BMC.

## Primary evidence

The [XCP-ng 8.3 xapi package specification](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
inspected on 2026-11-08 identifies source version 26.1.21, requires `ipmitool`, and installs
`/etc/xapi.d/plugins/IPMI.py` and `power-on-host`. Its matching upstream
[power-on plugin](https://github.com/xapi-project/xen-api/blob/v26.1.21/python3/poweron/power-on.py)
reads `power_on_ip`, `power_on_user` and the UUID in `power_on_password_secret` for IPMI.
[IPMI.py](https://github.com/xapi-project/xen-api/blob/v26.1.21/python3/poweron/IPMI.py)
uses the LAN-plus transport. Legacy iLO and DRAC implementations are absent from this lineage;
custom plugin configuration remains outside this typed editor.

The [host setter](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_host.ml)
stores the supplied configuration directly and cleans the old configuration's password
Secrets. The desktop therefore creates a fresh Secret for each IPMI credential change and
checks that the previous Secret has no other host or storage consumer before replacement.
This differs from PBD provisioning, which duplicates password Secrets.

## Ownership and recovery

- The review captures host UUID, pool, connection generation and the exact supported current
  mode/configuration. Unknown keys and custom plugins are unavailable, not silently removed.
- The password is transient, masked in Rune and passed only to `secret.create`. The journal
  and public history contain UUIDs, operation provenance, settings and completion evidence;
  they contain no password value or password hash. No Secret value-reading method is used.
- A bounded Secret UUID baseline and an operation ownership marker prove a created identity.
  The host setter receives that UUID. Existing and new Secret consumers are checked through
  fresh bounded host/PBD/SR reads. The selected host's configuration and idle state are
  revalidated on the newer consumer-scan record as well.
- Each write is journaled before dispatch. Late completion/identity receipts merge monotonically
  without reviving a recovered or acknowledged operation. The shared pool admission guard
  serializes this workflow against VM and other resource writes in this client.
- A lost setting response stays unknown even if current fields match. Recheck reads existing
  evidence only; it never creates a Secret, repeats the setter or deletes credentials.
  Startup leaves uncertain host-power operations for explicit recheck.
- A created Secret whose setting write never started can be recovered as a partial outcome.
  A separate review may delete it only after the source operation is terminal, ownership is
  intact, no setter may still be in flight, and complete host/storage scans show no consumers.
  A lost setter receipt prevents cleanup even after acknowledgement. Successful provisioning
  keeps the Secret in use; it is not automatically deleted by the client.
- External clients can race XAPI configuration writes: XAPI provides no compare-and-swap for
  this setter. Fresh checks reduce the window but do not provide cross-client exclusion.

## Verification

Synthetic checks cover IPMI Secret provisioning and replacement, value-free journal/public
history, changed review, shared old Secret refusal, a change during the consumer scan,
missing creation/setting replies, explicit unused-Secret cleanup, new storage consumers,
read-only/generation guards and recovery/acknowledgement during a delayed Secret response.
Journal checks reject malformed plans, changed baselines, cleared receipts and unreceipted
success. Rune checks cover compact/wide review footers, captured identity, invalid input,
unknown configuration and password clearing when changing modes.

Final validation on this branch:

- Eight new host-power RPC cases, one journal regression and three Rune interaction cases pass.
- The first full workspace run exposed the separately owned cross-pool recheck race in
  `recorded_completion_survives_removed_task_and_reconnect_without_a_second_transfer`: a
  higher-revision interrupted/unknown publication was observed before success. Evidence is
  retained in `/tmp/xenadmin-host-power-workspace.log`; it was reported to the transfer owner.
- The cross-pool owner fixed that progress-publication race in `f4fed28`. The next run exposed
  an older resource-creation fixture that treated any newer revision (including `Verifying`)
  as a final result. Its wait now requires a newer terminal/unknown result; its exact unknown,
  no-created-resource and no-second-write assertions are unchanged.
- The final unfiltered workspace run passed all 702 tests: 324 core unit, 197 protocol,
  five profile, four TLS, 172 desktop and four probe tests. No tests were skipped.
  Final log: `/tmp/xenadmin-host-power-workspace-final.log`.
- Strict workspace/all-target Clippy, formatting and shell/diff checks pass.
- Native Vulkan renders at 1440×900 and 513×652 were visually inspected; the review text wraps
  and the footer remains visible. Scenes use synthetic data and contain no real host content.
- Independent source review found a selected-host freshness gap in the consumer scan; the
  same-record check and dedicated race regression fixed it. The follow-up review found no
  remaining blocker in the reviewed paths.

No real host, BMC, Secret Service, password change, power-on
or infrastructure modification was exercised. Physical IPMI reachability, host plugin health
and cross-client concurrency require the deferred disposable lab qualification.

Host/root password changes, host configuration backup/restore and crashdump/support-bundle
transfers remain retained work; this increment does not claim those workflows.
