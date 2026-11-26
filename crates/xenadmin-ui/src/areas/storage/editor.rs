//! The logic carried over from the Rune modules `storage_view.rs` and
//! `storage_local_view.rs`: the editor of a storage change, discovery and
//! its fence, the review, the staleness check and the submission. Texts of
//! errors and reviews are the Rune ones, word for word.
//!
//! What changed with the port:
//!
//! - `rune::TextEdit` is `String`, the two password fields are `SecretText`.
//! - The editor follows the pool of its target (`Editor::pool`), not the
//!   scope session: `m.inventory_for`, `m.snapshot_for`, `m.connection_for`,
//!   `m.is_synthetic_pool`, `permissions::assess_for`.
//! - The Rune hub ("manage" mode) is gone. What its buttons did is here as
//!   functions that open the editor on the repository and stage the review:
//!   [`review_removal`], [`review_attachment`], [`open_attachment`],
//!   [`open_trim`], [`review_restore`]. The commands call them.
//! - `maintenance_review` and `repair_selection` (repair across hosts, read
//!   caching) are not fields here: they are the `storage_maintenance` area's.
//! - `step` and `trim` are new: the wizard step, and whether a dialog opened
//!   on a repository is the chooser of the host to reclaim blocks through.

use xenadmin_core::{
    ConnectionSnapshot, DiscoveredHbaDevice, Error, HostId, LocalDeviceDeclaration,
    LocalStorageKind, NfsVersion, ObservedResource, OperationId, PbdId, PermissionAssessment,
    PoolId, ResourceAction, ResourceCommand, ResourceTarget, SecretId, SrId, StorageAllocation,
    StorageBackend, StorageCredentials, StorageDiscovery, StorageDiscoveryResult, StorageLocation,
    StorageOperation, StorageQuery,
};

use crate::prelude::*;

/// What `stage` says when the connection the editor was opened on is not
/// the current one any more. Also the reason a stale review cannot be
/// confirmed.
pub const STALE: &str = "The connection changed; reopen this dialog before reviewing.";

/// The nine backends, by the number the Rune editor gave them.
pub const KINDS: [(u8, &str); 9] = [
    (0, "NFS virtual disks"),
    (1, "ISO library over NFS"),
    (2, "LVM over iSCSI"),
    (3, "ISO library over SMB 3.0"),
    (4, "LVM over HBA / Fibre Channel"),
    (5, "Local EXT"),
    (6, "Local LVM"),
    (7, "Local XFS"),
    (8, "SMB 3.0 virtual disks"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Create,
    Introduce,
    /// Opened on an existing repository for one reviewed change.
    Manage,
    Attachment,
}

pub struct Editor {
    pub(super) pool: PoolId,
    pub(super) target: ResourceTarget,
    pub(super) observed: Option<ObservedResource>,
    pub(super) mode: Mode,
    /// The wizard step while nothing is reviewed: 0 Type, 1 Name, 2 Location.
    pub(super) step: usize,
    /// A dialog on a repository that offers the hosts to reclaim blocks through.
    pub(super) trim: bool,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) server: String,
    pub(super) port: String,
    pub(super) path: String,
    pub(super) share: String,
    pub(super) authenticate: bool,
    pub(super) mutual: bool,
    pub(super) auth_epoch: u64,
    pub(super) username: String,
    pub(super) password: SecretText,
    pub(super) incoming_username: String,
    pub(super) incoming_password: SecretText,
    pub(super) review_credentials: Option<StorageCredentials>,
    pub(super) review_auth_label: Option<String>,
    pub(super) cleanup_review: Option<(ObservedResource, OperationId, Vec<SecretId>)>,
    pub(super) iqn: String,
    pub(super) scsi: String,
    pub(super) uuid: String,
    pub(super) confirmation: String,
    pub(super) kind: u8,
    pub(super) allocation: StorageAllocation,
    pub(super) hba_device: Option<(HostId, DiscoveredHbaDevice)>,
    pub(super) declaration: String,
    pub(super) local_acknowledged: bool,
    pub(super) local_introduction: Option<(u8, String)>,
    /// The repository a local probe row filled the form with, and the drivers whose
    /// probes reported it. A driver that did not report it cannot introduce it.
    pub(super) local_reported: Option<(String, Vec<LocalStorageKind>)>,
    pub(super) local_probe: Option<(HostId, String, bool)>,
    pub(super) version: NfsVersion,
    pub(super) host: Option<HostId>,
    pub(super) review: Option<(Option<ObservedResource>, ResourceTarget, StorageOperation)>,
    pub(super) discovery: Option<StorageDiscovery>,
    pub(super) query_inputs: Option<DiscoveryInputs>,
    pub(super) query_observed: Option<ObservedResource>,
    pub(super) backends: Vec<StorageBackend>,
    pub(super) discovered: Option<StorageDiscoveryResult>,
    pub(super) result_page: usize,
    pub(super) error: Option<String>,
}

impl Editor {
    pub fn pool(&self) -> PoolId {
        self.pool
    }

