//! The cloud-init drive of one halted VM: the logic of the Rune
//! `cloud_init_view.rs`.
//!
//! Three phases: the inputs; the preparation of private local media on a
//! worker thread (the result is collected by [`poll`]); the review, from
//! which `Connection::prepare_cloud_init` uploads the media to a new disk
//! and attaches it. The pool is the pool of the VM (the command's target).
//! Slot syntax, slot occupancy and the storage are judged by the core
//! (`CloudInitConfiguration::availability`), not by the form.

use std::sync::mpsc;

use xenadmin_core::{
    CloudInitConfiguration, CloudInitInputs, CloudInitMedia, Error, ObservedVm, OperationId,
    PoolId, PowerState, SrId, VmId,
};
use zeroize::Zeroize;

use super::edit;
use crate::prelude::*;

pub const STALE: &str =
    "Connection or pool changed. Close this review and reopen it from the intended VM.";
pub const READ_ONLY: &str =
    "A current connection with write access is required to upload and attach this drive.";
/// The caption of the inputs; also why the command is disabled for a VM
/// that is not halted (Rune relabelled its button "Cloud-init drive (shut
/// down first)").
pub const INTRODUCTION: &str = "For a halted VM with cloud-init and the NoCloud datasource already installed. Enter UTF-8 user-data; guest-side configuration validation happens in the guest.";
pub const PRIVACY: &str = "Inputs stay in private local media and are never included in the operation's entry on the Events tab of the Notifications view or in exports. Only the reviewed VM receives the new drive.";
pub const PREPARING: &str = "Preparing private local media…";
pub const NO_STORAGE: &str = "Choose storage for the new drive";
pub const WHAT_HAPPENS: &str = "Create a new data disk, upload the prepared media, then attach it read-only. The drive contains your user-data and stays attached. Review its removal separately after provisioning.";
pub const GUEST_LIMITS: &str = "The VM stays halted. Its guest must already support cloud-init with the NoCloud datasource. Preparing this drive does not prove the guest accepted or ran the configuration. A new instance ID may cause an existing cloud-init guest to run first-boot configuration again. External seed media can affect which configuration the guest selects.";
pub const POOL_BLOCK: &str = "Other changes in this pool wait while this operation is active or unresolved. Interrupted uploads or attachments are never retried automatically. Retained disk and attachment UUIDs appear on the Events tab of the Notifications view; uncertain operations may retain a private local artifact.";
const DEFAULT_USER_DATA: &str = "#cloud-config\n";

pub struct Panel {
    pub vm: VmId,
    pub pool: PoolId,
    /// The VM as observed at open; `None` in a synthetic pool.
    pub observed: Option<ObservedVm>,
    pub name: String,
    pub hostname: String,
    pub user_data: String,
    pub network: String,
    pub device: String,
    pub sr: Option<SrId>,
    /// The preparation thread runs.
    pub pending: bool,
    pub receiver: Option<mpsc::Receiver<Result<CloudInitMedia, String>>>,
    /// Present: the review phase.
    pub media: Option<CloudInitMedia>,
    pub error: Option<String>,
}

/// User-data and network-config can hold passwords and keys: what is still
/// in the form when it goes is wiped. (The Rune panel left this to the
/// allocator; its buffers were already emptied when the preparation
/// started.)
impl Drop for Panel {
    fn drop(&mut self) {
        self.user_data.zeroize();
        self.network.zeroize();
    }
}

fn panel(m: &mut Model) -> Option<&mut Panel> {
    m.areas.vm_lifecycle.drive.as_mut()
}

/// Closes the dialog. The receiver goes with it: a late preparation result
/// is discarded and never reopens the dialog.
pub(crate) fn close(m: &mut Model) {
    m.areas.vm_lifecycle.drive = None;
}

/// The panel an object would open: an ordinary VM of its pool's inventory.
/// `Err` is what the command is instead: hidden for anything else (Rune
/// showed no button), disabled for a VM that is not halted.
pub(crate) fn capture(m: &Model, object: &ObjectRef) -> Result<Panel, Eligibility> {
    if object.kind != ObjectKind::Vm {
        return Err(Eligibility::Hidden);
    }
    let (Some(pool), Some(vm)) = (object.pool, object.vm_id()) else {
        return Err(Eligibility::Hidden);
    };
    let Some(i) = m.inventory_for(pool) else {
        return Err(Eligibility::Hidden);
    };
    let Some(v) = i
        .vms
        .get(&vm)
        .filter(|v| !v.is_template && !v.is_control_domain)
    else {
        return Err(Eligibility::Hidden);
    };
    if v.power_state != PowerState::Halted {
        return Err(Eligibility::disabled(INTRODUCTION));
    }
    Ok(created(m, pool, vm).expect("the VM was just found"))
}

