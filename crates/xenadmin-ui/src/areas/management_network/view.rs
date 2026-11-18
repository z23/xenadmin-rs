//! The dialog `management-change`, described: the form of a management
//! change, the wait for the guest impact of a bond, and the continuation
//! panel of an unfinished IPv6 handover. Nothing here changes the model.
//! The review of every change is the `network` area's dialog.
//!
//! Rune keys kept: `network-review` (the button "Review network change"),
//! `management-bond-member-<pif>`, `management-bond-network-<network>`,
//! `management-bond-mode-<mode>`, `ipv6-bond-member-<pif>`,
//! `ipv6-bond-network-<network>`, `ipv6-bond-temporary-<pif>`,
//! `ipv6-bond-leave-<op>`, `ipv6-bond-continue-<op>`. The members were
//! buttons with a check mark and are checkboxes; the destinations, modes
//! and temporary interfaces were buttons and are the options of a choice.
//!
//! New keys, for elements Rune left without one: `management-close`,
//! `management-guidance`, `management-mode`, `management-recovery`,
//! `management-address`, `management-netmask`, `management-gateway`,
//! `management-dns`, `management-destination` and its options
//! `management-destination-<pif>`, `management-validation`,
//! `management-bond-endpoint`, `management-bond-network`,
//! `management-bond-mode`, `management-bond-hash`, `management-removal`,
//! `ipv6-bond-network`, `ipv6-bond-mode`, `ipv6-bond-hash`,
//! `ipv6-bond-removal`, `ipv6-bond-temporary`, `management-pending`,
//! `management-error`, `management-pool-changed`,
//! `ipv6-bond-handover-<op>`.

use xenadmin_core::{
    BondHash, BondMode, Inventory, Ipv6Mode, ManagementAddressFamily,
    ManagementBondOperation as Bond, ManagementDynamicMode, ManagementInterface,
    ManagementNetworkOperation, NetworkId, PifId, PoolId,
};

use super::{Editor, Form, bond, ipv6, management};
use crate::areas::network::{self, commands::NOT_WRITABLE};
use crate::prelude::*;

/// The dialog of a management change.
pub const SURFACE: &str = "management-change";
/// The Rune line of the networking dialog while the guest impact is read.
pub const READING: &str = "Reading the exact guest interface impact…";
const GUIDANCE: &str = "Use the installed address family, MTU 1500 and HA disabled. Existing physical interfaces, bond masters and VLANs over physical interfaces or bonds are supported when their complete topology can be reviewed.";
const MODES: [BondMode; 3] = [BondMode::ActiveBackup, BondMode::BalanceSlb, BondMode::Lacp];
const HASHES: [BondHash; 2] = [BondHash::SourceMac, BondHash::TcpUdpPorts];
/// Rune disabled the member button of the management parent without a
/// text.
const PARENT_STAYS: &str = "The management parent is always a member.";

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(e) = &m.areas.management_network.editor else {
        return Vec::new();
    };
    let inventory = m.inventory_for(e.pool).filter(|i| i.pool_id == e.pool);
    let (title, source) = match &e.form {
        Form::Management(v) if v.moving => ("Move host management", &v.source),
        Form::Management(v) => ("Management IP settings", &v.source),
        Form::Bond(v) => ("Create management bond", &v.source),
        Form::Removal(op) => ("Remove management bond", op.source()),
        Form::Ipv6(v) => ("Staged IPv6 management bond", &v.source),
    };
    let body = blocks![
        match &inventory {
            Some(i) => form(e, i),
            None => blocks![
                caption("The connection changed. Close and reopen this dialog.")
                    .key("management-pool-changed")
            ],
        },
        e.pending
            .is_some()
            .then(|| caption(READING).key("management-pending")),
        e.error
            .clone()
            .map(|error| alert(error).key("management-error")),
        continuation(m, e.pool),
    ];
    let surface = Surface::dialog(SURFACE, title)
        .experimental(true)
        .body(body)
        .footer([
            button("management-close", "Close", super::close),
            button("network-review", "Review network change", prepare)
                .primary()
                .disabled_if(e.pending.is_some(), READING)
                .disabled_if(!network::writable(m, e.pool), NOT_WRITABLE),
        ])
        .on_close(super::close);
    vec![match &inventory {
        // The Rune caption under the heading of the form.
        Some(i) => surface.subtitle(format!(
            "{} · {}",
            network::pif_name(i, source.pif),
            source.pif
        )),
        None => surface,
    }]
}

