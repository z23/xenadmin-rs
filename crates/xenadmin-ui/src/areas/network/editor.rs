//! The generic editor of the Rune `resource_admin.rs`, carried over: one
//! small reviewed change of a pool, a server, a storage repository or a
//! network.
//!
//! | Rune `Kind` | Here |
//! | --- | --- |
//! | `Rename` | [`Kind::Rename`]: the General page of the object's Properties dialog |
//! | `CreateNetwork` | [`Kind::CreateNetwork`]: the wizard New Network |
//! | `DeleteNetwork` | [`Kind::DeleteNetwork`]: `network.remove` |
//! | `ScanStorage` | [`Kind::ScanStorage`]: `storage.rescan` |
//! | `HostEnabled` | [`Kind::HostEnabled`]: `server.scheduling` |
//! | `DefaultStorage` | [`Kind::DefaultStorage`]: `storage.set-default` |
//! | `Mtu` | the MTU field of the Network Settings page (`workspace::Form::Mtu`) |
//! | `History`, `Acknowledge` | the `events` area |
//!
//! Validation, review lines, the staleness rule, the permission review and
//! the submission are the Rune ones. The editor follows the pool of its
//! target, not the scope session.

use xenadmin_core::{
    Error, Inventory, ObservedResource, OperationId, PermissionAssessment, PoolId,
    ResourceAction, ResourceCommand, ResourceTarget, SrId,
};

use crate::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Rename,
    CreateNetwork,
    DeleteNetwork,
    ScanStorage,
    HostEnabled(bool),
    DefaultStorage,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Rename => "Edit name and description",
            Self::CreateNetwork => "Create internal network",
            Self::DeleteNetwork => "Delete internal network",
            Self::ScanStorage => "Rescan storage repository",
            Self::HostEnabled(true) => "Enable host scheduling",
            Self::HostEnabled(false) => "Disable host scheduling",
            Self::DefaultStorage => "Choose default storage",
        }
    }

    pub fn danger(self) -> bool {
        matches!(self, Self::DeleteNetwork)
    }

    /// A change with nothing to enter: its command opens the review.
    pub fn opens_on_review(self) -> bool {
        !matches!(self, Self::Rename | Self::CreateNetwork)
    }
}

pub struct Review {
    pub name: String,
    pub description: String,
    pub mtu: u64,
    pub sr: Option<SrId>,
    pub lines: Vec<String>,
}

pub struct Editor {
    pub kind: Kind,
    pub target: ResourceTarget,
    pub observed: Option<ObservedResource>,
    pub pool: PoolId,
    pub label: String,
    pub expected_name: String,
    pub expected_description: String,
    pub expected_mtu: Option<u64>,
    pub expected_enabled: Option<bool>,
    pub expected_default: Option<SrId>,
    pub name: String,
    pub description: String,
    pub mtu: String,
    pub selected_sr: Option<SrId>,
    pub review: Option<Review>,
    pub error: Option<String>,
    /// The step of the New Network wizard that is showing (0 Type, 1 Name,
    /// 2 Settings); the review is its last step.
    pub step: usize,
}

/// The reviewed name and description: the object's own labels as the host holds them. A
/// pool's label is empty on a fresh XCP-ng installation while the inventory shows a
/// placeholder, so the review must expect the empty label or the host refuses the edit.
pub fn metadata(inventory: &Inventory, target: ResourceTarget) -> Option<(String, String)> {
    match target {
        ResourceTarget::Pool(id) if id == inventory.pool_id => {
            let pool = inventory.details.pool.as_ref()?;
            Some((pool.name.clone(), pool.description.clone()))
        }
        ResourceTarget::Host(id) => Some((
            inventory.hosts.get(&id)?.name.clone(),
            inventory.details.hosts.get(&id)?.description.clone(),
        )),
        ResourceTarget::Storage(id) => Some((
            inventory
                .storage
                .iter()
                .find(|s| s.uuid == id.uuid())?
                .name
                .clone(),
            inventory.details.storage.get(&id)?.description.clone(),
        )),
        ResourceTarget::Network(id) => Some((
            inventory
                .networks
                .iter()
                .find(|n| n.uuid == id.uuid())?
                .name
                .clone(),
            inventory.details.networks.get(&id)?.description.clone(),
        )),
        _ => None,
    }
}

