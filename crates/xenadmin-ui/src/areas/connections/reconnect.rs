//! Reconnect As: replaces a pool's session with one under other credentials
//! for this session only (the Rune `profile_reconnect.rs`).
//!
//! The saved connection, the keyring and the certificate trust are never
//! written; the session account is kept only in
//! `m.workspace.session_accounts`. Every connection to the same pool is
//! closed, aliases included; other pools are untouched. There is no separate
//! review step: the dialog is the review.

use xenadmin_core::{
    ConnectionProfile, Credentials, ObservedResource, PoolId, ProfileId, ResourceTarget,
};

use crate::prelude::*;
use crate::session;

/// The id and key of the dialog.
pub const DIALOG: &str = "reconnect-dialog";

/// What `apply` and the confirm button say when the saved connection or the
/// pool changed after the dialog opened.
pub const STALE: &str = "Connection settings or pool identity changed; reopen reconnect-as";

const BLANK_ACCOUNT: &str = "Enter the account for this session.";

pub struct Editor {
    /// The stored profile as captured at open.
    pub profile: ConnectionProfile,
    /// The pool as observed at open.
    pub observed: ObservedResource,
    pub pool: PoolId,
    /// The pool's name.
    pub name: String,
    pub username: String,
    /// The session password. Taken out when it is handed to the core.
    pub password: SecretText,
    pub read_only: bool,
    pub error: Option<String>,
}

/// The checks of `open`, with the Rune texts: the reason Reconnect As cannot
/// open for this saved connection now.
fn capture(m: &Model, id: ProfileId) -> Result<Editor, String> {
    let profile = m
        .workspace
        .profile(id)
        .cloned()
        .ok_or("Saved connection is unavailable")?;
    let connection = m
        .workspace
        .active
        .get(&id)
        .ok_or("Connect to this pool first")?;
    let snapshot = connection.latest();
    let inventory = snapshot
        .inventory()
        .ok_or("Wait for current pool inventory")?;
    let pool = inventory.pool_id;
    Ok(Editor {
        profile,
        observed: snapshot
            .observe_resource(ResourceTarget::Pool(pool))
            .map_err(|e| e.to_string())?,
        pool,
        name: inventory.pool_name.clone(),
        username: String::new(),
        password: SecretText::new(),
        read_only: connection.is_read_only(),
        error: None,
    })
}

/// Why Reconnect As cannot open for this saved connection now.
pub fn refusal(m: &Model, id: ProfileId) -> Option<String> {
    capture(m, id).err()
}

/// Opens the dialog for a saved connection. A refusal goes to `m.error`.
pub fn open(m: &mut Model, id: ProfileId) {
    match capture(m, id) {
        Ok(editor) => {
            session::cancel_editor(m);
            m.areas.connections.reconnect = Some(editor);
            m.workspace.show_connect = false;
        }
        Err(error) => m.error = Some(error),
    }
}

/// Opens the dialog for the session through which a pool is reached (the
/// Rune `profiles_view::reconnect_pool`): the scope session when its
/// inventory is that pool, else the first active one that is. The Users tab
/// offers it when the session is not a local superuser.
pub fn reconnect_pool(m: &mut Model, pool: PoolId) {
    if let Some(id) = session::profile_for_pool(m, pool) {
        open(m, id);
    }
}

fn current(m: &Model, e: &Editor) -> bool {
    m.workspace.profile(e.profile.id) == Some(&e.profile)
        && m.workspace.active.get(&e.profile.id).and_then(|c| {
            c.latest()
                .observe_resource(ResourceTarget::Pool(e.pool))
                .ok()
        }) == Some(e.observed)
}

/// The review is stale: the saved connection or the pool changed after the
/// dialog opened.
fn stale(m: &Model, e: &Editor) -> Option<String> {
    (!current(m, e)).then(|| STALE.to_owned())
}

/// Why the reconnect cannot be confirmed whatever is typed: the model is
/// synthetic, the window does not own the state directory, or the review is
/// stale. Rune disabled the button without a reason for the first two.
fn refused(m: &Model, e: &Editor) -> Option<String> {
    if m.demo {
        Some("Demo mode cannot change infrastructure.".into())
    } else if !m.workspace.writable {
        Some("This window cannot change connections while another window owns the state directory.".into())
    } else {
        stale(m, e)
    }
}

