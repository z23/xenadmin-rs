use super::*;
use std::hash::{Hash, Hasher};
use xenadmin_core::{
    CertificateDetails, CertificateInspectionTask, CertificateObservation, ProxyConfiguration,
    ProxyCredentials, ProxyEndpoint,
};

pub(super) struct Panel {
    mode: u8,
    proxy: TextEdit,
    username: TextEdit,
    password: TextEdit,
    endpoint: TextEdit,
    confirmation: TextEdit,
    verified: bool,
    details: BTreeMap<String, CertificateDetails>,
    context: Option<Context>,
    job: Option<CertificateInspectionTask>,
    observation: Option<CertificateObservation>,
    status: Option<String>,
    #[cfg(test)]
    result: Option<Result<CertificateObservation, String>>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            mode: 0,
            proxy: TextEdit::new(""),
            username: TextEdit::new(""),
            password: secure_field(),
            endpoint: TextEdit::new(""),
            confirmation: TextEdit::new(""),
            verified: false,
            details: BTreeMap::new(),
            context: None,
            job: None,
            observation: None,
            status: None,
            #[cfg(test)]
            result: None,
        }
    }
}
struct Context {
    profile: ConnectionProfile,
    epoch: uuid::Uuid,
    endpoint: Endpoint,
    secret_revision: u64,
}
fn secret_revision(s: &State) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    s.workspace.trust.password.text.hash(&mut hash);
    hash.finish()
}
pub(super) fn populate(s: &mut State, profile: &ConnectionProfile) {
    let mut p = Panel {
        details: profile.certificate_details.clone(),
        endpoint: TextEdit::new(profile.endpoint.as_str()),
        ..Default::default()
    };
    match &profile.proxy {
        ProxyConfiguration::Environment => {}
        ProxyConfiguration::Direct => p.mode = 1,
        ProxyConfiguration::Http { endpoint, username } => {
            p.mode = 2;
            p.proxy = TextEdit::new(endpoint.as_str());
            p.username = TextEdit::new(username.as_deref().unwrap_or(""));
        }
    }
    s.workspace.trust = p;
}
pub(super) fn form(s: &State, profile: &mut ConnectionProfile) -> Result<(), String> {
    let p = &s.workspace.trust;
    profile.proxy = match p.mode {
        0 => ProxyConfiguration::Environment,
        1 => ProxyConfiguration::Direct,
        _ => ProxyConfiguration::Http {
            endpoint: ProxyEndpoint::parse(&p.proxy.text).map_err(|e| e.to_string())?,
            username: (!p.username.text.trim().is_empty()).then(|| p.username.text.trim().into()),
        },
    };
    profile.certificate_details = p
        .details
        .iter()
        .filter(|(endpoint, d)| profile.host_certificates.get(*endpoint) == Some(&d.fingerprint))
        .map(|(e, d)| (e.clone(), d.clone()))
        .collect();
    Ok(())
}
pub(super) fn options(s: &State, profile: &ConnectionProfile) -> Result<ConnectionOptions, String> {
    let mut options = profile.options().map_err(|e| e.to_string())?;
    if let ProxyConfiguration::Http {
        username: Some(username),
        ..
    } = &profile.proxy
    {
        let credentials =
            if draft_proxy_matches(s, profile) && !s.workspace.trust.password.text.is_empty() {
                ProxyCredentials::new(username.clone(), s.workspace.trust.password.text.clone())
                    .map_err(|e| e.to_string())?
            } else {
                s.workspace
                    .proxy_sessions
                    .get(&profile.id)
                    .filter(|(p, _)| p == &profile.proxy)
                    .map(|(_, c)| c.clone())
                    .ok_or_else(|| xenadmin_core::Error::ProxyAuthenticationRequired.to_string())?
            };
        options
            .set_proxy_credentials(credentials)
            .map_err(|e| e.to_string())?;
    }
    options
        .connection_route(&profile.endpoint)
        .map_err(|e| e.to_string())?;
    Ok(options)
}
pub(super) fn remember_proxy(s: &mut State, profile: &ConnectionProfile) {
    if let ProxyConfiguration::Http {
        username: Some(username),
        ..
    } = &profile.proxy
        && draft_proxy_matches(s, profile)
        && !s.workspace.trust.password.text.is_empty()
        && let Ok(credentials) =
            ProxyCredentials::new(username.clone(), s.workspace.trust.password.text.clone())
    {
        s.workspace
            .proxy_sessions
            .insert(profile.id, (profile.proxy.clone(), credentials));
    }
    s.workspace.trust.password = secure_field();
}
pub(super) fn cancel(s: &mut State) {
    s.workspace.trust.job = None;
    s.workspace.trust.context = None;
    s.workspace.trust.observation = None;
    s.workspace.trust.password = secure_field();
}
fn draft_proxy_matches(s: &State, profile: &ConnectionProfile) -> bool {
    s.show_connect
        && s.workspace.editing.is_none_or(|id| id == profile.id)
        && s.workspace.trust.mode == 2
        && matches!(&profile.proxy, ProxyConfiguration::Http { endpoint, username }
            if ProxyEndpoint::parse(&s.workspace.trust.proxy.text).ok().as_ref() == Some(endpoint)
            && username.as_deref() == Some(s.workspace.trust.username.text.trim()))
}
fn current(s: &State, c: &Context) -> bool {
    s.show_connect
        && s.workspace.editor_epoch == c.epoch
        && secret_revision(s) == c.secret_revision
        && Endpoint::parse(&s.workspace.trust.endpoint.text)
            .ok()
            .as_ref()
            == Some(&c.endpoint)
        && form_profile(s).is_ok_and(|mut p| {
            if s.workspace.editing.is_none() {
                p.id = c.profile.id;
            }
            p == c.profile
        })
}
fn inspect(s: &mut State) {
    let result = (|| {
        if !s.workspace.writable {
            return Err("This preview cannot contact hosts. Open a writable connection editor to inspect a certificate.".into());
        }
        let profile = form_profile(s)?;
        let endpoint = if s.workspace.trust.endpoint.text.trim().is_empty() {
            profile.endpoint.clone()
        } else {
            Endpoint::parse(&s.workspace.trust.endpoint.text).map_err(|e| e.to_string())?
        };
        let options = options(s, &profile)?;
        s.workspace.trust.endpoint = TextEdit::new(endpoint.as_str());
        let context = Context {
            profile,
            epoch: s.workspace.editor_epoch,
            endpoint: endpoint.clone(),
            secret_revision: secret_revision(s),
        };
        #[cfg(not(test))]
        {
            s.workspace.trust.job = Some(CertificateInspectionTask::start(
                options,
                endpoint,
                s.waker.clone(),
            ));
        }
        #[cfg(test)]
        {
            let _ = (options, endpoint);
        }
        s.workspace.trust.context = Some(context);
        s.workspace.trust.observation = None;
        s.workspace.trust.confirmation = TextEdit::new("");
        s.workspace.trust.verified = false;
        s.workspace.trust.status =
            Some("Inspecting the endpoint's TLS certificate. No XAPI login is sent…".into());
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        s.workspace.trust.status = Some(error);
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    if s.workspace
        .trust
        .context
        .as_ref()
        .is_some_and(|c| !current(s, c))
    {
        s.workspace.trust.job = None;
        s.workspace.trust.context = None;
        s.workspace.trust.observation = None;
        s.workspace.trust.status = Some(
            "Connection settings changed. Inspect again before accepting a certificate.".into(),
        );
        return true;
    }
    #[cfg(not(test))]
    let result = s
        .workspace
        .trust
        .job
        .as_ref()
        .and_then(|j| j.take_result())
        .map(|r| r.map_err(|e| e.to_string()));
    #[cfg(test)]
    let result = s.workspace.trust.result.take();
    let Some(result) = result else { return false };
    s.workspace.trust.job = None;
    match result {
        Ok(observation)
            if s.workspace
                .trust
                .context
                .as_ref()
                .is_some_and(|c| c.endpoint == observation.endpoint) =>
        {
            s.workspace.trust.observation = Some(observation);
            s.workspace.trust.status = None;
        }
        Ok(_) => {
            s.workspace.trust.status = Some(
                "Certificate inspection no longer matches this endpoint. Inspect again.".into(),
            )
        }
        Err(e) => s.workspace.trust.status = Some(e),
    }
    true
}
fn can_accept(s: &State) -> bool {
    let p = &s.workspace.trust;
    p.context.as_ref().is_some_and(|c| current(s, c))
        && p.verified
        && !s.accept_invalid_certificates
        && p.observation.as_ref().is_some_and(|o| {
            p.confirmation.text.trim() == o.endpoint.as_str()
                && o.previous_fingerprint.as_ref() != Some(&o.details.fingerprint)
        })
}
fn accept(s: &mut State) {
    if !can_accept(s) {
        return;
    }
    let result = (|| {
        let mut profile = form_profile(s)?;
        let observation = s
            .workspace
            .trust
            .observation
            .as_ref()
            .ok_or("Inspect the certificate first")?;
        profile
            .accept_certificate(observation)
            .map_err(|e| e.to_string())?;
        s.fingerprint = TextEdit::new(
            profile
                .host_certificates
                .get(profile.endpoint.as_str())
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        s.host_certificates = TextEdit::new(
            profile
                .host_certificates
                .iter()
                .filter(|(e, _)| e.as_str() != profile.endpoint.as_str())
                .map(|(e, p)| format!("{e}={p}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        s.workspace.trust.details = profile.certificate_details;
        s.workspace.cached_credentials = None;
        s.workspace.trust.context = None;
        s.workspace.trust.observation = None;
        s.workspace.trust.status = Some(
            "Certificate pin added to unsaved connection settings. Save or connect when ready."
                .into(),
        );
        Ok::<_, String>(())
    })();
    if let Err(e) = result {
        s.workspace.trust.status = Some(e);
    }
}
pub(super) fn proxy_fields(s: &State, k: &Kit) -> El {
    let p = &s.workspace.trust;
    let mut choices = row().gap(6.0).flex_wrap();
    for (mode, label) in [(0, "Environment"), (1, "Direct"), (2, "HTTP proxy")] {
        choices = choices.child(
            Button::new(label)
                .variant(crate::theme::choice(mode == p.mode))
                .on_press(move |s: &mut State| {
                    if s.workspace.trust.mode != mode {
                        s.workspace.trust.mode = mode;
                        s.workspace.trust.password = secure_field();
                    }
                })
                .build(k)
                .key(format!("proxy-mode-{mode}")),
        );
    }
    let mut body = col()
        .gap(8.0)
        .child(caption("Connection route", k))
        .child(choices);
    if let Ok(profile) = form_profile(s) {
        let route = options(s, &profile).and_then(|o| {
            o.connection_route(&profile.endpoint)
                .map_err(|e| e.to_string())
        });
        body = body.child(
            caption(
                match route {
                    Ok(route) => format!("For {}: {route}", profile.endpoint),
                    Err(e) => e,
                },
                k,
            )
            .wrap()
            .key("proxy-route"),
        );
    }
    if p.mode == 0 {
        body = body.child(caption("Captures HTTP(S)_PROXY, ALL_PROXY and NO_PROXY when connecting. The same route is used by API requests, consoles, metrics and transfers. HTTPS proxies, SOCKS and PAC are unsupported.",k).wrap());
    }
    if p.mode == 2 {
        body = body.child(caption("HTTP CONNECT proxy URL",k)).child(TextInput::new(&p.proxy).label("HTTP proxy URL").placeholder("http://proxy.example:3128").on_edit(|s:&mut State|&mut s.workspace.trust.proxy).build(k).key("proxy-url"))
            .child(caption("Proxy username • optional",k)).child(TextInput::new(&p.username).label("Proxy username").on_edit(|s:&mut State|&mut s.workspace.trust.username).build(k).key("proxy-username"))
            .child(caption("Proxy password • current application session only",k)).child(TextInput::new(&p.password).secure().label("Proxy password").on_edit(|s:&mut State|&mut s.workspace.trust.password).build(k).key("proxy-password"))
            .child(caption("HTTP proxy authentication is sent to the proxy without TLS. Host traffic remains HTTPS. Proxy passwords stay in memory for this application session and are never saved with connection profiles.",k).wrap())
            .child(Button::new("Forget proxy password").variant(ButtonVariant::Outline).on_press(|s:&mut State| { s.workspace.trust.password = secure_field(); if let Some(id)=s.workspace.editing { s.workspace.proxy_sessions.remove(&id); } }).build(k).key("forget-proxy-password"));
    }
    body
}
pub(super) fn certificate_fields(s: &State, k: &Kit) -> El {
    let p = &s.workspace.trust;
    let mut body=col().gap(8.0).child(caption("Inspect an exact host endpoint",k))
        .child(TextInput::new(&p.endpoint).label("Certificate endpoint").placeholder("Use the connection endpoint, or a pool member").on_edit(|s:&mut State|&mut s.workspace.trust.endpoint).build(k).key("inspect-endpoint"))
        .child(Button::new("Inspect certificate").variant(ButtonVariant::Outline).on_press(inspect).build(k).key("inspect-certificate"))
        .child(caption("Inspection sends no XAPI login. Verify the SHA-256 fingerprint through a trusted channel before accepting it. A pin trusts this exact certificate even when its CA, hostname or validity checks fail.",k).wrap());
    if let Some(status) = &p.status {
        body = body.child(caption(status, k).wrap().key("certificate-status"));
    }
    if let Some(o) = &p.observation {
        body = body.child(heading(
            if o.previous_fingerprint
                .as_ref()
                .is_some_and(|old| old != &o.details.fingerprint)
            {
                "Certificate changed"
            } else {
                "Certificate inspection"
            },
            k,
        ));
        for (label, value) in [
            ("Endpoint", o.endpoint.to_string()),
            ("Route", o.route.clone()),
            ("Validation", o.validation.label().into()),
            (
                "Previous SHA-256",
                o.previous_fingerprint
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "No saved pin".into()),
            ),
            ("Presented SHA-256", o.details.fingerprint.to_string()),
            ("Subject", o.details.subject.clone()),
            ("Issuer", o.details.issuer.clone()),
            ("Names", o.details.names.join(", ")),
            ("Valid from", o.details.valid_from.clone()),
            ("Valid until", o.details.valid_until.clone()),
            ("Serial", o.details.serial.clone()),
        ] {
            body = body
                .child(caption(label, k))
                .child(text(value).font(k.ty.text_small).wrap_anywhere());
        }
        if let Some(old) = p.details.get(o.endpoint.as_str()) {
            body = body.child(
                caption(
                    format!(
                        "Previously accepted: {} · issuer {} · valid until {}",
                        old.subject, old.issuer, old.valid_until
                    ),
                    k,
                )
                .wrap(),
            );
        }
        body=body.child(caption(format!("TLS inspection completed in {} ms. Type the exact endpoint above to update only its certificate pin.",o.elapsed_ms),k).wrap())
            .child(TextInput::new(&p.confirmation).label("Confirm exact endpoint").on_edit(|s:&mut State|&mut s.workspace.trust.confirmation).build(k).key("confirm-certificate-endpoint"))
            .child(Checkbox::new(p.verified).label("I verified this SHA-256 through a trusted channel").on_toggle(|s:&mut State|s.workspace.trust.verified = !s.workspace.trust.verified).build(k).key("certificate-verified"))
            ;
    }
    body
}
pub(super) fn review_controls(s: &State, k: &Kit) -> Option<El> {
    let observation = s.workspace.trust.observation.as_ref()?;
    Some(
        row()
            .w_full()
            .h(64.0)
            .min_h(64.0)
            .padding(12.0)
            .gap(8.0)
            .justify_end()
            .child(
                Button::new("Back")
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut State| {
                        s.workspace.trust.observation = None;
                        s.workspace.trust.context = None;
                    })
                    .build(k)
                    .key("back-from-certificate"),
            )
            .child(
                Button::new(if observation.previous_fingerprint.is_some() {
                    "Replace pin"
                } else {
                    "Trust certificate"
                })
                .disabled(!can_accept(s))
                .on_press(accept)
                .build(k)
                .key("accept-certificate"),
            ),
    )
}
pub(super) fn inspect_failed(s: &mut State, id: ProfileId, endpoint: Endpoint) {
    if s.workspace.live(id) {
        s.error = Some("Disconnect this connection before changing its certificate trust.".into());
        return;
    }
    let Some(profile) = s.workspace.profile(id).cloned() else {
        return;
    };
    cancel_editor(s);
    cancel_load(s, id);
    super::populate(s, &profile);
    s.show_connect = true;
    s.workspace.show_manager = false;
    s.error = None;
    s.show_certificate_details = true;
    s.workspace.trust.endpoint = TextEdit::new(endpoint.as_str());
}

pub(super) fn scene(s: &mut State, review: bool) {
    super::scene(s, true);
    s.workspace.advanced = !review;
    s.show_certificate_details = review;
    s.workspace.trust.mode = 2;
    s.workspace.trust.proxy = TextEdit::new("http://proxy.example:3128");
    s.workspace.trust.endpoint = TextEdit::new("https://coordinator.example/");
    if review {
        let endpoint = Endpoint::parse("https://coordinator.example").unwrap();
        let old = "aa".repeat(32).parse().unwrap();
        s.host_certificates = TextEdit::new(format!("{endpoint}={old}"));
        let observation = CertificateObservation {
            endpoint: endpoint.clone(),
            details: CertificateDetails {
                fingerprint: "bb".repeat(32).parse().unwrap(),
                subject: "CN=coordinator.example".into(),
                issuer: "CN=Recovery lab CA".into(),
                names: vec!["coordinator.example".into()],
                valid_from: "Nov 8 00:00:00 2026 +00:00".into(),
                valid_until: "Nov 8 00:00:00 2027 +00:00".into(),
                serial: "01:23:45".into(),
            },
            previous_fingerprint: Some(old),
            validation: xenadmin_core::CertificateValidation::UnknownIssuer,
            route: "HTTP CONNECT via http://proxy.example:3128/".into(),
            inspected_at_ms: 0,
            elapsed_ms: 23,
        };
        let profile = form_profile(s).unwrap();
        s.workspace.trust.context = Some(Context {
            profile,
            epoch: s.workspace.editor_epoch,
            endpoint,
            secret_revision: secret_revision(s),
        });
        s.workspace.trust.observation = Some(observation);
    }
}

#[cfg(test)]
#[path = "profile_trust_tests.rs"]
mod tests;
