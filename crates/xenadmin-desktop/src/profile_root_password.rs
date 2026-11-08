use super::*;
use xenadmin_core::{
    ObservedResource, OperationId, PoolRootPasswordChange, ResourceTarget, RootPassword,
};
use zeroize::{Zeroize, Zeroizing};

pub(super) struct Editor {
    profile: ConnectionProfile,
    observed: Option<ObservedResource>,
    pool: PoolId,
    name: String,
    change: PoolRootPasswordChange,
    password: TextEdit,
    repeated: TextEdit,
    confirmation: TextEdit,
    review: bool,
    save: bool,
    candidate: Option<Zeroizing<String>>,
    operation: Option<OperationId>,
    settled: bool,
    saving: Option<CredentialJob>,
    message: Option<String>,
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.password.text.zeroize();
        self.repeated.text.zeroize();
    }
}
fn capture(s: &State) -> Result<Editor, String> {
    let pool = s
        .inventory
        .as_ref()
        .ok_or("Connect to the pool first")?
        .pool_id;
    let id = s
        .workspace
        .selected
        .filter(|id| {
            s.workspace
                .active
                .get(id)
                .is_some_and(|c| c.latest().inventory().is_some_and(|i| i.pool_id == pool))
        })
        .or_else(|| {
            s.workspace.active.iter().find_map(|(id, c)| {
                c.latest()
                    .inventory()
                    .is_some_and(|i| i.pool_id == pool)
                    .then_some(*id)
            })
        })
        .ok_or("A saved, current connection is required")?;
    let profile = s
        .workspace
        .profile(id)
        .cloned()
        .ok_or("Save this connection first")?;
    let connection = s.workspace.active.get(&id).ok_or("Connection closed")?;
    let account = s
        .workspace
        .session_accounts
        .get(&id)
        .map(|v| v.0.as_str())
        .unwrap_or(&profile.username);
    if account != "root" || connection.is_read_only() {
        return Err("Reconnect as local root with a writable session first".into());
    }
    let snapshot = connection.latest();
    let inventory = snapshot.inventory().ok_or("Wait for current inventory")?;
    Ok(Editor {
        profile,
        observed: Some(
            snapshot
                .observe_resource(ResourceTarget::Pool(pool))
                .map_err(|e| e.to_string())?,
        ),
        pool,
        name: inventory.pool_name.clone(),
        change: PoolRootPasswordChange {
            coordinator: inventory
                .coordinator
                .ok_or("Pool coordinator is unavailable")?,
            members: inventory.hosts.keys().copied().collect(),
        },
        password: secure_field(),
        repeated: secure_field(),
        confirmation: TextEdit::new(""),
        review: false,
        save: false,
        candidate: None,
        operation: None,
        settled: false,
        saving: None,
        message: None,
    })
}
pub(super) fn open(s: &mut State) {
    match capture(s) {
        Ok(editor) => {
            s.workspace.root_password = Some(editor);
            s.workspace.join = None;
            s.workspace.reconnect = None;
            s.workspace.show_manager = true;
            s.pool_editor = None;
            s.pool_security = None;
        }
        Err(error) => s.error = Some(error),
    }
}
fn current(s: &State, e: &Editor) -> bool {
    !s.demo
        && s.workspace.writable
        && s.workspace.profile(e.profile.id) == Some(&e.profile)
        && e.observed.is_some()
        && s.workspace.active.get(&e.profile.id).is_some_and(|c| {
            !c.is_read_only()
                && c.latest()
                    .observe_resource(ResourceTarget::Pool(e.pool))
                    .ok()
                    == e.observed
        })
}
fn review(s: &mut State) {
    let Some(e) = &mut s.workspace.root_password else {
        return;
    };
    let result = (|| {
        RootPassword::new(e.password.text.clone())
            .map_err(|_| "Enter a nonempty password without line breaks (maximum 4096 bytes)")?;
        if e.password.text != e.repeated.text {
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
fn permission(s: &State, e: &Editor) -> xenadmin_core::PermissionAssessment {
    xenadmin_core::ResourceAction::RootPassword(e.change.clone())
        .permission_requirements(ResourceTarget::Pool(e.pool))
        .assess(
            &s.workspace
                .active
                .get(&e.profile.id)
                .map(|c| c.latest().session_permissions())
                .unwrap_or_default(),
        )
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s
            .workspace
            .root_password
            .as_ref()
            .ok_or("Password view closed")?;
        if !current(s, e) || !e.review || e.operation.is_some() {
            return Err(
                "Connection settings or pool identity changed; reopen the password change".into(),
            );
        }
        let permission = permission(s, e);
        if permission.decision().is_denied() {
            return Err(permission.explanation());
        }
        if e.confirmation.text.trim() != e.pool.to_string() {
            return Err("Enter the pool UUID exactly".into());
        }
        if !s.workspace.credential_tasks.is_empty() {
            return Err(
                "Wait for pending password-store operations before changing the password".into(),
            );
        }
        let c = s
            .workspace
            .active
            .get(&e.profile.id)
            .ok_or("Connection closed")?;
        let op = c
            .change_pool_root_password(
                OperationId::new(),
                e.observed.unwrap(),
                e.change.clone(),
                RootPassword::new(e.password.text.clone()).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        if let Some(wake) = &s.waker {
            op.notify_on_change(wake.clone());
        }
        let pool = e.pool;
        let mut ids: std::collections::BTreeSet<_> = s
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
        if let Ok(store) = &s.workspace.store {
            ids.extend(
                store
                    .profiles()
                    .filter(|p| p.pool == Some(pool))
                    .map(|p| p.id),
            );
        }
        if s.workspace
            .cached_credentials
            .as_ref()
            .is_some_and(|(p, _)| ids.contains(&p.id))
        {
            s.workspace.cached_credentials = None;
        }
        for id in ids {
            cancel_load(s, id);
            s.workspace.pending_credentials.remove(&id);
        }
        let e = s.workspace.root_password.as_mut().unwrap();
        if e.save
            && e.profile.username == "root"
            && e.profile.credential_mode == CredentialMode::SecretService
        {
            e.candidate = Some(Zeroizing::new(std::mem::take(&mut e.password.text)));
        }
        e.password.text.zeroize();
        e.repeated.text.zeroize();
        e.password = secure_field();
        e.repeated = secure_field();
        e.operation = Some(op.latest().id);
        e.message=Some("Password change submitted. Closing this view does not cancel it; the result remains in operation history.".into());
        s.resource_operations.push(op);
        Ok::<_, String>(())
    })();
    if let Err(error) = result
        && let Some(e) = &mut s.workspace.root_password
    {
        e.message = Some(error);
    }
}
fn can_save(s: &State, e: &Editor) -> bool {
    current(s, e)
        && e.profile.username == "root"
        && e.profile.credential_mode == CredentialMode::SecretService
        && s.workspace.credential_tasks.is_empty()
}
fn verified_completion(record: &xenadmin_core::ResourceOperationRecord) -> bool {
    matches!(
        record.state,
        xenadmin_core::OperationState::Succeeded
            | xenadmin_core::OperationState::PartiallyCompleted
    ) && !record.was_interrupted()
        && record.root_password.as_ref().is_some_and(|p| {
            p.coordinator_changed && p.coordinator_login_verified && p.reconnect_credentials_updated
        })
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(e) = s.workspace.root_password.as_ref() else {
        return false;
    };
    if let Some(result) = e.saving.as_ref().and_then(CredentialJob::take_result) {
        let e = s.workspace.root_password.as_mut().unwrap();
        e.saving = None;
        e.message=Some(match result{Ok(CredentialOutcome::Saved)=>"Verified coordinator credentials saved for this exact connection profile. Other saved profiles and pool-member passwords still require separate review.".into(),Ok(_)=>"Password store returned an unexpected result. The host change remains completed; update the saved password manually.".into(),Err(error)=>format!("Coordinator password changed, but the saved password was not updated: {error}")});
        return true;
    }
    if e.operation.is_none() {
        if !s.demo
            && !current(s, e)
            && (e.review || !e.password.text.is_empty() || !e.repeated.text.is_empty())
        {
            let e = s.workspace.root_password.as_mut().unwrap();
            e.password.text.zeroize();
            e.repeated.text.zeroize();
            e.review = false;
            e.message=Some("The captured connection changed. Reopen the password change before entering credentials.".into());
            return true;
        }
        return false;
    }
    if e.settled {
        return false;
    }
    let Some(record) = s
        .resource_operations
        .iter()
        .map(|o| o.latest())
        .find(|r| Some(r.id) == e.operation)
    else {
        return false;
    };
    if !record.state.terminal() && record.state != xenadmin_core::OperationState::OutcomeUnknown {
        return false;
    }
    let save = can_save(s, e) && verified_completion(&record);
    let e = s.workspace.root_password.as_mut().unwrap();
    e.settled = true;
    e.message = Some(record.detail.clone());
    if let Some(candidate) = e.candidate.take() {
        if save {
            e.saving = Some(CredentialJob::save(
                CredentialKey::for_profile(&e.profile),
                Credentials::new("root".into(), candidate.to_string()),
                s.waker.clone(),
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
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.workspace.root_password.as_ref().unwrap();
    let permission = (e.review && e.operation.is_none()).then(|| permission(s, e));
    let denied = permission
        .as_ref()
        .is_some_and(|p| p.decision().is_denied());
    let mut body=col().gap(12.0).child(heading("Change pool root password",k))
        .child(caption(format!("{}\nPool: {}\nCoordinator: {}\n{} pool host(s) · local account: root",e.name,e.pool,e.change.coordinator,e.change.members.len()),k).wrap_anywhere())
        .child(caption("This changes the coordinator's local root password and requests synchronization to every pool member. Individual member failures are not reported by XCP-ng. A successful coordinator login does not verify the other hosts.",k).wrap())
        .child(caption("The current client's reconnect password changes only after a verified coordinator login. Existing connections that share this pool's old credentials close. Other saved profiles are not updated.",k).wrap());
    if e.operation.is_none() {
        if e.review {
            body=body.child(heading("Review password change",k)).child(caption(if e.save{"After verified completion, update the existing saved root password for this connection profile only."}else{"Use the new password for this session only. The desktop password store is unchanged."},k).wrap())
                .child(caption(format!("Type the pool UUID to confirm: {}",e.pool),k).wrap_anywhere())
                .child(TextInput::new(&e.confirmation).label("Confirmation pool UUID").on_edit(|s:&mut State|&mut s.workspace.root_password.as_mut().unwrap().confirmation).build(k).key("root-password-confirmation"));
        } else {
            body = body
                .child(
                    TextInput::new(&e.password)
                        .secure()
                        .label("New root password")
                        .on_edit(|s: &mut State| {
                            &mut s.workspace.root_password.as_mut().unwrap().password
                        })
                        .build(k)
                        .key("root-password-new"),
                )
                .child(
                    TextInput::new(&e.repeated)
                        .secure()
                        .label("Repeat new root password")
                        .on_edit(|s: &mut State| {
                            &mut s.workspace.root_password.as_mut().unwrap().repeated
                        })
                        .build(k)
                        .key("root-password-repeat"),
                );
            if e.profile.username == "root"
                && e.profile.credential_mode == CredentialMode::SecretService
            {
                body = body.child(
                    Checkbox::new(e.save)
                        .label("Update this profile's saved password after verified completion")
                        .on_toggle(|s: &mut State| {
                            let e = s.workspace.root_password.as_mut().unwrap();
                            e.save = !e.save;
                        })
                        .build(k)
                        .key("root-password-save"),
                );
            }
        }
    }
    if let Some(message) = &e.message {
        body = body.child(caption(message, k).wrap().role("alert"));
    }
    let mut footer = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .on_press(|s: &mut State| s.workspace.root_password = None)
            .build(k)
            .key("root-password-close"),
    );
    if e.operation.is_none() {
        if e.review {
            footer = footer
                .child(
                    Button::new("Back")
                        .on_press(|s: &mut State| {
                            s.workspace.root_password.as_mut().unwrap().review = false
                        })
                        .build(k),
                )
                .child(
                    Button::new("Change password")
                        .disabled(
                            !current(s, e)
                                || denied
                                || e.confirmation.text.trim() != e.pool.to_string(),
                        )
                        .on_press(submit)
                        .build(k)
                        .key("root-password-submit"),
                );
        } else {
            footer = footer.child(
                Button::new("Review change")
                    .on_press(review)
                    .build(k)
                    .key("root-password-review"),
            );
        }
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .fill(rune::Color::BLACK.alpha(0.5))
        .center()
        .role("dialog")
        .key("root-password-dialog")
        .label("Change pool root password")
        .modal(false, |s: &mut State| s.workspace.root_password = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 700.0))
                .h((s.window_height - 56.0).clamp(280.0, 760.0))
                .padding(20.0)
                .gap(16.0)
                .fill(k.c.bg_base)
                .radius(10.0)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(session_permissions_view::footer(
                    footer,
                    permission.as_ref(),
                    (s.window_width - 32.0).clamp(280.0, 700.0) - 40.0,
                    k,
                )),
        )
}
pub(super) fn scene(s: &mut State) {
    let pool = PoolId::parse("11111111-1111-1111-1111-111111111111").unwrap();
    let coordinator = HostId::parse("22222222-2222-2222-2222-222222222222").unwrap();
    let mut profile = ConnectionProfile::new(
        "Fixture pool".into(),
        Endpoint::parse("192.0.2.10").unwrap(),
        "root".into(),
    );
    profile.credential_mode = CredentialMode::SecretService;
    s.workspace.root_password = Some(Editor {
        profile,
        observed: None,
        pool,
        name: "Fixture pool".into(),
        change: PoolRootPasswordChange {
            coordinator,
            members: vec![
                coordinator,
                HostId::parse("44444444-4444-4444-4444-444444444444").unwrap(),
            ],
        },
        password: secure_field(),
        repeated: secure_field(),
        confirmation: TextEdit::new(""),
        review: true,
        save: false,
        candidate: None,
        operation: None,
        settled: false,
        saving: None,
        message: None,
    });
    s.demo = true;
    s.workspace.show_manager = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_positive_evidence_never_saves_unknown_acknowledged_or_interrupted_completion() {
        for (state, interrupted, allowed) in [
            ("Succeeded", false, true),
            ("PartiallyCompleted", false, true),
            ("OutcomeUnknown", false, false),
            ("AcknowledgedUnknown", true, false),
            ("PartiallyCompleted", true, false),
            ("Succeeded", true, false),
        ] {
            let mut s = crate::init();
            scene(&mut s);
            let e = s.workspace.root_password.as_ref().unwrap();
            let record:xenadmin_core::ResourceOperationRecord=serde_json::from_value(serde_json::json!({
                "id":OperationId::new(),"pool":e.pool,"target":{"Pool":e.pool},"action":{"RootPassword":e.change},
                "state":state,"interrupted":interrupted,"progress":null,"detail":"Fixture result","created_at_ms":1,
                "can_cancel":false,"cancel_requested":false,"steps":[],"created":null,
                "root_password":{"coordinator_changed":true,"coordinator_login_verified":true,"reconnect_credentials_updated":true,"unverified_members":[]}
            })).unwrap();
            assert_eq!(
                verified_completion(&record),
                allowed,
                "{state}, interrupted={interrupted}"
            );
        }
    }
    #[test]
    fn review_is_pool_scoped_masked_and_footer_stays_reachable() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let mut runtime = rune::Runtime::new(
                s,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let (x, y) = runtime.find("root-password-submit").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
            assert!(!runtime.state.workspace.root_password.as_ref().unwrap().save);
            assert!(
                runtime.find("root-password-new").is_none(),
                "Review must not render the secret again"
            );
        }
    }
    #[test]
    fn mismatched_passwords_and_stale_profile_clear_without_submission() {
        let mut s = crate::init();
        scene(&mut s);
        let e = s.workspace.root_password.as_mut().unwrap();
        e.review = false;
        e.password.text = "synthetic-first".into();
        e.repeated.text = "synthetic-other".into();
        review(&mut s);
        assert!(!s.workspace.root_password.as_ref().unwrap().review);
        s.workspace.root_password.as_mut().unwrap().repeated.text = "synthetic-first".into();
        review(&mut s);
        assert!(s.workspace.root_password.as_ref().unwrap().review);
        s.demo = false;
        assert!(poll(&mut s));
        let e = s.workspace.root_password.as_ref().unwrap();
        assert!(e.password.text.is_empty() && e.repeated.text.is_empty());
        assert!(!e.review);
        assert!(!can_save(&s, e));
        submit(&mut s);
        assert!(s.resource_operations.is_empty());
    }
    #[test]
    fn password_store_failure_preserves_host_result_and_never_retries_automatically() {
        let mut s = crate::init();
        scene(&mut s);
        let job = CredentialJob::save(
            CredentialKey::for_profile(&s.workspace.root_password.as_ref().unwrap().profile),
            Credentials::new("root".into(), "fixture".into()),
            None,
        );
        job.result
            .replace(Some(Err(xenadmin_core::CredentialStoreError::Locked)));
        s.workspace.root_password.as_mut().unwrap().saving = Some(job);
        assert!(poll(&mut s));
        let e = s.workspace.root_password.as_ref().unwrap();
        assert!(e.saving.is_none());
        assert!(
            e.message
                .as_ref()
                .unwrap()
                .contains("Coordinator password changed")
        );
        assert!(e.message.as_ref().unwrap().contains("locked"));
        assert!(!poll(&mut s));
        assert!(s.workspace.credential_tasks.is_empty());
    }
}
