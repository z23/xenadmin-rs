//! The network change that is open: the logic of the Rune `network_admin.rs`
//! and of its child `network_interfaces.rs`, carried over.
//!
//! The Rune `Workspace` was one dialog, "Host networking": a hub (`Browse`)
//! of buttons, a form for each change that has something to enter, and one
//! review and one submission for all of them. The hub is gone (its buttons
//! are commands, `commands.rs`); what stayed is the form, the review, the
//! staleness rule and the submission, word for word:
//!
//! - a change with something to enter is a [`Form`], reviewed by
//!   [`form_review`] ("Review network change");
//! - a change with nothing to enter is staged complete ([`Staged`], the
//!   Rune `review_button`) and opens on its review;
//! - [`submit`] ("Confirm network change") builds the command from the
//!   reviewed [`ResourceAction`], for the four families of actions the Rune
//!   function knew (network, management network, management IPv6 bond,
//!   network topology) and the internal MTU that the generic editor of
//!   `resource_admin.rs` had (merged into the MTU form, see [`Form::Mtu`]).
//!
//! The children of the Rune workspace that are other areas (`service`,
//! `topology`, `fabric`: `network_services`; `management`, `management_bond`,
//! `ipv6_bond`: `management_network`) keep their forms in their own state
//! and bring a complete review here with [`open_review`]: the review, its
//! staleness rule ([`Check`]) and the submission are shared, as in Rune.

use std::sync::Arc;

use xenadmin_core::{
    BondHash, BondId, BondMode, Error, HostId, Inventory, Ipv4Configuration, Ipv4Mode,
    Ipv6Configuration, Ipv6Mode, NetworkId, NetworkOperation, ObservedResource, OperationId,
    PhysicalInterface, PifId, PifIdentity, PoolId, ResourceAction, ResourceCommand,
    ResourceTarget,
};

use crate::prelude::*;

/// The Rune caption of the generic editor for a review whose connection or
/// resource moved on; the networking dialog disabled its confirm button for
/// the same condition without a text.
pub const STALE: &str =
    "The connection or resource changed. Close and reopen this review before applying it.";

/// A further reason a review cannot be confirmed, evaluated on every render
/// and again on submission: the Rune `service::unavailable`,
/// `fabric::unavailable`, `topology::review_unavailable`,
/// `management::review_unavailable` and `ipv6_bond::unavailable`, which
/// belong to the areas that stage those reviews.
#[derive(Clone, Copy)]
pub struct Check {
    pub reason: fn(&Model, &Workspace) -> Option<String>,
    /// `submit` answers `ResourceConflict` when the reason is there (Rune:
    /// the service and fabric reviews). Otherwise the reason only disables
    /// the confirm button and `submit` makes its own availability check.
    pub conflict: bool,
}

/// Where Back leads from a review that another area handed over: that
/// area kept the form the review was made from and shows it again. (Rune:
/// the forms were children of this workspace and Back cleared the review.)
#[derive(Clone, Copy)]
pub struct Back {
    /// The area that staged the review.
    pub area: &'static str,
    /// Runs after this dialog closed: it brings the area's form back.
    pub run: fn(&mut Model),
}

pub struct Review {
    pub target: ResourceTarget,
    pub observed: Option<ObservedResource>,
    pub operation: ResourceAction,
    pub lines: Vec<String>,
    pub check: Option<Check>,
    /// `Some`: the review was handed over with a form behind it.
    pub back: Option<Back>,
}

/// A change with nothing to enter: target, operation and review lines.
pub type Staged = (ResourceTarget, NetworkOperation, Vec<String>);

/// The secondary-interface editors of `network_interfaces.rs`.
pub enum InterfaceEditor {
    Ipv6 {
        id: PifId,
        expected: PifIdentity,
        before: Ipv6Configuration,
        mode: Ipv6Mode,
        address: String,
        gateway: String,
        dns: String,
    },
    Purpose {
        id: PifId,
        expected: PifIdentity,
        before: Option<String>,
        value: String,
    },
}

