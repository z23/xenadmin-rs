//! Add Server: the connect form and, inside the same dialog, the review of
//! an inspected certificate.
//!
//! This is the Rune `connection_dialog` of `main.rs` with
//! `profiles_view::form_fields`, `profile_trust::proxy_fields`,
//! `certificate_fields` and `review_controls`. The draft is the connect form
//! of `m.workspace` (`session.rs`): the dialog is open while
//! `m.workspace.show_connect`, and it serves a new connection
//! (`session::begin_new`) as well as "Connect / edit" of a saved one
//! (`session::edit`). Once an inspection has a result
//! (`m.workspace.trust.observation`), the review replaces the form and its
//! footer; Back and the accept button are all that remain.

use xenadmin_core::CredentialMode;

use super::trust;
use crate::prelude::*;
use crate::session;

/// The id and key of the dialog.
pub const DIALOG: &str = "connect-dialog";

/// The dialog, while the connect form is open.
pub fn surface(m: &Model) -> Option<Surface> {
    if !m.workspace.show_connect {
        return None;
    }
    let error = m
        .error
        .clone()
        .map(|error| alert(error).key("connect-error"));
    let surface = match &m.workspace.trust.observation {
        Some(observation) => Surface::dialog(DIALOG, "Review certificate")
            .body(blocks![certificate_fields(m), error])
            .footer([
                button("back-from-certificate", "Back", trust::back),
                button(
                    "accept-certificate",
                    if observation.previous_fingerprint.is_some() {
                        "Replace pin"
                    } else {
                        "Trust certificate"
                    },
                    trust::accept,
                )
                .primary()
                .disabled_opt(trust::accept_refusal(m)),
            ]),
        None => Surface::dialog(DIALOG, "Connect to XCP-ng")
            .body(blocks![form(m), error])
            .footer([
                button("cancel-connect", "Cancel", close),
                button("connect", "Connect", session::connect).primary(),
            ]),
    };
    Some(surface.on_close(close))
}

/// Cancel, Escape and the close button: the editor's side state goes, and
/// the password with it.
pub fn close(m: &mut Model) {
    session::cancel_editor(m);
    m.workspace.show_connect = false;
    m.workspace.password.clear();
}

/// Whether the saved connection being edited keeps its password in the
/// desktop keyring.
fn stored_with_keyring(m: &Model) -> bool {
    m.workspace
        .editing
        .and_then(|id| m.workspace.profile(id))
        .is_some_and(|profile| profile.credential_mode == CredentialMode::SecretService)
}

/// "Remember password in desktop keyring". Unticking it on a connection
/// whose password is in the keyring forgets that password instead of
/// toggling: the connection cannot be saved as session-only before.
fn toggle_remember(m: &mut Model) {
    if m.workspace.remember_password && stored_with_keyring(m) {
        session::forget_password(m);
    } else {
        m.workspace.remember_password = !m.workspace.remember_password;
        if !m.workspace.remember_password {
            m.workspace.auto_connect = false;
            m.workspace.cached_credentials = None;
        }
    }
}

fn toggle_auto_connect(m: &mut Model) {
    if m.workspace.remember_password {
        m.workspace.auto_connect = !m.workspace.auto_connect;
    }
}

/// "Choose CA certificate…": asks the front end for a file. The answer is
/// used only by the editor that asked; a cancelled prompt keeps the path.
pub fn choose_ca(m: &mut Model) {
    let epoch = m.workspace.editor_epoch;
    m.choose_file(
        FileRequest::open("Choose CA certificate", move |m, path| {
            if !m.workspace.show_connect || m.workspace.editor_epoch != epoch {
                return;
            }
            let Some(path) = path else {
                return;
            };
            match path.to_str() {
                Some(path) => {
                    m.workspace.ca_certificate = path.to_owned();
                    m.workspace.cached_credentials = None;
                    m.error = None;
                }
                None => {
                    m.error =
                        Some("Choose a certificate path with valid Unicode characters.".into())
                }
            }
        })
        .filter("PEM certificates", ["pem", "crt", "cer"]),
    );
}

