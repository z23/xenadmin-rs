use super::tests::Fixture;
use super::*;

fn save_profile(s: &mut State, name: &str, saved: bool, restore: bool) -> ConnectionProfile {
    let mut profile = ConnectionProfile::new(
        name.into(),
        Endpoint::parse("https://fixture.example").unwrap(),
        "operator".into(),
    );
    profile.credential_mode = if saved {
        CredentialMode::SecretService
    } else {
        CredentialMode::SessionOnly
    };
    profile.auto_connect = restore;
    s.workspace
        .store
        .as_mut()
        .unwrap()
        .save(profile.clone())
        .unwrap();
    profile
}

fn finish(
    s: &mut State,
    id: ProfileId,
    result: Result<CredentialOutcome, xenadmin_core::CredentialStoreError>,
) {
    let (_, job) = s
        .workspace
        .credential_tasks
        .iter()
        .find(|(work, _)| matches!(work,CredentialWork::Load(load) if load.profile.id==id))
        .unwrap();
    *job.result.borrow_mut() = Some(result);
    refresh_credentials(s);
}

fn loaded() -> Result<CredentialOutcome, xenadmin_core::CredentialStoreError> {
    Ok(CredentialOutcome::Loaded(Some(Credentials::new(
        "operator".into(),
        "synthetic-secret".into(),
    ))))
}

#[test]
fn startup_attempts_only_opted_in_profiles_once_and_explains_locked_keyring() {
    let mut fixture = Fixture::new();
    let s = &mut fixture.state;
    let opted = save_profile(s, "Restore", true, true);
    let manual = save_profile(s, "Manual", true, false);
    let session = save_profile(s, "Session only", false, false);
    restore_sessions(s);
    assert!(s.workspace.loads.contains_key(&opted.id));
    assert!(!s.workspace.loads.contains_key(&manual.id));
    assert!(!s.workspace.loads.contains_key(&session.id));
    finish(
        s,
        opted.id,
        Err(xenadmin_core::CredentialStoreError::Locked),
    );
    assert!(s.workspace.profile_status[&opted.id].contains("locked"));
    assert!(s.workspace.active.is_empty());
    restore_sessions(s);
    assert!(s.workspace.loads.is_empty());
    assert!(s.workspace.credential_tasks.is_empty());
    assert!(
        !std::fs::read_to_string(fixture.directory.join("profiles.json"))
            .unwrap()
            .contains("synthetic-secret")
    );
}

#[test]
fn bulk_loading_is_bounded_and_disconnect_all_invalidates_pending_credentials() {
    let mut fixture = Fixture::new();
    let s = &mut fixture.state;
    let mut profiles = Vec::new();
    for n in 0..6 {
        profiles.push(save_profile(s, &format!("Saved {n}"), true, false));
    }
    let session = save_profile(s, "Session only", false, false);
    connect_all(s);
    assert_eq!(s.workspace.loads.len(), 6);
    assert_eq!(s.workspace.credential_tasks.len(), 4);
    assert_eq!(s.workspace.load_queue.len(), 2);
    assert!(s.workspace.profile_status[&session.id].contains("Password required"));
    connect_all(s);
    assert_eq!(s.workspace.credential_tasks.len(), 4);
    let running = match &s.workspace.credential_tasks[0].0 {
        CredentialWork::Load(load) => load.profile.id,
        _ => unreachable!(),
    };
    finish(s, running, Ok(CredentialOutcome::Loaded(None)));
    assert_eq!(s.workspace.credential_tasks.len(), 4);
    assert_eq!(s.workspace.loads.len(), 5);
    assert!(s.workspace.profile_status[&running].contains("No saved password"));
    disconnect_all(s);
    assert!(s.workspace.loads.is_empty());
    assert!(s.workspace.load_queue.is_empty());
    for (_, job) in &s.workspace.credential_tasks {
        *job.result.borrow_mut() = Some(loaded());
    }
    refresh_credentials(s);
    assert!(s.workspace.active.is_empty());
    assert!(s.workspace.cached_credentials.is_none());
}

