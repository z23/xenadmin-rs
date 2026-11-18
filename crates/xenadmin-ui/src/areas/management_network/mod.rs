//! Area `management_network`: the management interface of a server and its
//! bonds (plan W3).
//!
//! Rune modules: `management_network_ui.rs` (management IP: static, DHCP,
//! autoconfiguration; moving management to another interface),
//! `management_bond_ui.rs` (a bond under IPv4 management, with the
//! asynchronous guest-impact query) and `management_ipv6_bond_ui.rs` (the
//! five-stage IPv6 management bond handover, its continuation and "leave
//! partial"). In Rune they were children of the "Host networking" dialog of
//! `network_admin.rs`, which is the area `network`: their buttons sat on the
//! card of a management interface in its hub, their forms were variants of
//! its `Form`, and its review and submission were shared.
//!
//! Now: the buttons are commands, the forms are this area's dialog
//! `management-change`, and a complete review is handed over to the shared
//! review dialog of `network` (`network::open_review`, with the Rune
//! staleness rule of the child as its `Check`): `network-change`, with the
//! Rune key `network-confirm` and the shared `submit`.
//!
//! ## Commands
//!
//! The subject of the first five is the management interface of a server:
//! of the server that is the target (context menu, the scope of the tab),
//! or the interface selected in the interface grid of a Networking tab
//! (`network::selected_pif`) when the target is a pool or a network.
//!
//! | Rune entry point | Command | Label | Where | Opens |
//! | --- | --- | --- | --- | --- |
//! | `management-ip` "Change management IP…" | `server.management.configure` | Configure Management IP | local row of Networking (210), context menu of a server | `management-change`, form |
//! | `management-move` "Move management…" | `server.management.move` | Move Management Interface | local row of Networking (220), context menu of a server | `management-change`, form |
//! | `management-bond-create` "Create management bond…" | `server.management.bond.create` | Create Management Bond | local row of NICs (210), context menu of a server | `management-change`, form |
//! | `management-bond-remove` "Remove management bond…" | `server.management.bond.remove` | Remove Management Bond | local row of NICs (220), context menu of a server | the guest-impact query, then the review |
//! | `ipv6-management-bond` "Staged IPv6 management bond…" | `server.management.bond.ipv6` | Staged IPv6 Management Bond | local row of NICs (230), context menu of a server | `management-change`, form |
//! | `ipv6-bond-continue-<op>` "Review stage {n}…" | `server.management.bond.ipv6.continue` | Review Next IPv6 Handover Stage (dynamic: "Review IPv6 Handover Stage {n}") | local rows of NICs and Networking (240), context menu of a server and a pool | the review of the next stage |
//! | `ipv6-bond-leave-<op>` "Leave partial configuration…" | `server.management.bond.ipv6.leave` | Leave Partial IPv6 Handover | local rows of NICs and Networking (250), context menu of a server and a pool | the review of leaving |
//!
//! None is in the menu bar: the plan's Menus table has no item for them.
//! Eligibility is in `commands.rs`.
//!
//! ## Surfaces
//!
//! | Id | Kind | Experimental | What |
//! | --- | --- | --- | --- |
//! | `management-change` | dialog | yes | the form of a management change (IP settings, move, bond, IPv6 bond), the wait for the guest impact of a bond, and the continuation panel of an unfinished IPv6 handover |
//! | `network-change` (area `network`) | dialog | yes | every review of this area, and its submission |
//!
//! Each stage of the IPv6 handover is one review and one submission, as in
//! Rune: stage 1 from the form, every later stage from the journal record
//! of its predecessor (`ipv6::handovers`).
//!
//! ## Scenes
//!
//! `management-network`, `management-dhcp`, `management-vlan`,
//! `management-bond`, `management-vlan-bond`, `management-bond-guests`,
//! `management-ipv6-bond` (`scenes.rs`): each ends on its review.
//!
//! ## Tests
//!
//! The header of `tests.rs` accounts for the twenty Rune tests.
//!
//! ## For other areas
//!
//! - [`continuation_notes`]: the captions of that panel alone, which the
//!   `network` area shows on the NICs and Networking tabs of a pool or
//!   server (the two buttons are commands of the local row there).
//! - [`continuation`]: the blocks of the Rune continuation panel for a pool
//!   (the caption of each unfinished IPv6 handover with its two buttons),
//!   for a tab or a record list that wants to show it;
//! - [`ipv6::handovers`]: the unfinished handovers of a pool, as data;
//! - [`close`];
//! - the command ids of `commands.rs`.
//!
//! ## What has no successor here, and why
//!
//! - **Back from the review to the form.** Rune's Back cleared the review
//!   and showed the form again. The shared review dialog of `network` has
//!   no Back for a review that was handed over, so this area's form is
//!   closed by the hand-over: to change an entry, close the review and
//!   open the command again.
//! - The check marks and plus signs in the labels of the Rune member,
//!   destination, mode and hash buttons: these are checkboxes and choices
//!   now. The Rune keys of the buttons are the keys of the checkboxes and
//!   of the options.
//! - The continuation panel was drawn in every state of the networking
//!   dialog. It is drawn in `management-change`, and its two buttons are
//!   also commands that need no dialog; the caption of the handover
//!   (stages verified, temporary recovery endpoint) is only in the panel.
//! - The Rune "Remove management bond…" button was also shown when the
//!   management interface could not be captured, and then did nothing: the
//!   command is hidden there.

