//! The carried-over logic of Rune `pool_security_view.rs`: the security
//! state of one pool as an asynchronous query reads it, the drafts, the
//! staged review, and the submission.
//!
//! One [`Editor`] as in Rune. It is the state behind the Users tab of its
//! pool (overview, the chosen subject, the filter) and, while
//! [`Editor::dialog`] is set, the one change that is open as a dialog. The
//! functions keep their Rune names and their texts word for word (`open`,
//! `query`, `poll`, `stage`, `directory`, `apply`, `select_host`,
//! `choose_file`, `load_files`, `read_file`, `review_lines`).
//!
//! What differs from Rune, and why:
//!
//! - The editor follows the pool of its target (`inventory_for`,
//!   `snapshot_for`, `connection_for`), not the scope session.
//! - `open` in a synthetic pool asks no host (Rune asked, failed, and its
//!   scene removed the error by hand).
//! - `apply` looks the connection up before it takes the password or the
//!   certificate out of the draft, and a submission that fails after they
//!   were taken returns to the draft: Rune left the review staged, and a
//!   second Confirm of a directory change would have sent an empty password
//!   (packet hazard 41).
//! - After a submission the dialog closes and the tab stays. The overview is
//!   read again once the operation has ended ([`Editor::pending`]); Rune
//!   closed the whole editor and jumped to the pool history.
//! - What Rune disabled without a word has a reason ([`gate`]).

use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

use xenadmin_core::{
    CertificateFingerprint, Credentials, DirectorySubject, HostId, Inventory, ObservedResource,
    OperationId, PoolId, PoolOperation, PoolSecurityDiscovery,
    PoolSecurityOperation as SecurityOp, PoolSecurityQuery, PoolSecurityResult,
    PoolSensitiveChange, ResourceCommand, ResourceOperation, ResourceTarget, SecurityOverview,
    ServerCertificate, SubjectId,
};

use crate::prelude::*;

/// Rune `poll`, on a connection change.
pub const INVALIDATED: &str = "Connection changed; close and reopen security before continuing";
/// Rune `query`, without a current observation of the pool.
pub const REOPEN: &str = "Reconnect and reopen security to refresh its pool identity";
/// Rune `query`, without a connection.
pub const CONNECT: &str = "Connect to inspect pool security";
/// Rune `apply`: demo, a connection that cannot write, a stale observation.
pub const CHANGED: &str = "The connection changed; reopen security";
/// Rune `apply`: the typed confirmation is not the pool's UUID.
pub const CONFIRMATION: &str = "Enter the pool UUID shown in the review";
/// Rune `directory`.
pub const CREDENTIALS: &str = "Enter directory credentials for this change";
/// Rune's caption of a session that is not the local superuser; here also
/// the reason of every change such a session cannot make.
pub const READ_ONLY: &str = "Read-only security inspection. Reconnect as local root to change authentication or certificates.";
/// Rune's caption of a local superuser session.
pub const SUPERUSER: &str = "Local superuser session · security changes available";
/// Rune's three captions about role-based access.
pub const RBAC_AVAILABLE: &str = "Role-based access is available on every member.";
pub const RBAC_RESTRICTED: &str = "Role-based access is restricted. Adding subjects is disabled because the server may assign full administration automatically.";
pub const RBAC_UNKNOWN: &str = "Role-based access availability is unknown. Adding subjects and changing roles are disabled.";
/// Rune's two progress captions.
pub const READING: &str = "Reading security state…";
pub const VALIDATING: &str = "Validating certificate and private key…";

// Reasons Rune did not give (its buttons were disabled without one).
/// No overview has been read for the pool.
pub const UNREAD: &str = "Security status has not been read. Refresh security status first.";
pub const HOSTS_UNKNOWN: &str = "The directory state of the pool members is unknown.";
pub const ALREADY_JOINED: &str = "A pool member already uses external authentication.";
pub const NOT_JOINED: &str = "Not every pool member is joined to Active Directory.";
pub const NO_SUBJECT: &str = "Select a directory user or group first.";
pub const NO_ROLES: &str = "The pool reported no roles.";
pub const UNRESOLVED: &str = "Resolve the directory identity first.";
pub const NO_FINGERPRINT: &str = "The current certificate of the server has not been read.";
pub const NO_CERTIFICATE: &str = "Choose and validate the certificate files first.";
pub const SAME_CERTIFICATE: &str = "The replacement is the certificate the server already has.";

