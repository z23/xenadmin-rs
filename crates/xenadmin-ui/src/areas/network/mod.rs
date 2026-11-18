//! Area `network`: host and pool networking (plan W3).
//!
//! Rune modules: `network_admin.rs` (the "Host networking" dialog),
//! `network_interfaces.rs` (its child for secondary interfaces) and the
//! generic editor of `resource_admin.rs` (rename, internal network, storage
//! rescan, host scheduling, default storage). Tabs: Networking of a pool, a
//! server and a network; NICs of a server.
//!
//! The Rune "Host networking" dialog was a hub: buttons to forms and to
//! one-click reviews, and one card per interface with more buttons. The hub
//! is gone. What it listed is on the tabs as grids, each button is a
//! command, and what a command opens is a dialog with its form or review,
//! or a page of a Properties dialog.
//!
//! ## Commands
//!
//! | Rune entry point | Command | Label | Where | Opens |
//! | --- | --- | --- | --- | --- |
//! | `create-network` "New network" | `network.new` | Add Network | local row of Networking | wizard `network-new` |
//! | "Add VLAN interface…" | `network.vlan.add` | Add VLAN Interface | local row of Networking, context menu of a network | `network-change`, form |
//! | `resource-Delete internal network` | `network.remove` | Remove Network | local row of Networking, context menu of a network | `network-remove`, review |
//! | "IPv4 configuration…" | `nic.ipv4` | Configure IPv4 | local row of Networking, row menu of an interface | `network-change`, form |
//! | "IPv6 configuration…" | `nic.ipv6` | Configure IPv6 | the same | `network-change`, form |
//! | "Interface purpose…" | `nic.purpose` | Interface Purpose | the same | `network-change`, form |
//! | "Plug interface" / "Unplug interface" | `nic.plug` | Plug Interface / Unplug Interface | row menu of an interface | `network-change`, review |
//! | "Protect from unplugging…" / "Allow unplugging…" | `nic.unplug-protection` | Protect from Unplugging / Allow Unplugging | row menu | `network-change`, review |
//! | "Delete VLAN interface…" | `nic.vlan.delete` | Delete VLAN Interface | row menu | `network-change`, review |
//! | "Change to {mode}…" | `server.bond.mode` | Change Bond Mode | row menu | `network-change`, a choice of the other modes |
//! | "Use {hash} hash…" | `server.bond.hash` | Change Bond Hash | row menu | `network-change`, a choice of the other hashes |
//! | "Create bond…" | `server.bond.create` | Create Bond | local row of NICs | `network-change`, form |
//! | "Destroy bond…" | `server.bond.delete` | Delete Bond | local row of NICs, row menu | `network-change`, review |
//! | "Scan interfaces · {host}" | `server.nics.rescan` | Rescan | local row of NICs | `network-change`, review |
//! | `resource-Enable/Disable host scheduling` | `server.scheduling` | Disable / Enable New VM Placement | Server menu, context menu of a server | `server-scheduling`, review |
//! | "Change default storage…" (pool hub) | `storage.set-default` | Set as Default | Storage menu, context menu of a repository, local row of Storage | `storage-set-default`, review |
//! | `resource-Rescan storage repository` | `storage.rescan` | Rescan | Storage menu, context menu of a repository, local row of Storage | `storage-rescan`, review |
//!
//! ## Properties pages (the dialogs are the kernel's)
//!
//! | Rune | Dialog | Page |
//! | --- | --- | --- |
//! | `resource-Edit name and description`, "Edit pool name and description" | pool, server, storage repository, network | General |
//! | "Change MTU…" and `resource-Change internal network MTU` | network | Network Settings (one field; the review is the internal one for a network without interfaces, see `workspace::Form::Mtu`) |
//! | "Enable / Disable automatic attachment" | network | Network Settings |
//! | "Enable / Disable pool IGMP snooping" | pool | Network Options |
//!
//! Every surface and page of the area is experimental, as the two Rune
//! editors were (`experimental_dialog_open`).
//!
//! The General page is offered for all four kinds because the Rune rename
//! had all four. For a server this is the first real General page of the
//! dialog: tests of `host_admin` that expect Multipathing to be the first
//! page, and tests of the kernel whose test area claims the id `general`
//! for a server, do not hold with it (`view::pages` decides, one line).
//!
//! ## Surfaces
//!
//! | Id | Kind | What |
//! | --- | --- | --- |
//! | `network-new` | wizard: Type, Name, Settings, Review | create an internal network |
//! | `network-change` | dialog | the form or the review of a network change |
//! | `network-remove` | dialog, on the review | delete an internal network |
//! | `server-scheduling` | dialog, on the review | enable or disable host scheduling |
//! | `storage-set-default` | dialog, on the review | choose default storage |
//! | `storage-rescan` | dialog, on the review | rescan a storage repository |
//!
//! Scenes: `network-interfaces`, `network-ipv6`, `network-create`,
//! `pool-administration` (`scenes.rs`). Tests: the header of `tests.rs`.
//!
//! ## For the areas whose Rune modules were children of the workspace
//!
//! `network_services` (service, topology, fabric) and `management_network`
//! (management interface, bonds) register their commands themselves, with
//! `.local(TabId::Networking, n)` or `.local(TabId::Nics, n)` and the
//! context kind; this area does not name them. Orders taken in the local
//! rows: Networking 10 to 60, NICs 10 to 30. What they need of this area
//! is public:
//!
//! - [`selected_pif`]: the interface selected in the interface grid of the
//!   scope's Networking or NICs tab (the target of a command on an
//!   interface, which is no tree object); a network is an ordinary command
//!   target;
//! - [`open_review`] with a [`Check`]: the shared review dialog
//!   `network-change` for a complete `ResourceAction` (network, management
//!   network, management IPv6 bond, network topology), with the Rune
//!   staleness rule of the child as the check, and the shared `submit`;
//! - [`open_staged`]: the same for a `NetworkOperation` with nothing more
//!   to check (the Rune `review_button`);
//! - [`name`], [`pif_name`], [`writable`] (the Rune `enabled`), [`close`];
//! - `scenes::prepare`: the inventory of the Rune `network_admin::scene`,
//!   from which their scenes started;
//! - `m.areas.network.workspace` ([`Workspace`], all fields public) for a
//!   scene or a test that inspects the review.
//!
//! Their forms and their pending guest-impact reviews are their own state
//! and surfaces: the `Form` here has only the variants of these modules.
//!
//! ## What has no successor here, and where it went
//!
//! - The hub, its opening button `network-administration-open`, its list
//!   of network buttons (the selection) and its heading: the tabs.
//! - "Operation history" in the footer of the hub: the button
//!   `network-operation-history` under the grids, which shows the History
//!   list of the pool in the Events tab. The line "Latest operation" is
//!   beside it.
//! - The chooser of the default-storage dialog: the command runs on the
//!   repository that is selected.
//! - Of `resource_admin.rs`: `selected_target` and `poll` are the kernel's;
//!   the history, Recheck, Acknowledge, cancellation and the operation
//!   notice are the `events` area's; the pool hub ("Pool administration")
//!   with its buttons is dissolved into the commands of the areas they
//!   belong to.
//! - The pending bond reviews, the IPv6 continuation panel and the forms
//!   `Fabric`, `FabricPool`, `Topology`, `Management`, `ManagementBond`
//!   and `Ipv6Bond`: `network_services` and `management_network`.
//!
//! The files: [`workspace`] and [`editor`] (the logic carried over),
//! [`commands`], [`view`] (surfaces and pages), [`tab`], [`info`] (the
//! information blocks of the tabs), [`scenes`], `tests`.