use xenadmin_core::{
    Error, HostId, ManagementNetworkOperation, PoolId, ResourceAction, ResourceTarget,
};

use crate::areas::network;
use crate::prelude::*;

pub mod bond;
pub mod commands;
pub mod ipv6;
pub mod management;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

pub use view::{continuation, continuation_notes};

/// The state of the area. `m.areas.management_network`.
#[derive(Default)]
pub struct State {
    /// The open management change, while it is a form or waits for the
    /// guest impact. Its review is the `network` area's.
    pub editor: Option<Editor>,
}

pub struct Editor {
    pub pool: PoolId,
    /// The server whose management interface changes.
    pub host: HostId,
    pub form: Form,
    /// The guest-impact query of a bond review that has not answered yet
    /// (Rune `bond_review` and `ipv6_review` of the workspace).
    pub pending: Option<Pending>,
    pub error: Option<String>,
}

pub enum Form {
    Management(Box<management::Editor>),
    Bond(Box<bond::Editor>),
    /// The removal of a management bond has nothing to enter (Rune went
    /// from the hub straight to the guest-impact query, with the form
    /// still `Browse`): the operation that waits for its guest impact.
    Removal(Box<ManagementNetworkOperation>),
    Ipv6(Box<ipv6::Editor>),
}

pub enum Pending {
    Bond(bond::Pending),
    Ipv6(ipv6::Pending),
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        poll,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("management_network")
    }
}

/// Close, Escape, another object or another session: the form and a
/// pending query are dropped (a late answer finds nothing to fill).
pub fn close(m: &mut Model) {
    m.areas.management_network.editor = None;
}

/// The Rune `network_admin::poll`: both pending queries, always evaluated.
fn poll(m: &mut Model, _clock: f64) -> bool {
    let ipv6 = ipv6::poll(m);
    let bond = bond::poll(m);
    ipv6 || bond
}

/// The Rune `set_form`: the form replaces whatever the area and the
/// networking area had open, without review, pending query or error.
pub(crate) fn open_form(m: &mut Model, pool: PoolId, host: HostId, form: Form) {
    network::close(m);
    m.areas.management_network.editor = Some(Editor {
        pool,
        host,
        form,
        pending: None,
        error: None,
    });
}

/// An error of the open form (Rune `edit(s).error`); without a form (a
/// command that opens none, the continuation of a handover) the shared
/// error line.
pub(crate) fn report(m: &mut Model, error: String) {
    match &mut m.areas.management_network.editor {
        Some(e) => e.error = Some(error),
        None => m.error = Some(error),
    }
}

/// The Rune `prepare_action`: a complete review. It is handed over to the
/// shared review dialog of `network`, and this area's form closes. When
/// the target cannot be observed the form stays, with the error, as in
/// Rune.
pub(crate) fn stage(
    m: &mut Model,
    pool: PoolId,
    target: ResourceTarget,
    operation: ResourceAction,
    lines: Vec<String>,
    check: network::Check,
) {
    if !m.is_synthetic_pool(pool)
        && let Err(error) = m
            .snapshot_for(pool)
            .ok_or(Error::Stale)
            .and_then(|snap| snap.observe_resource(target))
    {
        report(m, error.to_string());
        return;
    }
    let host = match target {
        ResourceTarget::Host(host) => Some(host),
        _ => None,
    };
    close(m);
    network::open_review(m, pool, None, host, target, operation, lines, Some(check));
}
