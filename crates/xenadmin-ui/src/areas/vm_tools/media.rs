//! Guest tools media: the Rune `guest_tools_view.rs`, logic and description.
//!
//! One dialog with three states, as in Rune: choose a CD drive and a tools
//! ISO, review the mount, then follow the submitted operation in place. The
//! dialog does not close on submit.
//!
//! The panel captures the inventory, the VM and its observation when it
//! opens and never retargets. The operation is built from the CAPTURED
//! inventory (what the user chose from); its availability is evaluated
//! against the LIVE inventory of the same pool. The confirm button and the
//! handler make the same checks; the handler says why it refused where Rune
//! returned silently behind its disabled button.
//!
//! Keys and labels are the Rune ones: the surface `tools-dialog`,
//! `tools-cd-<vbd>` and `tools-image-<vdi>` (the options of the two choices,
//! which were "Select drive" / "Select tools ISO" buttons), `tools-review`,
//! `tools-back`, `tools-confirm`, `tools-close`. New keys, for elements Rune
//! left without one: `tools-cd` and `tools-image` (the two choices),
//! `tools-no-cd`, `tools-no-media`, `tools-over-limit`, `tools-error`,
//! `tools-mount-review` (and so `tools-mount-review-stale`), `tools-outcome`,
//! `tools-admitted`, `tools-guide`.

use std::sync::Arc;

use xenadmin_core::{
    DeviceCommand, DeviceOperation, Inventory, ObservedVm, OperationId, OperationRecord,
    OperationState, PermissionAssessment, PoolId, VbdId, VdiId, VmCommand, VmId,
};

use crate::prelude::*;

/// The id of the dialog: the key of the Rune dialog.
pub const SURFACE: &str = "tools-dialog";

/// The key of the review, so `tools-mount-review-stale` is its stale line.
pub const REVIEW: &str = "tools-mount-review";

pub const GUIDE_URL: &str = "https://docs.xcp-ng.org/vms/";

pub const INTRO: &str = "Mount the host's guest tools ISO, then run the appropriate installer \
    inside the guest. Existing guest tools and drivers should be checked before changing versions.";

/// The empty state of the drive list, and the reason the command is
/// disabled for a VM without a CD drive. (It names the Rune devices page;
/// renamed with the other Rune places in the sweep of decision 10.)
pub const NO_CD: &str = "No CD drive is available. Shut down the VM and add an empty CD drive \
    with Add CD/DVD Drive on the Storage tab of the VM.";

/// The empty state of the media list, and the reason the command is
/// disabled in a pool without tools media.
pub const NO_MEDIA: &str = "No guest tools ISO is advertised or recognized in this inventory. \
    Check the host's tools repository or add the appropriate tools ISO to an ISO library.";

pub const OVER_LIMIT: &str = "This pool exceeds the media chooser's display limit; reduce the \
    advertised tools media before continuing.";

pub const MOUNTED: &str = "Tools media is mounted. Open the console to run the Windows \
    installer or the Linux installer from the mounted media. Guest tools status will update \
    when its agent reports to the host.";

/// Shown until the record of the submitted operation is known. (It names
/// the Rune Activity dialog: decision 10.)
pub const ADMITTED: &str = "Operation admitted; open the Events tab of the Notifications view to inspect its progress.";

/// New text. Rune disabled "Mount tools media" without a reason when the
/// connection was not writable and current or the VM was no longer the one
/// observed at capture.
pub const STALE: &str = "The connection or the VM changed after this dialog opened. Close and \
    reopen it before mounting.";

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

const MAX_DRIVES: usize = 128;
const MAX_MEDIA: usize = 256;

pub struct Panel {
    pub(super) inventory: Arc<Inventory>,
    pub(super) vm: VmId,
    pub(super) observed: Option<ObservedVm>,
    pub(super) cd: Option<VbdId>,
    pub(super) image: Option<VdiId>,
    pub(super) review: Option<DeviceOperation>,
    pub(super) submitted: Option<OperationId>,
    pub(super) error: Option<String>,
}

impl Panel {
    /// The pool of the captured VM: every check reads this pool's session,
    /// not the scope session's.
    pub fn pool(&self) -> PoolId {
        self.inventory.pool_id
    }

