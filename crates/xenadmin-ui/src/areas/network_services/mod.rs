//! Area `network_services`: the network services, topology and fabric
//! workflows (plan W3).
//!
//! Rune modules: `network_service_ui.rs` (child `service` of the networking
//! workspace of `network_admin.rs`: the NBD service purposes of a network),
//! `network_topology_ui.rs` (`topology`: pool-wide VLAN creation and
//! retagging), `network_fabric_ui.rs` (`fabric`: a GRE or VXLAN tunnel or
//! SR-IOV on one host, the coverage view, removal) and
//! `network_fabric_pool_ui.rs` (`fabric_pool`: coverage of every host of
//! the pool, and its removal). Tabs: none (Networking is the `network`
//! area's).
//!
//! In Rune these were sections of buttons in the hub of the "Host
//! networking" dialog and forms of its one workspace. The hub is gone: each
//! button is a command on a network, and what a command opens is a dialog
//! or a wizard of this area, or, for a change with nothing to enter, the
//! shared review dialog `network-change` of the `network` area.
//!
//! ## Commands
//!
//! All on the local row of the Networking tab (orders 110 to 180, after
//! the `network` area's) and in the context menu of a network.
//!
//! | Rune entry point | Command | Label | Opens |
//! | --- | --- | --- | --- |
//! | `topology-create` "Create pool-wide VLANs…" | `network.vlan.create-pool` | Create Pool-wide VLANs | dialog `network-topology`, form |
//! | `topology-retag` "Retag pool-wide VLANs…" | `network.vlan.retag-pool` | Retag Pool-wide VLANs | dialog `network-topology`, form |
//! | "Enable / Remove NBD with TLS…" | `network.service.nbd-tls` | Enable NBD with TLS / Remove NBD with TLS | `network-change` (shared), review |
//! | "Enable / Remove NBD without TLS…" | `network.service.nbd-plaintext` | Enable NBD without TLS / Remove NBD without TLS | `network-change` (shared), review |
//! | "Add GRE tunnel interface…", "Add VXLAN tunnel interface…", "Enable SR-IOV on one host…" | `network.fabric.add` | Add Tunnel or SR-IOV Interface | wizard `network-fabric`; the three buttons are its Type step |
//! | "Complete GRE / VXLAN / SR-IOV coverage…" | `network.fabric.complete-pool` | Complete Pool Coverage | wizard `network-fabric-pool`; the three buttons are its Type step |
//! | "Review removal…" (one per record) | `network.fabric.remove` | Remove Tunnel or SR-IOV Interface | dialog `network-fabric-remove`: the records, then the review |
//! | "{Remove pool tunnel interfaces / Remove pool SR-IOV interfaces}…" | `network.fabric.remove-pool` | the label of the operation (Remove Pool Coverage) | `network-change` (shared), review |
//!
//! ## Surfaces (all experimental, as the Rune dialog was)
//!
//! | Id | Kind | What |
//! | --- | --- | --- |
//! | `network-topology` | dialog | the form of a pool-wide VLAN change, then its review |
//! | `network-fabric` | wizard: Type, Name, Settings, Review | a tunnel or SR-IOV interface on one host |
//! | `network-fabric-pool` | wizard: Type, Name, Settings, Review | coverage of every host of the pool |
//! | `network-fabric-remove` | dialog | the records of the network, then the review of one removal |
//!
//! The plan puts the fabric types into "New Network: Wizard: Type, Name,
//! Settings, Review". That wizard is the `network` area's and creates the
//! internal type only; a tunnel or SR-IOV interface is added to a network
//! that exists (the core has no operation that creates both). So the two
//! wizards here have the plan's step names: Type is the kind, Name shows
//! the network the command was invoked on, Settings are the inputs.
//!
//! Scenes: `network-services`, `network-topology`, `network-fabric`,
//! `network-sriov`, `network-fabric-pool`, `network-fabric-pool-select`
//! (`scenes.rs`). Tests: the header of `tests.rs`.
//!
//! ## For other areas
//!
//! - [`networking_sections`]: the information the Rune hub showed for the
//!   chosen network without a button, "Network services" (the current
//!   purposes, why a change is unavailable) and "Tunnel and SR-IOV
//!   coverage" (the classes, and per host its records). It belongs on the
//!   Networking tab of a network, which the `network` area describes; until
//!   that tab calls it, the coverage is on the Name step of the wizards
//!   and the service state is in the labels and reasons of the two service
//!   commands.
//! - [`commands::ALL`]: the ids, for a row menu of the network grid.
//! - [`service::CHECK`], [`topology::CHECK`], [`fabric::CHECK`]: the Rune
//!   staleness rules as `network::Check`s of a review in the shared dialog.
//! - [`close`].
//!
//! ## What has no successor here, and where it went
//!
//! - The check marks on the Rune choice buttons: the selected option of a
//!   choice.
//! - The caption under a disabled candidate button: the reason and the
//!   detail of the disabled option.
//! - "Back" from a review of a service purpose or of a pool removal: these
//!   never had a form; their review is opened by its command and has no
//!   Back (the rule of every change with nothing to enter).
//! - `topology_export_contains_only_public_created_and_removed_ids`: the
//!   export is the `events` area's, which ported the test.
//!
//! The files: [`service`], [`topology`], [`fabric`] (the logic of the four
//! Rune modules, carried over), [`edit`] (the open workflow: draft, review,
//! submission), [`commands`], [`view`], [`scenes`], `tests`.

use xenadmin_core::{Inventory, NetworkId};

use crate::prelude::*;

pub mod commands;
pub mod edit;
pub mod fabric;
pub mod scenes;
pub mod service;
pub mod topology;
pub mod view;

// TESTS

/// The state of the area. `m.areas.network_services`.
#[derive(Default)]
pub struct State {
    /// The open workflow with a draft: one of the four surfaces of `view`.
    pub editor: Option<edit::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new("network_services")
    }
}

/// Close, Escape, a successful submission, another object or pool in
/// scope: nothing of the draft is kept.
pub fn close(m: &mut Model) {
    m.areas.network_services.editor = None;
}

/// The information blocks of this area for the Networking tab of a
/// network: the Rune `service::view` and `fabric::view` without their
/// buttons, which are the commands.
pub fn networking_sections(i: &Inventory, network: NetworkId) -> Vec<Block> {
    blocks![
        service::section(i, network),
        fabric::coverage(i, network),
    ]
}