/// The change that is open as a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Join,
    Leave,
    AddSubject,
    RemoveSubject,
    Role,
    InstallCertificate,
    ResetCertificate,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Join,
        Kind::Leave,
        Kind::AddSubject,
        Kind::RemoveSubject,
        Kind::Role,
        Kind::InstallCertificate,
        Kind::ResetCertificate,
    ];

    /// The title of the dialog: the core's label of the operation
    /// (`PoolSecurityOperation::label`), which Rune had as the heading of
    /// the review. The two role changes share one dialog.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Join => "Join Active Directory",
            Kind::Leave => "Leave Active Directory",
            Kind::AddSubject => "Grant directory subject access",
            Kind::RemoveSubject => "Remove directory subject access",
            Kind::Role => "Change subject roles",
            Kind::InstallCertificate => "Install host TLS certificate",
            Kind::ResetCertificate => "Reset host TLS certificate",
        }
    }

    pub fn surface_id(self) -> &'static str {
        match self {
            Kind::Join => "pool-security-join",
            Kind::Leave => "pool-security-leave",
            Kind::AddSubject => "pool-security-subject-add",
            Kind::RemoveSubject => "pool-security-subject-remove",
            Kind::Role => "pool-security-subject-role",
            Kind::InstallCertificate => "pool-security-certificate-install",
            Kind::ResetCertificate => "pool-security-certificate-reset",
        }
    }

    /// The two changes of one server's certificate.
    pub fn certificate(self) -> bool {
        matches!(self, Kind::InstallCertificate | Kind::ResetCertificate)
    }

    /// A change with nothing to enter opens on its review and has no Back.
    pub fn opens_on_review(self) -> bool {
        self == Kind::RemoveSubject
    }
}

pub struct Editor {
    /// The instance, compared by the replies of file prompts.
    pub(super) epoch: uuid::Uuid,
    pub(super) invalidated: bool,
    pub pool: PoolId,
    pub name: String,
    pub observed: Option<ObservedResource>,
    pub inventory: Arc<Inventory>,
    pub overview: Option<SecurityOverview>,
    pub query: Option<PoolSecurityDiscovery>,
    pub domain: String,
    pub username: String,
    pub password: SecretText,
    /// The filter of the subject grid and the name to resolve, as in Rune.
    pub subject_name: String,
    /// (query text, identifier, display name)
    pub resolved: Option<(String, String, String)>,
    pub subject: Option<SubjectId>,
    pub host: Option<HostId>,
    pub fingerprint: Option<(HostId, CertificateFingerprint)>,
    /// Certificate, private key, optional chain.
    pub files: [Option<PathBuf>; 3],
    pub file_revision: u64,
    pub file_result: Option<mpsc::Receiver<Result<ServerCertificate, String>>>,
    pub certificate: Option<ServerCertificate>,
    pub review: Option<SecurityOp>,
    pub confirmation: String,
    pub error: Option<String>,
    /// The change that is open as a dialog; `None`: only the tab.
    pub dialog: Option<Kind>,
    /// A submitted change: when it ends the overview is read again.
    pub pending: Option<OperationId>,
}

fn editor(m: &mut Model) -> Option<&mut Editor> {
    m.areas.pool_security.editor.as_mut()
}

/// What the pool is observed as now.
fn observe(m: &Model, pool: PoolId) -> Option<ObservedResource> {
    m.snapshot_for(pool)
        .and_then(|v| v.observe_resource(ResourceTarget::Pool(pool)).ok())
}

pub fn current(m: &Model, e: &Editor) -> bool {
    e.observed.is_some() && observe(m, e.pool) == e.observed
}