    /// The CD drives of the captured VM, in the order of its attachments.
    fn drives(&self) -> Vec<&xenadmin_core::VirtualBlockDevice> {
        let details = &self.inventory.details;
        details
            .vm_disks
            .get(&self.vm)
            .into_iter()
            .flatten()
            .filter_map(|id| details.vbds.get(id))
            .filter(|v| v.kind == "CD")
            .collect()
    }
}

/// The panel the command would open for a VM: `None` when the object is not
/// a VM its pool's inventory has. The first CD drive is preselected; the
/// tools image never is (an explicit choice is required).
pub fn capture(m: &Model, object: &ObjectRef) -> Option<Panel> {
    if object.kind != ObjectKind::Vm {
        return None;
    }
    let pool = object.pool?;
    let vm = object.vm_id()?;
    let inventory = m.inventory_for(pool)?;
    if !inventory.vms.contains_key(&vm) {
        return None;
    }
    let cd = inventory
        .details
        .vm_disks
        .get(&vm)
        .into_iter()
        .flatten()
        .find(|id| {
            inventory
                .details
                .vbds
                .get(id)
                .is_some_and(|v| v.kind == "CD")
        })
        .copied();
    Some(Panel {
        observed: m.snapshot_for(pool).and_then(|v| v.observe_vm(vm).ok()),
        inventory,
        vm,
        cd,
        image: None,
        review: None,
        submitted: None,
        error: None,
    })
}

/// Why no choice in the dialog could lead to a review: the text of the
/// empty state the dialog would show.
pub fn blocked(p: &Panel) -> Option<String> {
    if p.drives().is_empty() {
        Some(NO_CD.into())
    } else if p.inventory.details.tools_media.is_empty() {
        Some(NO_MEDIA.into())
    } else {
        None
    }
}

pub fn open(m: &mut Model, object: &ObjectRef) {
    if let Some(panel) = capture(m, object) {
        m.areas.vm_tools.media = Some(panel);
    }
}

pub fn close(m: &mut Model) {
    m.areas.vm_tools.media = None;
}

fn operation(p: &Panel) -> Result<DeviceOperation, String> {
    let cd = p.cd.ok_or("Select a CD drive")?;
    let image = p.image.ok_or("Select guest tools media")?;
    let drive = p
        .inventory
        .details
        .vbds
        .get(&cd)
        .ok_or("CD drive unavailable")?;
    let previous = match drive.empty {
        Some(true) if drive.vdi.is_none() => None,
        Some(false) => Some(drive.vdi.ok_or("Mounted image identity is unavailable")?),
        _ => return Err("Mounted image state is unknown".into()),
    };
    let sr = p
        .inventory
        .details
        .vdis
        .get(&image)
        .and_then(|d| d.sr)
        .ok_or("Image storage repository is unavailable")?;
    Ok(DeviceOperation::MountGuestTools {
        vbd: cd,
        image,
        sr,
        previous,
    })
}

pub fn review(m: &mut Model) {
    let Some(p) = &mut m.areas.vm_tools.media else {
        return;
    };
    match operation(p) {
        Ok(op) => {
            p.review = Some(op);
            p.error = None;
        }
        Err(e) => p.error = Some(e),
    }
}

pub fn back(m: &mut Model) {
    if let Some(p) = &mut m.areas.vm_tools.media {
        p.review = None;
    }
}

fn select_cd(m: &mut Model, id: &str) {
    if let Some(p) = &mut m.areas.vm_tools.media {
        let found = p.drives().into_iter().find(|v| v.id.to_string() == id);
        if let Some(id) = found.map(|v| v.id) {
            p.cd = Some(id);
        }
    }
}

fn select_image(m: &mut Model, id: &str) {
    if let Some(p) = &mut m.areas.vm_tools.media {
        let found = p
            .inventory
            .details
            .tools_media
            .keys()
            .find(|image| image.to_string() == id)
            .copied();
        if found.is_some() {
            p.image = found;
        }
    }
}

/// Whether the reviewed mount can still be submitted as reviewed: the pool
/// is writable and current, and the VM is the one observed at capture.
fn current(m: &Model, p: &Panel) -> bool {
    let pool = p.pool();
    m.can_mutate_pool(pool)
        && p.observed.is_some()
        && m.snapshot_for(pool).and_then(|v| v.observe_vm(p.vm).ok()) == p.observed
        && m.inventory_for(pool).is_some()
}

