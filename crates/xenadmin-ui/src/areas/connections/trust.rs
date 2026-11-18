//! Certificate inspection and pinning in the connect form: the part of the
//! Rune `profile_trust.rs` that is not on the connection path.
//!
//! The data is `m.workspace.trust` ([`crate::session::Trust`]); `populate`,
//! `form`, `options`, `remember_proxy` and `cancel` are in `session.rs`.
//! Here: starting an inspection, the staleness of its context, taking its
//! result, accepting a pin into the unsaved draft, and opening the form on
//! the endpoint whose certificate made a connection fail.
//!
//! Inspection sends no login and saves nothing. Acceptance changes only the
//! unsaved draft and never connects.

use xenadmin_core::{Endpoint, ProfileId};

use crate::prelude::*;
use crate::session::{self, TrustContext};

/// What the status line says when the connect form changed under an
/// inspection or its review.
pub const CHANGED: &str =
    "Connection settings changed. Inspect again before accepting a certificate.";

/// Whether the form is still what the inspection was started for: the same
/// open editor, the same proxy password, the same inspection endpoint and
/// the same formed profile.
pub fn current(m: &Model, c: &TrustContext) -> bool {
    m.workspace.show_connect
        && m.workspace.editor_epoch == c.epoch
        && session::trust_secret_revision(m) == c.secret_revision
        && Endpoint::parse(&m.workspace.trust.endpoint).ok().as_ref() == Some(&c.endpoint)
        && session::form_profile(m).is_ok_and(|mut p| {
            // A new profile gets a new id on every `form_profile` call.
            if m.workspace.editing.is_none() {
                p.id = c.profile.id;
            }
            p == c.profile
        })
}