/// "Review network change": the review of the form that is open.
fn prepare(m: &mut Model) {
    let Some(e) = &m.areas.management_network.editor else {
        return;
    };
    match &e.form {
        Form::Management(_) => management::prepare(m),
        Form::Bond(_) => bond::prepare(m),
        Form::Removal(op) => {
            let op = (**op).clone();
            bond::review(m, op);
        }
        Form::Ipv6(_) => ipv6::prepare(m),
    }
}

fn form(e: &Editor, i: &Inventory) -> Vec<Block> {
    match &e.form {
        Form::Management(v) => management_form(v, i),
        Form::Bond(v) => bond_form(v, i),
        Form::Removal(_) => blocks![
            caption("The bond under the management interface is removed. The review lists the exact guest interface impact, which is read from the host first.")
                .key("management-removal")
        ],
        Form::Ipv6(v) => ipv6_form(v, i),
    }
}

/// The interface with this id in the inventory of the open form's pool.
fn captured(m: &Model, id: &str) -> Option<ManagementInterface> {
    let e = m.areas.management_network.editor.as_ref()?;
    let i = m.inventory_for(e.pool)?;
    let pif = *i.details.pifs.keys().find(|pif| pif.to_string() == id)?;
    ManagementInterface::capture(&i, pif)
}

fn network_id(m: &Model, id: &str) -> Option<NetworkId> {
    let e = m.areas.management_network.editor.as_ref()?;
    let i = m.inventory_for(e.pool)?;
    i.networks
        .iter()
        .map(|n| NetworkId::from(n.uuid))
        .find(|n| n.to_string() == id)
}

/// A network without host or VM interface: the destination of a new bond.
fn empty_networks(i: &Inventory) -> impl Iterator<Item = (NetworkId, &str)> {
    i.networks
        .iter()
        .map(|n| (NetworkId::from(n.uuid), n.name.as_str()))
        .filter(|(id, _)| !network::workspace::network_used(i, *id))
}

// -- Management IP and move ---------------------------------------------------------

fn with_management(
    change: impl Fn(&mut management::Editor, &str) + 'static,
) -> impl Fn(&mut Model, &str) + 'static {
    move |m, text| {
        if let Some(v) = management::editor(m) {
            change(v, text);
        }
    }
}

fn dynamic_id(mode: ManagementDynamicMode) -> &'static str {
    match mode {
        ManagementDynamicMode::Ipv4Dhcp => "ipv4-dhcp",
        ManagementDynamicMode::Ipv6Dhcp => "ipv6-dhcp",
        ManagementDynamicMode::Ipv6Autoconf => "ipv6-autoconf",
    }
}

fn dynamic_modes(ipv6: bool) -> Vec<ManagementDynamicMode> {
    if ipv6 {
        vec![
            ManagementDynamicMode::Ipv6Dhcp,
            ManagementDynamicMode::Ipv6Autoconf,
        ]
    } else {
        vec![ManagementDynamicMode::Ipv4Dhcp]
    }
}

fn management_form(v: &management::Editor, i: &Inventory) -> Vec<Block> {
    blocks![
        caption(GUIDANCE).key("management-guidance"),
        if v.moving {
            destination(v, i)
        } else {
            addressing(v)
        },
        // The live refusal of the review, as Rune showed it under the form.
        management::review(v, i)
            .err()
            .map(|error| caption(error).key("management-validation")),
    ]
}

fn destination(v: &management::Editor, i: &Inventory) -> Vec<Block> {
    let options: Vec<OptionItem> = i
        .details
        .pifs
        .values()
        .filter(|p| p.host == Some(v.source.identity.host) && p.id != v.source.pif)
        .filter_map(|p| {
            let candidate = ManagementInterface::capture(i, p.id)?;
            let label = format!(
                "{} · {}",
                p.device,
                candidate
                    .address()
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "No static address".into())
            );
            let available = ManagementNetworkOperation::Move {
                source: v.source.clone(),
                destination: candidate,
            }
            .availability(i);
            Some(
                option(p.id.to_string(), label)
                    .key(format!("management-destination-{}", p.id))
                    .disabled_if(!available.is_available(), available.explanation()),
            )
        })
        .collect();
    // Chosen as Rune marked it: only while the interface is still exactly
    // the one that was captured.
    let chosen = v
        .destination
        .as_ref()
        .filter(|d| ManagementInterface::capture(i, d.pif).as_ref() == Some(*d))
        .map(|d| d.pif.to_string());
    blocks![
        caption("Choose an already plugged, statically configured interface on this host, with a different network and address."),
        choice(
            "management-destination",
            options,
            chosen.as_deref(),
            |m, id| {
                let candidate = captured(m, id);
                if let (Some(v), Some(candidate)) = (management::editor(m), candidate) {
                    v.destination = Some(candidate);
                }
            },
        )
        .label("Destination interface"),
    ]
}

