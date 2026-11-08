# Pool root password changes

This increment implements P12-10 for the local root account, with a Rune pool-level
review and a Rust operation journal. It has synthetic verification only. No real
host, pool password, BMC or desktop keyring was changed.

## Behavior

- The review captures the pool UUID, coordinator UUID and complete member UUID set.
  The operator enters the new password twice, then confirms the pool UUID. Passwords
  are masked, absent from the review, zeroized when cleared, and never serialized.
- A fresh own-session record must prove local-superuser authority; the connection
  account must be root and writable. Membership and coordinator are checked again
  before a single synchronous `session.change_password` call.
- A completion receipt is saved before a new root login to the exact authenticated
  coordinator endpoint. The new session must identify the same pool and membership.
  Its TLS, endpoint pinning and proxy configuration are the existing connection's.
- Verified completion replaces the canonical connection's reconnect credential and
  rekeys its pool authority. Connections sharing the old password are invalidated;
  dropping those aliases does not disconnect the canonical connection. New aliases
  must authenticate with the new credential and matching trust/access configuration.
- The server requests password-file synchronization on the other pool members but
  suppresses individual failures. Multi-member outcomes remain partially completed,
  with those member UUIDs explicitly unverified. Coordinator login is not evidence
  that another member's password changed.
- Updating the existing saved root password for the captured connection profile is
  an unchecked opt-in. It requires positive, uninterrupted completion and unchanged
  profile/settings. Other saved profiles are not rewritten. A locked/unavailable
  keyring is reported separately from the completed host change, without retries.

## Recovery and privacy

The private immutable plan contains only typed identities and boolean receipts. No
old/new password, password hash, opaque session reference or login argument is stored
in the journal or public history. XCP-ng ignores the old-password parameter for this
local-superuser operation, so the client sends an empty value and never asks for an
additional copy of the current password.

A lost change response remains unknown. Matching state, a new login or an operator
acknowledgement cannot manufacture its missing completion receipt. Explicit recheck
reads existing evidence; it never repeats the change or saves a password. A late
positive receipt is retained without reviving an interrupted or acknowledged worker.
The durable proof check and local credential replacement share a SQLite immediate
transaction, excluding recovery/acknowledgement from that handoff. Saved-credential
updates also reject interrupted positive evidence and acknowledged/unknown states.

## Primary evidence

- [XCP-ng 8.3 xapi package specification](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
  identifies xapi 26.1.21. The corresponding upstream tag includes the leading `v`.
- [XAPI v26.1.21 session implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_session.ml)
  defines the local-superuser check, ignored old password, local root update and
  best-effort `host.request_config_file_sync` calls with per-member exception handling.
- The reference client's `XenModel/Actions/Host/ChangeHostPasswordAction.cs` changes
  the session password and replaces its retained connection password. This client
  additionally records uncertainty and verifies coordinator login before rotation.

## Verification

Focused synthetic checks cover coordinator verification and reconnect, old-alias
invalidation and new-alias authority, unverified members, lost responses without replay,
changed membership/nonlocal-user refusal, recovery/ack during a delayed response,
immutable journal scope and receipt merging, transaction fencing, narrow/wide Rune
review, stale/mismatched input clearing, keyring failure and refusal to save late
positive evidence. Fake keyring adapters are used throughout.

All 736 unfiltered workspace tests passed: 333 core unit, 202 direct protocol,
5 saved-profile, 4 TLS, 188 Rune/desktop and 4 host-probe tests. The new focused
subset contains 2 journal, 5 protocol and 4 Rune/fake-keyring cases. Strict workspace
all-target Clippy, formatting, shell syntax and diff checks passed. Native Vulkan
synthetic review renders at 1440×900 and 513×652 were inspected; confirmation and
footer remain visible, and no password is rendered in the review.

Independent source review accepted the transactional local-rekey fence and the
positive-uninterrupted-completion gate for saved credentials. Tests initially exposed
fixture-only issues (a 60-second default mock event poll and the exact acknowledged
state); those fixtures now model bounded event polling and assert AcknowledgedUnknown,
without relaxing uncertainty or no-replay requirements.
Live coordinator password changes, actual pool-member synchronization, native Secret
Service behavior and recovery with a real host remain deferred to the prepared lab.
