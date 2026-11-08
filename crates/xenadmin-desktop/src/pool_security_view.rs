use super::*;
use std::{path::PathBuf, sync::mpsc};
use xenadmin_core::{
    CertificateFingerprint, DirectoryHostState, ObservedResource, OperationId, PoolOperation,
    PoolSecurityDiscovery, PoolSecurityOperation as SecurityOp, PoolSecurityQuery,
    PoolSecurityResult, PoolSensitiveChange, ResourceCommand, ResourceTarget, SecurityOverview,
    ServerCertificate, SubjectId,
};

pub(super) struct Editor {
    epoch: uuid::Uuid,
    invalidated: bool,
    pool: PoolId,
    name: String,
    observed: Option<ObservedResource>,
    inventory: std::sync::Arc<Inventory>,
    overview: Option<SecurityOverview>,
    query: Option<PoolSecurityDiscovery>,
    domain: TextEdit,
    username: TextEdit,
    password: TextEdit,
    subject_name: TextEdit,
    resolved: Option<(String, String, String)>,
    subject: Option<SubjectId>,
    host: Option<HostId>,
    fingerprint: Option<(HostId, CertificateFingerprint)>,
    files: [Option<PathBuf>; 3],
    file_revision: u64,
    file_result: Option<mpsc::Receiver<Result<ServerCertificate, String>>>,
    certificate: Option<ServerCertificate>,
    review: Option<SecurityOp>,
    confirmation: TextEdit,
    error: Option<String>,
}
fn editor(s: &mut State) -> &mut Editor {
    s.pool_security.as_mut().unwrap()
}
fn current(s: &State, e: &Editor) -> bool {
    e.observed.is_some()
        && s.snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}