/// Why a query of this pool could only fail: the texts of Rune `query`. A
/// synthetic pool has no observation, so it is refused with the first.
pub fn refusal(m: &Model, pool: PoolId) -> Option<&'static str> {
    if observe(m, pool).is_none() {
        Some(REOPEN)
    } else if m.connection_for(pool).is_none() {
        Some(CONNECT)
    } else {
        None
    }
}

/// Captures the pool and asks for its overview. Returns whether an editor
/// is open afterwards. It replaces an editor that was open: this is also
/// "close and reopen", the Refresh of the tab.
pub fn open(m: &mut Model, pool: PoolId) -> bool {
    let Some(inventory) = m.inventory_for(pool) else {
        return false;
    };
    m.areas.pool_security.editor = Some(Editor {
        epoch: uuid::Uuid::new_v4(),
        invalidated: false,
        pool: inventory.pool_id,
        name: inventory.pool_name.clone(),
        observed: observe(m, pool),
        inventory,
        overview: None,
        query: None,
        domain: String::new(),
        username: String::new(),
        password: SecretText::new(),
        subject_name: String::new(),
        resolved: None,
        subject: None,
        host: None,
        fingerprint: None,
        files: [None, None, None],
        file_revision: 0,
        file_result: None,
        certificate: None,
        review: None,
        confirmation: String::new(),
        error: None,
        dialog: None,
        pending: None,
    });
    if !m.is_synthetic_pool(pool) {
        query(m, PoolSecurityQuery::Overview);
    }
    true
}

