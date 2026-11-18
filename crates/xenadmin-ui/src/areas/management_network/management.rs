//! Management IP settings and moving management to another interface: the
//! logic of the Rune `management_network_ui.rs`, carried over. The texts of
//! the review and of the errors are word for word.

use xenadmin_core::{
    Inventory, Ipv4Configuration, Ipv4Mode, Ipv6Configuration, Ipv6Mode, ManagementAddressFamily,
    ManagementDynamicMode, ManagementInterface, ManagementNetworkOperation, PifId, PoolId,
    ResourceAction, ResourceTarget,
};

use super::Form;
use crate::areas::network::{self, Check, Workspace};
use crate::prelude::*;

/// The Rune error of `open`, word for word.
pub const INCOMPLETE: &str = "Management interface identity, IP settings or VLAN/bond topology are incomplete. Bond members cannot be management endpoints.";

/// The staleness rule of a management review (also of a management bond
/// review, which is a `ManagementNetwork` action): the reason disables the
/// confirm button; `submit` makes its own availability check.
pub const CHECK: Check = Check {
    reason: review_unavailable,
    conflict: false,
};

pub struct Editor {
    pub source: ManagementInterface,
    pub moving: bool,
    pub destination: Option<ManagementInterface>,
    pub address: String,
    pub netmask: String,
    pub gateway: String,
    pub dns: String,
    pub automatic: Option<ManagementDynamicMode>,
    pub recovery: String,
}

/// The form a command opens for a management interface, when it can be
/// captured.
pub fn capture(i: &Inventory, id: PifId, moving: bool) -> Option<Editor> {
    let source = ManagementInterface::capture(i, id)?;
    let ipv6 = source.primary == ManagementAddressFamily::Ipv6;
    Some(Editor {
        moving,
        automatic: None,
        recovery: source.address().map(|v| v.to_string()).unwrap_or_default(),
        destination: None,
        address: if ipv6 {
            source.ipv6.addresses.first().cloned().unwrap_or_default()
        } else {
            source.ipv4.address.clone()
        },
        netmask: source.ipv4.netmask.clone(),
        gateway: if ipv6 {
            source.ipv6.gateway.clone()
        } else {
            source.ipv4.gateway.clone()
        },
        dns: source.ipv4.dns.clone(),
        source,
    })
}

pub fn open(m: &mut Model, pool: PoolId, id: PifId, moving: bool) {
    let Some(value) = m
        .inventory_for(pool)
        .and_then(|i| capture(&i, id, moving))
    else {
        super::report(m, INCOMPLETE.into());
        return;
    };
    let host = value.source.identity.host;
    super::open_form(m, pool, host, Form::Management(Box::new(value)));
}

/// The open form, for a scene or a test.
pub fn editor(m: &mut Model) -> Option<&mut Editor> {
    match &mut m.areas.management_network.editor.as_mut()?.form {
        Form::Management(editor) => Some(editor),
        _ => None,
    }
}

pub fn operation(v: &Editor) -> Result<ManagementNetworkOperation, String> {
    if v.moving {
        return Ok(ManagementNetworkOperation::Move {
            source: v.source.clone(),
            destination: v
                .destination
                .clone()
                .ok_or("Choose a configured destination interface")?,
        });
    }
    if let Some(mode) = v.automatic {
        return Ok(ManagementNetworkOperation::ConfigureDynamic {
            interface: v.source.clone(),
            mode,
            recovery: v.recovery.trim().parse().map_err(
                |_| "Enter the expected recovery IP address, without a prefix or hostname",
            )?,
        });
    }
    Ok(match v.source.primary {
        ManagementAddressFamily::Ipv4 => ManagementNetworkOperation::ConfigureIpv4 {
            interface: v.source.clone(),
            configuration: Ipv4Configuration {
                mode: Ipv4Mode::Static,
                address: v.address.trim().into(),
                netmask: v.netmask.trim().into(),
                gateway: v.gateway.trim().into(),
                dns: v.dns.trim().into(),
            },
        },
        ManagementAddressFamily::Ipv6 => ManagementNetworkOperation::ConfigureIpv6 {
            interface: v.source.clone(),
            configuration: Ipv6Configuration {
                mode: Ipv6Mode::Static,
                addresses: vec![v.address.trim().into()],
                gateway: v.gateway.trim().into(),
                dns: v.dns.trim().into(),
            },
        },
    })
}

fn summary(p: &ManagementInterface) -> String {
    match p.primary {
        ManagementAddressFamily::Ipv4 => format!(
            "{} · {} / {} · gateway {} · DNS {}",
            p.ipv4.mode.as_str(),
            p.ipv4.address,
            p.ipv4.netmask,
            p.ipv4.gateway,
            p.ipv4.dns
        ),
        ManagementAddressFamily::Ipv6 => format!(
            "{} · {} · gateway {} · DNS {}",
            p.ipv6.mode.as_str(),
            p.ipv6.addresses.join(", "),
            p.ipv6.gateway,
            p.ipv6.dns
        ),
    }
}

