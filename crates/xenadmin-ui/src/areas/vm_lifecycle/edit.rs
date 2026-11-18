//! The lifecycle editor: the logic of the Rune `vm_lifecycle.rs`.
//!
//! One editor for four changes of one VM or template (create a VM from a
//! template, copy, convert to a template, delete). What each change is, how
//! its [`VmLifecycleOperation`] is built from the draft, the review, the
//! staleness check and the submission are carried over word for word. The
//! pool is the pool of the source (the command's target) instead of the
//! scope session's, and the command goes through
//! `Model::submit_tracked_command_for`.
//!
//! The fifth Rune kind, "Edit template name and description…", is not
//! here: name and description of a custom template are the General page of
//! its Properties dialog (area `vm_properties`).

use std::collections::BTreeSet;
use std::sync::Arc;

use xenadmin_core::{
    GuestFirmware, HostId, Inventory, NetworkId, ObservedVm, PoolId, PowerState, SnapshotId, SrId,
    VdiId, Vm, VmCommand, VmCopyMode, VmId, VmLifecycleOperation,
};

use super::{Panel, cloud, creation, deletion};
use crate::prelude::*;

/// The caption Rune put under the lifecycle buttons of the details pane
/// when they were disabled; the reason of a disabled command and of a
/// forward button without a writable connection.
pub const REQUIRES: &str =
    "These workflows require a halted VM or template and a current writable connection.";
/// What the surface says while the source is not what was captured.
pub const STALE: &str = "The connection or source changed. Cancel and reopen this workflow.";
pub const PREVIEW: &str = "Synthetic review • Apply is disabled; no host changes can be submitted.";
/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
/// Why New VM is disabled while no template is the target. Rune had no
/// such state: its button was on the selected template only.
pub const NO_TEMPLATE: &str = "Select a template: a new VM is created from the selected template.";
pub const NO_CHOICES: &str = "No eligible choices are reported in the current inventory.";
pub const NO_SR: &str = "Select a destination storage repository.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Create,
    Clone,
    Convert,
    Delete,
}

pub struct Review {
    pub operation: VmLifecycleOperation,
    pub lines: Vec<String>,
}

pub struct Editor {
    /// The source as observed when the editor opened: the token the command
    /// is submitted with. `None` only in a synthetic pool.
    pub target: Option<ObservedVm>,
    pub pool: PoolId,
    pub source: Vm,
    pub kind: Kind,
    /// Create only.
    pub step: creation::Step,
    pub name: String,
    pub description: String,
    pub vcpus: String,
    pub memory: String,
    pub disk_size: String,
    pub sr: Option<SrId>,
    pub network: Option<NetworkId>,
    /// `None` keeps the inherited media.
    pub iso: Option<VdiId>,
    pub firmware: GuestFirmware,
    /// `None` is the generic BIOS identity.
    pub bios_host: Option<HostId>,
    pub cloud: bool,
    pub cloud_input: cloud::Draft,
    pub full_copy: bool,
    pub as_template: bool,
    pub disks: BTreeSet<VdiId>,
    pub snapshots: BTreeSet<SnapshotId>,
    pub snapshot_disks: BTreeSet<VdiId>,
    /// The inventory at open: every choice list and review label reads it.
    pub initial: Arc<Inventory>,
    pub review: Option<Review>,
    pub error: Option<String>,
}

pub(crate) fn editor(m: &Model) -> Option<&Editor> {
    match m.areas.vm_lifecycle.panel.as_ref()? {
        Panel::Editor(editor) => Some(editor),
        Panel::CloudCreation(_) => None,
    }
}

pub(crate) fn editor_mut(m: &mut Model) -> Option<&mut Editor> {
    match m.areas.vm_lifecycle.panel.as_mut()? {
        Panel::Editor(editor) => Some(editor),
        Panel::CloudCreation(_) => None,
    }
}

/// Closes the editor or the continuation of a cloud creation, whichever is
/// open. A draft of cloud-init inputs is zeroized as it goes.
pub(crate) fn close(m: &mut Model) {
    m.areas.vm_lifecycle.panel = None;
}

/// The source is still in its pool's inventory, still a template or still
/// a VM as captured and, outside a synthetic pool, still what was observed.
/// A selection change does not retarget the editor.
pub(crate) fn current(m: &Model, form: &Editor) -> bool {
    m.inventory_for(form.pool).is_some_and(|inventory| {
        inventory.pool_id == form.pool
            && inventory
                .vms
                .get(&form.source.id)
                .is_some_and(|vm| vm.is_template == form.source.is_template)
    }) && (m.is_synthetic_pool(form.pool)
        || form.target.is_some_and(|target| {
            m.snapshot_for(form.pool)
                .and_then(|snapshot| snapshot.observe_vm(form.source.id).ok())
                == Some(target)
        }))
}

