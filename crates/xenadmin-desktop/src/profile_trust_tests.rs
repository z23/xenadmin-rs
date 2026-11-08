use super::*;
fn runtime(size: (f32, f32), review: bool) -> rune::Runtime<State> {
    let mut s = init();
    scene(&mut s, review);
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
        view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    for _ in 0..3 {
        r.frame(1.0 / 60.0);
    }
    r
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    r.update(|_, tx| tx.scroll_into_view(key));
    for _ in 0..3 {
        r.frame(1.0 / 60.0);
    }
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    assert!(
        y >= 0.0 && y < r.state.window_height,
        "{key} outside viewport at {y}"
    );
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    r.frame(0.0);
}
#[test]
fn compact_and_wide_editor_keep_header_and_connect_controls_visible() {
    for size in [(1440.0, 900.0), (513.0, 652.0), (480.0, 600.0)] {
        let mut r = runtime(size, false);
        for key in ["connect-heading", "cancel-connect", "connect"] {
            let (_, y) = r.find(key).unwrap();
            assert!(y >= 0.0 && y < size.1, "{key}: {y}");
        }
        click(&mut r, "proxy-mode-1");
        assert_eq!(r.state.workspace.trust.mode, 1);
        click(&mut r, "proxy-mode-2");
        assert_eq!(r.state.workspace.trust.mode, 2);
        click(&mut r, "cancel-connect");
        assert!(!r.state.show_connect);
    }
}
#[test]
fn reviewed_changed_pin_requires_exact_endpoint_and_explicit_fingerprint_check() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut r = runtime(size, true);
        let original = r.state.host_certificates.text.clone();
        click(&mut r, "accept-certificate");
        assert_eq!(r.state.host_certificates.text, original);
        r.update(|s, _| s.workspace.trust.confirmation = TextEdit::new("https://another.example/"));
        click(&mut r, "certificate-verified");
        click(&mut r, "accept-certificate");
        assert_eq!(r.state.host_certificates.text, original);
        r.update(|s, _| {
            s.workspace.trust.confirmation = TextEdit::new("https://coordinator.example/")
        });
        click(&mut r, "accept-certificate");
        assert!(r.state.host_certificates.text.contains(&"bb".repeat(32)));
        assert_eq!(r.state.endpoint.text, "https://xcp-lab.example");
        assert!(r.state.show_connect);
        assert!(r.state.workspace.active.is_empty());
        assert!(
            r.state
                .workspace
                .trust
                .status
                .as_ref()
                .unwrap()
                .contains("unsaved")
        );
    }
}
#[test]
fn delayed_inspection_and_review_cannot_follow_changed_route_pin_or_editor() {
    for changed in [
        "endpoint", "proxy", "ca", "pin", "password", "epoch", "closed",
    ] {
        let mut r = runtime((1440.0, 900.0), true);
        let observation = r.state.workspace.trust.observation.clone().unwrap();
        r.update(|s, _| {
            match changed {
                "endpoint" => s.workspace.trust.endpoint = TextEdit::new("https://other.example"),
                "proxy" => s.workspace.trust.mode = 1,
                "ca" => s.workspace.ca_certificate = TextEdit::new("/tmp/different-ca.pem"),
                "pin" => {
                    s.host_certificates =
                        TextEdit::new(format!("https://coordinator.example/={}", "cc".repeat(32)))
                }
                "password" => s.workspace.trust.password = TextEdit::new("new-proxy-secret"),
                "epoch" => s.workspace.editor_epoch = uuid::Uuid::new_v4(),
                "closed" => s.show_connect = false,
                _ => unreachable!(),
            }
            s.workspace.trust.result = Some(Ok(observation));
            poll(s);
        });
        assert!(r.state.workspace.trust.observation.is_none(), "{changed}");
        assert!(!can_accept(&r.state));
    }
}
#[test]
fn proxy_credentials_are_session_only_and_bound_to_the_exact_proxy_configuration() {
    let mut r = runtime((1440.0, 900.0), false);
    r.update(|s, _| {
        s.workspace.trust.proxy = TextEdit::new("http://proxy.example:3128");
        s.workspace.trust.username = TextEdit::new("proxy-user");
        s.workspace.trust.password = TextEdit::new("private-proxy-secret");
    });
    let p = form_profile(&r.state).unwrap();
    assert!(options(&r.state, &p).is_ok());
    assert!(
        !serde_json::to_string(&p)
            .unwrap()
            .contains("private-proxy-secret")
    );
    remember_proxy(&mut r.state, &p);
    assert!(r.state.workspace.trust.password.text.is_empty());
    assert!(options(&r.state, &p).is_ok());
    let mut changed = p.clone();
    changed.proxy = ProxyConfiguration::Http {
        endpoint: ProxyEndpoint::parse("http://different.example:3128").unwrap(),
        username: Some("proxy-user".into()),
    };
    assert!(options(&r.state, &changed).is_err());
    r.update(|s, _| s.workspace.editing = Some(p.id));
    click(&mut r, "forget-proxy-password");
    assert!(options(&r.state, &p).is_err());
}
#[test]
fn synthetic_inspection_cannot_contact_an_endpoint_or_load_a_password() {
    let mut r = runtime((1440.0, 900.0), true);
    click(&mut r, "inspect-certificate");
    assert!(r.state.workspace.trust.job.is_none());
    assert!(
        r.state
            .workspace
            .trust
            .status
            .as_ref()
            .unwrap()
            .contains("preview")
    );
    assert!(r.state.workspace.credential_tasks.is_empty());
}
#[test]
fn reviewing_failed_member_certificate_does_not_load_saved_host_credentials() {
    let mut s = init();
    let directory = tempfile::tempdir().unwrap();
    s.workspace.store = Ok(ProfileStore::open(directory.path().into()).unwrap());
    s.workspace.writable = true;
    let mut profile = ConnectionProfile::new(
        "Saved".into(),
        Endpoint::parse("seed.example").unwrap(),
        "root".into(),
    );
    profile.credential_mode = CredentialMode::SecretService;
    s.workspace
        .store
        .as_mut()
        .unwrap()
        .save(profile.clone())
        .unwrap();
    inspect_failed(
        &mut s,
        profile.id,
        Endpoint::parse("coordinator.example").unwrap(),
    );
    assert!(s.show_connect && s.show_certificate_details);
    assert_eq!(
        s.workspace.trust.endpoint.text,
        "https://coordinator.example/"
    );
    assert!(s.workspace.credential_tasks.is_empty());
    assert!(s.password.text.is_empty());
}
