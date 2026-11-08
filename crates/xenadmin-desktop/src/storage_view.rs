#[path = "storage_maintenance_view.rs"]
mod maintenance;
pub(super) fn maintenance_scene(s: &mut State) {
    maintenance::scene(s, false);
    s.storage_editor.as_mut().unwrap().maintenance_review = None;
}
pub(super) fn repair_scene(s: &mut State) {
    maintenance::scene(s, false);
}
pub(super) fn cache_scene(s: &mut State) {
    maintenance::scene(s, true);
}
use super::*;
use xenadmin_core::{
    NfsVersion, ObservedResource, OperationId, PbdId, ResourceCommand, ResourceTarget, SrId,
    StorageBackend, StorageDiscovery, StorageDiscoveryResult, StorageLocation, StorageOperation,
    StorageQuery,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Create,
    Introduce,
    Manage,
    Attachment,
}
pub(super) struct Editor {
    maintenance_review: Option<xenadmin_core::StorageMaintenance>,
    repair_selection: std::collections::BTreeSet<PbdId>,
    pool: PoolId,
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    mode: Mode,
    name: TextEdit,
    description: TextEdit,
    server: TextEdit,
    port: TextEdit,
    path: TextEdit,
    share: TextEdit,
    authenticate: bool,
    mutual: bool,
    auth_epoch: u64,
    username: TextEdit,
    password: TextEdit,
    incoming_username: TextEdit,
    incoming_password: TextEdit,
    review_credentials: Option<xenadmin_core::StorageCredentials>,
    review_auth_label: Option<String>,
    cleanup_review: Option<(ObservedResource, OperationId, Vec<xenadmin_core::SecretId>)>,
    iqn: TextEdit,
    scsi: TextEdit,
    uuid: TextEdit,
    confirmation: TextEdit,
    kind: u8,
    version: NfsVersion,
    host: Option<HostId>,
    review: Option<(Option<ObservedResource>, ResourceTarget, StorageOperation)>,
    discovery: Option<StorageDiscovery>,
    query_inputs: Option<DiscoveryInputs>,
    query_observed: Option<ObservedResource>,
    backends: Vec<StorageBackend>,
    discovered: Option<StorageDiscoveryResult>,
    result_page: usize,
    error: Option<String>,
}
pub(super) fn open_new(s: &mut State) {
    open(s, None);
}
pub(super) fn open_selected(s: &mut State) {
    let sr = s.selected().and_then(|r| SrId::parse(&r.id).ok());
    if let Some(sr) = sr {
        open(s, Some(sr));
    }
}
fn open(s: &mut State, sr: Option<SrId>) {
    let Some(inventory) = s.inventory.as_ref() else {
        return;
    };
    let target = sr
        .map(ResourceTarget::Storage)
        .unwrap_or(ResourceTarget::Pool(inventory.pool_id));
    let observed = s
        .snapshot
        .as_ref()
        .and_then(|s| s.observe_resource(target).ok());
    let name = sr
        .and_then(|id| inventory.storage.iter().find(|sr| sr.uuid == id.uuid()))
        .map(|sr| sr.name.as_str())
        .unwrap_or("");
    let description = sr
        .and_then(|id| inventory.details.storage.get(&id))
        .map(|sr| sr.description.as_str())
        .unwrap_or("");
    s.storage_editor = Some(Editor {
        maintenance_review: None,
        repair_selection: Default::default(),
        pool: inventory.pool_id,
        target,
        observed,
        mode: if sr.is_some() {
            Mode::Manage
        } else {
            Mode::Create
        },
        name: TextEdit::new(name),
        description: TextEdit::new(description),
        server: TextEdit::new(""),
        port: TextEdit::new("3260"),
        path: TextEdit::new("/"),
        share: TextEdit::new(""),
        authenticate: false,
        mutual: false,
        auth_epoch: 0,
        username: TextEdit::new(""),
        password: secure_field(),
        incoming_username: TextEdit::new(""),
        incoming_password: secure_field(),
        review_credentials: None,
        review_auth_label: None,
        cleanup_review: None,
        iqn: TextEdit::new(""),
        scsi: TextEdit::new(""),
        uuid: TextEdit::new(""),
        confirmation: TextEdit::new(""),
        kind: 0,
        version: NfsVersion::Automatic,
        host: inventory.hosts.keys().next().copied(),
        review: None,
        discovery: None,
        query_inputs: None,
        query_observed: None,
        backends: Vec::new(),
        discovered: None,
        result_page: 0,
        error: None,
    });
    query(s, 0);
}
fn location(editor: &Editor) -> StorageLocation {
    match editor.kind {
        3 => StorageLocation::IsoSmb {
            server: editor.server.text.trim().into(),
            share: editor.share.text.trim().into(),
            path: editor.path.text.trim_matches('/').into(),
        },
        1 => StorageLocation::IsoNfs {
            server: editor.server.text.trim().into(),
            path: editor.path.text.trim().into(),
            version: editor.version,
        },
        2 => StorageLocation::Iscsi {
            portal: editor.server.text.trim().into(),
            port: editor.port.text.trim().parse().unwrap_or(0),
            target_iqn: editor.iqn.text.trim().into(),
            scsi_id: editor.scsi.text.trim().into(),
        },
        _ => StorageLocation::Nfs {
            server: editor.server.text.trim().into(),
            path: editor.path.text.trim().into(),
            version: editor.version,
        },
    }
}
#[derive(Clone, PartialEq, Eq)]
struct DiscoveryInputs {
    host: Option<HostId>,
    kind: u8,
    version: NfsVersion,
    server: String,
    port: String,
    path: String,
    iqn: String,
    scsi: String,
    share: String,
    auth_epoch: u64,
    authenticate: bool,
    mutual: bool,
}
fn inputs(e: &Editor) -> DiscoveryInputs {
    DiscoveryInputs {
        host: e.host,
        kind: e.kind,
        version: e.version,
        server: e.server.text.clone(),
        port: e.port.text.clone(),
        path: e.path.text.clone(),
        iqn: e.iqn.text.clone(),
        scsi: e.scsi.text.clone(),
        share: e.share.text.clone(),
        auth_epoch: e.auth_epoch,
        authenticate: e.authenticate,
        mutual: e.mutual,
    }
}
fn results_current(s: &State, e: &Editor) -> bool {
    e.query_inputs.as_ref() == Some(&inputs(e))
        && (s.demo
            || e.query_observed.is_some_and(|o| {
                s.snapshot
                    .as_ref()
                    .and_then(|s| s.observe_resource(o.target()).ok())
                    == Some(o)
            }))
}
fn query(s: &mut State, step: u8) {
    let result = (|| {
        let e = s.storage_editor.as_ref().ok_or("Storage view is closed")?;
        let snapshot = s.snapshot.as_ref().ok_or("Connect to discover storage")?;
        if snapshot.inventory().is_none_or(|i| i.pool_id != e.pool) {
            return Err("Connection changed; reopen storage".into());
        }
        let observed = snapshot
            .observe_resource(ResourceTarget::Pool(e.pool))
            .map_err(|e| e.to_string())?;
        let host = e.host.ok_or("Choose a host")?;
        let server = e.server.text.trim().to_string();
        let port = e.port.text.trim().parse().unwrap_or(0);
        let query = match step {
            0 => StorageQuery::Backends,
            1 => StorageQuery::Probe {
                host,
                location: location(e),
            },
            2 => StorageQuery::NfsExports {
                host,
                server,
                version: e.version,
            },
            3 => StorageQuery::IscsiTargets {
                host,
                portal: server,
                port,
            },
            _ => StorageQuery::IscsiLuns {
                host,
                portal: server,
                port,
                target_iqn: e.iqn.text.trim().into(),
            },
        };
        let connection = s.connection.as_ref().ok_or("Connect before discovery")?;
        let handle = if step != 0 && e.authenticate && matches!(e.kind, 2 | 3) {
            connection.discover_authenticated_storage(
                observed,
                query,
                credentials(e).ok_or("Enter the storage username and password")?,
            )
        } else {
            connection.discover_storage(observed, query)
        }
        .map_err(|e| e.to_string())?;
        if let Some(waker) = &s.waker {
            handle.notify_on_change(waker.clone());
        }
        Ok::<_, String>((handle, inputs(e), observed))
    })();
    if let Some(e) = &mut s.storage_editor {
        match result {
            Ok((handle, inputs, observed)) => {
                e.discovery = Some(handle);
                e.query_inputs = Some(inputs);
                e.query_observed = Some(observed);
                e.error = None;
                e.discovered = None;
                e.result_page = 0;
            }
            Err(error) => {
                if !s.demo {
                    e.error = Some(error);
                }
            }
        }
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(e) = s.storage_editor.as_ref() else {
        return false;
    };
    let Some(result) = e.discovery.as_ref().and_then(StorageDiscovery::latest) else {
        return false;
    };
    let current = results_current(s, e);
    let e = s.storage_editor.as_mut().unwrap();
    e.discovery = None;
    if !current {
        e.discovered = None;
        e.error =
            Some("Storage inputs or connection changed during discovery. Discover again.".into());
        return true;
    }
    match result {
        Ok(StorageDiscoveryResult::Backends(backends)) => e.backends = backends,
        Ok(result) => {
            e.discovered = Some(result);
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
    true
}
fn select_result(s: &mut State, index: usize) {
    let Some(e) = s.storage_editor.as_ref() else {
        return;
    };
    if !results_current(s, e) {
        let e = s.storage_editor.as_mut().unwrap();
        e.discovered = None;
        e.error = Some(
            "Storage inputs or credentials changed. Discover again before selecting a result."
                .into(),
        );
        return;
    }
    let e = s.storage_editor.as_mut().unwrap();
    match &e.discovered {
        Some(StorageDiscoveryResult::NfsExports(rows)) => {
            if let Some(row) = rows.get(index) {
                e.path = TextEdit::new(&row.path);
                e.uuid = TextEdit::new("");
            }
        }
        Some(StorageDiscoveryResult::IscsiTargets(rows)) => {
            if let Some(row) = rows.get(index) {
                e.server = TextEdit::new(&row.portal);
                e.port = TextEdit::new(row.port.to_string());
                e.iqn = TextEdit::new(&row.iqn);
                e.scsi = TextEdit::new("");
                e.uuid = TextEdit::new("");
            }
        }
        Some(StorageDiscoveryResult::IscsiLuns(rows)) => {
            if let Some(row) = rows.get(index) {
                e.scsi = TextEdit::new(&row.scsi_id);
                e.uuid = TextEdit::new("");
            }
        }
        Some(StorageDiscoveryResult::Repositories(rows)) => {
            if let Some(row) = rows.get(index) {
                e.uuid = TextEdit::new(row.uuid.to_string());
                e.mode = Mode::Introduce;
            }
        }
        _ => return,
    }
    e.discovered = None;
    e.error = None;
    e.result_page = 0;
}
fn discovery_results(s: &State, e: &Editor, k: &Kit) -> El {
    let Some(result) = &e.discovered else {
        return el();
    };
    if !results_current(s, e) {
        return caption(
            "Inputs changed; discover again before selecting a result.",
            k,
        )
        .wrap();
    }
    let (title, rows) = match result {
        StorageDiscoveryResult::NfsExports(rows) => (
            "NFS exports",
            rows.iter()
                .map(|r| format!("{}\nAccess reported by server: {}", r.path, r.access))
                .collect::<Vec<_>>(),
        ),
        StorageDiscoveryResult::IscsiTargets(rows) => (
            "iSCSI targets",
            rows.iter()
                .map(|r| format!("{}\nPortal {} · port {}", r.iqn, r.portal, r.port))
                .collect(),
        ),
        StorageDiscoveryResult::IscsiLuns(rows) => (
            "iSCSI LUNs",
            rows.iter()
                .map(|r| {
                    format!(
                        "{}\nLUN {} · {} · {} {}",
                        r.scsi_id,
                        r.lun.map(|n| n.to_string()).unwrap_or("unknown".into()),
                        r.size_bytes.map(bytes).unwrap_or("unknown capacity".into()),
                        r.vendor,
                        r.serial
                    )
                })
                .collect(),
        ),
        StorageDiscoveryResult::Repositories(rows) => (
            "Existing repositories",
            rows.iter()
                .map(|r| format!("{} · {}", r.name, r.uuid))
                .collect(),
        ),
        StorageDiscoveryResult::Backends(_) => return el(),
    };
    let mut body = col().gap(8.0).child(heading(title, k)).child(
        caption(
            format!(
                "{} results. Selecting a row only fills the form; review remains required.",
                rows.len()
            ),
            k,
        )
        .wrap(),
    );
    if rows.is_empty() {
        body=body.child(caption("The backend reported no matching results. You can enter a known location manually.",k).wrap());
    }
    for (index, text) in rows.iter().enumerate().skip(e.result_page * 20).take(20) {
        body = body.child(caption(text, k).wrap_anywhere()).child(
            Button::new("Use this result")
                .variant(ButtonVariant::Outline)
                .on_press(move |s: &mut State| select_result(s, index))
                .build(k)
                .key(format!("storage-result-{index}")),
        );
    }
    if rows.len() > 20 {
        let page = e.result_page;
        body = body.child(
            row()
                .gap(8.0)
                .child(
                    Button::new("Previous")
                        .disabled(page == 0)
                        .on_press(|s: &mut State| {
                            let e = s.storage_editor.as_mut().unwrap();
                            e.result_page = e.result_page.saturating_sub(1);
                        })
                        .build(k),
                )
                .child(
                    Button::new("Next")
                        .disabled((page + 1) * 20 >= rows.len())
                        .on_press(|s: &mut State| {
                            s.storage_editor.as_mut().unwrap().result_page += 1
                        })
                        .build(k),
                ),
        );
    }
    body
}
fn credentials(e: &Editor) -> Option<xenadmin_core::StorageCredentials> {
    if e.username.text.is_empty() || e.password.text.is_empty() {
        return None;
    }
    if e.kind == 3 {
        Some(xenadmin_core::StorageCredentials::smb(
            e.username.text.clone(),
            e.password.text.clone(),
        ))
    } else if e.kind == 2 && e.mutual {
        if e.incoming_username.text.is_empty() || e.incoming_password.text.is_empty() {
            return None;
        }
        Some(xenadmin_core::StorageCredentials::mutual_chap(
            e.username.text.clone(),
            e.password.text.clone(),
            e.incoming_username.text.clone(),
            e.incoming_password.text.clone(),
        ))
    } else if e.kind == 2 {
        Some(xenadmin_core::StorageCredentials::chap(
            e.username.text.clone(),
            e.password.text.clone(),
        ))
    } else {
        None
    }
}
fn review(s: &mut State) {
    let result = (|| {
        let editor = s.storage_editor.as_ref().ok_or("Storage view is closed")?;
        if editor.kind == 2
            && editor
                .port
                .text
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .is_none()
        {
            return Err("Enter an iSCSI port from 1 to 65535".into());
        }
        let operation = match editor.mode {
            Mode::Create => StorageOperation::Create {
                host: editor.host.ok_or("Choose a host")?,
                name: editor.name.text.trim().into(),
                description: editor.description.text.clone(),
                location: location(editor),
            },
            Mode::Introduce => StorageOperation::Introduce {
                uuid: SrId::parse(editor.uuid.text.trim())
                    .map_err(|_| "Choose a valid discovered storage UUID")?,
                name: editor.name.text.trim().into(),
                description: editor.description.text.clone(),
                location: location(editor),
            },
            Mode::Attachment => StorageOperation::CreateAttachment {
                host: editor.host.ok_or("Choose a host")?,
                location: location(editor),
            },
            Mode::Manage => return Err("Choose a storage action".into()),
        };
        if matches!(
            operation,
            StorageOperation::Create { .. } | StorageOperation::Introduce { .. }
        ) && editor.name.text.trim().is_empty()
        {
            return Err("Enter a storage name".into());
        }
        Ok::<_, String>((editor.target, operation))
    })();
    match result {
        Ok((target, operation)) => stage(s, target, operation),
        Err(error) => {
            if let Some(editor) = &mut s.storage_editor {
                editor.error = Some(error);
            }
        }
    }
}
fn stage(s: &mut State, target: ResourceTarget, operation: StorageOperation) {
    let observed = s
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.observe_resource(target).ok());
    if let Some(editor) = &mut s.storage_editor {
        if !s.demo
            && (editor.observed.is_none()
                || observed.is_none()
                || !current(s.snapshot.as_deref(), editor))
        {
            editor.error = Some("The connection changed; reopen storage before reviewing.".into());
            return;
        }
        let auth = editor.authenticate
            && matches!(editor.kind, 2 | 3)
            && matches!(
                operation,
                StorageOperation::Create { .. } | StorageOperation::CreateAttachment { .. }
            );
        editor.review_credentials = if auth { credentials(editor) } else { None };
        if auth && editor.review_credentials.is_none() {
            editor.error = Some("Enter the storage username and password before review".into());
            return;
        }
        editor.review_auth_label = auth.then(|| {
            format!(
                "{} authentication as {}",
                if editor.kind == 3 {
                    "SMB 3.0"
                } else if editor.mutual {
                    "Mutual CHAP"
                } else {
                    "CHAP"
                },
                editor.username.text
            )
        });
        editor.review = Some((observed, target, operation));
        editor.error = None;
        editor.confirmation = TextEdit::new("");
    }
}
fn current(snapshot: Option<&ConnectionSnapshot>, editor: &Editor) -> bool {
    snapshot.is_some_and(|snapshot| {
        editor
            .observed
            .is_some_and(|observed| snapshot.observe_resource(editor.target).ok() == Some(observed))
    })
}
fn stage_cleanup(s: &mut State, id: OperationId) {
    let Some(editor) = s.storage_editor.as_ref() else {
        return;
    };
    let Some(record) = s.resource_rows.iter().find(|r| {
        r.id == id
            && r.pool == editor.pool
            && r.state.terminal()
            && matches!(
                r.action,
                xenadmin_core::ResourceAction::AuthenticatedStorage { .. }
            )
    }) else {
        return;
    };
    let Some(observed) = s
        .snapshot
        .as_ref()
        .and_then(|v| v.observe_resource(ResourceTarget::Pool(editor.pool)).ok())
    else {
        return;
    };
    let secrets = record
        .storage_secrets
        .iter()
        .filter(|v| !v.cleaned)
        .map(|v| v.secret)
        .collect();
    let editor = s.storage_editor.as_mut().unwrap();
    editor.cleanup_review = Some((observed, id, secrets));
    editor.error = None;
}
fn apply_cleanup(s: &mut State) {
    let result = (|| {
        let e = s
            .storage_editor
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?;
        let (pool, operation, secrets) = e
            .cleanup_review
            .as_ref()
            .ok_or(xenadmin_core::Error::InvalidResourceCommand)?;
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::CleanupStorageCredentials {
                    pool: *pool,
                    operation: *operation,
                    secrets: secrets.clone(),
                },
            )
    })();
    match result {
        Ok(operation) => {
            if let Some(w) = &s.waker {
                operation.notify_on_change(w.clone());
            }
            s.resource_operations.push(operation);
            s.storage_editor = None;
        }
        Err(e) => {
            if let Some(editor) = &mut s.storage_editor {
                editor.error = Some(e.to_string());
            }
        }
    }
}
fn apply(s: &mut State) {
    let result = (|| {
        let editor = s
            .storage_editor
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?;
        let (observed, _, operation) = editor
            .review
            .as_ref()
            .ok_or(xenadmin_core::Error::InvalidResourceCommand)?;
        let observed = observed.ok_or(xenadmin_core::Error::Stale)?;
        if matches!(operation,StorageOperation::Forget{expected_disks,..}|StorageOperation::Destroy{expected_disks,..} if expected_disks.len()>100)
        {
            return Err(xenadmin_core::Error::InvalidResourceCommand);
        }
        if !current(s.snapshot.as_deref(), editor) {
            return Err(xenadmin_core::Error::Stale);
        }
        if matches!(
            operation,
            StorageOperation::Destroy { .. } | StorageOperation::Forget { .. }
        ) && editor.confirmation.text != editor.target.uuid().to_string()
        {
            return Err(xenadmin_core::Error::InvalidResourceCommand);
        }
        let operation = operation.clone();
        let connection = s.connection.as_ref().ok_or(xenadmin_core::Error::Closed)?;
        let credentials = s.storage_editor.as_mut().unwrap().review_credentials.take();
        s.storage_editor.as_mut().unwrap().password = secure_field();
        s.storage_editor.as_mut().unwrap().incoming_password = secure_field();
        if let Some(credentials) = credentials {
            return connection.submit_authenticated_storage(
                OperationId::new(),
                observed,
                operation,
                credentials,
            );
        }
        connection.submit_resource(
            OperationId::new(),
            ResourceCommand::Storage {
                target: observed,
                operation: operation.clone(),
            },
        )
    })();
    match result {
        Ok(operation) => {
            if let Some(waker) = &s.waker {
                operation.notify_on_change(waker.clone());
            }
            s.resource_operations.push(operation);
            s.storage_editor = None;
        }
        Err(error) => {
            if let Some(editor) = &mut s.storage_editor {
                editor.error = Some(error.to_string());
                if editor.review_auth_label.is_some() && editor.review_credentials.is_none() {
                    editor.review = None;
                    editor.review_auth_label = None;
                    editor.error = Some(format!(
                        "{error}. Enter the storage password and review again."
                    ));
                }
            }
        }
    }
}
fn attachment_action(s: &mut State, pbd: PbdId, remove: bool) {
    let Some(attachment) = s.inventory.as_ref().and_then(|i| i.details.pbds.get(&pbd)) else {
        return;
    };
    if let (Some(sr), Some(host), Some(attached)) = (
        attachment.sr,
        attachment.host,
        attachment.currently_attached,
    ) {
        stage(
            s,
            ResourceTarget::Pbd(pbd),
            if remove {
                StorageOperation::RemoveAttachment { sr, host }
            } else {
                StorageOperation::SetAttached {
                    sr,
                    host,
                    expected_attached: attached,
                    attached: !attached,
                }
            },
        );
    }
}
fn removal(s: &mut State, destroy: bool) {
    let Some(editor) = &s.storage_editor else {
        return;
    };
    let ResourceTarget::Storage(sr) = editor.target else {
        return;
    };
    let Some(inventory) = &s.inventory else {
        return;
    };
    let Some(row) = inventory.storage.iter().find(|r| r.uuid == sr.uuid()) else {
        return;
    };
    let disks = inventory
        .details
        .sr_disks
        .get(&sr)
        .cloned()
        .unwrap_or_default();
    let name = row.name.clone();
    stage(
        s,
        ResourceTarget::Storage(sr),
        if destroy {
            StorageOperation::Destroy {
                expected_name: name,
                expected_disks: disks,
            }
        } else {
            StorageOperation::Forget {
                expected_name: name,
                expected_disks: disks,
            }
        },
    );
}
fn new_attachment(s: &mut State) {
    let Some(editor) = s.storage_editor.as_mut() else {
        return;
    };
    let ResourceTarget::Storage(sr) = editor.target else {
        return;
    };
    let template = s
        .inventory
        .as_ref()
        .and_then(|i| {
            i.details
                .pbds
                .values()
                .find(|p| p.sr == Some(sr) && p.location.is_some())
        })
        .and_then(|p| p.location.clone());
    editor.mode = Mode::Attachment;
    editor.error = None;
    if let Some(template) = template {
        editor.authenticate = s.inventory.as_ref().is_some_and(|i| {
            i.details
                .pbds
                .values()
                .any(|p| p.sr == Some(sr) && p.requires_credentials)
        });
        match template {
            StorageLocation::IsoSmb {
                server,
                share,
                path,
            } => {
                editor.kind = 3;
                editor.server = TextEdit::new(server);
                editor.share = TextEdit::new(share);
                editor.path = TextEdit::new(path);
            }
            StorageLocation::Nfs {
                server,
                path,
                version,
            }
            | StorageLocation::IsoNfs {
                server,
                path,
                version,
            } => {
                editor.kind = if s
                    .inventory
                    .as_ref()
                    .and_then(|i| i.details.storage.get(&sr))
                    .is_some_and(|d| d.content_type == "iso")
                {
                    1
                } else {
                    0
                };
                editor.server = TextEdit::new(&server);
                editor.path = TextEdit::new(&path);
                editor.version = version;
            }
            StorageLocation::Iscsi {
                portal,
                port,
                target_iqn,
                scsi_id,
            } => {
                editor.kind = 2;
                editor.server = TextEdit::new(&portal);
                editor.port = TextEdit::new(port.to_string());
                editor.iqn = TextEdit::new(&target_iqn);
                editor.scsi = TextEdit::new(&scsi_id);
            }
        }
    }
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    if s.page != Page::Storage {
        return el();
    }
    Button::new("Storage attachments and lifecycle…")
        .variant(ButtonVariant::Outline)
        .on_press(open_selected)
        .build(k)
        .key("storage-manage")
}
fn location_lines(location: &StorageLocation) -> Vec<String> {
    match location {
        StorageLocation::IsoSmb {
            server,
            share,
            path,
        } => vec![
            "Backend: ISO library over SMB 3.0".into(),
            format!("Share: //{server}/{share}/{path}"),
            "SMB 1 fallback is disabled".into(),
        ],
        StorageLocation::Nfs {
            server,
            path,
            version,
        }
        | StorageLocation::IsoNfs {
            server,
            path,
            version,
        } => vec![
            format!("Backend: {}", location.kind()),
            format!("Server: {server}"),
            format!("Export path: {path}"),
            format!(
                "NFS version: {}",
                match version {
                    NfsVersion::Automatic => "Automatic",
                    NfsVersion::V3 => "3",
                    NfsVersion::V4 => "4",
                    NfsVersion::V41 => "4.1",
                }
            ),
        ],
        StorageLocation::Iscsi {
            portal,
            port,
            target_iqn,
            scsi_id,
        } => vec![
            "Backend: LVM over iSCSI".into(),
            format!("Portal: {portal} · port {port}"),
            format!("Target IQN: {target_iqn}"),
            format!("LUN SCSI ID: {scsi_id}"),
        ],
    }
}
fn describe(operation: &StorageOperation) -> Vec<String> {
    let mut lines = vec![operation.label().into()];
    match operation {
        StorageOperation::Trim {host,..} => lines.extend([format!("Attached host: {host}"),"Asks the host storage driver to reclaim unused blocks. Completion depends on the backend and storage array; reported capacity may not change immediately.".into()]),
        StorageOperation::Create {host,name,description,location} => {
            lines.extend([format!("Name: {name}"),format!("Description: {description}"),format!("Host: {host}")]);
            lines.extend(location_lines(location));
            lines.push("Creates a shared repository. The server may create pool attachments. Review attached hosts after completion.".into());
            if matches!(location,StorageLocation::Iscsi { .. }) {lines.push("Creating LVM storage can initialize the selected LUN and erase existing data.".into());}
        }
        StorageOperation::Introduce {uuid,name,description,location} => {
            lines.extend([format!("Existing UUID: {uuid}"),format!("Name: {name}"),format!("Description: {description}")]);
            lines.extend(location_lines(location));
            lines.push("Registers existing data without formatting it. Create and plug host attachments separately after this completes.".into());
        }
        StorageOperation::CreateAttachment {host,location} => {
            lines.push(format!("Host: {host}"));lines.extend(location_lines(location));
            lines.push("Creates a detached attachment. Use Attach / repair afterward to connect this host.".into());
        }
        StorageOperation::SetAttached {sr,host,attached,..} => lines.extend([format!("Repository: {sr}"),format!("Host: {host}"),if *attached{"Authenticates/mounts the existing attachment. Attached status is not a storage health test.".into()}else{"Disconnects this host from the repository. Active disk consumers must be stopped first.".into()}]),
        StorageOperation::RemoveAttachment {sr,host} => lines.extend([format!("Repository: {sr}"),format!("Host: {host}"),"Removes the detached host attachment record, preserving storage data.".into()]),
        StorageOperation::Forget {expected_disks,..} => lines.extend([format!("Reviewed disks: {}",expected_disks.len()),"Removes repository and disk records from the pool database. Storage data remains, but dependent VM disk references must first be removed. All host attachments must be detached.".into()]),
        StorageOperation::Destroy {expected_disks,..} => lines.extend([format!("Reviewed disks: {}",expected_disks.len()),"PERMANENTLY DESTROYS THE REPOSITORY AND ITS DATA. All host attachments must be detached and all disk consumers removed. This action cannot be undone.".into()]),
    }
    lines
}
fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(editor) = &s.storage_editor else {
        return el();
    };
    let mut body = col()
        .gap(12.0)
        .child(heading("Storage administration", k).key("storage-heading"))
        .child(
            caption(
                format!("Pool {}\nTarget {}", editor.pool, editor.target.uuid()),
                k,
            )
            .wrap_anywhere(),
        );
    if editor.maintenance_review.is_some() {
        body = body.child(maintenance::review_body(s, k));
    } else if let Some((_, source, secrets)) = &editor.cleanup_review {
        body = body
            .child(heading("Review temporary Secret cleanup", k))
            .child(caption(format!("Source operation: {source}"), k).wrap_anywhere());
        for id in secrets {
            body = body.child(caption(format!("Temporary Secret: {id}"), k).wrap_anywhere());
        }
        body=body.child(caption("Deletes only the selected operation's owned, unused temporary Secrets. The backend refuses cleanup when provisioning may still be active or any storage configuration still references a Secret. Attachment-owned credential copies are preserved.",k).wrap());
    } else if let Some((_, target, operation)) = &editor.review {
        body = body
            .child(heading("Review storage change", k))
            .child(caption(format!("Target UUID: {}", target.uuid()), k).wrap_anywhere());
        for line in describe(operation) {
            body = body.child(caption(line, k).wrap_anywhere());
        }
        if let Some(label) = &editor.review_auth_label {
            body=body.child(caption(label,k).wrap()).child(caption("Credentials are sent through temporary server Secret objects and cleared from this form after submission. Verified provisioning removes unused temporary Secrets; uncertain outcomes retain their identities for explicit cleanup.",k).wrap());
        }
        if matches!(
            operation,
            StorageOperation::Forget { .. } | StorageOperation::Destroy { .. }
        ) {
            body = body
                .child(caption("Type the repository UUID to confirm", k).wrap())
                .child(
                    TextInput::new(&editor.confirmation)
                        .label("Type the repository UUID to confirm")
                        .on_edit(|s: &mut State| {
                            &mut s.storage_editor.as_mut().unwrap().confirmation
                        })
                        .build(k)
                        .key("storage-confirm-uuid"),
                );
            let disks = match operation {
                StorageOperation::Forget { expected_disks, .. }
                | StorageOperation::Destroy { expected_disks, .. } => expected_disks,
                _ => unreachable!(),
            };
            for disk in disks.iter().take(100) {
                body = body.child(caption(format!("Disk {disk}"), k).wrap_anywhere());
            }
            if disks.len() > 100 {
                body=body.child(caption("Only the first 100 disk UUIDs are shown. Review a smaller repository before removing it here.",k).wrap());
            }
        }
    } else if editor.mode == Mode::Manage {
        let ResourceTarget::Storage(sr) = editor.target else {
            return el();
        };
        body = body.child(maintenance::controls(s, k));
        body=body.child(heading(&editor.name.text,k)).child(caption("Host attachments · repair a detached connection with Attach / repair. Attachment status does not establish storage health.",k).wrap());
        if let Some(inventory) = &s.inventory {
            for attachment in inventory
                .details
                .pbds
                .values()
                .filter(|p| p.sr == Some(sr))
                .take(128)
            {
                let id = attachment.id;
                let host = attachment
                    .host
                    .and_then(|id| inventory.hosts.get(&id))
                    .map(|h| h.name.clone())
                    .unwrap_or_else(|| "Unknown host".into());
                body = body.child(heading(host, k)).child(
                    caption(
                        format!(
                            "Attachment {id}\nAttached: {} · Multipathing: {}",
                            yes_no(attachment.currently_attached),
                            yes_no(attachment.multipathed)
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                for path in &attachment.paths {
                    body = body.child(
                        caption(
                            format!("{} · {} / {} paths", path.device, path.active, path.maximum),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                }
                if attachment.requires_credentials {
                    body=body.child(caption("This attachment uses credentials. A new host attachment requires fresh credentials; existing secrets are never copied into this form.",k).wrap());
                }
                body = body.child(
                    Button::new(if attachment.currently_attached == Some(true) {
                        "Detach from host…"
                    } else {
                        "Attach / repair…"
                    })
                    .disabled(!s.demo && !s.can_mutate())
                    .on_press(move |s: &mut State| attachment_action(s, id, false))
                    .build(k),
                );
                body = body.child(
                    Button::new("Remove detached attachment…")
                        .variant(ButtonVariant::Ghost)
                        .disabled(
                            attachment.currently_attached != Some(false)
                                || (!s.demo && !s.can_mutate()),
                        )
                        .on_press(move |s: &mut State| attachment_action(s, id, true))
                        .build(k),
                );
            }
        }
        if let Some(inventory) = &s.inventory {
            let backend = inventory
                .storage
                .iter()
                .find(|r| r.uuid == sr.uuid())
                .map(|r| r.kind.as_str());
            let supported = editor.backends.iter().any(|b| {
                Some(b.kind.as_str()) == backend && b.features.iter().any(|f| f == "SR_TRIM")
            });
            if supported {
                for pbd in inventory
                    .details
                    .pbds
                    .values()
                    .filter(|p| p.sr == Some(sr) && p.currently_attached == Some(true))
                    .take(128)
                {
                    if let Some(host) = pbd.host {
                        let name = editor.name.text.clone();
                        body = body.child(
                            Button::new(format!("Reclaim unused blocks via host {host}…"))
                                .disabled(!s.demo && !s.can_mutate())
                                .on_press(move |s: &mut State| {
                                    stage(
                                        s,
                                        ResourceTarget::Storage(sr),
                                        StorageOperation::Trim {
                                            host,
                                            expected_name: name.clone(),
                                        },
                                    )
                                })
                                .build(k),
                        );
                    }
                }
            }
        }
        body = body
            .child(
                Button::new("Create missing host attachment…")
                    .on_press(new_attachment)
                    .build(k),
            )
            .child(
                Button::new("Forget repository…")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| removal(s, false))
                    .build(k),
            )
            .child(
                Button::new("Destroy repository and data…")
                    .variant(ButtonVariant::Danger)
                    .on_press(|s: &mut State| removal(s, true))
                    .build(k),
            );
    } else if editor.discovered.is_some() {
        body = body
            .child(
                caption(
                    format!(
                        "Discovered through {} · host {}",
                        editor.server.text,
                        editor
                            .host
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "unknown".into())
                    ),
                    k,
                )
                .wrap_anywhere(),
            )
            .child(discovery_results(s, editor, k))
            .child(
                Button::new("Back to storage form")
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut State| s.storage_editor.as_mut().unwrap().discovered = None)
                    .build(k),
            );
    } else {
        if editor.mode != Mode::Attachment {
            body = body.child(
                row()
                    .gap(8.0)
                    .child(
                        Button::new("Create new")
                            .variant(ButtonVariant::Outline)
                            .on_press(|s: &mut State| {
                                s.storage_editor.as_mut().unwrap().mode = Mode::Create
                            })
                            .build(k),
                    )
                    .child(
                        Button::new("Introduce existing")
                            .variant(ButtonVariant::Outline)
                            .on_press(|s: &mut State| {
                                s.storage_editor.as_mut().unwrap().mode = Mode::Introduce
                            })
                            .build(k),
                    ),
            );
            body = body
                .child(caption("Name", k).wrap())
                .child(
                    TextInput::new(&editor.name)
                        .label("Name")
                        .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().name)
                        .build(k)
                        .key("storage-name"),
                )
                .child(caption("Description", k).wrap())
                .child(
                    TextInput::new(&editor.description)
                        .label("Description")
                        .on_edit(|s: &mut State| {
                            &mut s.storage_editor.as_mut().unwrap().description
                        })
                        .build(k),
                );
        }
        body=body.child(caption("Choose a storage backend. Credentials are session-only inputs; the host stores its attachment credentials in server Secret objects.",k).wrap());
        for (kind, label) in [
            (0, "NFS virtual disks"),
            (1, "ISO library over NFS"),
            (2, "LVM over iSCSI"),
            (3, "ISO library over SMB 3.0"),
        ] {
            body = body.child(
                Button::new(format!(
                    "{}{label}",
                    if editor.kind == kind { "✓ " } else { "" }
                ))
                .variant(ButtonVariant::Outline)
                .on_press(move |s: &mut State| s.storage_editor.as_mut().unwrap().kind = kind)
                .build(k),
            );
        }
        body = body
            .child(caption("Server hostname or IP address", k).wrap())
            .child(
                TextInput::new(&editor.server)
                    .label("Server / iSCSI portal")
                    .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().server)
                    .build(k)
                    .key("storage-server"),
            );
        if editor.kind == 2 {
            body = body
                .child(caption("iSCSI port", k))
                .child(
                    TextInput::new(&editor.port)
                        .label("iSCSI port")
                        .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().port)
                        .build(k)
                        .key("storage-port"),
                )
                .child(caption("Target IQN", k).wrap())
                .child(
                    TextInput::new(&editor.iqn)
                        .label("Target IQN")
                        .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().iqn)
                        .build(k),
                )
                .child(caption("Exact LUN SCSI ID", k).wrap())
                .child(
                    TextInput::new(&editor.scsi)
                        .label("Exact LUN SCSI ID")
                        .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().scsi)
                        .build(k),
                );
        } else if editor.kind == 3 {
            body = body
                .child(caption("SMB share name", k))
                .child(
                    TextInput::new(&editor.share)
                        .label("SMB share name")
                        .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().share)
                        .build(k)
                        .key("storage-smb-share"),
                )
                .child(caption("Optional path inside share", k))
                .child(
                    TextInput::new(&editor.path)
                        .label("Path inside SMB share")
                        .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().path)
                        .build(k),
                );
        } else {
            body = body.child(caption("NFS export path", k).wrap()).child(
                TextInput::new(&editor.path)
                    .label("NFS export path")
                    .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().path)
                    .build(k),
            );
            body = body.child(caption("NFS version", k));
            let mut versions = row().gap(8.0);
            for (version, label) in [
                (NfsVersion::Automatic, "Automatic"),
                (NfsVersion::V3, "NFS 3"),
                (NfsVersion::V4, "NFS 4"),
                (NfsVersion::V41, "NFS 4.1"),
            ] {
                versions = versions.child(
                    Button::new(format!(
                        "{}{label}",
                        if editor.version == version {
                            "✓ "
                        } else {
                            ""
                        }
                    ))
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |s: &mut State| {
                        s.storage_editor.as_mut().unwrap().version = version
                    })
                    .build(k),
                );
            }
            body = body.child(versions);
        }
        if matches!(editor.kind, 2 | 3) {
            body = body.child(
                Checkbox::new(editor.authenticate)
                    .label("Use storage credentials")
                    .on_toggle(|s: &mut State| {
                        let e = s.storage_editor.as_mut().unwrap();
                        e.authenticate = !e.authenticate;
                        e.auth_epoch += 1;
                    })
                    .build(k)
                    .key("storage-auth"),
            );
            if editor.authenticate {
                if editor.kind == 2 {
                    body = body.child(
                        Checkbox::new(editor.mutual)
                            .label("Mutual CHAP")
                            .on_toggle(|s: &mut State| {
                                let e = s.storage_editor.as_mut().unwrap();
                                e.mutual = !e.mutual;
                                e.auth_epoch += 1;
                            })
                            .build(k),
                    );
                }
                body = body
                    .child(
                        caption(
                            if editor.kind == 3 {
                                "SMB username (DOMAIN\\user if needed)"
                            } else {
                                "CHAP username"
                            },
                            k,
                        )
                        .wrap(),
                    )
                    .child(
                        TextInput::new(&editor.username)
                            .label("Storage username")
                            .on_edit(|s: &mut State| {
                                let e = s.storage_editor.as_mut().unwrap();
                                e.auth_epoch += 1;
                                &mut e.username
                            })
                            .build(k)
                            .key("storage-username"),
                    )
                    .child(
                        TextInput::new(&editor.password)
                            .secure()
                            .label("Storage password")
                            .placeholder("Storage password")
                            .on_edit(|s: &mut State| {
                                let e = s.storage_editor.as_mut().unwrap();
                                e.auth_epoch += 1;
                                &mut e.password
                            })
                            .build(k)
                            .key("storage-password"),
                    );
                if editor.kind == 2 && editor.mutual {
                    body = body
                        .child(caption("Incoming CHAP credentials", k))
                        .child(
                            TextInput::new(&editor.incoming_username)
                                .label("Incoming CHAP username")
                                .on_edit(|s: &mut State| {
                                    let e = s.storage_editor.as_mut().unwrap();
                                    e.auth_epoch += 1;
                                    &mut e.incoming_username
                                })
                                .build(k),
                        )
                        .child(
                            TextInput::new(&editor.incoming_password)
                                .secure()
                                .label("Incoming CHAP password")
                                .on_edit(|s: &mut State| {
                                    let e = s.storage_editor.as_mut().unwrap();
                                    e.auth_epoch += 1;
                                    &mut e.incoming_password
                                })
                                .build(k),
                        );
                }
            }
        }
        body = body.child(caption("Host for creation, attachment or discovery", k).wrap());
        if let Some(inventory) = &s.inventory {
            for host in inventory.hosts.values().take(128) {
                let id = host.id;
                let existing = editor.mode == Mode::Attachment
                    && inventory.details.pbds.values().any(|p| {
                        p.host == Some(id)
                            && p.sr
                                .is_some_and(|sr| editor.target == ResourceTarget::Storage(sr))
                    });
                body = body.child(
                    Button::new(format!(
                        "{}{}{}",
                        if editor.host == Some(id) { "✓ " } else { "" },
                        host.name,
                        if existing {
                            " · attachment exists"
                        } else {
                            ""
                        }
                    ))
                    .disabled(existing)
                    .variant(ButtonVariant::Outline)
                    .on_press(move |s: &mut State| {
                        s.storage_editor.as_mut().unwrap().host = Some(id)
                    })
                    .build(k),
                );
            }
        }
        body = body.child(
            Button::new("Probe selected location for existing repositories")
                .disabled(editor.discovery.is_some() || s.demo)
                .on_press(|s: &mut State| query(s, 1))
                .build(k),
        );
        let mut scan=col().gap(8.0).child(caption("Discovery asks the selected host to contact storage. The backend can temporarily mount an export or open an iSCSI session; no repository is created.",k).wrap());
        for (step, label) in if editor.kind == 2 {
            vec![
                (3, "Discover iSCSI targets"),
                (4, "Discover LUNs for selected IQN"),
            ]
        } else if editor.kind == 3 {
            vec![]
        } else {
            vec![(2, "Discover NFS exports")]
        } {
            scan = scan.child(
                Button::new(label)
                    .disabled(editor.discovery.is_some() || s.demo)
                    .on_press(move |s: &mut State| query(s, step))
                    .build(k),
            );
        }
        body = body.child(scan);
        if editor.mode == Mode::Introduce {
            body = body
                .child(caption("Existing repository UUID", k).wrap())
                .child(
                    TextInput::new(&editor.uuid)
                        .label("Existing repository UUID")
                        .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().uuid)
                        .build(k),
                );
        }
        if let Some(backend) = editor
            .backends
            .iter()
            .find(|b| b.kind == location(editor).kind())
        {
            body = body.child(
                caption(
                    format!(
                        "Host storage driver: {} · Discovery: {}",
                        backend.name,
                        if backend.features.iter().any(|f| f == "SR_PROBE") {
                            "supported"
                        } else {
                            "not advertised"
                        }
                    ),
                    k,
                )
                .wrap(),
            );
        }
    }
    if editor.review.is_none()
        && editor.cleanup_review.is_none()
        && editor.maintenance_review.is_none()
    {
        for record in s
            .resource_rows
            .iter()
            .filter(|r| {
                r.pool == editor.pool
                    && matches!(
                        r.action,
                        xenadmin_core::ResourceAction::AuthenticatedStorage { .. }
                    )
                    && r.storage_secrets.iter().any(|s| !s.cleaned)
            })
            .take(20)
        {
            let id = record.id;
            body = body.child(
                caption(
                    format!("Temporary storage credentials retained · operation {id}"),
                    k,
                )
                .wrap_anywhere(),
            );
            for secret in record.storage_secrets.iter().filter(|v| !v.cleaned) {
                body = body.child(caption(format!("Secret {}", secret.secret), k).wrap_anywhere());
            }
            body = body.child(
                Button::new("Review unused Secret cleanup…")
                    .disabled(!record.state.terminal() || !s.can_mutate())
                    .on_press(move |s: &mut State| stage_cleanup(s, id))
                    .build(k),
            );
            if !record.state.terminal() {
                body=body.child(caption("Resolve or acknowledge the uncertain source operation in Activity before separate cleanup.",k).wrap());
            }
        }
    }
    if editor.discovery.is_some() {
        body = body.child(caption("Reading storage discovery…", k));
    }
    if let Some(error) = &editor.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .role("alert")
                .color(k.c.danger.icon),
        );
    }
    if s.connection.as_ref().is_some_and(|c| c.is_read_only()) {
        body=body.child(caption("Read-only connection: discovery and inspection are available; changes are disabled.",k).wrap());
    }
    if s.demo {
        body = body.child(caption(
            "Synthetic preview · changes cannot be submitted.",
            k,
        ));
    }
    let permission = editor
        .maintenance_review
        .as_ref()
        .map(|op| crate::session_permissions_view::assess(s, &op.permission_requirements()))
        .or_else(|| {
            editor.review.as_ref().map(|(_, _, operation)| {
                let requirements = operation.permission_requirements();
                let requirements = if editor.review_auth_label.is_some() {
                    requirements.with_host_dependent_steps()
                } else {
                    requirements
                };
                crate::session_permissions_view::assess(s, &requirements)
            })
        });
    let mut footer = row().gap(8.0).flex_wrap().justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.storage_editor = None)
            .build(k)
            .key("storage-close"),
    );
    if editor.maintenance_review.is_some() {
        footer = footer.child(maintenance::footer(s, k));
    } else if editor.cleanup_review.is_some() {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| {
                        s.storage_editor.as_mut().unwrap().cleanup_review = None
                    })
                    .build(k),
            )
            .child(
                Button::new("Delete unused temporary Secrets")
                    .disabled(s.demo || !s.can_mutate())
                    .on_press(apply_cleanup)
                    .build(k)
                    .key("storage-cleanup-apply"),
            );
    } else if let Some((_, _, operation)) = &editor.review {
        let destructive = matches!(
            operation,
            StorageOperation::Forget { .. } | StorageOperation::Destroy { .. }
        );
        let too_many = matches!(operation,StorageOperation::Forget{expected_disks,..}|StorageOperation::Destroy{expected_disks,..} if expected_disks.len()>100);
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| {
                        let e = s.storage_editor.as_mut().unwrap();
                        e.review = None;
                        e.review_credentials = None;
                        e.review_auth_label = None;
                    })
                    .build(k),
            )
            .child(
                Button::new("Confirm storage change")
                    .variant(if destructive {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Outline
                    })
                    .disabled(
                        s.demo
                            || !s.can_mutate()
                            || !current(s.snapshot.as_deref(), editor)
                            || permission
                                .as_ref()
                                .is_some_and(|p| p.decision().is_denied())
                            || too_many
                            || (destructive
                                && editor.confirmation.text != editor.target.uuid().to_string()),
                    )
                    .on_press(apply)
                    .build(k)
                    .key("storage-confirm"),
            );
    } else if editor.mode != Mode::Manage && editor.discovered.is_none() {
        footer = footer.child(
            Button::new("Review")
                .on_press(review)
                .build(k)
                .key("storage-review"),
        );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Storage administration")
        .modal(false, |s: &mut State| s.storage_editor = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 680.0))
                .h((s.window_height - 56.0).clamp(280.0, 820.0))
                .padding(20.0)
                .gap(16.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(crate::session_permissions_view::footer(
                    footer,
                    permission.as_ref(),
                    (s.window_width - 32.0).clamp(280.0, 680.0) - 40.0,
                    k,
                )),
        )
}

