//! Tunnel and SR-IOV interfaces of a network: the Rune `network_fabric_ui.rs`
//! (child `fabric`: one host) and `network_fabric_pool_ui.rs` (child
//! `fabric_pool`: every host of the pool), carried over.
//!
//! - one host: the draft [`Editor`] (kind and input), [`candidates`],
//!   [`review`], the review lines [`lines`], the staleness rule
//!   [`unavailable`];
//! - the records of a network and their removal: [`records`] (the Rune
//!   coverage lines with their "Review removal…" buttons), and the
//!   coverage as information, [`coverage`];
//! - the pool: the draft [`PoolEditor`] (kind and one input per host
//!   without coverage), [`pool_hosts`], [`reviewed`], [`review_lines`],
//!   [`pool_removal`]. A pool operation is a topology action: its
//!   staleness rule is `topology::review_unavailable` (Rune).

use std::collections::BTreeMap;

use xenadmin_core::{
    ClassAvailability, Error, HostId, Inventory, NetworkId, NetworkOperation,
    NetworkTopologyOperation, PifId, PoolId, ResourceAction, ResourceTarget, SdnControllerConfiguration, SriovConfigurationMode,
};
pub use xenadmin_core::{
    NetworkFabricChange as Change, NetworkFabricKind as Kind, NetworkFabricObject as Object,
    NetworkFabricOperation as Operation, NetworkFabricPool, TunnelProtocol,
};

use crate::areas::network::{Check, Staged, Workspace, name, pif_name};
use crate::prelude::*;

/// The kinds, in the order of the Rune buttons.
pub const KINDS: [Kind; 3] = [
    Kind::Tunnel(TunnelProtocol::Gre),
    Kind::Tunnel(TunnelProtocol::Vxlan),
    Kind::Sriov,
];

/// The id of a kind as an option of the Type step.
pub const fn kind_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Tunnel(TunnelProtocol::Gre) => "gre",
        Kind::Tunnel(TunnelProtocol::Vxlan) => "vxlan",
        Kind::Sriov => "sriov",
    }
}

pub fn kind_of(id: &str) -> Option<Kind> {
    KINDS.into_iter().find(|kind| kind_id(*kind) == id)
}

/// The Rune button of a kind on one host, without its ellipsis: the label
/// of the kind in the Type step of the wizard `network-fabric`.
pub const fn add_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Tunnel(TunnelProtocol::Gre) => "Add GRE tunnel interface",
        Kind::Tunnel(TunnelProtocol::Vxlan) => "Add VXLAN tunnel interface",
        Kind::Sriov => "Enable SR-IOV on one host",
    }
}

/// The Rune button of a kind for the pool, likewise (`network-fabric-pool`).
pub const fn complete_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Tunnel(TunnelProtocol::Gre) => "Complete GRE coverage",
        Kind::Tunnel(TunnelProtocol::Vxlan) => "Complete VXLAN coverage",
        Kind::Sriov => "Complete SR-IOV coverage",
    }
}

/// The staleness rule of a host-scoped fabric review. `submit` answers
/// `ResourceConflict` while the reason is there (Rune).
pub const CHECK: Check = Check {
    reason: unavailable,
    conflict: true,
};

// -- One host -----------------------------------------------------------------------

/// The draft of one host: the Rune `fabric::Editor`. The network is the
/// open editor's; the kind is chosen in the Type step.
#[derive(Default)]
pub struct Editor {
    pub kind: Option<Kind>,
    pub input: Option<PifId>,
}