/// The proxy route choice and the session proxy credentials.
fn proxy_fields(m: &Model) -> Vec<Block> {
    let p = &m.workspace.trust;
    let selected = p.mode.min(2).to_string();
    let route = session::form_profile(m).ok().map(|profile| {
        let route = session::trust_options(m, &profile).and_then(|options| {
            options
                .connection_route(&profile.endpoint)
                .map_err(|e| e.to_string())
        });
        caption(match route {
            Ok(route) => format!("For {}: {route}", profile.endpoint),
            Err(error) => error,
        })
        .key("proxy-route")
    });
    let mut body = blocks![
        // The options keep the Rune keys `proxy-mode-0`, `-1` and `-2`.
        choice(
            "proxy-mode",
            [("0", "Environment"), ("1", "Direct"), ("2", "HTTP proxy")],
            Some(selected.as_str()),
            |m, id| {
                let mode = id.parse().unwrap_or(0);
                if m.workspace.trust.mode != mode {
                    m.workspace.trust.mode = mode;
                    m.workspace.trust.password.clear();
                }
            },
        )
        .label("Connection route")
        .segmented(),
        route,
    ];
    if p.mode == 0 {
        body.push(caption("Captures HTTP(S)_PROXY, ALL_PROXY and NO_PROXY when connecting. The same route is used by API requests, consoles, metrics and transfers. HTTPS proxies, SOCKS and PAC are unsupported.").into());
    }
    if p.mode == 2 {
        body.extend(blocks![
            input("proxy-url", "HTTP proxy URL", &p.proxy, |m, text| {
                m.workspace.trust.proxy = text.to_owned()
            })
            .placeholder("http://proxy.example:3128")
            .help("HTTP CONNECT proxy URL"),
            input("proxy-username", "Proxy username", &p.username, |m, text| {
                m.workspace.trust.username = text.to_owned()
            })
            .help("Proxy username • optional"),
            secret_input("proxy-password", "Proxy password", &p.password, |m, text| {
                m.workspace.trust.password.set(text)
            })
            .help("Proxy password • current application session only"),
            caption("HTTP proxy authentication is sent to the proxy without TLS. Host traffic remains HTTPS. Proxy passwords stay in memory for this application session and are never saved with connection profiles."),
            button("forget-proxy-password", "Forget proxy password", |m| {
                m.workspace.trust.password.clear();
                if let Some(id) = m.workspace.editing {
                    m.workspace.proxy_sessions.remove(&id);
                }
            }),
        ]);
    }
    body
}

/// The saved-connection part of the form: name, the options section,
/// keyring and start-up choices.
fn form_fields(m: &Model) -> Vec<Block> {
    let ws = &m.workspace;
    let mut fields = blocks![
        input("profile-name", "Connection name", &ws.name, |m, text| {
            m.workspace.name = text.to_owned()
        })
        .placeholder("Connection name (optional)"),
        button("profile-options", "Saved connection options", |m| {
            m.workspace.advanced = !m.workspace.advanced
        }),
    ];
    if ws.advanced {
        fields.extend(blocks![
            proxy_fields(m),
            input(
                "profile-ca",
                "CA certificate file",
                &ws.ca_certificate,
                |m, text| { m.workspace.ca_certificate = text.to_owned() }
            )
            .placeholder("CA certificate file (optional)"),
            button("choose-ca-file", "Choose CA certificate…", choose_ca).disabled_if(
                ws.accept_invalid_certificates,
                "A CA certificate is not used while certificate validation is disabled.",
            ),
            input(
                "profile-timeout",
                "Request timeout in seconds",
                &ws.timeout,
                |m, text| { m.workspace.timeout = text.to_owned() }
            ),
            checkbox(
                "profile-read-only",
                "Read-only connection",
                ws.read_only,
                |m| { m.workspace.read_only = !m.workspace.read_only }
            ),
            button(
                "save-profile",
                "Save without connecting",
                session::save_only
            ),
        ]);
    }
    fields.extend(blocks![
        checkbox(
            "remember-password",
            "Remember password in desktop keyring",
            ws.remember_password,
            toggle_remember,
        ),
        checkbox(
            "profile-auto-connect",
            "Connect automatically when XenAdmin RS starts",
            ws.auto_connect,
            toggle_auto_connect,
        )
        .help("Automatic connection requires a password saved in the desktop keyring. A locked or unavailable keyring leaves the connection disconnected.")
        .disabled_if(
            !ws.remember_password,
            "Automatic connection requires a password saved in the desktop keyring.",
        ),
        ws.credential_status
            .clone()
            .map(|status| caption(status).key("credential-status")),
    ]);
    if stored_with_keyring(m) {
        fields.push(actions([
            button("load-password", "Load saved password", |m| {
                if let Some(profile) = m
                    .workspace
                    .editing
                    .and_then(|id| m.workspace.profile(id))
                    .cloned()
                {
                    session::load_password(m, &profile);
                }
            }),
            button(
                "forget-password",
                "Forget saved password",
                session::forget_password,
            ),
        ]));
    }
    fields
}

