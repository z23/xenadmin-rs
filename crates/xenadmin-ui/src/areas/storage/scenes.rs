//! The scenes of the area, under their Rune names.
//!
//! Each starts as the Rune scene did: the demo pool, its Storage tab (Rune:
//! `Page::Storage`), the wizard opened by its command, then the fields the
//! Rune scene set. A form scene shows the Location step, where the Rune
//! dialog had the fields the scene fills and the Review button.
//!
//! Not here: `storage-maintenance`, `storage-repair` and `storage-cache`.
//! Their Rune functions are wrappers in `storage_view.rs` around
//! `maintenance::scene` of `storage_maintenance_view.rs`: they belong to the
//! `storage_maintenance` area.

use std::collections::BTreeMap;

use xenadmin_core::{
    DiscoveredHbaDevice, DiscoveredIscsiTarget, LocalStorageKind, StorageAllocation,
    StorageBackend, StorageDiscoveryResult,
};

use super::commands::NEW;
use super::editor::{self, Editor};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("storage-create", "storage", |m| scene(m, false)),
        SceneSpec::new("storage-review", "storage", |m| scene(m, true)),
        SceneSpec::new("storage-discovery", "storage", discovery_scene),
        SceneSpec::new("storage-auth-review", "storage", authenticated_scene),
        SceneSpec::new("storage-hba", "storage", hba_scene),
        SceneSpec::new("storage-hba-review", "storage", hba_review_scene),
        SceneSpec::new("storage-local", "storage", |m| local_scene(m, false)),
        SceneSpec::new("storage-local-review", "storage", |m| local_scene(m, true)),
    ]
}

/// The open editor of a scene. Every scene opens the wizard first.
fn editor(m: &mut Model) -> Option<&mut Editor> {
    m.areas.storage.editor.as_mut()
}

/// The NFS creation form of the demo pool, or its review.
pub(super) fn scene(m: &mut Model, review_page: bool) {
    m.load_demo(3, 48);
    let Some(pool) = m.inventory.as_ref().map(|inventory| inventory.pool_id) else {
        return;
    };
    m.select(Target::one(ObjectRef::pool(pool)));
    m.set_active_tab(TabId::Storage);
    let target = m.target();
    m.invoke(NEW, &target);
    if let Some(e) = editor(m) {
        e.step = 2;
        e.name = "Lab NFS storage".into();
        e.description = "Synthetic preview".into();
        e.server = "nfs.example.test".into();
        e.path = "/vm-images".into();
    }
    if review_page {
        editor::review(m);
    }
}

/// The discovery results page: one iSCSI target.
pub(super) fn discovery_scene(m: &mut Model) {
    scene(m, false);
    let Some(e) = editor(m) else {
        return;
    };
    e.kind = 2;
    e.name = "Lab iSCSI storage".into();
    e.server = "storage.example.test".into();
    e.port = "3261".into();
    e.query_inputs = Some(editor::inputs(e));
    e.discovered = Some(StorageDiscoveryResult::IscsiTargets(vec![
        DiscoveredIscsiTarget {
            portal: "192.0.2.50".into(),
            port: 3261,
            iqn: "iqn.2026-11.example:lab-storage".into(),
        },
    ]));
}

/// The review of an SMB ISO library created with credentials.
pub(super) fn authenticated_scene(m: &mut Model) {
    scene(m, false);
    let Some(e) = editor(m) else {
        return;
    };
    e.kind = 3;
    e.authenticate = true;
    e.name = "Lab ISO library".into();
    e.server = "smb.example.test".into();
    e.share = "images".into();
    e.path = "install".into();
    e.username = "LAB\\fixture".into();
    e.password.set("synthetic-only-password");
    editor::review(m);
}

/// The HBA discovery page: thin allocation asked for, a driver that
/// advertises allocation, one device.
pub(super) fn hba_scene(m: &mut Model) {
    scene(m, false);
    let Some(e) = editor(m) else {
        return;
    };
    e.kind = 4;
    e.name = "Lab HBA storage".into();
    e.allocation = StorageAllocation::Thin;
    e.backends.push(StorageBackend {
        kind: "lvmohba".into(),
        name: "LVM over HBA".into(),
        features: vec!["SR_PROBE".into()],
        configuration: BTreeMap::from([("allocation".into(), "thin or thick".into())]),
    });
    e.query_inputs = Some(editor::inputs(e));
    e.discovered = Some(StorageDiscoveryResult::HbaDevices(vec![
        DiscoveredHbaDevice {
            scsi_id: "3600a098038314441455d531234567890".into(),
            size_bytes: 2 * 1024 * 1024 * 1024 * 1024,
            vendor: "Fixture array".into(),
            serial: "LAB-2026-001".into(),
        },
    ]));
}

/// The review of creating on the discovered HBA device, with the typed LUN
/// confirmation.
pub(super) fn hba_review_scene(m: &mut Model) {
    hba_scene(m);
    editor::select_result(m, 0);
    editor::review(m);
}

/// The Local EXT form with a device, the operator's declaration, the
/// acknowledgement and an empty probe; or its review with the typed path
/// confirmation (`storage_local_view.rs`).
pub(super) fn local_scene(m: &mut Model, review_page: bool) {
    scene(m, false);
    let Some(e) = editor(m) else {
        return;
    };
    e.kind = 5;
    e.name = "Lab local EXT".into();
    e.server = String::new();
    e.path = "/dev/disk/by-id/ata-FIXTURE_LAB_DISK_001".into();
    e.declaration = "Fixture lab disk 001, whole disk assigned to local VM storage".into();
    e.local_acknowledged = true;
    e.backends.extend(
        [
            LocalStorageKind::Ext,
            LocalStorageKind::Lvm,
            LocalStorageKind::Xfs,
        ]
        .map(|kind| StorageBackend {
            kind: kind.kind().into(),
            name: kind.label().into(),
            features: vec!["SR_PROBE".into()],
            configuration: BTreeMap::from([("device".into(), "local device path".into())]),
        }),
    );
    e.local_probe = e.host.map(|host| (host, e.path.clone(), true));
    if review_page {
        editor::review(m);
    }
}
