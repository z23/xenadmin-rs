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
    pool: PoolId,
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    mode: Mode,
    name: TextEdit,
    description: TextEdit,
    server: TextEdit,
    path: TextEdit,
    iqn: TextEdit,
    scsi: TextEdit,
    uuid: TextEdit,
    confirmation: TextEdit,
    kind: u8,
    version: NfsVersion,
    host: Option<HostId>,
    review: Option<(Option<ObservedResource>, ResourceTarget, StorageOperation)>,
    discovery: Option<StorageDiscovery>,
    query_location: Option<(HostId, StorageLocation)>,
    backends: Vec<StorageBackend>,
    discovered: Vec<xenadmin_core::DiscoveredRepository>,
    probe_completed: bool,
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
        path: TextEdit::new("/"),
        iqn: TextEdit::new(""),
        scsi: TextEdit::new(""),
        uuid: TextEdit::new(""),
        confirmation: TextEdit::new(""),
        kind: 0,
        version: NfsVersion::Automatic,
        host: inventory.hosts.keys().next().copied(),
        review: None,
        discovery: None,
        query_location: None,
        backends: Vec::new(),
        discovered: Vec::new(),
        probe_completed: false,
        error: None,
    });
    query(s, false);
}
fn location(editor: &Editor) -> StorageLocation {
    match editor.kind {
        1 => StorageLocation::IsoNfs {
            server: editor.server.text.trim().into(),
            path: editor.path.text.trim().into(),
            version: editor.version,
        },
        2 => StorageLocation::Iscsi {
            portal: editor.server.text.trim().into(),
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
fn query(s: &mut State, probe: bool) {
    let result = (|| {
        let editor = s.storage_editor.as_ref().ok_or("Storage view is closed")?;
        let snapshot = s
            .snapshot
            .as_ref()
            .ok_or("Connect to discover storage backends")?;
        if snapshot
            .inventory()
            .is_none_or(|i| i.pool_id != editor.pool)
        {
            return Err("The connection changed; reopen storage".into());
        }
        let observed = snapshot
            .observe_resource(ResourceTarget::Pool(editor.pool))
            .map_err(|e| e.to_string())?;
        let pair = if probe {
            Some((editor.host.ok_or("Choose a host")?, location(editor)))
        } else {
            None
        };
        let query = pair
            .clone()
            .map(|(host, location)| StorageQuery::Probe { host, location })
            .unwrap_or(StorageQuery::Backends);
        let handle = s
            .connection
            .as_ref()
            .ok_or("Connect before discovery")?
            .discover_storage(observed, query)
            .map_err(|e| e.to_string())?;
        if let Some(waker) = &s.waker {
            handle.notify_on_change(waker.clone());
        }
        Ok::<_, String>((handle, pair))
    })();
    if let Some(editor) = &mut s.storage_editor {
        match result {
            Ok((handle, pair)) => {
                editor.discovery = Some(handle);
                editor.query_location = pair;
                editor.error = None;
                editor.discovered.clear();
                editor.probe_completed = false;
            }
            Err(error) => {
                if !s.demo {
                    editor.error = Some(error);
                }
            }
        }
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(editor) = s.storage_editor.as_mut() else {
        return false;
    };
    let Some(result) = editor.discovery.as_ref().and_then(StorageDiscovery::latest) else {
        return false;
    };
    editor.discovery = None;
    if editor
        .query_location
        .as_ref()
        .is_some_and(|(host, config)| Some(*host) != editor.host || *config != location(editor))
    {
        editor.error = Some(
            "Storage inputs changed during discovery. Probe again with the reviewed inputs.".into(),
        );
        return true;
    }
    match result {
        Ok(StorageDiscoveryResult::Backends(backends)) => editor.backends = backends,
        Ok(StorageDiscoveryResult::Repositories(found)) => {
            editor.discovered = found;
            editor.probe_completed = true;
            editor.error = None;
        }
        Err(error) => editor.error = Some(error),
    }
    true
}
fn review(s: &mut State) {
    let result = (|| {
        let editor = s.storage_editor.as_ref().ok_or("Storage view is closed")?;
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
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
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
        match template {
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
                target_iqn,
                scsi_id,
            } => {
                editor.kind = 2;
                editor.server = TextEdit::new(&portal);
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
            target_iqn,
            scsi_id,
        } => vec![
            "Backend: LVM over iSCSI".into(),
            format!("Portal: {portal} · port 3260"),
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
        .child(heading("Storage administration", k))
        .child(
            caption(
                format!("Pool {}\nTarget {}", editor.pool, editor.target.uuid()),
                k,
            )
            .wrap_anywhere(),
        );
    if let Some((_, target, operation)) = &editor.review {
        body = body
            .child(heading("Review storage change", k))
            .child(caption(format!("Target UUID: {}", target.uuid()), k).wrap_anywhere());
        for line in describe(operation) {
            body = body.child(caption(line, k).wrap_anywhere());
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
                    body=body.child(caption("This attachment uses credentials. Existing attach/detach works; creating another authenticated attachment is not implemented.",k).wrap());
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
        body=body.child(caption("Backends: NFS, ISO over NFS, and unauthenticated LVM over iSCSI. CHAP and SMB provisioning are not implemented.",k).wrap());
        for (kind, label) in [
            (0, "NFS virtual disks"),
            (1, "ISO library over NFS"),
            (2, "LVM over iSCSI"),
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
            .child(caption("Server hostname or IP address · iSCSI uses port 3260", k).wrap())
            .child(
                TextInput::new(&editor.server)
                    .label("Server / iSCSI portal")
                    .on_edit(|s: &mut State| &mut s.storage_editor.as_mut().unwrap().server)
                    .build(k)
                    .key("storage-server"),
            );
        if editor.kind == 2 {
            body = body
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
                .on_press(|s: &mut State| query(s, true))
                .build(k),
        );
        let probe_current = editor
            .query_location
            .as_ref()
            .is_some_and(|(host, config)| {
                Some(*host) == editor.host && *config == location(editor)
            });
        if editor.probe_completed && probe_current && editor.discovered.is_empty() {
            body = body.child(
                caption(
                    "No existing repositories were reported at this location.",
                    k,
                )
                .wrap(),
            );
        }
        if editor.probe_completed && !probe_current {
            body=body.child(caption("Inputs changed since the last probe. Probe again before using discovered repositories.",k).wrap());
        }
        for found in editor.discovered.iter().filter(|_| probe_current) {
            let id = found.uuid;
            body = body.child(
                Button::new(format!("{} · {}", found.name, id))
                    .on_press(move |s: &mut State| {
                        let editor = s.storage_editor.as_mut().unwrap();
                        editor.uuid = TextEdit::new(id.to_string());
                        editor.mode = Mode::Introduce;
                    })
                    .build(k),
            );
        }
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
    let mut footer = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.storage_editor = None)
            .build(k),
    );
    if let Some((_, _, operation)) = &editor.review {
        let destructive = matches!(
            operation,
            StorageOperation::Forget { .. } | StorageOperation::Destroy { .. }
        );
        let too_many = matches!(operation,StorageOperation::Forget{expected_disks,..}|StorageOperation::Destroy{expected_disks,..} if expected_disks.len()>100);
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| s.storage_editor.as_mut().unwrap().review = None)
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
                            || too_many
                            || (destructive
                                && editor.confirmation.text != editor.target.uuid().to_string()),
                    )
                    .on_press(apply)
                    .build(k)
                    .key("storage-confirm"),
            );
    } else if editor.mode != Mode::Manage {
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
                .child(footer),
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

#[cfg(test)]
mod tests {
    use super::*;
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
