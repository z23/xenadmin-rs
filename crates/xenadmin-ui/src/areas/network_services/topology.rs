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
    Inventory, NetworkId, NetworkTopologyOperation, PifIdentity, PoolId, ResourceAction,
    ResourceTarget, ReviewedVlan, VlanUplink,
};

use crate::areas::network::{Check, Workspace, name, pif_name};
use crate::prelude::*;

pub struct Editor {
    pub network: NetworkId,
    pub tag: String,
    pub selection: Selection,
}

pub enum Selection {
    Create(Vec<VlanUplink>),
    Retag(Vec<ReviewedVlan>),
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

pub fn uplinks(i: &Inventory, source: NetworkId) -> Result<Vec<VlanUplink>, String> {
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
            Ok(VlanUplink {
                pif: p.id,
                expected: PifIdentity::from_inventory(p).ok_or("Uplink identity is incomplete")?,
            })
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
                uplink: VlanUplink {
                    pif: parent.id,
                    expected: PifIdentity::from_inventory(parent)
                        .ok_or("Uplink identity is incomplete")?,
                },
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
/// retag the VLAN interfaces of the network as they are now. `Err`: why
/// the network cannot be retagged, whatever is entered.
pub fn capture(i: &Inventory, network: NetworkId, changing: bool) -> Result<Editor, String> {
    Ok(Editor {
        network,
        tag: "100".into(),
        selection: if changing {
            Selection::Retag(retag(i, network)?)
        } else {
            Selection::Create(vec![])
        },
    })
}

/// The uplink networks a creation can choose from (the Rune buttons of
/// the form): every other network with interfaces for which [`uplinks`]
/// succeeds, with the uplinks it gives.
pub fn sources(i: &Inventory, network: NetworkId) -> Vec<(NetworkId, Vec<VlanUplink>)> {
    i.details
        .pifs
        .values()
        .filter_map(|p| p.network)
        .filter(|n| *n != network)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|source| Some((source, uplinks(i, source).ok()?)))
        .collect()
}

pub fn operation(editor: &Editor) -> Result<NetworkTopologyOperation, String> {
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
        "Every pool host must be online. Interfaces are changed one host at a time. A failure can leave a partially changed network; verified identities remain in operation history.".into(),
    ];
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
        return Some("The pool changed. Close and reopen networking.".into());
    };
    let available = op.availability(&i, network);
    (!available.is_available()).then(|| available.explanation())
}
