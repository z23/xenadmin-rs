//! The Home tab: the saved connections with their state, and the commands
//! that act on one of them.
//!
//! This is the Rune Connections dialog (`profiles_view::dialog`) as a tab.
//! The grid lists every saved connection; the row selected in it is the
//! target of the local action row, and its card (session account, status
//! message, last error, route, permissions, last failure) shows under the
//! grid. The buttons each Rune card carried are commands.
//!
//! A saved connection is targeted as its node in the tree: its pool once the
//! session has an inventory, otherwise the `Connection` node
//! (`session::session_object`). [`profile_of`] maps a target back.

use xenadmin_core::{Connection, ConnectionState, CredentialMode, Endpoint, ProfileId};

use super::{reconnect, trust};
use crate::prelude::*;
use crate::session;

/// The key of the grid of saved connections.
pub const TABLE: &str = "connections";

const DEMO: &str = "Demo mode cannot change infrastructure.";
const NOT_OWNER: &str = "This window cannot change saved connections while another window owns the state directory.";
const KEYRING_BUSY: &str = "Wait for the current password-store operation, then retry.";

fn profile_of_object(m: &Model, object: &ObjectRef) -> Option<ProfileId> {
    match object.kind {
        ObjectKind::Connection => {
            session::profile_id(&object.id).filter(|id| m.workspace.profile(*id).is_some())
        }
        ObjectKind::Pool | ObjectKind::Host => session::session_for_pool(m, object.pool?),
        _ => None,
    }
}

/// The saved connection a command target stands for: that of a `Connection`
/// node, or the session through which the pool of a pool or host is shown.
pub fn profile_of(m: &Model, target: &Target) -> Option<ProfileId> {
    profile_of_object(m, target.single()?)
}

/// The saved connections of a target of one or several objects, each once.
/// `None` when the target is empty or some object is not a session's.
fn profiles_of(m: &Model, target: &Target) -> Option<Vec<ProfileId>> {
    if target.is_empty() {
        return None;
    }
    let mut ids = Vec::new();
    for object in &target.objects {
        let id = profile_of_object(m, object)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Some(ids)
}

fn synthetic(m: &Model, id: ProfileId) -> bool {
    m.workspace.synthetic.contains_key(&id)
}

// -- Commands -----------------------------------------------------------------

/// Add Server: the connect form for a new connection.
pub fn add(m: &mut Model, _: &Target) {
    session::begin_new(m);
}

/// Connect (the Rune "Connect / edit"): shown for a saved connection that
/// is not connected and has no attempt waiting.
pub fn can_connect(m: &Model, target: &Target) -> Eligibility {
    let Some(object) = target.single() else {
        return Eligibility::Hidden;
    };
    let Some(id) = profile_of_object(m, object) else {
        return Eligibility::Hidden;
    };
    if m.workspace.loads.contains_key(&id) {
        // The row offers "Cancel connection attempt" instead.
        Eligibility::Hidden
    } else if synthetic(m, id) {
        Eligibility::disabled(DEMO)
    } else if m.workspace.live(id) {
        if object.kind == ObjectKind::Connection {
            Eligibility::disabled("Disconnect this connection before editing its settings.")
        } else {
            // A connected pool or host has nothing to connect.
            Eligibility::Hidden
        }
    } else {
        Eligibility::Enabled
    }
}

pub fn connect(m: &mut Model, target: &Target) {
    if let Some(id) = profile_of(m, target) {
        session::edit(m, id);
    }
}

pub fn can_cancel_connect(m: &Model, target: &Target) -> Eligibility {
    match profiles_of(m, target) {
        Some(ids) if ids.iter().any(|id| m.workspace.loads.contains_key(id)) => {
            Eligibility::Enabled
        }
        _ => Eligibility::Hidden,
    }
}

pub fn cancel_connect(m: &mut Model, target: &Target) {
    for id in profiles_of(m, target).unwrap_or_default() {
        session::cancel_load(m, id);
    }
}

pub fn can_connect_all(m: &Model, _: &Target) -> Eligibility {
    if !m.workspace.writable {
        return Eligibility::disabled(
            m.workspace
                .store
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| NOT_OWNER.into()),
        );
    }
    Eligibility::enabled_if(
        !m.workspace.profiles().is_empty(),
        "There are no saved connections to connect.",
    )
}

pub fn connect_all(m: &mut Model, _: &Target) {
    session::connect_all(m);
}

/// Disconnect: shown while a session of the target is connected, connecting
/// or reconnecting.
pub fn can_disconnect(m: &Model, target: &Target) -> Eligibility {
    match profiles_of(m, target) {
        Some(ids) if ids.iter().any(|id| m.workspace.live(*id)) => Eligibility::Enabled,
        _ => Eligibility::Hidden,
    }
}

pub fn disconnect(m: &mut Model, target: &Target) {
    for id in profiles_of(m, target).unwrap_or_default() {
        if m.workspace.live(id) {
            session::disconnect(m, id);
        }
    }
}

