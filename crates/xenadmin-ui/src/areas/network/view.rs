//! The open surface and the Properties pages of the area, described.
//! Nothing here changes the model.
//!
//! Two editors, each a dialog of its own or a page of a Properties dialog:
//!
//! - the generic editor (`editor.rs`): the wizard `network-new`, the
//!   dialogs `network-remove`, `storage-rescan`, `server-scheduling` and
//!   `storage-set-default`, and the General page (name and description) of
//!   the Properties dialog of a pool, a server, a storage repository and a
//!   network;
//! - the network change (`workspace.rs`): the dialog `network-change`, the
//!   page Network Settings of a network and the page Network Options of a
//!   pool.
//!
//! Rune keys kept: `resource-name`, `resource-mtu`, `resource-review`,
//! `resource-confirm`, `network-review`, `network-confirm`. The Rune keys
//! `network-review-line-<n>` were the lines of the review: they are the
//! lines of the review block `network-change-review` now.
//!
//! New keys, for elements Rune left without one: `resource-description`,
//! `resource-close`, `resource-back`, `resource-next`,
//! `resource-change-review` (and its `-stale` line), `resource-stale`,
//! `resource-preview`, `resource-error`, `network-new-type`,
//! `network-close`, `network-back`, `network-change-review`,
//! `network-preview`, `network-error`, `network-unavailable`,
//! `network-pool-changed`, `network-vlan-tag`, `network-vlan-parent`,
//! `network-bond-network`, `network-bond-mode`, `network-bond-hash`,
//! `network-bond-member-<pif>`, `network-bond-current`, `network-mtu`,
//! `network-automatic`, `network-automatic-state`, `network-igmp`,
//! `network-igmp-state`, `network-ipv4-mode`, `network-ipv4-address`,
//! `network-ipv4-netmask`, `network-ipv4-gateway`, `network-ipv4-dns`,
//! `network-interface`, `network-ipv6-mode`, `network-ipv6-address`,
//! `network-ipv6-gateway`, `network-ipv6-dns`, `network-purpose`,
//! `network-page-error`.

use xenadmin_core::{
    BondHash, BondMode, Inventory, Ipv4Mode, Ipv6Mode, NetworkId, PermissionAssessment, PifId,
    ResourceTarget,
};

use super::commands::{self, NOT_WRITABLE};
use crate::areas::network_services::commands::{FABRIC_ADD, FABRIC_COMPLETE_POOL};
use super::editor::{self, Editor, Kind};
use super::workspace::{self, Form, InterfaceEditor, Page, Workspace};
use crate::prelude::*;

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";
/// The Rune caption of the generic editor in the demo.
pub const PREVIEW: &str = "Synthetic preview · host changes cannot be submitted.";
/// The dialog of a network change.
pub const CHANGE: &str = "network-change";
/// The wizard New Network.
pub const WIZARD: &str = "network-new";
pub const STEPS: [&str; 4] = ["Type", "Name", "Settings", "Review"];
/// The review blocks; `<key>-stale` is the stale line of each.
pub const REVIEW: &str = "network-change-review";
pub const EDITOR_REVIEW: &str = "resource-change-review";

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let s = &m.areas.network;
    if let Some(e) = s.editor.as_ref().filter(|e| e.kind != Kind::Rename) {
        return vec![editor_surface(m, e)];
    }
    if let Some(w) = s.workspace.as_ref().filter(|w| w.page.is_none()) {
        return vec![workspace_surface(m, w)];
    }
    Vec::new()
}

// -- The generic editor ----------------------------------------------------------

/// The id of the dialog of each change of the generic editor.
pub fn dialog_id(kind: Kind) -> &'static str {
    match kind {
        Kind::CreateNetwork => WIZARD,
        Kind::DeleteNetwork => "network-remove",
        Kind::ScanStorage => "storage-rescan",
        Kind::HostEnabled(_) => "server-scheduling",
        Kind::DefaultStorage => "storage-set-default",
        Kind::Rename => "resource-rename",
    }
}