/// The host offers `operation` for the VM now: nothing runs on it, the
/// operation is not blocked and it is among the allowed ones.
fn advertised(inventory: &Inventory, id: VmId, operation: &str) -> bool {
    inventory.details.vms.get(&id).is_some_and(|details| {
        details.current_operations.is_empty()
            && !details.blocked_operations.contains_key(operation)
            && details
                .allowed_operations
                .as_ref()
                .is_some_and(|allowed| allowed.iter().any(|allowed| allowed == operation))
    })
}

/// The simplest operation of a kind, to ask the core why the kind is not
/// offered. Creation has no operation without a draft: its probe is the
/// fast clone it starts with.
fn probe(source: &Vm, kind: Kind) -> VmLifecycleOperation {
    match kind {
        Kind::Create | Kind::Clone => VmLifecycleOperation::Clone {
            name: source.name.clone(),
            expected_template: source.is_template,
            mode: VmCopyMode::Fast,
            as_template: source.is_template,
        },
        Kind::Convert => VmLifecycleOperation::ConvertToTemplate,
        Kind::Delete => VmLifecycleOperation::Delete {
            expected_template: source.is_template,
            disks: Vec::new(),
            snapshots: Vec::new(),
        },
    }
}

/// The editor an object would open: the Rune `open` without its side
/// effects, behind the rule of the Rune button. `Err` is what the command
/// is instead: hidden for an object the change does not apply to (where
/// Rune showed no button), disabled where Rune disabled the button or
/// `open` returned silently. The reason is the caption Rune showed for a
/// source that is not halted or not writable, else what the core says
/// against the simplest operation of the kind.
pub(crate) fn capture(m: &Model, object: &ObjectRef, kind: Kind) -> Result<Editor, Eligibility> {
    if !matches!(object.kind, ObjectKind::Vm | ObjectKind::Template) {
        return Err(Eligibility::Hidden);
    }
    let (Some(pool), Some(id)) = (object.pool, object.vm_id()) else {
        return Err(Eligibility::Hidden);
    };
    let Some(inventory) = m.inventory_for(pool) else {
        return Err(Eligibility::Hidden);
    };
    let Some(source) = inventory.vms.get(&id).filter(|vm| !vm.is_control_domain) else {
        return Err(Eligibility::Hidden);
    };
    match kind {
        Kind::Create if !source.is_template => return Err(Eligibility::Hidden),
        Kind::Convert if source.is_template => return Err(Eligibility::Hidden),
        _ => {}
    }
    let synthetic = m.is_synthetic_pool(pool);
    let writable = synthetic || m.can_mutate_pool(pool);
    if !writable || source.power_state != PowerState::Halted {
        return Err(Eligibility::disabled(REQUIRES));
    }
    let target = m
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(id).ok());
    if !synthetic && target.is_none() {
        return Err(Eligibility::disabled(REQUIRES));
    }
    let custom = inventory
        .details
        .vms
        .get(&id)
        .is_some_and(|details| details.is_default_template == Some(false));
    let offered = match kind {
        Kind::Create | Kind::Clone => {
            advertised(&inventory, id, "clone") || advertised(&inventory, id, "copy")
        }
        Kind::Convert => advertised(&inventory, id, "make_into_template"),
        Kind::Delete => (!source.is_template || custom) && advertised(&inventory, id, "destroy"),
    };
    if !offered {
        let status = probe(source, kind).availability(&inventory, id);
        return Err(Eligibility::disabled(if status.is_available() {
            REQUIRES.to_owned()
        } else {
            status.explanation()
        }));
    }
    Ok(Editor {
        target,
        pool,
        source: source.clone(),
        kind,
        step: creation::Step::Name,
        name: match kind {
            Kind::Create => format!("{} VM", source.name),
            Kind::Clone => format!("{} copy", source.name),
            _ => source.name.clone(),
        },
        description: source.description.clone(),
        vcpus: source.vcpus.max(1).to_string(),
        memory: if source.memory_bytes > 0 && source.memory_bytes % (1 << 20) == 0 {
            format!("{} MiB", source.memory_bytes / (1 << 20))
        } else {
            source.memory_bytes.to_string()
        },
        disk_size: String::new(),
        sr: None,
        network: None,
        iso: None,
        firmware: GuestFirmware::Inherited,
        bios_host: None,
        cloud: false,
        cloud_input: cloud::Draft::default(),
        full_copy: false,
        as_template: source.is_template,
        disks: BTreeSet::new(),
        snapshots: BTreeSet::new(),
        snapshot_disks: BTreeSet::new(),
        initial: inventory.clone(),
        review: None,
        error: None,
    })
}