/// The stale line of the review. A synthetic pool has no observation to go
/// stale; its confirm button says [`DEMO`].
pub fn stale(m: &Model, p: &Panel) -> Option<String> {
    (p.review.is_some()
        && p.submitted.is_none()
        && !m.is_synthetic_pool(p.pool())
        && !current(m, p))
    .then(|| STALE.to_owned())
}

/// The core's explanation when the reviewed mount is not available in the
/// live inventory of the captured pool.
pub fn unavailable(m: &Model, p: &Panel) -> Option<String> {
    let op = p.review.as_ref()?;
    let live = m.inventory_for(p.pool())?;
    let availability = op.availability(&live, p.vm);
    (!availability.is_available()).then(|| availability.explanation())
}

fn permission(m: &Model, p: &Panel) -> Option<PermissionAssessment> {
    p.review
        .as_ref()
        .map(|op| permissions::assess_for(m, p.pool(), &op.permission_requirements()))
}

/// Why the confirm button is disabled, and why [`apply`] refuses. The first
/// reason wins: stale, unavailable, a known permission denial, the demo.
fn refusal(m: &Model, p: &Panel) -> Option<String> {
    stale(m, p)
        .or_else(|| unavailable(m, p))
        .or_else(|| permission(m, p).as_ref().and_then(permissions::denial))
        .or_else(|| m.is_synthetic_pool(p.pool()).then(|| DEMO.to_owned()))
}

/// Submits the reviewed mount, once. Every check of the button is made
/// again; the submission goes through the model's one choke point for VM
/// commands (Rune had its own copy of it here), which refuses a synthetic
/// pool, a busy session and a closed connection.
pub fn apply(m: &mut Model) {
    let Some(p) = &m.areas.vm_tools.media else {
        return;
    };
    if p.submitted.is_some() {
        return;
    }
    let Some(op) = p.review.clone() else { return };
    let pool = p.pool();
    let result = match (refusal(m, p), p.observed) {
        (Some(reason), _) => Err(reason),
        (None, None) => Err(STALE.to_owned()),
        (None, Some(observed)) => m
            .submit_tracked_command_for(
                pool,
                VmCommand::Device(DeviceCommand {
                    vm: observed,
                    operation: op,
                }),
            )
            .map_err(|e| e.to_string()),
    };
    let Some(p) = &mut m.areas.vm_tools.media else {
        return;
    };
    match result {
        Ok(id) => p.submitted = Some(id),
        Err(e) => p.error = Some(e),
    }
}

/// The record of the submitted operation: the live handle first, as in
/// Rune, else the merged rows.
fn record(m: &Model, id: OperationId) -> Option<OperationRecord> {
    m.operations
        .iter()
        .map(|o| o.latest())
        .find(|r| r.id == id)
        .map(|r| (*r).clone())
        .or_else(|| m.operation_rows.iter().find(|r| r.id == id).cloned())
}

fn image_name(i: &Inventory, id: VdiId) -> String {
    i.details
        .vdis
        .get(&id)
        .map(|d| format!("{} ({id})", d.name))
        .unwrap_or_else(|| id.to_string())
}

// -- Description ----------------------------------------------------------------

pub fn surface(m: &Model) -> Option<Surface> {
    let p = m.areas.vm_tools.media.as_ref()?;
    let i = &p.inventory;
    let name = i
        .vms
        .get(&p.vm)
        .map(|v| v.name.as_str())
        .unwrap_or("Unavailable");
    let state = match (p.submitted, &p.review) {
        (Some(id), _) => submitted(m, id),
        (None, Some(op)) => reviewing(m, p, op),
        (None, None) => choosing(p),
    };
    let surface = Surface::dialog(SURFACE, "Guest tools media")
        .subtitle(format!("{name} ({})", p.vm))
        .experimental(true)
        .body(blocks![
            caption(INTRO),
            p.error.clone().map(|e| alert(e).key("tools-error")),
            state,
        ])
        .footer(footer(m, p))
        .on_close(close);
    Some(match permission(m, p) {
        Some(permission) => surface.footer_note(permissions::footer_note(&permission)),
        None => surface,
    })
}

