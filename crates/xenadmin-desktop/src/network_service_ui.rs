use super::*;
use xenadmin_core::{NetworkServicePurpose, NetworkServiceReview};

fn lines(
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

pub(super) fn view(i: &Inventory, network: NetworkId, enabled: bool, k: &Kit) -> El {
    let mut body = col().gap(8.0).child(heading("Network services", k));
    let review = match NetworkServiceReview::capture(i, network) {
        Ok(review) => review,
        Err(error) => return body.child(caption(error.to_string(), k).wrap()),
    };
    body = body.child(
        caption(
            if review.purposes.is_empty() {
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
            },
            k,
        )
        .wrap(),
    );
    for purpose in [
        NetworkServicePurpose::NbdTls,
        NetworkServicePurpose::NbdPlaintext,
    ] {
        let enable = !review.purposes.contains(purpose.as_str());
        let available = review.availability(i, network, purpose, enable);
        body = body.child(review_button(
            format!(
                "{} {}…",
                if enable { "Enable" } else { "Remove" },
                purpose.label()
            ),
            ResourceTarget::Network(network),
            NetworkOperation::SetServicePurpose {
                review: review.clone(),
                purpose,
                enabled: enable,
            },
            lines(i, network, &review, purpose, enable),
            enabled && available.is_ok(),
            k,
        ));
        if let Err(error) = available {
            body = body.child(caption(error.to_string(), k).wrap());
        }
    }
    body
}

pub(super) fn unavailable(s: &State, w: &Workspace) -> Option<String> {
    let review = w.review.as_ref()?;
    let xenadmin_core::ResourceAction::Network(NetworkOperation::SetServicePurpose {
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
    let Some(i) = s.inventory.as_ref().filter(|i| i.pool_id == w.pool) else {
        return Some("The connection changed; reopen the service review".into());
    };
    captured
        .availability(i, network, *purpose, *enabled)
        .err()
        .map(|e| e.to_string())
}

pub(super) fn scene(s: &mut State) {
    super::scene(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let network = NetworkId::from(i.networks[0].uuid);
    for n in &i.networks {
        let id = NetworkId::from(n.uuid);
        if !i.details.networks.contains_key(&id) {
            let mut details = i.details.networks[&network].clone();
            details.id = id;
            i.details.networks.insert(id, details);
        }
    }
    let p = i.details.pifs.values_mut().next().unwrap();
    p.network = Some(network);
    p.ip = "192.0.2.20".into();
    let review = NetworkServiceReview::capture(i, network).unwrap();
    let lines = lines(i, network, &review, NetworkServicePurpose::NbdTls, true);
    prepare(
        s,
        ResourceTarget::Network(network),
        NetworkOperation::SetServicePurpose {
            review,
            purpose: NetworkServicePurpose::NbdTls,
            enabled: true,
        },
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_captures_plaintext_policy_and_interfaces_at_both_widths() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let i = s.inventory.as_ref().unwrap();
            let network = NetworkId::from(i.networks[0].uuid);
            let review = NetworkServiceReview::capture(i, network).unwrap();
            let lines = lines(
                i,
                network,
                &review,
                NetworkServicePurpose::NbdPlaintext,
                true,
            );
            assert!(lines.iter().any(|s| s.contains("Plaintext NBD")));
            assert!(lines.iter().any(|s| s.contains("192.0.2.20")));
            prepare(
                &mut s,
                ResourceTarget::Network(network),
                NetworkOperation::SetServicePurpose {
                    review: review.clone(),
                    purpose: NetworkServicePurpose::NbdPlaintext,
                    enabled: true,
                },
                lines,
            );
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
            let (x, y) = runtime.find("network-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
            assert!(
                runtime
                    .state
                    .network_workspace
                    .as_ref()
                    .unwrap()
                    .review
                    .is_some()
            );
        }
    }
    #[test]
    fn changed_interface_pool_or_service_policy_invalidates_captured_review() {
        let mut s = crate::init();
        scene(&mut s);
        assert!(unavailable(&s, s.network_workspace.as_ref().unwrap()).is_none());
        let before = s.inventory.clone();
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pifs
            .values_mut()
            .next()
            .unwrap()
            .ip = "192.0.2.99".into();
        assert!(unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some());
        s.inventory = before.clone();
        Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id =
            PoolId::from(uuid::Uuid::from_u128(999));
        assert!(unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some());
        s.inventory = before;
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        let id = NetworkId::from(i.networks[1].uuid);
        i.details.networks.get_mut(&id).unwrap().purposes = Some(["insecure_nbd".into()].into());
        assert!(
            unavailable(&s, s.network_workspace.as_ref().unwrap())
                .unwrap()
                .contains("conflicts")
        );
    }
}