/// Opens the editor of a kind for an object. Nothing opens for an object
/// the command is hidden or disabled for. A conversion has nothing to
/// enter and opens on its review.
pub(crate) fn open(m: &mut Model, object: &ObjectRef, kind: Kind) {
    let Ok(form) = capture(m, object, kind) else {
        return;
    };
    super::close(m);
    m.areas.vm_lifecycle.panel = Some(Panel::Editor(Box::new(form)));
    m.error = None;
    if kind == Kind::Convert {
        review(m);
    }
}

pub(crate) fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 255 || name.chars().any(char::is_control) {
        Err("Enter a name of 1–255 characters without control characters.".into())
    } else {
        Ok(trimmed.into())
    }
}

pub(crate) fn validate_description(description: &str) -> Result<String, String> {
    if description.len() > 4096
        || description
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err("Description must be at most 4096 bytes without control characters.".into())
    } else {
        Ok(description.into())
    }
}

/// The operation the draft describes, or what is wrong with the draft.
pub(crate) fn operation(form: &Editor) -> Result<VmLifecycleOperation, String> {
    Ok(match form.kind {
        Kind::Create => creation::operation(form)?,
        Kind::Clone => VmLifecycleOperation::Clone {
            name: validate_name(&form.name)?,
            expected_template: form.source.is_template,
            mode: if form.full_copy {
                VmCopyMode::Full {
                    sr: form.sr.ok_or(NO_SR)?,
                }
            } else {
                VmCopyMode::Fast
            },
            as_template: form.as_template,
        },
        Kind::Convert => VmLifecycleOperation::ConvertToTemplate,
        Kind::Delete => deletion::operation(form),
    })
}

/// Asks for the review: the source must be current, the draft valid and
/// the operation available in the inventory as it is now, in this order.
/// The operation stored with the review is what [`apply`] sends; it is not
/// built again from the fields.
pub(crate) fn review(m: &mut Model) {
    let Some(form) = editor(m) else { return };
    let result = (|| {
        if !current(m, form) {
            return Err(STALE.to_owned());
        }
        let operation = operation(form)?;
        let inventory = m.inventory_for(form.pool).ok_or("No current inventory.")?;
        let availability = operation.availability(&inventory, form.source.id);
        if !availability.is_available() {
            return Err(availability.explanation());
        }
        let lines = match form.kind {
            Kind::Create => creation::review(form),
            Kind::Delete => deletion::review(form),
            Kind::Clone => vec![
                format!(
                    "Create {} “{}” using {}.",
                    if form.as_template { "template" } else { "VM" },
                    form.name.trim(),
                    if form.full_copy {
                        "a full disk copy"
                    } else {
                        "a fast clone on the existing storage"
                    }
                ),
                form.sr
                    .filter(|_| form.full_copy)
                    .map(|id| format!("Destination storage: {}", storage_label(&form.initial, id)))
                    .unwrap_or_else(|| {
                        "Existing storage must support fast cloning; the host verifies this capability."
                            .into()
                    }),
                "The result stays halted. A failed later step retains the created object and its UUID on the Events tab of the Notifications view.".into(),
                "Other changes in this pool are blocked while copying is active or its outcome is unknown. Resolve or explicitly acknowledge an unknown outcome to release this block.".into(),
            ],
            Kind::Convert => vec![
                "Convert this halted VM into a reusable template. It will no longer appear as a runnable VM. Its disks remain attached to the template.".into(),
            ],
        };
        Ok(Review { operation, lines })
    })();
    if let Some(form) = editor_mut(m) {
        match result {
            Ok(review) => {
                form.review = Some(review);
                form.error = None;
            }
            Err(error) => form.error = Some(error),
        }
    }
}

/// Back from the review to the draft; the fields are kept.
pub(crate) fn back(m: &mut Model) {
    if let Some(form) = editor_mut(m) {
        form.review = None;
    }
}

/// What the core says against the reviewed operation in the inventory as
/// it is now; `None` when it is available (or there is no inventory, which
/// [`current`] reports).
pub(crate) fn unavailable(m: &Model, form: &Editor) -> Option<String> {
    let review = form.review.as_ref()?;
    let inventory = m.inventory_for(form.pool)?;
    let status = review.operation.availability(&inventory, form.source.id);
    (!status.is_available()).then(|| {
        format!(
            "This change is no longer available: {}",
            status.explanation()
        )
    })
}

