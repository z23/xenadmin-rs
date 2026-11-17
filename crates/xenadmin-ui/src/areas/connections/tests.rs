//! Tests of the area.
//!
//! The Rune tests of `profile_trust_tests.rs`, `profile_reconnect.rs`, the
//! two `rune::Runtime` tests of `profiles_view.rs` and the three of
//! `profile_restore_tests.rs` that the kernel left to this area, each under
//! its Rune name. A test that built a `rune::Runtime` and pressed keys is a
//! harness test asserting the same thing; what a Rune test asserted about
//! positions in a 1440x900, 513x652 or 480x600 window is retired with the
//! Rune layout (three tests, marked below). The other tests of the Rune
//! modules are in `session.rs` and `launch.rs`.
//!
//! After them: tests of what the port added (the Home tab, the commands,
//! the scenes).

use std::path::PathBuf;

use xenadmin_core::{
    ConnectionProfile, ConnectionState, CredentialMode, Endpoint, ProfileId, ProxyConfiguration,
    ProxyEndpoint,
};

use super::home::TABLE;
use super::{connect, reconnect, trust};
use crate::Harness;
use crate::command::{self, MenuItem};
use crate::prelude::*;
use crate::scene;
use crate::session::tests::{Fixture, finish, loaded, save_profile};
use crate::session::{self, CredentialWork, LoadPurpose};
use crate::surface::{duplicate_keys, flatten_blocks};

/// A harness over the model of a fixture. The fixture keeps the state
/// directory and removes it when the test ends.
fn harness(fixture: &mut Fixture) -> Harness {
    Harness::with(std::mem::take(&mut fixture.model))
}

/// The connect form of a fixture is open at the start (`begin_new`); this
/// closes it, so the Home tab takes input.
fn home(fixture: &mut Fixture) -> Harness {
    let mut h = harness(fixture);
    connect::close(&mut h.model);
    h.model.show_home();
    h
}

fn ids(items: &[MenuItem]) -> Vec<&'static str> {
    items.iter().map(|item| item.id.0).collect()
}

/// The reason an offered command is disabled; `None` when it is enabled.
/// Panics when the command is not offered.
fn reason(items: &[MenuItem], id: &str) -> Option<String> {
    items
        .iter()
        .find(|item| item.id.0 == id)
        .unwrap_or_else(|| panic!("`{id}` is not offered"))
        .disabled
        .clone()
}

// -- profiles_view.rs ---------------------------------------------------------

// Rune: profiles_view.rs, the same name. The file prompt is an intent; the
// harness answers it as the front end does.
#[test]
fn ca_picker_applies_to_its_editor_only_and_cancel_preserves_the_path() {
    let mut h = Harness::new();
    session::begin_new(&mut h.model);
    h.click("profile-options");
    let pick = |h: &mut Harness| {
        h.click("choose-ca-file");
        match h.model.intents.as_slice() {
            [Intent::ChooseFile(request)] => {
                assert_eq!(request.mode, FileMode::Open);
                assert_eq!(request.title, "Choose CA certificate");
                assert_eq!(
                    request.filters,
                    [FileFilter {
                        name: "PEM certificates".into(),
                        extensions: vec!["pem".into(), "crt".into(), "cer".into()],
                    }]
                );
            }
            _ => panic!("one file prompt is expected"),
        }
    };
    pick(&mut h);
    h.answer_file(Some("/tmp/synthetic-ca.pem".into()));
    assert_eq!(h.model.workspace.ca_certificate, "/tmp/synthetic-ca.pem");
    assert_eq!(
        h.node("profile-ca").value.as_deref(),
        Some("/tmp/synthetic-ca.pem")
    );
    pick(&mut h);
    h.answer_file(None);
    assert_eq!(h.model.workspace.ca_certificate, "/tmp/synthetic-ca.pem");
    // A path that is not Unicode is refused and the path stays.
    pick(&mut h);
    let not_unicode: PathBuf =
        <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(vec![b'/', 0xff]).into();
    h.answer_file(Some(not_unicode));
    assert_eq!(h.model.workspace.ca_certificate, "/tmp/synthetic-ca.pem");
    assert_eq!(
        h.node("connect-error").text.as_deref(),
        Some("Choose a certificate path with valid Unicode characters.")
    );
    // An answer that arrives after another editor opened is dropped.
    pick(&mut h);
    session::begin_new(&mut h.model);
    h.answer_file(Some("/tmp/old-editor.pem".into()));
    assert!(h.model.workspace.ca_certificate.is_empty());
    // And so is one that arrives after the form closed.
    pick(&mut h);
    h.click("cancel-connect");
    h.answer_file(Some("/tmp/closed-editor.pem".into()));
    assert!(h.model.workspace.ca_certificate.is_empty());
}

// Rune: profiles_view.rs, the same name. The Connections dialog is the Home
// tab; `edit-profile-{id}` is the command `server.connect` on the row.
#[test]
fn connections_dialog_exposes_saved_profiles_and_opens_an_editor() {
    let mut fixture = Fixture::new();
    fixture.model.workspace.endpoint = "host.example".into();
    fixture.model.workspace.name = "Fixture pool".into();
    session::save_only(&mut fixture.model);
    let id = fixture.model.workspace.editing.unwrap();
    let mut h = harness(&mut fixture);
    assert_eq!(h.surface_id(), None);
    assert_eq!(h.model.active_tab(), TabId::Home);
    assert!(h.has_text("Fixture pool"));
    h.select_row(TABLE, &id.to_string());
    h.click("server.connect");
    assert!(h.model.workspace.show_connect);
    assert_eq!(h.surface().id, connect::DIALOG);
    assert_eq!(h.model.workspace.endpoint, "https://host.example/");
    assert_eq!(
        h.node("host-endpoint").value.as_deref(),
        Some("https://host.example/")
    );
    assert!(h.model.workspace.password.is_empty());
}

// -- profile_trust_tests.rs ---------------------------------------------------

// Rune: profile_trust_tests.rs, the same name. Retired with the Rune layout:
// that the heading, Cancel and Connect are inside a 1440x900, 513x652 and
// 480x600 viewport. Kept: they are there, the proxy modes switch, and Cancel
// closes.
#[test]
fn compact_and_wide_editor_keep_header_and_connect_controls_visible() {
    let mut h = Harness::scene("connection-trust");
    let surface = h.surface();
    assert_eq!(surface.id, connect::DIALOG);
    assert_eq!(surface.title, "Connect to XCP-ng");
    let footer: Vec<_> = surface.footer.iter().map(|b| b.key.as_str()).collect();
    assert_eq!(footer, ["cancel-connect", "connect"]);
    assert_eq!(h.node("connect").disabled, None);
    h.assert_unique_keys();
    h.click("proxy-mode-1");
    assert_eq!(h.model.workspace.trust.mode, 1);
    assert!(h.find("proxy-url").is_none());
    h.click("proxy-mode-2");
    assert_eq!(h.model.workspace.trust.mode, 2);
    assert!(h.find("proxy-url").is_some());
    h.click("cancel-connect");
    assert!(!h.model.workspace.show_connect);
    assert_eq!(h.surface_id(), None);
}