    pub fn target(&self) -> ResourceTarget {
        self.target
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
}

/// The New Storage Repository wizard for a pool. `false`: the pool has no
/// inventory, nothing opened.
pub fn open_new(m: &mut Model, pool: PoolId) -> bool {
    open(m, pool, None)
}

pub(super) fn open(m: &mut Model, pool: PoolId, sr: Option<SrId>) -> bool {
    let Some(inventory) = m.inventory_for(pool) else {
        return false;
    };
    let target = sr
        .map(ResourceTarget::Storage)
        .unwrap_or(ResourceTarget::Pool(inventory.pool_id));
    let observed = m
        .snapshot_for(pool)
        .and_then(|s| s.observe_resource(target).ok());
    let name = sr
        .and_then(|id| inventory.storage.iter().find(|sr| sr.uuid == id.uuid()))
        .map(|sr| sr.name.as_str())
        .unwrap_or("");
    let description = sr
        .and_then(|id| inventory.details.storage.get(&id))
        .map(|sr| sr.description.as_str())
        .unwrap_or("");
    m.areas.storage.editor = Some(Editor {
        pool: inventory.pool_id,
        target,
        observed,
        mode: if sr.is_some() {
            Mode::Manage
        } else {
            Mode::Create
        },
        step: 0,
        trim: false,
        name: name.to_owned(),
        description: description.to_owned(),
        server: String::new(),
        port: "3260".to_owned(),
        path: "/".to_owned(),
        share: String::new(),
        authenticate: false,
        mutual: false,
        auth_epoch: 0,
        username: String::new(),
        password: SecretText::new(),
        incoming_username: String::new(),
        incoming_password: SecretText::new(),
        review_credentials: None,
        review_auth_label: None,
        cleanup_review: None,
        iqn: String::new(),
        scsi: String::new(),
        uuid: String::new(),
        confirmation: String::new(),
        kind: 0,
        allocation: StorageAllocation::DriverDefault,
        hba_device: None,
        declaration: String::new(),
        local_acknowledged: false,
        local_introduction: None,
        local_reported: None,
        local_probe: None,
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
    query(m, 0);
    true
}

pub(super) fn is_local(e: &Editor) -> bool {
    matches!(e.kind, 5..=7)
}

pub(super) fn local_kind_index(kind: LocalStorageKind) -> u8 {
    match kind {
        LocalStorageKind::Ext => 5,
        LocalStorageKind::Lvm => 6,
        LocalStorageKind::Xfs => 7,
    }
}

/// The drivers whose probes reported the repository the form names, when a probe row
/// named it.
pub(super) fn reported_kinds(e: &Editor) -> Option<&[LocalStorageKind]> {
    e.local_reported
        .as_ref()
        .filter(|(uuid, _)| uuid == e.uuid.trim())
        .map(|(_, kinds)| kinds.as_slice())
}

pub(super) fn kind_labels(kinds: &[LocalStorageKind]) -> String {
    kinds
        .iter()
        .map(|kind| kind.label())
        .collect::<Vec<_>>()
        .join(" / ")
}

pub(super) fn location(editor: &Editor) -> StorageLocation {
    let allocation = if editor.mode == Mode::Introduce {
        StorageAllocation::DriverDefault
    } else {
        editor.allocation
    };
    match editor.kind {
        5..=7 => StorageLocation::Local {
            kind: if editor.kind == 5 {
                LocalStorageKind::Ext
            } else if editor.kind == 6 {
                LocalStorageKind::Lvm
            } else {
                LocalStorageKind::Xfs
            },
            device: editor.path.trim().into(),
        },
        8 => StorageLocation::Smb {
            server: editor.server.trim().into(),
            share: editor.share.trim().into(),
        },
        4 => StorageLocation::Hba {
            scsi_id: editor.scsi.trim().into(),
            allocation,
        },
        3 => StorageLocation::IsoSmb {
            server: editor.server.trim().into(),
            share: editor.share.trim().into(),
            path: editor.path.trim_matches('/').into(),
        },
        1 => StorageLocation::IsoNfs {
            server: editor.server.trim().into(),
            path: editor.path.trim().into(),
            version: editor.version,
        },
        2 => StorageLocation::Iscsi {
            portal: editor.server.trim().into(),
            port: editor.port.trim().parse().unwrap_or(0),
            target_iqn: editor.iqn.trim().into(),
            scsi_id: editor.scsi.trim().into(),
            allocation,
        },
        _ => StorageLocation::Nfs {
            server: editor.server.trim().into(),
            path: editor.path.trim().into(),
            version: editor.version,
        },
    }
}

/// Whether the driver of the chosen backend advertises allocation choices.
pub(super) fn allocation_advertised(e: &Editor) -> bool {
    e.backends
        .iter()
        .any(|b| b.kind == location(e).kind() && b.configuration.contains_key("allocation"))
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct DiscoveryInputs {
    host: Option<HostId>,
    kind: u8,
    version: NfsVersion,
    allocation: StorageAllocation,
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

pub(super) fn inputs(e: &Editor) -> DiscoveryInputs {
    DiscoveryInputs {
        host: e.host,
        kind: e.kind,
        version: e.version,
        allocation: e.allocation,
        server: e.server.clone(),
        port: e.port.clone(),
        path: e.path.clone(),
        iqn: e.iqn.clone(),
        scsi: e.scsi.clone(),
        share: e.share.clone(),
        auth_epoch: e.auth_epoch,
        authenticate: e.authenticate,
        mutual: e.mutual,
    }
}

pub(super) fn results_current(m: &Model, e: &Editor) -> bool {
    e.query_inputs.as_ref() == Some(&inputs(e))
        && (m.is_synthetic_pool(e.pool)
            || e.query_observed.is_some_and(|o| {
                m.snapshot_for(e.pool)
                    .and_then(|s| s.observe_resource(o.target()).ok())
                    == Some(o)
            }))
}

/// Starts a discovery. Steps: 0 backends, 1 probe of the location, 2 NFS
/// exports, 3 iSCSI targets, 4 iSCSI LUNs, 5 HBA devices, 6 local device.
pub(super) fn query(m: &mut Model, step: u8) {
    if step == 6
        && let Some(e) = &mut m.areas.storage.editor
    {
        e.local_probe = None;
        e.local_reported = None;
    }
    let result = (|| {
        let e = m
            .areas
            .storage
            .editor
            .as_ref()
            .ok_or("Storage view is closed")?;
        let snapshot = m.snapshot_for(e.pool).ok_or("Connect to discover storage")?;
        if snapshot.inventory().is_none_or(|i| i.pool_id != e.pool) {
            return Err("Connection changed; reopen this dialog".into());
        }
        let observed = snapshot
            .observe_resource(ResourceTarget::Pool(e.pool))
            .map_err(|e| e.to_string())?;
        let host = e.host.ok_or("Choose a host")?;
        let server = e.server.trim().to_string();
        let port = e.port.trim().parse().unwrap_or(0);
        let query = match step {
            0 => StorageQuery::Backends,
            5 => StorageQuery::HbaDevices { host },
            6 => StorageQuery::LocalDevice {
                host,
                device: e.path.trim().into(),
            },
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
                target_iqn: e.iqn.trim().into(),
            },
        };
        let connection = m
            .connection_for(e.pool)
            .ok_or("Connect before discovery")?;
        let handle = if step != 0 && e.authenticate && matches!(e.kind, 2 | 3 | 8) {
            connection.discover_authenticated_storage(
                observed,
                query,
                credentials(e).ok_or("Enter the storage username and password")?,
            )
        } else {
            connection.discover_storage(observed, query)
        }
        .map_err(|e| e.to_string())?;
        if let Some(waker) = &m.waker {
            handle.notify_on_change(waker.clone());
        }
        Ok::<_, String>((handle, inputs(e), observed))
    })();
    let synthetic = m
        .areas
        .storage
        .editor
        .as_ref()
        .is_some_and(|e| m.is_synthetic_pool(e.pool));
    if let Some(e) = &mut m.areas.storage.editor {
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
                if !synthetic {
                    e.error = Some(error);
                }
            }
        }
    }
}

/// Reads the discovery handle. `true` when a result arrived.
pub fn poll(m: &mut Model, _clock: f64) -> bool {
    let Some(e) = m.areas.storage.editor.as_ref() else {
        return false;
    };
    let Some(result) = e.discovery.as_ref().and_then(StorageDiscovery::latest) else {
        return false;
    };
    let current = results_current(m, e);
    let Some(e) = m.areas.storage.editor.as_mut() else {
        return false;
    };
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
            if let StorageDiscoveryResult::LocalRepositories(rows) = &result {
                e.local_probe = e
                    .host
                    .map(|host| (host, e.path.trim().into(), rows.is_empty()));
            }
            e.discovered = Some(result);
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
    true
}

/// Fills the form from a discovery row. It never creates a review.
pub(super) fn select_result(m: &mut Model, index: usize) {
    let Some(e) = m.areas.storage.editor.as_ref() else {
        return;
    };
    let current = results_current(m, e);
    let Some(e) = m.areas.storage.editor.as_mut() else {
        return;
    };
    if !current {
        e.discovered = None;
        e.error = Some(
            "Storage inputs or credentials changed. Discover again before selecting a result."
                .into(),
        );
        return;
    }
    match &e.discovered {
        Some(StorageDiscoveryResult::LocalRepositories(rows)) => {
            if let Some(row) = rows.get(index) {
                // One reporting driver leaves nothing to choose. Two do: the EXT and XFS
                // probes look for the same prefix, and the operator says which it is.
                if let [kind] = row.probe_kinds[..] {
                    e.kind = local_kind_index(kind);
                }
                e.local_introduction = None;
                e.local_reported =
                    Some((row.repository.uuid.to_string(), row.probe_kinds.clone()));
                e.uuid = row.repository.uuid.to_string();
                e.mode = Mode::Introduce;
            }
        }
        Some(StorageDiscoveryResult::HbaDevices(rows)) => {
            if let (Some(row), Some(host)) = (rows.get(index), e.host) {
                e.scsi = row.scsi_id.clone();
                e.hba_device = Some((host, row.clone()));
                e.uuid = String::new();
            }
        }
        Some(StorageDiscoveryResult::NfsExports(rows)) => {
            if let Some(row) = rows.get(index) {
                e.path = row.path.clone();
                e.uuid = String::new();
            }
        }
        Some(StorageDiscoveryResult::IscsiTargets(rows)) => {
            if let Some(row) = rows.get(index) {
                e.server = row.portal.clone();
                e.port = row.port.to_string();
                e.iqn = row.iqn.clone();
                e.scsi = String::new();
                e.uuid = String::new();
            }
        }
        Some(StorageDiscoveryResult::IscsiLuns(rows)) => {
            if let Some(row) = rows.get(index) {
                e.scsi = row.scsi_id.clone();
                e.uuid = String::new();
            }
        }
        Some(StorageDiscoveryResult::Repositories(rows)) => {
            if let Some(row) = rows.get(index) {
                e.uuid = row.uuid.to_string();
                e.mode = Mode::Introduce;
            }
        }
        _ => return,
    }
    e.discovered = None;
    e.error = None;
    e.result_page = 0;
}

/// The backend choice. Changing it clears what depended on the old one.
pub(super) fn set_kind(m: &mut Model, kind: u8) {
    let Some(e) = &mut m.areas.storage.editor else {
        return;
    };
    if e.kind != kind {
        if matches!(kind, 5..=7) && !is_local(e) {
            e.path = String::new();
        }
        if !(matches!(kind, 5..=7) && is_local(e)) {
            // What a local probe reported belongs to the device path it was asked about.
            e.local_reported = None;
        }
        e.local_probe = None;
        e.local_acknowledged = false;
        e.local_introduction = None;
        e.kind = kind;
        e.hba_device = None;
        e.scsi = String::new();
        e.allocation = StorageAllocation::DriverDefault;
    }
}

/// The host choice. Changing it clears what was discovered on the old one.
pub(super) fn set_host(m: &mut Model, id: HostId) {
    let Some(e) = &mut m.areas.storage.editor else {
        return;
    };
    if e.host != Some(id) {
        e.host = Some(id);
        e.local_probe = None;
        e.local_reported = None;
        e.local_acknowledged = false;
        e.hba_device = None;
        if e.kind == 4 {
            e.scsi = String::new();
        }
    }
}

pub(super) fn credentials(e: &Editor) -> Option<StorageCredentials> {
    if e.username.is_empty() || e.password.is_empty() {
        return None;
    }
    if e.kind == 8 {
        Some(StorageCredentials::smb_disks(
            e.username.clone(),
            e.password.expose().to_owned(),
        ))
    } else if e.kind == 3 {
        Some(StorageCredentials::smb(
            e.username.clone(),
            e.password.expose().to_owned(),
        ))
    } else if e.kind == 2 && e.mutual {
        if e.incoming_username.is_empty() || e.incoming_password.is_empty() {
            return None;
        }
        Some(StorageCredentials::mutual_chap(
            e.username.clone(),
            e.password.expose().to_owned(),
            e.incoming_username.clone(),
            e.incoming_password.expose().to_owned(),
        ))
    } else if e.kind == 2 {
        Some(StorageCredentials::chap(
            e.username.clone(),
            e.password.expose().to_owned(),
        ))
    } else {
        None
    }
}

/// The operator's declaration for formatting a local device (from
/// `storage_local_view.rs`). `Ok(None)`: not a local creation.
pub(super) fn capture(e: &Editor) -> Result<Option<LocalDeviceDeclaration>, String> {
    if !is_local(e) {
        return Ok(None);
    }
    if e.mode == Mode::Introduce {
        if let Some(kinds) = reported_kinds(e)
            && !kinds.iter().any(|kind| local_kind_index(*kind) == e.kind)
        {
            return Err(format!(
                "The probe reported this repository through {} only. Choose that driver, or probe the device again.",
                kind_labels(kinds)
            ));
        }
        if !e
            .local_introduction
            .as_ref()
            .is_some_and(|(kind, uuid)| *kind == e.kind && uuid == e.uuid.trim())
        {
            return Err("Verify the existing repository's driver independently and confirm your selected driver before introduction. EXT and XFS probes cannot identify its filesystem.".into());
        }
        return Ok(None);
    }
    if e.mode != Mode::Create {
        return Ok(None);
    }
    if !e
        .local_probe
        .as_ref()
        .is_some_and(|(host, path, empty)| Some(*host) == e.host && path == e.path.trim() && *empty)
    {
        return Err("Probe the exact host and device for EXT, LVM and advertised XFS repositories before reviewing a format. Known repositories must be introduced instead.".into());
    }
    if !e.local_acknowledged
        || e.declaration.trim().is_empty()
        || e.declaration.len() > 512
        || e.declaration.chars().any(char::is_control)
    {
        return Err("Enter your physical-device identity statement and acknowledge the loss of existing data.".into());
    }
    Ok(Some(LocalDeviceDeclaration {
        identity_statement: e.declaration.trim().into(),
        acknowledge_data_loss: true,
    }))
}

/// Builds the operation from the form and stages its review. The order of
/// the checks is the Rune order.
pub(super) fn review(m: &mut Model) {
    let result = (|| {
        let editor = m
            .areas
            .storage
            .editor
            .as_ref()
            .ok_or("Storage view is closed")?;
        if editor.kind == 2
            && editor
                .port
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .is_none()
        {
            return Err("Enter an iSCSI port from 1 to 65535".into());
        }
        let hba_device = if editor.kind == 4 && editor.mode == Mode::Create {
            let (host, device) = editor
                .hba_device
                .as_ref()
                .ok_or("Discover and select an HBA device before review")?;
            if editor.host != Some(*host) || editor.scsi != device.scsi_id {
                return Err(
                    "The selected HBA device belongs to different inputs. Discover again.".into(),
                );
            }
            Some(device.clone())
        } else {
            None
        };
        if editor.mode == Mode::Create
            && editor.allocation != StorageAllocation::DriverDefault
            && !allocation_advertised(editor)
        {
            return Err("The selected driver has not advertised allocation choices. Refresh driver discovery or choose its default.".into());
        }
        let local_device = capture(editor)?;
        let operation = match editor.mode {
            Mode::Create => StorageOperation::Create {
                hba_device,
                local_device,
                host: editor.host.ok_or("Choose a host")?,
                name: editor.name.trim().into(),
                description: editor.description.clone(),
                location: location(editor),
            },
            Mode::Introduce => StorageOperation::Introduce {
                uuid: SrId::parse(editor.uuid.trim())
                    .map_err(|_| "Choose a valid discovered storage UUID")?,
                name: editor.name.trim().into(),
                description: editor.description.clone(),
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
        ) && editor.name.trim().is_empty()
        {
            return Err("Enter a storage name".into());
        }
        Ok::<_, String>((editor.target, operation))
    })();
    match result {
        Ok((target, operation)) => stage(m, target, operation),
        Err(error) => {
            if let Some(editor) = &mut m.areas.storage.editor {
                editor.error = Some(error);
            }
        }
    }
}

/// Captures the review: the observation of `target` now, the operation and,
/// for an authenticated creation, the credentials.
pub(super) fn stage(m: &mut Model, target: ResourceTarget, operation: StorageOperation) {
    let Some(pool) = m.areas.storage.editor.as_ref().map(|e| e.pool) else {
        return;
    };
    let snapshot = m.snapshot_for(pool);
    let observed = snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.observe_resource(target).ok());
    let synthetic = m.is_synthetic_pool(pool);
    if let Some(editor) = &mut m.areas.storage.editor {
        if !synthetic
            && (editor.observed.is_none()
                || observed.is_none()
                || !current(snapshot.as_deref(), editor))
        {
            editor.error = Some(STALE.into());
            return;
        }
        let auth = editor.authenticate
            && matches!(editor.kind, 2 | 3 | 8)
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
                if matches!(editor.kind, 3 | 8) {
                    "SMB 3.0"
                } else if editor.mutual {
                    "Mutual CHAP"
                } else {
                    "CHAP"
                },
                editor.username
            )
        });
        editor.review = Some((observed, target, operation));
        editor.error = None;
        editor.confirmation = String::new();
    }
}