pub fn query(m: &mut Model, query: PoolSecurityQuery) {
    let result = (|| {
        let e = m
            .areas
            .pool_security
            .editor
            .as_ref()
            .ok_or("Security view is closed")?;
        let observed = e.observed.filter(|_| current(m, e)).ok_or(REOPEN)?;
        m.connection_for(e.pool)
            .ok_or(CONNECT)?
            .inspect_pool_security(observed, query)
            .map_err(|e| e.to_string())
    })();
    let waker = m.waker.clone();
    let Some(e) = editor(m) else { return };
    match result {
        Ok(handle) => {
            if let Some(wake) = waker {
                handle.notify_on_change(wake);
            }
            e.query = Some(handle);
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
}

pub fn poll(m: &mut Model) -> bool {
    let Some(e) = m.areas.pool_security.editor.as_ref() else {
        return false;
    };
    if !m.is_synthetic_pool(e.pool) && !current(m, e) && !e.invalidated {
        let Some(e) = editor(m) else { return false };
        e.invalidated = true;
        e.query = None;
        e.file_result = None;
        e.certificate = None;
        e.password.clear();
        e.error = Some(INVALIDATED.into());
        return true;
    }
    let result = e.query.as_ref().and_then(|q| q.latest());
    let files = e.file_result.as_ref().and_then(|r| r.try_recv().ok());
    // A submitted change has ended: what the tab shows is out of date.
    let ended = e.pending.filter(|id| {
        m.resource_rows
            .iter()
            .any(|r| r.id == *id && r.state.terminal())
    });
    let mut changed = result.is_some() || files.is_some();
    let Some(e) = editor(m) else { return false };
    let mut next = None;
    if let Some(result) = result {
        e.query = None;
        match result {
            Ok(PoolSecurityResult::Overview(v)) => {
                e.overview = Some(v);
                e.resolved = None;
                e.subject = None;
                // A certificate dialog that opened before the overview was
                // there asks for its fingerprint now.
                if let Some(host) = e.host
                    && e.fingerprint.is_none()
                    && e.dialog.is_some_and(Kind::certificate)
                {
                    next = Some(PoolSecurityQuery::Certificate { host });
                }
            }
            Ok(PoolSecurityResult::Subject {
                query,
                identifier,
                name,
            }) if query == e.subject_name => e.resolved = Some((query, identifier, name)),
            Ok(PoolSecurityResult::Certificate { host, fingerprint }) if e.host == Some(host) => {
                e.fingerprint = Some((host, fingerprint))
            }
            Ok(_) => {}
            Err(error) => e.error = Some(error),
        }
    }
    if let Some(result) = files {
        e.file_result = None;
        match result {
            Ok(certificate) => e.certificate = Some(certificate),
            Err(error) => e.error = Some(error),
        }
    }
    if ended.is_some() && next.is_none() && e.query.is_none() && e.dialog.is_none() {
        e.pending = None;
        if !e.invalidated {
            next = Some(PoolSecurityQuery::Overview);
        }
    }
    if let Some(next) = next {
        query(m, next);
        changed = true;
    }
    changed
}

pub fn stage(m: &mut Model, op: SecurityOp) {
    let Some(e) = editor(m) else { return };
    e.review = Some(op);
    e.confirmation.clear();
    e.error = None;
}

pub fn directory(m: &mut Model, join: bool, remove_machine_accounts: bool) {
    let Some(e) = editor(m) else { return };
    let Some(overview) = &e.overview else { return };
    let hosts = overview.directory_hosts.clone();
    let op = if join {
        SecurityOp::JoinDirectory {
            domain: e.domain.trim().into(),
            expected_hosts: hosts,
        }
    } else {
        SecurityOp::LeaveDirectory {
            expected_hosts: hosts,
            remove_machine_accounts,
        }
    };
    if (join || remove_machine_accounts) && (e.username.is_empty() || e.password.is_empty()) {
        e.error = Some(CREDENTIALS.into());
        return;
    }
    stage(m, op);
}

/// The checks of Rune `apply`, then the hand-over. `consumed` says whether
/// the password or the certificate left the draft.
fn submit(m: &mut Model, consumed: &mut bool) -> Result<ResourceOperation, String> {
    let e = m
        .areas
        .pool_security
        .editor
        .as_ref()
        .ok_or("Security view is closed")?;
    if m.is_synthetic_pool(e.pool) || !m.can_mutate_pool(e.pool) || !current(m, e) {
        return Err(CHANGED.into());
    }
    if e.confirmation != e.pool.to_string() {
        return Err(CONFIRMATION.into());
    }
    let op = e.review.clone().ok_or("Review the change first")?;
    let target = e.observed.ok_or(CHANGED)?;
    let connection = m
        .connection_for(e.pool)
        .ok_or("Connect before applying changes")?;
    let e = editor(m).ok_or("Security view is closed")?;
    let sensitive = match &op {
        SecurityOp::JoinDirectory {
            domain,
            expected_hosts,
        } => {
            *consumed = true;
            Some(PoolSensitiveChange::JoinDirectory {
                domain: domain.clone(),
                expected_hosts: expected_hosts.clone(),
                credentials: Credentials::new(
                    e.username.clone(),
                    e.password.take().expose().to_owned(),
                ),
            })
        }
        SecurityOp::LeaveDirectory {
            expected_hosts,
            remove_machine_accounts: true,
        } => {
            *consumed = true;
            Some(PoolSensitiveChange::LeaveDirectory {
                expected_hosts: expected_hosts.clone(),
                credentials: Credentials::new(
                    e.username.clone(),
                    e.password.take().expose().to_owned(),
                ),
            })
        }
        SecurityOp::InstallCertificate { host, expected, .. } => {
            *consumed = true;
            Some(PoolSensitiveChange::InstallCertificate {
                host: *host,
                expected: expected.clone(),
                certificate: e
                    .certificate
                    .take()
                    .ok_or("Choose and validate the certificate files again")?,
            })
        }
        _ => None,
    };
    e.password.clear();
    let command = match sensitive {
        Some(change) => ResourceCommand::PoolSensitive { target, change },
        None => ResourceCommand::Pool {
            target,
            operation: PoolOperation::Security(op),
        },
    };
    connection
        .submit_resource(OperationId::new(), command)
        .map_err(|e| e.to_string())
}

pub fn apply(m: &mut Model) {
    let mut consumed = false;
    match submit(m, &mut consumed) {
        Ok(operation) => {
            let id = m.track_resource(operation);
            if let Some(e) = editor(m) {
                e.pending = Some(id);
            }
            super::close(m);
        }
        Err(error) => {
            let Some(e) = editor(m) else { return };
            e.error = Some(error);
            if consumed {
                // The secret is gone: the review cannot be confirmed again.
                e.review = None;
                e.confirmation.clear();
            }
        }
    }
}

pub fn select_host(m: &mut Model, host: HostId) {
    let synthetic = m
        .areas
        .pool_security
        .editor
        .as_ref()
        .is_some_and(|e| m.is_synthetic_pool(e.pool));
    let Some(e) = editor(m) else { return };
    e.host = Some(host);
    e.fingerprint = None;
    e.certificate = None;
    e.file_result = None;
    e.files = [None, None, None];
    e.file_revision += 1;
    if e.overview.is_some() {
        query(m, PoolSecurityQuery::Certificate { host });
    } else if e.query.is_none() && !synthetic {
        // The overview first (it says what the session may do); `poll` asks
        // for the fingerprint when it is there.
        query(m, PoolSecurityQuery::Overview);
    }
}

pub const FILE_TITLES: [&str; 3] = [
    "Choose host certificate",
    "Choose unencrypted PKCS8 private key",
    "Choose optional certificate chain",
];

pub fn choose_file(m: &mut Model, index: usize) {
    let Some(e) = editor(m) else { return };
    let epoch = e.epoch;
    let revision = e.file_revision;
    let host = e.host;
    // A reply is for the editor, the server and the selection it was asked
    // from, and never changes a staged review.
    fn asked(
        m: &mut Model,
        epoch: uuid::Uuid,
        host: Option<HostId>,
        revision: u64,
    ) -> Option<&mut Editor> {
        editor(m).filter(|e| {
            e.epoch == epoch && e.host == host && e.file_revision == revision && e.review.is_none()
        })
    }
    m.choose_file(
        FileRequest::open(FILE_TITLES[index], move |m, path| {
            let Some(e) = asked(m, epoch, host, revision) else {
                return;
            };
            if let Some(path) = path {
                e.files[index] = Some(path);
                e.file_revision += 1;
                e.certificate = None;
                e.file_result = None;
                e.error = None;
            }
        })
        .filter("PEM files", ["pem", "crt", "key"])
        .on_failure(move |m, _| {
            if let Some(e) = asked(m, epoch, host, revision) {
                e.error = Some("The file chooser could not open the selected file".into());
            }
        }),
    );
}

static FILE_WORKER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct FilePermit;
impl Drop for FilePermit {
    fn drop(&mut self) {
        FILE_WORKER.store(false, std::sync::atomic::Ordering::Release);
    }
}

pub fn load_files(m: &mut Model) {
    let wake = m.waker.clone();
    let Some(e) = editor(m) else { return };
    if e.file_result.is_some() {
        return;
    }
    if FILE_WORKER
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        e.error = Some("Previous certificate validation is still finishing".into());
        return;
    }
    let permit = FilePermit;
    let files = e.files.clone();
    let (send, recv) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("certificate-validation".into())
        .spawn(move || {
            let _permit = permit;
            let result = (|| {
                let cert = read_file(
                    files[0].as_deref().ok_or("Choose a certificate file")?,
                    256 * 1024,
                )?;
                let chain = files[2]
                    .as_deref()
                    .map(|path| read_file(path, 512 * 1024))
                    .transpose()?
                    .unwrap_or_default();
                let key = read_file(
                    files[1].as_deref().ok_or("Choose a private key file")?,
                    64 * 1024,
                )?;
                ServerCertificate::new(cert, key, chain).map_err(|_| {
                    "Use one PEM certificate, a matching unencrypted PKCS8 private key, and a valid optional PEM chain".to_string()
                })
            })();
            let _ = send.send(result);
            if let Some(wake) = wake {
                wake()
            }
        });
    e.certificate = None;
    e.error = None;
    if spawned.is_ok() {
        e.file_result = Some(recv)
    } else {
        e.error = Some("Cannot start certificate validation".into())
    }
}

pub(super) fn read_file(path: &std::path::Path, limit: u64) -> Result<String, String> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags.bits() as i32)
        .open(path)
        .map_err(|_| {
            "Cannot read a selected certificate file; choose a regular file rather than a symlink"
        })?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect a selected certificate file")?
        .is_file()
    {
        return Err("Select regular PEM files".into());
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read a selected certificate file")?;
    if bytes.len() as u64 > limit {
        return Err("A selected certificate file exceeds its size limit".into());
    }
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| "A selected certificate file is not UTF-8 PEM".into())
}