#[test]
fn edited_removed_and_cancelled_profiles_discard_late_results() {
    let mut fixture = Fixture::new();
    let s = &mut fixture.state;
    for change in [
        "endpoint",
        "username",
        "trust",
        "read_only",
        "remove",
        "cancel",
    ] {
        let profile = save_profile(s, change, true, false);
        queue_load(s, &profile, LoadPurpose::Connect);
        pump_loads(s);
        let mut modified = profile.clone();
        match change {
            "endpoint" => modified.set_endpoint(Endpoint::parse("different.example").unwrap()),
            "username" => modified.username = "another-operator".into(),
            "trust" => modified.accept_invalid_certificates = true,
            "read_only" => modified.read_only = true,
            "remove" => {
                s.workspace
                    .store
                    .as_mut()
                    .unwrap()
                    .remove(profile.id)
                    .unwrap();
            }
            "cancel" => cancel_load(s, profile.id),
            _ => unreachable!(),
        }
        if !matches!(change, "remove" | "cancel") {
            s.workspace.store.as_mut().unwrap().save(modified).unwrap();
        }
        finish(s, profile.id, loaded());
        assert!(
            !s.workspace.active.contains_key(&profile.id),
            "late result used after {change}"
        );
        assert!(s.workspace.cached_credentials.is_none());
    }
}

#[test]
fn retry_does_not_accept_an_old_attempt_and_editor_trust_edits_discard_loaded_passwords() {
    let mut fixture = Fixture::new();
    let s = &mut fixture.state;
    let profile = save_profile(s, "Retry", true, false);
    queue_load(s, &profile, LoadPurpose::Connect);
    let first = s.workspace.loads[&profile.id].clone();
    cancel_load(s, profile.id);
    queue_load(s, &profile, LoadPurpose::Connect);
    let second = s.workspace.loads[&profile.id].id;
    apply_credential_result(s, CredentialWork::Load(Box::new(first)), loaded());
    assert_eq!(s.workspace.loads[&profile.id].id, second);
    assert!(s.workspace.active.is_empty());
    cancel_load(s, profile.id);
    populate(s, &profile);
    s.show_connect = true;
    load_password(s, &profile);
    s.accept_invalid_certificates = true;
    finish(s, profile.id, loaded());
    assert!(s.workspace.cached_credentials.is_none());
    assert!(
        s.workspace
            .credential_status
            .as_ref()
            .unwrap()
            .contains("settings changed")
    );
}

#[test]
fn concurrent_successes_connect_their_own_profiles_without_replacing_the_open_editor() {
    let mut fixture = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let backend = Backend::with_state_directory(fixture.directory.join("backend")).unwrap();
    let s = &mut fixture.state;
    s.backend = Ok(backend);
    let mut a = save_profile(s, "First saved connection", true, false);
    let mut b = save_profile(s, "Second saved connection", true, false);
    for profile in [&mut a, &mut b] {
        profile.set_endpoint(endpoint.clone());
        s.workspace
            .store
            .as_mut()
            .unwrap()
            .save(profile.clone())
            .unwrap();
    }
    let editor = save_profile(s, "Unrelated editor", false, false);
    populate(s, &editor);
    s.show_connect = true;
    connect_all(s);
    finish(s, b.id, loaded());
    finish(s, a.id, loaded());
    assert!(s.workspace.active.contains_key(&a.id));
    assert!(s.workspace.active.contains_key(&b.id));
    assert_eq!(s.workspace.editing, Some(editor.id));
    assert_eq!(s.endpoint.text, editor.endpoint.as_str());
    assert!(s.password.text.is_empty());
    connect_all(s);
    assert!(!s.workspace.loads.contains_key(&a.id));
    assert!(!s.workspace.loads.contains_key(&b.id));
    disconnect_all(s);
}