/// The panel of a VM whatever its power state: the Rune `open`.
fn created(m: &Model, pool: PoolId, vm: VmId) -> Option<Panel> {
    let i = m.inventory_for(pool)?;
    let v = i.vms.get(&vm)?;
    if v.is_template || v.is_control_domain {
        return None;
    }
    let slot = (0_u16..=255)
        .find(|slot| {
            !i.details
                .vbds
                .values()
                .any(|v| v.vm == Some(vm) && v.userdevice == slot.to_string())
        })
        .unwrap_or(0);
    Some(Panel {
        vm,
        pool,
        observed: m.snapshot_for(pool).and_then(|snap| snap.observe_vm(vm).ok()),
        name: v.name.clone(),
        hostname: String::new(),
        user_data: DEFAULT_USER_DATA.into(),
        network: String::new(),
        device: slot.to_string(),
        sr: None,
        pending: false,
        receiver: None,
        media: None,
        error: None,
    })
}

/// Opens the dialog on its inputs. Nothing opens for an object the command
/// is hidden or disabled for.
pub(crate) fn open(m: &mut Model, object: &ObjectRef) {
    if let Ok(panel) = capture(m, object) {
        super::close(m);
        m.areas.vm_lifecycle.drive = Some(panel);
    }
}

/// Opens the dialog for the VM a creation made, with the inputs of the
/// creation wizard and the repository chosen there, and prepares the media
/// at once.
pub fn open_created(
    m: &mut Model,
    pool: PoolId,
    vm: VmId,
    sr: SrId,
    hostname: String,
    user_data: String,
    network: String,
) {
    let mut secrets = (user_data, network);
    let Some(mut panel) = created(m, pool, vm) else {
        secrets.0.zeroize();
        secrets.1.zeroize();
        return;
    };
    panel.sr = Some(sr);
    panel.hostname = hostname;
    panel.user_data = std::mem::take(&mut secrets.0);
    panel.network = std::mem::take(&mut secrets.1);
    m.areas.vm_lifecycle.drive = Some(panel);
    prepare(m);
}

/// The VM is still in its pool's inventory and, outside a synthetic pool,
/// still what was observed.
pub(crate) fn current(m: &Model, p: &Panel) -> bool {
    m.inventory_for(p.pool)
        .is_some_and(|i| i.pool_id == p.pool && i.vms.contains_key(&p.vm))
        && (m.is_synthetic_pool(p.pool)
            || p.observed.is_some_and(|o| {
                m.snapshot_for(p.pool)
                    .and_then(|snap| snap.observe_vm(p.vm).ok())
                    == Some(o)
            }))
}

/// The configuration the form describes. Before the media exists its
/// identity is a placeholder.
pub(crate) fn config(p: &Panel) -> Option<CloudInitConfiguration> {
    Some(CloudInitConfiguration {
        sr: p.sr?,
        device: p.device.clone(),
        configuration_id: p.media.as_ref().map_or_else(
            || uuid::Uuid::from_u128(1),
            CloudInitMedia::configuration_id,
        ),
    })
}

/// What the core says against the configuration in the inventory as it is
/// now; `None` when it is available, when no repository is chosen yet and
/// when there is no inventory.
pub(crate) fn unavailable(m: &Model, p: &Panel) -> Option<String> {
    let config = config(p)?;
    let i = m.inventory_for(p.pool)?;
    let status = config.availability(&i, p.vm);
    (!status.is_available()).then(|| status.explanation())
}

pub(crate) fn can_prepare(m: &Model, p: &Panel) -> bool {
    current(m, p)
        && config(p).is_some_and(|c| {
            m.inventory_for(p.pool)
                .is_some_and(|i| c.availability(&i, p.vm).is_available())
        })
}

/// Why "Prepare and review" is disabled: the first reason [`can_prepare`]
/// fails for.
pub(crate) fn prepare_blocked(m: &Model, p: &Panel) -> Option<String> {
    if !current(m, p) {
        return Some(STALE.into());
    }
    if p.sr.is_none() {
        return Some(NO_STORAGE.into());
    }
    unavailable(m, p).or_else(|| (!can_prepare(m, p)).then(|| STALE.into()))
}

