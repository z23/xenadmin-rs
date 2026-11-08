use super::*;
use xenadmin_core::{ObservedResource, ResourceTarget};

pub(super) struct Editor {
    profile: ConnectionProfile,
    observed: ObservedResource,
    pool: PoolId,
    name: String,
    username: TextEdit,
    password: TextEdit,
    read_only: bool,
    error: Option<String>,
}
pub(super) fn open(s: &mut State, id: ProfileId) {
    let result = (|| {
        let profile = s
            .workspace
            .profile(id)
            .cloned()
            .ok_or("Saved connection is unavailable")?;
        let snapshot = s
            .workspace
            .active
            .get(&id)
            .ok_or("Connect to this pool first")?
            .latest();
        let inventory = snapshot
            .inventory()
            .ok_or("Wait for current pool inventory")?;
        let pool = inventory.pool_id;
        Ok::<_, String>(Editor {
            profile,
            observed: snapshot
                .observe_resource(ResourceTarget::Pool(pool))
                .map_err(|e| e.to_string())?,
            pool,
            name: inventory.pool_name.clone(),
            username: TextEdit::new(""),
            password: secure_field(),
            read_only: s.workspace.active.get(&id).unwrap().is_read_only(),
            error: None,
        })
    })();
    match result {
        Ok(editor) => {
            cancel_editor(s);
            s.workspace.reconnect = Some(editor);
            s.workspace.show_manager = true;
            s.show_connect = false;
            s.pool_security = None;
        }
        Err(error) => s.error = Some(error),
    }
}
fn current(s: &State, e: &Editor) -> bool {
    s.workspace.profile(e.profile.id) == Some(&e.profile)
        && s.workspace.active.get(&e.profile.id).and_then(|c| {
            c.latest()
                .observe_resource(ResourceTarget::Pool(e.pool))
                .ok()
        }) == Some(e.observed)
}
fn apply(s: &mut State) {
    let result = (|| {
        let e = s
            .workspace
            .reconnect
            .as_ref()
            .ok_or("Reconnect view is closed")?;
        if s.demo || !s.workspace.writable || !current(s, e) {
            return Err("Connection settings or pool identity changed; reopen reconnect-as".into());
        }
        s.backend.as_ref().map_err(Clone::clone)?;
        let mut profile = e.profile.clone();
        profile.username = e.username.text.trim().into();
        profile.read_only = e.read_only;
        profile.pool = Some(e.pool);
        trust::options(s, &profile)?;
        if e.password.text.is_empty()
            || e.password.text.len() > 65536
            || e.password.text.contains('\0')
        {
            return Err("Enter a password for this session".into());
        }
        let pool = e.pool;
        let credentials = Credentials::new(
            profile.username.clone(),
            std::mem::take(&mut s.workspace.reconnect.as_mut().unwrap().password.text),
        );
        let affected: Vec<_> = s
            .workspace
            .active
            .iter()
            .filter(|(_, c)| c.latest().inventory().is_some_and(|i| i.pool_id == pool))
            .map(|(id, c)| (*id, c.clone()))
            .collect();
        let saved: Vec<_> = s
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
            cancel_load(s, id);
            s.workspace.pending_credentials.remove(&id);
        }
        for (id, c) in &affected {
            cancel_load(s, *id);
            s.workspace.pending_credentials.remove(id);
            c.disconnect();
        }
        for (id, _) in affected {
            s.workspace.active.remove(&id);
            s.workspace.seen.remove(&id);
            s.workspace.session_accounts.remove(&id);
        }
        clear_view(s);
        start_connection(s, &profile, credentials)?;
        s.workspace
            .session_accounts
            .insert(profile.id, (profile.username.clone(), profile.read_only));
        select(s, profile.id);
        s.workspace.reconnect = None;
        Ok::<_, String>(())
    })();
    if let Err(error) = result
        && let Some(e) = &mut s.workspace.reconnect
    {
        e.password = secure_field();
        e.error = Some(error);
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.workspace.reconnect.as_ref().unwrap();
    let mut body=col().gap(12.0).child(heading("Reconnect as another account",k))
        .child(caption(format!("{}\n{}\n{}",e.name,e.pool,e.profile.endpoint),k).wrap_anywhere())
        .child(caption("This closes this pool's sessions and switches the active view. The active console closes; other pool connections remain open. Interrupted operations stay in history.",k).wrap())
        .child(caption(format!("Saved account: {}. Saved credentials, startup preferences and certificate trust are unchanged. These credentials are used for this session only.",e.profile.username),k).wrap())
        .child(TextInput::new(&e.username).label("Session username").placeholder("Account for this session").on_edit(|s:&mut State|&mut s.workspace.reconnect.as_mut().unwrap().username).build(k).key("reconnect-username"))
        .child(TextInput::new(&e.password).secure().label("Session password").placeholder("Password · session only").on_edit(|s:&mut State|&mut s.workspace.reconnect.as_mut().unwrap().password).build(k).key("reconnect-password"))
        .child(Checkbox::new(e.read_only).label("Read-only session").on_toggle(|s:&mut State|{let e=s.workspace.reconnect.as_mut().unwrap();e.read_only = !e.read_only;}).build(k));
    if let Some(error) = &e.error {
        body = body.child(caption(error, k).wrap().role("alert"));
    }
    let footer = row()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.workspace.reconnect = None)
                .build(k),
        )
        .child(
            Button::new("Reconnect")
                .disabled(
                    s.demo
                        || !s.workspace.writable
                        || !current(s, e)
                        || e.username.text.trim().is_empty()
                        || e.password.text.is_empty(),
                )
                .on_press(apply)
                .build(k)
                .key("reconnect-confirm"),
        );
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Reconnect as another account")
        .modal(false, |s: &mut State| s.workspace.reconnect = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 620.0))
                .h((s.window_height - 56.0).clamp(280.0, 600.0))
                .padding(20.0)
                .gap(16.0)
                .fill(k.c.bg_base)
                .radius(10.0)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, Read, Write},
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    struct Server {
        endpoint: Endpoint,
        stop: Arc<AtomicBool>,
        logins: Arc<Mutex<Vec<(String, String)>>>,
    }
    impl Server {
        fn new(pool: PoolId) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint =
                Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let logins = Arc::new(Mutex::new(Vec::new()));
            let signal = stop.clone();
            let seen = logins.clone();
            std::thread::spawn(move || {
                while !signal.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let logins = seen.clone();
                            std::thread::spawn(move || {
                                stream
                                    .set_read_timeout(Some(Duration::from_secs(2)))
                                    .unwrap();
                                let mut input = std::io::BufReader::new(stream);
                                let mut length = 0;
                                loop {
                                    let mut line = String::new();
                                    if input.read_line(&mut line).unwrap_or(0) == 0 {
                                        return;
                                    }
                                    if line == "\r\n" {
                                        break;
                                    }
                                    if let Some(value) =
                                        line.to_ascii_lowercase().strip_prefix("content-length:")
                                    {
                                        length = value.trim().parse::<usize>().unwrap();
                                    }
                                }
                                if length > 65536 {
                                    return;
                                }
                                let mut bytes = vec![0; length];
                                if input.read_exact(&mut bytes).is_err() {
                                    return;
                                }
                                let request: serde_json::Value =
                                    serde_json::from_slice(&bytes).unwrap();
                                let method = request["method"].as_str().unwrap();
                                let result = match method {
                                    "session.login_with_password" => {
                                        logins.lock().unwrap().push((
                                            request["params"][0].as_str().unwrap().into(),
                                            request["params"][1].as_str().unwrap().into(),
                                        ));
                                        serde_json::json!("OpaqueRef:fixture-session")
                                    }
                                    "event.from" => {
                                        if request["params"][2] != "" {
                                            std::thread::sleep(Duration::from_millis(100));
                                            serde_json::json!({"token":"fixture","events":[]})
                                        } else {
                                            serde_json::json!({"token":"fixture","events":[{"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":pool.to_string(),"name_label":"Fixture pool"}}]})
                                        }
                                    }
                                    "session.logout" => serde_json::Value::Null,
                                    _ => panic!("Unexpected fixture request: {method}"),
                                };
                                let body = serde_json::to_vec(
                                    &serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                                )
                                .unwrap();
                                let mut stream = input.into_inner();
                                let header = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = stream.write_all(header.as_bytes());
                                let _ = stream.write_all(&body);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                endpoint,
                stop,
                logins,
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release)
        }
    }
    fn ready(c: &Connection) {
        let start = Instant::now();
        while !c.latest().is_current() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "{:?}",
                c.latest().error()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn connected(
        f: &mut super::super::tests::Fixture,
        server: &Server,
        pool: PoolId,
    ) -> ConnectionProfile {
        let s = &mut f.state;
        s.workspace.restore_started = true;
        s.backend = Ok(Backend::with_state_directory(f.directory.clone()).unwrap());
        let mut profile =
            ConnectionProfile::new("Fixture".into(), server.endpoint.clone(), "operator".into());
        profile.pool = Some(pool);
        profile.read_only = true;
        profile.credential_mode = CredentialMode::SecretService;
        profile.auto_connect = true;
        s.workspace
            .store
            .as_mut()
            .unwrap()
            .save(profile.clone())
            .unwrap();
        start_connection(
            s,
            &profile,
            Credentials::new("operator".into(), "synthetic-original".into()),
        )
        .unwrap();
        ready(s.workspace.active.get(&profile.id).unwrap());
        refresh(s);
        select(s, profile.id);
        s.workspace.profile(profile.id).unwrap().clone()
    }
    #[test]
    fn reconnect_is_ephemeral_cancels_same_pool_aliases_and_retains_other_pool() {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let other_pool = PoolId::from(uuid::Uuid::new_v4());
        let other = Server::new(other_pool);
        let mut f = super::super::tests::Fixture::new();
        let profile = connected(&mut f, &server, pool);
        let s = &mut f.state;
        let mut alias = profile.clone();
        alias.id = ProfileId::new();
        s.workspace
            .store
            .as_mut()
            .unwrap()
            .save(alias.clone())
            .unwrap();
        let old = s.workspace.active[&profile.id].clone();
        s.workspace.active.insert(alias.id, old.clone());
        let mut other_profile =
            ConnectionProfile::new("Other pool".into(), other.endpoint.clone(), "other".into());
        other_profile.pool = Some(other_pool);
        s.workspace
            .store
            .as_mut()
            .unwrap()
            .save(other_profile.clone())
            .unwrap();
        start_connection(
            s,
            &other_profile,
            Credentials::new("other".into(), "synthetic-other".into()),
        )
        .unwrap();
        ready(&s.workspace.active[&other_profile.id]);
        let other_generation = s.workspace.active[&other_profile.id].latest().generation();
        open(s, profile.id);
        let e = s.workspace.reconnect.as_mut().unwrap();
        e.username = TextEdit::new("root");
        e.password = TextEdit::new("synthetic-elevated");
        e.read_only = false;
        apply(s);
        assert!(s.workspace.reconnect.is_none());
        ready(&s.workspace.active[&profile.id]);
        refresh(s);
        assert_eq!(s.workspace.profile(profile.id), Some(&profile));
        assert!(!s.workspace.active.contains_key(&alias.id));
        assert_eq!(
            s.workspace.active[&other_profile.id].latest().generation(),
            other_generation
        );
        assert!(!s.workspace.active[&profile.id].is_read_only());
        assert_eq!(
            s.workspace.session_accounts[&profile.id],
            ("root".into(), false)
        );
        assert!(s.workspace.credential_tasks.is_empty());
        assert!(s.workspace.pending_credentials.is_empty());
        assert_eq!(
            server.logins.lock().unwrap().last(),
            Some(&("root".into(), "synthetic-elevated".into()))
        );
        let stored = std::fs::read_to_string(f.directory.join("profiles.json")).unwrap();
        assert!(!stored.contains("synthetic-elevated"));
        assert!(!stored.contains("\"username\": \"root\""));
        disconnect_all(s);
    }
    #[test]
    fn changed_profile_refuses_reconnect_and_clears_entered_password() {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let mut f = super::super::tests::Fixture::new();
        let profile = connected(&mut f, &server, pool);
        let s = &mut f.state;
        open(s, profile.id);
        s.workspace.reconnect.as_mut().unwrap().username = TextEdit::new("root");
        s.workspace.reconnect.as_mut().unwrap().password = TextEdit::new("discard-me");
        let mut changed = profile.clone();
        changed.name = "Changed settings".into();
        s.workspace.store.as_mut().unwrap().save(changed).unwrap();
        apply(s);
        assert!(s.workspace.reconnect.as_ref().unwrap().error.is_some());
        assert!(
            s.workspace
                .reconnect
                .as_ref()
                .unwrap()
                .password
                .text
                .is_empty()
        );
        assert!(s.workspace.active[&profile.id].latest().is_current());
        assert_eq!(server.logins.lock().unwrap().len(), 1);
        disconnect_all(s);
    }
    #[test]
    fn compact_reconnect_review_cancel_never_changes_current_session() {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let mut f = super::super::tests::Fixture::new();
        let profile = connected(&mut f, &server, pool);
        open(&mut f.state, profile.id);
        let mut state = crate::init();
        std::mem::swap(&mut state, &mut f.state);
        let mut rt = rune::Runtime::new(
            state,
            crate::view,
            (513.0, 652.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("reconnect-confirm").unwrap();
        assert!(x > 0.0 && x < 513.0 && y > 0.0 && y < 652.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert_eq!(server.logins.lock().unwrap().len(), 1);
        assert!(rt.state.workspace.active[&profile.id].latest().is_current());
        disconnect_all(&mut rt.state);
    }
}