fn addressing(v: &management::Editor) -> Vec<Block> {
    let ipv6 = v.source.primary == ManagementAddressFamily::Ipv6;
    let modes: Vec<OptionItem> = std::iter::once(option("static", "Static"))
        .chain(
            dynamic_modes(ipv6)
                .into_iter()
                .map(|mode| option(dynamic_id(mode), mode.label())),
        )
        .collect();
    let mode = choice(
        "management-mode",
        modes,
        Some(v.automatic.map(dynamic_id).unwrap_or("static")),
        with_management(|v, id| {
            let ipv6 = v.source.primary == ManagementAddressFamily::Ipv6;
            v.automatic = dynamic_modes(ipv6)
                .into_iter()
                .find(|mode| dynamic_id(*mode) == id);
        }),
    )
    .label("Addressing")
    .segmented();
    if v.automatic.is_some() {
        return blocks![
            mode,
            input(
                "management-recovery",
                "Expected recovery IP",
                &v.recovery,
                with_management(|v, text| v.recovery = text.to_owned()),
            )
            .help("Enter the address reserved by DHCP or expected from stable IPv6 autoconfiguration. The client verifies the actual assignment and host certificate. It does not create reservations, scan for hosts or update DNS aliases."),
        ];
    }
    blocks![
        mode,
        input(
            "management-address",
            if ipv6 {
                "IPv6 address/prefix"
            } else {
                "IPv4 address"
            },
            &v.address,
            with_management(|v, text| v.address = text.to_owned()),
        ),
        (!ipv6).then(|| input(
            "management-netmask",
            "Netmask",
            &v.netmask,
            with_management(|v, text| v.netmask = text.to_owned()),
        )),
        input(
            "management-gateway",
            "Gateway",
            &v.gateway,
            with_management(|v, text| v.gateway = text.to_owned()),
        )
        .help("Gateway (optional)"),
        input(
            "management-dns",
            "DNS servers",
            &v.dns,
            with_management(|v, text| v.dns = text.to_owned()),
        )
        .help("DNS servers, comma separated (only this address family's entries change)"),
    ]
}

// -- A bond under IPv4 management ------------------------------------------------------

fn bond_form(v: &bond::Editor, i: &Inventory) -> Vec<Block> {
    let parent = Bond::management_parent(&v.source);
    let members: Vec<Block> = i
        .details
        .pifs
        .values()
        .filter(|p| {
            p.host == Some(v.source.identity.host)
                && p.physical == Some(true)
                && p.vlan == Some(-1)
                && ManagementInterface::capture(i, p.id).is_some()
        })
        .map(|p| {
            let id = p.id;
            checkbox(
                format!("management-bond-member-{id}"),
                format!("{} · {} ({id})", p.device, p.mac),
                v.members.iter().any(|m| m.pif == id),
                move |m| toggle_bond_member(m, id),
            )
            .disabled_if(Some(id) == parent, PARENT_STAYS)
            .into()
        })
        .collect();
    let networks: Vec<OptionItem> = empty_networks(i)
        .map(|(id, name)| {
            option(id.to_string(), format!("{name} ({id})"))
                .key(format!("management-bond-network-{id}"))
        })
        .collect();
    let chosen = v.network.map(|id| id.to_string());
    let unavailable = bond::operation(v).ok().and_then(|op| {
        let a = op.availability(i);
        (!a.is_available()).then(|| a.explanation())
    });
    blocks![
        caption(format!(
            "Management endpoint: {} · {}",
            network::pif_name(i, v.source.pif),
            v.source.ipv4.address
        ))
        .key("management-bond-endpoint"),
        caption("Choose 2–4 physical members on this host. The management VLAN parent stays first. Reviewed VLANs retain their networks and static IPv4 settings."),
        members,
        choice(
            "management-bond-network",
            networks,
            chosen.as_deref(),
            |m, id| {
                let network = network_id(m, id);
                if let (Some(v), Some(network)) = (bond::editor(m), network) {
                    v.network = Some(network);
                }
            },
        )
        .label("Destination network (create an empty internal network first if needed)"),
        choice(
            "management-bond-mode",
            MODES.map(|mode| {
                option(mode.as_str(), mode.as_str())
                    .key(format!("management-bond-mode-{}", mode.as_str()))
            }),
            Some(v.mode.as_str()),
            |m, id| {
                if let (Some(v), Some(mode)) =
                    (bond::editor(m), MODES.into_iter().find(|mode| mode.as_str() == id))
                {
                    v.mode = mode;
                }
            },
        )
        .label("Bond mode")
        .segmented(),
        (v.mode == BondMode::Lacp).then(|| {
            choice(
                "management-bond-hash",
                HASHES.map(|hash| option(hash.as_str(), hash.as_str())),
                Some(v.hash.as_str()),
                |m, id| {
                    if let (Some(v), Some(hash)) = (
                        bond::editor(m),
                        HASHES.into_iter().find(|hash| hash.as_str() == id),
                    ) {
                        v.hash = hash;
                    }
                },
            )
            .label("LACP hash")
            .segmented()
        }),
        unavailable.map(|reason| caption(reason).key("management-validation")),
        caption("Review the exact members, switch mode and recovery endpoint before applying."),
    ]
}