pub(super) fn current(snapshot: Option<&ConnectionSnapshot>, editor: &Editor) -> bool {
    snapshot.is_some_and(|snapshot| {
        editor
            .observed
            .is_some_and(|observed| snapshot.observe_resource(editor.target).ok() == Some(observed))
    })
}

/// Why the open review cannot be confirmed any more: the connection it was
/// captured on changed. `None` in a synthetic pool, where nothing can be
/// submitted anyway, and while nothing is reviewed.
pub(super) fn stale(m: &Model, e: &Editor) -> Option<String> {
    (e.review.is_some()
        && !m.is_synthetic_pool(e.pool)
        && !current(m.snapshot_for(e.pool).as_deref(), e))
    .then(|| STALE.to_owned())
}

/// Back from a review to the form: the captured credentials go with it.
pub(super) fn back(m: &mut Model) {
    if let Some(e) = &mut m.areas.storage.editor {
        e.review = None;
        e.review_credentials = None;
        e.review_auth_label = None;
    }
}

/// The text the user has to type before a destructive review can be
/// confirmed. `None`: the operation needs no typed confirmation.
pub(super) fn confirmation_text(e: &Editor, operation: &StorageOperation) -> Option<String> {
    match operation {
        StorageOperation::Create {
            location: StorageLocation::Hba { scsi_id, .. },
            ..
        } => Some(scsi_id.clone()),
        StorageOperation::Create {
            location: StorageLocation::Local { device, .. },
            ..
        } => Some(device.clone()),
        StorageOperation::Forget { .. } | StorageOperation::Destroy { .. } => {
            Some(e.target.uuid().to_string())
        }
        _ => None,
    }
}