pub fn can_disconnect_all(m: &Model, _: &Target) -> Eligibility {
    Eligibility::enabled_if(
        !(m.workspace.active.is_empty() && m.workspace.loads.is_empty()),
        "No connection is open and no connection attempt is waiting.",
    )
}

pub fn disconnect_all(m: &mut Model, _: &Target) {
    session::disconnect_all(m);
}

pub fn can_reconnect_as(m: &Model, target: &Target) -> Eligibility {
    match profile_of(m, target) {
        Some(id) => Eligibility::enabled_unless(reconnect::refusal(m, id)),
        None => Eligibility::Hidden,
    }
}

pub fn reconnect_as(m: &mut Model, target: &Target) {
    if let Some(id) = profile_of(m, target) {
        reconnect::open(m, id);
    }
}

/// The endpoint whose certificate made the last connection attempt fail.
fn failed_endpoint(m: &Model, id: ProfileId) -> Option<Endpoint> {
    m.workspace
        .active
        .get(&id)?
        .latest()
        .failure()?
        .certificate_endpoint
        .clone()
}

/// "Review this endpoint's certificate…": shown when the last failure names
/// a certificate endpoint and the connection is not live.
pub fn can_review_certificate(m: &Model, target: &Target) -> Eligibility {
    match profile_of(m, target) {
        Some(id) if failed_endpoint(m, id).is_some() && !m.workspace.live(id) => {
            Eligibility::Enabled
        }
        _ => Eligibility::Hidden,
    }
}

pub fn review_certificate(m: &mut Model, target: &Target) {
    if let Some(id) = profile_of(m, target)
        && let Some(endpoint) = failed_endpoint(m, id)
    {
        trust::inspect_failed(m, id, endpoint);
    }
}

pub fn can_remove(m: &Model, target: &Target) -> Eligibility {
    let Some(id) = profile_of(m, target) else {
        return Eligibility::Hidden;
    };
    if m.workspace.loads.contains_key(&id) {
        Eligibility::Hidden
    } else if synthetic(m, id) {
        Eligibility::disabled(DEMO)
    } else if !m.workspace.writable {
        Eligibility::disabled(NOT_OWNER)
    } else if m.workspace.live(id) {
        Eligibility::disabled("Disconnect this connection before removing its saved settings.")
    } else if session::credential_pending(m, id) {
        Eligibility::disabled(KEYRING_BUSY)
    } else {
        Eligibility::Enabled
    }
}

pub fn remove(m: &mut Model, target: &Target) {
    if let Some(id) = profile_of(m, target) {
        session::remove(m, id);
        // A connection without a keyring password is gone at once: its row
        // must not stay the target of the action row until the next poll.
        m.reconcile_selection();
    }
}

/// Forget Password: for a saved connection whose password is in the desktop
/// keyring. In Rune it was a button of the connect form only.
pub fn can_forget_password(m: &Model, target: &Target) -> Eligibility {
    let Some(id) = profile_of(m, target) else {
        return Eligibility::Hidden;
    };
    let keyring = m
        .workspace
        .profile(id)
        .is_some_and(|profile| profile.credential_mode == CredentialMode::SecretService);
    if !keyring {
        Eligibility::Hidden
    } else if synthetic(m, id) {
        Eligibility::disabled(DEMO)
    } else if !m.workspace.writable {
        Eligibility::disabled(NOT_OWNER)
    } else if m.workspace.live(id) {
        Eligibility::disabled("Disconnect this connection before editing its settings.")
    } else if session::credential_pending(m, id) {
        Eligibility::disabled(KEYRING_BUSY)
    } else {
        Eligibility::Enabled
    }
}

pub fn forget_password(m: &mut Model, target: &Target) {
    if let Some(id) = profile_of(m, target) {
        session::forget_password_of(m, id);
    }
}

// -- The tab ------------------------------------------------------------------

fn row_profile(m: &Model, row: &str) -> Option<ProfileId> {
    session::profile_id(row).filter(|id| m.workspace.profile(*id).is_some())
}

/// A press on a row: the saved connection becomes the target of the local
/// action row. The tree scope does not change.
fn select_row(m: &mut Model, row: &str) {
    if let Some(id) = row_profile(m, row) {
        let node = session::session_object(m, id);
        m.select_in_grid(Target::one(node));
    }
}

/// A double click or Enter on a row: the first action the Rune card of the
/// connection offered. "View pool" for a live session, nothing while an
/// attempt waits, otherwise "Connect / edit".
fn activate_row(m: &mut Model, row: &str) {
    let Some(id) = row_profile(m, row) else {
        return;
    };
    if m.workspace.live(id) || synthetic(m, id) {
        session::select(m, id);
    } else if !m.workspace.loads.contains_key(&id) {
        session::edit(m, id);
    }
}