pub fn target_label(target: ResourceTarget) -> String {
    match target {
        ResourceTarget::Disk(id) => format!("Disk {id}"),
        ResourceTarget::Vm(id) => format!("VM {id}"),
        ResourceTarget::SnapshotPolicy(id) => format!("Snapshot schedule {id}"),
        ResourceTarget::Pif(id) => format!("Interface {id}"),
        ResourceTarget::Vlan(id) => format!("VLAN {id}"),
        ResourceTarget::Bond(id) => format!("Bond {id}"),
        ResourceTarget::Tunnel(id) => format!("Tunnel {id}"),
        ResourceTarget::Sriov(id) => format!("SR-IOV network {id}"),
        ResourceTarget::Pool(id) => format!("Pool {id}"),
        ResourceTarget::Host(id) => format!("Host {id}"),
        ResourceTarget::Storage(id) => format!("Storage {id}"),
        ResourceTarget::Network(id) => format!("Network {id}"),
        ResourceTarget::Task(id) => format!("Task {id}"),
        ResourceTarget::Message(id) => format!("Message {id}"),
        ResourceTarget::Pbd(id) => format!("Storage attachment {id}"),
        ResourceTarget::Appliance(id) => format!("vApp {id}"),
    }
}

pub fn display_name(inventory: &Inventory, target: ResourceTarget) -> Option<String> {
    match target {
        ResourceTarget::Pool(id) if id == inventory.pool_id => Some(inventory.pool_name.clone()),
        ResourceTarget::Host(id) => inventory.hosts.get(&id).map(|host| host.name.clone()),
        ResourceTarget::Storage(id) => inventory
            .storage
            .iter()
            .find(|sr| sr.uuid == id.uuid())
            .map(|sr| sr.name.clone()),
        ResourceTarget::Network(id) => inventory
            .networks
            .iter()
            .find(|network| network.uuid == id.uuid())
            .map(|network| network.name.clone()),
        _ => None,
    }
}

/// The editor of a change for a target of `pool`, or the Rune reason there
/// is none. Nothing of the model changes.
pub fn capture(
    m: &Model,
    pool: PoolId,
    target: ResourceTarget,
    kind: Kind,
) -> Result<Editor, String> {
    let inventory = m
        .inventory_for(pool)
        .ok_or("Current inventory is unavailable")?;
    let inventory = &*inventory;
    let observed = if m.is_synthetic_pool(pool) {
        None
    } else {
        Some(
            m.snapshot_for(pool)
                .ok_or("The connection is not current")?
                .observe_resource(target)
                .map_err(|e| e.to_string())?,
        )
    };
    let (name, description) = metadata(inventory, target).unwrap_or_else(|| {
        (
            display_name(inventory, target).unwrap_or_else(|| target_label(target)),
            String::new(),
        )
    });
    if kind == Kind::Rename && metadata(inventory, target).is_none() {
        return Err(
            "The host did not supply the current name and description. Refresh before editing."
                .into(),
        );
    }
    let expected_mtu = match target {
        ResourceTarget::Network(id) => inventory.details.networks.get(&id).and_then(|n| n.mtu),
        _ => None,
    };
    let expected_enabled = match target {
        ResourceTarget::Host(id) => inventory.hosts.get(&id).map(|h| h.enabled),
        _ => None,
    };
    let expected_default = inventory.details.pool.as_ref().and_then(|p| p.default_sr);
    if kind == Kind::DefaultStorage
        && !inventory
            .details
            .pool
            .as_ref()
            .is_some_and(|p| p.default_sr_known)
    {
        return Err(
            "The current default storage repository is unknown. Refresh before changing it."
                .into(),
        );
    }
    Ok(Editor {
        kind,
        target,
        observed,
        pool: inventory.pool_id,
        label: display_name(inventory, target).unwrap_or_else(|| name.clone()),
        expected_name: name.clone(),
        expected_description: description.clone(),
        expected_mtu,
        expected_enabled,
        expected_default,
        name: if kind == Kind::CreateNetwork {
            String::new()
        } else {
            name
        },
        description: if kind == Kind::CreateNetwork {
            String::new()
        } else {
            description
        },
        mtu: expected_mtu.unwrap_or(1500).to_string(),
        selected_sr: expected_default,
        review: None,
        error: None,
        step: 0,
    })
}

