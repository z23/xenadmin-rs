# Session permission eligibility — 2026-11-08

The client now distinguishes a known API permission denial from unavailable permission
information. Every new authenticated session reads its own effective permission list and
local-superuser flag. Single-VM and selected-VM power controls disable known-denied actions
with an explanation. Connections show the effective permission count or local-superuser
status and any recognized session role labels. Existing reconnect-as remains the explicit
way to use another account; no automatic role grant, credential elevation or saved-account
change is performed.

## Authority and currentness

Ordinary RPC calls check the exact current session's permission list before sending a
known-denied method. Asynchronous calls use the synchronous permission name and class names
are normalized to lowercase. Role names are not ranked or used as a substitute for effective
permissions. A grant is only one prerequisite: inventory restrictions and server rejection
remain authoritative. Session discovery does not call through its own permission guard.
Authentication/session-invalid and certificate/coordinator errors retain their existing
recovery handling.

The permission set belongs to one session and is replaced after every new login. Connection
snapshots hide it when stale, disconnected or renewing and clear it on a generation change.
Unsupported methods, discovery permission denial and malformed data produce explicit unknown
information, not a fabricated read-only session. Such calls still reach normal server
authorization. Empty valid permission sets represent known denials; an explicit local
superuser flag grants the ordinary RBAC prerequisite even when its permission list is empty.

Protected map-key add/remove calls are deliberately unknown at the local guard. The current
session's own permission list cannot identify keys restricted to other roles, and an ordinary
method grant does not establish access to a protected key. The host checks these calls.
Direct HTTP transfer/console permissions also remain server-enforced; they are not inferred
from an unrelated RPC permission. No raw session reference, subject SID or server error
context is exposed by the new projection.

## UI boundary at this checkpoint

The later [review-requirements increment](2026-11-08-permission-reviews.md) adds upfront
checks to the ordinary administration reviews described below.

Single and bulk VM power have upfront known-denial controls. Other ordinary RPC workflows
receive the central backend guard and retain their existing step failure/partial/unknown
reporting. Advanced VM, device, storage, network, host and pool review dialogs do not yet
calculate a complete permission requirement for every later step. A known denied call in
such a workflow can stop it after earlier steps have completed; no rollback or retry is
implied. Those review requirements and direct HTTP eligibility remain in the ledger.

## Verification

- 723 workspace tests passed: 333 core unit/protocol, 194 direct connection, 183 desktop,
  five profile, four TLS and four host-probe tests.
- Three new unit tests cover exact Async normalization, no inferred authority from role
  labels, explicit unknown/local-superuser behavior, protected keys and stale generation
  publication.
- Five new protocol tests cover denial before power dispatch, host refusal despite a known
  grant, unsupported/denied/malformed discovery, changed permissions after renewal and
  session expiration during permission discovery. Existing fixture servers now explicitly
  answer the added own-session discovery read.
- Two Rune interaction tests cover compact/wide denial controls, visible explanations,
  unknown information reaching ordinary eligibility checks, and grants not overriding an
  ineligible object. No new screenshot scene is needed for these existing controls.
- Strict workspace Clippy, workspace build, formatting and whitespace checks passed.

All checks use synthetic/loopback data. No production host was contacted and no private
screenshots or inventories were published. Actual restricted-role behavior on a disposable
XCP-ng pool and native desktop qualification remain open.

## Primary semantics

The XCP-ng [8.3 RPM source specification](https://github.com/xcp-ng-rpms/xapi/blob/8.3/SPECS/xapi.spec)
currently selects XAPI v26.1.21. Its [RBAC implementation](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/rbac.ml)
checks effective session permissions, treats local superuser specially and substitutes
protected-key permission names. Its [dispatcher generator](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/ocaml_backend/gen_server.ml)
removes the asynchronous prefix before that check. The
[session data model](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/idl/datamodel.ml)
defines the permission list as static, and the
[reference client](https://github.com/xcp-ng/xenadmin/blob/master/XenModel/XenAPI/Session.cs)
likewise uses effective session permissions rather than role labels to determine authority.

This does not claim every deployment supports changing subject roles. Upstream
[subject operations](https://github.com/xapi-project/xen-api/blob/v26.1.21/ocaml/xapi/xapi_subject.ml)
gate role assignment on the RBAC feature and can otherwise give a newly created subject
the pool administrator role. Existing security workflows continue refusing role creation
when support is restricted or unknown. Effective session permission discovery neither
enables that feature nor changes its policy.