pub fn review_lines(e: &Editor, op: &SecurityOp) -> Vec<String> {
    match op {
        SecurityOp::JoinDirectory {
            domain,
            expected_hosts,
        } => vec![
            format!(
                "Join {} members to {domain} using directory account {}.",
                expected_hosts.len(),
                e.username
            ),
            "Host authentication changes. A partial failure can leave members configured differently. No automatic leave or rollback is performed.".into(),
        ],
        SecurityOp::LeaveDirectory {
            expected_hosts,
            remove_machine_accounts,
        } => vec![
            format!(
                "Leave Active Directory on {} members. Remove directory machine accounts: {}.",
                expected_hosts.len(),
                if *remove_machine_accounts {
                    "yes, using supplied credentials"
                } else {
                    "no; directory cleanup remains manual"
                }
            ),
            "Directory users will lose access. Keep local root credentials available. No automatic rollback is performed.".into(),
        ],
        SecurityOp::AddSubject { identifier } => vec![
            format!(
                "Add directory identity {identifier} with no roles. Grant a role separately after successful creation."
            ),
            "Servers with unavailable or restricted RBAC are refused because they may assign full administration automatically.".into(),
        ],
        SecurityOp::RemoveSubject {
            subject,
            expected_identifier,
            ..
        } => vec![format!(
            "Remove {expected_identifier} ({subject}) from pool access and close the sessions opened under it. Sessions of a removed group's members stay open until the host next checks them."
        )],
        SecurityOp::ChangeSubjectRole {
            subject,
            expected_identifier,
            role,
            grant,
            ..
        } => vec![format!(
            "{} role {} ({role}) for {expected_identifier} ({subject}).",
            if *grant { "Grant" } else { "Remove" },
            e.overview
                .as_ref()
                .and_then(|v| v.roles.iter().find(|v| v.id == *role))
                .map(|v| v.name.as_str())
                .unwrap_or("Unknown")
        )],
        SecurityOp::InstallCertificate {
            host,
            expected,
            fingerprint,
        } => vec![
            format!(
                "Host: {} ({host})",
                e.inventory
                    .hosts
                    .get(host)
                    .map(|v| v.name.as_str())
                    .unwrap_or("Unknown")
            ),
            format!("Current SHA-256: {expected}\nReplacement SHA-256: {fingerprint}"),
            "The key and certificate are sent directly to this pool, never saved in operation history. Connections may close. Reconnect and verify trust independently; saved trust is not automatically changed.".into(),
        ],
        SecurityOp::ResetCertificate { host, expected } => vec![
            format!(
                "Replace host {host} certificate ({expected}) with a new self-signed certificate."
            ),
            "Connections may close. Reconnect and verify the new certificate independently. Saved trust is not automatically changed.".into(),
        ],
    }
}

