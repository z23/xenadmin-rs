# Authenticated storage

This increment adds CHAP and mutual CHAP for LVM over iSCSI, ISO libraries over
explicit SMB 3.0, and authenticated creation of a missing host attachment. Inputs
remain bound to the selected host, SR, location and connection generation. Introducing
existing SR metadata and creating/plugging its host attachment remain separate reviews.
NFS uses the existing host/export access model; no unsupported NFS password setting
is invented. Automatic all-host repair and other storage drivers remain separate work.

Passwords and usernames are transient `StorageCredentials` inputs. Public history
contains the authentication kind and temporary Secret UUIDs, with no credentials,
private references or username digests. The private journal stores immutable intent,
Secret/task UUID baselines, username digests and monotonic receipts. It never stores password
values or hashes. Discovery can send transient credentials to the selected host's
`SR.probe`; those queries do not create persistent Secret objects.

Provisioning creates tagged bootstrap `secret` objects, dispatches one asynchronous
SR/PBD creation and records its task/result. Verification requires new object identities,
exact host/SR/configuration and new attachment-owned Secret copies. XAPI ignores the
`other_config` argument to `PBD.create`; the previous verification assumption is removed
and the fixture now models that behavior. Successful provisioning scans all bounded
PBD device configurations and SR storage-manager configurations before deleting only
unused operation-owned bootstrap Secrets. It never deletes the host-owned copies.

Lost Secret creation responses can recover a uniquely tagged new UUID without
continuing provisioning. Lost provisioning/task/cleanup responses remain explicitly
uncertain. Failed provisioning can leave resources and credentials, so failure does
not authorize automatic cleanup. Late receipts preserve recovered or acknowledged
states and cannot dispatch the next step. Explicit cleanup is a separate reviewed
operation: it requires the source row's ownership evidence, a stopped provisioning
outcome, exact Secret marker/UUID and no storage consumers. A missing provisioning
task receipt prevents cleanup even after acknowledgement.

The desktop provides optional CHAP/mutual CHAP and SMB credentials, a captured review,
password clearing after submission and retained-Secret cleanup reviews. Discovery
results are invalidated by credential, endpoint, host or generation changes. SMB
uses `vers=3.0`; it does not fall back to SMB1. Existing SMB1 or unspecified-version
attachment configurations are not silently converted into new SMB3 attachments.

Verification is synthetic: protocol fixtures cover CHAP/mutual/SMB SR and PBD creation,
server-side credential copies, ignored PBD metadata, secret privacy in public JSON and
SQL-visible journal/WAL payloads, read-only discovery, lost replies, wrong ownership,
consumer refusal, explicit cleanup and late recovery/acknowledgement. Journal fixtures
reject corrupt private intent and missing completion proof. Rune fixtures check captured
reviews, credential-result fencing and compact/wide footer reachability. Verification:

- Full workspace: 598 passing tests (284 core unit, 162 direct protocol, 139 desktop,
  five profile, four TLS and four probe tests).
- Final focused rerun after bounded successful-task cleanup: two auth journal and
  eight auth protocol tests pass.
- Strict workspace/all-target Clippy, formatting, diff and shell checks pass.
- Native synthetic review renders at 1440×900 and 513×652 were visually inspected;
  review text wraps, scrolling retains the compact footer, and demo confirmation stays disabled.
- A peer review identified a changed-between-reads attachment race; same-record
  host/SR/location/state and credential verification now covers it. Additional task
  UUID rebinding fixtures verify that cleanup cannot rely on a substituted task record.

No production connection or mutation was used. Real XCP-ng 8.3 CHAP arrays, SMB servers,
RBAC permissions, storage-driver errors and reboot/reconnect behavior still require the
prepared disposable lab. These fixtures do not establish storage health or server-side
credential logging behavior. The bounded Secret inventory is limited to 4096 records;
storage consumer scans are limited to 10000 PBD/SR records and 128 configuration fields.
Individual read/dispatch phases have a 60-second deadline; task polling stops after five
minutes with an explicit uncertain outcome.

Primary protocol evidence:

- [XCP-ng 8.3 iSCSI driver](https://github.com/xcp-ng/sm/blob/3.2.12-8.3-devel/drivers/BaseISCSI.py): outgoing and incoming CHAP fields and Secret UUID support.
- [XCP-ng 8.3 ISO driver](https://github.com/xcp-ng/sm/blob/3.2.12-8.3-devel/drivers/ISOSR.py) and [CIFS credential helper](https://github.com/xcp-ng/sm/blob/3.2.12-8.3-devel/drivers/cifutils.py): SMB3 configuration, explicit-version behavior and username/password handling.
- [XCP-ng XAPI package specification](https://github.com/xcp-ng-rpms/xapi/blob/master/SPECS/xapi.spec): upstream v26.19.0 base for the 8.3 package.
- [XAPI v26.19.0 Secret implementation](https://github.com/xapi-project/xen-api/blob/v26.19.0/ocaml/xapi/xapi_secret.ml) and [PBD implementation](https://github.com/xapi-project/xen-api/blob/v26.19.0/ocaml/xapi/xapi_pbd.ml): new Secret UUID duplication for `password_secret` keys, PBD-owned cleanup, and ignored PBD creation metadata.
- [Secret API](https://xapi-project.github.io/xen-api/classes/secret.html): creation, UUID/metadata inspection and deletion. The client never requests Secret values or complete Secret records.

Reference behavior was also checked in `XenModel/Actions/SR/SrCreateAction.cs` and the
IQN/LUN discovery actions. No reference source was copied. The reference client's
unconditional best-effort Secret cleanup after failure was deliberately not adopted,
because an uncertain asynchronous dispatch may still consume those credentials.