// One form exists at a time: its size does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Form {
    /// Nothing to enter (Rune `Browse`, without its hub): a staged review,
    /// or a page whose changes are all staged.
    None,
    Vlan {
        parent: Option<PifId>,
        tag: String,
    },
    Bond {
        members: Vec<PifId>,
        mode: BondMode,
        hash: BondHash,
    },
    /// The MTU of the network. Rune had two changes for it: "Change MTU…"
    /// of the networking dialog (`SetMtu`, with the interfaces of the
    /// network) and "Change internal network MTU" of the generic editor
    /// (`ConfigureInternalNetwork`, only for a network without interfaces).
    /// One field now: the review is the internal one when the network has
    /// no interface, the other one when it has.
    Mtu {
        value: String,
    },
    Ipv4 {
        id: PifId,
        identity: PifIdentity,
        before: Ipv4Configuration,
        mode: Ipv4Mode,
        address: String,
        netmask: String,
        gateway: String,
        dns: String,
    },
    Interface(Box<InterfaceEditor>),
    /// Rune: one button "Change to {mode}…" per other mode. The choice of
    /// the mode is the draft; the review is the Rune one.
    BondMode {
        bond: BondId,
        mode: Option<BondMode>,
    },
    /// Rune: one button "Use {hash} hash…" per other hash.
    BondHash {
        bond: BondId,
        hash: Option<BondHash>,
    },
}

/// The page of a Properties dialog a workspace is, when it is one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    /// Network > Network Settings: MTU and automatic attachment.
    NetworkSettings,
    /// Pool > Network Options: IGMP snooping.
    NetworkOptions,
}

impl Page {
    pub const fn id(self) -> &'static str {
        match self {
            Self::NetworkSettings => "network-settings",
            Self::NetworkOptions => "network-options",
        }
    }
}

pub struct Workspace {
    pub pool: PoolId,
    /// The network the change is about (Rune: from the selection, or
    /// chosen in the hub).
    pub network: Option<NetworkId>,
    /// The server whose interfaces are offered (Rune: from the selection).
    pub host: Option<HostId>,
    pub form: Form,
    pub review: Option<Review>,
    pub error: Option<String>,
    /// `Some`: the workspace is that page of a Properties dialog, not a
    /// dialog of its own.
    pub page: Option<Page>,
}

impl Workspace {
    pub fn new(pool: PoolId, network: Option<NetworkId>, host: Option<HostId>, form: Form) -> Self {
        Self {
            pool,
            network,
            host,
            form,
            review: None,
            error: None,
            page: None,
        }
    }
}

/// Opens the dialog of a network change on its form. Any other surface of
/// the area gives way (Rune: `open` cleared `resource_editor`).
pub fn open(
    m: &mut Model,
    pool: PoolId,
    network: Option<NetworkId>,
    host: Option<HostId>,
    form: Form,
) {
    super::close(m);
    m.areas.network.workspace = Some(Workspace::new(pool, network, host, form));
}

/// Opens the dialog on a complete review of a network operation.
pub fn open_staged(
    m: &mut Model,
    pool: PoolId,
    network: Option<NetworkId>,
    host: Option<HostId>,
    (target, operation, lines): Staged,
) {
    open_review(
        m,
        pool,
        network,
        host,
        target,
        ResourceAction::Network(operation),
        lines,
        None,
    );
}

/// Opens the dialog on a complete review of any resource action this
/// workspace can submit. For the areas whose forms were children of the
/// Rune workspace: they review in their own surface and hand the result
/// over. When the observation fails the dialog shows the error instead.
#[allow(clippy::too_many_arguments)]
pub fn open_review(
    m: &mut Model,
    pool: PoolId,
    network: Option<NetworkId>,
    host: Option<HostId>,
    target: ResourceTarget,
    operation: ResourceAction,
    lines: Vec<String>,
    check: Option<Check>,
) {
    open(m, pool, network, host, Form::None);
    prepare_checked(m, target, operation, lines, check);
}

pub fn name(inventory: &Inventory, id: NetworkId) -> String {
    inventory
        .networks
        .iter()
        .find(|n| n.uuid == id.uuid())
        .map(|n| n.name.clone())
        .unwrap_or_else(|| id.to_string())
}

pub fn pif_name(inventory: &Inventory, id: PifId) -> String {
    inventory
        .details
        .pifs
        .get(&id)
        .map(|p| {
            format!(
                "{} · {}",
                p.host
                    .and_then(|id| inventory.hosts.get(&id))
                    .map(|h| h.name.as_str())
                    .unwrap_or("Unknown host"),
                p.device
            )
        })
        .unwrap_or_else(|| id.to_string())
}

/// `network_used` of `resource_admin.rs`: any host or VM interface on it.
pub fn network_used(inventory: &Inventory, id: NetworkId) -> bool {
    inventory
        .details
        .pifs
        .values()
        .any(|interface| interface.network == Some(id))
        || inventory
            .details
            .vifs
            .values()
            .any(|interface| interface.network == Some(id))
}

pub fn edit(m: &mut Model) -> Option<&mut Workspace> {
    m.areas.network.workspace.as_mut()
}

/// Replaces the form and drops the review and the error.
pub fn set_form(m: &mut Model, form: Form) {
    if let Some(w) = edit(m) {
        w.form = form;
        w.review = None;
        w.error = None;
    }
}

