use super::*;
use xenadmin_core::{
    HostDataSource, HostDataSourceQuery, HostOperation, HostRecordingAction, ObservedResource,
    OperationId, PoolId, ResourceCommand, ResourceTarget,
};

pub(super) struct Panel {
    pool: PoolId,
    target: ResourceTarget,
    name: String,
    observed: Option<ObservedResource>,
    query: Option<HostDataSourceQuery>,
    sources: Option<Vec<HostDataSource>>,
    selected: Option<String>,
    search: TextEdit,
    page: usize,
    review: Option<HostOperation>,
    acknowledged: bool,
    error: Option<String>,
}
pub(super) fn open(s: &mut State, target: ResourceTarget) {
    let Some(i) = &s.inventory else { return };
    let ResourceTarget::Host(host) = target else {
        return;
    };
    let Some(h) = i.hosts.get(&host) else { return };
    s.host_monitoring = Some(Panel {
        pool: i.pool_id,
        target,
        name: h.name.clone(),
        observed: s
            .snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(target).ok()),
        query: None,
        sources: None,
        selected: None,
        search: TextEdit::new(""),
        page: 0,
        review: None,
        acknowledged: false,
        error: None,
    });
    s.host_workspace = None;
    console_view::release(s);
    refresh(s);
}
fn current(s: &State, p: &Panel) -> bool {
    s.inventory.as_ref().is_some_and(|i| i.pool_id == p.pool)
        && p.observed.is_some()
        && s.snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(p.target).ok())
            == p.observed
}
fn refresh(s: &mut State) {
    let Some(p) = &mut s.host_monitoring else {
        return;
    };
    p.query = None;
    p.sources = None;
    p.review = None;
    p.selected = None;
    p.error = None;
    p.acknowledged = false;
    p.page = 0;
    if s.demo {
        return;
    }
    let result = p
        .observed
        .ok_or(xenadmin_core::Error::Stale)
        .and_then(|observed| {
            s.connection
                .as_ref()
                .ok_or(xenadmin_core::Error::Closed)?
                .host_data_sources(observed)
        });
    match result {
        Ok(q) => {
            if let Some(w) = &s.waker {
                q.notify_on_change(w.clone());
            }
            p.query = Some(q);
        }
        Err(e) => p.error = Some(e.to_string()),
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(p) = &s.host_monitoring else {
        return false;
    };
    let Some(result) = p.query.as_ref().and_then(HostDataSourceQuery::latest) else {
        return false;
    };
    let valid = current(s, p);
    let p = s.host_monitoring.as_mut().unwrap();
    p.query = None;
    match result {
        Ok(result) if valid && ResourceTarget::Host(result.host) == p.target => {
            p.sources = Some(result.sources)
        }
        Ok(_) => p.error = Some(
            "Connection changed while reading the catalog. Reopen monitoring for the current host."
                .into(),
        ),
        Err(e) => p.error = Some(e),
    }
    true
}
fn selected(p: &Panel) -> Option<&HostDataSource> {
    p.sources
        .as_ref()?
        .iter()
        .find(|v| Some(&v.name) == p.selected.as_ref())
}
fn review(s: &mut State, action: HostRecordingAction) {
    let Some(p) = &mut s.host_monitoring else {
        return;
    };
    let Some(source) = selected(p).cloned() else {
        return;
    };
    if action == HostRecordingAction::StartRecording && source.enabled {
        return;
    }
    p.review = Some(HostOperation::DataSource {
        expected: source,
        action,
    });
    p.acknowledged = false;
    p.error = None;
}
fn can_apply(s: &State, p: &Panel) -> bool {
    s.can_mutate()
        && current(s, p)
        && s.resource_operations.len() < 32
        && p.review.as_ref().is_some_and(|op| {
            (!matches!(
                op,
                HostOperation::DataSource {
                    action: HostRecordingAction::ForgetHistory,
                    ..
                }
            ) || p.acknowledged)
                && s.inventory
                    .as_ref()
                    .is_some_and(|i| op.availability(i, p.target).is_available())
                && !session_permissions_view::assess(s, &op.permission_requirements())
                    .decision()
                    .is_denied()
        })
}
fn submit(s: &mut State) {
    let result = (|| {
        let p = s
            .host_monitoring
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)?;
        if !can_apply(s, p) {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Host {
                    target: p.observed.ok_or(xenadmin_core::Error::Stale)?,
                    operation: p.review.clone().ok_or(xenadmin_core::Error::Stale)?,
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            let pool = s.host_monitoring.as_ref().unwrap().pool;
            s.host_monitoring = None;
            activity::open_history(s, pool);
        }
        Err(e) => {
            if let Some(p) = &mut s.host_monitoring {
                p.error = Some(e.to_string());
            }
        }
    }
}
fn source_details(source: &HostDataSource, k: &Kit) -> El {
    col()
        .gap(6.0)
        .child(heading(&source.name, k).wrap_anywhere())
        .child(caption(&source.description, k).wrap_anywhere())
        .child(
            caption(
                format!(
                    "Units: {} · range: {} to {}",
                    source.units, source.minimum, source.maximum
                ),
                k,
            )
            .wrap_anywhere(),
        )
        .child(
            caption(
                format!(
                    "Recording at catalog refresh: {} · standard source: {}",
                    if source.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    if source.standard { "yes" } else { "no" }
                ),
                k,
            )
            .wrap(),
        )
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.host_monitoring else {
        return el();
    };
    let mut body = col().gap(12.0);
    let mut footer = row().gap(8.0).flex_wrap().justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.host_monitoring = None)
            .build(k)
            .key("monitoring-close"),
    );
    let permission = p
        .review
        .as_ref()
        .map(|op| session_permissions_view::assess(s, &op.permission_requirements()));
    if let Some(HostOperation::DataSource { expected, action }) = &p.review {
        body = body
            .child(heading(action.label(), k))
            .child(source_details(expected, k));
        if *action == HostRecordingAction::ForgetHistory {
            body=body.child(caption("This deletes this source's recorded history from the host's RRD archives. It is not a pause and cannot be undone here. Standard/default sources may start recording again immediately; the host can also enable other sources automatically.",k).wrap());
            body = body.child(
                Checkbox::new(p.acknowledged)
                    .label("I understand that this source's recorded history will be deleted")
                    .on_toggle(|s: &mut State| {
                        if let Some(p) = &mut s.host_monitoring {
                            p.acknowledged = !p.acknowledged;
                        }
                    })
                    .build(k)
                    .key("monitoring-confirm-history-loss"),
            );
        } else {
            body=body.child(caption("Begin collecting this data source on the host. Earlier missing samples are not recovered. Changing the displayed graph alone does not enable host recording.",k).wrap());
        }
        body=body.child(caption("The exact host, source metadata and observed recording state are checked again before the change. A lost reply remains unknown and is never automatically repeated.",k).wrap());
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| {
                        if let Some(p) = &mut s.host_monitoring {
                            p.review = None;
                            p.acknowledged = false;
                        }
                    })
                    .build(k)
                    .key("monitoring-back"),
            )
            .child(
                Button::new(action.label())
                    .variant(if *action == HostRecordingAction::ForgetHistory {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Neutral
                    })
                    .disabled(!can_apply(s, p))
                    .on_press(submit)
                    .build(k)
                    .key("monitoring-submit"),
            );
    } else {
        body=body.child(caption("Host recording controls affect the history collected by XAPI. Your graph selections are separate.",k).wrap())
            .child(TextInput::new(&p.search).placeholder("Find a data source").on_edit(|s: &mut State| { let p=s.host_monitoring.as_mut().unwrap();p.page=0;&mut p.search }).build(k).key("monitoring-search"));
        if p.query.is_some() {
            body = body.child(caption("Reading host data sources…", k));
        }
        if let Some(sources) = &p.sources {
            let needle = p.search.text.to_lowercase();
            let visible: Vec<_> = sources
                .iter()
                .filter(|v| {
                    v.name.to_lowercase().contains(&needle)
                        || v.description.to_lowercase().contains(&needle)
                })
                .collect();
            let pages = visible.len().div_ceil(12).max(1);
            let page = p.page.min(pages - 1);
            body = body.child(caption(
                format!("{} sources · page {} of {}", visible.len(), page + 1, pages),
                k,
            ));
            for source in visible.iter().skip(page * 12).take(12) {
                let name = source.name.clone();
                body = body.child(
                    Button::new(format!(
                        "{} · {}{}",
                        source.name,
                        if source.enabled {
                            "Recording"
                        } else {
                            "Not recording"
                        },
                        if p.selected.as_ref() == Some(&source.name) {
                            " · selected"
                        } else {
                            ""
                        }
                    ))
                    .on_press(move |s: &mut State| {
                        if let Some(p) = &mut s.host_monitoring {
                            p.selected = Some(name.clone());
                        }
                    })
                    .build(k)
                    .key(format!("monitoring-source-{}", source.name)),
                );
            }
            if pages > 1 {
                body = body.child(
                    row()
                        .gap(8.0)
                        .flex_wrap()
                        .child(
                            Button::new("Previous sources")
                                .disabled(page == 0)
                                .on_press(|s: &mut State| {
                                    if let Some(p) = &mut s.host_monitoring {
                                        p.page = p.page.saturating_sub(1);
                                    }
                                })
                                .build(k),
                        )
                        .child(
                            Button::new("Next sources")
                                .disabled(page + 1 >= pages)
                                .on_press(|s: &mut State| {
                                    if let Some(p) = &mut s.host_monitoring {
                                        p.page += 1;
                                    }
                                })
                                .build(k),
                        ),
                );
            }
            if let Some(source) = selected(p) {
                body = body.child(source_details(source, k)).child(
                    row()
                        .gap(8.0)
                        .flex_wrap()
                        .child(
                            Button::new("Review start recording")
                                .disabled(source.enabled)
                                .on_press(|s: &mut State| {
                                    review(s, HostRecordingAction::StartRecording)
                                })
                                .build(k)
                                .key("monitoring-review-start"),
                        )
                        .child(
                            Button::new("Review history deletion")
                                .on_press(|s: &mut State| {
                                    review(s, HostRecordingAction::ForgetHistory)
                                })
                                .build(k)
                                .key("monitoring-review-forget"),
                        ),
                );
            }
        }
        body=body.child(caption("Stock XCP-ng 8.3 does not ship Nagios NRPE integration. This page manages XAPI data-source recording; it does not install monitoring agents or change NRPE settings.",k).wrap());
        footer = footer.child(
            Button::new("Refresh catalog")
                .disabled(!s.demo && !current(s, p))
                .on_press(refresh)
                .build(k)
                .key("monitoring-refresh"),
        );
    }
    if s.connection.as_ref().is_some_and(|c| c.is_read_only()) {
        body = body.child(
            caption(
                "Read-only connection: recording changes and history deletion are unavailable.",
                k,
            )
            .wrap(),
        );
    }
    if let Some(op) = &p.review
        && let Some(i) = &s.inventory
    {
        let availability = op.availability(i, p.target);
        if !availability.is_available() {
            body = body.child(caption(availability.explanation(), k).wrap());
        }
    }
    if !s.demo && !current(s, p) {
        body = body.child(
            caption(
                "The connection changed. Close and reopen monitoring before applying a change.",
                k,
            )
            .wrap()
            .role("alert"),
        );
    }
    if s.demo {
        body =
            body.child(caption("Synthetic preview · no host changes can be submitted", k).wrap());
    }
    if let Some(e) = &p.error {
        body = body.child(caption(e, k).color(k.c.danger.icon).wrap().role("alert"));
    }
    let pool = p.pool;
    footer = footer.child(
        Button::new("History")
            .variant(ButtonVariant::Ghost)
            .on_press(move |s: &mut State| {
                s.host_monitoring = None;
                activity::open_history(s, pool);
            })
            .build(k)
            .key("monitoring-history"),
    );
    let width = (s.window_width - 32.0).clamp(280.0, 740.0);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Host monitoring recording")
        .modal(false, |s: &mut State| s.host_monitoring = None)
        .child(
            col()
                .w(width)
                .h((s.window_height - 40.0).clamp(280.0, 840.0))
                .padding(18.0)
                .gap(12.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(
                    col()
                        .gap(6.0)
                        .child(heading("Host monitoring recording", k).key("monitoring-heading"))
                        .child(
                            caption(
                                format!("{} · {}\nPool {}", p.name, p.target.uuid(), p.pool),
                                k,
                            )
                            .wrap_anywhere(),
                        ),
                )
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(session_permissions_view::footer(
                    footer,
                    permission.as_ref(),
                    width - 36.0,
                    k,
                )),
        )
}
pub(super) fn scene(s: &mut State) {
    host_admin::scene(s);
    let id = *s.inventory.as_ref().unwrap().hosts.keys().next().unwrap();
    open(s, ResourceTarget::Host(id));
    let p = s.host_monitoring.as_mut().unwrap();
    p.sources = Some(vec![HostDataSource {
        name: "cpu7".into(),
        description: "Processor 7 utilization".into(),
        units: "fraction".into(),
        standard: true,
        enabled: true,
        minimum: "0".into(),
        maximum: "1".into(),
    }]);
    p.selected = Some("cpu7".into());
    review(s, HostRecordingAction::ForgetHistory);
}

pub(super) fn catalog_scene(s: &mut State) {
    scene(s);
    let p = s.host_monitoring.as_mut().unwrap();
    p.review = None;
    let mut second = p.sources.as_ref().unwrap()[0].clone();
    second.name = "memory_free_kib".into();
    second.description = "Host free memory".into();
    second.units = "KiB".into();
    second.maximum = "Infinity".into();
    second.standard = false;
    second.enabled = false;
    p.selected = Some(second.name.clone());
    p.sources.as_mut().unwrap().push(second);
}

#[cfg(test)]
#[path = "host_monitoring_view_tests.rs"]
mod tests;