/// Forget and Destroy review at most 100 disks.
pub(super) fn too_many_disks(operation: &StorageOperation) -> bool {
    matches!(operation,StorageOperation::Forget{expected_disks,..}|StorageOperation::Destroy{expected_disks,..} if expected_disks.len()>100)
}

/// The permission assessment of whatever is reviewed now.
pub(super) fn permission(m: &Model, e: &Editor) -> Option<PermissionAssessment> {
    if let Some((pool, operation, secrets)) = &e.cleanup_review {
        let required = ResourceAction::CleanupStorageCredentials {
            operation: *operation,
            secrets: secrets.clone(),
        }
        .permission_requirements(pool.target());
        return Some(permissions::assess_for(m, e.pool, &required));
    }
    e.review.as_ref().map(|(_, _, operation)| {
        let requirements = if e.review_auth_label.is_some() {
            operation.authenticated_permission_requirements()
        } else {
            operation.permission_requirements()
        };
        permissions::assess_for(m, e.pool, &requirements)
    })
}

/// Opens the review of deleting the unused temporary Secrets an
/// authenticated storage operation left behind. The source operation must
/// be terminal.
pub(super) fn stage_cleanup(m: &mut Model, id: OperationId) {
    let Some(editor) = m.areas.storage.editor.as_ref() else {
        return;
    };
    let Some(record) = m.resource_rows.iter().find(|r| {
        r.id == id
            && r.pool == editor.pool
            && r.state.terminal()
            && matches!(r.action, ResourceAction::AuthenticatedStorage { .. })
    }) else {
        return;
    };
    let Some(observed) = m
        .snapshot_for(editor.pool)
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
    if let Some(editor) = m.areas.storage.editor.as_mut() {
        editor.cleanup_review = Some((observed, id, secrets));
        editor.error = None;
    }
}

