//! The staged IPv6 management bond handover: the logic of the Rune
//! `management_ipv6_bond_ui.rs`, carried over. Five stages, each one review
//! and one submission: stage 1 from the form ([`prepare`]), every later
//! stage from the journal record of its predecessor ([`handovers`],
//! [`resume`]), which can also be left partial.

use xenadmin_core::{
    BondMode, Inventory, ManagementBondOperation as Bond, ManagementInterface,
    ManagementIpv6Bond as Operation, NetworkId, ObservedResource, OperationId, OperationState,
    PifId, PoolId, ResourceAction, ResourceTarget,
};

use super::Form;
use crate::areas::network::{self, Check, Workspace};
use crate::prelude::*;

/// The staleness rule of a stage review: the reason disables the confirm
/// button; `submit` makes its own availability check.
pub const CHECK: Check = Check {
    reason: unavailable,
    conflict: false,
};

pub struct Editor {
    pub source: ManagementInterface,
    pub operation: Bond,
    pub temporary: Option<ManagementInterface>,
}

pub struct Pending {
    pub query: xenadmin_core::ManagementIpv6BondReviewQuery,
    pub observed: ObservedResource,
    pub operation: Operation,
}

/// The form for a management interface: removal when its parent is the
/// master of a bond, creation otherwise. `None` where Rune returned
/// silently (incomplete topology).
pub fn capture(i: &Inventory, id: PifId) -> Option<Editor> {
    let source = ManagementInterface::capture(i, id)?;
    let parent = Bond::management_parent(&source)?;
    let operation = if let Some(b) = source
        .topology
        .as_ref()
        .and_then(|t| t.bonds.values().find(|b| b.master == Some(parent)))
    {
        Bond::Destroy {
            bond: b.id,
            primary: b.primary_slave?,
        }
    } else {
        Bond::Create {
            members: vec![ManagementInterface::capture(i, parent)?],
            network: NetworkId::from(uuid::Uuid::nil()),
            mode: BondMode::ActiveBackup,
            hash: None,
        }
    };
    Some(Editor {
        source,
        operation,
        temporary: None,
    })
}

pub fn open(m: &mut Model, pool: PoolId, id: PifId) {
    let Some(value) = m.inventory_for(pool).and_then(|i| capture(&i, id)) else {
        return;
    };
    let host = value.source.identity.host;
    super::open_form(m, pool, host, Form::Ipv6(Box::new(value)));
}

/// The open form, for a scene or a test.
pub fn editor(m: &mut Model) -> Option<&mut Editor> {
    match &mut m.areas.management_network.editor.as_mut()?.form {
        Form::Ipv6(editor) => Some(editor),
        _ => None,
    }
}

pub fn operation(v: &Editor) -> Result<Operation, String> {
    Ok(Operation {
        purpose: Default::default(),
        source: v.source.clone(),
        temporary: v
            .temporary
            .clone()
            .ok_or("Choose the independent static IPv6 recovery interface")?,
        operation: v.operation.clone(),
        consumers: None,
        predecessor: None,
    })
}