use crate::prelude::*;

pub mod commands;
pub mod editor;
mod info;
pub mod scenes;
pub mod tab;
pub mod view;
pub mod workspace;

#[cfg(test)]
mod tests;

pub use commands::{selected_pif, writable};
pub use workspace::{Check, Staged, Workspace, name, open_review, open_staged, pif_name};

/// The state of the area. `m.areas.network`.
#[derive(Default)]
pub struct State {
    /// The open network change: a dialog, or a page of a Properties dialog
    /// (`Workspace::page`).
    pub workspace: Option<workspace::Workspace>,
    /// The open change of the generic editor: a dialog, or (name and
    /// description) the General page of a Properties dialog.
    pub editor: Option<editor::Editor>,
    /// Why the General page has no editor (the Rune reasons of `open`).
    pub page_error: Option<String>,
    /// The selected row of the interface grid of the Networking or NICs
    /// tab: the id of a physical interface.
    pub pif: Option<String>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: tab::tab,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: view::page_event,
        on_scope_change: leave,
        on_session_change: leave,
        ..AreaSpec::new("network")
    }
}

/// Close, Escape, a successful submission, another surface of the area
/// opening: nothing of an editor is kept. An editor that is a page takes
/// the Properties dialog with it.
pub fn close(m: &mut Model) {
    let s = &mut m.areas.network;
    let workspace = s.workspace.take().is_some_and(|w| w.page.is_some());
    let editor = s
        .editor
        .take()
        .is_some_and(|e| e.kind == editor::Kind::Rename);
    let error = s.page_error.take().is_some();
    if workspace || editor || error {
        m.close_properties();
    }
}

/// Another object selected in the tree, another pool shown: the editors
/// and the grid's selection belonged to the previous one.
fn leave(m: &mut Model) {
    close(m);
    m.areas.network.pif = None;
}