// -- What the port adds around the Rune logic -------------------------------

/// The subject chosen in the grid, while the overview still has it.
pub fn subject(e: &Editor) -> Option<&DirectorySubject> {
    let chosen = e.subject?;
    e.overview.as_ref()?.subjects.iter().find(|r| r.id == chosen)
}

/// Rune `supported`: the session is the local superuser and the server
/// offers the method. `None`: it may be called.
pub fn unsupported(v: &SecurityOverview, method: &str) -> Option<String> {
    if !v.is_local_superuser {
        Some(READ_ONLY.into())
    } else if !v.supported_methods.iter().any(|m| m == method) {
        Some(format!("This pool does not offer {method} to this session."))
    } else {
        None
    }
}

pub fn rbac(v: &SecurityOverview) -> Option<String> {
    match v.rbac_available {
        Some(true) => None,
        Some(false) => Some(RBAC_RESTRICTED.into()),
        None => Some(RBAC_UNKNOWN.into()),
    }
}

/// The method a role change of this direction calls.
pub fn role_method(grant: bool) -> &'static str {
    if grant {
        "subject.add_to_roles"
    } else {
        "subject.remove_from_roles"
    }
}

/// Why a change cannot be reviewed whatever is entered: the conditions under
/// which Rune disabled the button that staged it. The same function gives a
/// command its eligibility and the open dialog its refusal.
pub fn gate(e: &Editor, kind: Kind) -> Option<String> {
    let Some(v) = &e.overview else {
        return Some(if e.query.is_some() { READING } else { UNREAD }.into());
    };
    let hosts_known = || {
        (v.directory_hosts.is_empty() || v.errors.contains_key("directory"))
            .then(|| HOSTS_UNKNOWN.to_owned())
    };
    match kind {
        Kind::Join => unsupported(v, "pool.enable_external_auth")
            .or_else(hosts_known)
            .or_else(|| {
                v.directory_hosts
                    .iter()
                    .any(|h| !h.auth_type.is_empty())
                    .then(|| ALREADY_JOINED.into())
            }),
        Kind::Leave => unsupported(v, "pool.disable_external_auth")
            .or_else(hosts_known)
            .or_else(|| {
                v.directory_hosts
                    .iter()
                    .any(|h| h.auth_type != "AD")
                    .then(|| NOT_JOINED.into())
            }),
        Kind::AddSubject => unsupported(v, "subject.create").or_else(|| rbac(v)),
        Kind::RemoveSubject => match subject(e) {
            Some(_) => unsupported(v, "subject.destroy"),
            None => Some(NO_SUBJECT.into()),
        },
        Kind::Role => {
            let Some(subject) = subject(e) else {
                return Some(NO_SUBJECT.into());
            };
            if let Some(reason) = rbac(v) {
                return Some(reason);
            }
            if v.roles.is_empty() {
                return Some(NO_ROLES.into());
            }
            // Disabled only when no role of the pool can be changed.
            v.roles
                .iter()
                .map(|role| unsupported(v, role_method(!subject.roles.contains(&role.id))))
                .reduce(|a, b| a.and(b))
                .flatten()
        }
        Kind::InstallCertificate => unsupported(v, "host.install_server_certificate"),
        Kind::ResetCertificate => unsupported(v, "host.reset_server_certificate"),
    }
}