pub fn lines(op: &Operation, i: &Inventory, stage: usize) -> Vec<String> {
    let name = network::name;
    let pif_name = network::pif_name;
    if op.purpose == xenadmin_core::ManagementIpv6Next::LeavePartial {
        return vec![format!("Leave the partial configuration after {stage}/5 verified stages. Host {} · exact predecessor {}",op.source.identity.host,op.predecessor.unwrap()),format!("Retained temporary interface {} · {}",op.temporary.pif,op.temporary.ipv6.addresses.join(", ")),"This only releases the local pool reservation. No XAPI calls, rollback or topology completion will be claimed. Temporary addresses and any created bond/VLAN objects remain in place.".into(),"Continue will be permanently disabled for this handover. Inspect the current host and use a new independent review for subsequent changes. An active or uncertain child cannot be abandoned through this action.".into()];
    }
    let mut lines=vec![format!("Host {} · stage {} of 5",op.source.identity.host,stage+1),format!("Original management: {} ({}) · {} · gateway {} · DNS {}",pif_name(i,op.source.pif),op.source.pif,op.source.ipv6.addresses.join(", "),op.source.ipv6.gateway,op.source.ipv6.dns),format!("Temporary management: {} ({}) · network {} · {} · gateway {} · DNS {}",pif_name(i,op.temporary.pif),op.temporary.pif,name(i,op.temporary.identity.network),op.temporary.ipv6.addresses.join(", "),op.temporary.ipv6.gateway,op.temporary.ipv6.dns),"The temporary interface must already be configured and reachable on HTTPS at the current connection port. Its component must be separate from the affected bond/VLAN graph, with no guest or service consumers. Its exact host certificate is checked on the login connection before any write; current CA, pin or skip-validation and proxy policy still apply.".into()];
    match &op.operation {
        Bond::Create {
            members,
            network,
            mode,
            hash,
        } => {
            lines.push(format!(
                "Create bond on {} ({network}) · {}{}",
                name(i, *network),
                mode.as_str(),
                hash.map(|h| format!(" · {}", h.as_str()))
                    .unwrap_or_default()
            ));
            for p in members {
                lines.push(format!(
                    "Member {} ({}) · MAC {} · network {}",
                    pif_name(i, p.pif),
                    p.pif,
                    p.identity.mac,
                    name(i, p.identity.network)
                ));
            }
        }
        Bond::Destroy { bond, primary } => lines.push(format!(
            "Remove bond {bond}; return the reviewed management endpoint/VLAN to primary {} ({primary})",
            pif_name(i, *primary)
        )),
    }
    let components = match &op.operation {
        Bond::Create { members, .. } => members
            .iter()
            .filter_map(|p| p.topology.as_deref())
            .collect::<Vec<_>>(),
        Bond::Destroy { .. } => op.source.topology.as_deref().into_iter().collect(),
    };
    for t in components {
        for vlan in t.vlans.values() {
            if let Some(p) = vlan.untagged_pif.and_then(|id| t.pifs.get(&id)) {
                lines.push(format!(
                    "Recreate VLAN {} · tag {} · PIF {} · network {} · IPv4 {} {} / {} · IPv6 {}",
                    vlan.id,
                    vlan.tag.unwrap_or_default(),
                    p.id,
                    p.network.map(|n| name(i, n)).unwrap_or_default(),
                    p.ip_configuration_mode,
                    p.ip,
                    p.netmask,
                    p.ipv6.join(", ")
                ));
            }
        }
    }
    for (n, label) in [
        "Move management to the verified temporary interface",
        "Clear IPv6 from the original non-management interface",
        "Change the reviewed bond and VLAN topology",
        "Restore the original IPv6 settings on the proven target",
        "Verify the target path and return management",
    ]
    .iter()
    .enumerate()
    {
        lines.push(format!(
            "{}{}. {label}",
            if n == stage { "NEXT: " } else { "" },
            n + 1
        ));
    }
    if let Some(consumers) = &op.consumers {
        for (id, n) in &consumers.networks {
            lines.push(format!(
                "Network {} ({id}) · MTU {} · default locking {}",
                n.name,
                n.mtu,
                n.default_locking_mode.as_deref().unwrap_or("unavailable")
            ));
        }
        for v in consumers.interfaces.values() {
            lines.push(format!(
                "{}: {} ({}) · NIC {} ({}) · MAC {} · {} ({}) → {} ({}) · {:?}",
                v.change.label(),
                v.vm_name,
                v.vm,
                v.device,
                v.interface,
                v.mac,
                name(i, v.network),
                v.network,
                name(i, v.destination),
                v.destination,
                v.locality
            ));
        }
    }
    lines.extend(["Only the displayed next stage is submitted. After it and its authenticated host route are verified, review Continue for the next stage. The pool remains reserved throughout the unfinished handover.".into(),"Management and guest traffic can be interrupted. Configure the connected switch first. During stages 1–4, recover through the temporary address shown above. The temporary interface remains configured after completion.".into(),"Lost, failed or cancelled task receipts remain unresolved; Recheck only reads evidence and never repeats a write. Acknowledging an uncertain result does not authorize continuation.".into(),"This workflow requires one static IPv6 management address and members already using IPv6 as their primary family. Other affected VLANs may use static IPv4 or no IP. HA, clustering, service-purpose, tunnel and SR-IOV dependencies are excluded from this particular handover.".into()]);
    lines
}

/// The review of one stage (or of leaving the handover partial):
/// availability, lines, hand-over.
pub fn finish(m: &mut Model, pool: PoolId, op: Operation, stage: usize) {
    let Some(i) = m.inventory_for(pool).filter(|i| i.pool_id == pool) else {
        return;
    };
    let a = op.availability(&i);
    if !a.is_available() {
        super::report(m, a.explanation());
        return;
    }
    let text = lines(&op, &i, stage);
    super::stage(
        m,
        pool,
        ResourceTarget::Host(op.source.identity.host),
        ResourceAction::ManagementIpv6Bond(Box::new(op)),
        text,
        CHECK,
    );
}

