//! Pool-wide VLAN creation and retagging: the Rune `network_topology_ui.rs`
//! (child `topology` of the networking workspace), carried over.
//!
//! The draft ([`Editor`]: the tag, and the uplinks chosen or the VLAN
//! interfaces captured), its validation ([`operation`], [`review`]) and
//! the staleness rule of the review ([`review_unavailable`]) are the Rune
//! ones. The dialog is `network-topology` (`view.rs`); its review and
//! submission are in `edit.rs`. [`sources`] is what the Rune form listed
//! as buttons: the networks a creation can take its uplinks from.

use std::collections::BTreeSet;

use xenadmin_core::{
    Inventory, NetworkId, NetworkTopologyOperation, PoolId, ResourceAction, ResourceTarget,
    ReviewedVlan, VlanUplink,
};

use crate::areas::network::{Check, Workspace, name, pif_name};
use crate::prelude::*;

pub struct Editor {
    pub network: NetworkId,
    pub tag: String,
    pub selection: Selection,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Create,
    Retag,
    Delete,
}

pub enum Selection {
    Create(Vec<VlanUplink>),
    Retag(Vec<ReviewedVlan>),
    Delete(Vec<ReviewedVlan>),
}

/// The staleness rule of a topology review, also of the pool-wide fabric
/// operations, which are topology actions. `submit` makes the same
/// availability check itself and answers `Stale` (Rune).
pub const CHECK: Check = Check {
    reason: review_unavailable,
    conflict: false,
};

/// What `uplinks` refuses a network for; also the reason the creation has
/// nothing to choose from.
pub const NO_UPLINK: &str =
    "Choose an uplink network with exactly one managed untagged interface on every pool host";

/// Why a removal cannot be opened on a network that has no interface.
pub const NO_VLANS: &str = "This network has no VLAN interfaces to remove";

pub fn uplinks(
    i: &Inventory,
    source: NetworkId,
    target: NetworkId,
) -> Result<Vec<VlanUplink>, String> {
    i.hosts
        .keys()
        .map(|host| {
            let candidates = i
                .details
                .pifs
                .values()
                .filter(|p| {
                    p.host == Some(*host)
                        && p.network == Some(source)
                        && p.vlan == Some(-1)
                        && p.managed == Some(true)
                })
                .collect::<Vec<_>>();
            let [p] = candidates.as_slice() else {
                return Err(NO_UPLINK.into());
            };
            VlanUplink::capture(i, p.id, target).map_err(|e| e.to_string())
        })
        .collect()
}

pub fn retag(i: &Inventory, network: NetworkId) -> Result<Vec<ReviewedVlan>, String> {
    let members = i
        .details
        .pifs
        .values()
        .filter(|p| p.network == Some(network));
    members
        .map(|p| {
            let matches = i
                .details
                .vlans
                .values()
                .filter(|v| v.untagged_pif == Some(p.id))
                .collect::<Vec<_>>();
            let [v] = matches.as_slice() else {
                return Err("Every interface on this network must be a VLAN interface".into());
            };
            let parent = v
                .tagged_pif
                .and_then(|id| i.details.pifs.get(&id))
                .ok_or("VLAN uplink is unavailable")?;
            Ok(ReviewedVlan {
                vlan: v.id,
                child: p.id,
                uplink: VlanUplink::capture(i, parent.id, network).map_err(|e| e.to_string())?,
                tag: v
                    .tag
                    .and_then(|v| u16::try_from(v).ok())
                    .ok_or("Current VLAN tag is unavailable")?,
                attached: p
                    .currently_attached
                    .ok_or("Interface attachment state is unavailable")?,
            })
        })
        .collect()
}