pub(super) fn open(s: &mut State) {
    let Some(inventory) = s.inventory.clone() else {
        return;
    };
    s.pool_security = Some(Editor {
        epoch: uuid::Uuid::new_v4(),
        invalidated: false,
        pool: inventory.pool_id,
        name: inventory.pool_name.clone(),
        observed: s.snapshot.as_ref().and_then(|v| {
            v.observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .ok()
        }),
        inventory,
        overview: None,
        query: None,
        domain: TextEdit::new(""),
        username: TextEdit::new(""),
        password: secure_field(),
        subject_name: TextEdit::new(""),
        resolved: None,
        subject: None,
        host: None,
        fingerprint: None,
        files: [None, None, None],
        file_revision: 0,
        file_result: None,
        certificate: None,
        review: None,
        confirmation: TextEdit::new(""),
        error: None,
    });
    s.pool_editor = None;
    query(s, PoolSecurityQuery::Overview);
}
fn query(s: &mut State, query: PoolSecurityQuery) {
    let result = (|| {
        let e = s.pool_security.as_ref().ok_or("Security view is closed")?;
        if !current(s, e) {
            return Err("Reconnect and reopen security to refresh its pool identity".into());
        }
        s.connection
            .as_ref()
            .ok_or("Connect to inspect pool security")?
            .inspect_pool_security(e.observed.unwrap(), query)
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(handle) => {
            if let Some(wake) = &s.waker {
                handle.notify_on_change(wake.clone());
            }
            editor(s).query = Some(handle);
            editor(s).error = None;
        }
        Err(error) => editor(s).error = Some(error),
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(e) = s.pool_security.as_ref() else {
        return false;
    };
    if !s.demo && !current(s, e) && !e.invalidated {
        let e = editor(s);
        e.invalidated = true;
        e.query = None;
        e.file_result = None;
        e.certificate = None;
        e.password = secure_field();
        e.error = Some("Connection changed; close and reopen security before continuing".into());
        return true;
    }
    let result = e.query.as_ref().and_then(|q| q.latest());
    let files = e.file_result.as_ref().and_then(|r| r.try_recv().ok());
    let changed = result.is_some() || files.is_some();
    let e = editor(s);
    if let Some(result) = result {
        e.query = None;
        match result {
            Ok(PoolSecurityResult::Overview(v)) => {
                e.overview = Some(v);
                e.resolved = None;
                e.subject = None;
            }
            Ok(PoolSecurityResult::Subject {
                query,
                identifier,
                name,
            }) if query == e.subject_name.text => e.resolved = Some((query, identifier, name)),
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
    changed
}
fn stage(s: &mut State, op: SecurityOp) {
    let e = editor(s);
    e.review = Some(op);
    e.confirmation = TextEdit::new("");
    e.error = None;
}
fn directory(s: &mut State, join: bool, remove_machine_accounts: bool) {
    let e = editor(s);
    let Some(overview) = &e.overview else { return };
    let hosts = overview.directory_hosts.clone();
    let op = if join {
        SecurityOp::JoinDirectory {
            domain: e.domain.text.trim().into(),
            expected_hosts: hosts,
        }
    } else {
        SecurityOp::LeaveDirectory {
            expected_hosts: hosts,
            remove_machine_accounts,
        }
    };
    if (join || remove_machine_accounts)
        && (e.username.text.is_empty() || e.password.text.is_empty())
    {
        e.error = Some("Enter directory credentials for this change".into());
        return;
    }
    stage(s, op);
}
fn apply(s: &mut State) {
    let result = (|| {
        let e = s.pool_security.as_ref().ok_or("Security view is closed")?;
        if s.demo || !s.can_mutate() || !current(s, e) {
            return Err("The connection changed; reopen security".into());
        }
        if e.confirmation.text != e.pool.to_string() {
            return Err("Enter the pool UUID shown in the review".into());
        }
        let op = e.review.clone().ok_or("Review the change first")?;
        let target = e.observed.unwrap();
        let e = editor(s);
        let sensitive = match &op {
            SecurityOp::JoinDirectory {
                domain,
                expected_hosts,
            } => Some(PoolSensitiveChange::JoinDirectory {
                domain: domain.clone(),
                expected_hosts: expected_hosts.clone(),
                credentials: Credentials::new(
                    e.username.text.clone(),
                    std::mem::take(&mut e.password.text),
                ),
            }),
            SecurityOp::LeaveDirectory {
                expected_hosts,
                remove_machine_accounts: true,
            } => Some(PoolSensitiveChange::LeaveDirectory {
                expected_hosts: expected_hosts.clone(),
                credentials: Credentials::new(
                    e.username.text.clone(),
                    std::mem::take(&mut e.password.text),
                ),
            }),
            SecurityOp::InstallCertificate { host, expected, .. } => {
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
        e.password = secure_field();
        let command = match sensitive {
            Some(change) => ResourceCommand::PoolSensitive { target, change },
            None => ResourceCommand::Pool {
                target,
                operation: PoolOperation::Security(op),
            },
        };
        let handle = s
            .connection
            .as_ref()
            .ok_or("Connect before applying changes")?
            .submit_resource(OperationId::new(), command)
            .map_err(|e| e.to_string())?;
        if let Some(waker) = &s.waker {
            handle.notify_on_change(waker.clone());
        }
        Ok::<_, String>(handle)
    })();
    match result {
        Ok(handle) => {
            s.resource_operations.push(handle);
            s.pool_security = None;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        }
        Err(error) => editor(s).error = Some(error),
    }
}
fn select_host(s: &mut State, host: HostId) {
    let e = editor(s);
    e.host = Some(host);
    e.fingerprint = None;
    e.certificate = None;
    e.file_result = None;
    e.files = [None, None, None];
    e.file_revision += 1;
    query(s, PoolSecurityQuery::Certificate { host });
}
fn choose_file(s: &mut State, tx: &mut Tx, index: usize) {
    let e = editor(s);
    let epoch = e.epoch;
    let revision = e.file_revision;
    let host = e.host;
    tx.open_file(
        rune::FileDialogOptions {
            title: [
                "Choose host certificate",
                "Choose unencrypted PKCS8 private key",
                "Choose optional certificate chain",
            ][index]
                .into(),
            ..Default::default()
        }
        .filter("PEM files", &["pem", "crt", "key"]),
        move |s: &mut State, result, _| {
            let Some(e) = s.pool_security.as_mut().filter(|e| {
                e.epoch == epoch
                    && e.host == host
                    && e.file_revision == revision
                    && e.review.is_none()
            }) else {
                return;
            };
            match result {
                Ok(Some(path)) => {
                    e.files[index] = Some(path);
                    e.file_revision += 1;
                    e.certificate = None;
                    e.file_result = None;
                    e.error = None;
                }
                Ok(None) => {}
                Err(_) => {
                    e.error = Some("The file chooser could not open the selected file".into())
                }
            }
        },
    );
}
static FILE_WORKER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct FilePermit;
impl Drop for FilePermit {
    fn drop(&mut self) {
        FILE_WORKER.store(false, std::sync::atomic::Ordering::Release);
    }
}
fn load_files(s: &mut State) {
    let e = editor(s);
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
    let wake = s.waker.clone();
    let spawned=std::thread::Builder::new().name("certificate-validation".into()).spawn(move||{
        let _permit = permit;
        let result=(||{
            let cert=read_file(files[0].as_deref().ok_or("Choose a certificate file")?,256*1024)?;
            let chain=files[2].as_deref().map(|path|read_file(path,512*1024)).transpose()?.unwrap_or_default();
            let key=read_file(files[1].as_deref().ok_or("Choose a private key file")?,64*1024)?;
            ServerCertificate::new(cert,key,chain).map_err(|_|"Use one PEM certificate, a matching unencrypted PKCS8 private key, and a valid optional PEM chain".to_string())
        })();
        let _=send.send(result);if let Some(wake)=wake{wake()}
    });
    let e = editor(s);
    e.certificate = None;
    e.error = None;
    if spawned.is_ok() {
        e.file_result = Some(recv)
    } else {
        e.error = Some("Cannot start certificate validation".into())
    }
}
fn read_file(path: &std::path::Path, limit: u64) -> Result<String, String> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path).map_err(|_| "Cannot read a selected certificate file; choose a regular file rather than a symlink")?;
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
fn review_lines(e: &Editor, op: &SecurityOp) -> Vec<String> {
    match op {
        SecurityOp::JoinDirectory{domain,expected_hosts}=>vec![format!("Join {} members to {domain} using directory account {}.",expected_hosts.len(),e.username.text),"Host authentication changes. A partial failure can leave members configured differently. No automatic leave or rollback is performed.".into()],
        SecurityOp::LeaveDirectory{expected_hosts,remove_machine_accounts}=>vec![format!("Leave Active Directory on {} members. Remove directory machine accounts: {}.",expected_hosts.len(),if *remove_machine_accounts{"yes, using supplied credentials"}else{"no; directory cleanup remains manual"}),"Directory users will lose access. Keep local root credentials available. No automatic rollback is performed.".into()],
        SecurityOp::AddSubject{identifier}=>vec![format!("Add directory identity {identifier} with no roles. Grant a role separately after successful creation."),"Servers with unavailable or restricted RBAC are refused because they may assign full administration automatically.".into()],
        SecurityOp::RemoveSubject{subject,expected_identifier,..}=>vec![format!("Remove {expected_identifier} ({subject}) from pool access. Existing sessions are managed by the host." )],
        SecurityOp::ChangeSubjectRole{subject,expected_identifier,role,grant,..}=>vec![format!("{} role {} ({role}) for {expected_identifier} ({subject}).",if *grant{"Grant"}else{"Remove"},e.overview.as_ref().and_then(|v|v.roles.iter().find(|v|v.id==*role)).map(|v|v.name.as_str()).unwrap_or("Unknown"))],
        SecurityOp::InstallCertificate{host,expected,fingerprint}=>vec![format!("Host: {} ({host})",e.inventory.hosts.get(host).map(|v|v.name.as_str()).unwrap_or("Unknown")),format!("Current SHA-256: {expected}\nReplacement SHA-256: {fingerprint}"),"The key and certificate are sent directly to this pool, never saved in operation history. Connections may close. Reconnect and verify trust independently; saved trust is not automatically changed.".into()],
        SecurityOp::ResetCertificate{host,expected}=>vec![format!("Replace host {host} certificate ({expected}) with a new self-signed certificate."),"Connections may close. Reconnect and verify the new certificate independently. Saved trust is not automatically changed.".into()],
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> rune::El {
    let e = s.pool_security.as_ref().unwrap();
    let mut body = col()
        .gap(12.0)
        .child(heading("Pool authentication and certificates", k))
        .child(caption(format!("{}\n{}", e.name, e.pool), k).wrap_anywhere());
    if let Some(op) = &e.review {
        body = body.child(heading(op.label(), k));
        for line in review_lines(e, op) {
            body = body.child(caption(line, k).wrap_anywhere());
        }
        body = body
            .child(caption(format!("Type the pool UUID to confirm: {}", e.pool), k).wrap_anywhere())
            .child(
                TextInput::new(&e.confirmation)
                    .label("Pool UUID confirmation")
                    .on_edit(|s: &mut State| &mut editor(s).confirmation)
                    .build(k),
            );
    } else {
        body = body.child(
            Button::new("Refresh security status")
                .disabled(e.query.is_some())
                .on_press(|s: &mut State| query(s, PoolSecurityQuery::Overview))
                .build(k),
        );
        if let Some(v) = &e.overview {
            body=body.child(caption(if v.is_local_superuser{"Local superuser session · security changes available"}else{"Read-only security inspection. Reconnect as local root to change authentication or certificates."},k).wrap());
            if !v.is_local_superuser {
                let pool = e.pool;
                body = body.child(
                    Button::new("Reconnect as local administrator…")
                        .disabled(s.demo || !current(s, e))
                        .on_press(move |s: &mut State| profiles_view::reconnect_pool(s, pool))
                        .build(k),
                );
            }
            for (section, error) in &v.errors {
                body = body.child(caption(format!("{section}: {error}"), k).wrap());
            }
            body = body.child(heading("Active Directory", k));
            for h in &v.directory_hosts {
                body = body.child(
                    caption(
                        format!(
                            "{} · {} {}",
                            e.inventory
                                .hosts
                                .get(&h.host)
                                .map(|h| h.name.as_str())
                                .unwrap_or("Unknown host"),
                            if h.auth_type.is_empty() {
                                "Local authentication"
                            } else {
                                &h.auth_type
                            },
                            h.service_name
                        ),
                        k,
                    )
                    .wrap(),
                );
            }
            body = body
                .child(
                    TextInput::new(&e.domain)
                        .label("Directory domain")
                        .placeholder("example.test")
                        .on_edit(|s: &mut State| &mut editor(s).domain)
                        .build(k),
                )
                .child(
                    TextInput::new(&e.username)
                        .label("Directory account")
                        .placeholder("Directory account")
                        .on_edit(|s: &mut State| &mut editor(s).username)
                        .build(k),
                )
                .child(
                    TextInput::new(&e.password)
                        .secure()
                        .label("Directory password")
                        .placeholder("Directory password · session only")
                        .on_edit(|s: &mut State| &mut editor(s).password)
                        .build(k),
                );
            let supported = |method: &str| {
                v.is_local_superuser && v.supported_methods.iter().any(|m| m == method)
            };
            let hosts_known = !v.directory_hosts.is_empty() && !v.errors.contains_key("directory");
            body = body
                .child(
                    Button::new("Review joining directory")
                        .disabled(
                            !supported("pool.enable_external_auth")
                                || !hosts_known
                                || v.directory_hosts.iter().any(|h| !h.auth_type.is_empty()),
                        )
                        .on_press(|s: &mut State| directory(s, true, false))
                        .build(k),
                )
                .child(
                    Button::new("Review leaving · local configuration only")
                        .disabled(
                            !supported("pool.disable_external_auth")
                                || !hosts_known
                                || v.directory_hosts.iter().any(|h| h.auth_type != "AD"),
                        )
                        .on_press(|s: &mut State| directory(s, false, false))
                        .build(k),
                )
                .child(
                    Button::new("Review leaving and remove machine accounts")
                        .disabled(
                            !supported("pool.disable_external_auth")
                                || !hosts_known
                                || v.directory_hosts.iter().any(|h| h.auth_type != "AD"),
                        )
                        .on_press(|s: &mut State| directory(s, false, true))
                        .build(k),
                );
            body = body.child(caption(match v.rbac_available {
                Some(true) => "Role-based access is available on every member.",
                Some(false) => "Role-based access is restricted. Adding subjects is disabled because the server may assign full administration automatically.",
                None => "Role-based access availability is unknown. Adding subjects and changing roles are disabled.",
            }, k).wrap());
            body = body
                .child(heading("Directory users and groups", k))
                .child(
                    TextInput::new(&e.subject_name)
                        .label("Find directory user or group")
                        .placeholder("Directory user or group")
                        .on_edit(|s: &mut State| &mut editor(s).subject_name)
                        .build(k),
                )
                .child(
                    Button::new("Resolve directory identity")
                        .disabled(e.query.is_some() || e.subject_name.text.is_empty())
                        .on_press(|s: &mut State| {
                            let name = editor(s).subject_name.text.clone();
                            query(s, PoolSecurityQuery::ResolveSubject { name });
                        })
                        .build(k),
                );
            if let Some((query, identifier, name)) = &e.resolved
                && query == &e.subject_name.text
            {
                let identifier = identifier.clone();
                body = body
                    .child(caption(format!("Resolved: {name} · {identifier}"), k).wrap_anywhere())
                    .child(
                        Button::new("Review adding access")
                            .disabled(
                                !supported("subject.create") || v.rbac_available != Some(true),
                            )
                            .on_press(move |s: &mut State| {
                                stage(
                                    s,
                                    SecurityOp::AddSubject {
                                        identifier: identifier.clone(),
                                    },
                                )
                            })
                            .build(k),
                    );
            }
            let search = e.subject_name.text.to_lowercase();
            for subject in v
                .subjects
                .iter()
                .filter(|r| {
                    search.is_empty()
                        || r.name.to_lowercase().contains(&search)
                        || r.identifier.to_lowercase().contains(&search)
                })
                .take(30)
            {
                let id = subject.id;
                body = body.child(
                    Button::new(format!(
                        "{} {} · {}",
                        if e.subject == Some(id) { "✓" } else { "○" },
                        subject.name,
                        subject.identifier
                    ))
                    .on_press(move |s: &mut State| editor(s).subject = Some(id))
                    .build(k),
                );
            }
            if v.subjects.len() > 30 {
                body = body.child(
                    caption("Showing up to 30 matches. Search to narrow the list.", k).wrap(),
                );
            }
            if let Some(subject) = v.subjects.iter().find(|r| Some(r.id) == e.subject) {
                let op = SecurityOp::RemoveSubject {
                    subject: subject.id,
                    expected_identifier: subject.identifier.clone(),
                    expected_roles: subject.roles.clone(),
                };
                body = body.child(
                    Button::new("Review removing subject access")
                        .disabled(!supported("subject.destroy"))
                        .on_press(move |s: &mut State| stage(s, op.clone()))
                        .build(k),
                );
                for role in &v.roles {
                    let grant = !subject.roles.contains(&role.id);
                    let op = SecurityOp::ChangeSubjectRole {
                        subject: subject.id,
                        expected_identifier: subject.identifier.clone(),
                        expected_roles: subject.roles.clone(),
                        role: role.id,
                        grant,
                    };
                    body = body.child(
                        Button::new(format!(
                            "{} {}",
                            if grant { "Grant" } else { "Remove" },
                            role.name
                        ))
                        .disabled(
                            v.rbac_available != Some(true)
                                || !supported(if grant {
                                    "subject.add_to_roles"
                                } else {
                                    "subject.remove_from_roles"
                                }),
                        )
                        .on_press(move |s: &mut State| stage(s, op.clone()))
                        .build(k),
                    );
                }
            }
            body = body.child(heading("Host TLS certificates", k));
            for host in e.inventory.hosts.values().take(256) {
                let id = host.id;
                body = body.child(
                    Button::new(format!(
                        "{} {}",
                        if e.host == Some(id) { "✓" } else { "○" },
                        host.name
                    ))
                    .on_press(move |s: &mut State| select_host(s, id))
                    .build(k),
                );
            }
            if let Some((host, fingerprint)) = &e.fingerprint
                && e.host == Some(*host)
            {
                body = body
                    .child(caption(format!("Current SHA-256: {fingerprint}"), k).wrap_anywhere());
                for index in 0..3 {
                    body = body.child(
                        Button::new(
                            [
                                "Choose certificate…",
                                "Choose private key…",
                                "Choose optional chain…",
                            ][index],
                        )
                        .on_press_tx(move |s: &mut State, tx: &mut Tx| choose_file(s, tx, index))
                        .build(k),
                    );
                    if let Some(path) = &e.files[index] {
                        body = body.child(
                            caption(path.file_name().unwrap_or_default().to_string_lossy(), k)
                                .wrap_anywhere(),
                        );
                    }
                }
                body = body.child(
                    Button::new("Validate selected PEM files")
                        .disabled(
                            e.file_result.is_some() || e.files[0].is_none() || e.files[1].is_none(),
                        )
                        .on_press(load_files)
                        .build(k),
                );
                if let Some(cert) = &e.certificate {
                    let op = SecurityOp::InstallCertificate {
                        host: *host,
                        expected: fingerprint.clone(),
                        fingerprint: cert.fingerprint(),
                    };
                    body = body
                        .child(
                            caption(format!("Replacement SHA-256: {}", cert.fingerprint()), k)
                                .wrap_anywhere(),
                        )
                        .child(
                            Button::new("Review installing certificate")
                                .disabled(
                                    !supported("host.install_server_certificate")
                                        || cert.fingerprint() == *fingerprint,
                                )
                                .on_press(move |s: &mut State| stage(s, op.clone()))
                                .build(k),
                        );
                }
                let op = SecurityOp::ResetCertificate {
                    host: *host,
                    expected: fingerprint.clone(),
                };
                body = body.child(
                    Button::new("Review resetting to self-signed")
                        .disabled(!supported("host.reset_server_certificate"))
                        .on_press(move |s: &mut State| stage(s, op.clone()))
                        .build(k),
                );
            }
        }
        if e.query.is_some() {
            body = body.child(caption("Reading security state…", k));
        }
        if e.file_result.is_some() {
            body = body.child(caption("Validating certificate and private key…", k));
        }
    }
    if let Some(error) = &e.error {
        body = body.child(caption(error, k).wrap());
    }
    let mut footer = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.pool_security = None)
            .build(k),
    );
    if e.review.is_some() {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| editor(s).review = None)
                    .build(k),
            )
            .child(
                Button::new("Confirm change")
                    .variant(ButtonVariant::Danger)
                    .disabled(
                        s.demo
                            || !s.can_mutate()
                            || !current(s, e)
                            || e.confirmation.text != e.pool.to_string(),
                    )
                    .on_press(apply)
                    .build(k)
                    .key("security-confirm"),
            );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Pool security")
        .modal(false, |s: &mut State| s.pool_security = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 720.0))
                .h((s.window_height - 56.0).clamp(280.0, 820.0))
                .padding(20.0)
                .gap(16.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}
pub(super) fn scene(s: &mut State, review: bool) {
    crate::demo(s);
    open(s);
    let e = editor(s);
    e.error = None;
    e.overview = Some(SecurityOverview {
        is_local_superuser: true,
        supported_methods: vec![
            "pool.enable_external_auth".into(),
            "host.reset_server_certificate".into(),
        ],
        directory_hosts: e
            .inventory
            .hosts
            .keys()
            .map(|host| DirectoryHostState {
                host: *host,
                auth_type: String::new(),
                service_name: String::new(),
            })
            .collect(),
        ..Default::default()
    });
    if review {
        e.domain = TextEdit::new("example.test");
        e.username = TextEdit::new("directory-admin");
        let expected_hosts = e.overview.as_ref().unwrap().directory_hosts.clone();
        stage(
            s,
            SecurityOp::JoinDirectory {
                domain: "example.test".into(),
                expected_hosts,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn certificate_files_are_regular_bounded_and_errors_do_not_include_contents() {
        let dir =
            std::env::temp_dir().join(format!("xenadmin-cert-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("key.pem");
        std::fs::write(&path, b"synthetic-private-key").unwrap();
        assert_eq!(read_file(&path, 64).unwrap(), "synthetic-private-key");
        assert!(!read_file(&path, 4).unwrap_err().contains("synthetic"));
        let link = dir.join("link.pem");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_file(&link, 64).is_err());
        assert!(read_file(&dir, 64).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn compact_review_keeps_footer_visible_and_captured_identity_immutable() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s, true);
            let pool = editor(&mut s).pool;
            let captured = editor(&mut s).review.clone();
            let mut rt = rune::Runtime::new(
                s,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let (x, y) = rt.find("security-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.resource_operations.is_empty());
            assert_eq!(editor(&mut rt.state).pool, pool);
            assert_eq!(editor(&mut rt.state).review, captured);
        }
    }
    #[test]
    fn native_certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection()
     {
        let mut s = crate::init();
        scene(&mut s, false);
        let mut rt = rune::Runtime::new(
            s,
            |_: &State, cx: &mut rune::Cx<State>| {
                Button::new("Pick")
                    .on_press_tx(|s: &mut State, tx: &mut Tx| choose_file(s, tx, 0))
                    .build(&Kit::neutral(cx.system_dark()))
                    .key("pick")
            },
            (480.0, 640.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let pick = |rt: &mut rune::Runtime<State>| {
            rt.frame(0.0);
            let (x, y) = rt.find("pick").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.file_dialog().is_some());
        };
        pick(&mut rt);
        rt.resolve_file_dialog(Some("/tmp/synthetic-cert.pem".into()))
            .unwrap();
        rt.frame(0.0);
        assert_eq!(
            editor(&mut rt.state).files[0],
            Some("/tmp/synthetic-cert.pem".into())
        );
        pick(&mut rt);
        rt.resolve_file_dialog(None).unwrap();
        rt.frame(0.0);
        assert!(editor(&mut rt.state).files[0].is_some());
        pick(&mut rt);
        editor(&mut rt.state).file_revision += 1;
        rt.resolve_file_dialog(Some("/tmp/late-key.pem".into()))
            .unwrap();
        rt.frame(0.0);
        assert_eq!(
            editor(&mut rt.state).files[0],
            Some("/tmp/synthetic-cert.pem".into())
        );
        pick(&mut rt);
        scene(&mut rt.state, false);
        rt.resolve_file_dialog(Some("/tmp/old-editor.pem".into()))
            .unwrap();
        rt.frame(0.0);
        assert!(editor(&mut rt.state).files[0].is_none());
    }
    #[test]
    fn late_certificate_validation_is_discarded_when_host_changes() {
        let mut s = crate::init();
        scene(&mut s, false);
        let (send, recv) = mpsc::channel();
        editor(&mut s).file_result = Some(recv);
        let host = *editor(&mut s).inventory.hosts.keys().next().unwrap();
        select_host(&mut s, host);
        assert!(send.send(Err("stale validation".into())).is_err());
        assert!(editor(&mut s).certificate.is_none());
    }
}
