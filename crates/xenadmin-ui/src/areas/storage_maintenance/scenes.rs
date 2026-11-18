//! The scenes of the area, under their Rune names.
//!
//! Each starts as the Rune scene did: the demo pool with the attachments
//! the scene needs, which the plain demo inventory does not have. The Rune
//! scenes of `storage_maintenance_view.rs` opened the storage hub on the
//! first repository; here `storage-maintenance` and `storage-repair` run the
//! command `storage.repair` on it, and `storage-cache` opens its Properties
//! dialog on the Read Caching page.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    ClassAvailability, HostId, NfsVersion, PbdId, PoolId, ResourceTarget, SrId, StorageAttachment,
    StorageBackend, StorageLocation,
};

use super::commands::{MULTIPATH_WORKFLOW, REPAIR};
use super::{graph, maintenance};
use crate::prelude::*;

const AREA: &str = "storage_maintenance";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("storage-repair", AREA, |m| repair_scene(m, true)),
        SceneSpec::new("storage-graph", AREA, |m| graph_scene(m, false)),
        SceneSpec::new("storage-multipath", AREA, |m| graph_scene(m, true)),
        SceneSpec::new("storage-maintenance", AREA, |m| repair_scene(m, false)),
        SceneSpec::new("storage-cache", AREA, cache_scene),
    ]
}

fn nfs(path: &str, version: NfsVersion) -> StorageLocation {
    StorageLocation::Nfs {
        server: "storage.example.test".into(),
        path: path.into(),
        version,
    }
}

fn attachment(
    id: u128,
    host: HostId,
    sr: SrId,
    attached: bool,
    location: StorageLocation,
) -> StorageAttachment {
    let id = PbdId::from(Uuid::from_u128(id));
    StorageAttachment {
        id,
        host: Some(host),
        sr: Some(sr),
        currently_attached: Some(attached),
        location: Some(location),
        requires_credentials: false,
        multipathed: None,
        paths: vec![],
    }
}

/// The demo pool where the first repository is NFS and has a detached
/// attachment on each of the first two servers (the Rune
/// `maintenance::scene` before it opened the hub). The pool's Storage tab
/// is shown. Returns the pool and the repository.
pub(super) fn prepare(m: &mut Model) -> Option<(PoolId, SrId)> {
    m.load_demo(3, 48);
    let mut i = m.inventory.take()?.as_ref().clone();
    let sr = i.details.storage.keys().next().copied()?;
    let hosts = i.hosts.keys().copied().take(2).collect::<Vec<_>>();
    for (n, host) in hosts.into_iter().enumerate() {
        let p = attachment(
            950 + n as u128,
            host,
            sr,
            false,
            nfs("/guests", NfsVersion::V4),
        );
        i.details.pbds.insert(p.id, p);
    }
    if let Some(row) = i.storage.iter_mut().find(|r| r.uuid == sr.uuid()) {
        row.kind = "nfs".into();
    }
    for p in i.details.pbds.values_mut().filter(|p| p.sr == Some(sr)) {
        p.currently_attached = Some(false);
    }
    i.details
        .classes
        .insert("pbd".into(), ClassAvailability::Available);
    let pool = i.pool_id;
    m.inventory = Some(Arc::new(i));
    m.select(Target::one(ObjectRef::pool(pool)));
    m.set_active_tab(TabId::Storage);
    Some((pool, sr))
}

/// The Repair dialog with every detached attachment chosen, or its review.
pub(super) fn repair_scene(m: &mut Model, review: bool) {
    let Some((pool, sr)) = prepare(m) else {
        return;
    };
    m.invoke(REPAIR, &Target::one(ObjectRef::storage(pool, sr.uuid())));
    let Some(i) = m.inventory_for(pool) else {
        return;
    };
    let Some(e) = &mut m.areas.storage_maintenance.editor else {
        return;
    };
    e.selection = i
        .details
        .pbds
        .values()
        .filter(|p| p.sr == Some(sr))
        .map(|p| p.id)
        .collect();
    if review {
        maintenance::stage(m);
    }
}

/// What the Rune scene put in place of the driver discovery.
pub(super) fn nfs_backend() -> StorageBackend {
    StorageBackend {
        kind: "nfs".into(),
        name: "NFS".into(),
        features: vec!["VDI_READ_CACHING".into(), "THIN_PROVISIONING".into()],
        configuration: Default::default(),
    }
}

/// The Read Caching page of the repository's Properties dialog, on the
/// review of disabling read caching.
pub(super) fn cache_scene(m: &mut Model) {
    let Some((pool, sr)) = prepare(m) else {
        return;
    };
    let target = Target::one(ObjectRef::storage(pool, sr.uuid()));
    m.select(target.clone());
    m.open_properties(&target, Some(maintenance::PAGE));
    let Some(e) = &mut m.areas.storage_maintenance.editor else {
        return;
    };
    e.backends = vec![nfs_backend()];
    maintenance::stage(m);
}

/// The storage workflow: the reviewed repair of the first repository's
/// connections (one shared NFS connection on the coordinator, the second
/// server chosen), or the reviewed multipath change of the first server
/// (disabled, in maintenance, multipathing off, no resident guests).
pub(super) fn graph_scene(m: &mut Model, multipath: bool) {
    m.load_demo(3, 48);
    let Some(mut i) = m.inventory.take().map(|i| i.as_ref().clone()) else {
        return;
    };
    let hosts = i.hosts.keys().copied().take(2).collect::<Vec<_>>();
    let Some(sr) = i.details.storage.keys().next().copied() else {
        return;
    };
    let [first, second] = hosts[..] else {
        return;
    };
    i.coordinator = Some(first);
    i.details
        .classes
        .insert("pbd".into(), ClassAvailability::Available);
    if let Some(details) = i.details.storage.get_mut(&sr) {
        details.shared = Some(true);
    }
    if let Some(pool) = i.details.pool.as_mut() {
        pool.ha_enabled = Some(false);
    }
    i.details.pbds.clear();
    let p = attachment(
        9950,
        first,
        sr,
        true,
        nfs("/data", NfsVersion::Automatic),
    );
    i.details.pbds.insert(p.id, p);
    if multipath {
        let h = i.details.hosts.entry(first).or_default();
        h.enabled = Some(false);
        h.maintenance_mode = Some(true);
        h.multipathing = Some(false);
        h.standard_multipath_handler = Some(false);
        for v in i.vms.values_mut() {
            v.host = None;
        }
    }
    let pool = i.pool_id;
    m.inventory = Some(Arc::new(i));
    if multipath {
        let target = Target::one(ObjectRef::host(pool, first));
        m.select(target.clone());
        m.invoke(MULTIPATH_WORKFLOW, &target);
    } else {
        m.select(Target::one(ObjectRef::pool(pool)));
        m.set_active_tab(TabId::Storage);
        graph::open(m, pool, ResourceTarget::Storage(sr));
        if let Some(e) = &mut m.areas.storage_maintenance.graph {
            e.hosts.insert(second);
        }
        graph::stage(m);
    }
}