#[test]
fn rune_bulk_controls_show_profile_errors_and_cancel_pending_attempts() {
    use rune::{Env, Runtime};
    let mut fixture = Fixture::new();
    let saved = save_profile(&mut fixture.state, "Saved fixture", true, false);
    let session = save_profile(&mut fixture.state, "Session fixture", false, false);
    fixture.state.show_connect = false;
    fixture.state.workspace.show_manager = true;
    let mut state = init();
    std::mem::swap(&mut state, &mut fixture.state);
    let mut runtime = Runtime::new(
        state,
        view,
        (1440.0, 900.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    for _ in 0..3 {
        runtime.frame(1.0 / 60.0);
    }
    let (x, y) = runtime.find("connect-all").unwrap();
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.0);
    assert!(runtime.state.workspace.loads.contains_key(&saved.id));
    assert!(runtime.state.workspace.profile_status[&session.id].contains("Password required"));
    assert!(
        runtime
            .accessibility_json()
            .to_string()
            .contains("Loading saved password")
    );
    let (x, y) = runtime.find("disconnect-all").unwrap();
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.0);
    assert!(runtime.state.workspace.loads.is_empty());
    assert!(
        runtime
            .accessibility_json()
            .to_string()
            .contains("cancelled")
    );
}

#[test]
fn forgetting_saved_credentials_also_disables_startup_restore() {
    let mut fixture = Fixture::new();
    let s = &mut fixture.state;
    let profile = save_profile(s, "Forget", true, true);
    populate(s, &profile);
    apply_credential_result(
        s,
        CredentialWork::Forget(profile.id),
        Ok(CredentialOutcome::Removed),
    );
    let stored = s.workspace.profile(profile.id).unwrap();
    assert_eq!(stored.credential_mode, CredentialMode::SessionOnly);
    assert!(!stored.auto_connect);
    assert!(!s.workspace.auto_connect);
}

#[test]
fn previews_do_not_restore_profiles_and_keyring_mutations_cannot_race_profile_saves() {
    for flag in ["--demo", "--scene=connect", "--snap", "--tree"] {
        assert!(preview_arguments([flag.to_owned()].into_iter()));
    }
    assert!(!preview_arguments(
        ["--theme".into(), "dark".into()].into_iter()
    ));
    let mut fixture = Fixture::new();
    let s = &mut fixture.state;
    let profile = save_profile(s, "Saved", true, true);
    s.demo = true;
    restore_sessions(s);
    assert!(s.workspace.loads.is_empty());
    s.demo = false;
    populate(s, &profile);
    s.workspace.credential_tasks.push((
        CredentialWork::Save(Box::new(profile.clone())),
        CredentialJob::save(
            CredentialKey::for_profile(&profile),
            Credentials::new("operator".into(), "synthetic-secret".into()),
            None,
        ),
    ));
    s.endpoint.text = "https://changed.example".into();
    save_only(s);
    assert!(
        s.error
            .as_ref()
            .unwrap()
            .contains("password-store operation")
    );
    assert_eq!(
        s.workspace.profile(profile.id).unwrap().endpoint,
        profile.endpoint
    );
    assert_eq!(s.workspace.credential_tasks.len(), 1);
}

fn connect_load_queued(s: &State) -> bool {
    let connects = |load: &CredentialLoad| load.purpose == LoadPurpose::Connect;
    s.workspace.loads.values().any(connects)
        || s.workspace.credential_tasks.iter().any(|(work, _)| {
            matches!(work, CredentialWork::Load(load) if connects(load))
        })
}

fn restore_queues_connect(arguments: &[&str]) -> bool {
    let report = std::env::temp_dir().join(format!("xenadmin-restore-{}", uuid::Uuid::new_v4()));
    let thread = std::thread::current();
    let name = thread
        .name()
        .expect("the test harness names this thread");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env("XENADMIN_RESTORE_REPORT", &report)
        .args([name, "--exact", "--test-threads=1", "--"])
        .args(arguments)
        .output()
        .unwrap();
    let reported = std::fs::read_to_string(&report).ok();
    let _ = std::fs::remove_file(&report);
    assert!(
        output.status.success(),
        "restore probe {arguments:?} status {:?} failed\n{}{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    match reported.as_deref() {
        Some("queued") => true,
        Some("skipped") => false,
        other => panic!("restore probe {arguments:?} wrote {other:?}"),
    }
}