/// Opens the dialog of a change. The reason it cannot open goes into the
/// open editor when there is one, else to the shared error line (Rune).
pub fn open(m: &mut Model, pool: PoolId, target: ResourceTarget, kind: Kind) -> bool {
    match capture(m, pool, target, kind) {
        Ok(editor) => {
            super::close(m);
            m.areas.network.editor = Some(editor);
            m.error = None;
            true
        }
        Err(error) => {
            if let Some(editor) = &mut m.areas.network.editor {
                editor.error = Some(error);
            } else {
                m.error = Some(error);
            }
            false
        }
    }
}

fn valid_name(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 255 && !value.chars().any(char::is_control)
}

/// What the review of the editor would be, or why there is none.
pub fn reviewed(m: &Model, editor: &Editor) -> Result<Review, String> {
    let mut review = Review {
        name: editor.name.trim().into(),
        description: editor.description.clone(),
        mtu: 1500,
        sr: editor.selected_sr,
        lines: Vec::new(),
    };
    match editor.kind {
        Kind::Rename | Kind::CreateNetwork => {
            if !valid_name(&review.name)
                || review.description.chars().count() > 4096
                || review
                    .description
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err("Use a name of 1–255 characters and a description of at most 4096 characters.".into());
            }
            if editor.kind == Kind::Rename
                && review.name == editor.expected_name
                && review.description == editor.expected_description
            {
                return Err("No name or description changes to apply.".into());
            }
            review.lines.push(format!("Name: {}", review.name));
            review.lines.push(format!(
                "Description: {}",
                if review.description.is_empty() {
                    "(empty)"
                } else {
                    &review.description
                }
            ));
        }
        Kind::DefaultStorage => {
            let id = review.sr.ok_or("Choose a storage repository")?;
            if Some(id) == editor.expected_default {
                return Err("This repository is already the default.".into());
            }
            let inventory = m.inventory_for(editor.pool);
            let sr = inventory
                .as_ref()
                .and_then(|i| i.storage.iter().find(|sr| sr.uuid == id.uuid()))
                .ok_or("The selected storage repository is no longer available")?;
            review.lines.push(format!("New default: {} ({id})", sr.name));
        }
        Kind::HostEnabled(enabled) => {
            if editor.expected_enabled.is_none() || editor.expected_enabled == Some(enabled) {
                return Err("The host's scheduling state changed or is unavailable.".into());
            }
            review.lines.push(format!(
                "Allow new VM placement on this host: {}",
                if enabled { "Yes" } else { "No" }
            ));
            review.lines.push(
                "This controls scheduling. Running guests are not evacuated or shut down.".into(),
            );
        }
        Kind::DeleteNetwork => review.lines.push("The internal network record will be deleted. The host must report no physical or VM interfaces on it.".into()),
        Kind::ScanStorage => review.lines.push("Ask the storage backend to refresh its disk inventory. The operation will appear in history.".into()),
    }
    if editor.kind == Kind::CreateNetwork {
        review.mtu = editor
            .mtu
            .parse::<u64>()
            .ok()
            .filter(|n| (576..=65535).contains(n))
            .ok_or("Enter an MTU between 576 and 65535")?;
        review.lines.push(format!("MTU: {}", review.mtu));
    }
    Ok(review)
}

/// "Review": stores the review or the reason there is none.
pub fn review(m: &mut Model) {
    let Some(editor) = &m.areas.network.editor else {
        return;
    };
    let result = reviewed(m, editor);
    let Some(editor) = &mut m.areas.network.editor else {
        return;
    };
    match result {
        Ok(review) => {
            editor.review = Some(review);
            editor.error = None;
        }
        Err(error) => editor.error = Some(error),
    }
}