pub fn prepare(
    m: &mut Model,
    target: ResourceTarget,
    operation: NetworkOperation,
    lines: Vec<String>,
) {
    prepare_action(m, target, ResourceAction::Network(operation), lines);
}

pub fn prepare_action(
    m: &mut Model,
    target: ResourceTarget,
    operation: ResourceAction,
    lines: Vec<String>,
) {
    prepare_checked(m, target, operation, lines, None);
}

/// Stores the review of the open workspace. In a synthetic pool there is
/// no observation; otherwise the target is observed now, and a failure is
/// the workspace's error.
pub fn prepare_checked(
    m: &mut Model,
    target: ResourceTarget,
    operation: ResourceAction,
    lines: Vec<String>,
    check: Option<Check>,
) {
    let Some(pool) = m.areas.network.workspace.as_ref().map(|w| w.pool) else {
        return;
    };
    let observed = if m.is_synthetic_pool(pool) {
        None
    } else {
        match m
            .snapshot_for(pool)
            .ok_or(Error::Stale)
            .and_then(|snap| snap.observe_resource(target))
        {
            Ok(observed) => Some(observed),
            Err(error) => {
                if let Some(w) = edit(m) {
                    w.error = Some(error.to_string());
                }
                return;
            }
        }
    };
    if let Some(w) = edit(m) {
        w.review = Some(Review {
            target,
            observed,
            operation,
            lines,
            check,
            back: None,
        });
        w.error = None;
    }
}

/// Gives the review that was just handed over ([`open_review`]) a Back:
/// the dialog closes and `back.run` shows the form of the area that staged
/// it. Nothing happens when no review is open (its observation failed).
pub fn review_back(m: &mut Model, back: Back) {
    if let Some(review) = edit(m).and_then(|w| w.review.as_mut()) {
        review.back = Some(back);
    }
}

/// The area whose form is behind the open review, when it was handed over
/// with a Back.
pub fn handed_over_by(m: &Model) -> Option<&'static str> {
    let review = m.areas.network.workspace.as_ref()?.review.as_ref()?;
    review.back.map(|back| back.area)
}

/// Back from the review to what was entered: this workspace's own form,
/// or the form of the area that handed the review over.
pub fn back(m: &mut Model) {
    let handed = edit(m)
        .and_then(|w| w.review.as_ref())
        .and_then(|review| review.back);
    if let Some(back) = handed {
        super::close(m);
        (back.run)(m);
        return;
    }
    if let Some(w) = edit(m) {
        w.review = None;
        w.error = None;
    }
}

pub fn ipv4(pif: &PhysicalInterface) -> Option<Ipv4Configuration> {
    Some(Ipv4Configuration {
        mode: match pif.ip_configuration_mode.as_str() {
            "None" => Ipv4Mode::None,
            "DHCP" => Ipv4Mode::Dhcp,
            "Static" => Ipv4Mode::Static,
            _ => return None,
        },
        address: pif.ip.clone(),
        netmask: pif.netmask.clone(),
        gateway: pif.gateway.clone(),
        dns: pif.dns.clone(),
    })
}

/// The IPv4 form of an interface, or `None` when its identity or its
/// configuration is incomplete (Rune `open_ipv4` returned silently).
pub fn ipv4_form(inventory: &Inventory, id: PifId) -> Option<Form> {
    let pif = inventory.details.pifs.get(&id)?;
    let (identity, before) = (PifIdentity::from_inventory(pif)?, ipv4(pif)?);
    Some(Form::Ipv4 {
        id,
        identity,
        mode: before.mode,
        address: before.address.clone(),
        netmask: before.netmask.clone(),
        gateway: before.gateway.clone(),
        dns: before.dns.clone(),
        before,
    })
}

// -- network_interfaces.rs ------------------------------------------------------

pub fn ipv6(p: &PhysicalInterface) -> Option<Ipv6Configuration> {
    Some(Ipv6Configuration {
        mode: match p.ipv6_configuration_mode.as_str() {
            "None" => Ipv6Mode::None,
            "DHCP" => Ipv6Mode::Dhcp,
            "Autoconf" => Ipv6Mode::Autoconf,
            "Static" => Ipv6Mode::Static,
            _ => return None,
        },
        addresses: p.ipv6.clone(),
        gateway: p.ipv6_gateway.clone(),
        dns: p.dns.clone(),
    })
}

pub fn host_ipv6(i: &Inventory, p: &PhysicalInterface) -> bool {
    i.details.pifs.values().any(|m| {
        m.host == p.host
            && m.management == Some(true)
            && matches!(
                m.ipv6_configuration_mode.as_str(),
                "Static" | "DHCP" | "Autoconf"
            )
    })
}