fn editor_surface(m: &Model, e: &Editor) -> Surface {
    let permission = editor::permission_review(m, e);
    let surface = match e.kind {
        Kind::CreateNetwork => Surface::wizard(
            WIZARD,
            "New Network",
            STEPS,
            if e.review.is_some() { 3 } else { e.step.min(2) },
        ),
        kind => Surface::dialog(dialog_id(kind), kind.title()),
    };
    // The Rune caption under the heading: name, target and pool.
    let subtitle = format!(
        "{}\n{}{}",
        e.label,
        editor::target_label(e.target),
        if matches!(e.target, ResourceTarget::Pool(_)) {
            String::new()
        } else {
            format!("\nPool {}", e.pool)
        }
    );
    let surface = surface
        .subtitle(subtitle)
        .experimental(true)
        .body(editor_body(m, e))
        .footer(editor_footer(m, e, permission.as_ref()))
        .on_close(super::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn editor_body(m: &Model, e: &Editor) -> Vec<Block> {
    blocks![
        // A page says which change it is; a dialog has it as its title.
        (e.kind == Kind::Rename).then(|| heading(e.kind.title())),
        match &e.review {
            Some(review) => blocks![
                (e.kind == Kind::CreateNetwork).then(|| heading(e.kind.title())),
                crate::surface::review(EDITOR_REVIEW, review.lines.clone())
                    .stale(editor::stale(m, e)),
            ],
            None => blocks![
                draft(m, e),
                editor::stale(m, e)
                    .map(|reason| caption(reason).tone(Tone::Warning).key("resource-stale")),
            ],
        },
        editor::synthetic(m, e).then(|| notice(NoticeKind::Preview, PREVIEW).key("resource-preview")),
        e.error.clone().map(|error| alert(error).key("resource-error")),
    ]
}

/// A handler that edits the open generic editor with the new text.
fn edited(change: impl Fn(&mut Editor, &str) + 'static) -> impl Fn(&mut Model, &str) + 'static {
    move |m, text| {
        if let Some(e) = &mut m.areas.network.editor {
            change(e, text);
            e.error = None;
        }
    }
}

fn name_fields(e: &Editor) -> Vec<Block> {
    blocks![
        input(
            "resource-name",
            "Name",
            &e.name,
            edited(|e, text| e.name = text.to_owned())
        ),
        input(
            "resource-description",
            "Description",
            &e.description,
            edited(|e, text| e.description = text.to_owned())
        ),
    ]
}

/// The types of the Type step that are another area's: the fabric wizards
/// of `network_services`, which have the same four steps. Choosing one hands
/// over to its command on the network the wizard was opened on.
pub const HANDOVERS: [(&str, CommandId, &str); 2] = [
    (
        "fabric",
        FABRIC_ADD,
        "A GRE or VXLAN tunnel or SR-IOV on one server, added to the selected network.",
    ),
    (
        "fabric-pool",
        FABRIC_COMPLETE_POOL,
        "The same on every server of the pool that lacks it.",
    ),
];

/// Why a fabric type cannot be chosen while no network is the target: the
/// core adds a tunnel or SR-IOV interface to a network that exists.
pub const NEEDS_NETWORK: &str = "Select the network in the grid of the Networking tab first: a tunnel or SR-IOV interface is added to a network that exists.";

/// The option of a handed-over type: the label of its command, disabled
/// with the command's own reason.
fn handover_option(m: &Model, id: &'static str, command: CommandId, detail: &str) -> OptionItem {
    let label = crate::command::find(command).map_or(command.0, |spec| spec.label);
    let reason = match m.eligibility(command, &m.target()) {
        Eligibility::Enabled => None,
        Eligibility::Disabled(reason) => Some(reason),
        Eligibility::Hidden => Some(NEEDS_NETWORK.to_owned()),
    };
    option(id, label)
        .detail(detail)
        .key(format!("network-new-type-{id}"))
        .disabled_if(reason.is_some(), reason.unwrap_or_default())
}

/// A type of another area was chosen: its command opens its wizard on the
/// target the wizard New Network was opened on, and this one closes. When
/// the command refuses, this wizard stays and says why.
fn hand_over(m: &mut Model, id: &str) {
    let Some((_, command, _)) = HANDOVERS.iter().find(|(option, ..)| *option == id) else {
        return;
    };
    let target = m.target();
    let mine = m.areas.network.editor.take();
    if !m.invoke(*command, &target) {
        m.areas.network.editor = mine;
        let refusal = m.error.take();
        if let Some(e) = &mut m.areas.network.editor {
            e.error = refusal;
        }
    }
}

fn draft(m: &Model, e: &Editor) -> Vec<Block> {
    match (e.kind, e.step) {
        (Kind::Rename, _) => name_fields(e),
        // These modules create one type of network. A tunnel or SR-IOV
        // interface is the `network_services` area's: its two wizards are
        // the other types. A VLAN interface and a bond are added to a
        // network that exists, from its Networking tab and the NICs tab.
        (Kind::CreateNetwork, 0) => blocks![
            choice(
                "network-new-type",
                std::iter::once(
                    option("internal", "Internal network")
                        .detail("A network without a physical interface, for traffic between VMs on one server.")
                )
                .chain(
                    HANDOVERS
                        .iter()
                        .map(|(id, command, detail)| handover_option(m, id, *command, detail))
                )
                .collect::<Vec<_>>(),
                Some("internal"),
                hand_over
            )
            .label("Network type"),
            caption("To put an existing network on a VLAN or a bond, add the interface from the network's Networking tab or from the NICs tab of a server."),
        ],
        (Kind::CreateNetwork, 1) => name_fields(e),
        (Kind::CreateNetwork, _) => blocks![input(
            "resource-mtu",
            "MTU",
            &e.mtu,
            edited(|e, text| e.mtu = text.to_owned())
        )
        .on_submit(editor::review)],
        _ => Vec::new(),
    }
}

fn editor_footer(m: &Model, e: &Editor, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let page = e.kind == Kind::Rename;
    let close = button(
        "resource-close",
        if page { "Close" } else { "Cancel" },
        close_all,
    );
    let step = |by: isize| {
        move |m: &mut Model| {
            if let Some(e) = &mut m.areas.network.editor {
                e.step = e.step.saturating_add_signed(by).min(2);
                e.error = None;
            }
        }
    };
    if e.review.is_some() {
        let confirm = button("resource-confirm", "Confirm", editor::submit);
        let confirm = if e.kind.danger() {
            confirm.danger()
        } else {
            confirm.primary()
        };
        // The first reason wins: stale, then a known denial, then the demo.
        let confirm = confirm
            .disabled_opt(editor::stale(m, e))
            .disabled_opt(permission.and_then(permissions::denial))
            .disabled_if(editor::synthetic(m, e), DEMO);
        // A review that was opened by its command has no draft to go back to.
        return if e.kind.opens_on_review() {
            vec![close, confirm]
        } else {
            vec![close, button("resource-back", "Back", editor::back), confirm]
        };
    }
    let review = button("resource-review", "Review", editor::review).primary();
    let back = button("resource-back", "Back", step(-1));
    let next = button("resource-next", "Next", step(1)).primary();
    match (e.kind, e.step) {
        (Kind::CreateNetwork, 0) => vec![close, next],
        (Kind::CreateNetwork, 1) => vec![close, back, next],
        (Kind::CreateNetwork, _) => vec![close, back, review],
        _ => vec![close, review],
    }
}

/// Close of a dialog or of a page: a page takes its Properties dialog along.
fn close_all(m: &mut Model) {
    super::close(m);
    m.close_properties();
}

// -- The network change -----------------------------------------------------------

fn form_title(form: &Form) -> &'static str {
    match form {
        Form::None => "Network change",
        Form::Vlan { .. } => "Add VLAN interface",
        Form::Bond { .. } => "Create bond",
        Form::Mtu { .. } => "Network MTU",
        Form::Ipv4 { .. } => "IPv4 configuration",
        Form::Interface(editor) => match **editor {
            InterfaceEditor::Ipv6 { .. } => "IPv6 configuration",
            InterfaceEditor::Purpose { .. } => "Interface purpose",
        },
        Form::BondMode { .. } => "Change bond mode",
        Form::BondHash { .. } => "Change bond hash",
    }
}

fn workspace_surface(m: &Model, w: &Workspace) -> Surface {
    let permission = workspace::permission(m, w);
    let inventory = m.inventory_for(w.pool);
    let (title, subtitle) = match &w.review {
        Some(review) => (
            review.operation.label(),
            Some(format!(
                "Target {} · pool {}",
                review.target.uuid(),
                w.pool
            )),
        ),
        None => (
            form_title(&w.form),
            w.network.zip(inventory.as_ref()).map(|(network, i)| {
                format!("Network: {} ({network})", workspace::name(i, network))
            }),
        ),
    };
    let surface = Surface::dialog(CHANGE, title)
        .experimental(true)
        .body(workspace_body(m, w))
        .footer(workspace_footer(m, w, permission.as_ref()))
        .on_close(super::close);
    let surface = match subtitle {
        Some(subtitle) => surface.subtitle(subtitle),
        None => surface,
    };
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn workspace_body(m: &Model, w: &Workspace) -> Vec<Block> {
    let inventory = m.inventory_for(w.pool);
    blocks![
        match (&w.review, &inventory) {
            (Some(review), _) => blocks![
                w.page.map(|_| heading(review.operation.label())),
                crate::surface::review(REVIEW, review.lines.clone()).stale(workspace::stale(m, w)),
            ],
            (None, Some(i)) => form(m, w, i),
            (None, None) => blocks![
                caption("The connection changed. Close and reopen this dialog.")
                    .key("network-pool-changed")
            ],
        },
        workspace::unavailable(m, w).map(|reason| alert(reason).key("network-unavailable")),
        workspace::synthetic(m, w).then(|| notice(NoticeKind::Preview, PREVIEW).key("network-preview")),
        w.error.clone().map(|error| alert(error).key("network-error")),
    ]
}

/// A handler that edits the form of the open workspace with the new text
/// (an edited text, or the id of a picked option).
fn formed(change: impl Fn(&mut Form, &str) + 'static) -> impl Fn(&mut Model, &str) + 'static {
    move |m, text| {
        if let Some(w) = &mut m.areas.network.workspace {
            change(&mut w.form, text);
            w.error = None;
        }
    }
}

/// The interface of the workspace's pool with this id as text.
fn pif_by_text(m: &Model, id: &str) -> Option<PifId> {
    let w = m.areas.network.workspace.as_ref()?;
    m.inventory_for(w.pool)?
        .details
        .pifs
        .keys()
        .copied()
        .find(|pif| pif.to_string() == id)
}

fn bond_caption(i: &Inventory, id: xenadmin_core::BondId) -> Option<Text> {
    let bond = i.details.bonds.values().find(|bond| bond.id == id)?;
    Some(
        caption(format!(
            "Bond {} · {} · hash {}",
            bond.id,
            bond.mode,
            bond.hashing_algorithm.as_deref().unwrap_or("default")
        ))
        .key("network-bond-current"),
    )
}

fn mode_choice(modes: &[BondMode], selected: Option<BondMode>, label: &str) -> Choice {
    choice(
        "network-bond-mode",
        modes.iter().map(|mode| option(mode.as_str(), mode.as_str())),
        selected.map(|mode| mode.as_str()),
        formed(|form, id| {
            let picked = workspace::BOND_MODES
                .into_iter()
                .find(|mode| mode.as_str() == id);
            match form {
                Form::Bond { mode, .. } => {
                    if let Some(picked) = picked {
                        *mode = picked;
                    }
                }
                Form::BondMode { mode, .. } => *mode = picked,
                _ => {}
            }
        }),
    )
    .label(label)
    .segmented()
}

fn hash_choice(hashes: &[BondHash], selected: Option<BondHash>) -> Choice {
    choice(
        "network-bond-hash",
        hashes
            .iter()
            .map(|hash| option(hash.as_str(), format!("Hash: {}", hash.as_str()))),
        selected.map(|hash| hash.as_str()),
        formed(|form, id| {
            let picked = workspace::BOND_HASHES
                .into_iter()
                .find(|hash| hash.as_str() == id);
            match form {
                Form::Bond { hash, .. } => {
                    if let Some(picked) = picked {
                        *hash = picked;
                    }
                }
                Form::BondHash { hash, .. } => *hash = picked,
                _ => {}
            }
        }),
    )
    .label("Hash")
    .segmented()
}

fn form(m: &Model, w: &Workspace, i: &Inventory) -> Vec<Block> {
    let writable = commands::writable(m, w.pool);
    match &w.form {
        Form::None => match w.page {
            Some(Page::NetworkOptions) => igmp(i, writable),
            _ => Vec::new(),
        },
        Form::Vlan { parent, tag } => {
            let parents: Vec<OptionItem> = i
                .details
                .pifs
                .values()
                .filter(|p| {
                    p.vlan == Some(-1)
                        && (p.physical == Some(true)
                            || i.details.bonds.values().any(|b| b.master == Some(p.id)))
                        && w.host.is_none_or(|id| p.host == Some(id))
                        && p.network != w.network
                })
                .map(|p| {
                    option(
                        p.id.to_string(),
                        format!("{} · {}", workspace::pif_name(i, p.id), p.mac),
                    )
                })
                .collect();
            let selected = parent.map(|id| id.to_string());
            blocks![
                input(
                    "network-vlan-tag",
                    "VLAN tag (0–4094)",
                    tag,
                    formed(|form, text| {
                        if let Form::Vlan { tag, .. } = form {
                            *tag = text.to_owned();
                        }
                    })
                )
                .on_submit(workspace::form_review),
                choice(
                    "network-vlan-parent",
                    parents,
                    selected.as_deref(),
                    |m, id| {
                        let picked = pif_by_text(m, id);
                        if let Some(w) = &mut m.areas.network.workspace
                            && let Form::Vlan { parent, .. } = &mut w.form
                            && picked.is_some()
                        {
                            *parent = picked;
                            w.error = None;
                        }
                    }
                )
                .label("Physical interface"),
            ]
        }
        Form::Bond {
            members,
            mode,
            hash,
        } => {
            let networks: Vec<OptionItem> = i
                .networks
                .iter()
                .map(|n| option(n.uuid.to_string(), n.name.clone()))
                .collect();
            let selected = w.network.map(|id| id.uuid().to_string());
            let candidates = i.details.pifs.values().filter(|p| {
                p.physical == Some(true)
                    && p.management == Some(false)
                    && p.vlan == Some(-1)
                    && p.network != w.network
                    && w.host.is_none_or(|id| p.host == Some(id))
            });
            blocks![
                caption("Select two to four unused physical NICs on the same host. The switch must be configured for the chosen mode."),
                select(
                    "network-bond-network",
                    "Destination network",
                    networks,
                    selected.as_deref(),
                    |m, id| {
                        let picked = uuid::Uuid::parse_str(id).ok().map(NetworkId::from);
                        if let Some(w) = &mut m.areas.network.workspace
                            && picked.is_some()
                        {
                            w.network = picked;
                            w.error = None;
                        }
                    }
                ),
                mode_choice(
                    &[BondMode::ActiveBackup, BondMode::BalanceSlb, BondMode::Lacp],
                    Some(*mode),
                    "Mode"
                ),
                (*mode == BondMode::Lacp)
                    .then(|| hash_choice(&workspace::BOND_HASHES, Some(*hash))),
                candidates
                    .map(|p| {
                        let id = p.id;
                        checkbox(
                            format!("network-bond-member-{id}"),
                            format!("{} · {}", workspace::pif_name(i, id), p.mac),
                            members.contains(&id),
                            move |m| {
                                if let Some(w) = &mut m.areas.network.workspace
                                    && let Form::Bond { members, .. } = &mut w.form
                                {
                                    if members.contains(&id) {
                                        members.retain(|m| *m != id);
                                    } else if members.len() < 4 {
                                        members.push(id);
                                    }
                                    w.error = None;
                                }
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            ]
        }
        Form::Mtu { value } => settings(i, w, value, writable),
        Form::Ipv4 {
            mode,
            address,
            netmask,
            gateway,
            dns,
            ..
        } => {
            let field = |key: &str, label: &str, value: &str, pick: fn(&mut Form) -> Option<&mut String>| {
                input(
                    key,
                    label,
                    value,
                    formed(move |form, text| {
                        if let Some(value) = pick(form) {
                            *value = text.to_owned();
                        }
                    }),
                )
            };
            blocks![
                choice(
                    "network-ipv4-mode",
                    [Ipv4Mode::None, Ipv4Mode::Dhcp, Ipv4Mode::Static]
                        .map(|mode| option(mode.as_str(), mode.as_str())),
                    Some(mode.as_str()),
                    formed(|form, id| {
                        let picked = [Ipv4Mode::None, Ipv4Mode::Dhcp, Ipv4Mode::Static]
                            .into_iter()
                            .find(|mode| mode.as_str() == id);
                        if let (Form::Ipv4 { mode, .. }, Some(picked)) = (form, picked) {
                            *mode = picked;
                        }
                    })
                )
                .label("Mode")
                .segmented(),
                (*mode == Ipv4Mode::Static).then(|| blocks![
                    field("network-ipv4-address", "IP address", address, |form| match form {
                        Form::Ipv4 { address, .. } => Some(address),
                        _ => None,
                    }),
                    field("network-ipv4-netmask", "Netmask", netmask, |form| match form {
                        Form::Ipv4 { netmask, .. } => Some(netmask),
                        _ => None,
                    }),
                    field(
                        "network-ipv4-gateway",
                        "Gateway (optional)",
                        gateway,
                        |form| match form {
                            Form::Ipv4 { gateway, .. } => Some(gateway),
                            _ => None,
                        }
                    ),
                ]),
                field(
                    "network-ipv4-dns",
                    "DNS servers, comma separated",
                    dns,
                    |form| match form {
                        Form::Ipv4 { dns, .. } => Some(dns),
                        _ => None,
                    }
                ),
            ]
        }
        Form::Interface(editor) => interface(i, editor),
        Form::BondMode { bond, mode } => {
            let modes = i
                .details
                .bonds
                .values()
                .find(|b| b.id == *bond)
                .map(workspace::other_modes)
                .unwrap_or_default();
            blocks![
                bond_caption(i, *bond),
                mode_choice(&modes, *mode, "Change to"),
            ]
        }
        Form::BondHash { bond, hash } => {
            let hashes = i
                .details
                .bonds
                .values()
                .find(|b| b.id == *bond)
                .map(workspace::other_hashes)
                .unwrap_or_default();
            blocks![bond_caption(i, *bond), hash_choice(&hashes, *hash)]
        }
    }
}

/// The IPv6 and the purpose form of `network_interfaces.rs`.
fn interface(i: &Inventory, editor: &InterfaceEditor) -> Vec<Block> {
    let id = match editor {
        InterfaceEditor::Ipv6 { id, .. } | InterfaceEditor::Purpose { id, .. } => *id,
    };
    let field = |key: &str,
                 label: &str,
                 value: &str,
                 pick: fn(&mut InterfaceEditor) -> Option<&mut String>| {
        input(
            key,
            label,
            value,
            formed(move |form, text| {
                if let Form::Interface(editor) = form
                    && let Some(value) = pick(editor)
                {
                    *value = text.to_owned();
                }
            }),
        )
    };
    const MODES: [Ipv6Mode; 4] = [
        Ipv6Mode::None,
        Ipv6Mode::Dhcp,
        Ipv6Mode::Autoconf,
        Ipv6Mode::Static,
    ];
    blocks![
        caption(format!(
            "Interface: {} ({id})",
            workspace::pif_name(i, id)
        ))
        .key("network-interface"),
        match editor {
            InterfaceEditor::Purpose { value, .. } => blocks![
                caption("A label for secondary management or storage traffic; leave empty to remove it."),
                field("network-purpose", "Purpose", value, |editor| match editor {
                    InterfaceEditor::Purpose { value, .. } => Some(value),
                    _ => None,
                })
                .on_submit(workspace::form_review),
            ],
            InterfaceEditor::Ipv6 {
                mode,
                address,
                gateway,
                dns,
                ..
            } => blocks![
                choice(
                    "network-ipv6-mode",
                    MODES.map(|mode| option(mode.as_str(), mode.as_str())),
                    Some(mode.as_str()),
                    formed(|form, id| {
                        let picked = MODES.into_iter().find(|mode| mode.as_str() == id);
                        if let Form::Interface(editor) = form
                            && let InterfaceEditor::Ipv6 { mode, .. } = &mut **editor
                            && let Some(picked) = picked
                        {
                            *mode = picked;
                        }
                    })
                )
                .label("Mode")
                .segmented(),
                (*mode == Ipv6Mode::Static).then(|| blocks![
                    field(
                        "network-ipv6-address",
                        "IPv6 address/prefix",
                        address,
                        |editor| match editor {
                            InterfaceEditor::Ipv6 { address, .. } => Some(address),
                            _ => None,
                        }
                    )
                    .help("IPv6 address/prefix (for example 2001:db8::20/64)"),
                    field(
                        "network-ipv6-gateway",
                        "IPv6 gateway",
                        gateway,
                        |editor| match editor {
                            InterfaceEditor::Ipv6 { gateway, .. } => Some(gateway),
                            _ => None,
                        }
                    )
                    .help("IPv6 gateway (optional)"),
                ]),
                field("network-ipv6-dns", "DNS servers", dns, |editor| match editor {
                    InterfaceEditor::Ipv6 { dns, .. } => Some(dns),
                    _ => None,
                })
                .help("DNS servers, comma separated (shared with IPv4)"),
            ],
        },
    ]
}

/// The page Network Settings: the MTU, and the automatic attachment with
/// the Rune button that goes straight to its review.
fn settings(i: &Inventory, w: &Workspace, value: &str, writable: bool) -> Vec<Block> {
    let automatic = w
        .network
        .and_then(|id| i.details.networks.get(&id))
        .filter(|n| n.automatic_known)
        .map(|n| n.automatic.as_deref() != Some("false"));
    blocks![
        group(
            "Network MTU",
            blocks![
                input(
                    "network-mtu",
                    "MTU in bytes",
                    value,
                    formed(|form, text| {
                        if let Form::Mtu { value } = form {
                            *value = text.to_owned();
                        }
                    })
                )
                .on_submit(workspace::form_review)
            ]
        ),
        automatic.map(|automatic| {
            group(
                "Automatic attachment",
                blocks![
                    caption(format!(
                        "Automatic attachment to new VMs: {}",
                        if automatic { "enabled" } else { "disabled" }
                    ))
                    .key("network-automatic-state"),
                    button(
                        "network-automatic",
                        if automatic {
                            "Disable automatic attachment"
                        } else {
                            "Enable automatic attachment"
                        },
                        |m| {
                            let staged = m.areas.network.workspace.as_ref().and_then(|w| {
                                let inventory = m.inventory_for(w.pool)?;
                                workspace::automatic(&inventory, w.network?)
                            });
                            if let Some((target, operation, lines)) = staged {
                                workspace::prepare(m, target, operation, lines);
                            }
                        }
                    )
                    .disabled_if(!writable, NOT_WRITABLE),
                ],
            )
        }),
    ]
}

/// The page Network Options of a pool: IGMP snooping, with the Rune
/// button that goes straight to its review. Rune showed nothing while the
/// pool does not report the state.
fn igmp(i: &Inventory, writable: bool) -> Vec<Block> {
    let state = i
        .details
        .pool
        .as_ref()
        .and_then(|pool| pool.igmp_snooping_enabled);
    blocks![group(
        "IGMP snooping",
        blocks![
            caption(format!(
                "IGMP snooping: {}",
                match state {
                    Some(true) => "enabled",
                    Some(false) => "disabled",
                    None => "not reported by this pool",
                }
            ))
            .key("network-igmp-state"),
            state.map(|igmp| {
                button(
                    "network-igmp",
                    if igmp {
                        "Disable pool IGMP snooping"
                    } else {
                        "Enable pool IGMP snooping"
                    },
                    |m| {
                        let staged = m.areas.network.workspace.as_ref().and_then(|w| {
                            workspace::igmp(&*m.inventory_for(w.pool)?)
                        });
                        if let Some((target, operation, lines)) = staged {
                            workspace::prepare(m, target, operation, lines);
                        }
                    },
                )
                .disabled_if(!writable, NOT_WRITABLE)
            }),
        ]
    )]
}

fn workspace_footer(
    m: &Model,
    w: &Workspace,
    permission: Option<&PermissionAssessment>,
) -> Vec<Button> {
    let close = button("network-close", "Close", close_all);
    let drafted = !matches!(w.form, Form::None);
    if w.review.is_some() {
        // The first reason wins: stale, the review's own rule, a known
        // denial, then the demo.
        let confirm = button("network-confirm", "Confirm network change", workspace::submit)
            .primary()
            .disabled_opt(workspace::stale(m, w))
            .disabled_opt(workspace::unavailable(m, w))
            .disabled_opt(permission.and_then(permissions::denial))
            .disabled_if(workspace::synthetic(m, w), DEMO);
        // A review opened by its command has no draft to go back to; a
        // page always has itself; a review handed over with its form goes
        // back to that form.
        let handed = w.review.as_ref().is_some_and(|review| review.back.is_some());
        return if drafted || w.page.is_some() || handed {
            vec![close, button("network-back", "Back", workspace::back), confirm]
        } else {
            vec![close, confirm]
        };
    }
    if !drafted {
        return vec![close];
    }
    vec![
        close,
        button("network-review", "Review network change", workspace::form_review)
            .primary()
            .disabled_if(!commands::writable(m, w.pool), NOT_WRITABLE),
    ]
}

// -- Properties pages --------------------------------------------------------------

const GENERAL: &str = "general";

/// The pages this area has for a target: General (name and description)
/// for a pool, a server, a storage repository and a network the inventory
/// has; Network Options for a pool; Network Settings for a network. All
/// outside the supported part, as the Rune editors were.
pub fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    let Some(kind) = PropertiesKind::of(target) else {
        return Vec::new();
    };
    let known = target.single().is_some_and(|object| {
        object
            .pool
            .and_then(|pool| m.inventory_for(pool))
            .zip(resource_target(object))
            .is_some_and(|(inventory, target)| editor::display_name(&inventory, target).is_some())
    });
    if !known {
        return Vec::new();
    }
    let ids: &[&'static str] = match kind {
        PropertiesKind::Host | PropertiesKind::Storage => &[GENERAL],
        PropertiesKind::Pool => &[GENERAL, Page::NetworkOptions.id()],
        PropertiesKind::Network => &[GENERAL, Page::NetworkSettings.id()],
        _ => &[],
    };
    ids.iter()
        .map(|id| PropertyPage::planned(kind, id).experimental(true))
        .collect()
}

/// The draft of a page lives in this area's state: created when the page
/// is entered, dropped when it is left.
pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) => {
            drop_pages(m);
            let Some((pool, resource)) = target
                .single()
                .and_then(|object| object.pool.zip(resource_target(object)))
            else {
                return;
            };
            if id == GENERAL {
                match editor::capture(m, pool, resource, Kind::Rename) {
                    Ok(editor) => m.areas.network.editor = Some(editor),
                    Err(error) => m.areas.network.page_error = Some(error),
                }
            } else if id == Page::NetworkSettings.id() {
                let ResourceTarget::Network(network) = resource else {
                    return;
                };
                let value = m
                    .inventory_for(pool)
                    .and_then(|i| i.details.networks.get(&network).and_then(|n| n.mtu))
                    .unwrap_or(1500);
                let form = Form::Mtu {
                    value: value.to_string(),
                };
                let mut w = Workspace::new(pool, Some(network), None, form);
                w.page = Some(Page::NetworkSettings);
                m.areas.network.workspace = Some(w);
            } else if id == Page::NetworkOptions.id() {
                let mut w = Workspace::new(pool, None, None, Form::None);
                w.page = Some(Page::NetworkOptions);
                m.areas.network.workspace = Some(w);
            }
        }
        // Only a page of this area: another area's page that is left says
        // nothing about a page of this one that was just entered.
        PageEvent::Left(id) => {
            if [GENERAL, Page::NetworkSettings.id(), Page::NetworkOptions.id()].contains(&id) {
                drop_pages(m);
            }
        }
        PageEvent::Closed => drop_pages(m),
    }
}

/// Drops whatever of the area's state is a page. A dialog of the area
/// stays.
fn drop_pages(m: &mut Model) {
    let s = &mut m.areas.network;
    if s.editor.as_ref().is_some_and(|e| e.kind == Kind::Rename) {
        s.editor = None;
    }
    if s.workspace.as_ref().is_some_and(|w| w.page.is_some()) {
        s.workspace = None;
    }
    s.page_error = None;
}

/// The page that is showing.
pub fn page(m: &Model, _target: &Target, id: &str) -> PageView {
    let s = &m.areas.network;
    if id == GENERAL {
        if let Some(e) = s.editor.as_ref().filter(|e| e.kind == Kind::Rename) {
            let permission = editor::permission_review(m, e);
            return PageView {
                body: editor_body(m, e),
                footer: editor_footer(m, e, permission.as_ref()),
                footer_note: permission.as_ref().map(permissions::footer_note),
            };
        }
        // Rune reported why the editor could not open in the shared error
        // line; a page says it itself.
        return PageView::new(blocks![
            heading(Kind::Rename.title()),
            s.page_error
                .clone()
                .map(|error| alert(error).key("network-page-error")),
        ])
        .footer([button("resource-close", "Close", close_all)]);
    }
    let Some(w) = s
        .workspace
        .as_ref()
        .filter(|w| w.page.is_some_and(|page| page.id() == id))
    else {
        return PageView::default();
    };
    let permission = workspace::permission(m, w);
    PageView {
        body: workspace_body(m, w),
        footer: workspace_footer(m, w, permission.as_ref()),
        footer_note: permission.as_ref().map(permissions::footer_note),
    }
}