// Rune: profile_trust_tests.rs, the same name. The Rune button did nothing
// when pressed too early; here it is also disabled, with the condition that
// is missing.
#[test]
fn reviewed_changed_pin_requires_exact_endpoint_and_explicit_fingerprint_check() {
    const TYPE_IT: &str = "Type the exact endpoint to update only its certificate pin.";
    let mut h = Harness::scene("certificate-review");
    let surface = h.surface();
    assert_eq!(surface.title, "Review certificate");
    let footer: Vec<_> = surface
        .footer
        .iter()
        .map(|b| (b.key.as_str(), b.label.as_str()))
        .collect();
    assert_eq!(
        footer,
        [
            ("back-from-certificate", "Back"),
            ("accept-certificate", "Replace pin")
        ]
    );
    assert!(h.has_text("Certificate changed"));
    assert!(h.find("connect").is_none(), "the review replaces the form");
    h.assert_unique_keys();
    let original = h.model.workspace.host_certificates.clone();
    assert_eq!(h.try_click("accept-certificate"), Err(TYPE_IT.into()));
    trust::accept(&mut h.model);
    assert_eq!(h.model.workspace.host_certificates, original);
    h.type_text("confirm-certificate-endpoint", "https://another.example/");
    h.toggle("certificate-verified");
    assert_eq!(h.try_click("accept-certificate"), Err(TYPE_IT.into()));
    trust::accept(&mut h.model);
    assert_eq!(h.model.workspace.host_certificates, original);
    h.type_text(
        "confirm-certificate-endpoint",
        "https://coordinator.example/",
    );
    // Typed, but not ticked: the other condition.
    h.toggle("certificate-verified");
    assert_eq!(
        h.try_click("accept-certificate"),
        Err("Confirm that you verified this SHA-256 through a trusted channel.".into())
    );
    assert_eq!(h.model.workspace.host_certificates, original);
    h.toggle("certificate-verified");
    h.click("accept-certificate");
    assert!(
        h.model
            .workspace
            .host_certificates
            .contains(&"bb".repeat(32))
    );
    assert_eq!(h.model.workspace.endpoint, "https://xcp-lab.example");
    assert!(h.model.workspace.show_connect);
    assert!(h.model.workspace.active.is_empty());
    assert!(
        h.model
            .workspace
            .trust
            .status
            .as_ref()
            .unwrap()
            .contains("unsaved")
    );
    // The dialog is the form again, with the status line.
    assert_eq!(h.surface().title, "Connect to XCP-ng");
    assert!(h.find("connect").is_some());
    assert!(
        h.node("certificate-status")
            .text
            .unwrap()
            .contains("unsaved")
    );
}

// Rune: profile_trust_tests.rs, the same name. Added: while the stale review
// still shows, it says so and cannot be accepted.
#[test]
fn delayed_inspection_and_review_cannot_follow_changed_route_pin_or_editor() {
    for changed in [
        "endpoint", "proxy", "ca", "pin", "password", "epoch", "closed",
    ] {
        let mut h = Harness::scene("certificate-review");
        assert!(trust::stale(&h.model).is_none(), "the scene is current");
        let observation = h.model.workspace.trust.observation.clone().unwrap();
        let ws = &mut h.model.workspace;
        match changed {
            "endpoint" => ws.trust.endpoint = "https://other.example".into(),
            "proxy" => ws.trust.mode = 1,
            "ca" => ws.ca_certificate = "/tmp/different-ca.pem".into(),
            "pin" => {
                ws.host_certificates = format!("https://coordinator.example/={}", "cc".repeat(32))
            }
            "password" => ws.trust.password.set("new-proxy-secret"),
            "epoch" => ws.editor_epoch = uuid::Uuid::new_v4(),
            "closed" => ws.show_connect = false,
            _ => unreachable!(),
        }
        assert_eq!(
            trust::stale(&h.model).as_deref(),
            Some(trust::CHANGED),
            "{changed}"
        );
        if changed != "closed" {
            assert_eq!(
                h.try_click("accept-certificate"),
                Err(trust::CHANGED.into()),
                "{changed}"
            );
            assert!(h.find("certificate-review-stale").is_some(), "{changed}");
        }
        h.model.workspace.trust.result = Some(Ok(observation));
        trust::poll(&mut h.model);
        assert!(h.model.workspace.trust.observation.is_none(), "{changed}");
        assert!(!trust::can_accept(&h.model));
        assert_eq!(
            h.model.workspace.trust.status.as_deref(),
            Some(trust::CHANGED),
            "{changed}"
        );
    }
}

// Rune: profile_trust_tests.rs, the same name.
#[test]
fn proxy_credentials_are_session_only_and_bound_to_the_exact_proxy_configuration() {
    let mut h = Harness::scene("connection-trust");
    h.type_text("proxy-url", "http://proxy.example:3128");
    h.type_text("proxy-username", "proxy-user");
    h.type_text("proxy-password", "private-proxy-secret");
    assert!(!h.has_text("private-proxy-secret"), "a secret shows bullets");
    let p = session::form_profile(&h.model).unwrap();
    assert!(session::trust_options(&h.model, &p).is_ok());
    assert!(
        !serde_json::to_string(&p)
            .unwrap()
            .contains("private-proxy-secret")
    );
    session::trust_remember_proxy(&mut h.model, &p);
    assert!(h.model.workspace.trust.password.is_empty());
    assert!(session::trust_options(&h.model, &p).is_ok());
    let mut changed = p.clone();
    changed.proxy = ProxyConfiguration::Http {
        endpoint: ProxyEndpoint::parse("http://different.example:3128").unwrap(),
        username: Some("proxy-user".into()),
    };
    assert!(session::trust_options(&h.model, &changed).is_err());
    h.model.workspace.editing = Some(p.id);
    h.click("forget-proxy-password");
    assert!(session::trust_options(&h.model, &p).is_err());
}

// Rune: profile_trust_tests.rs, the same name.
#[test]
fn synthetic_inspection_cannot_contact_an_endpoint_or_load_a_password() {
    let mut h = Harness::scene("certificate-review");
    h.click("inspect-certificate");
    assert!(h.model.workspace.trust.job.is_none());
    assert!(
        h.model
            .workspace
            .trust
            .status
            .as_ref()
            .unwrap()
            .contains("preview")
    );
    assert!(
        h.node("certificate-status")
            .text
            .unwrap()
            .contains("preview")
    );
    assert!(h.model.workspace.credential_tasks.is_empty());
}

