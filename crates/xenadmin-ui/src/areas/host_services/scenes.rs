//! The scenes of the area, under their Rune names. Each goes the way a user
//! goes (the demo pool, its first server selected, a command) and then sets
//! what the Rune scene set by hand.

use std::sync::Arc;

use xenadmin_core::{
    HostArchiveOperation, HostBackupSummary, HostCrashdump, HostPowerConfiguration,
    HostPowerSettings, HostSupportCapability, HostSupportOverview, TransferAction, TransferFile,
};

use super::commands::{POWER_ON_SETTINGS, STATUS_REPORT};
use super::power;
use super::support::{self, Mode};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("host-support", "host_services", support_scene),
        SceneSpec::new("host-restore", "host_services", restore_scene),
        SceneSpec::new("host-crashdump-delete", "host_services", crashdump_scene),
        SceneSpec::new("host-power-settings", "host_services", power_scene),
    ]
}

/// The demo pool with its first server selected. Returns that server.
fn server(m: &mut Model) -> Option<Target> {
    m.load_demo(3, 48);
    let inventory = m.inventory.clone()?;
    let host = inventory.hosts.keys().next().copied()?;
    let target = Target::one(ObjectRef::host(inventory.pool_id, host));
    m.select(target.clone());
    Some(target)
}

/// The crashdump the deletion scene reviews.
pub fn scene_dump() -> HostCrashdump {
    HostCrashdump {
        uuid: uuid::Uuid::from_u128(400).into(),
        timestamp: "20261108T00:00:00Z".into(),
        size: 8 * 1024 * 1024 * 1024,
    }
}

/// Rune `host_power_view::scene`: remote power-on is disabled on the first
/// server; an IPMI draft with a synthetic password, reviewed.
pub(crate) fn power_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(mut inventory) = m.inventory.take().map(|i| i.as_ref().clone()) else {
        return;
    };
    let Some(host) = inventory.hosts.keys().next().copied() else {
        return;
    };
    inventory
        .details
        .hosts
        .entry(host)
        .or_default()
        .power_configuration = Some(HostPowerConfiguration {
        settings: HostPowerSettings::Disabled,
        secret: None,
    });
    let target = Target::one(ObjectRef::host(inventory.pool_id, host));
    m.inventory = Some(Arc::new(inventory));
    m.select(target.clone());
    m.invoke(POWER_ON_SETTINGS, &target);
    if let Some(e) = &mut m.areas.host_services.power {
        e.mode = 2;
        e.address = "192.0.2.60".into();
        e.username = "fixture-admin".into();
        e.password.set("synthetic-password-not-shown");
    }
    power::review(m);
}

/// Rune `host_support_view::scene`: the status report of the first server
/// with one advertised category, selected.
pub(crate) fn support_scene(m: &mut Model) {
    let Some(target) = server(m) else { return };
    m.invoke(STATUS_REPORT, &target);
    if let Some(p) = &mut m.areas.host_services.support {
        p.selected.insert("host-crashdump-logs".into());
        p.overview = Some(HostSupportOverview {
            host: p.host,
            version: p.version.clone(),
            capabilities: vec![HostSupportCapability {
                key: "host-crashdump-logs".into(),
                privacy: "yes".into(),
                minimum_bytes: None,
                maximum_bytes: None,
                maximum_seconds: Some(30),
                default_selected: false,
            }],
            capabilities_error: None,
            crashdumps: vec![],
            crashdumps_error: None,
        });
    }
}

/// Rune `host_support_view::restore_scene`: the staging review of a
/// synthetic backup summary.
pub(crate) fn restore_scene(m: &mut Model) {
    support_scene(m);
    support::mode(m, Mode::Restore);
    if let Some(p) = &mut m.areas.host_services.support {
        let archive = HostBackupSummary {
            host: p.host,
            version: p.version.clone(),
            entries: 12042,
            logical_bytes: 3 * 1024 * 1024 * 1024,
            sha256: [0x42; 32],
        };
        p.file =
            TransferFile::destination(std::env::temp_dir().join("example-host-backup.tar.gz")).ok();
        p.review = Some(TransferAction::Host {
            host: p.host,
            expected_version: p.version.clone(),
            operation: HostArchiveOperation::StageRestore { archive },
        });
    }
}

/// Rune `host_support_view::crashdump_scene`: one retained crashdump of
/// 8 GiB, under review for deletion.
pub(crate) fn crashdump_scene(m: &mut Model) {
    support_scene(m);
    let dump = scene_dump();
    if let Some(overview) = m
        .areas
        .host_services
        .support
        .as_mut()
        .and_then(|p| p.overview.as_mut())
    {
        overview.crashdumps = vec![dump.clone()];
    }
    support::review_deletion(m, dump);
}