pub(super) fn scene(s: &mut State, review_page: bool) {
    crate::demo(s);
    s.page = Page::Storage;
    s.rebuild();
    open_new(s);
    if let Some(editor) = &mut s.storage_editor {
        editor.name = TextEdit::new("Lab NFS storage");
        editor.description = TextEdit::new("Synthetic preview");
        editor.server = TextEdit::new("nfs.example.test");
        editor.path = TextEdit::new("/vm-images");
    }
    if review_page {
        review(s);
    }
}

pub(super) fn discovery_scene(s: &mut State) {
    scene(s, false);
    let e = s.storage_editor.as_mut().unwrap();
    e.kind = 2;
    e.name = TextEdit::new("Lab iSCSI storage");
    e.server = TextEdit::new("storage.example.test");
    e.port = TextEdit::new("3261");
    e.query_inputs = Some(inputs(e));
    e.discovered = Some(StorageDiscoveryResult::IscsiTargets(vec![
        xenadmin_core::DiscoveredIscsiTarget {
            portal: "192.0.2.50".into(),
            port: 3261,
            iqn: "iqn.2026-11.example:lab-storage".into(),
        },
    ]));
}

pub(super) fn authenticated_scene(s: &mut State) {
    scene(s, false);
    let e = s.storage_editor.as_mut().unwrap();
    e.kind = 3;
    e.authenticate = true;
    e.name = TextEdit::new("Lab ISO library");
    e.server = TextEdit::new("smb.example.test");
    e.share = TextEdit::new("images");
    e.path = TextEdit::new("install");
    e.username = TextEdit::new("LAB\\fixture");
    e.password = secure_field();
    e.password.text = "synthetic-only-password".into();
    review(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_storage_review_is_captured_and_footer_remains_reachable() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            authenticated_scene(&mut s);
            let e = s.storage_editor.as_mut().unwrap();
            assert!(e.review_credentials.is_some());
            e.server = TextEdit::new("changed.example.test");
            e.username = TextEdit::new("changed");
            assert!(
                e.review_auth_label
                    .as_ref()
                    .unwrap()
                    .contains("LAB\\fixture")
            );
            assert!(
                matches!(&e.review.as_ref().unwrap().2,StorageOperation::Create{location:StorageLocation::IsoSmb{server,..},..} if server=="smb.example.test")
            );
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
            let (x, y) = rt.find("storage-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn storage_discovery_result_fence_includes_credential_changes() {
        let mut s = crate::init();
        discovery_scene(&mut s);
        let e = s.storage_editor.as_mut().unwrap();
        e.authenticate = true;
        e.query_inputs = Some(inputs(e));
        e.auth_epoch += 1;
        select_result(&mut s, 0);
        assert_eq!(
            s.storage_editor.as_ref().unwrap().server.text,
            "storage.example.test"
        );
        assert!(s.storage_editor.as_ref().unwrap().error.is_some());
        scene(&mut s, false);
        let e = s.storage_editor.as_mut().unwrap();
        e.kind = 3;
        e.authenticate = true;
        e.share = TextEdit::new("images");
        review(&mut s);
        assert!(s.storage_editor.as_ref().unwrap().review.is_none());
    }
    #[test]
    fn discovery_selection_is_reachable_bound_to_inputs_and_keeps_review_separate() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            discovery_scene(&mut s);
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
            rt.update(|_, tx| tx.scroll_into_view("storage-result-0"));
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let (x, y) = rt.find("storage-result-0").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            let e = rt.state.storage_editor.as_ref().unwrap();
            assert_eq!(e.server.text, "192.0.2.50");
            assert_eq!(e.port.text, "3261");
            assert!(e.review.is_none());
            assert!(e.discovered.is_none());
            assert!(rt.state.resource_operations.is_empty());
            let (x, y) = rt.find("storage-review").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
        }
        let mut s = crate::init();
        discovery_scene(&mut s);
        s.storage_editor.as_mut().unwrap().server = TextEdit::new("changed.example.test");
        select_result(&mut s, 0);
        assert_eq!(
            s.storage_editor.as_ref().unwrap().server.text,
            "changed.example.test"
        );
        discovery_scene(&mut s);
        s.storage_editor.as_mut().unwrap().port = TextEdit::new("0");
        review(&mut s);
        assert!(s.storage_editor.as_ref().unwrap().review.is_none());
        assert!(
            s.storage_editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("65535")
        );
    }
    #[test]
    fn chosen_lun_requires_separate_destructive_review() {
        let mut s = crate::init();
        discovery_scene(&mut s);
        let e = s.storage_editor.as_mut().unwrap();
        e.iqn = TextEdit::new("iqn.2026-11.example:test");
        e.query_inputs = Some(inputs(e));
        e.discovered = Some(StorageDiscoveryResult::IscsiLuns(
            (0..25)
                .map(|i| xenadmin_core::DiscoveredIscsiLun {
                    scsi_id: format!("3600fixture{i}"),
                    lun: Some(i),
                    size_bytes: Some(4096),
                    vendor: "Fixture".into(),
                    serial: String::new(),
                })
                .collect(),
        ));
        select_result(&mut s, 24);
        let e = s.storage_editor.as_ref().unwrap();
        assert_eq!(e.scsi.text, "3600fixture24");
        assert!(e.review.is_none());
        assert!(s.resource_operations.is_empty());
        review(&mut s);
        let e = s.storage_editor.as_ref().unwrap();
        assert!(
            matches!(&e.review.as_ref().unwrap().2,StorageOperation::Create{location:StorageLocation::Iscsi{port:3261,scsi_id,..},..} if scsi_id=="3600fixture24")
        );
        assert!(
            describe(&e.review.as_ref().unwrap().2)
                .iter()
                .any(|l| l.contains("erase existing data"))
        );
    }
    #[test]
    fn storage_review_preserves_inputs_and_compact_footer_never_submits_preview() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            crate::demo(&mut s);
            open_new(&mut s);
            let editor = s.storage_editor.as_mut().unwrap();
            editor.name = TextEdit::new("Fixture NFS");
            editor.server = TextEdit::new("nfs.example.test");
            editor.path = TextEdit::new("/images");
            let pool = editor.pool;
            let mut runtime = rune::Runtime::new(
                s,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let (x, y) = runtime.find("storage-review").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            let editor = runtime.state.storage_editor.as_ref().unwrap();
            let (_, target, op) = editor.review.as_ref().unwrap();
            assert_eq!(*target, ResourceTarget::Pool(pool));
            assert!(matches!(op,StorageOperation::Create{name,..} if name=="Fixture NFS"));
            let (x, y) = runtime.find("storage-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
        }
    }
    #[test]
    fn storage_forget_and_destroy_have_distinct_review_and_exact_disk_set() {
        let mut s = crate::init();
        crate::demo(&mut s);
        s.page = Page::Storage;
        s.rebuild();
        s.table.select(s.rows[0].id.clone());
        open_selected(&mut s);
        let target = s.storage_editor.as_ref().unwrap().target;
        removal(&mut s, false);
        assert!(matches!(
            s.storage_editor
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .2,
            StorageOperation::Forget { .. }
        ));
        removal(&mut s, true);
        let editor = s.storage_editor.as_ref().unwrap();
        assert_eq!(editor.target, target);
        assert!(matches!(
            editor.review.as_ref().unwrap().2,
            StorageOperation::Destroy { .. }
        ));
        assert!(
            describe(&editor.review.as_ref().unwrap().2)
                .iter()
                .any(|s| s.contains("PERMANENTLY DESTROYS"))
        );
        s.page = Page::Guests;
        s.rebuild();
        assert_eq!(s.storage_editor.as_ref().unwrap().target, target);
    }
}