/// Why the staged review cannot be confirmed any more (Rune disabled the
/// button and `apply` said this).
pub fn stale(m: &Model, e: &Editor) -> Option<String> {
    (e.review.is_some()
        && !m.is_synthetic_pool(e.pool)
        && !(m.can_mutate_pool(e.pool) && current(m, e)))
    .then(|| CHANGED.into())
}

/// Opens the dialog of a change on the editor of its pool, which is opened
/// when the tab has none for that pool. A certificate change names its
/// server.
pub fn begin(m: &mut Model, pool: PoolId, kind: Kind, host: Option<HostId>) {
    let open_for_pool = m
        .areas
        .pool_security
        .editor
        .as_ref()
        .is_some_and(|e| e.pool == pool);
    if !open_for_pool && !open(m, pool) {
        return;
    }
    let Some(e) = editor(m) else { return };
    e.review = None;
    e.confirmation.clear();
    e.password.clear();
    if open_for_pool {
        e.error = None;
    }
    match kind {
        Kind::RemoveSubject => {
            let Some(subject) = subject(e) else { return };
            let op = SecurityOp::RemoveSubject {
                subject: subject.id,
                expected_identifier: subject.identifier.clone(),
                expected_roles: subject.roles.clone(),
            };
            e.dialog = Some(kind);
            stage(m, op);
        }
        Kind::InstallCertificate | Kind::ResetCertificate => {
            let Some(host) = host else { return };
            e.dialog = Some(kind);
            select_host(m, host);
        }
        _ => e.dialog = Some(kind),
    }
}

/// Closes the dialog and leaves the tab's state: the drafts of the change,
/// its secrets and its files go.
pub fn end(e: &mut Editor) {
    e.dialog = None;
    e.review = None;
    e.confirmation.clear();
    e.password.clear();
    e.error = None;
    e.host = None;
    e.fingerprint = None;
    e.certificate = None;
    e.file_result = None;
    e.files = [None, None, None];
    e.file_revision += 1;
}
