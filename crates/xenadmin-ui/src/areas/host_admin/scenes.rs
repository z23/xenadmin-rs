//! The scenes of the area, under their Rune names.
//!
//! A scene sets the model up the way a user would have reached the state:
//! the demo pool, a server selected in the tree, a command invoked. The Rune
//! scenes set the editor's fields directly; going through the command shows
//! that the command leads to the same state.

use std::sync::Arc;

use xenadmin_core::{ClassAvailability, HostDetails, HostMetrics, HostMetricsId, VmDetails, VmId};

use super::commands::{CONTROL_DOMAIN_MEMORY, MAINTENANCE};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("host-administration", "host_admin", scene),
        SceneSpec::new("host-memory", "host_admin", memory_scene),
    ]
}

/// The demo pool with what the first server needs for its changes to be
/// reviewable, which the plain demo inventory does not have: live metrics
/// with 64 GiB, a control domain of 2 GiB, the operations it allows, boot
/// and toolstack start times, no power-on mode, a known log destination and
/// multipathing off. That server is selected. Returns it as the target.
pub fn prepare(m: &mut Model) -> Option<Target> {
    m.load_demo(3, 48);
    let mut i = m.inventory.take()?.as_ref().clone();
    let id = *i.hosts.keys().next()?;
    let metrics = HostMetricsId::from(uuid::Uuid::from_u128(0x9901));
    let control = VmId::from(uuid::Uuid::from_u128(0x9902));
    i.details.host_metrics.insert(
        metrics,
        HostMetrics {
            id: metrics,
            live: Some(true),
            memory_total: Some(64 * 1024 * 1024 * 1024),
            memory_free: Some(32 * 1024 * 1024 * 1024),
            last_updated: None,
        },
    );
    i.details.hosts.insert(
        id,
        HostDetails {
            allowed_operations: Some(vec![
                "enable".into(),
                "evacuate".into(),
                "shutdown".into(),
                "reboot".into(),
                "power_on".into(),
            ]),
            current_operations: Some(vec![]),
            maintenance_mode: Some(false),
            boot_time: Some("100".into()),
            agent_start_time: Some("110".into()),
            metrics: Some(metrics),
            control_domain: Some(control),
            power_on_mode: Some(String::new()),
            power_on_config_empty: Some(true),
            syslog_known: true,
            multipathing: Some(false),
            ..Default::default()
        },
    );
    i.details.vms.insert(
        control,
        VmDetails {
            memory_static_min: Some(1024 * 1024),
            memory_static_max: Some(2 * 1024 * 1024 * 1024),
            memory_dynamic_min: Some(2 * 1024 * 1024 * 1024),
            memory_dynamic_max: Some(2 * 1024 * 1024 * 1024),
            ..Default::default()
        },
    );
    i.details
        .classes
        .insert("task".into(), ClassAvailability::Available);
    i.details
        .classes
        .insert("pbd".into(), ClassAvailability::Available);
    let target = Target::one(ObjectRef::host(i.pool_id, id));
    m.inventory = Some(Arc::new(i));
    // What the Rune scene's `Page::Hosts` and its first table row were.
    m.select(target.clone());
    Some(target)
}

/// The review of entering maintenance, with the guests that would move: a
/// supported change, so no experimental notice.
fn scene(m: &mut Model) {
    if let Some(target) = prepare(m) {
        m.invoke(MAINTENANCE, &target);
    }
}

/// The same server with a change of its control domain's memory open: a
/// change outside the supported part.
fn memory_scene(m: &mut Model) {
    if let Some(target) = prepare(m) {
        m.invoke(CONTROL_DOMAIN_MEMORY, &target);
    }
}