/// "Review network change" of the form: asks the host for the guest
/// impact; in a synthetic pool the review is the one without it.
pub fn prepare(m: &mut Model) {
    let Some(pool) = m.areas.management_network.editor.as_ref().map(|e| e.pool) else {
        return;
    };
    let Some(result) = editor(m).map(|v| operation(v)) else {
        return;
    };
    let op = match result {
        Ok(v) => v,
        Err(e) => {
            super::report(m, e);
            return;
        }
    };
    if m.is_synthetic_pool(pool) {
        finish(m, pool, op, 0);
        return;
    }
    let result = (|| {
        let observed = m
            .snapshot_for(pool)
            .ok_or(xenadmin_core::Error::Stale)?
            .observe_resource(ResourceTarget::Host(op.source.identity.host))?;
        let query = m
            .connection_for(pool)
            .ok_or(xenadmin_core::Error::Closed)?
            .review_management_ipv6_bond(observed, op.clone())?;
        if let Some(w) = &m.waker {
            query.notify_on_change(w.clone());
        }
        Ok::<_, xenadmin_core::Error>(Pending {
            query,
            observed,
            operation: op,
        })
    })();
    match result {
        Ok(v) => {
            if let Some(e) = &mut m.areas.management_network.editor {
                e.pending = Some(super::Pending::Ipv6(v));
                e.error = None;
            }
        }
        Err(e) => super::report(m, e.to_string()),
    }
}

/// Collects the answer of the pending guest-impact query. `true` when
/// there was one.
pub fn poll(m: &mut Model) -> bool {
    let Some(e) = m.areas.management_network.editor.as_mut() else {
        return false;
    };
    let Some(result) = (match &e.pending {
        Some(super::Pending::Ipv6(p)) => p.query.latest(),
        _ => None,
    }) else {
        return false;
    };
    let Some(super::Pending::Ipv6(p)) = e.pending.take() else {
        return false;
    };
    let pool = e.pool;
    let same = m
        .snapshot_for(pool)
        .and_then(|v| v.observe_resource(p.observed.target()).ok())
        == Some(p.observed)
        && m.areas.management_network.editor.as_ref().is_some_and(
            |e| matches!(&e.form, Form::Ipv6(v) if operation(v).as_ref() == Ok(&p.operation)),
        );
    if !same {
        super::report(
            m,
            "The connection or reviewed inputs changed; read the guest impact again".into(),
        );
    } else {
        match result {
            Ok(op) => finish(m, pool, op, 0),
            Err(e) => super::report(m, e),
        }
    }
    true
}

/// Why a reviewed stage cannot be confirmed any more. `None` when the
/// pool changed: the observation rule of the confirm button blocks that.
pub fn unavailable(m: &Model, w: &Workspace) -> Option<String> {
    let r = w.review.as_ref()?;
    let ResourceAction::ManagementIpv6Bond(op) = &r.operation else {
        return None;
    };
    let i = m.inventory_for(w.pool).filter(|i| i.pool_id == w.pool)?;
    let a = op.availability(&i);
    (!a.is_available()).then(|| a.explanation())
}

/// An unfinished handover of a pool: a partially completed record that
/// still holds the pool's reservation and has no successor.
pub struct Handover {
    /// The journal record of the last submitted stage: the predecessor of
    /// what comes next.
    pub record: OperationId,
    /// The stages verified so far; the next one is `stage + 1`.
    pub stage: usize,
    /// The Rune caption of the continuation panel.
    pub caption: String,
    /// The next stage: the recorded operation with this predecessor.
    pub next: Operation,
    /// The same, to leave the configuration partial.
    pub leave: Operation,
}

/// The unfinished handovers of a pool, in the order of the records.
pub fn handovers(m: &Model, pool: PoolId) -> Vec<Handover> {
    m.resource_rows
        .iter()
        .filter(|r| {
            r.pool == pool
                && r.state == OperationState::PartiallyCompleted
                && r.management_ipv6_bond
                    .as_ref()
                    .is_some_and(|v| v.retains_reservation && v.successor.is_none())
        })
        .filter_map(|r| {
            let ResourceAction::ManagementIpv6Bond(op) = &r.action else {
                return None;
            };
            let e = r.management_ipv6_bond.as_ref()?;
            let stage = e.completed_stages;
            let mut next = (**op).clone();
            next.predecessor = Some(r.id);
            let mut leave = next.clone();
            leave.purpose = xenadmin_core::ManagementIpv6Next::LeavePartial;
            Some(Handover {
                record: r.id,
                stage,
                caption: format!(
                    "IPv6 handover: {stage}/5 stages verified. Pool reserved. Temporary recovery: {}. {}",
                    e.temporary_endpoint, r.detail
                ),
                next,
                leave,
            })
        })
        .collect()
}

/// "Review stage {n}…" and "Leave partial configuration…" of one handover:
/// the review of what comes after the record, built from the record as it
/// is now.
pub fn resume(m: &mut Model, pool: PoolId, record: OperationId, leave: bool) {
    let Some(handover) = handovers(m, pool)
        .into_iter()
        .find(|h| h.record == record)
    else {
        return;
    };
    let op = if leave {
        handover.leave
    } else {
        handover.next
    };
    finish(m, pool, op, handover.stage);
}
