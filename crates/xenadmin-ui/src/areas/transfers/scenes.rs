//! The scenes of the area, under their Rune names. Each sets the demo pool
//! up as its Rune scene did.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    DiskImageFormat, ExportSource, GuestFirmware, Inventory, OvfAttachment, OvfDisk, OvfGuest,
    OvfNic, OvfPackageFormat, OvfSummary, PbdId, PowerState, Snapshot, SnapshotDetails,
    SnapshotId, SrId, StorageAttachment, VdiId, VmId, XvaCompression,
};

use super::panel;
use crate::prelude::*;

const AREA: &str = "transfers";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("archive-export", AREA, |m| {
            export(m);
        }),
        SceneSpec::new("xva-compression", AREA, compression_scene),
        SceneSpec::new("checkpoint-export", AREA, checkpoint_scene),
        SceneSpec::new("ovf-import", AREA, |m| ovf_scene(m, false)),
        SceneSpec::new("ovf-export", AREA, |m| ovf_scene(m, true)),
        SceneSpec::new("disk-export", AREA, disk_scene),
        SceneSpec::new("local-conversion", AREA, conversion_scene),
        SceneSpec::new("local-conversion-review", AREA, conversion_review_scene),
    ]
}

/// Replaces the scope session's inventory with an edited copy.
pub(crate) fn amend(m: &mut Model, edit: impl FnOnce(&mut Inventory)) {
    if let Some(inventory) = m.inventory.take() {
        let mut inventory = inventory.as_ref().clone();
        edit(&mut inventory);
        m.inventory = Some(Arc::new(inventory));
    }
}

/// The Rune `transfers::scene`: the first guest of the demo pool, halted
/// and allowed to export, selected, with its export open.
pub(crate) fn export(m: &mut Model) -> Option<VmId> {
    m.load_demo(3, 48);
    let i = m.inventory.clone()?;
    let id = i
        .vms
        .values()
        .find(|v| !v.is_control_domain && !v.is_template)?
        .id;
    amend(m, |i| {
        if let Some(vm) = i.vms.get_mut(&id) {
            vm.power_state = PowerState::Halted;
        }
        if let Some(details) = i.details.vms.get_mut(&id) {
            details.allowed_operations = Some(vec!["export".into()]);
        }
    });
    m.select(Target::one(ObjectRef::vm(i.pool_id, id)));
    panel::begin(
        m,
        Some(i.pool_id),
        Some(ExportSource::Vm {
            vm: id,
            expected_template: false,
        }),
    );
    Some(id)
}

fn compression_scene(m: &mut Model) {
    export(m);
    m.areas.transfers.panel.compression = XvaCompression::Zstd;
}

pub(crate) fn checkpoint_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(parent) = m
        .inventory
        .as_ref()
        .and_then(|i| i.vms.keys().next().copied())
    else {
        return;
    };
    let snapshot = SnapshotId::from(Uuid::from_u128(0x9901));
    let memory = VdiId::from(Uuid::from_u128(0x9902));
    amend(m, |i| {
        let Some(mut disk) = i.details.vdis.values().next().cloned() else {
            return;
        };
        disk.id = memory;
        disk.name = "Checkpoint memory".into();
        disk.kind = "suspend".into();
        disk.virtual_size = Some(512 * 1024 * 1024);
        disk.managed = Some(true);
        disk.missing = Some(false);
        i.details.vdis.insert(memory, disk);
        i.details.snapshots.insert(
            snapshot,
            SnapshotDetails {
                memory_vdi: Some(memory),
                memory_vdi_known: true,
                can_export: Some(true),
                ..Default::default()
            },
        );
        i.snapshots.insert(Snapshot {
            id: snapshot,
            vm: Some(parent),
            name: "Application memory checkpoint".into(),
            description: "Synthetic checkpoint for archive review".into(),
            created_at: None,
            power_state: PowerState::Suspended,
            allowed_actions: vec![],
        });
    });
    let Some(i) = m.inventory.clone() else { return };
    let Some(source) = ExportSource::checkpoint(&i, snapshot, parent, false) else {
        return;
    };
    panel::begin(m, Some(i.pool_id), Some(source));
}

/// The Rune `ovf_ui::scene`: the review of an OVA export, or of an OVF
/// import with a synthetic package.
fn ovf_scene(m: &mut Model, exporting: bool) {
    export(m);
    if exporting {
        let p = &mut m.areas.transfers.panel;
        p.package_export = Some(OvfPackageFormat::Ova);
        p.review = true;
        return;
    }
    let Some(pool) = m.inventory.as_ref().map(|i| i.pool_id) else {
        return;
    };
    panel::begin(m, Some(pool), None);
    let sr = m
        .inventory
        .as_ref()
        .and_then(|i| i.storage.first().map(|v| SrId::from(v.uuid)));
    let mut network = None;
    amend(m, |i| {
        if let Some(sr) = sr {
            let id = PbdId::from(Uuid::from_u128(0x9991));
            i.details.pbds.insert(
                id,
                StorageAttachment {
                    id,
                    location: None,
                    requires_credentials: false,
                    multipathed: None,
                    paths: Vec::new(),
                    host: i.hosts.keys().next().copied(),
                    sr: Some(sr),
                    currently_attached: Some(true),
                },
            );
        }
        network = i.details.networks.keys().next().copied();
    });
    let p = &mut m.areas.transfers.panel;
    p.ovf_import = true;
    p.sr = sr;
    if let Some(network) = network {
        p.networks.insert("Application network".into(), network);
    }
    p.ovf_preview = Some(OvfSummary {
        guests: vec![OvfGuest {
            key: "application".into(),
            name: "Portable application".into(),
            description: "Synthetic guest for layout verification".into(),
            cpus: 4,
            memory_bytes: 4 << 30,
            firmware: GuestFirmware::Uefi,
            disks: vec![OvfAttachment {
                disk: "system".into(),
                device: 0,
            }],
            nics: vec![OvfNic {
                network: "Application network".into(),
                device: 0,
            }],
        }],
        disks: vec![OvfDisk {
            key: "system".into(),
            file: Some("system.vhd".into()),
            virtual_size: 32 << 30,
            format: DiskImageFormat::Vhd,
        }],
        networks: vec!["Application network".into()],
        manifest_verified: true,
    });
    p.review = true;
}

fn disk_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(i) = m.inventory.clone() else { return };
    let Some(d) = i.details.vdis.values().next() else {
        return;
    };
    let (Some(sr), Some(size)) = (d.sr, d.virtual_size) else {
        return;
    };
    panel::begin(m, Some(i.pool_id), None);
    let p = &mut m.areas.transfers.panel;
    p.disk_export = Some((d.id, sr, size));
    p.source_name = d.name.clone();
    p.format = Some(DiskImageFormat::Vhd);
    amend(m, |i| {
        for vm in i.vms.values_mut() {
            vm.power_state = PowerState::Halted;
        }
        for vbd in i.details.vbds.values_mut() {
            vbd.currently_attached = Some(false);
        }
    });
}

pub(crate) fn conversion_scene(m: &mut Model) {
    let p = &mut m.areas.transfers.conversion;
    p.show = true;
    p.preview = true;
    p.source = Some("/home/operator/images/offline-guest.qcow2".into());
    p.destination = Some("/home/operator/images/converted-guest.vhd".into());
    p.output = DiskImageFormat::Vhd;
}

fn conversion_review_scene(m: &mut Model) {
    conversion_scene(m);
    m.areas.transfers.conversion.preview_review = true;
}