/// "Reconnect". On any refusal the typed password is cleared and the reason
/// shown; a refusal before the old sessions are closed leaves the session as
/// it was. The store is never written.
pub fn apply(m: &mut Model) {
    let result = (|| {
        let e = m
            .areas
            .connections
            .reconnect
            .as_ref()
            .ok_or("Reconnect view is closed")?;
        if m.demo || !m.workspace.writable || !current(m, e) {
            return Err(STALE.into());
        }
        m.backend.as_ref().map_err(Clone::clone)?;
        let mut profile = e.profile.clone();
        profile.username = e.username.trim().into();
        profile.read_only = e.read_only;
        profile.pool = Some(e.pool);
        session::trust_options(m, &profile)?;
        // The confirm button is disabled for a blank account; the handler
        // checks again.
        if profile.username.is_empty() {
            return Err(BLANK_ACCOUNT.into());
        }
        if e.password.is_empty()
            || e.password.expose().len() > 65536
            || e.password.expose().contains('\0')
        {
            return Err("Enter a password for this session".into());
        }
        let pool = e.pool;
        // The draft is emptied as the password is handed to the core.
        let secret = m
            .areas
            .connections
            .reconnect
            .as_mut()
            .map(|e| e.password.take())
            .unwrap_or_default();
        let credentials = Credentials::new(profile.username.clone(), secret.expose().to_owned());
        drop(secret);
        let affected: Vec<_> = m
            .workspace
            .active
            .iter()
            .filter(|(_, c)| c.latest().inventory().is_some_and(|i| i.pool_id == pool))
            .map(|(id, c)| (*id, c.clone()))
            .collect();
        let saved: Vec<ProfileId> = m
            .workspace
            .store
            .as_ref()
            .map(|store| {
                store
                    .profiles()
                    .filter(|p| p.pool == Some(pool))
                    .map(|p| p.id)
                    .collect()
            })
            .unwrap_or_default();
        for id in saved {
            session::cancel_load(m, id);
            m.workspace.pending_credentials.remove(&id);
        }
        for (id, c) in &affected {
            session::cancel_load(m, *id);
            m.workspace.pending_credentials.remove(id);
            c.disconnect();
        }
        for (id, _) in affected {
            // The handles of a session that is removed will not change again.
            m.forget_operations_of(id);
            m.workspace.active.remove(&id);
            m.workspace.seen.remove(&id);
            m.workspace.session_accounts.remove(&id);
        }
        session::clear_view(m);
        session::start_connection(m, &profile, credentials)?;
        m.workspace
            .session_accounts
            .insert(profile.id, (profile.username.clone(), profile.read_only));
        session::select(m, profile.id);
        m.areas.connections.reconnect = None;
        Ok::<_, String>(())
    })();
    if let Err(error) = result
        && let Some(e) = &mut m.areas.connections.reconnect
    {
        e.password.clear();
        e.error = Some(error);
    }
}

pub fn close(m: &mut Model) {
    m.areas.connections.reconnect = None;
}

fn edit(m: &mut Model, change: impl FnOnce(&mut Editor)) {
    if let Some(editor) = &mut m.areas.connections.reconnect {
        change(editor);
    }
}

/// The dialog, while the editor is open.
pub fn surface(m: &Model) -> Option<Surface> {
    let e = m.areas.connections.reconnect.as_ref()?;
    let stale = stale(m, e);
    let refused = refused(m, e);
    let body = blocks![
        review(
            "reconnect-review",
            [
                "This closes this pool's sessions and switches the active view. The active console closes; other pool connections remain open. Interrupted operations stay in history.".to_owned(),
                format!(
                    "Saved account: {}. Saved credentials, startup preferences and certificate trust are unchanged. These credentials are used for this session only.",
                    e.profile.username
                ),
            ],
        )
        .affected([
            property("Pool", &e.name),
            property("Pool UUID", e.pool.to_string()).copyable(),
            property("Endpoint", e.profile.endpoint.to_string()),
        ])
        .stale(stale),
        input("reconnect-username", "Session username", &e.username, |m, text| {
            edit(m, |e| e.username = text.to_owned())
        })
        .placeholder("Account for this session"),
        secret_input("reconnect-password", "Session password", &e.password, |m, text| {
            edit(m, |e| e.password.set(text))
        })
        .placeholder("Password · session only"),
        checkbox("reconnect-read-only", "Read-only session", e.read_only, |m| {
            edit(m, |e| e.read_only = !e.read_only)
        }),
        e.error
            .clone()
            .map(|error| alert(error).key("reconnect-error")),
    ];
    Some(
        Surface::dialog(DIALOG, "Reconnect as another account")
            .subtitle(format!("{} ({})", e.name, e.pool))
            .body(body)
            .footer([
                button("reconnect-cancel", "Cancel", close),
                button("reconnect-confirm", "Reconnect", apply)
                    .primary()
                    .disabled_opt(refused)
                    .disabled_if(
                        e.username.trim().is_empty(),
                        BLANK_ACCOUNT,
                    )
                    .disabled_if(e.password.is_empty(), "Enter a password for this session"),
            ])
            .on_close(close),
    )
}
