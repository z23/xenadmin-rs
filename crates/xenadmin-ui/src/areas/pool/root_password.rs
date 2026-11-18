//! Change Server Password: the logic of the Rune `profile_root_password.rs`,
//! carried over. Entry, review with the pool's UUID typed and the
//! permission assessment, submission, settling, and the optional update of
//! the saved password after a verified completion.
//!
//! The editor belongs to one saved connection, not to the tree selection:
//! it was a child of the Rune connection manager, and it stays open after
//! the submission to show the result and to save the password.

use xenadmin_core::{
    ConnectionProfile, CredentialKey, CredentialMode, CredentialOutcome, Credentials, HostId,
    ObservedResource, OperationId, OperationState, PermissionAssessment, PoolId,
    PoolRootPasswordChange, ResourceAction, ResourceOperationRecord, ResourceTarget, RootPassword,
};

use crate::prelude::*;
use crate::session::{self, CredentialJob};

/// The refusal of a submission whose connection moved on.
pub const STALE: &str = "Connection settings or pool identity changed; reopen the password change";

/// What the poll says when the captured connection changed under a draft.
pub const CHANGED: &str =
    "The captured connection changed. Reopen the password change before entering credentials.";

pub const CONFIRM: &str = "Enter the pool UUID exactly";

pub const NOT_ROOT: &str = "Reconnect as local root with a writable session first";

pub const SUBMITTED: &str = "Password change submitted. Closing this view does not cancel it; the result remains on the Events tab of the Notifications view.";

pub struct Editor {
    pub profile: ConnectionProfile,
    pub observed: Option<ObservedResource>,
    pub pool: PoolId,
    pub name: String,
    pub change: PoolRootPasswordChange,
    /// The new password, twice. Zeroized when dropped ([`SecretText`]).
    pub password: SecretText,
    pub repeated: SecretText,
    pub confirmation: String,
    pub review: bool,
    /// "Update this profile's saved password after verified completion".
    pub save: bool,
    /// The new password, kept only when `save` applies, until the operation
    /// settles.
    pub candidate: Option<SecretText>,
    pub operation: Option<OperationId>,
    /// The terminal or unknown state was handled once.
    pub settled: bool,
    /// The keyring update in flight.
    pub saving: Option<CredentialJob>,
    pub message: Option<String>,
}

impl Editor {
    /// An editor over literal values, for scenes and tests: no connection
    /// stands behind it.
    pub fn fixture(
        profile: ConnectionProfile,
        pool: PoolId,
        name: &str,
        coordinator: HostId,
        members: Vec<HostId>,
    ) -> Self {
        Self {
            profile,
            observed: None,
            pool,
            name: name.into(),
            change: PoolRootPasswordChange {
                coordinator,
                members,
            },
            password: SecretText::new(),
            repeated: SecretText::new(),
            confirmation: String::new(),
            review: false,
            save: false,
            candidate: None,
            operation: None,
            settled: false,
            saving: None,
            message: None,
        }
    }
}

/// The editor for a pool, or why there is none. The saved connection is
/// the scope session when it shows this pool, else the first that does.
pub fn capture(m: &Model, pool: PoolId) -> Result<Editor, String> {
    let shows = |c: &xenadmin_core::Connection| {
        c.latest().inventory().is_some_and(|i| i.pool_id == pool)
    };
    let id = m
        .workspace
        .selected
        .filter(|id| m.workspace.active.get(id).is_some_and(shows))
        .or_else(|| {
            m.workspace
                .active
                .iter()
                .find_map(|(id, c)| shows(c).then_some(*id))
        })
        .ok_or("A saved, current connection is required")?;
    let profile = m
        .workspace
        .profile(id)
        .cloned()
        .ok_or("Save this connection first")?;
    let connection = m.workspace.active.get(&id).ok_or("Connection closed")?;
    let account = m
        .workspace
        .session_accounts
        .get(&id)
        .map(|v| v.0.as_str())
        .unwrap_or(&profile.username);
    if account != "root" || connection.is_read_only() {
        return Err(NOT_ROOT.into());
    }
    let snapshot = connection.latest();
    let inventory = snapshot.inventory().ok_or("Wait for current inventory")?;
    let observed = snapshot
        .observe_resource(ResourceTarget::Pool(pool))
        .map_err(|e| e.to_string())?;
    let coordinator = inventory
        .coordinator
        .ok_or("Pool coordinator is unavailable")?;
    let name = inventory.pool_name.clone();
    let members = inventory.hosts.keys().copied().collect();
    let mut editor = Editor::fixture(profile, pool, &name, coordinator, members);
    editor.observed = Some(observed);
    Ok(editor)
}