pub fn back(m: &mut Model) {
    if let Some(editor) = &mut m.areas.network.editor {
        editor.review = None;
        editor.error = None;
    }
}

pub fn synthetic(m: &Model, editor: &Editor) -> bool {
    m.is_synthetic_pool(editor.pool)
}

pub fn current(m: &Model, editor: &Editor) -> bool {
    m.can_mutate_pool(editor.pool)
        && editor.observed.is_some()
        && editor.observed
            == m.snapshot_for(editor.pool)
                .and_then(|snapshot| snapshot.observe_resource(editor.target).ok())
}

/// The Rune caption under a form or a review that is not current any more.
/// `None` in a synthetic pool, which has its own line.
pub fn stale(m: &Model, editor: &Editor) -> Option<&'static str> {
    (!synthetic(m, editor) && !current(m, editor)).then_some(super::workspace::STALE)
}

pub fn permission_review(m: &Model, editor: &Editor) -> Option<PermissionAssessment> {
    use ResourceAction as Action;
    let review = editor.review.as_ref()?;
    let action = match editor.kind {
        Kind::Rename => Action::RenameMetadata {
            expected_name: editor.expected_name.clone(),
            expected_description: editor.expected_description.clone(),
            name: review.name.clone(),
            description: review.description.clone(),
        },
        Kind::CreateNetwork => Action::CreateInternalNetwork {
            name: review.name.clone(),
            description: review.description.clone(),
            mtu: review.mtu,
        },
        Kind::DeleteNetwork => Action::DeleteInternalNetwork {
            expected_name: editor.expected_name.clone(),
        },
        Kind::ScanStorage => Action::ScanStorage,
        Kind::HostEnabled(enabled) => Action::SetHostEnabled {
            expected_enabled: editor.expected_enabled?,
            enabled,
        },
        Kind::DefaultStorage => Action::SetDefaultStorage {
            sr: review.sr?,
            expected_default: editor.expected_default,
        },
    };
    Some(permissions::assess_for(
        m,
        editor.pool,
        &action.permission_requirements(editor.target),
    ))
}

/// "Confirm". On success the surface closes and the operation is followed
/// in the Events tab; a refusal stays in the editor.
pub fn submit(m: &mut Model) {
    let result = (|| {
        let editor = m.areas.network.editor.as_ref().ok_or(Error::Stale)?;
        if !current(m, editor) {
            return Err(Error::Stale);
        }
        if permission_review(m, editor).is_some_and(|p| p.decision().is_denied()) {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        let target = editor.observed.ok_or(Error::Stale)?;
        let review = editor.review.as_ref().ok_or(Error::Stale)?;
        let connection = m.connection_for(editor.pool).ok_or(Error::Closed)?;
        let command = match editor.kind {
            Kind::Rename => ResourceCommand::RenameMetadata {
                target,
                expected_name: editor.expected_name.clone(),
                expected_description: editor.expected_description.clone(),
                name: review.name.clone(),
                description: review.description.clone(),
            },
            Kind::CreateNetwork => ResourceCommand::CreateInternalNetwork {
                pool: target,
                name: review.name.clone(),
                description: review.description.clone(),
                mtu: review.mtu,
            },
            Kind::DeleteNetwork => ResourceCommand::DeleteInternalNetwork {
                network: target,
                expected_name: editor.expected_name.clone(),
            },
            Kind::ScanStorage => ResourceCommand::ScanStorage { sr: target },
            Kind::HostEnabled(enabled) => ResourceCommand::SetHostEnabled {
                host: target,
                expected_enabled: editor.expected_enabled.ok_or(Error::Stale)?,
                enabled,
            },
            Kind::DefaultStorage => ResourceCommand::SetDefaultStorage {
                pool: target,
                sr: review.sr.ok_or(Error::Stale)?,
                expected_default: editor.expected_default,
            },
        };
        connection.submit_resource(OperationId::new(), command)
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            super::close(m);
            m.error = None;
        }
        Err(error) => {
            if let Some(editor) = &mut m.areas.network.editor {
                editor.error = Some(error.to_string());
            }
        }
    }
}
