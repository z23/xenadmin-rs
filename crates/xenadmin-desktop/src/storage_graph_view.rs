use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    ClassAvailability, ObservedResource, OperationId, OperationState, PbdId, ResourceAction,
    ResourceCommand, ResourceTarget, ReviewedPbd, StorageGraph, StorageRepairHost,
};
pub(super) struct Editor {
    pool: PoolId,
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    initial: Arc<Inventory>,
    source: Option<PbdId>,
    hosts: BTreeSet<HostId>,
    review: Option<StorageGraph>,
    error: Option<String>,
}
pub(super) fn open(s: &mut State, target: ResourceTarget) {
    let Some(initial) = s.inventory.clone() else {
        return;
    };
    let source = if let ResourceTarget::Storage(sr) = target {
        initial
            .details
            .pbds
            .values()
            .filter(|p| p.sr == Some(sr) && p.location.as_ref().is_some_and(|l| l.shared()))
            .map(|p| p.id)
            .next()
    } else {
        None
    };
    s.storage_graph = Some(Editor {
        pool: initial.pool_id,
        target,
        observed: s
            .snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(target).ok()),
        initial,
        source,
        hosts: BTreeSet::new(),
        review: None,
        error: None,
    });
    s.storage_editor = None;
    s.host_workspace = None;
}
fn editor(s: &mut State) -> &mut Editor {
    s.storage_graph.as_mut().expect("storage graph editor")
}
fn current(s: &State, e: &Editor) -> bool {
    s.inventory.as_ref().is_some_and(|i| i.pool_id == e.pool)
        && (s.demo
            || e.observed.is_some()
                && e.observed
                    == s.snapshot
                        .as_ref()
                        .and_then(|s| s.observe_resource(e.target).ok()))
}
fn proposal(e: &Editor) -> Result<StorageGraph, String> {
    let i = &e.initial;
    if i.details.classes.get("pbd") != Some(&ClassAvailability::Available) {
        return Err("Complete attachment inventory is required".into());
    }
    match e.target {
        ResourceTarget::Storage(sr) => {
            let source = ReviewedPbd::from_inventory(
                i,
                e.source.ok_or("Choose an existing source connection")?,
            )
            .filter(|p| p.sr == sr)
            .ok_or("Source connection details are incomplete")?;
            let hosts = e
                .hosts
                .iter()
                .map(|h| {
                    let pairs = i
                        .details
                        .pbds
                        .values()
                        .filter(|p| p.host == Some(*h) && p.sr == Some(sr))
                        .collect::<Vec<_>>();
                    if pairs.len() > 1 {
                        return Err("A host has duplicate storage connections".to_string());
                    }
                    let attachment = pairs
                        .first()
                        .map(|p| {
                            ReviewedPbd::from_inventory(i, p.id)
                                .ok_or("Connection details are incomplete".to_string())
                        })
                        .transpose()?;
                    Ok(StorageRepairHost {
                        host: *h,
                        attachment,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let op = StorageGraph::Repair {
                coordinator: i.coordinator.ok_or("Pool coordinator is unknown")?,
                source,
                hosts,
            };
            if !op.availability(i, e.target).is_available() {
                return Err(
                    "Select 1–32 missing or detached host connections on shared supported storage"
                        .into(),
                );
            }
            Ok(op)
        }
        ResourceTarget::Host(host) => {
            let d = i
                .details
                .hosts
                .get(&host)
                .ok_or("Host details are incomplete")?;
            let expected = d.multipathing.ok_or("Host multipathing mode is unknown")?;
            let expected_handler = d
                .standard_multipath_handler
                .ok_or("Custom or unknown multipath handler needs separate configuration")?;
            let attachments = i
                .details
                .pbds
                .values()
                .filter(|p| p.host == Some(host))
                .map(|p| {
                    ReviewedPbd::from_inventory(i, p.id)
                        .ok_or("Complete attachment details are required")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if d.enabled != Some(false)
                || d.maintenance_mode != Some(true)
                || i.details.pool.as_ref().and_then(|p| p.ha_enabled) != Some(false)
            {
                return Err(
                    "First disable HA, evacuate guests and enter maintenance for this host".into(),
                );
            }
            if i.vms
                .values()
                .any(|v| !v.is_control_domain && v.host == Some(host))
            {
                return Err("Evacuate all resident guests first".into());
            }
            let op = StorageGraph::Multipath {
                host,
                expected,
                expected_handler,
                enabled: !expected,
                attachments,
            };
            if !op.availability(i, e.target).is_available() {
                return Err(
                    "The complete host attachment graph is unavailable or exceeds 64 connections"
                        .into(),
                );
            }
            Ok(op)
        }
        _ => Err("Choose a host or shared storage repository".into()),
    }
}
fn stage(s: &mut State) {
    let result = {
        let e = s.storage_graph.as_ref().unwrap();
        if !current(s, e) {
            Err("Captured connection changed; reopen this workflow".into())
        } else {
            proposal(e).and_then(|op| {
                if s.inventory
                    .as_ref()
                    .is_some_and(|i| op.availability(i, e.target).is_available())
                {
                    Ok(op)
                } else {
                    Err("Reviewed attachment or host state changed; reopen this workflow".into())
                }
            })
        }
    };
    match result {
        Ok(op) => {
            editor(s).review = Some(op);
            editor(s).error = None
        }
        Err(e) => editor(s).error = Some(e),
    }
}
fn can_submit(s: &State, e: &Editor, op: &StorageGraph) -> bool {
    !s.demo
        && s.can_mutate()
        && s.resource_operations.len() < 64
        && current(s, e)
        && s.inventory
            .as_ref()
            .is_some_and(|i| op.availability(i, e.target).is_available())
        && !session_permissions_view::assess(s, &op.permission_requirements())
            .decision()
            .is_denied()
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s
            .storage_graph
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?;
        let op = e.review.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if !can_submit(s, e, op) {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::StorageGraph {
                    target: e.observed.ok_or(xenadmin_core::Error::Stale)?,
                    operation: op.clone(),
                },
            )
    })();
    match result {
        Ok(o) => {
            if let Some(w) = &s.waker {
                o.notify_on_change(w.clone())
            }
            s.resource_operations.push(o);
            editor(s).review = None;
            editor(s).error=Some("Submitted. History below tracks each host; reopen the workflow before submitting another change.".into());
        }
        Err(err) => editor(s).error = Some(err.to_string()),
    }
}
fn host_name(i: &Inventory, id: HostId) -> String {
    i.hosts
        .get(&id)
        .map(|h| h.name.clone())
        .unwrap_or_else(|| "Host".into())
}
fn attachment_line(i: &Inventory, p: &ReviewedPbd, k: &Kit) -> El {
    let mut out = col()
        .gap(4.)
        .child(
            heading(
                format!(
                    "{} · {}",
                    host_name(i, p.host),
                    if p.attached { "attached" } else { "detached" }
                ),
                k,
            )
            .wrap(),
        )
        .child(
            caption(
                format!("Host {}\nRepository {}\nConnection {}", p.host, p.sr, p.pbd),
                k,
            )
            .wrap_anywhere(),
        );
    if let Some(location) = &p.location {
        for line in storage_view::location_lines(location) {
            out = out.child(caption(line, k).wrap_anywhere());
        }
    }
    out
}

fn review_body(i: &Inventory, op: &StorageGraph, k: &Kit) -> El {
    let mut body = col().gap(10.).child(heading(op.label(), k));
    match op {
        StorageGraph::Repair {
            coordinator,
            source,
            hosts,
        } => {
            body=body.child(heading("Configuration source",k)).child(attachment_line(i,source,k)).child(caption("Creates only reviewed missing host/SR connections, then plugs selected connections. Coordinator runs first when selected. Custom or host-dependent configuration is refused before any change.",k).wrap()).child(caption(if source.requires_credentials{"The host copies source credential references into fresh attachment-owned Secrets. Password values are not read or shown. Failed creation may leave unattributed resources; no automatic deletion is attempted."}else{"Uses the exact existing source connection configuration, rechecked before creation."},k).wrap());
            let mut hosts = hosts.clone();
            hosts.sort_by_key(|h| (h.host != *coordinator, h.host));
            for h in hosts {
                body = body.child(heading(host_name(i, h.host), k).wrap()).child(
                    caption(
                        format!(
                            "{} · {}{}",
                            h.host,
                            if h.attachment.is_none() {
                                "Create and connect"
                            } else {
                                "Connect existing"
                            },
                            if h.host == *coordinator {
                                " · coordinator"
                            } else {
                                ""
                            }
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                if let Some(p) = h.attachment {
                    body = body.child(caption(format!("Connection {}", p.pbd), k).wrap_anywhere())
                }
            }
        }
        StorageGraph::Multipath {
            host,
            expected,
            expected_handler,
            enabled,
            attachments,
        } => {
            body=body.child(caption(format!("{} ({host})\nMultipathing: {expected} → {enabled}\nHandler: {} → {}",host_name(i,*host),if *expected_handler{"dmp"}else{"absent"},if *enabled{"dmp"}else{"absent"}),k).wrap_anywhere()).child(caption("Disconnects every originally attached repository, changes the host policy, then reconnects exactly that subset. Originally detached connections remain detached. The backend requires a disabled, empty maintenance host, HA off and no active/reserved iSCSI or HBA storage consumers across the pool. Storage used for caching or metadata replication is refused.",k).wrap());
            for p in attachments {
                body = body.child(attachment_line(i, p, k)).child(
                    caption(
                        if p.attached {
                            "Disconnect, then reconnect after the policy change"
                        } else {
                            "Preserve detached state"
                        },
                        k,
                    )
                    .wrap(),
                )
            }
            body=body.child(caption("This does not configure storage-array paths, switches or custom multipath.conf. Attached status and policy readback do not prove working multipath I/O.",k).wrap());
        }
    }
    body.child(caption("Each step is journaled. A refusal or uncertain response stops later writes; earlier changes remain. Recheck only reads state and never continues pending steps. No automatic compensation or cleanup occurs. After a known partial result, use the separately reviewed Restore connection controls below. Resolve unknown outcomes before starting another change.",k).wrap())
}
fn history(s: &State, e: &Editor, k: &Kit) -> El {
    let mut out = col()
        .gap(8.)
        .child(heading("Recent storage workflow results", k));
    for r in s
        .resource_rows
        .iter()
        .filter(|r| {
            r.pool == e.pool
                && r.target == e.target
                && matches!(r.action, ResourceAction::StorageGraph(_))
        })
        .take(20)
    {
        out = out
            .child(heading(format!("{} · {:?}", r.action.label(), r.state), k).wrap())
            .child(caption(&r.detail, k).wrap());
        for step in &r.steps {
            out = out.child(caption(format!("{} · {:?}", step.label, step.state), k).wrap())
        }
        for result in &r.storage_graph {
            out = out.child(
                caption(
                    format!(
                        "Host {} · connection {} · {}{}",
                        result.host,
                        result.pbd,
                        if !result.verified {
                            "identity retained; configuration unverified"
                        } else if result.attached {
                            "verified attached"
                        } else {
                            "verified detached"
                        },
                        if result.created {
                            " · newly created"
                        } else {
                            ""
                        }
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
            if r.state == OperationState::PartiallyCompleted && result.verified && !result.attached
            {
                let id = result.pbd;
                let sr = result.sr;
                let host = result.host;
                let pool = e.pool;
                let available = s
                    .inventory
                    .as_ref()
                    .filter(|i| i.pool_id == pool)
                    .and_then(|i| i.details.pbds.get(&id))
                    .is_some_and(|p| {
                        p.host == Some(result.host)
                            && p.sr == Some(sr)
                            && p.currently_attached == Some(false)
                    });
                out = out.child(
                    Button::new("Restore this connection…")
                        .disabled(!available || !current(s, e))
                        .on_press(move |s: &mut State| {
                            if s.inventory.as_ref().is_some_and(|i| i.pool_id == pool) {
                                s.storage_graph = None;
                                storage_view::review_restore(s, sr, id, host);
                            }
                        })
                        .build(k)
                        .key(format!("storage-restore-{id}")),
                );
            }
        }
    }
    out.child(
        Button::new("Open Activity history")
            .on_press(|s: &mut State| {
                s.storage_graph = None;
                s.activity.show = true;
            })
            .build(k),
    )
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(e) = &s.storage_graph else {
        return el();
    };
    let width = (s.window_width - 24.).clamp(280., 780.);
    let proposed = proposal(e);
    let op = e.review.as_ref().or_else(|| proposed.as_ref().ok());
    let assessment = op.map(|o| session_permissions_view::assess(s, &o.permission_requirements()));
    let mut body = col()
        .w_full()
        .grow(1.)
        .min_h(0.)
        .gap(12.)
        .padding(20.)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(heading(
            if e.review.is_some() {
                "Review storage workflow"
            } else {
                "Storage connections and multipathing"
            },
            k,
        ))
        .child(
            caption(
                format!(
                    "{}\nPool {} · target {}",
                    e.initial.pool_name,
                    e.pool,
                    e.target.uuid()
                ),
                k,
            )
            .wrap_anywhere(),
        );
    if let Some(op) = &e.review {
        body = body.child(review_body(&e.initial, op, k));
    } else if let ResourceTarget::Storage(sr) = e.target {
        body=body.child(heading("Choose a configuration source",k)).child(caption("Shared supported repositories only. Choose one existing connection as the source, then up to 32 missing or detached host connections. Physical storage ownership and custom per-host options are not inferred.",k).wrap());
        for p in e
            .initial
            .details
            .pbds
            .values()
            .filter(|p| p.sr == Some(sr))
            .take(128)
        {
            let id = p.id;
            body = body.child(
                Button::new(format!(
                    "{}{} · {}",
                    if e.source == Some(id) { "✓ " } else { "" },
                    p.host
                        .map(|h| host_name(&e.initial, h))
                        .unwrap_or_else(|| "Unknown host".into()),
                    id
                ))
                .on_press(move |s: &mut State| {
                    editor(s).source = Some(id);
                    editor(s).hosts.clear();
                })
                .build(k)
                .key(format!("storage-source-{id}")),
            );
        }
        body = body.child(heading("Selected host connections", k));
        for host in e.initial.hosts.values().take(128) {
            let id = host.id;
            let pairs = e
                .initial
                .details
                .pbds
                .values()
                .filter(|p| p.sr == Some(sr) && p.host == Some(id))
                .collect::<Vec<_>>();
            let eligible =
                pairs.is_empty() || pairs.len() == 1 && pairs[0].currently_attached == Some(false);
            let status = if pairs.is_empty() {
                "missing"
            } else if eligible {
                "detached"
            } else {
                "already attached or incomplete"
            };
            body = body.child(
                Checkbox::new(e.hosts.contains(&id))
                    .label(format!("{} · {status}", host.name))
                    .disabled(!eligible)
                    .on_toggle(move |s: &mut State| {
                        let e = editor(s);
                        if !e.hosts.remove(&id) {
                            e.hosts.insert(id);
                        }
                        e.error = None;
                    })
                    .build(k)
                    .key(format!("storage-host-{id}")),
            );
        }
    } else if let Ok(op) = &proposed {
        body = body.child(review_body(&e.initial, op, k));
    }
    if let Err(err) = &proposed {
        body = body.child(caption(err, k).wrap().color(k.c.danger.icon));
    }
    if !current(s, e) {
        body = body.child(
            caption(
                "Captured connection changed or inventory is stale. Reopen this workflow.",
                k,
            )
            .wrap()
            .color(k.c.danger.icon),
        );
    }
    if let Some(op) = &e.review
        && s.inventory
            .as_ref()
            .is_none_or(|i| !op.availability(i, e.target).is_available())
    {
        body = body.child(
            caption(
                "Reviewed host or attachment data changed; submission is disabled.",
                k,
            )
            .wrap()
            .color(k.c.danger.icon),
        );
    }
    if let Some(err) = &e.error {
        body = body.child(caption(err, k).wrap());
    }
    if s.demo {
        body = body.child(caption("Synthetic preview · no changes can be submitted", k).wrap());
    }
    body = body.child(history(s, e, k));
    let mut controls = row().gap(8.).flex_wrap().justify_end().child(
        Button::new("Close")
            .on_press(|s: &mut State| s.storage_graph = None)
            .build(k)
            .key("storage-graph-close"),
    );
    if let Some(op) = &e.review {
        controls = controls
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| editor(s).review = None)
                    .build(k)
                    .key("storage-graph-back"),
            )
            .child(
                Button::new("Confirm storage changes")
                    .disabled(!can_submit(s, e, op))
                    .on_press(submit)
                    .build(k)
                    .key("storage-graph-submit"),
            );
    } else {
        controls = controls.child(
            Button::new("Review changes…")
                .disabled(proposed.is_err() || !current(s, e))
                .on_press(stage)
                .build(k)
                .key("storage-graph-review"),
        );
    }
    el().w_full()
        .h_full()
        .absolute(0., 0.)
        .center()
        .fill(rune::Color::BLACK.alpha(0.55))
        .role("dialog")
        .label("Storage workflow")
        .modal(false, |s: &mut State| s.storage_graph = None)
        .child(
            col()
                .w(width)
                .h((s.window_height - 32.).clamp(260., 820.))
                .fill(k.c.bg_base)
                .radius(12.)
                .child(body)
                .child(
                    session_permissions_view::footer(controls, assessment.as_ref(), width - 40., k)
                        .padding(16.),
                ),
        )
}
pub(super) fn scene(s: &mut State, multipath: bool) {
    demo(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let hosts = i.hosts.keys().copied().take(2).collect::<Vec<_>>();
    let sr = *i.details.storage.keys().next().unwrap();
    i.coordinator = Some(hosts[0]);
    i.details
        .classes
        .insert("pbd".into(), ClassAvailability::Available);
    i.details.storage.get_mut(&sr).unwrap().shared = Some(true);
    i.details.pool.as_mut().unwrap().ha_enabled = Some(false);
    i.details.pbds.clear();
    let id = PbdId::from(uuid::Uuid::from_u128(9950));
    i.details.pbds.insert(
        id,
        xenadmin_core::StorageAttachment {
            id,
            host: Some(hosts[0]),
            sr: Some(sr),
            currently_attached: Some(true),
            location: Some(xenadmin_core::StorageLocation::Nfs {
                server: "storage.example.test".into(),
                path: "/data".into(),
                version: xenadmin_core::NfsVersion::Automatic,
            }),
            requires_credentials: false,
            multipathed: None,
            paths: vec![],
        },
    );
    if multipath {
        let h = i.details.hosts.entry(hosts[0]).or_default();
        h.enabled = Some(false);
        h.maintenance_mode = Some(true);
        h.multipathing = Some(false);
        h.standard_multipath_handler = Some(false);
        for v in i.vms.values_mut() {
            v.host = None;
        }
        open(s, ResourceTarget::Host(hosts[0]));
    } else {
        open(s, ResourceTarget::Storage(sr));
        editor(s).hosts.insert(hosts[1]);
    }
    stage(s);
}
#[cfg(test)]
#[path = "storage_graph_view_tests.rs"]
mod tests;
