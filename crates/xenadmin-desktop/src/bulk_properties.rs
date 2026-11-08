use super::*;
use xenadmin_core::{
    BatchVmProperties, ObservedResource, ResourceCommand, ResourceTarget, VmPropertiesBatch,
};
#[path = "bulk_properties_model.rs"]
mod model;
use model::Property;

pub(super) struct Editor {
    pool: PoolId,
    pool_name: String,
    observed: Option<ObservedResource>,
    targets: Vec<Vm>,
    hosts: Vec<(xenadmin_core::HostId, String)>,
    property: Property,
    choosing: bool,
    values: [TextEdit; 4],
    host: Option<xenadmin_core::HostId>,
    toggle: bool,
    review: Option<Review>,
    error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Review {
    batch: VmPropertiesBatch,
    rows: Vec<ReviewRow>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReviewRow {
    id: VmId,
    name: String,
    state: PowerState,
    changed: bool,
    summary: String,
}
fn edit(s: &mut State) -> &mut Editor {
    s.bulk_properties.as_mut().expect("property batch editor")
}
fn selected(s: &State) -> Result<Vec<Vm>, String> {
    if !matches!(s.page, Page::Guests | Page::Templates) {
        return Err("Select VM or custom-template rows first".into());
    }
    let inventory = s.inventory.as_ref().ok_or("Inventory is unavailable")?;
    let rows = s
        .rows
        .iter()
        .filter(|r| s.table.is_selected(&r.id))
        .collect::<Vec<_>>();
    if !(1..=32).contains(&rows.len()) {
        return Err("Select between 1 and 32 visible targets".into());
    }
    rows.into_iter()
        .map(|r| {
            let vm =
                r.vm.and_then(|id| inventory.vms.get(&id))
                    .ok_or("A selected row is no longer present in VM inventory")?;
            if vm.is_control_domain || vm.is_template != (s.page == Page::Templates) {
                return Err("A selected row changed type; reopen the selection".into());
            }
            Ok(vm.clone())
        })
        .collect()
}
pub(super) fn open(s: &mut State) {
    let targets = match selected(s) {
        Ok(v) => v,
        Err(e) => {
            s.error = Some(e);
            return;
        }
    };
    let Some(i) = &s.inventory else {
        return;
    };
    let mut hosts = i
        .hosts
        .values()
        .map(|h| (h.id, h.name.clone()))
        .collect::<Vec<_>>();
    hosts.sort_by_cached_key(|(_, name)| name.to_lowercase());
    s.bulk_properties = Some(Editor {
        pool: i.pool_id,
        pool_name: i.pool_name.clone(),
        observed: s.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .observe_resource(ResourceTarget::Pool(i.pool_id))
                .ok()
        }),
        targets,
        hosts,
        property: Property::Description,
        choosing: false,
        values: std::array::from_fn(|_| TextEdit::new("")),
        host: None,
        toggle: false,
        review: None,
        error: None,
    });
    choose(s, Property::Description);
}
fn choose(s: &mut State, property: Property) {
    let e = edit(s);
    if e.review.is_some() {
        return;
    }
    e.property = property;
    e.choosing = false;
    e.error = None;
    e.values = std::array::from_fn(|_| TextEdit::new(""));
    // A shared value is entered explicitly; do not silently use the first target's value.
    let defaults: &[&str] = match property {
        Property::Cpus => &["2", "2"],
        Property::LiveCpus => &["2"],
        Property::Memory => &["512 MiB", "1 GiB", "2 GiB", "2 GiB"],
        Property::DynamicMemory => &["1 GiB", "2 GiB"],
        Property::Topology => &["1"],
        Property::Weight => &["256"],
        Property::Shadow => &["1"],
        Property::BootOrder => &["cdn"],
        _ => &[],
    };
    for (field, value) in e.values.iter_mut().zip(defaults) {
        *field = TextEdit::new(*value);
    }
}
fn current(s: &State, e: &Editor) -> bool {
    if s.inventory.as_ref().is_none_or(|i| i.pool_id != e.pool) {
        return false;
    }
    s.demo
        || e.observed.is_some()
            && e.observed
                == s.snapshot
                    .as_ref()
                    .and_then(|snap| snap.observe_resource(ResourceTarget::Pool(e.pool)).ok())
}
fn proposed(s: &State, e: &Editor) -> Result<Review, String> {
    if !current(s, e) {
        return Err("Connection changed or inventory is stale. Reopen this selection.".into());
    }
    let i = s.inventory.as_ref().ok_or("Inventory is unavailable")?;
    let mut rows = Vec::new();
    let mut vms = Vec::new();
    let mut errors = Vec::new();
    for captured in &e.targets {
        let result = (|| {
            let now = i
                .vms
                .get(&captured.id)
                .ok_or("Target disappeared from inventory")?;
            if now.power_state != captured.power_state
                || now.is_template != captured.is_template
                || now.name != captured.name
                || now.is_control_domain
            {
                return Err(
                    "Target name, type or power state changed; reopen this selection".into(),
                );
            }
            let (patch, summary) = model::proposal(i, now, e)?;
            rows.push(ReviewRow {
                id: now.id,
                name: now.name.clone(),
                state: now.power_state.clone(),
                changed: patch.is_some(),
                summary,
            });
            if let Some(patch) = patch {
                vms.push(BatchVmProperties {
                    vm: now.id,
                    expected_state: now.power_state.clone(),
                    patch,
                });
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            errors.push(format!("{} ({}) · {error}", captured.name, captured.id));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(Review {
        batch: VmPropertiesBatch { vms },
        rows,
    })
}
fn review(s: &mut State) {
    let Some(e) = &s.bulk_properties else {
        return;
    };
    match proposed(s, e) {
        Ok(r) if r.batch.vms.is_empty() => {
            edit(s).error = Some(
                "All selected targets already have the desired value. No changes to submit.".into(),
            )
        }
        Ok(r) => {
            edit(s).review = Some(r);
            edit(s).error = None;
        }
        Err(error) => edit(s).error = Some(error),
    }
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s
            .bulk_properties
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)?;
        let reviewed = e.review.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if s.demo
            || !s.can_mutate()
            || s.resource_operations.len() >= 64
            || proposed(s, e).as_ref() != Ok(reviewed)
        {
            return Err(xenadmin_core::Error::Stale);
        }
        let assessment =
            session_permissions_view::assess(s, &reviewed.batch.permission_requirements());
        if assessment.decision().is_denied() {
            return Err(xenadmin_core::Error::UnavailableAction);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                xenadmin_core::OperationId::new(),
                ResourceCommand::PropertiesBatch {
                    pool: e.observed.ok_or(xenadmin_core::Error::Stale)?,
                    selection: reviewed.batch.clone(),
                },
            )
    })();
    match result {
        Ok(operation) => {
            if let Some(waker) = &s.waker {
                operation.notify_on_change(waker.clone());
            }
            s.resource_operations.push(operation);
            s.bulk_properties = None;
        }
        Err(error) => edit(s).error = Some(error.to_string()),
    }
}
pub(super) fn selection_button(s: &State, k: &Kit) -> El {
    Button::new("Edit selection…")
        .disabled(selected(s).is_err())
        .on_press(open)
        .build(k)
        .key("batch-properties-open")
}
pub(super) fn selection_bar(s: &State, k: &Kit) -> El {
    row()
        .gap(10.0)
        .center_y()
        .child(
            caption("Shift/Ctrl-click to select up to 32 custom templates", k)
                .wrap()
                .grow(1.0),
        )
        .child(selection_button(s, k))
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(e) = &s.bulk_properties else {
        return el();
    };
    let width = (s.window_width - 24.0).clamp(280.0, 800.0);
    let proposed = proposed(s, e);
    let changed = proposed.as_ref().ok().map_or(0, |r| r.batch.vms.len());
    let assessment = e
        .review
        .as_ref()
        .map(|r| session_permissions_view::assess(s, &r.batch.permission_requirements()));
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(10.0)
        .padding(20.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(heading(
            if e.review.is_some() {
                "Review bulk property changes"
            } else {
                "Properties for selected targets"
            },
            k,
        ))
        .child(
            caption(
                format!(
                    "{} · {}\n{} targets captured",
                    e.pool_name,
                    e.pool,
                    e.targets.len()
                ),
                k,
            )
            .wrap_anywhere(),
        );
    if e.review.is_none() {
        body = body.child(
            Button::new(format!("Property: {}", e.property.label()))
                .on_press(|s: &mut State| edit(s).choosing = !edit(s).choosing)
                .build(k)
                .key("batch-property-picker"),
        );
        if e.choosing {
            for p in Property::ALL {
                body = body.child(
                    Button::new(p.label())
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| choose(s, p))
                        .build(k)
                        .key(format!("batch-property-{p:?}")),
                );
            }
        } else {
            for (n, label) in e.property.labels().iter().enumerate() {
                body = body.child(
                    col().gap(4.0).child(caption(*label, k)).child(
                        TextInput::new(&e.values[n])
                            .label(*label)
                            .on_edit(move |s: &mut State| &mut edit(s).values[n])
                            .on_change(|s: &mut State, _| edit(s).error = None)
                            .build(k)
                            .key(format!("batch-property-value-{n}")),
                    ),
                );
            }
            match e.property {
                Property::HomeHost => {
                    body = body.child(
                        Button::new(if e.host.is_none() {
                            "✓ No preference"
                        } else {
                            "No preference"
                        })
                        .on_press(|s: &mut State| edit(s).host = None)
                        .build(k),
                    );
                    for (id, name) in &e.hosts {
                        let id = *id;
                        body = body.child(
                            Button::new(format!(
                                "{}{} ({id})",
                                if e.host == Some(id) { "✓ " } else { "" },
                                name
                            ))
                            .on_press(move |s: &mut State| edit(s).host = Some(id))
                            .build(k),
                        );
                    }
                }
                Property::Autostart | Property::SecureBoot | Property::Firmware => {
                    for value in [false, true] {
                        let label = if e.property == Property::Firmware {
                            if value { "UEFI" } else { "BIOS" }
                        } else if value {
                            "Enabled"
                        } else {
                            "Disabled"
                        };
                        body = body.child(
                            Button::new(format!(
                                "{}{label}",
                                if e.toggle == value { "✓ " } else { "" }
                            ))
                            .on_press(move |s: &mut State| edit(s).toggle = value)
                            .build(k)
                            .key(format!("batch-property-toggle-{value}")),
                        );
                    }
                }
                _ => {}
            }
        }
    }
    body = body.child(caption(e.property.note(), k).wrap());
    let display = e.review.as_ref().or_else(|| proposed.as_ref().ok());
    if let Some(r) = display {
        body = body.child(heading(
            format!(
                "{} changes · {} already desired",
                r.batch.vms.len(),
                r.rows.len() - r.batch.vms.len()
            ),
            k,
        ));
        for row in &r.rows {
            body = body.child(
                col().gap(4.0).child(heading(&row.name, k)).child(
                    caption(
                        format!(
                            "{} · {}\n{}{}",
                            row.id,
                            row.state,
                            if row.changed {
                                ""
                            } else {
                                "Already desired — excluded from changes\n"
                            },
                            row.summary
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                ),
            );
        }
    } else {
        for target in &e.targets {
            body = body.child(
                caption(
                    format!("{} · {} · {}", target.name, target.power_state, target.id),
                    k,
                )
                .wrap_anywhere(),
            );
        }
    }
    if let Err(error) = &proposed {
        body = body.child(caption(error, k).color(k.c.danger.icon).wrap_anywhere());
    }
    if let Some(r) = &e.review {
        if proposed.as_ref() != Ok(r) {
            body=body.child(caption("Reviewed values changed. Nothing will be sent; go Back or reopen the selection.",k).color(k.c.danger.icon).wrap());
        }
        body=body.child(caption("Targets and setters run serially. A failure or uncertain response stops the remainder; completed changes remain. There is no automatic rollback. Operation history retains per-target steps, and recheck never sends pending changes.",k).wrap());
    }
    if !s.demo && !s.can_mutate() {
        body=body.child(caption("This connection is read-only, disconnected or stale. Review is available; submission is disabled.",k).wrap());
    }
    if let Some(error) = &e.error {
        body = body.child(caption(error, k).color(k.c.danger.icon).wrap());
    }
    let controls = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .on_press(|s: &mut State| s.bulk_properties = None)
            .build(k)
            .key("batch-properties-close"),
    );
    let controls = if let Some(review) = &e.review {
        controls
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| {
                        edit(s).review = None;
                        edit(s).error = None;
                    })
                    .build(k)
                    .key("batch-properties-back"),
            )
            .child(
                Button::new("Apply changes")
                    .disabled(
                        s.demo
                            || !s.can_mutate()
                            || s.resource_operations.len() >= 64
                            || !current(s, e)
                            || proposed.as_ref() != Ok(review)
                            || assessment
                                .as_ref()
                                .is_some_and(|a| a.decision().is_denied()),
                    )
                    .on_press(submit)
                    .build(k)
                    .key("batch-properties-submit"),
            )
    } else {
        controls.child(
            Button::new("Review changes")
                .disabled(proposed.is_err() || changed == 0)
                .on_press(review)
                .build(k)
                .key("batch-properties-review"),
        )
    };
    let footer = session_permissions_view::footer(controls, assessment.as_ref(), width - 40.0, k)
        .padding(20.0);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.55))
        .role("dialog")
        .label("Bulk property editor")
        .modal(false, |s: &mut State| s.bulk_properties = None)
        .child(
            col()
                .w(width)
                .h((s.window_height - 32.0).clamp(260.0, 760.0))
                .fill(k.c.bg_base)
                .radius(12.0)
                .child(body)
                .child(footer),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let ids = s
        .rows
        .iter()
        .take(3)
        .map(|r| r.id.clone())
        .collect::<Vec<_>>();
    if let Some(id) = ids.first() {
        s.table.select(id.clone());
    }
    open(s);
    let targets = ids
        .iter()
        .filter_map(|id| {
            s.inventory
                .as_ref()?
                .vms
                .values()
                .find(|v| v.id.to_string() == *id)
                .cloned()
        })
        .collect();
    if let Some(e) = s.bulk_properties.as_mut() {
        e.targets = targets;
    }
    if let Some(e) = s.bulk_properties.as_mut() {
        e.values[0] = TextEdit::new("Reviewed shared description");
    }
    review(s);
}
#[cfg(test)]
#[path = "bulk_properties_tests.rs"]
mod tests;