/// The connect form of a saved profile (the Rune `editor_for`).
fn editor_for(profile: ConnectionProfile) -> Fixture {
    let mut fixture = Fixture::new();
    let m = &mut fixture.model;
    m.workspace
        .store
        .as_mut()
        .unwrap()
        .save(profile.clone())
        .unwrap();
    session::populate(m, &profile);
    fixture
}

fn pinned_profile() -> (ConnectionProfile, String) {
    let endpoint = Endpoint::parse("https://lab.example").unwrap();
    let pin = "ab".repeat(32);
    let mut profile = ConnectionProfile::new("Lab".into(), endpoint.clone(), "root".into());
    profile
        .host_certificates
        .insert(endpoint.to_string(), pin.parse().unwrap());
    (profile, pin)
}

// Rune: profile_trust_tests.rs, the same name.
#[test]
fn endpoint_edit_drops_the_previous_certificate_pin() {
    let (profile, pin) = pinned_profile();
    let old = profile.endpoint.to_string();
    let mut fixture = editor_for(profile);
    let m = &mut fixture.model;
    assert_eq!(m.workspace.fingerprint, pin);
    m.workspace.endpoint = "https://other.example/".into();
    let saved = session::form_profile(m).unwrap();
    assert_eq!(saved.endpoint.as_str(), "https://other.example/");
    assert!(saved.options().unwrap().certificate_fingerprint.is_none());
    assert!(!saved.accept_invalid_certificates);
    assert!(!saved.host_certificates.contains_key(&old));
}

// Rune: profile_trust_tests.rs, the same name.
#[test]
fn endpoint_edit_keeps_a_different_fingerprint_on_the_new_endpoint_only() {
    let (profile, _) = pinned_profile();
    let old = profile.endpoint.to_string();
    let mut fixture = editor_for(profile);
    let m = &mut fixture.model;
    let new_pin = "cd".repeat(32);
    m.workspace.endpoint = "https://other.example/".into();
    m.workspace.fingerprint = new_pin.clone();
    let saved = session::form_profile(m).unwrap();
    assert_eq!(saved.endpoint.as_str(), "https://other.example/");
    assert_eq!(
        saved
            .options()
            .unwrap()
            .certificate_fingerprint
            .unwrap()
            .to_string(),
        new_pin
    );
    assert!(!saved.accept_invalid_certificates);
    assert!(!saved.host_certificates.contains_key(&old));
    assert_eq!(saved.host_certificates.len(), 1);
    assert_eq!(
        saved
            .host_certificates
            .get("https://other.example/")
            .unwrap()
            .to_string(),
        new_pin
    );
}

// Rune: profile_trust_tests.rs, the same name.
#[test]
fn unchanged_endpoint_keeps_its_certificate_pin() {
    let (profile, pin) = pinned_profile();
    let old = profile.endpoint.clone();
    let fixture = editor_for(profile);
    let saved = session::form_profile(&fixture.model).unwrap();
    assert_eq!(saved.endpoint, old);
    assert_eq!(
        saved
            .options()
            .unwrap()
            .certificate_fingerprint
            .unwrap()
            .to_string(),
        pin
    );
    assert!(saved.host_certificates.contains_key(old.as_str()));
    assert!(!saved.accept_invalid_certificates);
}

// Rune: profile_trust_tests.rs, the same name.
#[test]
fn reviewing_failed_member_certificate_does_not_load_saved_host_credentials() {
    let mut fixture = Fixture::new();
    let mut profile = ConnectionProfile::new(
        "Saved".into(),
        Endpoint::parse("seed.example").unwrap(),
        "root".into(),
    );
    profile.credential_mode = CredentialMode::SecretService;
    fixture
        .model
        .workspace
        .store
        .as_mut()
        .unwrap()
        .save(profile.clone())
        .unwrap();
    let mut h = home(&mut fixture);
    trust::inspect_failed(
        &mut h.model,
        profile.id,
        Endpoint::parse("coordinator.example").unwrap(),
    );
    let m = &h.model;
    assert!(m.workspace.show_connect && m.workspace.show_certificate_details);
    assert_eq!(m.workspace.trust.endpoint, "https://coordinator.example/");
    assert!(m.workspace.credential_tasks.is_empty());
    assert!(m.workspace.password.is_empty());
    // The form of that saved connection, on the endpoint that failed; the
    // inspection is the user's to start.
    assert_eq!(m.workspace.editing, Some(profile.id));
    assert!(m.workspace.trust.context.is_none() && m.workspace.trust.status.is_none());
    assert_eq!(h.surface().title, "Connect to XCP-ng");
    assert_eq!(
        h.node("inspect-endpoint").value.as_deref(),
        Some("https://coordinator.example/")
    );
    assert_eq!(h.node("inspect-certificate").disabled, None);
}

// -- profile_reconnect.rs -----------------------------------------------------

