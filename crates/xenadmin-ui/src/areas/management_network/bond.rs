//! A bond under IPv4 management, created or removed: the logic of the Rune
//! `management_bond_ui.rs`, carried over. The review needs the exact guest
//! interface impact, which the host is asked for ([`review`]); the answer
//! is accepted only when the inputs and the host observation are still the
//! ones it was asked for ([`poll`], [`inputs_current`]).

use xenadmin_core::{
    BondHash, BondMode, Inventory, ManagementBondOperation as Bond, ManagementInterface,
    ManagementNetworkOperation as Operation, NetworkId, ObservedResource, PifId, PoolId,
    ResourceAction, ResourceTarget,
};

use super::{Form, management};
use crate::areas::network;
use crate::prelude::*;

/// The two Rune errors of `open`, word for word.
pub const INCOMPLETE: &str = "Management interface information is incomplete";
pub const NO_PARENT: &str = "Management VLAN parent is unavailable";

pub struct Editor {
    pub source: ManagementInterface,
    /// The management parent first.
    pub members: Vec<ManagementInterface>,
    pub network: Option<NetworkId>,
    pub mode: BondMode,
    pub hash: BondHash,
}

/// The form of a new management bond, or the Rune error of `open`.
pub fn capture(i: &Inventory, id: PifId) -> Result<Editor, &'static str> {
    let source = ManagementInterface::capture(i, id).ok_or(INCOMPLETE)?;
    let parent = Bond::management_parent(&source)
        .and_then(|id| ManagementInterface::capture(i, id))
        .ok_or(NO_PARENT)?;
    Ok(Editor {
        members: vec![parent],
        source,
        network: None,
        mode: BondMode::ActiveBackup,
        hash: BondHash::SourceMac,
    })
}

pub fn open(m: &mut Model, pool: PoolId, id: PifId) {
    let captured = m
        .inventory_for(pool)
        .ok_or(INCOMPLETE)
        .and_then(|i| capture(&i, id));
    match captured {
        Ok(value) => {
            let host = value.source.identity.host;
            super::open_form(m, pool, host, Form::Bond(Box::new(value)));
        }
        Err(error) => super::report(m, error.into()),
    }
}

/// The open form, for a scene or a test.
pub fn editor(m: &mut Model) -> Option<&mut Editor> {
    match &mut m.areas.management_network.editor.as_mut()?.form {
        Form::Bond(editor) => Some(editor),
        _ => None,
    }
}

pub fn operation(v: &Editor) -> Result<Operation, String> {
    Ok(Operation::Bond {
        consumers: None,
        source: v.source.clone(),
        operation: Bond::Create {
            members: v.members.clone(),
            network: v.network.ok_or("Choose an empty destination network")?,
            mode: v.mode,
            hash: (v.mode == BondMode::Lacp).then_some(v.hash),
        },
    })
}