/// The draft a command opens (the Rune `open`): the tag "100", and for a
/// retag or removal the VLAN interfaces of the network as they are now.
/// `Err`: why the network cannot be retagged or removed, whatever is entered.
pub fn capture(i: &Inventory, network: NetworkId, mode: Mode) -> Result<Editor, String> {
    Ok(Editor {
        network,
        tag: "100".into(),
        selection: match mode {
            Mode::Create => Selection::Create(vec![]),
            Mode::Retag => Selection::Retag(retag(i, network)?),
            Mode::Delete => {
                let interfaces = retag(i, network)?;
                if interfaces.is_empty() {
                    return Err(NO_VLANS.into());
                }
                Selection::Delete(interfaces)
            }
        },
    })
}

/// The uplink networks a creation can choose from (the Rune buttons of
/// the form): every other network with interfaces for which [`uplinks`]
/// succeeds, with the uplinks it gives.
pub fn sources(i: &Inventory, network: NetworkId) -> Vec<(NetworkId, Vec<VlanUplink>)> {
    candidates(i, network)
        .into_iter()
        .filter_map(|(source, uplinks)| Some((source, uplinks.ok()?)))
        .collect()
}

/// The networks that have one managed untagged interface on every pool host and still
/// cannot be chosen, each with the reason its review cannot be captured. An SR-IOV
/// network is one of these until its hosts are disabled and evacuated; without the
/// reason it would only be missing from the choices.
pub fn unavailable_sources(i: &Inventory, network: NetworkId) -> Vec<(NetworkId, String)> {
    candidates(i, network)
        .into_iter()
        .filter_map(|(source, uplinks)| Some((source, uplinks.err()?)))
        .collect()
}

/// Every other network with one managed untagged interface on each pool host, with its
/// uplinks or the reason their review cannot be captured: what [`sources`] and
/// [`unavailable_sources`] each give one half of.
pub fn candidates(
    i: &Inventory,
    network: NetworkId,
) -> Vec<(NetworkId, Result<Vec<VlanUplink>, String>)> {
    i.details
        .pifs
        .values()
        .filter_map(|p| p.network)
        .filter(|n| *n != network)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|source| (source, uplinks(i, source, network)))
        .filter(|(_, uplinks)| uplinks.as_ref().err().is_none_or(|e| e != NO_UPLINK))
        .collect()
}

pub fn operation(editor: &Editor) -> Result<NetworkTopologyOperation, String> {
    if let Selection::Delete(interfaces) = &editor.selection {
        return Ok(NetworkTopologyOperation::DeleteVlans {
            interfaces: interfaces.clone(),
        });
    }
    let tag = editor
        .tag
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|v| *v <= 4094)
        .ok_or("Enter a VLAN tag from 0 to 4094")?;
    Ok(match &editor.selection {
        Selection::Create(uplinks) => NetworkTopologyOperation::CreateVlans {
            uplinks: uplinks.clone(),
            tag,
        },
        Selection::Retag(interfaces) => NetworkTopologyOperation::RetagVlans {
            interfaces: interfaces.clone(),
            tag,
        },
        Selection::Delete(_) => unreachable!(),
    })
}

