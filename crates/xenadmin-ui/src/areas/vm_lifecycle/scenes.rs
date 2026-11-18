//! The scenes of the area, under their Rune names.
//!
//! Each sets the demo pool up as its Rune scene did and then goes the way
//! a user goes: the object selected, the command invoked. The draft is
//! then written as the Rune scene wrote it.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    CloudInitInputs, CloudInitMedia, Inventory, OperationAction, OperationId, OperationRecord,
    PbdId, PowerState, StorageAttachment, VmId,
};

use super::commands::{CLOUD_INIT, CREATE_FROM_SELECTION};
use super::{Panel, cloud, creation, edit};
use crate::prelude::*;

const AREA: &str = "vm_lifecycle";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("vm-create", AREA, create_scene),
        SceneSpec::new("lifecycle-permissions", AREA, review_scene),
        SceneSpec::new("firmware-create", AREA, firmware_scene),
        SceneSpec::new("creation-cloud", AREA, cloud_scene),
        SceneSpec::new("creation-cloud-pending", AREA, cloud_pending_scene),
        SceneSpec::new("cloud-init", AREA, drive_scene),
        SceneSpec::new("cloud-init-review", AREA, drive_review_scene),
    ]
}

/// The first guest of the demo pool: the source of every lifecycle scene.
pub fn demo_source() -> VmId {
    VmId::from(Uuid::from_u128(0x3000_0000_0000_0000_0000_0000_0000_0000))
}

/// The VM the synthetic creation of `creation-cloud-pending` made.
pub fn demo_created() -> VmId {
    VmId::from(Uuid::from_u128(0x3000_0000_0000_0000_0000_0000_0000_0099))
}

/// Replaces the scope session's inventory with an edited copy.
pub(crate) fn amend(m: &mut Model, change: impl FnOnce(&mut Inventory)) {
    if let Some(inventory) = m.inventory.take() {
        let mut inventory = inventory.as_ref().clone();
        change(&mut inventory);
        m.inventory = Some(Arc::new(inventory));
    }
}

/// The fixture of the Rune `vm_lifecycle::scene` before anything opens:
/// the demo pool, its first guest made the custom template "Application
/// template" that may be cloned, copied and destroyed. The template is
/// selected and returned.
pub fn prepare(m: &mut Model) -> Option<ObjectRef> {
    m.load_demo(3, 48);
    let id = demo_source();
    amend(m, |inventory| {
        if let Some(source) = inventory.vms.get_mut(&id) {
            source.is_template = true;
            source.name = "Application template".into();
        }
        if let Some(details) = inventory.details.vms.get_mut(&id) {
            details.is_default_template = Some(false);
            details.allowed_operations =
                Some(vec!["clone".into(), "copy".into(), "destroy".into()]);
        }
    });
    let pool = m.inventory.as_ref()?.pool_id;
    let object = ObjectRef::template(pool, id);
    m.select(Target::one(object.clone()));
    Some(object)
}

/// `vm-create`: the wizard on its Storage step (the Rune scene showed its
/// third step, which had the repository, the network and the disk size),
/// with a name, the first repository, the first network and an additional
/// disk of 16 GiB.
fn create_scene(m: &mut Model) {
    let Some(template) = prepare(m) else { return };
    m.invoke(CREATE_FROM_SELECTION, &Target::one(template));
    if let Some(form) = edit::editor_mut(m) {
        form.step = creation::Step::Storage;
        form.name = "New application VM".into();
        form.sr = form.initial.details.storage.keys().next().copied();
        form.network = form.initial.details.networks.keys().next().copied();
        form.disk_size = "16 GiB".into();
    }
}

/// `lifecycle-permissions`: the review of that creation, with its
/// permission line.
fn review_scene(m: &mut Model) {
    create_scene(m);
    edit::review(m);
}

/// `firmware-create`: the CPU and Memory step for a BIOS template with an
/// empty BIOS identity: the firmware and the BIOS identity choices.
fn firmware_scene(m: &mut Model) {
    create_scene(m);
    if let Some(form) = edit::editor_mut(m) {
        form.step = creation::Step::Resources;
        let inventory = Arc::make_mut(&mut form.initial);
        if let Some(details) = inventory.details.vms.get_mut(&form.source.id) {
            details.firmware = Some("bios".into());
            details.bios_identity_empty = Some(true);
        }
    }
}