mod reconnecting {
    use std::{
        io::{BufRead, Read, Write},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

    use xenadmin_core::{Backend, Connection, Credentials, PoolId};

    use super::*;

    /// A local JSON-RPC server that answers a login, the first event batch
    /// with one pool, and a logout (the Rune fixture).
    struct Server {
        address: std::net::SocketAddr,
        endpoint: Endpoint,
        stop: Arc<AtomicBool>,
        logins: Arc<Mutex<Vec<(String, String)>>>,
    }
    impl Server {
        fn new(pool: PoolId) -> Self {
            Self::on(std::net::TcpListener::bind("127.0.0.1:0").unwrap(), pool)
        }

        /// A server on the address another one had: the host is back. The
        /// address is free once the thread of the dropped server has seen
        /// its stop flag.
        fn at(address: std::net::SocketAddr, pool: PoolId) -> Self {
            let start = Instant::now();
            loop {
                match std::net::TcpListener::bind(address) {
                    Ok(listener) => return Self::on(listener, pool),
                    Err(error) => {
                        assert!(start.elapsed() < Duration::from_secs(3), "{error}");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }

        fn on(listener: std::net::TcpListener, pool: PoolId) -> Self {
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let endpoint = Endpoint::parse(&format!("http://{address}")).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let logins = Arc::new(Mutex::new(Vec::new()));
            let signal = stop.clone();
            let seen = logins.clone();
            std::thread::spawn(move || {
                while !signal.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let logins = seen.clone();
                            std::thread::spawn(move || serve(stream, pool, &logins));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                address,
                endpoint,
                stop,
                logins,
            }
        }

        fn logins(&self) -> Vec<(String, String)> {
            self.logins.lock().unwrap().clone()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release)
        }
    }

    /// Answers one request of one connection.
    fn serve(stream: std::net::TcpStream, pool: PoolId, logins: &Mutex<Vec<(String, String)>>) {
        // The listener is non-blocking; the accepted stream must not be.
        stream.set_nonblocking(false).unwrap();
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
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
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
        let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
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
                    serde_json::json!({"token": "fixture", "events": []})
                } else {
                    serde_json::json!({"token": "fixture", "events": [{
                        "class": "pool",
                        "operation": "add",
                        "ref": "OpaqueRef:pool",
                        "snapshot": {"uuid": pool.to_string(), "name_label": "Fixture pool"}
                    }]})
                }
            }
            "session.logout" => serde_json::Value::Null,
            _ => panic!("Unexpected fixture request: {method}"),
        };
        let body = serde_json::to_vec(
            &serde_json::json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
        )
        .unwrap();
        let mut stream = input.into_inner();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
    }

    /// Polls the model, as its window does on every wake, until the
    /// condition holds.
    fn poll_until(h: &mut Harness, what: &str, condition: impl Fn(&Harness) -> bool) {
        let start = Instant::now();
        loop {
            h.tick(0.02);
            if condition(h) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "{what}");
            std::thread::sleep(Duration::from_millis(10));
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

    /// A fixture with one saved connection that is connected to the server
    /// and is the scope session (the Rune `connected`).
    fn connected(f: &mut Fixture, server: &Server, pool: PoolId) -> ConnectionProfile {
        let m = &mut f.model;
        connect::close(m);
        m.workspace.restore_started = true;
        m.backend = Ok(Backend::with_state_directory(f.directory.clone()).unwrap());
        let mut profile =
            ConnectionProfile::new("Fixture".into(), server.endpoint.clone(), "operator".into());
        profile.pool = Some(pool);
        profile.read_only = true;
        profile.credential_mode = CredentialMode::SecretService;
        profile.auto_connect = true;
        m.workspace
            .store
            .as_mut()
            .unwrap()
            .save(profile.clone())
            .unwrap();
        session::start_connection(
            m,
            &profile,
            Credentials::new("operator".into(), "synthetic-original".into()),
        )
        .unwrap();
        ready(m.workspace.active.get(&profile.id).unwrap());
        session::refresh(m);
        session::select(m, profile.id);
        m.workspace.profile(profile.id).unwrap().clone()
    }

    /// Opens Reconnect As on the pool of a saved connection, as the Pool
    /// menu does.
    fn open(h: &mut Harness, id: ProfileId) {
        let target = Target::one(session::session_object(&h.model, id));
        assert_eq!(target.kind(), ObjectKind::Pool);
        h.invoke_on("pool.reconnect-as", &target);
        assert_eq!(h.surface().id, reconnect::DIALOG);
    }

    // Rune: profile_reconnect.rs, the same name. Driven through the dialog.
    #[test]
    fn reconnect_is_ephemeral_cancels_same_pool_aliases_and_retains_other_pool() {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let other_pool = PoolId::from(uuid::Uuid::new_v4());
        let other = Server::new(other_pool);
        let mut f = Fixture::new();
        let profile = connected(&mut f, &server, pool);
        let m = &mut f.model;
        let mut alias = profile.clone();
        alias.id = ProfileId::new();
        m.workspace
            .store
            .as_mut()
            .unwrap()
            .save(alias.clone())
            .unwrap();
        let old = m.workspace.active[&profile.id].clone();
        m.workspace.active.insert(alias.id, old.clone());
        let mut other_profile =
            ConnectionProfile::new("Other pool".into(), other.endpoint.clone(), "other".into());
        other_profile.pool = Some(other_pool);
        m.workspace
            .store
            .as_mut()
            .unwrap()
            .save(other_profile.clone())
            .unwrap();
        session::start_connection(
            m,
            &other_profile,
            Credentials::new("other".into(), "synthetic-other".into()),
        )
        .unwrap();
        ready(&m.workspace.active[&other_profile.id]);
        let other_generation = m.workspace.active[&other_profile.id].latest().generation();
        let mut h = harness(&mut f);
        open(&mut h, profile.id);
        assert!(h.has_text("Saved account: operator."));
        assert!(!h.surface().experimental);
        h.assert_unique_keys();
        h.type_text("reconnect-username", "root");
        h.type_text("reconnect-password", "synthetic-elevated");
        assert!(!h.has_text("synthetic-elevated"), "a secret shows bullets");
        // The session being replaced is read-only, and so is the draft.
        assert_eq!(h.node("reconnect-read-only").checked, Some(true));
        h.toggle("reconnect-read-only");
        h.click("reconnect-confirm");
        let m = &mut h.model;
        assert!(m.areas.connections.reconnect.is_none());
        ready(&m.workspace.active[&profile.id]);
        session::refresh(m);
        assert_eq!(m.workspace.profile(profile.id), Some(&profile));
        assert!(!m.workspace.active.contains_key(&alias.id));
        assert_eq!(
            m.workspace.active[&other_profile.id].latest().generation(),
            other_generation
        );
        assert!(!m.workspace.active[&profile.id].is_read_only());
        assert_eq!(
            m.workspace.session_accounts[&profile.id],
            ("root".into(), false)
        );
        assert!(m.workspace.credential_tasks.is_empty());
        assert!(m.workspace.pending_credentials.is_empty());
        assert_eq!(
            server.logins().last(),
            Some(&("root".into(), "synthetic-elevated".into()))
        );
        let stored = std::fs::read_to_string(f.directory.join("profiles.json")).unwrap();
        assert!(!stored.contains("synthetic-elevated"));
        assert!(!stored.contains("\"username\": \"root\""));
        // The Home tab says under which account the session runs, and the
        // row of a connected pool offers Disconnect and no Connect. The poll
        // gives the scope session its new inventory.
        h.tick(0.1);
        h.model.show_home();
        assert!(h.has_text("operator · as root"));
        h.select_row(TABLE, &profile.id.to_string());
        assert!(h.has_text("Current session account: root · saved account unchanged"));
        assert!(h.has_text("Role changes require a new login."));
        let row = h.local_row();
        assert!(!ids(&row).contains(&"server.connect"));
        assert_eq!(
            reason(&row, "server.remove-connection").as_deref(),
            Some("Disconnect this connection before removing its saved settings.")
        );
        assert_eq!(reason(&row, "server.disconnect"), None);
        assert_eq!(reason(&row, "server.reconnect-as"), None);
        session::disconnect_all(&mut h.model);
    }

    // Rune: profile_reconnect.rs, the same name. Added: the stale review
    // also disables the confirm button.
    #[test]
    fn changed_profile_refuses_reconnect_and_clears_entered_password() {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let mut f = Fixture::new();
        let profile = connected(&mut f, &server, pool);
        let mut h = harness(&mut f);
        open(&mut h, profile.id);
        h.type_text("reconnect-username", "root");
        h.type_text("reconnect-password", "discard-me");
        assert_eq!(h.node("reconnect-confirm").disabled, None);
        let mut changed = profile.clone();
        changed.name = "Changed settings".into();
        h.model
            .workspace
            .store
            .as_mut()
            .unwrap()
            .save(changed)
            .unwrap();
        assert_eq!(
            h.try_click("reconnect-confirm"),
            Err(reconnect::STALE.into())
        );
        assert_eq!(
            h.node("reconnect-review-stale").text.as_deref(),
            Some(reconnect::STALE)
        );
        reconnect::apply(&mut h.model);
        let editor = h.model.areas.connections.reconnect.as_ref().unwrap();
        assert_eq!(editor.error.as_deref(), Some(reconnect::STALE));
        assert!(editor.password.is_empty());
        assert_eq!(
            h.node("reconnect-error").text.as_deref(),
            Some(reconnect::STALE)
        );
        assert!(h.model.workspace.active[&profile.id].latest().is_current());
        assert_eq!(server.logins().len(), 1);
        session::disconnect_all(&mut h.model);
    }

    // Rune: profile_reconnect.rs, the same name. Retired with the Rune
    // layout: that the Reconnect button is inside a 513x652 viewport. Kept:
    // pressing it with empty fields changes nothing, and neither does Cancel.
    #[test]
    fn compact_reconnect_review_cancel_never_changes_current_session() {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let mut f = Fixture::new();
        let profile = connected(&mut f, &server, pool);
        let mut h = harness(&mut f);
        open(&mut h, profile.id);
        let footer: Vec<_> = h.surface().footer.iter().map(|b| b.key.clone()).collect();
        assert_eq!(footer, ["reconnect-cancel", "reconnect-confirm"]);
        assert_eq!(
            h.try_click("reconnect-confirm"),
            Err("Enter the account for this session.".into())
        );
        h.type_text("reconnect-username", "root");
        assert_eq!(
            h.try_click("reconnect-confirm"),
            Err("Enter a password for this session".into())
        );
        // The handler refuses by itself too.
        reconnect::apply(&mut h.model);
        assert_eq!(
            h.node("reconnect-error").text.as_deref(),
            Some("Enter a password for this session")
        );
        assert_eq!(server.logins().len(), 1);
        assert!(h.model.workspace.active[&profile.id].latest().is_current());
        h.click("reconnect-cancel");
        assert_eq!(h.surface_id(), None);
        assert_eq!(server.logins().len(), 1);
        assert!(h.model.workspace.active[&profile.id].latest().is_current());
        assert!(h.model.workspace.session_accounts.is_empty());
        session::disconnect_all(&mut h.model);
    }

    // Plan package P3.8: a lost session and its renewal as the interface
    // shows them. (Rune had no test of the views for this; the core has its
    // own of the connection.)
    #[test]
    fn a_lost_session_shows_as_reconnecting_and_its_renewal_leaves_old_reviews_stale() {
        const LOST: &str = "Cannot reach the XCP-ng management endpoint";
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let address = server.address;
        let mut f = Fixture::new();
        let profile = connected(&mut f, &server, pool);
        let id = profile.id;
        let mut h = harness(&mut f);
        h.tick(0.1);
        let generation = h.model.workspace.active[&id].latest().generation();
        // A Reconnect As review of the session as it is now.
        open(&mut h, id);
        h.type_text("reconnect-username", "root");
        h.type_text("reconnect-password", "typed-before-the-loss");
        assert_eq!(h.node("reconnect-confirm").disabled, None);

        // The host goes away.
        drop(server);
        // Both what the model adopted for the scope session and what the
        // connection list saw: the two are read at different moments of one
        // poll.
        let shows = |h: &Harness, state: ConnectionState| {
            let adopted = h.model.snapshot.as_ref().map(|snapshot| snapshot.state());
            let listed = h.model.workspace.snapshot(id).map(|snapshot| snapshot.state());
            adopted == Some(state) && listed == Some(state)
        };
        poll_until(&mut h, "the session notices the loss", |h| {
            shows(h, ConnectionState::Reconnecting)
        });
        assert_eq!(
            h.try_click("reconnect-confirm"),
            Err(reconnect::STALE.into())
        );
        h.click("reconnect-cancel");
        assert_eq!(
            h.model.connection_status(),
            "Connection lost • inventory is stale • reconnecting…"
        );
        assert!(!h.model.can_mutate());
        assert!(h.model.inventory.is_some(), "the stale inventory stays");
        h.model.show_home();
        h.select_row(TABLE, &id.to_string());
        assert_eq!(
            h.model.target().kind(),
            ObjectKind::Pool,
            "the session is still its pool"
        );
        assert!(h.has_text("Reconnecting…"));
        assert!(h.has_text(LOST));
        assert!(h.has_text("Last failure stage: "));
        assert_eq!(
            h.node(&format!("profile-permissions-{id}")).text.as_deref(),
            Some("Session permissions are unavailable until the connection is current")
        );
        let row = h.local_row();
        assert_eq!(
            reason(&row, "server.disconnect"),
            None,
            "a session that is reconnecting can be disconnected"
        );
        assert!(
            reason(&row, "server.reconnect-as").is_some(),
            "no review of a session that is not current"
        );
        assert!(!ids(&row).contains(&"server.connect"));
        assert!(!ids(&row).contains(&"server.review-certificate"));

        // The host is back at its address: the session is renewed with the
        // credentials it had, without a prompt.
        let server = Server::at(address, pool);
        poll_until(&mut h, "the session is renewed", |h| {
            shows(h, ConnectionState::Ready)
        });
        assert_ne!(
            h.model.snapshot.as_ref().unwrap().generation(),
            generation
        );
        assert_eq!(
            server.logins(),
            [("operator".into(), "synthetic-original".into())]
        );
        assert!(h.model.connection_status().starts_with("Connected directly"));
        assert!(h.has_text("Connected") && !h.has_text("Reconnecting…"));
        assert!(!h.has_text(LOST) && !h.has_text("Last failure stage: "));
        assert_eq!(reason(&h.local_row(), "server.reconnect-as"), None);
        assert_eq!(h.surface_id(), None);
        session::disconnect_all(&mut h.model);
    }
}

// -- profile_restore_tests.rs -------------------------------------------------

// Rune: profile_restore_tests.rs, the same name. `connect-all` and
// `disconnect-all` are the commands of the Home tab's action row.
#[test]
fn rune_bulk_controls_show_profile_errors_and_cancel_pending_attempts() {
    let mut fixture = Fixture::new();
    let saved = save_profile(&mut fixture.model, "Saved fixture", true, false);
    let session = save_profile(&mut fixture.model, "Session fixture", false, false);
    let mut h = home(&mut fixture);
    assert_eq!(
        h.try_click("server.disconnect-all"),
        Err("No connection is open and no connection attempt is waiting.".into())
    );
    h.click("server.connect-all");
    assert!(h.model.workspace.loads.contains_key(&saved.id));
    assert!(h.model.workspace.profile_status[&session.id].contains("Password required"));
    assert!(h.has_text("Loading saved password"));
    assert!(h.has_text("Password required"));
    h.click("server.disconnect-all");
    assert!(h.model.workspace.loads.is_empty());
    assert!(h.has_text("cancelled"));
}

fn connect_load_queued(m: &Model) -> bool {
    let connects = |load: &session::CredentialLoad| load.purpose == LoadPurpose::Connect;
    m.workspace.loads.values().any(connects)
        || m.workspace
            .credential_tasks
            .iter()
            .any(|(work, _)| matches!(work, CredentialWork::Load(load) if connects(load)))
}

/// The path of the test below, as the test binary lists it.
const RESTORE_PROBE: &str =
    "areas::connections::tests::headless_flags_do_not_restore_a_saved_session";

/// Runs this test binary again with `arguments` on its command line, where
/// the test below restores the sessions of a fixture, and says whether a
/// connection attempt was queued.
fn restore_queues_connect(arguments: &[&str]) -> bool {
    let report = std::env::temp_dir().join(format!("xenadmin-restore-{}", uuid::Uuid::new_v4()));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env("XENADMIN_RESTORE_REPORT", &report)
        .args([RESTORE_PROBE, "--exact", "--test-threads=1", "--"])
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

// Rune: profile_restore_tests.rs, the same name. The flag matrix of
// `preview_arguments` and `launch::preview` is in `launch.rs`
// (`headless_flags_are_previews_unless_native_is_asked_for`); this is the
// part that needs a second process, because `restore_sessions` reads the
// real command line. Added: the flags of the new binary and its scene tool.
#[test]
fn headless_flags_do_not_restore_a_saved_session() {
    assert_eq!(
        format!(
            "{}::headless_flags_do_not_restore_a_saved_session",
            module_path!()
        ),
        format!("{}::{RESTORE_PROBE}", env!("CARGO_CRATE_NAME")),
        "the probe names this test"
    );
    if let Ok(path) = std::env::var("XENADMIN_RESTORE_REPORT") {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        let _profile = save_profile(m, "Saved", true, true);
        session::restore_sessions(m);
        std::fs::write(
            path,
            if connect_load_queued(m) {
                "queued"
            } else {
                "skipped"
            },
        )
        .unwrap();
        return;
    }
    let mut missed = Vec::new();
    if !restore_queues_connect(&["--theme", "dark"]) {
        missed.push("non-preview argv did not queue a Connect load");
    }
    if !restore_queues_connect(&["--native", "--frames", "1"]) {
        missed.push("--native with --frames did not queue a Connect load");
    }
    if restore_queues_connect(&["--html", "out.html"]) {
        missed.push("headless argv --html queued a Connect load");
    }
    if restore_queues_connect(&["--html=out.html"]) {
        missed.push("headless argv --html=out.html queued a Connect load");
    }
    if restore_queues_connect(&["--frames", "1"]) {
        missed.push("headless argv --frames queued a Connect load");
    }
    if restore_queues_connect(&["--frames=1"]) {
        missed.push("headless argv --frames=1 queued a Connect load");
    }
    for flag in ["--demo", "--scene=connect", "--snap=x.png", "--tree=x.json"] {
        if restore_queues_connect(&[flag]) {
            missed.push(flag);
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
}

// Rune: profile_restore_tests.rs, the same name. The Rune checkbox ignored
// the press; here it is also disabled, with the reason.
#[test]
fn rune_startup_restore_checkbox_requires_explicit_keyring_choice() {
    let mut fixture = Fixture::new();
    fixture.model.workspace.show_connect = true;
    let mut h = harness(&mut fixture);
    assert_eq!(
        h.try_click("profile-auto-connect"),
        Err("Automatic connection requires a password saved in the desktop keyring.".into())
    );
    assert!(!h.model.workspace.auto_connect);
    h.toggle("remember-password");
    assert!(h.model.workspace.remember_password);
    h.toggle("profile-auto-connect");
    assert!(h.model.workspace.auto_connect);
    assert_eq!(h.node("profile-auto-connect").checked, Some(true));
    h.toggle("remember-password");
    assert!(!h.model.workspace.auto_connect);
    assert_eq!(h.node("profile-auto-connect").checked, Some(false));
}

// -- What the port added ------------------------------------------------------

#[test]
fn the_six_connection_scenes_show_what_the_rune_scenes_showed() {
    for name in [
        "connect",
        "connect-insecure",
        "connections",
        "connection-settings",
        "connection-trust",
        "certificate-review",
    ] {
        let spec = scene::find(name).unwrap_or_else(|| panic!("{name} is not registered"));
        assert_eq!(spec.area, "connections");
        assert!(!scene::unported().contains(&name));
        let h = Harness::scene(name);
        assert!(!h.model.experimental_notice(), "{name}");
        // Scenes are synthetic: nothing in them can reach a host or disk.
        assert!(h.model.backend.is_err(), "{name}");
        assert!(!h.model.workspace.writable, "{name}");
        for surface in h.surfaces() {
            assert!(!surface.experimental, "{name}");
            assert!(duplicate_keys(&surface.nodes()).is_empty(), "{name}");
        }
    }

    let h = Harness::scene("connect");
    assert_eq!(h.surface().id, connect::DIALOG);
    assert_eq!(h.surface().title, "Connect to XCP-ng");
    assert_eq!(h.node("host-endpoint").value.as_deref(), Some(""));
    assert_eq!(h.node("insecure-tls").checked, Some(false));
    assert!(h.find("certificate-options").is_some());
    assert!(
        h.find("proxy-mode").is_none(),
        "the saved connection options are closed"
    );

    let h = Harness::scene("connect-insecure");
    assert_eq!(h.node("insecure-tls").checked, Some(true));
    assert!(h.find("certificate-options").is_none());

    let h = Harness::scene("connections");
    assert_eq!(h.surface_id(), None);
    assert_eq!(h.model.active_tab(), TabId::Home);
    let rows: Vec<_> = h
        .nodes()
        .into_iter()
        .filter(|node| node.role == "row")
        .filter_map(|node| node.key)
        .collect();
    assert_eq!(
        rows,
        [
            "connections:c0000000-0000-0000-0000-000000000001",
            "connections:c0000000-0000-0000-0000-000000000002"
        ]
    );
    for text in [
        "Lab pool",
        "https://xcp-lab.example/",
        "Recovery pool",
        "https://recovery.example/",
        "operator",
        "Read-only",
        "Disconnected",
    ] {
        assert!(h.has_text(text), "{text}");
    }
    assert!(h.node("server.connect-all").is_disabled());
    assert!(h.node("server.disconnect-all").is_disabled());

    let h = Harness::scene("connection-settings");
    assert_eq!(
        h.node("profile-name").value.as_deref(),
        Some("Lab recovery")
    );
    assert_eq!(
        h.node("host-endpoint").value.as_deref(),
        Some("https://xcp-lab.example")
    );
    assert_eq!(h.node("profile-timeout").value.as_deref(), Some("60"));
    assert_eq!(h.node("profile-read-only").checked, Some(true));
    assert_eq!(h.node("proxy-mode-0").checked, Some(true));
    assert!(h.find("save-profile").is_some());

    let h = Harness::scene("connection-trust");
    assert_eq!(h.node("proxy-mode-2").checked, Some(true));
    assert_eq!(
        h.node("proxy-url").value.as_deref(),
        Some("http://proxy.example:3128")
    );
    assert!(
        h.node("proxy-route")
            .text
            .unwrap()
            .contains("HTTP CONNECT via http://proxy.example:3128/")
    );
    assert!(h.find("inspect-endpoint").is_none());

    let h = Harness::scene("certificate-review");
    assert_eq!(h.surface().title, "Review certificate");
    assert!(h.has_text("Certificate changed"));
    assert!(h.has_text(&"aa".repeat(32)) && h.has_text(&"bb".repeat(32)));
    assert!(h.has_text("TLS inspection completed in 23 ms."));
    assert!(h.node("accept-certificate").is_disabled());
}

#[test]
fn the_commands_are_those_of_the_plans_menus_and_of_the_rune_connection_cards() {
    let places: Vec<_> = command::registry()
        .iter()
        .filter(|spec| spec.area == "connections")
        .map(|spec| (spec.id.0, spec.label, spec.menu))
        .collect();
    assert_eq!(
        places,
        [
            ("server.add", "Add Server…", Some(place::SERVER_ADD)),
            ("server.connect", "Connect…", Some(place::SERVER_CONNECT)),
            ("server.cancel-connect", "Cancel connection attempt", None),
            (
                "server.disconnect",
                "Disconnect",
                Some(place::SERVER_DISCONNECT)
            ),
            (
                "pool.disconnect",
                "Disconnect",
                Some(place::POOL_DISCONNECT)
            ),
            (
                "server.reconnect-as",
                "Reconnect As…",
                Some(place::SERVER_RECONNECT_AS)
            ),
            (
                "pool.reconnect-as",
                "Reconnect As…",
                Some(place::POOL_RECONNECT_AS)
            ),
            (
                "server.review-certificate",
                "Review this endpoint's certificate…",
                None
            ),
            (
                "server.forget-password",
                "Forget Password",
                Some(place::SERVER_PASSWORD_FORGET)
            ),
            ("server.remove-connection", "Remove connection", None),
            (
                "server.connect-all",
                "Connect All",
                Some(place::SERVER_CONNECT_ALL)
            ),
            (
                "server.disconnect-all",
                "Disconnect All",
                Some(place::SERVER_DISCONNECT_ALL)
            ),
        ]
    );
    let add = command::find(CommandId("server.add")).unwrap();
    assert_eq!(add.toolbar, Some(ToolbarSlot::ADD_SERVER));
    // With nothing selected the menus offer what needs no object.
    let h = Harness::new();
    for id in ["server.add", "server.connect-all", "server.disconnect-all"] {
        assert!(h.menu_item(id).is_some(), "{id}");
    }
    for id in [
        "server.connect",
        "server.disconnect",
        "pool.disconnect",
        "server.reconnect-as",
        "pool.reconnect-as",
        "server.forget-password",
    ] {
        assert!(h.menu_item(id).is_none(), "{id}");
        assert_eq!(h.eligibility(id), Eligibility::Hidden, "{id}");
    }
    // A fixture owns no state directory: the store's own explanation.
    assert!(h.model.workspace.store.is_err());
    assert_eq!(
        h.menu_item("server.connect-all").unwrap().disabled,
        h.model.workspace.store.as_ref().err().cloned()
    );
}

#[test]
fn add_server_opens_the_empty_form_and_cancel_forgets_the_typed_password() {
    let mut fixture = Fixture::new();
    let mut h = home(&mut fixture);
    assert!(h.has_text(
        "Save a host address to reconnect to it later. Passwords are requested each session."
    ));
    h.click("server.add");
    assert_eq!(h.surface().id, connect::DIALOG);
    assert!(!h.surface().experimental);
    h.assert_unique_keys();
    assert_eq!(h.model.workspace.editing, None);
    assert_eq!(h.node("username").value.as_deref(), Some("root"));
    h.type_text("host-endpoint", "host.example");
    h.type_text("password", "typed-secret");
    assert!(!h.has_text("typed-secret"), "a secret shows bullets");
    assert_eq!(h.model.workspace.password.expose(), "typed-secret");
    // Any change of the endpoint turns certificate validation back on.
    h.toggle("insecure-tls");
    assert!(h.model.workspace.accept_invalid_certificates);
    assert!(h.find("certificate-options").is_none());
    h.type_text("host-endpoint", "other.example");
    assert!(!h.model.workspace.accept_invalid_certificates);
    assert!(h.find("certificate-options").is_some());
    // Enter in the password field is Connect. This fixture has no backend:
    // the refusal shows in the dialog, and nothing was saved before it.
    h.submit("password");
    assert!(h.model.backend.is_err());
    assert_eq!(
        h.node("connect-error").text,
        h.model.backend.as_ref().err().cloned()
    );
    assert!(h.model.workspace.profiles().is_empty());
    assert!(h.model.workspace.show_connect);
    h.dismiss();
    assert_eq!(h.surface_id(), None);
    assert!(h.model.workspace.password.is_empty());
}

#[test]
fn the_home_tab_lists_saved_connections_and_its_row_is_the_target_of_the_commands() {
    let mut fixture = Fixture::new();
    let kept = save_profile(&mut fixture.model, "Keyring fixture", true, true);
    let plain = save_profile(&mut fixture.model, "Session fixture", false, false);
    let mut h = home(&mut fixture);
    let body = h.tab().blocks();
    assert!(duplicate_keys(&flatten_blocks("tab", &body)).is_empty());
    assert!(h.has_text("Connects automatically"));
    assert!(h.find("connection-details").is_none(), "no row is selected");
    assert_eq!(
        ids(&h.local_row()),
        ["server.add", "server.connect-all", "server.disconnect-all"]
    );

    // A saved connection that is not connected.
    h.select_row(TABLE, &plain.id.to_string());
    assert_eq!(
        h.model.target(),
        Target::one(ObjectRef::new(None, ObjectKind::Connection, plain.id))
    );
    assert!(h.model.scope().is_empty(), "the tree selection stays");
    assert_eq!(
        h.node(&format!("connections:{}", plain.id)).checked,
        Some(true)
    );
    assert_eq!(
        h.node(&format!("profile-summary-{}", plain.id))
            .text
            .as_deref(),
        Some("https://fixture.example/ · operator · Disconnected")
    );
    let row = h.local_row();
    assert_eq!(
        ids(&row),
        [
            "server.add",
            "server.connect",
            "server.reconnect-as",
            "server.remove-connection",
            "server.connect-all",
            "server.disconnect-all"
        ]
    );
    assert_eq!(reason(&row, "server.connect"), None);
    assert_eq!(
        reason(&row, "server.reconnect-as").as_deref(),
        Some("Connect to this pool first")
    );
    assert_eq!(reason(&row, "server.remove-connection"), None);
    assert_eq!(
        ids(&h.row_menu(TABLE)),
        [
            "server.connect",
            "server.reconnect-as",
            "server.remove-connection"
        ]
    );
    assert_eq!(
        h.eligibility("server.forget-password"),
        Eligibility::Hidden,
        "no password of this connection is in the keyring"
    );
    // The same commands are in the context menu of its node in the tree.
    let node = h.model.target();
    let context = h.context_menu(&node);
    for id in ["server.connect", "server.remove-connection"] {
        assert_eq!(reason(&context, id), None);
    }

    // Remove: a session-only connection goes at once.
    h.click("server.remove-connection");
    assert!(h.model.workspace.profile(plain.id).is_none());
    assert!(h.find(&format!("connections:{}", plain.id)).is_none());
    assert!(
        h.model.target().is_empty(),
        "the selection does not outlive its row"
    );

    // A connection with a keyring password: Forget Password asks the
    // keyring first, and nothing else may change the connection meanwhile.
    h.select_row(TABLE, &kept.id.to_string());
    assert_eq!(
        h.eligibility("server.forget-password"),
        Eligibility::Enabled
    );
    h.invoke("server.forget-password");
    assert!(matches!(
        h.model.workspace.credential_tasks.as_slice(),
        [(CredentialWork::Forget(id), _)] if *id == kept.id
    ));
    assert_eq!(
        h.node("connections-credential-status").text.as_deref(),
        Some("Removing the saved password…")
    );
    const BUSY: &str = "Wait for the current password-store operation, then retry.";
    assert_eq!(
        h.eligibility("server.forget-password"),
        Eligibility::disabled(BUSY)
    );
    assert_eq!(h.try_click("server.remove-connection"), Err(BUSY.into()));
    assert!(h.model.workspace.profile(kept.id).is_some());

    // The shared error line shows on the tab.
    h.model.error = Some("A refusal.".into());
    assert_eq!(
        h.node("connections-error").text.as_deref(),
        Some("A refusal.")
    );
}

#[test]
fn activating_a_row_opens_its_editor_and_a_waiting_attempt_offers_only_cancel() {
    let mut fixture = Fixture::new();
    let kept = save_profile(&mut fixture.model, "Keyring fixture", true, false);
    let mut h = home(&mut fixture);
    // Double click or Enter: "Connect / edit", which loads the saved
    // password for the editor.
    h.activate_row(TABLE, &kept.id.to_string());
    assert_eq!(h.surface().id, connect::DIALOG);
    assert_eq!(h.model.workspace.editing, Some(kept.id));
    assert_eq!(h.node("remember-password").checked, Some(true));
    assert!(h.find("load-password").is_some() && h.find("forget-password").is_some());
    assert!(matches!(
        h.model.workspace.credential_tasks.as_slice(),
        [(CredentialWork::Load(load), _)] if load.purpose == LoadPurpose::Editor
    ));
    finish(&mut h.model, kept.id, loaded());
    assert!(h.model.workspace.cached_credentials.is_some());
    assert!(
        h.model.workspace.password.is_empty(),
        "a loaded password is not typed into the field"
    );
    // Unticking "remember" on a connection whose password is in the keyring
    // forgets that password instead of toggling.
    h.toggle("remember-password");
    assert!(h.model.workspace.remember_password);
    assert!(matches!(
        h.model.workspace.credential_tasks.as_slice(),
        [(CredentialWork::Forget(id), _)] if *id == kept.id
    ));
    assert!(
        h.node("credential-status")
            .text
            .unwrap()
            .contains("Removing the saved password")
    );
    h.click("cancel-connect");
    assert!(h.model.workspace.cached_credentials.is_none());

    // While an attempt waits for the keyring, the row offers its
    // cancellation and neither Connect nor Remove.
    let mut fixture = Fixture::new();
    let waiting = save_profile(&mut fixture.model, "Waiting fixture", true, false);
    let mut h = home(&mut fixture);
    h.click("server.connect-all");
    h.select_row(TABLE, &waiting.id.to_string());
    assert_eq!(
        ids(&h.local_row()),
        [
            "server.add",
            "server.cancel-connect",
            "server.reconnect-as",
            "server.connect-all",
            "server.disconnect-all"
        ]
    );
    h.activate_row(TABLE, &waiting.id.to_string());
    assert_eq!(h.surface_id(), None, "activation does nothing meanwhile");
    h.click("server.cancel-connect");
    assert!(h.model.workspace.loads.is_empty());
    assert!(h.has_text("Connection attempt cancelled."));
    assert_eq!(h.eligibility("server.connect"), Eligibility::Enabled);
}

#[test]
fn the_demo_pool_is_a_session_the_commands_explain_but_cannot_change() {
    const DEMO: &str = "Demo mode cannot change infrastructure.";
    let mut h = Harness::with(Model::demo(3, 48));
    let pool = h.model.scope().clone();
    assert_eq!(pool.kind(), ObjectKind::Pool);
    assert_eq!(h.eligibility("server.connect"), Eligibility::disabled(DEMO));
    assert_eq!(h.eligibility("server.disconnect"), Eligibility::Hidden);
    assert_eq!(h.eligibility("pool.disconnect"), Eligibility::Hidden);
    assert_eq!(
        h.eligibility("pool.reconnect-as"),
        Eligibility::disabled("Connect to this pool first")
    );
    assert_eq!(
        h.eligibility("server.remove-connection"),
        Eligibility::disabled(DEMO)
    );
    // On the Home tab the demo's connection is listed, and activating its
    // row shows the pool.
    h.model.show_home();
    assert!(h.has_text("Demo"));
    let id = h.model.workspace.selected.unwrap().to_string();
    h.activate_row(TABLE, &id);
    assert_eq!(h.model.scope(), &pool);
}