/// The IPv6 or the purpose form of an interface (Rune `interfaces::open`),
/// or `None` when its identity or configuration is incomplete.
pub fn interface_form(inventory: &Inventory, id: PifId, purpose: bool) -> Option<Form> {
    let p = inventory.details.pifs.get(&id)?;
    let expected = PifIdentity::from_inventory(p)?;
    let editor = if purpose {
        InterfaceEditor::Purpose {
            id,
            expected,
            before: p.management_purpose.clone(),
            value: p.management_purpose.clone().unwrap_or_default(),
        }
    } else {
        let before = ipv6(p)?;
        InterfaceEditor::Ipv6 {
            id,
            expected,
            mode: before.mode,
            address: before.addresses.first().cloned().unwrap_or_default(),
            gateway: before.gateway.clone(),
            dns: before.dns.clone(),
            before,
        }
    };
    Some(Form::Interface(Box::new(editor)))
}

pub fn interface_review(editor: &InterfaceEditor, i: &Inventory) -> Result<Staged, String> {
    let (id, expected) = match editor {
        InterfaceEditor::Ipv6 { id, expected, .. }
        | InterfaceEditor::Purpose { id, expected, .. } => (*id, expected),
    };
    let p = i.details.pifs.get(&id).ok_or("The interface disappeared")?;
    if PifIdentity::from_inventory(p).as_ref() != Some(expected)
        || p.management != Some(false)
        || p.managed != Some(true)
    {
        return Err("The interface identity or management role changed; reopen the editor".into());
    }
    let mut lines = vec![format!("Interface: {} ({id})", pif_name(i, id))];
    let operation = match editor {
        InterfaceEditor::Ipv6 {
            before,
            mode,
            address,
            gateway,
            dns,
            ..
        } => {
            if ipv6(p).as_ref() != Some(before) || !host_ipv6(i, p) {
                return Err(
                    "The IPv6 configuration changed or is unavailable; reopen the editor".into(),
                );
            }
            let configuration = Ipv6Configuration {
                mode: *mode,
                addresses: if *mode == Ipv6Mode::Static {
                    vec![address.trim().into()]
                } else {
                    vec![]
                },
                gateway: if *mode == Ipv6Mode::Static {
                    gateway.trim().into()
                } else {
                    String::new()
                },
                dns: dns.trim().into(),
            };
            if !configuration.valid() || &configuration == before {
                return Err("Enter changed valid IPv6 settings: one address/prefix for Static and comma-separated DNS addresses".into());
            }
            lines.extend([
                format!(
                    "Previous: {} {} · gateway {} · DNS {}",
                    before.mode.as_str(),
                    before.addresses.join(", "),
                    before.gateway,
                    before.dns
                ),
                format!(
                    "New: {} {} · gateway {} · DNS {}",
                    mode.as_str(),
                    configuration.addresses.join(", "),
                    configuration.gateway,
                    configuration.dns
                ),
                "Traffic using this interface may be interrupted. DNS is shared with the interface's IPv4 configuration. This change retains the primary management interface and installed address family.".into(),
            ]);
            NetworkOperation::ConfigureIpv6 {
                expected: expected.clone(),
                before: before.clone(),
                configuration,
            }
        }
        InterfaceEditor::Purpose { before, value, .. } => {
            if &p.management_purpose != before {
                return Err("The interface purpose changed; reopen the editor".into());
            }
            let purpose = (!value.trim().is_empty()).then(|| value.trim().to_owned());
            if &purpose == before
                || purpose
                    .as_ref()
                    .is_some_and(|v| v.len() > 256 || v.chars().any(char::is_control))
            {
                return Err("Enter a changed purpose of at most 256 bytes, or clear it".into());
            }
            lines.extend([
                format!(
                    "Purpose: {} → {}",
                    before.as_deref().unwrap_or("Not set"),
                    purpose.as_deref().unwrap_or("Not set")
                ),
                "Purpose labels describe secondary management or storage interfaces. Changing the label does not change routing, IP configuration or unplug protection.".into(),
            ]);
            NetworkOperation::SetInterfacePurpose {
                expected: expected.clone(),
                before: before.clone(),
                purpose,
            }
        }
    };
    Ok((ResourceTarget::Pif(id), operation, lines))
}

// -- The changes with nothing to enter (Rune `review_button` call sites) ---------

pub fn scan(i: &Inventory, host: HostId) -> Option<Staged> {
    let host = i.hosts.get(&host)?;
    Some((
        ResourceTarget::Host(host.id),
        NetworkOperation::ScanInterfaces,
        vec![
            format!("Host: {} ({})", host.name, host.id),
            "Refresh physical interface discovery on this host.".into(),
        ],
    ))
}