pub fn lines(i: &Inventory, op: &Operation) -> Vec<String> {
    let r = &op.review;
    let mut result = vec![
        format!("Network: {} ({})", name(i, r.network), r.network),
        format!(
            "Host: {} ({})",
            i.hosts
                .get(&r.host)
                .map(|h| h.name.as_str())
                .unwrap_or("Unknown"),
            r.host
        ),
        format!(
            "Input: {} · interface {} · MAC {}",
            r.input.device, r.input.id, r.input.mac
        ),
        format!(
            "{} on this host only. Other pool hosts are not changed.",
            op.label()
        ),
        format!("Target MTU {} · default locking {}", r.mtu, r.locking_mode),
    ];
    match r.kind {
        Kind::Tunnel(protocol) => {
            result.push(format!(
                "Protocol: {} · transport IPv4 {}",
                protocol.as_str(),
                r.input.ip
            ));
            if let Some(controller) = &r.controller {
                result.push(format!(
                    "Configured controller {} · {}",
                    controller.id,
                    controller_description(controller.configuration.as_ref())
                ));
            }
            result.push("The configured controller must supply tunnel peers and keys. Completion verifies the interface configuration; it does not establish traffic reachability. Controller activity is shown separately in coverage.".into());
        }
        Kind::Sriov => {
            if let Some(pci) = &r.hardware {
                result.push(format!(
                    "PCI {} · vendor {} · device {} · driver {}",
                    pci.address.as_deref().unwrap_or("Unknown"),
                    pci.vendor_id.as_deref().unwrap_or("Unknown"),
                    pci.device_id.as_deref().unwrap_or("Unknown"),
                    pci.driver.as_deref().unwrap_or("Unknown")
                ));
            }
            result.push("The disabled, evacuated host must remain live. The NIC driver may be reloaded; every interface using that driver is included below. Host reboot is never submitted by this operation.".into());
            for p in r.driver_interfaces.values() {
                result.push(format!(
                    "Same-driver interface: {} · {} · MAC {}",
                    p.device, p.id, p.mac
                ));
            }
            result.push("If XAPI reports a reboot requirement, review a separate host reboot before expecting VF capacity. Removing manually configured SR-IOV does not necessarily disable hardware VFs.".into());
        }
    }
    match op.change {
        Change::Create => result.push("A newly identified network object and interface UUID will be retained in history. Partial creation is not automatically cleaned up or retried.".into()),
        Change::Delete(object) => {
            // The Rune code unwrapped here: a captured removal names a member.
            if let Some(p) = r.members.values().find(|m| m.object == object) {
                result.push(format!(
                    "Remove exactly object {} and interface {}. The network record and underlying interface remain.",
                    object.uuid(),
                    p.interface.id
                ));
            }
        }
    }
    result.push(format!(
        "Existing target coverage: {} interface(s), {} pool host(s). Other clients can change coverage; the captured identities are rechecked immediately before dispatch.",
        r.members.len(),
        r.hosts.len()
    ));
    result
}

fn controller_description(value: Option<&SdnControllerConfiguration>) -> String {
    match value {
        Some(SdnControllerConfiguration::Ssl { address, port }) => {
            format!("TLS to {address}:{port}")
        }
        Some(SdnControllerConfiguration::PassiveSsl) => "Passive TLS connection".into(),
        None => "Unknown configuration".into(),
    }
}

fn sriov_mode(value: Option<SriovConfigurationMode>) -> &'static str {
    match value {
        Some(SriovConfigurationMode::Sysfs) => "sysfs",
        Some(SriovConfigurationMode::Modprobe) => "driver reload",
        Some(SriovConfigurationMode::Manual) => "manual",
        _ => "unknown",
    }
}

fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

/// An interface a draft can choose as its input (a Rune button of the
/// form): its label, and why it cannot be chosen when it cannot.
pub struct Candidate {
    pub pif: PifId,
    pub label: String,
    pub available: Result<(), String>,
}

/// The inputs of one host's draft: every interface that is not on the
/// network (physical ones only for SR-IOV), with the result of capturing
/// the creation from it.
pub fn candidates(i: &Inventory, network: NetworkId, kind: Kind) -> Vec<Candidate> {
    i.details
        .pifs
        .values()
        .filter(|p| p.network != Some(network) && (kind != Kind::Sriov || p.physical == Some(true)))
        .map(|p| Candidate {
            pif: p.id,
            label: format!("{} · {}", pif_name(i, p.id), p.mac),
            available: Operation::capture(i, network, p.id, kind, Change::Create)
                .map(|_| ())
                .map_err(|error| error.to_string()),
        })
        .collect()
}

pub fn review(network: NetworkId, editor: &Editor, i: &Inventory) -> Result<Staged, String> {
    let kind = editor.kind.ok_or(CHOOSE_TYPE)?;
    let input = editor.input.ok_or("Select an eligible interface")?;
    let op = Operation::capture(i, network, input, kind, Change::Create)
        .map_err(|e| e.to_string())?;
    let lines = lines(i, &op);
    Ok((
        ResourceTarget::Network(network),
        NetworkOperation::Fabric(Box::new(op)),
        lines,
    ))
}