/// Why "Next" and "Review change" are disabled.
pub(crate) fn step_blocked(m: &Model, form: &Editor) -> Option<String> {
    if !current(m, form) {
        return Some(STALE.into());
    }
    (!m.is_synthetic_pool(form.pool) && !m.can_mutate_pool(form.pool)).then(|| REQUIRES.into())
}

/// Why the confirm button is disabled, the first that applies; `None` when
/// the reviewed operation can be submitted.
pub(crate) fn blocked(m: &Model, form: &Editor) -> Option<String> {
    if !current(m, form) {
        return Some(STALE.into());
    }
    if let Some(reason) = unavailable(m, form) {
        return Some(reason);
    }
    let denial = form.review.as_ref().and_then(|review| {
        permissions::denial(&permissions::assess_for(
            m,
            form.pool,
            &review.operation.permission_requirements(),
        ))
    });
    if denial.is_some() {
        return denial;
    }
    if m.is_synthetic_pool(form.pool) {
        return Some(DEMO.into());
    }
    (!m.can_mutate_pool(form.pool) || form.target.is_none()).then(|| REQUIRES.into())
}

/// Submits the reviewed operation. Every check of the button is made
/// again; an operation that is refused before it reaches the connection
/// changes nothing (the Rune `apply` returned silently). A synthetic pool
/// has no observed source and never submits. A permission the session is
/// known not to have and a refusal of the submission go to `m.error`, which
/// the surface shows under the editor's own error, as in Rune. An accepted
/// creation with cloud-init inputs turns the surface into its
/// continuation; every other accepted operation closes it.
pub(crate) fn apply(m: &mut Model) {
    let Some(form) = editor(m) else { return };
    let Some(target) = form.target else { return };
    let Some(review) = &form.review else { return };
    let (pool, source) = (form.pool, form.source.id);
    if !m.can_mutate_pool(pool)
        || !current(m, form)
        || !m
            .inventory_for(pool)
            .is_some_and(|inventory| review.operation.availability(&inventory, source).is_available())
    {
        return;
    }
    let operation = review.operation.clone();
    let permission = permissions::assess_for(m, pool, &operation.permission_requirements());
    if let Some(denial) = permissions::denial(&permission) {
        m.error = Some(denial);
        return;
    }
    let continuation = (form.kind == Kind::Create && form.cloud)
        .then_some(form.sr)
        .flatten();
    let command = VmCommand::Lifecycle {
        source: target,
        operation: operation.clone(),
    };
    match m.submit_tracked_command_for(pool, command) {
        Ok(id) => {
            m.error = None;
            let panel = m.areas.vm_lifecycle.panel.take();
            if let (Some(sr), Some(Panel::Editor(mut form))) = (continuation, panel) {
                let draft = std::mem::take(&mut form.cloud_input);
                m.areas.vm_lifecycle.panel =
                    Some(Panel::CloudCreation(Box::new(cloud::Pending {
                        id,
                        pool,
                        source,
                        target: Some(target),
                        operation,
                        sr,
                        draft,
                    })));
            }
        }
        Err(error) => m.error = Some(error.to_string()),
    }
}

/// "{name} ({id})", or the bare id of a repository the inventory does not
/// have.
pub(crate) fn storage_label(inventory: &Inventory, id: SrId) -> String {
    inventory
        .storage
        .iter()
        .find(|sr| sr.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |sr| format!("{} ({id})", sr.name))
}

pub(crate) fn network_label(inventory: &Inventory, id: NetworkId) -> String {
    inventory
        .networks
        .iter()
        .find(|network| network.uuid == id.uuid())
        .map_or_else(
            || id.to_string(),
            |network| format!("{} ({id})", network.name),
        )
}

/// "{name} ({id}) • {size}", or the bare id of a disk the inventory does
/// not have.
pub fn disk_label(inventory: &Inventory, id: VdiId) -> String {
    inventory.details.vdis.get(&id).map_or_else(
        || id.to_string(),
        |disk| {
            format!(
                "{} ({id}) • {}",
                disk.name,
                storage_bytes(disk.virtual_size)
            )
        },
    )
}

/// The repositories a copy or a new VM can be put on: not an ISO library,
/// and `vdi_create` allowed; "{name} ({id})", by name.
pub(crate) fn storage_choices(form: &Editor) -> Vec<(SrId, String)> {
    let mut items: Vec<_> = form
        .initial
        .details
        .storage
        .values()
        .filter(|sr| {
            sr.content_type != "iso"
                && sr
                    .allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "vdi_create"))
        })
        .map(|sr| (sr.id, storage_label(&form.initial, sr.id)))
        .collect();
    items.sort_by_cached_key(|(_, name)| name.to_lowercase());
    items
}