fn toggle_bond_member(m: &mut Model, id: PifId) {
    let candidate = captured(m, &id.to_string());
    let Some(v) = bond::editor(m) else {
        return;
    };
    if let Some(index) = v.members.iter().position(|m| m.pif == id) {
        v.members.remove(index);
    } else if let Some(candidate) = candidate {
        v.members.push(candidate);
    }
}

// -- The staged IPv6 bond ----------------------------------------------------------------

fn ipv6_form(v: &ipv6::Editor, i: &Inventory) -> Vec<Block> {
    let temporary: Vec<OptionItem> = i
        .details
        .pifs
        .values()
        .filter(|p| p.host == Some(v.source.identity.host) && p.management == Some(false))
        .filter_map(|p| {
            let captured = ManagementInterface::capture(i, p.id).filter(|p| {
                p.primary == ManagementAddressFamily::Ipv6 && p.ipv6.mode == Ipv6Mode::Static
            })?;
            Some(
                option(
                    p.id.to_string(),
                    format!(
                        "{} · {} ({})",
                        p.device,
                        captured.ipv6.addresses.join(", "),
                        p.id
                    ),
                )
                .key(format!("ipv6-bond-temporary-{}", p.id)),
            )
        })
        .collect();
    let chosen = v.temporary.as_ref().map(|p| p.pif.to_string());
    let unavailable = ipv6::operation(v).ok().and_then(|op| {
        let a = op.availability(i);
        (!a.is_available()).then(|| a.explanation())
    });
    blocks![
        caption("Choose an independently reachable temporary interface. Each stage requires its own review; the temporary address remains configured."),
        ipv6_bond(v, i),
        heading("Independent temporary IPv6 path"),
        choice(
            "ipv6-bond-temporary",
            temporary,
            chosen.as_deref(),
            |m, id| {
                let candidate = captured(m, id);
                if let (Some(v), Some(candidate)) = (ipv6::editor(m), candidate) {
                    v.temporary = Some(candidate);
                }
            },
        )
        .label("Temporary management interface"),
        unavailable.map(|reason| caption(reason).key("management-validation")),
    ]
}