/// The Type step without a choice (the Rune buttons each carried a kind).
pub const CHOOSE_TYPE: &str = "Choose a type.";

pub fn unavailable(m: &Model, w: &Workspace) -> Option<String> {
    unavailable_for(m, w.pool, &w.review.as_ref()?.operation)
}

/// The same rule for a review wherever it is kept.
pub fn unavailable_for(m: &Model, pool: PoolId, operation: &ResourceAction) -> Option<String> {
    let ResourceAction::Network(NetworkOperation::Fabric(op)) = operation else {
        return None;
    };
    let Some(i) = m.inventory_for(pool) else {
        return Some("The connection changed; reopen the network review".into());
    };
    op.availability(&i).err().map(|e| e.to_string())
}

// -- The records of a network, and their removal ------------------------------------

/// A tunnel or SR-IOV record whose interface is on the network: one line
/// of the Rune coverage view.
pub struct Record {
    pub object: Object,
    /// The interface of the record on the network.
    pub pif: PifId,
    /// The Rune coverage line.
    pub line: String,
    /// `None`: Rune offered nothing (the record does not name its input
    /// or protocol). `Some(Err)`: "Removal unavailable: {e}".
    /// `Some(Ok)`: "Review removal…".
    pub removal: Option<Result<Operation, String>>,
}

pub fn records(i: &Inventory, network: NetworkId) -> Vec<Record> {
    let f = &i.details.network_fabric;
    let mut result = Vec::new();
    for p in i
        .details
        .pifs
        .values()
        .filter(|p| p.network == Some(network))
    {
        for t in f.tunnels.values().filter(|t| t.access == Some(p.id)) {
            result.push(Record {
                object: Object::Tunnel(t.id),
                pif: p.id,
                line: format!(
                    "Tunnel {} · interface {} · protocol {} · controller active {} · controller error {}",
                    t.id,
                    p.id,
                    t.protocol.map(TunnelProtocol::as_str).unwrap_or("unknown"),
                    flag(t.controller_active),
                    flag(t.controller_error)
                ),
                removal: t.transport.zip(t.protocol).map(|(input, protocol)| {
                    removal(i, network, input, Kind::Tunnel(protocol), Object::Tunnel(t.id))
                }),
            });
        }
        for v in f.sriovs.values().filter(|v| v.logical == Some(p.id)) {
            result.push(Record {
                object: Object::Sriov(v.id),
                pif: p.id,
                line: format!(
                    "SR-IOV {} · interface {} · mode {} · attached {} · requires reboot {}",
                    v.id,
                    p.id,
                    sriov_mode(v.mode),
                    flag(p.currently_attached),
                    flag(v.requires_reboot)
                ),
                removal: v
                    .physical
                    .map(|input| removal(i, network, input, Kind::Sriov, Object::Sriov(v.id))),
            });
        }
    }
    result
}

fn removal(
    i: &Inventory,
    network: NetworkId,
    input: PifId,
    kind: Kind,
    object: Object,
) -> Result<Operation, String> {
    Operation::capture(i, network, input, kind, Change::Delete(object)).map_err(|e| e.to_string())
}

/// The caption Rune showed in place of "Review removal…".
pub fn removal_unavailable(error: &str) -> String {
    format!("Removal unavailable: {error}")
}

/// The draft of a removal: which record. The Rune view had one button
/// per record; the dialog lists the records and reviews the chosen one.
#[derive(Default)]
pub struct Removal {
    pub chosen: Option<Object>,
}

impl Removal {
    /// The draft the command opens: the record is chosen already when
    /// only one can be removed.
    pub fn capture(i: &Inventory, network: NetworkId) -> Self {
        let removable = records(i, network)
            .into_iter()
            .filter(|record| matches!(record.removal, Some(Ok(_))))
            .map(|record| record.object)
            .collect::<Vec<_>>();
        Self {
            chosen: match removable.as_slice() {
                [only] => Some(*only),
                _ => None,
            },
        }
    }
}

