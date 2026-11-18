//! The scenes of the area, under their Rune names.
//!
//! The four scenes of the pool start from the demo pool, selected in the
//! tree with its HA tab showing, and reach their state through the command
//! a user would run. The Rune scenes of the join and the password change
//! built their editors from literal values (no connection stands behind
//! them); so do these.

use std::sync::Arc;

use xenadmin_core::{
    ApplianceOrder, ConnectionProfile, CredentialMode, Endpoint, HostId, Inventory, PoolId, SrId,
};

use super::commands::{HA_DISABLE, HA_RESTART_PRIORITY, MAKE_STANDALONE};
use super::{join, policy, root_password, view};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("ha-policy", "pool", ha_policy),
        SceneSpec::new("pool-join", "pool", join_scene),
        SceneSpec::new("root-password", "pool", root_password_scene),
        SceneSpec::new("pool-dissolve", "pool", dissolve),
        SceneSpec::new("pool-policies", "pool", policies),
        SceneSpec::new("pool-policy-review", "pool", policy_review),
    ]
}

/// The demo pool with its inventory amended, selected, on its HA tab.
/// Returns the pool as the target.
pub fn prepare(m: &mut Model, amend: impl FnOnce(&mut Inventory)) -> Option<Target> {
    m.load_demo(3, 48);
    let mut inventory = m.inventory.take()?.as_ref().clone();
    amend(&mut inventory);
    let target = Target::one(ObjectRef::pool(inventory.pool_id));
    m.inventory = Some(Arc::new(inventory));
    m.select(target.clone());
    m.set_active_tab(TabId::Ha);
    Some(target)
}

/// Rune: the pool editor in its edit state. That state is the HA tab now.
fn policies(m: &mut Model) {
    prepare(m, |_| {});
}

/// Rune: the review of Disable HA, which asks for the pool's UUID. The
/// command is offered while HA is on, so the scene's pool has it on, with
/// its first repository as the heartbeat.
fn policy_review(m: &mut Model) {
    let target = prepare(m, |i| {
        let heartbeat: Vec<SrId> = i.storage.first().map(|sr| SrId::from(sr.uuid)).into_iter().collect();
        if let Some(pool) = i.details.pool.as_mut() {
            pool.administration.ha_enabled = Some(true);
            pool.administration.heartbeat_srs = heartbeat;
            pool.administration.heartbeat_srs_known = true;
        }
    });
    if let Some(target) = target {
        m.invoke(HA_DISABLE, &target);
    }
}

/// Rune `ha_policy::scene`: a known timeout default of 60 seconds, a first
/// guest with known vApp data and order 0/0/0, that VM chosen, and the
/// review of its order changed to 2.
fn ha_policy(m: &mut Model) {
    let mut vm = None;
    prepare(m, |i| {
        if let Some(pool) = i.details.pool.as_mut() {
            let a = &mut pool.administration;
            a.ha_timeout_known = true;
            a.ha_timeout_override = Some("60".into());
            a.ha_allow_overcommit = Some(false);
        }
        vm = i.guests().next().map(|vm| vm.id);
        if let Some(d) = vm.and_then(|vm| i.details.vms.get_mut(&vm)) {
            d.appliance_known = true;
            d.appliance = None;
            d.appliance_order = Some(ApplianceOrder {
                order: 0,
                start_delay: 0,
                shutdown_delay: 0,
            });
        }
    });
    let Some(vm) = vm else {
        return;
    };
    view::select_vm_row(m, &vm.to_string());
    let target = m.target();
    m.invoke(HA_RESTART_PRIORITY, &target);
    if let Some(e) = &mut m.areas.pool.editor {
        e.startup[0] = "2".into();
    }
    policy::startup(m, 0);
}

/// Rune `pool_dissolve_view::scene`: the first server is the coordinator,
/// HA is off, the other servers are disabled.
fn dissolve(m: &mut Model) {
    let target = prepare(m, |i| {
        let Some(coordinator) = i.hosts.keys().next().copied() else {
            return;
        };
        if let Some(pool) = i.details.pool.as_mut() {
            pool.administration.coordinator = Some(coordinator);
            pool.administration.ha_enabled = Some(false);
        }
        for host in i.hosts.values_mut() {
            if host.id != coordinator {
                host.enabled = false;
            }
        }
    });
    if let Some(target) = target {
        m.invoke(MAKE_STANDALONE, &target);
    }
}

fn fixture(name: &str, pool: &str, host: &str, address: &str) -> Option<join::Captured> {
    Some(join::Captured {
        profile: ConnectionProfile::new(name.into(), Endpoint::parse(address).ok()?, "root".into()),
        observed: None,
        pool: PoolId::parse(pool).ok()?,
        name: name.into(),
        host: HostId::parse(host).ok()?,
        host_name: format!("{name} host"),
        address: address.into(),
    })
}

/// Rune `profile_pool_join::scene`: the review of one destination, over
/// literal pools.
fn join_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let source = fixture(
        "Standalone lab",
        "11111111-1111-1111-1111-111111111111",
        "22222222-2222-2222-2222-222222222222",
        "192.0.2.10",
    );
    let destination = fixture(
        "Destination lab",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        "192.0.2.20",
    );
    let (Some(source), Some(destination)) = (source, destination) else {
        return;
    };
    m.areas.pool.join = Some(join::Editor {
        source,
        destinations: vec![destination],
        destination: Some(0),
        review: true,
        username: "root".into(),
        password: SecretText::new(),
        confirmation: String::new(),
        error: None,
    });
}

/// Rune `profile_root_password::scene`: the review, over a literal pool of
/// two servers and a keyring profile of `root`.
fn root_password_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let ids = (
        PoolId::parse("11111111-1111-1111-1111-111111111111"),
        HostId::parse("22222222-2222-2222-2222-222222222222"),
        HostId::parse("44444444-4444-4444-4444-444444444444"),
        Endpoint::parse("192.0.2.10"),
    );
    let (Ok(pool), Ok(coordinator), Ok(member), Ok(endpoint)) = ids else {
        return;
    };
    let mut profile = ConnectionProfile::new("Fixture pool".into(), endpoint, "root".into());
    profile.credential_mode = CredentialMode::SecretService;
    let mut editor = root_password::Editor::fixture(
        profile,
        pool,
        "Fixture pool",
        coordinator,
        vec![coordinator, member],
    );
    editor.review = true;
    m.areas.pool.root_password = Some(editor);
}