/// Only when the pool reports its IGMP snooping state.
pub fn igmp(i: &Inventory) -> Option<Staged> {
    let igmp = i.details.pool.as_ref()?.igmp_snooping_enabled?;
    Some((
        ResourceTarget::Pool(i.pool_id),
        NetworkOperation::SetIgmpSnooping {
            expected: igmp,
            enabled: !igmp,
        },
        vec![
            format!("Pool: {} ({})", i.pool_name, i.pool_id),
            format!("IGMP snooping: {} → {}", igmp, !igmp),
            "This changes multicast forwarding behavior across the pool.".into(),
        ],
    ))
}

/// Only when the network reports its automatic attachment.
pub fn automatic(i: &Inventory, id: NetworkId) -> Option<Staged> {
    let n = i.details.networks.get(&id).filter(|n| n.automatic_known)?;
    let automatic = n.automatic.as_deref() != Some("false");
    Some((
        ResourceTarget::Network(id),
        NetworkOperation::SetAutomaticAttachment {
            expected: n.automatic.clone(),
            enabled: !automatic,
        },
        vec![
            format!("Network: {} ({id})", name(i, id)),
            format!(
                "Automatically attach this network to new VMs: {}",
                !automatic
            ),
        ],
    ))
}

/// Plug or unplug: only when identity and attachment state are known.
pub fn attach(i: &Inventory, p: &PhysicalInterface) -> Option<Staged> {
    let (expected, attached) = (PifIdentity::from_inventory(p)?, p.currently_attached?);
    Some((
        ResourceTarget::Pif(p.id),
        NetworkOperation::SetAttached {
            expected,
            expected_attached: attached,
            attached: !attached,
        },
        vec![
            format!("Interface: {} ({})", pif_name(i, p.id), p.id),
            format!("Attached: {attached} → {}", !attached),
            "Unplugging interrupts VM and storage traffic on this interface.".into(),
        ],
    ))
}

/// The Rune rule for the plug button: plugging always, unplugging only an
/// interface that is not the management interface and is not protected.
pub fn may_change_attachment(p: &PhysicalInterface) -> bool {
    p.currently_attached == Some(false)
        || (p.management == Some(false) && p.disallow_unplug != Some(true))
}

/// The Rune `allowed` of `network_interfaces.rs`, without the connection.
pub fn secondary(p: &PhysicalInterface) -> bool {
    p.managed == Some(true) && p.management == Some(false)
}

pub fn protection(i: &Inventory, p: &PhysicalInterface) -> Option<Staged> {
    let (expected, before) = (PifIdentity::from_inventory(p)?, p.disallow_unplug?);
    let id = p.id;
    Some((
        ResourceTarget::Pif(id),
        NetworkOperation::SetUnplugProtection {
            expected,
            before,
            enabled: !before,
        },
        vec![
            format!("Interface: {} ({id})", pif_name(i, id)),
            format!("Unplug protection: {before} → {}", !before),
            "Protection prevents XAPI from unplugging this interface. Disabling it permits later unplug operations; it does not unplug now.".into(),
        ],
    ))
}

/// The VLAN whose interface this is, when everything the deletion names is
/// known.
pub fn delete_vlan(i: &Inventory, p: &PhysicalInterface) -> Option<Staged> {
    i.details
        .vlans
        .values()
        .filter(|v| v.untagged_pif == Some(p.id))
        .find_map(|vlan| {
            let (parent, tag, network) = (
                vlan.tagged_pif?,
                vlan.tag.and_then(|t| u16::try_from(t).ok())?,
                p.network?,
            );
            Some((
                ResourceTarget::Vlan(vlan.id),
                NetworkOperation::DeleteVlan {
                    parent,
                    child: p.id,
                    network,
                    tag,
                },
                vec![
                    format!("VLAN {} · tag {tag}", vlan.id),
                    format!("Child interface: {}", p.id),
                    "Deletes this host's VLAN interface. The network record and other hosts' interfaces remain. Attached VMs must be moved first.".into(),
                ],
            ))
        })
}

/// The bond whose master interface this is.
pub fn bond_of(i: &Inventory, pif: PifId) -> Option<&xenadmin_core::NetworkBond> {
    i.details.bonds.values().find(|b| b.master == Some(pif))
}

pub fn delete_bond(i: &Inventory, p: &PhysicalInterface) -> Option<Staged> {
    let bond = bond_of(i, p.id)?;
    let primary = bond.primary_slave?;
    Some((
        ResourceTarget::Bond(bond.id),
        NetworkOperation::DeleteBond {
            master: p.id,
            members: bond.slaves.clone(),
            primary,
        },
        vec![
            format!("Bond {}", bond.id),
            format!("Primary member: {} ({primary})", pif_name(i, primary)),
            "The host returns the bond configuration to its primary member. This workflow requires VM, VLAN and IP users to be moved first.".into(),
        ],
    ))
}