pub fn review(
    editor: &Editor,
    i: &Inventory,
) -> Result<(NetworkTopologyOperation, Vec<String>), String> {
    let operation = operation(editor)?;
    let available = operation.availability(i, editor.network);
    if !available.is_available() {
        return Err(available.explanation());
    }
    let mut lines = vec![
        format!("Network: {} ({})", name(i, editor.network), editor.network),
        format!(
            "New VLAN tag: {} · all {} pool hosts",
            editor.tag.trim(),
            i.hosts.len()
        ),
        "Every pool host must be online. Interfaces are changed one host at a time. A failure can leave a partially changed network; verified identities remain on the Events tab of the Notifications view.".into(),
    ];
    if matches!(editor.selection, Selection::Delete(_)) {
        lines[1] = format!(
            "Remove reviewed VLANs from all {} pool hosts",
            i.hosts.len()
        );
    }
    match &editor.selection {
        Selection::Create(uplinks) => {
            lines.push("Create and plug a new VLAN interface on each reviewed uplink. Tag 0 requests the API's native VLAN.".into());
            for p in uplinks {
                lines.push(format!(
                    "{} · host {} · uplink {}",
                    pif_name(i, p.pif),
                    p.expected.host,
                    p.pif
                ));
            }
        }
        Selection::Delete(interfaces) => {
            lines.push("Remove exactly the reviewed VLAN interfaces. The network, logical SR-IOV interfaces and physical NICs remain. No driver activation request is submitted by deletion.".into());
            for v in interfaces {
                lines.push(format!(
                    "Host {} · VLAN {} / interface {} · tag {}",
                    v.uplink.expected.host, v.vlan, v.child, v.tag
                ));
            }
        }
        Selection::Retag(interfaces) => {
            lines.push("Delete each old VLAN interface, create its replacement with the new tag, and restore its plugged or unplugged state. This requires an unused network with no VM attachments, IP configuration, interface purpose, protection or nested topology. No automatic rollback is performed.".into());
            for p in interfaces {
                lines.push(format!(
                    "{} · host {} · uplink {} · old VLAN {} / interface {} · tag {} → {} · {}",
                    pif_name(i, p.uplink.pif),
                    p.uplink.expected.host,
                    p.uplink.pif,
                    p.vlan,
                    p.child,
                    p.tag,
                    editor.tag.trim(),
                    if p.attached {
                        "plugged"
                    } else {
                        "unplugged (preserved)"
                    }
                ));
            }
        }
    }
    let uplinks = match &editor.selection {
        Selection::Create(v) => v.iter().collect::<Vec<_>>(),
        Selection::Retag(v) | Selection::Delete(v) => v.iter().map(|v| &v.uplink).collect(),
    };
    if !matches!(editor.selection, Selection::Delete(_))
        && uplinks.iter().any(|p| p.sriov.is_some())
    {
        lines.insert(
            2,
            "Creating or reattaching a VLAN re-applies its SR-IOV parent configuration. Affected hosts must be disabled and evacuated with HA off. A new reboot requirement leaves a partial result; no reboot is automatic.".into(),
        );
    }
    for p in uplinks.into_iter().filter_map(|p| p.sriov.as_ref()) {
        if let (Some(logical), Some(hardware)) = (p.object.logical, p.fabric.hardware.as_ref()) {
            lines.push(format!(
                "SR-IOV logical interface {} · object {} · physical interface {} · PCI {}",
                logical, p.object.id, p.fabric.input.id, hardware.id
            ));
        }
        for physical in p.fabric.driver_interfaces.values() {
            lines.push(format!(
                "Same-driver interface: {} · {} · MAC {}",
                physical.device, physical.id, physical.mac
            ));
        }
        lines.push(format!(
            "{} other VLAN interfaces are retained with their captured identities and configuration",
            p.preserved.len()
        ));
    }
    lines.push("A lost reply stops the workflow. Recheck inspects existing receipts; it never repeats a request or starts remaining host steps.".into());
    Ok((operation, lines))
}

pub fn review_unavailable(m: &Model, w: &Workspace) -> Option<String> {
    let review = w.review.as_ref()?;
    unavailable_for(m, w.pool, review.target, &review.operation)
}

/// The same rule for a review wherever it is kept: the shared dialog of
/// the `network` area (a pool-wide removal) or a surface of this area.
pub fn unavailable_for(
    m: &Model,
    pool: PoolId,
    target: ResourceTarget,
    operation: &ResourceAction,
) -> Option<String> {
    let ResourceAction::NetworkTopology(op) = operation else {
        return None;
    };
    let ResourceTarget::Network(network) = target else {
        return Some("Network identity changed".into());
    };
    let Some(i) = m.inventory_for(pool) else {
        return Some("The pool changed. Close and reopen this dialog.".into());
    };
    let available = op.availability(&i, network);
    (!available.is_available()).then(|| available.explanation())
}