pub fn lines(op: &Operation, i: &Inventory) -> Vec<String> {
    let name = network::name;
    let pif_name = network::pif_name;
    let mut lines = vec![
        format!("Host: {}", op.source().identity.host),
        format!(
            "Current management: {} ({})",
            pif_name(i, op.source().pif),
            op.source().pif
        ),
        format!(
            "Static IPv4 {} / {} · gateway {} · DNS {}",
            op.source().ipv4.address,
            op.source().ipv4.netmask,
            op.source().ipv4.gateway,
            op.source().ipv4.dns
        ),
    ];
    if let Operation::Bond { operation, .. } = op {
        match operation {
            Bond::Create {
                members,
                network,
                mode,
                hash,
            } => {
                lines.push(format!(
                    "New bond network: {} ({network}) · mode {}{}",
                    name(i, *network),
                    mode.as_str(),
                    hash.map(|h| format!(" · hash {}", h.as_str()))
                        .unwrap_or_default()
                ));
                for m in members {
                    lines.push(format!(
                        "Member: {} ({}) · {} · network {}",
                        pif_name(i, m.pif),
                        m.pif,
                        m.identity.mac,
                        name(i, m.identity.network)
                    ));
                }
                lines.push("The host creates a bond using the reviewed primary physical member’s MAC. It transfers management and the listed VLANs, retaining their network and IPv4 settings. Recreated VLANs receive new interface and VLAN UUIDs.".into());
            }
            Bond::Destroy { bond, primary } => {
                lines.push(format!("Remove bond {bond}; return management or its VLAN to primary physical member {} ({primary})",pif_name(i,*primary)));
                lines.push("The host removes the bond master and returns its members to their original networks. The network objects are retained.".into());
            }
        }
    }
    if let Operation::Bond {
        source, operation, ..
    } = op
    {
        let components = match operation {
            Bond::Create { members, .. } => members
                .iter()
                .filter_map(|m| m.topology.as_deref())
                .collect::<Vec<_>>(),
            Bond::Destroy { .. } => source.topology.as_deref().into_iter().collect(),
        };
        for t in components {
            for v in t.vlans.values() {
                let Some(p) = v.untagged_pif.and_then(|id| t.pifs.get(&id)) else {
                    continue;
                };
                lines.push(format!("Recreate VLAN {} · tag {} · interface {} · network {} ({}) · {} {} / {} · gateway {} · DNS {} · attached {}",v.id,v.tag.unwrap_or_default(),p.id,p.network.map(|id|name(i,id)).unwrap_or_default(),p.network.map(|v|v.to_string()).unwrap_or_default(),p.ip_configuration_mode,p.ip,p.netmask,p.gateway,p.dns,p.currently_attached.unwrap_or(false)));
            }
        }
    }
    if let Operation::Bond {
        consumers: Some(consumers),
        ..
    } = op
    {
        lines.push(format!("Guest interface impact: {} reviewed interfaces. VM power state and NIC identity, MAC, attachment and settings are preserved.",consumers.interfaces.len()));
        lines.push("NICs using network_default inherit their destination network’s default locking policy.".into());
        for (id, n) in &consumers.networks {
            lines.push(format!(
                "Network {} ({id}) · MTU {} · default locking policy {}.",
                n.name,
                n.mtu,
                n.default_locking_mode
                    .as_deref()
                    .unwrap_or("unavailable (no guest consumers)")
            ));
        }
        for v in consumers.interfaces.values() {
            let locality = match &v.locality {
                xenadmin_core::ManagementVmLocality::Resident(host) => {
                    format!("resident on {host}")
                }
                xenadmin_core::ManagementVmLocality::PossibleHosts(hosts) if hosts.is_empty() => {
                    "no eligible host (authoritative server response; treated as local)".into()
                }
                xenadmin_core::ManagementVmLocality::PossibleHosts(hosts) => format!(
                    "eligible hosts: {}",
                    hosts
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            lines.push(format!("{}: {} ({}) · {}{}{} · NIC {} ({}) · MAC {} · attached {} · {} ({}) → {} ({}) · {locality}",v.change.label(),v.vm_name,v.vm,v.power_state,if v.template {" template"}else{""},if v.snapshot {" snapshot"}else{""},v.device,v.interface,v.mac,v.attached,name(i,v.network),v.network,name(i,v.destination),v.destination));
        }
        lines.push("The host moves the reviewed local NICs inside the bond task. Guest traffic may be interrupted. Remote and otherwise eligible guests retain their network. Changes to the consumer set, guest placement or NIC settings require a new review.".into());
    }
    lines.extend(["Traffic and the management connection can be interrupted. Configure the connected switch for the reviewed bond mode first and keep an independent recovery path available.".into(),format!("Recovery endpoint: {} using HTTPS on the current connection port. Success requires the same host certificate, exact resulting topology and address; existing trust settings still apply.",op.recovery_address().map(|v|v.to_string()).unwrap_or_default()),"Lost replies remain unresolved. Recheck reads existing task and topology evidence; no write, rollback or duplicate creation is sent. New bond/master UUIDs are retained when positively identified.".into(),"This workflow supports static IPv4 management and reviewed VLANs with unique tags. VLAN metadata/properties must be empty because the host recreates those objects. Guest interfaces require the captured impact review. Clustering, storage-purpose, tunnel and SR-IOV dependencies require separate workflows.".into()]);
    lines
}

/// The review of an operation whose guest impact is known (or, in a
/// synthetic pool, not asked for): availability, lines, hand-over.
pub fn finish_review(m: &mut Model, pool: PoolId, op: Operation) {
    let Some(i) = m.inventory_for(pool).filter(|i| i.pool_id == pool) else {
        super::report(m, "The pool changed".into());
        return;
    };
    let available = op.availability(&i);
    if !available.is_available() {
        super::report(m, available.explanation());
        return;
    }
    let text = lines(&op, &i);
    super::stage(
        m,
        pool,
        ResourceTarget::Host(op.source().identity.host),
        ResourceAction::ManagementNetwork(Box::new(op)),
        text,
        management::CHECK,
    );
}

pub struct Pending {
    pub query: xenadmin_core::ManagementBondReviewQuery,
    pub observed: ObservedResource,
    pub operation: Operation,
}

/// Asks the host for the guest interfaces the operation touches. In a
/// synthetic pool there is nobody to ask: the review is the one without
/// the impact.
pub fn review(m: &mut Model, op: Operation) {
    let Some(pool) = m.areas.management_network.editor.as_ref().map(|e| e.pool) else {
        return;
    };
    if m.is_synthetic_pool(pool) {
        finish_review(m, pool, op);
        return;
    }
    let result = (|| {
        let observed = m
            .snapshot_for(pool)
            .ok_or("Connect before reviewing guest interfaces")?
            .observe_resource(ResourceTarget::Host(op.source().identity.host))
            .map_err(|e| e.to_string())?;
        let query = m
            .connection_for(pool)
            .ok_or("Connect before reviewing guest interfaces")?
            .review_management_bond(observed, op.clone())
            .map_err(|e| e.to_string())?;
        if let Some(waker) = &m.waker {
            query.notify_on_change(waker.clone());
        }
        Ok::<_, String>(Pending {
            query,
            observed,
            operation: op,
        })
    })();
    match result {
        Ok(pending) => {
            if let Some(e) = &mut m.areas.management_network.editor {
                e.pending = Some(super::Pending::Bond(pending));
                e.error = None;
            }
        }
        Err(error) => super::report(m, error),
    }
}

/// Collects the answer of the pending guest-impact query. `true` when
/// there was one.
pub fn poll(m: &mut Model) -> bool {
    let Some(e) = m.areas.management_network.editor.as_mut() else {
        return false;
    };
    let Some(result) = (match &e.pending {
        Some(super::Pending::Bond(p)) => p.query.latest(),
        _ => None,
    }) else {
        return false;
    };
    let Some(super::Pending::Bond(pending)) = e.pending.take() else {
        return false;
    };
    let pool = e.pool;
    let current = inputs_current(m, &pending.operation)
        && m.snapshot_for(pool)
            .and_then(|v| v.observe_resource(pending.observed.target()).ok())
            == Some(pending.observed);
    if !current {
        super::report(
            m,
            "The bond inputs or connection changed. Review guest interfaces again.".into(),
        );
        return true;
    }
    match result {
        Ok(op) => finish_review(m, pool, op),
        Err(error) => super::report(m, error),
    }
    true
}

/// Whether the open form still asks for exactly the operation a query was
/// started for, in the same pool.
pub fn inputs_current(m: &Model, requested: &Operation) -> bool {
    let Some(e) = m.areas.management_network.editor.as_ref() else {
        return false;
    };
    m.inventory_for(e.pool)
        .is_some_and(|i| i.pool_id == e.pool)
        && match (requested, &e.form) {
            (
                Operation::Bond {
                    operation: Bond::Create { .. },
                    ..
                },
                Form::Bond(v),
            ) => operation(v).is_ok_and(|v| &v == requested),
            // Rune: the form is still `Browse`. Here: the removal that
            // waits is still this one.
            (
                Operation::Bond {
                    operation: Bond::Destroy { .. },
                    ..
                },
                Form::Removal(waiting),
            ) => &**waiting == requested,
            _ => false,
        }
}

/// "Review network change" of the form.
pub fn prepare(m: &mut Model) {
    let Some(result) = editor(m).map(|v| operation(v)) else {
        return;
    };
    match result {
        Ok(op) => review(m, op),
        Err(error) => super::report(m, error),
    }
}

/// The removal of the bond under a management interface, when the bond
/// and its primary member are known.
pub fn removal(i: &Inventory, id: PifId) -> Option<Operation> {
    let source = ManagementInterface::capture(i, id)?;
    let b = source.topology.as_ref().and_then(|t| {
        t.bonds
            .values()
            .find(|b| b.master == Bond::management_parent(&source))
    })?;
    let primary = b.primary_slave?;
    let bond = b.id;
    Some(Operation::Bond {
        consumers: None,
        source,
        operation: Bond::Destroy { bond, primary },
    })
}

/// "Remove management bond…": nothing to enter, straight to the review.
pub fn remove(m: &mut Model, pool: PoolId, id: PifId) {
    let Some(op) = m.inventory_for(pool).and_then(|i| removal(&i, id)) else {
        return;
    };
    let host = op.source().identity.host;
    super::open_form(m, pool, host, Form::Removal(Box::new(op.clone())));
    review(m, op);
}
