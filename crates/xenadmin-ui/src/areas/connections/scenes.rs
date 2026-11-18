//! The connection scenes, under their Rune names.
//!
//! Scenes are synthetic: they cannot save, connect or touch the desktop
//! password store.

use xenadmin_core::{
    CertificateDetails, CertificateObservation, CertificateValidation, ConnectionProfile, Endpoint,
};

use super::AREA;
use crate::prelude::*;
use crate::session::{self, TrustContext, Workspace};

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("connect", AREA, connect),
        SceneSpec::new("connect-insecure", AREA, connect_insecure),
        SceneSpec::new("connections", AREA, connections),
        SceneSpec::new("connection-settings", AREA, connection_settings),
        SceneSpec::new("connection-trust", AREA, connection_trust),
        SceneSpec::new("certificate-review", AREA, certificate_review),
    ]
}

/// The empty connect form.
fn connect(m: &mut Model) {
    m.workspace.show_connect = true;
}

/// The connect form with certificate validation disabled: no "Certificate
/// options".
fn connect_insecure(m: &mut Model) {
    m.workspace.show_connect = true;
    m.workspace.accept_invalid_certificates = true;
}

/// The Rune `profiles_view::scene` before its two branches: a workspace that
/// owns nothing and a backend that cannot connect.
fn preview(m: &mut Model) {
    m.workspace = Workspace::load(None, false);
    m.backend = Err("Preview only".into());
    m.error = None;
}

fn preview_profile(id: &str, name: &str, endpoint: &str, username: &str) -> ConnectionProfile {
    let mut profile = ConnectionProfile::new(
        name.into(),
        Endpoint::parse(endpoint).expect("scene endpoint"),
        username.into(),
    );
    // A fixed id, so the tree of the scene is the same on every run.
    profile.id = session::profile_id(id).expect("scene profile id");
    profile
}

/// The Home tab with two saved connections that are not connected.
fn connections(m: &mut Model) {
    preview(m);
    let first = preview_profile(
        "c0000000-0000-0000-0000-000000000001",
        "Lab pool",
        "https://xcp-lab.example",
        "root",
    );
    let mut second = preview_profile(
        "c0000000-0000-0000-0000-000000000002",
        "Recovery pool",
        "https://recovery.example",
        "operator",
    );
    second.read_only = true;
    m.workspace.preview_profiles = vec![first, second];
    m.show_home();
}

/// The connect form with the saved-connection options open.
fn connection_settings(m: &mut Model) {
    preview(m);
    session::begin_new(m);
    m.workspace.endpoint = "https://xcp-lab.example".into();
    m.workspace.name = "Lab recovery".into();
    m.workspace.advanced = true;
    m.workspace.read_only = true;
    m.workspace.timeout = "60".into();
}

/// The Rune `profile_trust::scene`.
fn trust(m: &mut Model, review: bool) {
    connection_settings(m);
    m.workspace.advanced = !review;
    m.workspace.show_certificate_details = review;
    m.workspace.trust.mode = 2;
    m.workspace.trust.proxy = "http://proxy.example:3128".into();
    m.workspace.trust.endpoint = "https://coordinator.example/".into();
    if !review {
        return;
    }
    let endpoint = Endpoint::parse("https://coordinator.example").expect("scene endpoint");
    let old = "aa".repeat(32).parse().expect("scene pin");
    m.workspace.host_certificates = format!("{endpoint}={old}");
    let observation = CertificateObservation {
        endpoint: endpoint.clone(),
        details: CertificateDetails {
            fingerprint: "bb".repeat(32).parse().expect("scene fingerprint"),
            subject: "CN=coordinator.example".into(),
            issuer: "CN=Recovery lab CA".into(),
            names: vec!["coordinator.example".into()],
            valid_from: "Nov 8 00:00:00 2026 +00:00".into(),
            valid_until: "Nov 8 00:00:00 2027 +00:00".into(),
            serial: "01:23:45".into(),
        },
        previous_fingerprint: Some(old),
        validation: CertificateValidation::UnknownIssuer,
        route: "HTTP CONNECT via http://proxy.example:3128/".into(),
        inspected_at_ms: 0,
        elapsed_ms: 23,
    };
    let Ok(profile) = session::form_profile(m) else {
        return;
    };
    m.workspace.trust.context = Some(TrustContext {
        profile,
        epoch: m.workspace.editor_epoch,
        endpoint,
        secret_revision: session::trust_secret_revision(m),
    });
    m.workspace.trust.observation = Some(observation);
}

/// The connect form with the HTTP proxy fields.
fn connection_trust(m: &mut Model) {
    trust(m, false);
}

/// The certificate review of a changed certificate: "Replace pin" disabled.
fn certificate_review(m: &mut Model) {
    trust(m, true);
}