/// The form state of the dialog.
fn form(m: &Model) -> Vec<Block> {
    let ws = &m.workspace;
    let mut body = blocks![
        form_fields(m),
        caption("Use a host management address. Credentials go directly to that host over HTTPS."),
        input("host-endpoint", "Host endpoint", &ws.endpoint, |m, text| {
            // Any change of the endpoint turns certificate validation back on.
            if m.workspace.endpoint != text {
                m.workspace.endpoint = text.to_owned();
                m.workspace.accept_invalid_certificates = false;
            }
        })
        .placeholder("https://xcp-host.example"),
        input("username", "Username", &ws.username, |m, text| {
            m.workspace.username = text.to_owned()
        })
        .placeholder("root"),
        secret_input("password", "Password", &ws.password, |m, text| {
            m.workspace.password.set(text)
        })
        .placeholder("Host password")
        .help("Password • or load a saved password in connection options")
        .on_submit(session::connect),
        checkbox(
            "insecure-tls",
            "Disable certificate validation",
            ws.accept_invalid_certificates,
            |m| {
                m.workspace.accept_invalid_certificates = !m.workspace.accept_invalid_certificates
            },
        )
        .help("Applies to this connection, coordinator redirects and consoles. Host identity is not verified."),
    ];
    if !ws.accept_invalid_certificates {
        body.push(
            button("certificate-options", "Certificate options", |m| {
                m.workspace.show_certificate_details = !m.workspace.show_certificate_details
            })
            .into(),
        );
    }
    if !ws.accept_invalid_certificates && ws.show_certificate_details {
        body.extend(blocks![
            certificate_fields(m),
            input(
                "certificate-fingerprint",
                "Certificate SHA-256 • optional",
                &ws.fingerprint,
                |m, text| { m.workspace.fingerprint = text.to_owned() }
            )
            .placeholder("Verified fingerprint for a self-signed host")
            .help("Leave the fingerprint empty for CA and hostname verification. Verify a fingerprint through a trusted channel before using it."),
            textarea(
                "host-certificates",
                "Additional host certificates",
                &ws.host_certificates,
                |m, text| { m.workspace.host_certificates = text.to_owned() }
            )
            .placeholder("One https://host=SHA256 entry per line")
            .help("Other pool host certificates • optional"),
        ]);
    }
    body
}

/// The inspection controls and, once there is an observation, its review:
/// what the certificate options of the form show, and all the review state
/// of the dialog shows.
fn certificate_fields(m: &Model) -> Vec<Block> {
    let p = &m.workspace.trust;
    let mut body = blocks![
        caption("Inspect an exact host endpoint"),
        input(
            "inspect-endpoint",
            "Certificate endpoint",
            &p.endpoint,
            |m, text| { m.workspace.trust.endpoint = text.to_owned() }
        )
        .placeholder("Use the connection endpoint, or a pool member"),
        button("inspect-certificate", "Inspect certificate", trust::inspect),
        caption("Inspection sends no XAPI login. Verify the SHA-256 fingerprint through a trusted channel before accepting it. A pin trusts this exact certificate even when its CA, hostname or validity checks fail."),
        p.status
            .clone()
            .map(|status| caption(status).key("certificate-status")),
    ];
    let Some(o) = &p.observation else {
        return body;
    };
    let changed = o
        .previous_fingerprint
        .as_ref()
        .is_some_and(|old| old != &o.details.fingerprint);
    let mut lines = Vec::new();
    if let Some(old) = p.details.get(o.endpoint.as_str()) {
        lines.push(format!(
            "Previously accepted: {} · issuer {} · valid until {}",
            old.subject, old.issuer, old.valid_until
        ));
    }
    lines.push(format!(
        "TLS inspection completed in {} ms. Type the exact endpoint above to update only its certificate pin.",
        o.elapsed_ms
    ));
    body.extend(blocks![
        heading(if changed {
            "Certificate changed"
        } else {
            "Certificate inspection"
        }),
        review("certificate-review", lines)
            .affected([
                property("Endpoint", o.endpoint.to_string()),
                property("Route", o.route.clone()),
                property("Validation", o.validation.label()),
                property(
                    "Previous SHA-256",
                    o.previous_fingerprint
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "No saved pin".into()),
                ),
                property("Presented SHA-256", o.details.fingerprint.to_string()).copyable(),
                property("Subject", o.details.subject.clone()),
                property("Issuer", o.details.issuer.clone()),
                property("Names", o.details.names.join(", ")),
                property("Valid from", o.details.valid_from.clone()),
                property("Valid until", o.details.valid_until.clone()),
                property("Serial", o.details.serial.clone()),
            ])
            .stale(trust::stale(m)),
        input(
            "confirm-certificate-endpoint",
            "Confirm exact endpoint",
            &p.confirmation,
            |m, text| { m.workspace.trust.confirmation = text.to_owned() }
        ),
        checkbox(
            "certificate-verified",
            "I verified this SHA-256 through a trusted channel",
            p.verified,
            |m| { m.workspace.trust.verified = !m.workspace.trust.verified }
        ),
    ]);
    body
}