pub const BOND_MODES: [BondMode; 3] = [BondMode::BalanceSlb, BondMode::ActiveBackup, BondMode::Lacp];
pub const BOND_HASHES: [BondHash; 2] = [BondHash::SourceMac, BondHash::TcpUdpPorts];

/// The modes the bond can change to: every one but its own.
pub fn other_modes(bond: &xenadmin_core::NetworkBond) -> Vec<BondMode> {
    BOND_MODES
        .into_iter()
        .filter(|mode| bond.mode != mode.as_str())
        .collect()
}

/// The hashes an LACP bond can change to; none for another mode or while
/// the current hash is not known.
pub fn other_hashes(bond: &xenadmin_core::NetworkBond) -> Vec<BondHash> {
    if bond.mode != "lacp" || !bond.hashing_algorithm_known {
        return Vec::new();
    }
    BOND_HASHES
        .into_iter()
        .filter(|hash| bond.hashing_algorithm.as_deref() != Some(hash.as_str()))
        .collect()
}

// -- Review and submission ----------------------------------------------------------

/// "Review network change": validates the form against the inventory of
/// the workspace's pool and stores the review, or the reason there is none.
pub fn form_review(m: &mut Model) {
    let result = (|| -> Result<(ResourceTarget, ResourceAction, Vec<String>), String> {
        let w = m
            .areas
            .network
            .workspace
            .as_ref()
            .ok_or("Close and reopen this dialog")?;
        let i = m.inventory_for(w.pool).ok_or("The pool changed")?;
        let i = &*i;
        let network_op = |(target, operation, lines): Staged| {
            (target, ResourceAction::Network(operation), lines)
        };
        Ok(match &w.form {
            Form::Vlan { parent, tag } => {
                let network = w.network.ok_or("Choose the destination network")?;
                let parent = parent.ok_or("Select a physical interface")?;
                let expected = PifIdentity::from_inventory(
                    i.details.pifs.get(&parent).ok_or("Interface disappeared")?,
                )
                .ok_or("Interface identity is incomplete")?;
                let tag = tag
                    .parse::<u16>()
                    .ok()
                    .filter(|v| (0..=4094).contains(v))
                    .ok_or("Enter a VLAN tag from 0 to 4094")?;
                network_op((
                    ResourceTarget::Network(network),
                    NetworkOperation::CreateVlan {
                        parent,
                        expected,
                        tag,
                    },
                    vec![
                        format!("Destination: {} ({network})", name(i, network)),
                        format!("Parent: {} ({parent})", pif_name(i, parent)),
                        format!("VLAN tag: {tag}"),
                        "Creates an interface on this host. Repeat on other hosts that need this network.".into(),
                    ],
                ))
            }
            Form::Bond {
                members,
                mode,
                hash,
            } => {
                if !(2..=4).contains(&members.len()) {
                    return Err("Select two to four physical interfaces on one host".into());
                }
                let network = w.network.ok_or("Choose the destination network")?;
                let members = members
                    .iter()
                    .map(|id| {
                        PifIdentity::from_inventory(
                            i.details.pifs.get(id).ok_or("Interface disappeared")?,
                        )
                        .map(|identity| (*id, identity))
                        .ok_or("Interface identity is incomplete")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if members.iter().any(|(_, p)| p.host != members[0].1.host) {
                    return Err("All bond members must be on the same host".into());
                }
                let mut lines = vec![
                    format!("Destination: {} ({network})", name(i, network)),
                    format!("Mode: {} · hash: {}", mode.as_str(), hash.as_str()),
                    "The host will inherit the primary member's MAC. Active IP, VLAN and VM attachments must be moved separately before creating this bond.".into(),
                ];
                lines.extend(
                    members
                        .iter()
                        .map(|(id, _)| format!("Member: {} ({id})", pif_name(i, *id))),
                );
                network_op((
                    ResourceTarget::Network(network),
                    NetworkOperation::CreateBond {
                        members,
                        mode: *mode,
                        hash: *hash,
                    },
                    lines,
                ))
            }
            Form::Mtu { value } => {
                let network = w.network.ok_or("Choose a network")?;
                let current = i.details.networks.get(&network).and_then(|n| n.mtu);
                if network_used(i, network) {
                    // `network_admin.rs`: the network and its interfaces.
                    let expected = current.ok_or("Current MTU is unavailable")?;
                    let mtu = value
                        .parse::<u64>()
                        .ok()
                        .filter(|v| (576..=65_535).contains(v))
                        .ok_or("Enter an MTU from 576 to 65535")?;
                    if expected == mtu {
                        return Err("Choose a different MTU".into());
                    }
                    let pifs = i
                        .details
                        .pifs
                        .values()
                        .filter(|p| p.network == Some(network))
                        .map(|p| p.id)
                        .collect();
                    network_op((
                        ResourceTarget::Network(network),
                        NetworkOperation::SetMtu {
                            expected,
                            mtu,
                            pifs,
                        },
                        vec![
                            format!("Network: {} ({network})", name(i, network)),
                            format!("MTU: {expected} → {mtu}"),
                            "Traffic may be interrupted. Confirm that the physical path supports the new MTU. Management interfaces are not changed by this workflow.".into(),
                        ],
                    ))
                } else {
                    // `resource_admin.rs`, `Kind::Mtu`: an unused internal
                    // network.
                    let mtu = value
                        .parse::<u64>()
                        .ok()
                        .filter(|n| (576..=65535).contains(n))
                        .ok_or("Enter an MTU between 576 and 65535")?;
                    if current == Some(mtu) {
                        return Err("The MTU is unchanged.".into());
                    }
                    let expected_mtu = current.ok_or("The current MTU is unknown. Refresh first.")?;
                    (
                        ResourceTarget::Network(network),
                        ResourceAction::ConfigureInternalNetwork { expected_mtu, mtu },
                        vec![
                            format!("MTU: {mtu}"),
                            "Changing MTU requires an unused internal network; attached physical and VM interfaces must be absent.".into(),
                        ],
                    )
                }
            }
            Form::Ipv4 {
                id,
                identity,
                before,
                mode,
                address,
                netmask,
                gateway,
                dns,
            } => {
                let fixed = |value: &String| {
                    if *mode == Ipv4Mode::Static {
                        value.trim().into()
                    } else {
                        String::new()
                    }
                };
                let configuration = Ipv4Configuration {
                    mode: *mode,
                    address: fixed(address),
                    netmask: fixed(netmask),
                    gateway: fixed(gateway),
                    dns: dns.trim().into(),
                };
                if !configuration.valid() || before == &configuration {
                    return Err("Enter valid changed IP settings, a contiguous netmask and comma-separated DNS addresses".into());
                }
                let lines = vec![
                    format!("Interface: {} ({id})", pif_name(i, *id)),
                    format!(
                        "Previous: {} {} / {} gateway {} DNS {}",
                        before.mode.as_str(),
                        before.address,
                        before.netmask,
                        before.gateway,
                        before.dns
                    ),
                    format!(
                        "New: {} {} / {} gateway {} DNS {}",
                        mode.as_str(),
                        configuration.address,
                        configuration.netmask,
                        configuration.gateway,
                        configuration.dns
                    ),
                    "Storage or other traffic using this interface may be interrupted. This does not change the host management interface.".into(),
                ];
                network_op((
                    ResourceTarget::Pif(*id),
                    NetworkOperation::ConfigureIpv4 {
                        expected: identity.clone(),
                        before: before.clone(),
                        configuration,
                    },
                    lines,
                ))
            }
            Form::Interface(editor) => network_op(interface_review(editor, i)?),
            Form::BondMode { bond, mode } => {
                let bond = i
                    .details
                    .bonds
                    .values()
                    .find(|b| b.id == *bond)
                    .ok_or("Interface disappeared")?;
                let mode = mode
                    .filter(|mode| bond.mode != mode.as_str())
                    .ok_or("Choose a change to review")?;
                network_op((
                    ResourceTarget::Bond(bond.id),
                    NetworkOperation::SetBondMode {
                        expected_mode: bond.mode.clone(),
                        mode,
                    },
                    vec![
                        format!("Bond {}: {} → {}", bond.id, bond.mode, mode.as_str()),
                        "Coordinate switch configuration first. Network traffic may be interrupted.".into(),
                    ],
                ))
            }
            Form::BondHash { bond, hash } => {
                let bond = i
                    .details
                    .bonds
                    .values()
                    .find(|b| b.id == *bond)
                    .ok_or("Interface disappeared")?;
                let hash = hash
                    .filter(|hash| other_hashes(bond).contains(hash))
                    .ok_or("Choose a change to review")?;
                network_op((
                    ResourceTarget::Bond(bond.id),
                    NetworkOperation::SetBondHash {
                        expected: bond.hashing_algorithm.clone(),
                        hash,
                    },
                    vec![
                        format!("Bond {} hash → {}", bond.id, hash.as_str()),
                        "Traffic distribution changes immediately.".into(),
                    ],
                ))
            }
            Form::None => {
                return Err("Choose a change to review".into());
            }
        })
    })();
    match result {
        Ok((target, operation, lines)) => prepare_action(m, target, operation, lines),
        Err(error) => {
            if let Some(w) = edit(m) {
                w.error = Some(error);
            }
        }
    }
}

/// Whether the workspace's pool is a synthetic one: reviews are previews.
pub fn synthetic(m: &Model, w: &Workspace) -> bool {
    m.is_synthetic_pool(w.pool)
}

/// The Rune rule of the confirm button for the connection: a writable
/// current connection, and the reviewed target observed as it was.
pub fn current(m: &Model, w: &Workspace) -> bool {
    m.can_mutate_pool(w.pool)
        && w.review
            .as_ref()
            .and_then(|r| r.observed)
            .is_some_and(|observed| {
                m.snapshot_for(w.pool)
                    .and_then(|snap| snap.observe_resource(observed.target()).ok())
                    == Some(observed)
            })
}

/// Why the open review cannot be confirmed any more. `None` in a synthetic
/// pool, which says so itself.
pub fn stale(m: &Model, w: &Workspace) -> Option<&'static str> {
    (w.review.is_some() && !synthetic(m, w) && !current(m, w)).then_some(STALE)
}

/// The reason of the review's own [`Check`].
pub fn unavailable(m: &Model, w: &Workspace) -> Option<String> {
    let check = w.review.as_ref()?.check?;
    (check.reason)(m, w)
}

pub fn permission(m: &Model, w: &Workspace) -> Option<xenadmin_core::PermissionAssessment> {
    w.review.as_ref().map(|review| {
        permissions::assess_for(
            m,
            w.pool,
            &review.operation.permission_requirements(review.target),
        )
    })
}

/// "Confirm network change". Every rule of the confirm button is checked
/// again (Rune relied on the button for the observation and the
/// permission). On success the surface closes: the operation is followed
/// on the tab ("Latest operation") and in the Events tab.
pub fn submit(m: &mut Model) {
    let result = (|| {
        let w = m.areas.network.workspace.as_ref().ok_or(Error::Stale)?;
        if !m.can_mutate_pool(w.pool) {
            return Err(Error::Stale);
        }
        let review = w.review.as_ref().ok_or(Error::Stale)?;
        if review
            .check
            .is_some_and(|check| check.conflict && (check.reason)(m, w).is_some())
        {
            return Err(Error::ResourceConflict);
        }
        let target = review.observed.ok_or(Error::Stale)?;
        if !current(m, w) {
            return Err(Error::Stale);
        }
        if permission(m, w)
            .as_ref()
            .and_then(permissions::denial)
            .is_some()
        {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        let inventory: Option<Arc<Inventory>> = m.inventory_for(w.pool);
        let command = match &review.operation {
            ResourceAction::Network(operation) => ResourceCommand::Network {
                target,
                operation: operation.clone(),
            },
            ResourceAction::ConfigureInternalNetwork { expected_mtu, mtu } => {
                ResourceCommand::ConfigureInternalNetwork {
                    network: target,
                    expected_mtu: *expected_mtu,
                    mtu: *mtu,
                }
            }
            ResourceAction::ManagementIpv6Bond(operation) => {
                if !inventory
                    .as_ref()
                    .is_some_and(|i| operation.availability(i).is_available())
                {
                    return Err(Error::Stale);
                }
                ResourceCommand::ManagementIpv6Bond {
                    host: target,
                    operation: operation.clone(),
                }
            }
            ResourceAction::ManagementNetwork(operation) => {
                if !inventory
                    .as_ref()
                    .is_some_and(|i| operation.availability(i).is_available())
                {
                    return Err(Error::Stale);
                }
                ResourceCommand::ManagementNetwork {
                    host: target,
                    operation: operation.clone(),
                }
            }
            ResourceAction::NetworkTopology(operation) => {
                let ResourceTarget::Network(network) = review.target else {
                    return Err(Error::Stale);
                };
                if !inventory
                    .as_ref()
                    .is_some_and(|i| operation.availability(i, network).is_available())
                {
                    return Err(Error::Stale);
                }
                ResourceCommand::NetworkTopology {
                    target,
                    operation: operation.clone(),
                }
            }
            _ => return Err(Error::InvalidResourceCommand),
        };
        m.connection_for(w.pool)
            .ok_or(Error::Closed)?
            .submit_resource(OperationId::new(), command)
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            super::close(m);
        }
        Err(error) => {
            if let Some(w) = edit(m) {
                w.error = Some(error.to_string());
            }
        }
    }
}