/// Why "Upload and attach drive" is disabled.
pub(crate) fn apply_blocked(m: &Model, p: &Panel) -> Option<String> {
    if let Some(reason) = prepare_blocked(m, p) {
        return Some(reason);
    }
    let denial = config(p).and_then(|c| {
        permissions::denial(&permissions::assess_for(
            m,
            p.pool,
            &c.permission_requirements(),
        ))
    });
    if denial.is_some() {
        return denial;
    }
    if m.is_synthetic_pool(p.pool) {
        return Some(edit::DEMO.into());
    }
    (!m.can_mutate_pool(p.pool) || p.observed.is_none()).then(|| READ_ONLY.into())
}

/// Validates the inputs and starts the preparation of the private media.
/// The validation comes before the secrets are taken: a bad hostname keeps
/// the typed user-data. Once the thread runs, the form's buffers are empty.
pub(crate) fn prepare(m: &mut Model) {
    let Some(p) = m.areas.vm_lifecycle.drive.as_ref() else {
        return;
    };
    if p.pending || !can_prepare(m, p) {
        return;
    }
    let wake = m.waker.clone();
    let Some(p) = panel(m) else { return };
    let vm = p.vm;
    if let Err(e) = CloudInitInputs::validate(
        p.hostname.trim(),
        &p.user_data,
        (!p.network.trim().is_empty()).then_some(p.network.as_str()),
    ) {
        p.error = Some(e.to_string());
        return;
    }
    let inputs = CloudInitInputs::new(
        p.hostname.trim().into(),
        std::mem::take(&mut p.user_data),
        (!p.network.trim().is_empty()).then(|| std::mem::take(&mut p.network)),
    );
    let inputs = match inputs {
        Ok(v) => v,
        Err(e) => {
            p.error = Some(e.to_string());
            return;
        }
    };
    let (tx, rx) = mpsc::channel();
    match std::thread::Builder::new()
        .name("cloud-init-media".into())
        .spawn(move || {
            let result = CloudInitMedia::prepare(vm, inputs).map_err(|e| e.to_string());
            let _ = tx.send(result);
            if let Some(w) = wake {
                w();
            }
        }) {
        Ok(_) => {
            p.pending = true;
            p.receiver = Some(rx);
            p.error = None;
            p.user_data.zeroize();
            p.network.zeroize();
        }
        Err(_) => p.error = Some("Cannot start private media preparation".into()),
    }
}

/// Collects the result of the preparation thread, without waiting.
pub(crate) fn poll(m: &mut Model, _clock: f64) -> bool {
    let Some(p) = panel(m) else { return false };
    let Some(rx) = &p.receiver else {
        return false;
    };
    let result = match rx.try_recv() {
        Ok(v) => v,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("Private media preparation stopped".into()),
    };
    p.receiver = None;
    p.pending = false;
    match result {
        Ok(media) => p.media = Some(media),
        Err(e) => p.error = Some(e),
    }
    true
}

/// "Discard and edit": the media goes, the inputs start again.
pub(crate) fn discard(m: &mut Model) {
    if let Some(p) = panel(m) {
        p.media = None;
        p.user_data = DEFAULT_USER_DATA.into();
    }
}

/// Uploads and attaches the reviewed drive. Every check of the button is
/// made again. The media is consumed even when the call then fails: the
/// form returns to its inputs with empty user-data. Accepted, the dialog
/// closes and the Events tab shows the operation (Rune opened Activity).
pub(crate) fn apply(m: &mut Model) {
    let Some(p) = m.areas.vm_lifecycle.drive.as_ref() else {
        return;
    };
    let pool = p.pool;
    if m.is_synthetic_pool(pool) || !m.can_mutate_pool(pool) || !can_prepare(m, p) {
        return;
    }
    let Some(observed) = p.observed else { return };
    let Some(config) = config(p) else { return };
    let permission = permissions::assess_for(m, pool, &config.permission_requirements());
    let denial = permissions::denial(&permission);
    let connection = m.connection_for(pool);
    let Some(p) = panel(m) else { return };
    if let Some(denial) = denial {
        p.error = Some(denial);
        return;
    }
    let Some(media) = p.media.take() else { return };
    let result = connection.ok_or(Error::Closed).and_then(|c| {
        c.prepare_cloud_init(
            OperationId::new(),
            observed,
            config.sr,
            config.device,
            media,
        )
    });
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            close(m);
            crate::areas::events::show(m);
        }
        Err(e) => p.error = Some(e.to_string()),
    }
}