#[test]
fn headless_flags_do_not_restore_a_saved_session() {
    // restore_sessions reads std::env::args, so only a fresh process can show a chosen argv.
    if let Ok(path) = std::env::var("XENADMIN_RESTORE_REPORT") {
        let mut fixture = Fixture::new();
        let s = &mut fixture.state;
        let _profile = save_profile(s, "Saved", true, true);
        restore_sessions(s);
        std::fs::write(
            path,
            if connect_load_queued(s) {
                "queued"
            } else {
                "skipped"
            },
        )
        .unwrap();
        return;
    }

    let headless = [
        "--html",
        "--html=out.html",
        "--video",
        "--video=clip.mp4",
        "--strip",
        "--strip=out.png",
        "--list-scenes",
        "--first-pixels",
        "--list-windows",
        "--desktop",
        "--desktop=backdrop.png",
        "--script",
        "--script=replay.json",
        "--trace",
        "--trace=motion.jsonl",
        "--frames",
        "--frames=1",
        "--duration",
        "--duration=auto",
        "--out",
        "--out=frames",
    ];
    let mut missed = Vec::new();
    for flag in headless {
        let args = vec![flag.to_string()];
        if !preview_arguments(args.iter().cloned()) {
            missed.push(format!("preview_arguments {flag}"));
        }
        if !crate::instance_launch::preview(&args) {
            missed.push(format!("instance_launch::preview {flag}"));
        }
    }
    for flag in [
        "--script",
        "--script=replay.json",
        "--trace",
        "--trace=motion.jsonl",
        "--frames",
        "--frames=1",
        "--duration",
        "--duration=auto",
        "--out",
        "--out=frames",
    ] {
        let args = vec![flag.to_string(), "--native".to_string()];
        if preview_arguments(args.iter().cloned()) {
            missed.push(format!("preview_arguments {flag} with --native"));
        }
        if crate::instance_launch::preview(&args) {
            missed.push(format!("instance_launch::preview {flag} with --native"));
        }
    }
    for args in [
        vec!["--html-template".to_string()],
        vec!["--html-template=page.html".to_string()],
        vec!["--html-widths".to_string()],
        vec!["--html-widths=360,640".to_string()],
    ] {
        let label = args.join(" ");
        if preview_arguments(args.iter().cloned()) {
            missed.push(format!("preview_arguments {label}"));
        }
        if crate::instance_launch::preview(&args) {
            missed.push(format!("instance_launch::preview {label}"));
        }
    }
    let html_and_native = vec!["--native".to_string(), "--html=out.html".to_string()];
    if !preview_arguments(html_and_native.iter().cloned()) {
        missed.push("preview_arguments --native --html=out.html".into());
    }
    if !crate::instance_launch::preview(&html_and_native) {
        missed.push("instance_launch::preview --native --html=out.html".into());
    }
    if !restore_queues_connect(&["--theme", "dark"]) {
        missed.push("non-preview argv did not queue a Connect load".into());
    }
    if !restore_queues_connect(&["--native", "--frames", "1"]) {
        missed.push("--native with --frames did not queue a Connect load".into());
    }
    if restore_queues_connect(&["--html", "out.html"]) {
        missed.push("headless argv --html queued a Connect load".into());
    }
    if restore_queues_connect(&["--html=out.html"]) {
        missed.push("headless argv --html=out.html queued a Connect load".into());
    }
    if restore_queues_connect(&["--frames", "1"]) {
        missed.push("headless argv --frames queued a Connect load".into());
    }
    if restore_queues_connect(&["--frames=1"]) {
        missed.push("headless argv --frames=1 queued a Connect load".into());
    }
    assert!(missed.is_empty(), "{missed:#?}");
}

#[test]
fn rune_startup_restore_checkbox_requires_explicit_keyring_choice() {
    use rune::{Env, Runtime};
    let mut fixture = Fixture::new();
    fixture.state.show_connect = true;
    let mut state = init();
    std::mem::swap(&mut state, &mut fixture.state);
    let mut runtime = Runtime::new(
        state,
        view,
        (1440.0, 900.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    for _ in 0..3 {
        runtime.frame(1.0 / 60.0);
    }
    let (x, y) = runtime.find("profile-auto-connect").unwrap();
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.0);
    assert!(!runtime.state.workspace.auto_connect);
    let (x, y) = runtime.find("remember-password").unwrap();
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.0);
    assert!(runtime.state.workspace.remember_password);
    let (x, y) = runtime.find("profile-auto-connect").unwrap();
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.0);
    assert!(runtime.state.workspace.auto_connect);
    let (x, y) = runtime.find("remember-password").unwrap();
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.0);
    assert!(!runtime.state.workspace.auto_connect);
}