/// The Rune `open`, for the pool of the command's target. A refusal goes
/// to the shared error line.
pub fn open(m: &mut Model, pool: PoolId) {
    match capture(m, pool) {
        Ok(editor) => {
            super::close_all(m);
            m.areas.pool.root_password = Some(editor);
        }
        Err(error) => m.error = Some(error),
    }
}

pub fn close(m: &mut Model) {
    m.areas.pool.root_password = None;
}

/// The Rune `s.demo`: the scope session is synthetic, or the pool is shown
/// by a synthetic session.
pub fn synthetic(m: &Model, e: &Editor) -> bool {
    m.demo || m.is_synthetic_pool(e.pool)
}

pub fn current(m: &Model, e: &Editor) -> bool {
    !synthetic(m, e)
        && m.workspace.writable
        && m.workspace.profile(e.profile.id) == Some(&e.profile)
        && e.observed.is_some()
        && m.workspace.active.get(&e.profile.id).is_some_and(|c| {
            !c.is_read_only()
                && c.latest()
                    .observe_resource(ResourceTarget::Pool(e.pool))
                    .ok()
                    == e.observed
        })
}

pub fn review(m: &mut Model) {
    let Some(e) = &mut m.areas.pool.root_password else {
        return;
    };
    let result = (|| {
        RootPassword::new(e.password.expose().to_owned())
            .map_err(|_| "Enter a nonempty password without line breaks (maximum 4096 bytes)")?;
        if e.password != e.repeated {
            return Err("The new passwords do not match");
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            e.review = true;
            e.message = None;
        }
        Err(error) => e.message = Some(error.into()),
    }
}

pub fn back(m: &mut Model) {
    if let Some(e) = &mut m.areas.pool.root_password {
        e.review = false;
    }
}

/// The assessment of the change for the session of the captured connection.
pub fn permission(m: &Model, e: &Editor) -> PermissionAssessment {
    ResourceAction::RootPassword(e.change.clone())
        .permission_requirements(ResourceTarget::Pool(e.pool))
        .assess(
            &m.workspace
                .active
                .get(&e.profile.id)
                .map(|c| c.latest().session_permissions())
                .unwrap_or_default(),
        )
}

/// The saved password of this profile can be updated: a keyring profile of
/// `root`.
pub fn saves(e: &Editor) -> bool {
    e.profile.username == "root" && e.profile.credential_mode == CredentialMode::SecretService
}

pub fn submit(m: &mut Model) {
    let result = (|| {
        let e = m
            .areas
            .pool
            .root_password
            .as_ref()
            .ok_or("Password view closed")?;
        if !current(m, e) || !e.review || e.operation.is_some() {
            return Err(STALE.to_owned());
        }
        let permission = permission(m, e);
        if permission.decision().is_denied() {
            return Err(permission.explanation());
        }
        if e.confirmation.trim() != e.pool.to_string() {
            return Err(CONFIRM.into());
        }
        if !m.workspace.credential_tasks.is_empty() {
            return Err(
                "Wait for pending password-store operations before changing the password".into(),
            );
        }
        let c = m
            .workspace
            .active
            .get(&e.profile.id)
            .ok_or("Connection closed")?;
        let op = c
            .change_pool_root_password(
                OperationId::new(),
                e.observed.ok_or(STALE)?,
                e.change.clone(),
                RootPassword::new(e.password.expose().to_owned()).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        let pool = e.pool;
        // Credentials of the old password are dropped for every saved
        // connection of this pool, connected or not.
        let mut ids: std::collections::BTreeSet<_> = m
            .workspace
            .active
            .iter()
            .filter_map(|(id, c)| {
                c.latest()
                    .inventory()
                    .is_some_and(|i| i.pool_id == pool)
                    .then_some(*id)
            })
            .collect();
        if let Ok(store) = &m.workspace.store {
            ids.extend(
                store
                    .profiles()
                    .filter(|p| p.pool == Some(pool))
                    .map(|p| p.id),
            );
        }
        if m.workspace
            .cached_credentials
            .as_ref()
            .is_some_and(|(p, _)| ids.contains(&p.id))
        {
            m.workspace.cached_credentials = None;
        }
        for id in ids {
            session::cancel_load(m, id);
            m.workspace.pending_credentials.remove(&id);
        }
        let id = m.track_resource(op);
        if let Some(e) = m.areas.pool.root_password.as_mut() {
            if e.save && saves(e) {
                e.candidate = Some(e.password.take());
            }
            e.password.clear();
            e.repeated.clear();
            e.operation = Some(id);
            e.message = Some(SUBMITTED.into());
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = result
        && let Some(e) = &mut m.areas.pool.root_password
    {
        e.message = Some(error);
    }
}

pub fn can_save(m: &Model, e: &Editor) -> bool {
    current(m, e) && saves(e) && m.workspace.credential_tasks.is_empty()
}

/// Only a completion the host verified may update the saved password: not
/// an unknown outcome, not an acknowledged one, not an interrupted one.
pub fn verified_completion(record: &ResourceOperationRecord) -> bool {
    matches!(
        record.state,
        OperationState::Succeeded | OperationState::PartiallyCompleted
    ) && !record.was_interrupted()
        && record.root_password.as_ref().is_some_and(|p| {
            p.coordinator_changed && p.coordinator_login_verified && p.reconnect_credentials_updated
        })
}

/// True when what the dialog shows changed.
pub fn poll(m: &mut Model) -> bool {
    let Some(e) = m.areas.pool.root_password.as_ref() else {
        return false;
    };
    if let Some(result) = e.saving.as_ref().and_then(CredentialJob::take_result) {
        let message = match result {
            Ok(CredentialOutcome::Saved) => "Verified coordinator credentials saved for this exact connection profile. Other saved profiles and pool-member passwords still require separate review.".into(),
            Ok(_) => "Password store returned an unexpected result. The host change remains completed; update the saved password manually.".into(),
            Err(error) => format!(
                "Coordinator password changed, but the saved password was not updated: {error}"
            ),
        };
        if let Some(e) = m.areas.pool.root_password.as_mut() {
            e.saving = None;
            e.message = Some(message);
        }
        return true;
    }
    if e.operation.is_none() {
        if !synthetic(m, e)
            && !current(m, e)
            && (e.review || !e.password.is_empty() || !e.repeated.is_empty())
            && let Some(e) = m.areas.pool.root_password.as_mut()
        {
            e.password.clear();
            e.repeated.clear();
            e.review = false;
            e.message = Some(CHANGED.into());
            return true;
        }
        return false;
    }
    if e.settled {
        return false;
    }
    // The handle while the kernel still holds it, else the merged record:
    // the kernel drops the handle of a finished operation once its last
    // record is in `resource_rows`, which is before this poll runs.
    let Some(record) = m
        .resource_operations
        .iter()
        .map(|o| o.latest().as_ref().clone())
        .find(|r| Some(r.id) == e.operation)
        .or_else(|| {
            m.resource_rows
                .iter()
                .find(|r| Some(r.id) == e.operation)
                .cloned()
        })
    else {
        return false;
    };
    if !record.state.terminal() && record.state != OperationState::OutcomeUnknown {
        return false;
    }
    let save = can_save(m, e) && verified_completion(&record);
    let waker = m.waker.clone();
    let Some(e) = m.areas.pool.root_password.as_mut() else {
        return false;
    };
    e.settled = true;
    e.message = Some(record.detail.clone());
    if let Some(candidate) = e.candidate.take() {
        if save {
            e.saving = Some(CredentialJob::save(
                CredentialKey::for_profile(&e.profile),
                Credentials::new("root".into(), candidate.expose().to_owned()),
                waker,
            ));
            e.message = Some(format!(
                "{} Updating this profile's saved password…",
                record.detail
            ));
        } else {
            e.message = Some(format!(
                "{} The saved password was not changed. Use an explicit reconnect to verify access, then update the saved profile manually.",
                record.detail
            ));
        }
    }
    true
}