fn rows(m: &Model) -> Vec<Row> {
    session::sessions(m)
        .into_iter()
        .filter_map(|session| {
            let id = session.profile;
            let profile = m.workspace.profile(id)?;
            let account = match m.workspace.session_accounts.get(&id) {
                Some((username, _)) => format!("{} · as {username}", profile.username),
                None => profile.username.clone(),
            };
            let mut message = Vec::new();
            if let Some(status) = m.workspace.profile_status.get(&id) {
                message.push(status.clone());
            }
            if let Some(error) = m
                .workspace
                .active
                .get(&id)
                .and_then(|connection| connection.latest().error().map(str::to_owned))
            {
                message.push(error);
            }
            Some(
                row(
                    id,
                    [
                        profile.name.clone(),
                        profile.endpoint.to_string(),
                        account,
                        session.status.to_owned(),
                        if profile.read_only { "Read-only" } else { "" }.to_owned(),
                        if profile.auto_connect {
                            "Connects automatically"
                        } else {
                            ""
                        }
                        .to_owned(),
                        message.join(" · "),
                    ],
                )
                .tone(if session.state == Some(ConnectionState::Failed) {
                    Tone::Danger
                } else {
                    Tone::Normal
                }),
            )
        })
        .collect()
}

/// The card of one saved connection: every line the Rune Connections dialog
/// showed for it.
fn details(m: &Model, id: ProfileId) -> Option<Group> {
    let profile = m.workspace.profile(id)?;
    let connection = m.workspace.active.get(&id);
    let snapshot = connection.map(Connection::latest);
    let status = if synthetic(m, id) {
        "Demo"
    } else {
        session::status(connection)
    };
    let mut lines = blocks![
        caption(format!(
            "{} · {} · {}{}",
            profile.endpoint,
            profile.username,
            status,
            if profile.read_only {
                " · Read-only"
            } else {
                ""
            }
        ))
        .key(format!("profile-summary-{id}")),
        profile
            .auto_connect
            .then(|| caption("Connects automatically at startup")),
        m.workspace
            .session_accounts
            .get(&id)
            .map(|(username, read_only)| {
                caption(format!(
                    "Current session account: {username}{} · saved account unchanged",
                    if *read_only { " · Read-only" } else { "" }
                ))
            }),
        m.workspace
            .profile_status
            .get(&id)
            .map(|message| caption(message).key(format!("profile-status-{id}"))),
    ];
    if let Some(snapshot) = &snapshot {
        let permissions = snapshot.session_permissions();
        let roles = permissions.roles().join(", ");
        lines.extend(blocks![
            snapshot
                .error()
                .map(|error| text(error).tone(Tone::Danger)),
            snapshot
                .connection_route()
                .map(|route| caption(format!("Last connection route: {route}"))),
            caption(permissions.summary()).key(format!("profile-permissions-{id}")),
            (!roles.is_empty()).then(|| caption(format!("Session roles: {roles}"))),
            snapshot.is_current().then(|| caption("Role changes require a new login. Reconnect as another user to use a different account; no roles are granted automatically.")),
            snapshot.failure().map(|failure| caption(format!(
                "Last failure stage: {}",
                failure.stage.label()
            ))),
        ]);
    }
    Some(group(profile.name.clone(), lines).key("connection-details"))
}

/// The body of the Home tab.
pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if tab != TabId::Home || !scope.is_empty() {
        return None;
    }
    let rows = rows(m);
    // The store error shows only when no synthetic connection replaces the
    // store in the list.
    let store_error = m
        .workspace
        .preview_profiles
        .is_empty()
        .then(|| m.workspace.store.as_ref().err().cloned())
        .flatten();
    let selected = m
        .selection
        .grid
        .single()
        .and_then(|object| profile_of_object(m, object));
    let selected_row = selected.map(|id| id.to_string());
    let grid = (store_error.is_none() || !rows.is_empty()).then(|| {
        table(
            TABLE,
            [
                column("name", "Name"),
                column("endpoint", "Endpoint"),
                column("account", "Account"),
                column("status", "Status"),
                column("access", "Access"),
                column("startup", "At startup"),
                column("message", "Message"),
            ],
            rows,
        )
        .single(selected_row.as_deref(), select_row)
        .on_activate(activate_row)
        .row_menu([
            CommandId("server.connect"),
            CommandId("server.cancel-connect"),
            CommandId("server.disconnect"),
            CommandId("server.reconnect-as"),
            CommandId("server.review-certificate"),
            CommandId("server.forget-password"),
            CommandId("server.remove-connection"),
        ])
        .empty("Save a host address to reconnect to it later. Passwords are requested each session.")
    });
    Some(TabBody::Blocks(blocks![
        caption("Keep several pools connected and switch between them. Same-pool entries share one session; disconnecting either disconnects that pool."),
        store_error.map(|error| notice(NoticeKind::Info, error).key("connections-store")),
        grid,
        selected.and_then(|id| details(m, id)),
        m.workspace
            .credential_status
            .clone()
            .map(|status| caption(status).key("connections-credential-status")),
        // The Connections dialog showed the shared error line: a refused
        // edit or removal is reported where the user pressed.
        m.error
            .clone()
            .map(|error| alert(error).key("connections-error")),
    ]))
}