pub(super) fn apply_cleanup(m: &mut Model) {
    let result = (|| {
        let e = m.areas.storage.editor.as_ref().ok_or(Error::Closed)?;
        let (pool, operation, secrets) = e
            .cleanup_review
            .as_ref()
            .ok_or(Error::InvalidResourceCommand)?;
        let required = ResourceAction::CleanupStorageCredentials {
            operation: *operation,
            secrets: secrets.clone(),
        }
        .permission_requirements(pool.target());
        if permissions::assess_for(m, e.pool, &required)
            .decision()
            .is_denied()
        {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        m.connection_for(e.pool)
            .ok_or(Error::Closed)?
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
            m.track_resource(operation);
            m.areas.storage.editor = None;
        }
        Err(e) => {
            if let Some(editor) = &mut m.areas.storage.editor {
                editor.error = Some(e.to_string());
            }
        }
    }
}

/// Submits the reviewed operation. Every check of the confirm button is
/// made again, in the Rune order; a synthetic pool is refused because its
/// review has no observation.
pub(super) fn apply(m: &mut Model) {
    let checked = (|| {
        let editor = m.areas.storage.editor.as_ref().ok_or(Error::Closed)?;
        let (observed, _, operation) = editor
            .review
            .as_ref()
            .ok_or(Error::InvalidResourceCommand)?;
        let observed = observed.ok_or(Error::Stale)?;
        if too_many_disks(operation) {
            return Err(Error::InvalidResourceCommand);
        }
        if !current(m.snapshot_for(editor.pool).as_deref(), editor) {
            return Err(Error::Stale);
        }
        if matches!(
            operation,
            StorageOperation::Destroy { .. } | StorageOperation::Forget { .. }
        ) && editor.confirmation != editor.target.uuid().to_string()
        {
            return Err(Error::InvalidResourceCommand);
        }
        if let StorageOperation::Create {
            location: StorageLocation::Hba { scsi_id, .. },
            ..
        } = operation
            && editor.confirmation != *scsi_id
        {
            return Err(Error::InvalidResourceCommand);
        }
        if let StorageOperation::Create {
            location: StorageLocation::Local { device, .. },
            ..
        } = operation
            && editor.confirmation != *device
        {
            return Err(Error::InvalidResourceCommand);
        }
        let requirements = if editor.review_auth_label.is_some() {
            operation.authenticated_permission_requirements()
        } else {
            operation.permission_requirements()
        };
        if permissions::assess_for(m, editor.pool, &requirements)
            .decision()
            .is_denied()
        {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        let connection = m.connection_for(editor.pool).ok_or(Error::Closed)?;
        Ok((observed, operation.clone(), connection))
    })();
    let result = checked.and_then(|(observed, operation, connection)| {
        let editor = m.areas.storage.editor.as_mut().ok_or(Error::Closed)?;
        let credentials = editor.review_credentials.take();
        editor.password = SecretText::new();
        editor.incoming_password = SecretText::new();
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
                operation,
            },
        )
    });
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            m.areas.storage.editor = None;
        }
        Err(error) => {
            if let Some(editor) = &mut m.areas.storage.editor {
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

/// Stages detaching or attaching (`remove == false`, whichever the
/// attachment's state leaves) or removing a host attachment, on the open
/// editor.
pub(super) fn attachment_action(m: &mut Model, pbd: PbdId, remove: bool) {
    let Some(pool) = m.areas.storage.editor.as_ref().map(|e| e.pool) else {
        return;
    };
    let Some(inventory) = m.inventory_for(pool) else {
        return;
    };
    let Some(attachment) = inventory.details.pbds.get(&pbd) else {
        return;
    };
    if let (Some(sr), Some(host), Some(attached)) = (
        attachment.sr,
        attachment.host,
        attachment.currently_attached,
    ) {
        stage(
            m,
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

/// Stages Forget or Destroy of the repository the editor is open on, with
/// its name and the exact set of its disks as they are now.
pub(super) fn removal(m: &mut Model, destroy: bool) {
    let Some(editor) = &m.areas.storage.editor else {
        return;
    };
    let ResourceTarget::Storage(sr) = editor.target else {
        return;
    };
    let Some(inventory) = m.inventory_for(editor.pool) else {
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
        m,
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

/// Turns the editor of a repository into the form of a new host attachment,
/// prefilled from an attachment the repository already has.
pub(super) fn new_attachment(m: &mut Model) {
    let Some(editor) = m.areas.storage.editor.as_ref() else {
        return;
    };
    let ResourceTarget::Storage(sr) = editor.target else {
        return;
    };
    let inventory = m.inventory_for(editor.pool);
    let template = inventory
        .as_ref()
        .and_then(|i| {
            i.details
                .pbds
                .values()
                .find(|p| p.sr == Some(sr) && p.location.is_some())
        })
        .and_then(|p| p.location.clone());
    let Some(editor) = m.areas.storage.editor.as_mut() else {
        return;
    };
    editor.mode = Mode::Attachment;
    editor.error = None;
    if let Some(template) = template {
        editor.authenticate = inventory.as_ref().is_some_and(|i| {
            i.details
                .pbds
                .values()
                .any(|p| p.sr == Some(sr) && p.requires_credentials)
        });
        editor.allocation = template.allocation();
        match template {
            StorageLocation::Smb { server, share } => {
                editor.kind = 8;
                editor.server = server;
                editor.share = share;
                editor.path = String::new();
            }
            StorageLocation::Local { kind, device } => {
                editor.kind = local_kind_index(kind);
                editor.path = device;
            }
            StorageLocation::Hba { scsi_id, .. } => {
                editor.kind = 4;
                editor.scsi = scsi_id;
            }
            StorageLocation::IsoSmb {
                server,
                share,
                path,
            } => {
                editor.kind = 3;
                editor.server = server;
                editor.share = share;
                editor.path = path;
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
                editor.kind = if inventory
                    .as_ref()
                    .and_then(|i| i.details.storage.get(&sr))
                    .is_some_and(|d| d.content_type == "iso")
                {
                    1
                } else {
                    0
                };
                editor.server = server;
                editor.path = path;
                editor.version = version;
            }
            StorageLocation::Iscsi {
                portal,
                port,
                target_iqn,
                scsi_id,
                ..
            } => {
                editor.kind = 2;
                editor.server = portal;
                editor.port = port.to_string();
                editor.iqn = target_iqn;
                editor.scsi = scsi_id;
            }
        }
    }
}

// -- What the buttons of the Rune hub did, as entry points -----------------------

/// Opens the review of forgetting (`destroy == false`) or destroying a
/// repository. `false`: nothing opened.
pub fn review_removal(m: &mut Model, pool: PoolId, sr: SrId, destroy: bool) -> bool {
    if !open(m, pool, Some(sr)) {
        return false;
    }
    removal(m, destroy);
    true
}

/// Opens the review of detaching or attaching a host attachment (whichever
/// its state leaves), or of removing it when it is detached.
pub fn review_attachment(m: &mut Model, pool: PoolId, pbd: PbdId, remove: bool) -> bool {
    let sr = m
        .inventory_for(pool)
        .and_then(|i| i.details.pbds.get(&pbd).and_then(|p| p.sr));
    let Some(sr) = sr else {
        return false;
    };
    if !open(m, pool, Some(sr)) {
        return false;
    }
    attachment_action(m, pbd, remove);
    true
}

/// Opens the form of a new host attachment of a repository.
pub fn open_attachment(m: &mut Model, pool: PoolId, sr: SrId) -> bool {
    if !open(m, pool, Some(sr)) {
        return false;
    }
    new_attachment(m);
    true
}

/// Opens the chooser of the host to reclaim unused blocks through. The
/// hosts are offered once the driver of the repository advertises
/// `SR_TRIM`, which the discovery started here reports.
pub fn open_trim(m: &mut Model, pool: PoolId, sr: SrId) -> bool {
    if !open(m, pool, Some(sr)) {
        return false;
    }
    if let Some(e) = &mut m.areas.storage.editor {
        e.trim = true;
    }
    true
}

/// Whether a backend of the repository's kind advertises `SR_TRIM`.
pub(super) fn trim_supported(m: &Model, e: &Editor) -> bool {
    let ResourceTarget::Storage(sr) = e.target else {
        return false;
    };
    let Some(inventory) = m.inventory_for(e.pool) else {
        return false;
    };
    let backend = inventory
        .storage
        .iter()
        .find(|r| r.uuid == sr.uuid())
        .map(|r| r.kind.as_str());
    e.backends
        .iter()
        .any(|b| Some(b.kind.as_str()) == backend && b.features.iter().any(|f| f == "SR_TRIM"))
}

/// Stages reclaiming unused blocks through `host`, on the open editor.
pub(super) fn review_trim(m: &mut Model, host: HostId) {
    let Some(e) = &m.areas.storage.editor else {
        return;
    };
    let ResourceTarget::Storage(sr) = e.target else {
        return;
    };
    let name = e.name.clone();
    stage(
        m,
        ResourceTarget::Storage(sr),
        StorageOperation::Trim {
            host,
            expected_name: name,
        },
    );
}

/// "Restore this connection…" of the storage workflow's history: verifies
/// that the attachment is still the detached one of this repository and
/// host, then opens the review that can only attach it. A refusal goes to
/// the shared error line.
pub fn review_restore(m: &mut Model, pool: PoolId, sr: SrId, pbd: PbdId, host: HostId) {
    if !m
        .inventory_for(pool)
        .as_ref()
        .and_then(|i| i.details.pbds.get(&pbd))
        .is_some_and(|p| {
            p.sr == Some(sr) && p.host == Some(host) && p.currently_attached == Some(false)
        })
    {
        m.error = Some("The detached attachment changed; close and reopen this dialog".into());
        return;
    }
    if !open(m, pool, Some(sr)) {
        return;
    }
    stage(
        m,
        ResourceTarget::Pbd(pbd),
        StorageOperation::SetAttached {
            sr,
            host,
            expected_attached: false,
            attached: true,
        },
    );
}

// -- Review texts --------------------------------------------------------------------

/// The lines that describe a storage location in a review. Also used by the
/// storage workflow and the repair review of `storage_maintenance`.
pub fn location_lines(location: &StorageLocation) -> Vec<String> {
    match location {
        StorageLocation::Smb { server, share } => vec![
            "Backend: SMB 3.0 virtual disks".into(),
            format!("Share root: //{server}/{share}"),
            "Shared VHD storage; driver enforces SMB 3.0".into(),
        ],
        StorageLocation::Local { kind, device } => vec![
            format!("Backend: {}", kind.label()),
            format!("Operator-selected device path: {device}"),
            "Local repository; not shared across hosts".into(),
        ],
        StorageLocation::Hba {
            scsi_id,
            allocation,
        } => vec![
            "Backend: LVM over HBA / Fibre Channel".into(),
            format!("LUN SCSI ID: {scsi_id}"),
            format!("Allocation: {}", allocation.label()),
        ],
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
            allocation,
        } => vec![
            "Backend: LVM over iSCSI".into(),
            format!("Portal: {portal} · port {port}"),
            format!("Target IQN: {target_iqn}"),
            format!("LUN SCSI ID: {scsi_id}"),
            format!("Allocation: {}", allocation.label()),
        ],
    }
}

pub(super) fn describe(operation: &StorageOperation) -> Vec<String> {
    let mut lines = vec![operation.label().into()];
    match operation {
        StorageOperation::Trim { host, .. } => lines.extend([
            format!("Attached host: {host}"),
            "Asks the host storage driver to reclaim unused blocks. Completion depends on the backend and storage array; reported capacity may not change immediately.".into(),
        ]),
        StorageOperation::Create {
            host,
            name,
            description,
            location,
            hba_device,
            local_device,
        } => {
            lines.extend([
                format!("Name: {name}"),
                format!("Description: {description}"),
                format!("Host: {host}"),
            ]);
            lines.extend(location_lines(location));
            lines.push(if location.shared() {
                "Creates a shared repository. The server may create pool attachments. Review attached hosts after completion."
            } else {
                "Creates a local repository on this host only. PERMANENTLY ERASES existing data on the selected device."
            }.into());
            if let Some(declaration) = local_device {
                lines.extend([
                    format!("Operator identity declaration (not verified by the server): {}", declaration.identity_statement),
                    "The API does not fully verify device identity, capacity, partitions, signatures or use. Empty probes and unmatched aliases do not prove the disk is safe to format. The host driver may refuse the operation; partial formatting may remain after failure.".into(),
                    "Known EXT/LVM/XFS repositories and known current attachments are refused. No existing repository is automatically forgotten or destroyed.".into(),
                ]);
            }
            if matches!(
                location,
                StorageLocation::Iscsi { .. } | StorageLocation::Hba { .. }
            ) {
                lines.push("Creating LVM storage initializes the selected LUN and can permanently erase existing data. Confirm that no other pool or system uses it. Discovery cannot establish external ownership.".into());
            }
            if let Some(device) = hba_device {
                lines.extend([
                    format!("Reviewed device: {} · {} · serial {}", bytes(device.size_bytes), device.vendor, device.serial),
                    "The host will rediscover this exact device and refuse known existing repositories. No existing storage is automatically forgotten or destroyed.".into(),
                ]);
            }
            if location.allocation() == StorageAllocation::Thin {
                lines.push("Thin allocation grows VHD disks as they are written; raw disks remain fully allocated. Monitor free space: exhausting the repository can interrupt VMs. This does not convert existing repositories.".into());
            }
        }
        StorageOperation::Introduce {
            uuid,
            name,
            description,
            location,
        } => {
            lines.extend([
                format!("Existing UUID: {uuid}"),
                format!("Name: {name}"),
                format!("Description: {description}"),
            ]);
            lines.extend(location_lines(location));
            lines.push("Registers existing data without formatting it. Create and plug host attachments separately after this completes.".into());
            if matches!(location, StorageLocation::Local { .. }) {
                lines.push("The selected driver is your independently verified choice. EXT and XFS probes share a volume-group prefix and cannot establish the filesystem type.".into());
            }
        }
        StorageOperation::CreateAttachment { host, location } => {
            lines.push(format!("Host: {host}"));
            lines.extend(location_lines(location));
            lines.push("Creates a detached attachment. Use Attach / repair afterward to connect this host.".into());
        }
        StorageOperation::SetAttached {
            sr, host, attached, ..
        } => lines.extend([
            format!("Repository: {sr}"),
            format!("Host: {host}"),
            if *attached {
                "Authenticates/mounts the existing attachment. Attached status is not a storage health test.".into()
            } else {
                "Disconnects this host from the repository. Active disk consumers must be stopped first.".into()
            },
        ]),
        StorageOperation::RemoveAttachment { sr, host } => lines.extend([
            format!("Repository: {sr}"),
            format!("Host: {host}"),
            "Removes the detached host attachment record, preserving storage data.".into(),
        ]),
        StorageOperation::Forget { expected_disks, .. } => lines.extend([
            format!("Reviewed disks: {}", expected_disks.len()),
            "Removes repository and disk records from the pool database. Storage data remains, but dependent VM disk references must first be removed. All host attachments must be detached.".into(),
        ]),
        StorageOperation::Destroy { expected_disks, .. } => lines.extend([
            format!("Reviewed disks: {}", expected_disks.len()),
            "PERMANENTLY DESTROYS THE REPOSITORY AND ITS DATA. All host attachments must be detached and all disk consumers removed. This action cannot be undone.".into(),
        ]),
    }
    lines
}