pub fn review_removal(
    network: NetworkId,
    removal: &Removal,
    i: &Inventory,
) -> Result<Staged, String> {
    let chosen = removal.chosen.ok_or(CHOOSE_RECORD)?;
    let records = records(i, network);
    let record = records
        .iter()
        .find(|record| record.object == chosen)
        .ok_or(RECORD_GONE)?;
    let op = match &record.removal {
        Some(Ok(op)) => op.clone(),
        Some(Err(error)) => return Err(removal_unavailable(error)),
        None => return Err(RECORD_GONE.into()),
    };
    let lines = lines(i, &op);
    Ok((
        ResourceTarget::Network(network),
        NetworkOperation::Fabric(Box::new(op)),
        lines,
    ))
}

pub const CHOOSE_RECORD: &str = "Choose the tunnel or SR-IOV interface to remove.";
pub const RECORD_GONE: &str = "The tunnel or SR-IOV record is no longer reported. Reopen the removal.";
/// Why the removal command is disabled on a network without a record
/// whose removal can be reviewed.
pub const NOTHING_TO_REMOVE: &str =
    "This network has no tunnel or SR-IOV interface whose removal can be reviewed.";

/// The Rune `fabric::view` without its buttons: what coverage means, the
/// availability of the four classes it is read from, and per host the
/// records of its interfaces on the network. Information: for the
/// Networking tab of a network; the wizards show it on their Name step.
pub fn coverage(i: &Inventory, network: NetworkId) -> Vec<Block> {
    let mut body = blocks![
        heading("Tunnel and SR-IOV coverage"),
        caption(
            "Coverage is configuration per host. Controller activity and a saved SR-IOV interface are not a traffic or hardware qualification test."
        ),
    ];
    for (class, label) in [
        ("tunnel", "Tunnel records"),
        ("sdn_controller", "Controller settings"),
        ("network_sriov", "SR-IOV records"),
        ("pci", "PCI hardware"),
    ] {
        let status = match i.details.classes.get(class).cloned().unwrap_or_default() {
            ClassAvailability::Available => "available",
            ClassAvailability::Unsupported => "unsupported by this host",
            ClassAvailability::PermissionDenied => "not permitted",
            ClassAvailability::InvalidData => "invalid response",
            ClassAvailability::Unknown => "not yet known",
        };
        body.push(caption(format!("{label}: {status}")).into());
    }
    let records = records(i, network);
    for h in i.hosts.values() {
        let members = i
            .details
            .pifs
            .values()
            .filter(|p| p.host == Some(h.id) && p.network == Some(network))
            .collect::<Vec<_>>();
        body.push(heading(h.name.clone()));
        if members.is_empty()
            && i.details.classes.get("pif") != Some(&ClassAvailability::Available)
        {
            body.push(caption("Coverage unknown: host interface inventory is unavailable").into());
        } else if members.is_empty() {
            body.push(
                caption("No interface reported on this network; this host is not covered.").into(),
            );
        }
        for p in members {
            let mut found = false;
            for record in records.iter().filter(|record| record.pif == p.id) {
                found = true;
                body.push(caption(record.line.clone()).into());
                if let Some(Err(error)) = &record.removal {
                    body.push(caption(removal_unavailable(error)).into());
                }
            }
            if !found {
                body.push(
                    caption(format!(
                        "{} · {} · other or unknown interface kind",
                        p.device, p.id
                    ))
                    .into(),
                );
            }
        }
    }
    body
}

// -- The pool -----------------------------------------------------------------------

/// The draft of the pool: the Rune `fabric_pool::Editor`.
#[derive(Default)]
pub struct PoolEditor {
    pub kind: Option<Kind>,
    pub inputs: BTreeMap<HostId, PifId>,
}

