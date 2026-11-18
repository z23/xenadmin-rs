//! Area `connections`: the connection surfaces (plan packages P2.9, P3.8).
//!
//! Rune modules: `connection_dialog` of `main.rs`, the views of
//! `profiles_view.rs`, `profile_trust.rs`, `profile_reconnect.rs`. The
//! logic of saved connections and their sessions is the kernel's
//! (`crate::session`); this area describes what shows of it and holds the
//! parts of the Rune modules that were not on the connection path.
//!
//! - [`home`]: the Home tab (the Rune Connections dialog): the grid of saved
//!   connections with its local action row, and the commands.
//! - [`connect`]: Add Server (`connect-dialog`): the connect form with its
//!   saved-connection options and, in the same dialog, the certificate
//!   review.
//! - [`trust`]: inspecting a certificate and accepting its pin.
//! - [`reconnect`]: Reconnect As (`reconnect-dialog`).
//!
//! Session renewal and a coordinator change show as the status word of the
//! connection ("Reconnecting…", "Connected"), its last error, route and
//! failure stage on the Home tab; a certificate that changed shows as a
//! failed connection with `server.review-certificate`, which opens the
//! connect form on the endpoint that failed.
//!
//! No surface here is experimental: Rune shows no notice for the connect
//! form, the Connections dialog or Reconnect As.

use crate::prelude::*;

pub mod connect;
pub mod home;
pub mod reconnect;
mod scenes;
pub mod trust;

#[cfg(test)]
mod tests;

const AREA: &str = "connections";

/// The state of the area. `m.areas.connections`.
///
/// The draft of the connect form and the certificate inspection are in
/// `m.workspace` (the connection path of `session.rs` reads them); the row
/// selected on the Home tab is the grid selection of the model.
#[derive(Default)]
pub struct State {
    /// The open Reconnect As dialog.
    pub reconnect: Option<reconnect::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands,
        scenes: scenes::scenes,
        surfaces,
        tab: home::tab,
        poll,
        ..AreaSpec::new(AREA)
    }
}

/// The kinds that stand for a session: its pool, a host of it, or the saved
/// connection while it has no inventory.
const SESSION: &[ObjectKind] = &[ObjectKind::Pool, ObjectKind::Host, ObjectKind::Connection];
/// The kinds that stand for a saved connection.
const SAVED: &[ObjectKind] = &[ObjectKind::Pool, ObjectKind::Connection];

/// The commands. Those of the plan's menus carry its labels; the others
/// keep the label of the Rune button they replace.
///
/// Disconnect and Reconnect As are in both the Pool and the Server menu of
/// the plan, and a command has one menu position: `pool.disconnect` and
/// `pool.reconnect-as` are the same functions under a second id.
fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("server.add", "Add Server…", AREA)
            .menu(place::SERVER_ADD)
            .toolbar(ToolbarSlot::ADD_SERVER)
            .context(&[ObjectKind::None])
            .local(TabId::Home, 10)
            .local_scopes(&[ObjectKind::None])
            .invoke(home::add),
        CommandSpec::new("server.connect", "Connect…", AREA)
            .menu(place::SERVER_CONNECT)
            .context(SAVED)
            .local(TabId::Home, 20)
            .local_scopes(&[ObjectKind::None])
            .eligibility(home::can_connect)
            .invoke(home::connect),
        CommandSpec::new("server.cancel-connect", "Cancel connection attempt", AREA)
            .context(SAVED)
            .local(TabId::Home, 30)
            .local_scopes(&[ObjectKind::None])
            .multi()
            .eligibility(home::can_cancel_connect)
            .invoke(home::cancel_connect),
        CommandSpec::new("server.disconnect", "Disconnect", AREA)
            .menu(place::SERVER_DISCONNECT)
            .context(SESSION)
            .local(TabId::Home, 40)
            .local_scopes(&[ObjectKind::None])
            .multi()
            .eligibility(home::can_disconnect)
            .invoke(home::disconnect),
        CommandSpec::new("pool.disconnect", "Disconnect", AREA)
            .menu(place::POOL_DISCONNECT)
            .multi()
            .eligibility(home::can_disconnect)
            .invoke(home::disconnect),
        CommandSpec::new("server.reconnect-as", "Reconnect As…", AREA)
            .menu(place::SERVER_RECONNECT_AS)
            .context(SESSION)
            .local(TabId::Home, 50)
            .local_scopes(&[ObjectKind::None])
            .eligibility(home::can_reconnect_as)
            .invoke(home::reconnect_as),
        CommandSpec::new("pool.reconnect-as", "Reconnect As…", AREA)
            .menu(place::POOL_RECONNECT_AS)
            .eligibility(home::can_reconnect_as)
            .invoke(home::reconnect_as),
        CommandSpec::new(
            "server.review-certificate",
            "Review this endpoint's certificate…",
            AREA,
        )
        .context(SAVED)
        .local(TabId::Home, 60)
        .local_scopes(&[ObjectKind::None])
        .eligibility(home::can_review_certificate)
        .invoke(home::review_certificate),
        CommandSpec::new("server.forget-password", "Forget Password", AREA)
            .menu(place::SERVER_PASSWORD_FORGET)
            .context(SAVED)
            .eligibility(home::can_forget_password)
            .invoke(home::forget_password),
        CommandSpec::new("server.remove-connection", "Remove connection", AREA)
            .context(SAVED)
            .local(TabId::Home, 70)
            .local_scopes(&[ObjectKind::None])
            .eligibility(home::can_remove)
            .invoke(home::remove),
        CommandSpec::new("server.connect-all", "Connect All", AREA)
            .menu(place::SERVER_CONNECT_ALL)
            .local(TabId::Home, 80)
            .local_scopes(&[ObjectKind::None])
            .eligibility(home::can_connect_all)
            .invoke(home::connect_all),
        CommandSpec::new("server.disconnect-all", "Disconnect All", AREA)
            .menu(place::SERVER_DISCONNECT_ALL)
            .local(TabId::Home, 90)
            .local_scopes(&[ObjectKind::None])
            .eligibility(home::can_disconnect_all)
            .invoke(home::disconnect_all),
    ]
}

/// The connect form, and in front of it Reconnect As (in Rune the
/// Connections layer drew over the connect form; the two are never open
/// together, since opening one closes the other).
fn surfaces(m: &Model) -> Vec<Surface> {
    connect::surface(m)
        .into_iter()
        .chain(reconnect::surface(m))
        .collect()
}

/// The part of the Rune `profiles_view::refresh` that left the kernel:
/// `trust::poll`.
fn poll(m: &mut Model, _clock: f64) -> bool {
    trust::poll(m)
}
