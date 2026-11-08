# Pool authentication, access and certificates

This increment adds direct-XAPI security workflows to the Rune desktop. Tests use
synthetic local servers, generated certificates and fake credentials. No real host,
directory, certificate installation or keyring was accessed. Live qualification
remains deferred to the prepared lab.

Open **Manage pool → Pool policies, HA and membership → Authentication, access and
certificates**. The workspace captures its pool and connection generation. Reviews
identify the affected directory, subject/role or host and require the pool UUID.

## Implemented

- Read-only discovery shows each member's directory configuration, session-local
  superuser status, advertised security methods, explicit RBAC availability,
  subjects and assignable roles. Missing or malformed sections have visible errors.
- Join an unconfigured pool to Active Directory with transient account credentials.
  Leave can remove local configuration only, or also request removal of directory
  machine accounts using credentials. The UI distinguishes these outcomes.
- Resolve a directory user/group before adding access. New subjects start with no
  roles; grant a role separately. Creation refuses unknown or restricted RBAC:
  upstream can silently assign pool-administrator access on restricted servers.
  Subject removal and role edits compare fresh identifiers and the exact reviewed
  role set. Leaf permission records are not offered as user roles.
- Inspect host certificate SHA-256 fingerprints, install a PEM certificate with a
  matching unencrypted PKCS8 private key and optional chain, or request a new
  self-signed certificate. Native file pickers and bounded background validation
  keep the UI responsive. The private key is never shown in a review.

All mutation paths currently require a local superuser session. Discovery remains
available through read-only connections, subject to the server's permissions.
Restricted/unknown RBAC disables subject creation and role changes. Dedicated
reconnect-as/elevation and general per-method RBAC eligibility remain open.
Certificate trust changes are independent: the application never silently replaces
saved pins or disables validation following certificate installation/reset.

## Ownership, privacy and recovery

`PoolSecurityOperation` contains only the reviewable intent. A separate
`PoolSensitiveChange` carries credentials or a validated `ServerCertificate`;
neither transient type supports serialization or cloning, and their debug output
is redacted. The private key uses zeroizing storage. The public operation record,
SQLite payload and inspector/export projections contain no passwords, PEM keys,
session references or opaque returned subject references.

Every command uses the existing pool-wide write reservation, fresh pool/member
checks and durable dispatch/completion receipt. A failed or lost reply stays
unknown: directory changes can partially succeed and certificate changes can
interrupt the connection. Matching final state alone does not prove completion.
An accepted receipt permits read-only verification after reconnect without the
original credentials. No mutation is replayed, no automatic directory leave or
rollback occurs, and saved trust is unchanged. New-subject verification checks the
returned identity against a pre-dispatch UUID baseline and verifies empty roles.

Discovery has a 45-second deadline, bounded records/text, a shared four-worker
limit, and cancellation on generation change or handle drop. File validation
accepts only regular files, refuses symlinks, caps each file's bytes and uses one
background worker. Late native-picker or worker results cannot retarget another
editor or host. Password fields are masked and cleared on submission/invalidation.

## Verification

Protocol tests cover read-only discovery, exact directory arguments, missing
privileges/methods, changed members, partial and lost replies, receipt recovery,
reconnect without credential replay, restricted RBAC, unexpected automatic roles,
leaf permission rejection, role conflicts, certificate replacement/reset,
fingerprint conflicts and database/history secret absence. Unit tests cover PEM
key matching, multiple-key rejection and bounded/ambiguous inputs. Rune tests cover
compact/wide fixed review footers, immutable targets, late picker/validation
responses, regular-file checks and disabled demo submission.

Validation: 364 workspace tests passed (4 probe, 166 core unit, 5 connection
profiles, 97 direct RPC, 4 TLS and 88 desktop), including 12 new security RPC,
2 security unit and 4 Rune/file-picker tests. Strict workspace/all-target Clippy,
formatting and diff whitespace checks passed in an isolated build directory.
Wide security overview and compact directory-review native synthetic scenes were
rendered and visually inspected under `/tmp/xenadmin-security-review-artifacts/`,
outside the repository. These checks do not establish real directory compatibility,
certificate reconnect behavior, Wayland input or accessibility qualification.

## Primary evidence and remaining work

The [pool API](https://xapi-project.github.io/xen-api/classes/pool.html),
[auth API](https://xapi-project.github.io/xen-api/classes/auth.html),
[subject API](https://xapi-project.github.io/xen-api/classes/subject.html),
[role API](https://xapi-project.github.io/xen-api/classes/role.html) and
[host certificate API](https://xapi-project.github.io/xen-api/classes/host.html)
define the methods and argument shapes. The
[upstream subject implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_subject.ml)
establishes automatic pool-administrator assignment when RBAC is restricted;
XenAdmin's `Host.RestrictRBAC` reads the corresponding `license_params` flag.
Reference `EnableAdAction`, `DisableAdAction`, `AddRemoveSubjectsAction`,
`AddRemoveRolesAction` and `InstallCertificateAction` were consulted; no source
was copied and the reference client's automatic AD compensation was not adopted.

[XCP-ng 8.3 release notes](https://docs.xcp-ng.org/releases/release-8-3/) describe the
PBIS-to-winbind AD transition, so AD is capability-qualified rather than claimed
removed. The [XCP-ng TLS guide](https://docs.xcp-ng.org/guides/TLS-certificates-xcpng/)
documents certificate installation/reset and pool verification. SHA-256 is computed
from the returned leaf DER rather than assuming the legacy Certificate fingerprint
field uses SHA-256.

Pool-to-pool join still needs independently verified destination trust/identity and
coordinated reservations across both pools; it is separate from directory join.
Coordinated dissolve, advanced HA controls, general RBAC eligibility/elevation and
live qualification remain open in the parity ledger.
