//! Read-only SR-IOV remaining VF capacity inspection and driver-impact guard:
//! the Rune `sriov_capacity_ui.rs`, carried over into the `network_services`
//! area.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use xenadmin_core::{
    Inventory, NetworkSriovId, ObservedResource, PifId, PoolId, ResourceTarget, SriovCapacity,
    SriovCapacityQuery,
};

use super::fabric;
use crate::prelude::*;

pub const SRIOV_DRIVER_REVIEW: &str = "SR-IOV attachment changes need a separate driver impact review. Generic plug/unplug is unavailable; pool-wide VLAN creation/retagging includes that review.";

pub struct Observation {
    pub observed: ObservedResource,
    pub query: Option<SriovCapacityQuery>,
    pub result: Option<Result<SriovCapacity, String>>,
}

pub fn refresh(m: &mut Model, pool: PoolId, id: NetworkSriovId) {
    let result = (|| {
        let connection = m
            .connection_for(pool)
            .ok_or("Connect before inspecting VF capacity")?;
        let snapshot = connection.latest();
        let observed = snapshot
            .observe_resource(ResourceTarget::Sriov(id))
            .map_err(|e| e.to_string())?;
        if snapshot.inventory().map(|i| i.pool_id) != Some(pool) {
            return Err("The selected pool changed".into());
        }
        let query = connection
            .inspect_sriov_capacity(observed)
            .map_err(|e| e.to_string())?;
        if let Some(waker) = &m.waker {
            query.notify_on_change(waker.clone());
        }
        Ok::<_, String>(Observation {
            observed,
            query: Some(query),
            result: None,
        })
    })();
    match result {
        Ok(observation) => {
            m.areas
                .network_services
                .capacities
                .insert((pool, id), observation);
        }
        Err(error) => m.error = Some(error),
    }
}

pub fn poll(m: &mut Model, _clock: f64) -> bool {
    let mut changed = false;
    for observation in m.areas.network_services.capacities.values_mut() {
        if let Some(result) = observation.query.as_ref().and_then(|q| q.latest()) {
            observation.result = Some(result);
            observation.query = None;
            changed = true;
        }
    }
    changed
}

pub fn view(m: &Model, pool: PoolId, id: NetworkSriovId) -> Vec<Block> {
    let observation = m.areas.network_services.capacities.get(&(pool, id));
    let current = observation.is_some_and(|o| {
        m.snapshot_for(pool)
            .and_then(|snap| snap.observe_resource(ResourceTarget::Sriov(id)).ok())
            == Some(o.observed)
    });
    let text = match observation {
        Some(_) if !current => {
            "VF capacity observation is stale; refresh after reconnecting.".into()
        }
        Some(o) if o.query.is_some() => "Inspecting remaining VF capacity…".into(),
        Some(o) => match &o.result {
            Some(Ok(r)) => format!(
                "Remaining VFs: {} · observed {} seconds ago · attached {} · requires reboot {}",
                r.remaining_vfs,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .saturating_sub(u128::from(r.observed_at_ms))
                    / 1000,
                fabric::flag(r.attached),
                fabric::flag(r.requires_reboot),
            ),
            Some(Err(e)) => format!("VF capacity unavailable: {e}"),
            None => "VF capacity has not been inspected.".into(),
        },
        None => "VF capacity has not been inspected.".into(),
    };
    let disconnected = m.connection_for(pool).is_none();
    let in_flight = observation.is_some_and(|o| o.query.is_some() && current);
    blocks![
        caption(text),
        caption(
            "This is a read-only observation, not a reservation or allocation guarantee. A pending reboot or detached interface can prevent use even when VFs remain."
        ),
        actions([button(
            format!("sriov-capacity-{id}"),
            "Refresh VF capacity",
            move |m| refresh(m, pool, id),
        )
        .disabled_if(
            disconnected || in_flight,
            if disconnected {
                "Connect before inspecting VF capacity."
            } else {
                "Inspecting remaining VF capacity."
            },
        )]),
    ]
}

pub fn needs_driver_review(i: &Inventory, id: PifId) -> bool {
    let mut related = BTreeSet::from([id]);
    for v in i
        .details
        .vlans
        .values()
        .filter(|v| v.untagged_pif == Some(id))
    {
        related.extend(v.tagged_pif);
    }
    for b in i.details.bonds.values().filter(|b| b.master == Some(id)) {
        related.extend(&b.slaves);
    }
    i.details.network_fabric.sriovs.values().any(|s| {
        s.logical.is_some_and(|p| related.contains(&p))
            || s.physical.is_some_and(|p| related.contains(&p))
    })
}