/// What a new bond is made of; for a removal, which bond goes.
fn ipv6_bond(v: &ipv6::Editor, i: &Inventory) -> Vec<Block> {
    let (members, network, mode, hash) = match &v.operation {
        Bond::Create {
            members,
            network,
            mode,
            hash,
        } => (members, *network, *mode, *hash),
        Bond::Destroy { bond, primary } => {
            return blocks![
                caption(format!(
                    "Remove bond {bond}; return the reviewed management endpoint/VLAN to primary {} ({primary})",
                    network::pif_name(i, *primary)
                ))
                .key("ipv6-bond-removal")
            ];
        }
    };
    let parent = Bond::management_parent(&v.source);
    let boxes: Vec<Block> = i
        .details
        .pifs
        .values()
        .filter(|p| {
            p.host == Some(v.source.identity.host)
                && p.physical == Some(true)
                && p.primary_address_type.as_deref() == Some("IPv6")
                && ManagementInterface::capture(i, p.id).is_some()
        })
        .map(|p| {
            let id = p.id;
            checkbox(
                format!("ipv6-bond-member-{id}"),
                format!("member {} ({id})", p.device),
                members.iter().any(|v| v.pif == id),
                move |m| toggle_ipv6_member(m, id),
            )
            .disabled_if(Some(id) == parent, PARENT_STAYS)
            .into()
        })
        .collect();
    let networks: Vec<OptionItem> = empty_networks(i)
        .map(|(id, name)| {
            option(id.to_string(), format!("{name} ({id})"))
                .key(format!("ipv6-bond-network-{id}"))
        })
        .collect();
    let chosen = network.to_string();
    blocks![
        boxes,
        choice(
            "ipv6-bond-network",
            networks,
            Some(chosen.as_str()),
            |m, id| {
                let chosen = network_id(m, id);
                if let (Some(v), Some(chosen)) = (ipv6::editor(m), chosen)
                    && let Bond::Create { network, .. } = &mut v.operation
                {
                    *network = chosen;
                }
            },
        )
        .label("Destination network"),
        choice(
            "ipv6-bond-mode",
            MODES.map(|mode| option(mode.as_str(), mode.as_str())),
            Some(mode.as_str()),
            |m, id| {
                let next = MODES.into_iter().find(|mode| mode.as_str() == id);
                if let (Some(v), Some(next)) = (ipv6::editor(m), next)
                    && let Bond::Create { mode, hash, .. } = &mut v.operation
                {
                    *mode = next;
                    *hash = (next == BondMode::Lacp).then_some(BondHash::SourceMac);
                }
            },
        )
        .label("Bond mode")
        .segmented(),
        (mode == BondMode::Lacp).then(|| {
            choice(
                "ipv6-bond-hash",
                HASHES.map(|hash| option(hash.as_str(), hash.as_str())),
                hash.map(|hash| hash.as_str()),
                |m, id| {
                    let next = HASHES.into_iter().find(|hash| hash.as_str() == id);
                    if let (Some(v), Some(next)) = (ipv6::editor(m), next)
                        && let Bond::Create { hash, .. } = &mut v.operation
                    {
                        *hash = Some(next);
                    }
                },
            )
            .label("LACP hash")
            .segmented()
        }),
    ]
}

fn toggle_ipv6_member(m: &mut Model, id: PifId) {
    let candidate = captured(m, &id.to_string());
    let Some(v) = ipv6::editor(m) else {
        return;
    };
    let Bond::Create { members, .. } = &mut v.operation else {
        return;
    };
    if let Some(n) = members.iter().position(|p| p.pif == id) {
        members.remove(n);
    } else if let Some(candidate) = candidate {
        members.push(candidate);
    }
}

// -- The continuation of an unfinished IPv6 handover ---------------------------------------

/// The caption of each unfinished IPv6 handover of a pool, without the
/// panel's buttons: for a tab whose local row has the two continuation
/// commands (the NICs and Networking tabs of the `network` area). Empty
/// when the pool has none.
pub fn continuation_notes(m: &Model, pool: PoolId) -> Vec<Block> {
    ipv6::handovers(m, pool)
        .into_iter()
        .map(|h| caption(h.caption).key(format!("ipv6-bond-handover-note-{}", h.record)).into())
        .collect()
}

/// The Rune continuation panel for a pool: per unfinished IPv6 handover its
/// caption and the buttons "Leave partial configuration…" and "Review stage
/// {n}…". Empty when the pool has none. Public: a tab or a record list of
/// another area may show it.
pub fn continuation(m: &Model, pool: PoolId) -> Vec<Block> {
    let writable = m.can_mutate_pool(pool);
    ipv6::handovers(m, pool)
        .into_iter()
        .flat_map(|h| {
            let record = h.record;
            blocks![
                caption(h.caption).key(format!("ipv6-bond-handover-{record}")),
                actions([
                    button(
                        format!("ipv6-bond-leave-{record}"),
                        "Leave partial configuration…",
                        move |m| ipv6::resume(m, pool, record, true),
                    )
                    .disabled_if(!writable, NOT_WRITABLE),
                    button(
                        format!("ipv6-bond-continue-{record}"),
                        format!("Review stage {}…", h.stage + 1),
                        move |m| ipv6::resume(m, pool, record, false),
                    )
                    .disabled_if(!writable, NOT_WRITABLE),
                ]),
            ]
        })
        .collect()
}