fn choosing(p: &Panel) -> Vec<Block> {
    let i = &p.inventory;
    let cds = p.drives();
    let drives: Vec<OptionItem> = cds
        .iter()
        .take(MAX_DRIVES)
        .map(|cd| {
            let contents = match (cd.empty, cd.vdi) {
                (Some(true), None) => "Empty".into(),
                (Some(false), Some(id)) => image_name(i, id),
                _ => "Media state unavailable".to_owned(),
            };
            option(cd.id.to_string(), format!("Drive {}", cd.userdevice))
                .detail(contents)
                .key(format!("tools-cd-{}", cd.id))
        })
        .collect();
    let media: Vec<OptionItem> = i
        .details
        .tools_media
        .iter()
        .take(MAX_MEDIA)
        .map(|(id, source)| {
            option(id.to_string(), source.label())
                .detail(image_name(i, *id))
                .key(format!("tools-image-{id}"))
        })
        .collect();
    let cd = p.cd.map(|id| id.to_string());
    let image = p.image.map(|id| id.to_string());
    blocks![
        heading("CD drive"),
        drives
            .is_empty()
            .then(|| caption(NO_CD).key("tools-no-cd")),
        (!drives.is_empty()).then(|| choice("tools-cd", drives, cd.as_deref(), select_cd)),
        heading("Guest tools ISO"),
        media
            .is_empty()
            .then(|| caption(NO_MEDIA).key("tools-no-media")),
        (!media.is_empty()).then(|| choice("tools-image", media, image.as_deref(), select_image)),
        (cds.len() > MAX_DRIVES || i.details.tools_media.len() > MAX_MEDIA)
            .then(|| caption(OVER_LIMIT).key("tools-over-limit")),
    ]
}

fn reviewing(m: &Model, p: &Panel, op: &DeviceOperation) -> Vec<Block> {
    let DeviceOperation::MountGuestTools {
        vbd,
        image,
        sr,
        previous,
    } = op
    else {
        return Vec::new();
    };
    let i = &p.inventory;
    let line = match previous {
        Some(old) => format!(
            "Eject {} first, then mount the selected tools ISO. If mounting fails, the drive \
             can remain empty; inspect the operation before retrying.",
            image_name(i, *old)
        ),
        None => "Mount the selected tools ISO in the reviewed empty CD drive.".to_owned(),
    };
    blocks![
        crate::surface::review(REVIEW, [line])
            .affected([
                property("CD drive", vbd.to_string()),
                property("Tools ISO", image_name(i, *image)),
                property("Storage repository", sr.to_string()),
            ])
            .warnings(unavailable(m, p))
            .stale(stale(m, p))
    ]
}

fn submitted(m: &Model, id: OperationId) -> Vec<Block> {
    let Some(record) = record(m, id) else {
        return blocks![caption(ADMITTED).key("tools-admitted")];
    };
    let done = record.state == OperationState::Succeeded;
    blocks![
        outcome("tools-outcome", outcome_state(&record), "Operation")
            .line(format!("{:?}: {}", record.state, record.detail))
            .lines(
                record
                    .steps
                    .iter()
                    .map(|step| format!("{} · {:?}", step.label, step.state))
            )
            .lines(done.then_some(MOUNTED))
    ]
}

fn outcome_state(record: &OperationRecord) -> OutcomeState {
    match record.state {
        OperationState::Succeeded => OutcomeState::Succeeded,
        OperationState::PartiallyCompleted => OutcomeState::Partial,
        OperationState::Failed | OperationState::Rejected => OutcomeState::Failed,
        OperationState::Cancelled => OutcomeState::Cancelled,
        OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown => {
            OutcomeState::Unknown
        }
        _ => OutcomeState::Running(record.progress.map(|p| p.clamp(0.0, 1.0) as f32)),
    }
}

/// Close, the guide, then the step: "Review mounting" while choosing, Back
/// and "Mount tools media" while reviewing, nothing once submitted. The
/// last button is the default action.
fn footer(m: &Model, p: &Panel) -> Vec<Button> {
    let close = button("tools-close", "Close", close);
    let guide = button("tools-guide", "Installation guide", |m| {
        m.open_url(GUIDE_URL);
    });
    if p.submitted.is_some() {
        return vec![guide, close];
    }
    if p.review.is_none() {
        return vec![
            close,
            guide,
            button("tools-review", "Review mounting", review)
                .primary()
                .disabled_if(p.cd.is_none(), "Select a CD drive")
                .disabled_if(p.image.is_none(), "Select guest tools media"),
        ];
    }
    vec![
        close,
        guide,
        button("tools-back", "Back", back),
        button("tools-confirm", "Mount tools media", apply)
            .primary()
            .disabled_opt(refusal(m, p)),
    ]
}