pub fn review_lines(i: &Inventory, op: &NetworkFabricPool) -> Vec<String> {
    let Some(first) = op.operations.first() else {
        return Vec::new();
    };
    let mut result = vec![
        format!(
            "{} · network {} ({})",
            op.label(),
            name(i, first.review.network),
            first.review.network
        ),
        format!(
            "{} host step(s) · {} existing member(s) · {} current pool hosts",
            op.operations.len(),
            first.review.members.len(),
            first.review.hosts.len()
        ),
        "Every current pool host must be online. Steps run serially; creation starts with the coordinator when needed, and removal leaves its interface until last. Existing members are preserved when completing coverage.".into(),
        "A failed or interrupted step stops the remaining hosts. Recheck only reads submitted work; it never repeats a write or starts the remaining steps. Review current coverage again before a new operation.".into(),
        "Completion verifies configuration. Controller activity, inter-host traffic and usable hardware VF capacity remain separate checks. No host reboot is submitted.".into(),
    ];
    for m in first.review.members.values() {
        result.push(format!(
            "Captured existing member: object {} · interface {} · host {}",
            m.object.uuid(),
            m.interface.id,
            m.interface
                .host
                .map(|id| id.to_string())
                .unwrap_or_else(|| "Unknown".into())
        ));
    }
    for (index, item) in op.operations.iter().enumerate() {
        result.push(format!(
            "Step {}: {} · host {} · input {}",
            index + 1,
            item.label(),
            item.review.host,
            item.review.input.id
        ));
        result.extend(lines(i, item).into_iter().filter(|line| {
            !line.contains("this host only") && !line.starts_with("Existing target coverage:")
        }));
    }
    result
}

/// What a host has on the network (the Rune form, per host).
pub enum HostCoverage {
    /// "Existing interface {id} · preserved".
    Existing(Vec<PifId>),
    /// "Missing coverage — select an input": the inputs to choose from.
    Missing(Vec<Candidate>),
}

/// Every host of the pool with its coverage, or its candidate inputs with
/// the reason an input is refused (`NetworkFabricPool::review_input`).
pub fn pool_hosts(
    i: &Inventory,
    network: NetworkId,
    kind: Kind,
) -> Vec<(HostId, String, HostCoverage)> {
    i.hosts
        .values()
        .map(|host| {
            let members = i
                .details
                .pifs
                .values()
                .filter(|p| p.network == Some(network) && p.host == Some(host.id))
                .map(|p| p.id)
                .collect::<Vec<_>>();
            if !members.is_empty() {
                return (host.id, host.name.clone(), HostCoverage::Existing(members));
            }
            let candidates = i
                .details
                .pifs
                .values()
                .filter(|p| {
                    p.host == Some(host.id)
                        && p.network != Some(network)
                        && (kind != Kind::Sriov || p.physical == Some(true))
                })
                .map(|p| Candidate {
                    pif: p.id,
                    label: format!("{} · {}", pif_name(i, p.id), p.mac),
                    available: NetworkFabricPool::review_input(i, network, kind, p.id)
                        .map(|_| ())
                        .map_err(|error| match error {
                            Error::Protocol(message) => message.into(),
                            other => other.to_string(),
                        }),
                })
                .collect();
            (host.id, host.name.clone(), HostCoverage::Missing(candidates))
        })
        .collect()
}

pub fn reviewed(
    network: NetworkId,
    e: &PoolEditor,
    i: &Inventory,
) -> Result<(NetworkFabricPool, Vec<String>), String> {
    let kind = e.kind.ok_or(CHOOSE_TYPE)?;
    let op = NetworkFabricPool::create(i, network, kind, e.inputs.values().copied().collect())
        .map_err(|e| e.to_string())?;
    let lines = review_lines(i, &op);
    Ok((op, lines))
}

/// The pool-wide removal Rune offered for the network: the first kind
/// (GRE, VXLAN, SR-IOV) whose removal the core can build, with its review
/// lines. `None`: Rune showed no button.
pub fn pool_removal(i: &Inventory, network: NetworkId) -> Option<(NetworkFabricPool, Vec<String>)> {
    KINDS.into_iter().find_map(|kind| {
        let op = NetworkFabricPool::remove(i, network, kind).ok()?;
        let lines = review_lines(i, &op);
        Some((op, lines))
    })
}

/// Stages the removal of the pool's coverage and opens its review: the
/// Rune button "{label}…" of `fabric_pool::buttons`, which had nothing to
/// enter. The review is the shared dialog of the `network` area, with the
/// topology rule as its staleness check (a pool operation is a topology
/// action).
pub fn open_pool_removal(m: &mut Model, pool: PoolId, network: NetworkId) {
    let Some(i) = m.inventory_for(pool) else {
        return;
    };
    let Some((operation, lines)) = pool_removal(&i, network) else {
        return;
    };
    super::close(m);
    crate::areas::network::open_review(
        m,
        pool,
        Some(network),
        None,
        ResourceTarget::Network(network),
        ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(Box::new(operation))),
        lines,
        Some(super::topology::CHECK),
    );
}