/// "Inspect certificate". Any error becomes the status line. Nothing is
/// saved.
pub fn inspect(m: &mut Model) {
    let result = (|| {
        if !m.workspace.writable {
            return Err("This preview cannot contact hosts. Open a writable connection editor to inspect a certificate.".into());
        }
        let profile = session::form_profile(m)?;
        let endpoint = if m.workspace.trust.endpoint.trim().is_empty() {
            profile.endpoint.clone()
        } else {
            Endpoint::parse(&m.workspace.trust.endpoint).map_err(|e| e.to_string())?
        };
        let options = session::trust_options(m, &profile)?;
        m.workspace.trust.endpoint = endpoint.as_str().to_owned();
        let context = TrustContext {
            profile,
            epoch: m.workspace.editor_epoch,
            endpoint: endpoint.clone(),
            secret_revision: session::trust_secret_revision(m),
        };
        // No test contacts an endpoint: a test puts the answer into
        // `trust.result`.
        #[cfg(not(test))]
        {
            m.workspace.trust.job = Some(xenadmin_core::CertificateInspectionTask::start(
                options,
                endpoint,
                m.waker.clone(),
            ));
        }
        #[cfg(test)]
        {
            let _ = (options, endpoint);
        }
        m.workspace.trust.context = Some(context);
        m.workspace.trust.observation = None;
        m.workspace.trust.confirmation = String::new();
        m.workspace.trust.verified = false;
        m.workspace.trust.status =
            Some("Inspecting the endpoint's TLS certificate. No XAPI login is sent…".into());
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        m.workspace.trust.status = Some(error);
    }
}

/// The area's poll: drops an inspection whose context is no longer current,
/// before its result is taken; then takes the result.
pub fn poll(m: &mut Model) -> bool {
    if m.workspace
        .trust
        .context
        .as_ref()
        .is_some_and(|c| !current(m, c))
    {
        m.workspace.trust.job = None;
        m.workspace.trust.context = None;
        m.workspace.trust.observation = None;
        m.workspace.trust.status = Some(CHANGED.into());
        return true;
    }
    #[cfg(not(test))]
    let result = m
        .workspace
        .trust
        .job
        .as_ref()
        .and_then(|j| j.take_result())
        .map(|r| r.map_err(|e| e.to_string()));
    #[cfg(test)]
    let result = m.workspace.trust.result.take();
    let Some(result) = result else { return false };
    m.workspace.trust.job = None;
    match result {
        Ok(observation)
            if m.workspace
                .trust
                .context
                .as_ref()
                .is_some_and(|c| c.endpoint == observation.endpoint) =>
        {
            m.workspace.trust.observation = Some(observation);
            m.workspace.trust.status = None;
        }
        Ok(_) => {
            m.workspace.trust.status = Some(
                "Certificate inspection no longer matches this endpoint. Inspect again.".into(),
            )
        }
        Err(e) => m.workspace.trust.status = Some(e),
    }
    true
}

/// Whether the review is of a form that changed since the inspection: the
/// reason, for the review and for the accept button.
pub fn stale(m: &Model) -> Option<String> {
    let current = m
        .workspace
        .trust
        .context
        .as_ref()
        .is_some_and(|c| current(m, c));
    (!current).then(|| CHANGED.to_owned())
}

pub fn can_accept(m: &Model) -> bool {
    let p = &m.workspace.trust;
    p.context.as_ref().is_some_and(|c| current(m, c))
        && p.verified
        && !m.workspace.accept_invalid_certificates
        && p.observation.as_ref().is_some_and(|o| {
            p.confirmation.trim() == o.endpoint.as_str()
                && o.previous_fingerprint.as_ref() != Some(&o.details.fingerprint)
        })
}

/// Why the pin cannot be accepted now: what the disabled button says. `None`
/// exactly when [`can_accept`] holds. The Rune button was disabled without a
/// reason; the texts name the condition of [`can_accept`] that fails.
pub fn accept_refusal(m: &Model) -> Option<String> {
    if can_accept(m) {
        return None;
    }
    let p = &m.workspace.trust;
    let reason = if let Some(reason) = stale(m) {
        reason
    } else if m.workspace.accept_invalid_certificates {
        "Certificate validation is disabled for this connection, so no pin is used.".into()
    } else {
        match &p.observation {
            None => "Inspect the certificate first".into(),
            Some(o) if o.previous_fingerprint.as_ref() == Some(&o.details.fingerprint) => {
                "The presented certificate is already the pin saved for this endpoint.".into()
            }
            Some(o) if p.confirmation.trim() != o.endpoint.as_str() => {
                "Type the exact endpoint to update only its certificate pin.".into()
            }
            Some(_) => "Confirm that you verified this SHA-256 through a trusted channel.".into(),
        }
    };
    Some(reason)
}

/// "Trust certificate" or "Replace pin": the pin goes into the unsaved
/// draft. It neither saves nor connects.
pub fn accept(m: &mut Model) {
    if !can_accept(m) {
        return;
    }
    let result = (|| {
        let mut profile = session::form_profile(m)?;
        let observation = m
            .workspace
            .trust
            .observation
            .as_ref()
            .ok_or("Inspect the certificate first")?;
        profile
            .accept_certificate(observation)
            .map_err(|e| e.to_string())?;
        m.workspace.fingerprint = profile
            .host_certificates
            .get(profile.endpoint.as_str())
            .map(ToString::to_string)
            .unwrap_or_default();
        m.workspace.host_certificates = profile
            .host_certificates
            .iter()
            .filter(|(e, _)| e.as_str() != profile.endpoint.as_str())
            .map(|(e, p)| format!("{e}={p}"))
            .collect::<Vec<_>>()
            .join("\n");
        m.workspace.trust.details = profile.certificate_details;
        m.workspace.cached_credentials = None;
        m.workspace.trust.context = None;
        m.workspace.trust.observation = None;
        m.workspace.trust.status = Some(
            "Certificate pin added to unsaved connection settings. Save or connect when ready."
                .into(),
        );
        Ok::<_, String>(())
    })();
    if let Err(e) = result {
        m.workspace.trust.status = Some(e);
    }
}

/// "Back" from the certificate review: the observation and its context are
/// dropped and the form shows again.
pub fn back(m: &mut Model) {
    m.workspace.trust.observation = None;
    m.workspace.trust.context = None;
}

/// "Review this endpoint's certificate…": opens the connect form of the
/// saved connection with the certificate options expanded and the inspection
/// endpoint preset to the one that failed. It does not load the saved
/// password and does not start the inspection.
pub fn inspect_failed(m: &mut Model, id: ProfileId, endpoint: Endpoint) {
    if m.workspace.live(id) {
        m.error = Some("Disconnect this connection before changing its certificate trust.".into());
        return;
    }
    let Some(profile) = m.workspace.profile(id).cloned() else {
        return;
    };
    session::cancel_editor(m);
    session::cancel_load(m, id);
    session::populate(m, &profile);
    m.workspace.show_connect = true;
    m.error = None;
    m.workspace.show_certificate_details = true;
    m.workspace.trust.endpoint = endpoint.as_str().to_owned();
}