/// The operation of the form and the lines of its review, or why there is
/// none. The form shows the refusal while it is edited; the same function
/// is the gate of the review.
pub fn review(
    v: &Editor,
    i: &Inventory,
) -> Result<(ManagementNetworkOperation, Vec<String>), String> {
    let op = operation(v)?;
    let available = op.availability(i);
    if !available.is_available() {
        return Err(available.explanation());
    }
    let mut lines = vec![
        format!(
            "Host: {} ({})",
            i.hosts
                .get(&v.source.identity.host)
                .map(|h| h.name.as_str())
                .unwrap_or("Unknown"),
            v.source.identity.host
        ),
        format!(
            "Current interface: {} ({})",
            network::pif_name(i, v.source.pif),
            v.source.pif
        ),
        format!(
            "Current {}: {}",
            v.source.primary.as_str(),
            summary(&v.source)
        ),
    ];
    let mut components = std::collections::BTreeSet::new();
    let endpoints = match &op {
        ManagementNetworkOperation::Move {
            source,
            destination,
        } => vec![source, destination],
        _ => vec![op.source()],
    };
    for endpoint in endpoints {
        if let Some(topology) = &endpoint.topology {
            let ids = topology.pifs.keys().copied().collect::<Vec<_>>();
            if !components.insert(ids) {
                continue;
            }
            lines.push(format!("Reviewed topology: {} interfaces, {} bonds, {} VLANs. Membership, configuration and protection are rechecked before the change.",topology.pifs.len(),topology.bonds.len(),topology.vlans.len()));
            for bond in topology.bonds.values() {
                let label = |id| format!("{} ({id})", network::pif_name(i, id));
                lines.push(format!(
                    "Bond {} · {} · master {} · members {}",
                    bond.id,
                    bond.mode,
                    label(bond.master.unwrap()),
                    bond.slaves
                        .iter()
                        .map(|id| label(*id))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for vlan in topology.vlans.values() {
                lines.push(format!(
                    "VLAN {} · tag {} · parent {} · interface {}",
                    vlan.id,
                    vlan.tag.unwrap(),
                    vlan.tagged_pif.unwrap(),
                    vlan.untagged_pif.unwrap()
                ));
            }
            lines.push("Existing bond and VLAN topology is retained. This operation does not create, destroy or retag interfaces, or move the same address between interfaces.".into());
        }
    }
    match &op {
        ManagementNetworkOperation::Bond { .. } => {
            return Err("Review management bonds in the bond editor".into());
        }
        ManagementNetworkOperation::Move { destination, .. } => {
            lines.push(format!(
                "Destination: {} ({}) · network {}",
                network::pif_name(i, destination.pif),
                destination.pif,
                network::name(i, destination.identity.network)
            ));
            lines.push(format!("Destination settings: {}", summary(destination)));
            lines.push("The old interface stays plugged and retains its IP settings. Clear its address separately after verifying access.".into());
        }
        ManagementNetworkOperation::ConfigureIpv4 {
            configuration: c, ..
        } => lines.push(format!(
            "New IPv4: {} / {} · gateway {} · DNS {}",
            c.address, c.netmask, c.gateway, c.dns
        )),
        ManagementNetworkOperation::ConfigureIpv6 {
            configuration: c, ..
        } => lines.push(format!(
            "New IPv6: {} · gateway {} · DNS {}",
            c.addresses.join(", "),
            c.gateway,
            c.dns
        )),
        ManagementNetworkOperation::ConfigureDynamic { mode, recovery, .. } => {
            lines.push(format!(
                "New addressing: {}. Expected assigned address: {recovery}.",
                mode.label()
            ));
            if *mode == ManagementDynamicMode::Ipv6Dhcp {
                lines.push("XCP-ng 8.3 documents incomplete DHCPv6 support. This workflow requires the actual assignment and recovery connection to succeed before reporting completion.".into());
            } else if *mode == ManagementDynamicMode::Ipv6Autoconf {
                lines.push("XCP-ng 8.3 autoconfiguration can replace manually configured DNS. The host supplies the new IPv6 network settings; the other address family's settings must remain unchanged.".into());
            }
            lines.push("The host obtains its address and primary-family network settings automatically. This address must match a DHCP reservation or a known stable autoconfiguration address; entering it does not create a reservation.".into());
            lines.push("If the host obtains a different address, this operation remains unresolved. The client does not scan the network or update DNS aliases. Use an independently verified host address to reconnect and inspect it.".into());
        }
    }
    lines.extend([format!("Recovery address: {} · HTTPS on the current connection port",op.recovery_address().ok_or("Enter a usable recovery address")?),"Management and storage traffic may be interrupted. Have an independent path to the host available. The client checks the new configuration and authenticates the reviewed host at the recovery address.".into(),"Certificate validation follows this connection's existing settings. A new address may need its own trusted certificate; no trust is copied automatically.".into(),"Lost replies remain uncertain. Recheck inspects existing evidence and never repeats the change or rolls it back. Other workspaces are not deliberately disconnected.".into()]);
    Ok((op, lines))
}

/// "Review network change" of the form.
pub fn prepare(m: &mut Model) {
    let result = (|| {
        let e = m
            .areas
            .management_network
            .editor
            .as_ref()
            .ok_or("Close and reopen networking")?;
        let i = m
            .inventory_for(e.pool)
            .filter(|i| i.pool_id == e.pool)
            .ok_or("The pool changed")?;
        let Form::Management(v) = &e.form else {
            return Err("Choose a management change".into());
        };
        review(v, &i).map(|(op, lines)| (e.pool, op, lines))
    })();
    match result {
        Ok((pool, op, lines)) => super::stage(
            m,
            pool,
            ResourceTarget::Host(op.source().identity.host),
            ResourceAction::ManagementNetwork(Box::new(op)),
            lines,
            CHECK,
        ),
        Err(error) => super::report(m, error),
    }
}

/// Why a reviewed management change (or management bond) cannot be
/// confirmed any more.
pub fn review_unavailable(m: &Model, w: &Workspace) -> Option<String> {
    let ResourceAction::ManagementNetwork(op) = &w.review.as_ref()?.operation else {
        return None;
    };
    let Some(i) = m.inventory_for(w.pool).filter(|i| i.pool_id == w.pool) else {
        return Some("The pool changed. Close and reopen networking.".into());
    };
    let available = op.availability(&i);
    (!available.is_available()).then(|| available.explanation())
}