/// `creation-cloud`: the Cloud-init step with the configuration turned on,
/// a hostname and user-data.
fn cloud_scene(m: &mut Model) {
    create_scene(m);
    if let Some(form) = edit::editor_mut(m) {
        form.cloud = true;
        form.step = creation::Step::Cloud;
        form.cloud_input.hostname = "new-application".into();
        form.cloud_input.user_data = "#cloud-config\npackages: [curl]\n".into();
    }
}

/// `creation-cloud-pending`: what follows the submission of that creation,
/// with a synthetic record of its success and the VM it made in the
/// inventory.
fn cloud_pending_scene(m: &mut Model) {
    cloud_scene(m);
    let Some(form) = edit::editor_mut(m) else {
        return;
    };
    let (Ok(operation), Some(sr)) = (creation::operation(form), form.sr) else {
        return;
    };
    let p = cloud::Pending {
        id: OperationId::new(),
        pool: form.pool,
        source: form.source.id,
        target: None,
        operation,
        sr,
        draft: std::mem::take(&mut form.cloud_input),
    };
    let vm = demo_created();
    amend(m, |i| {
        let mut guest = i.vms[&p.source].clone();
        guest.id = vm;
        guest.is_template = false;
        guest.name = "New application VM".into();
        i.vms.insert(vm, guest);
        let details = i.details.vms[&p.source].clone();
        i.details.vms.insert(vm, details);
    });
    let record: Result<OperationRecord, _> = serde_json::from_value(serde_json::json!({
        "id": p.id,
        "pool": p.pool,
        "vm": p.source,
        "action": OperationAction::Lifecycle(p.operation.clone()),
        "state": "Succeeded",
        "progress": null,
        "detail": "Synthetic creation completed; configuration not uploaded",
        "created_at_ms": 1,
        "created_vm": vm,
    }));
    if let Ok(record) = record {
        m.operation_rows.push(record);
    }
    m.areas.vm_lifecycle.panel = Some(Panel::CloudCreation(Box::new(p)));
}

/// `cloud-init`: the inputs of the cloud-init drive of the first halted
/// guest of the demo pool, every repository attached, with a hostname and
/// the first repository for user data chosen.
fn drive_scene(m: &mut Model) {
    m.load_demo(3, 48);
    amend(m, |inventory| {
        let host = inventory.hosts.keys().next().copied();
        let repositories: Vec<_> = inventory.details.storage.keys().copied().collect();
        for (index, sr) in repositories.into_iter().enumerate() {
            let id = PbdId::from(Uuid::from_u128(900 + index as u128));
            inventory.details.pbds.insert(
                id,
                StorageAttachment {
                    id,
                    location: None,
                    requires_credentials: false,
                    multipathed: Some(false),
                    paths: vec![],
                    host,
                    sr: Some(sr),
                    currently_attached: Some(true),
                },
            );
        }
    });
    let Some(inventory) = m.inventory.clone() else {
        return;
    };
    let Some(vm) = inventory
        .guests()
        .find(|v| v.power_state == PowerState::Halted)
        .map(|v| v.id)
    else {
        return;
    };
    let object = ObjectRef::vm(inventory.pool_id, vm);
    m.select(Target::one(object.clone()));
    m.invoke(CLOUD_INIT, &Target::one(object));
    if let Some(p) = m.areas.vm_lifecycle.drive.as_mut() {
        p.hostname = "fixture-guest".into();
        p.sr = inventory
            .details
            .storage
            .values()
            .find(|sr| sr.content_type == "user")
            .map(|sr| sr.id);
    }
}

/// `cloud-init-review`: the review of that drive. Synthetic render setup
/// only: the media is prepared here, where the interactive path prepares
/// it on a worker thread.
fn drive_review_scene(m: &mut Model) {
    drive_scene(m);
    if let Some(p) = m.areas.vm_lifecycle.drive.as_mut() {
        p.media = CloudInitInputs::new(
            "fixture-guest".into(),
            "#cloud-config\nwrite_files: []\n".into(),
            None,
        )
        .ok()
        .and_then(|inputs| CloudInitMedia::prepare(p.vm, inputs).ok());
        p.user_data = String::new();
    }
}
