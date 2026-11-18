//! The NBD service purposes of a network: the Rune `network_service_ui.rs`
//! (child `service` of the networking workspace), carried over.
//!
//! The module never had an editor. Its two buttons, "Enable / Remove NBD
//! with TLS…" and "Enable / Remove NBD without TLS…", staged a complete
//! review in one press; they are the commands `network.service.nbd-tls`
//! and `network.service.nbd-plaintext`, whose review is the shared dialog
//! `network-change` of the `network` area ([`open`]), with
//! [`unavailable`] as the staleness rule of the review ([`CHECK`]).
//!
//! What the Rune view showed without a button (the current purposes, why
//! a change is unavailable) is [`section`]: information for the
//! Networking tab of a network.

use xenadmin_core::{
    Inventory, NetworkId, NetworkOperation, NetworkServicePurpose, NetworkServiceReview, PoolId,
    ResourceAction, ResourceTarget,
};

use crate::areas::network::{self, Check, Workspace, name, pif_name};
use crate::prelude::*;

/// The two purposes, in the order of the Rune buttons.
pub const PURPOSES: [NetworkServicePurpose; 2] = [
    NetworkServicePurpose::NbdTls,
    NetworkServicePurpose::NbdPlaintext,
];

/// The staleness rule of a service review. `submit` answers
/// `ResourceConflict` while the reason is there, as the Rune `submit` did.
pub const CHECK: Check = Check {
    reason: unavailable,
    conflict: true,
};

pub fn lines(
    i: &Inventory,
    network: NetworkId,
    review: &NetworkServiceReview,
    purpose: NetworkServicePurpose,
    enabled: bool,
) -> Vec<String> {
    let mut lines = vec![
        format!("Network: {} ({network})", name(i, network)),
        format!("{}: {}", purpose.label(), if enabled { "enable" } else { "remove" }),
        "This changes the host NBD service policy for this network. Interface addresses and other network policies are retained.".into(),
        "The policy also applies to interfaces attached to this network later. The interfaces below must still match at submission.".into(),
    ];
    if purpose == NetworkServicePurpose::NbdPlaintext && enabled {
        lines.push("Plaintext NBD has no transport confidentiality or integrity. This explicitly enables the host's insecure_nbd policy.".into());
    }
    if !enabled {
        lines.push("Existing NBD sessions may be interrupted when the host removes access through this network.".into());
    }
    if review.interfaces.is_empty() {
        lines.push(
            "No current host interfaces. The policy will apply when interfaces are attached later."
                .into(),
        );
    }
    for p in &review.interfaces {
        lines.push(format!(
            "{} · PIF {} · host {} · {} · IPv4 {} · IPv6 {}",
            pif_name(i, p.pif),
            p.pif,
            p.identity.host,
            if p.attached { "attached" } else { "unplugged" },
            p.ipv4
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "none".into()),
            if p.ipv6.is_empty() {
                "none".into()
            } else {
                p.ipv6.iter().cloned().collect::<Vec<_>>().join(", ")
            }
        ));
    }
    lines.push("Completion verifies the saved service policy and interface identities. It does not test the NBD listener, firewall propagation or guest data transfers.".into());
    lines
}

/// What the button of a purpose would do now (the Rune `view`, per
/// purpose): the captured review, whether it enables or removes the
/// purpose, and whether the core lets it.
pub struct Offer {
    pub review: NetworkServiceReview,
    pub enable: bool,
    pub available: Result<(), String>,
}

/// `Err`: the service policy of the network cannot be captured (Rune
/// showed only this text in place of the buttons).
pub fn offer(
    i: &Inventory,
    network: NetworkId,
    purpose: NetworkServicePurpose,
) -> Result<Offer, String> {
    let review = NetworkServiceReview::capture(i, network).map_err(|error| error.to_string())?;
    let enable = !review.purposes.contains(purpose.as_str());
    let available = review
        .availability(i, network, purpose, enable)
        .map(|_| ())
        .map_err(|error| error.to_string());
    Ok(Offer {
        review,
        enable,
        available,
    })
}

/// The label of the Rune button, without its ellipsis.
pub fn label(purpose: NetworkServicePurpose, enable: bool) -> String {
    format!(
        "{} {}",
        if enable { "Enable" } else { "Remove" },
        purpose.label()
    )
}

/// Stages the change of one purpose and opens its review: the Rune
/// `review_button`. The lines are computed now, from the inventory the
/// operation is captured from.
pub fn open(m: &mut Model, pool: PoolId, network: NetworkId, purpose: NetworkServicePurpose) {
    let Some(i) = m.inventory_for(pool) else {
        return;
    };
    let Ok(offer) = offer(&i, network, purpose) else {
        return;
    };
    if offer.available.is_err() {
        return;
    }
    let lines = lines(&i, network, &offer.review, purpose, offer.enable);
    super::close(m);
    network::open_review(
        m,
        pool,
        Some(network),
        None,
        ResourceTarget::Network(network),
        ResourceAction::Network(NetworkOperation::SetServicePurpose {
            review: offer.review,
            purpose,
            enabled: offer.enable,
        }),
        lines,
        Some(CHECK),
    );
}

pub fn unavailable(m: &Model, w: &Workspace) -> Option<String> {
    let review = w.review.as_ref()?;
    let ResourceAction::Network(NetworkOperation::SetServicePurpose {
        review: captured,
        purpose,
        enabled,
    }) = &review.operation
    else {
        return None;
    };
    let ResourceTarget::Network(network) = review.target else {
        return Some("The reviewed network changed".into());
    };
    let Some(i) = m.inventory_for(w.pool) else {
        return Some("The connection changed; reopen the service review".into());
    };
    captured
        .availability(&i, network, *purpose, *enabled)
        .err()
        .map(|e| e.to_string())
}

/// The information of the Rune `service::view`: the heading, the current
/// purposes, and under each purpose that cannot be changed now the reason
/// (Rune: the caption under its disabled button). The buttons are the two
/// commands.
pub fn section(i: &Inventory, network: NetworkId) -> Vec<Block> {
    let mut body = blocks![heading("Network services")];
    let review = match NetworkServiceReview::capture(i, network) {
        Ok(review) => review,
        Err(error) => {
            body.push(caption(error.to_string()).key("network-service-policy").into());
            return body;
        }
    };
    body.push(
        caption(if review.purposes.is_empty() {
            "NBD service policy: disabled".into()
        } else {
            format!(
                "Current service purposes: {}",
                review
                    .purposes
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .key("network-service-policy")
        .into(),
    );
    for purpose in PURPOSES {
        let enable = !review.purposes.contains(purpose.as_str());
        if let Err(error) = review.availability(i, network, purpose, enable) {
            body.push(
                caption(format!("{}: {error}", label(purpose, enable)))
                    .key(format!("network-service-{}", purpose.as_str()))
                    .into(),
            );
        }
    }
    body
}
