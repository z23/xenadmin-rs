use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    HaRestartPriority, ObservedResource, OperationId, PoolOperation, ResourceCommand,
    ResourceTarget, SrId,
};

pub(super) struct Editor {
    pool: PoolId,
    name: String,
    observed: Option<ObservedResource>,
    inventory: std::sync::Arc<Inventory>,
    heartbeat: BTreeSet<SrId>,
    failures: TextEdit,
    search: TextEdit,
    vm: Option<VmId>,
    host: Option<HostId>,
    review: Option<PoolOperation>,
    confirmation: TextEdit,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let Some(inventory) = s.inventory.as_ref() else {
        return;
    };
    let pool = inventory.pool_id;
    s.pool_editor = Some(Editor {
        pool,
        name: inventory.pool_name.clone(),
        observed: s
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(pool)).ok()),
        inventory: inventory.clone(),
        heartbeat: BTreeSet::new(),
        failures: TextEdit::new(
            inventory
                .details
                .pool
                .as_ref()
                .and_then(|p| p.administration.ha_failures_to_tolerate)
                .unwrap_or(0)
                .to_string(),
        ),
        search: TextEdit::new(""),
        vm: None,
        host: None,
        review: None,
        confirmation: TextEdit::new(""),
        error: None,
    });
    s.resource_editor = None;
}
fn stage(s: &mut State, op: PoolOperation) {
    if let Some(editor) = &mut s.pool_editor {
        editor.review = Some(op);
        editor.error = None;
        editor.confirmation = TextEdit::new("");
    }
}
fn confirmation(op: &PoolOperation, pool: PoolId) -> Option<String> {
    match op {
        PoolOperation::EjectHost { host, .. } => Some(host.to_string()),
        PoolOperation::DesignateCoordinator { .. }
        | PoolOperation::DisableHa
        | PoolOperation::RotateSecret
        | PoolOperation::EnableTlsVerification => Some(pool.to_string()),
        _ => None,
    }
}
fn current(s: &State, editor: &Editor) -> bool {
    s.snapshot.as_ref().and_then(|snapshot| {
        snapshot
            .observe_resource(ResourceTarget::Pool(editor.pool))
            .ok()
    }) == editor.observed
        && editor.observed.is_some()
}
fn apply(s: &mut State) {
    let result = (|| {
        let editor = s.pool_editor.as_ref().ok_or("Pool view is closed")?;
        let op = editor.review.as_ref().ok_or("Review a change first")?;
        if s.demo || !s.can_mutate() || !current(s, editor) {
            return Err("The connection changed; reopen pool administration".into());
        }
        if confirmation(op, editor.pool)
            .is_some_and(|required| editor.confirmation.text != required)
        {
            return Err("Enter the exact requested UUID".into());
        }
        let handle = s
            .connection
            .as_ref()
            .ok_or("Connect before applying changes")?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Pool {
                    target: editor
                        .observed
                        .ok_or("Current pool observation is unavailable")?,
                    operation: op.clone(),
                },
            )
            .map_err(|e| e.to_string())?;
        if let Some(waker) = &s.waker {
            handle.notify_on_change(waker.clone());
        }
        Ok::<_, String>(handle)
    })();
    match result {
        Ok(handle) => {
            s.resource_operations.push(handle);
            s.pool_editor = None;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        }
        Err(error) => {
            if let Some(editor) = &mut s.pool_editor {
                editor.error = Some(error);
            }
        }
    }
}
fn review_failures(s: &mut State) {
    let result = (|| {
        let editor = s.pool_editor.as_ref().unwrap();
        let expected = editor
            .inventory
            .details
            .pool
            .as_ref()
            .and_then(|p| p.administration.ha_failures_to_tolerate)
            .ok_or("HA failure tolerance is unavailable")?;
        let failures: u64 = editor
            .failures
            .text
            .parse()
            .map_err(|_| "Enter a whole number of host failures")?;
        if failures >= editor.inventory.hosts.len() as u64 || failures == expected {
            return Err("Choose a different value below the number of pool hosts");
        }
        Ok(PoolOperation::SetHaFailuresToTolerate { expected, failures })
    })();
    match result {
        Ok(op) => stage(s, op),
        Err(error) => s.pool_editor.as_mut().unwrap().error = Some(error.into()),
    }
}
fn priority(s: &mut State, value: HaRestartPriority) {
    let op = s.pool_editor.as_ref().and_then(|editor| {
        let vm = editor.vm?;
        let expected = HaRestartPriority::parse(
            editor
                .inventory
                .details
                .vms
                .get(&vm)?
                .ha_restart_priority
                .as_deref()?,
        )?;
        (expected != value).then_some(PoolOperation::SetVmRestartPriority {
            vm,
            expected,
            priority: value,
        })
    });
    if let Some(op) = op {
        stage(s, op);
    }
}
fn describe(editor: &Editor, op: &PoolOperation) -> Vec<String> {
    let host = |id: HostId| {
        editor
            .inventory
            .hosts
            .get(&id)
            .map(|h| format!("{} ({id})", h.name))
            .unwrap_or_else(|| id.to_string())
    };
    match op {
        PoolOperation::Join { .. } => vec!["Join another independently connected pool".into()],
        PoolOperation::Security(op) => vec![op.label().into()],
        PoolOperation::SetMigrationCompression {expected,enabled} => vec![format!("Migration compression: {expected} → {enabled}"), "Applies to subsequent migrations; compression trades host CPU for lower network usage.".into()],
        PoolOperation::SetHttpsOnly {expected,enabled} => vec![format!("HTTPS-only access: {expected} → {enabled}"),"Changes the pool hosts’ HTTP firewall policy. Existing tools that rely on plain HTTP may stop working.".into()],
        PoolOperation::DesignateCoordinator {expected,host:target} => vec![format!("Current coordinator: {}",host(*expected)),format!("New coordinator: {}",host(*target)),"Performs a planned handover. This connection may briefly disconnect or move to the new coordinator. HA must already be disabled. Does not evacuate or shut down guests.".into()],
        PoolOperation::SetHaFailuresToTolerate {expected,failures} => vec![format!("Host failures to tolerate: {expected} → {failures}"),"The server must confirm a failover plan exists before submission. A lower value reduces the promised failure tolerance.".into()],
        PoolOperation::EnableHa {heartbeat_srs,expected_failures} => {
            let mut lines=vec![format!("Enable HA with failure tolerance {expected_failures}."),"HA can fence and reboot a host that loses heartbeat connectivity. Requires prepared storage and management networking; three or more hosts are recommended.".into()];
            for id in heartbeat_srs {lines.push(format!("Heartbeat storage: {} ({id})",editor.inventory.storage.iter().find(|sr|sr.uuid==id.uuid()).map(|sr|sr.name.as_str()).unwrap_or("Unknown")));}
            lines
        },
        PoolOperation::DisableHa => vec!["Disables automatic HA recovery for the entire pool. Protected VMs will no longer be restarted automatically after host failure.".into()],
        PoolOperation::SetVmRestartPriority {vm,expected,priority} => vec![format!("VM: {} ({vm})",editor.inventory.vms.get(vm).map(|vm|vm.name.as_str()).unwrap_or("Unknown")),format!("HA priority: {} → {}",expected.label(),priority.label()),"Restart protection requires agile storage/network placement. This does not start or stop the VM.".into()],
        PoolOperation::EjectHost {host:target,..} => vec![format!("Host leaving the pool: {}",host(*target)),"The host will reboot and its pool configuration will be reset. Its local disks must be empty of guest data. Disable and evacuate this host first; HA must already be disabled. No guest shutdown or disk deletion is performed by this workflow.".into()],
        PoolOperation::EnableTlsVerification => vec!["Enables certificate verification between all pool members. This is separate from this application's connection trust setting. Every member must support the feature and trust its peers; upgraded pools need all members on XCP-ng 8.3.".into()],
        PoolOperation::RotateSecret => vec!["Rotates the shared authentication secret used between pool members. All hosts must be reachable and HA must already be disabled. No secret is shown or stored by this client.".into()],
    }
}
fn action(k: &Kit, label: impl Into<String>, op: PoolOperation, disabled: bool, key: &str) -> El {
    Button::new(label.into())
        .disabled(disabled)
        .on_press(move |s: &mut State| stage(s, op.clone()))
        .build(k)
        .key(key)
}
fn known(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Enabled",
        Some(false) => "Disabled",
        None => "Unknown / unavailable",
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(editor) = &s.pool_editor else {
        return el();
    };
    let admin = editor
        .inventory
        .details
        .pool
        .as_ref()
        .map(|p| p.administration.clone())
        .unwrap_or_default();
    let mut body = col()
        .gap(12.0)
        .child(heading("Pool policies and availability", k))
        .child(caption(format!("{}\n{}", editor.name, editor.pool), k).wrap_anywhere());
    if let Some(op) = &editor.review {
        body = body.child(heading(op.label(), k));
        for line in describe(editor, op) {
            body = body.child(caption(line, k).wrap());
        }
        if let Some(required) = confirmation(op, editor.pool) {
            body = body
                .child(caption(format!("Type this UUID to confirm: {required}"), k).wrap_anywhere())
                .child(
                    TextInput::new(&editor.confirmation)
                        .label("Confirmation UUID")
                        .on_edit(|s: &mut State| &mut s.pool_editor.as_mut().unwrap().confirmation)
                        .build(k)
                        .key("pool-confirmation"),
                );
        }
        body = body.child(caption("Fresh host checks run again before submission. Interrupted operations stay in history for recheck; no mutation is replayed automatically.",k).wrap());
    } else {
        body = body
            .child(
                Button::new("Join another pool…")
                    .disabled(s.demo || !s.can_mutate() || editor.inventory.hosts.len() != 1)
                    .on_press(profiles_view::join_pool)
                    .build(k)
                    .key("pool-join-open"),
            )
            .child(
                Button::new("Authentication, access and certificates…")
                    .on_press(pool_security_view::open)
                    .build(k)
                    .key("pool-security-open"),
            )
            .child(heading("Pool policies", k));
        if let Some(expected) = admin.migration_compression {
            body = body.child(action(
                k,
                format!("Migration compression: {} · change", known(Some(expected))),
                PoolOperation::SetMigrationCompression {
                    expected,
                    enabled: !expected,
                },
                false,
                "pool-compression",
            ));
        }
        if let Some(expected) = admin.https_only {
            body = body.child(action(
                k,
                format!("HTTPS only: {} · change", known(Some(expected))),
                PoolOperation::SetHttpsOnly {
                    expected,
                    enabled: !expected,
                },
                false,
                "pool-https",
            ));
        }
        body = body.child(
            caption(
                format!(
                    "Pool certificate verification: {}",
                    known(admin.tls_verification_enabled)
                ),
                k,
            )
            .wrap(),
        );
        if admin.tls_verification_enabled == Some(false) {
            body = body.child(action(
                k,
                "Enable pool TLS verification…",
                PoolOperation::EnableTlsVerification,
                false,
                "pool-tls",
            ));
        }
        if admin.secret_rotation_pending == Some(false) {
            body = body.child(action(
                k,
                "Rotate pool authentication secret…",
                PoolOperation::RotateSecret,
                admin.ha_enabled != Some(false),
                "pool-secret",
            ));
        }
        body = body.child(heading("High availability", k)).child(
            caption(
                format!(
                    "HA: {}\nConfigured failures: {} · Current plan: {}\nOvercommitted: {}",
                    known(admin.ha_enabled),
                    admin
                        .ha_failures_to_tolerate
                        .map(|v| v.to_string())
                        .unwrap_or("Unknown".into()),
                    admin
                        .ha_plan_exists_for
                        .map(|v| v.to_string())
                        .unwrap_or("Unknown".into()),
                    match admin.ha_overcommitted {
                        Some(true) => "Yes",
                        Some(false) => "No",
                        None => "Unknown",
                    }
                ),
                k,
            )
            .wrap(),
        );
        if admin.ha_enabled == Some(true) {
            if admin.heartbeat_srs_known {
                for sr in &admin.heartbeat_srs {
                    body = body.child(
                        caption(
                            format!(
                                "Heartbeat: {} ({sr})",
                                editor
                                    .inventory
                                    .storage
                                    .iter()
                                    .find(|r| r.uuid == sr.uuid())
                                    .map(|r| r.name.as_str())
                                    .unwrap_or("Unknown storage")
                            ),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                }
            } else {
                body = body.child(caption("Heartbeat storage identity is unavailable", k));
            }
            body = body.child(action(
                k,
                "Disable high availability…",
                PoolOperation::DisableHa,
                false,
                "pool-ha-disable",
            ));
        }
        if admin.ha_failures_to_tolerate.is_some() {
            body = body
                .child(caption("Host failures to tolerate", k))
                .child(
                    TextInput::new(&editor.failures)
                        .label("Host failures to tolerate")
                        .on_edit(|s: &mut State| &mut s.pool_editor.as_mut().unwrap().failures)
                        .build(k),
                )
                .child(
                    Button::new("Review failure tolerance")
                        .on_press(review_failures)
                        .build(k),
                );
        }
        if admin.ha_enabled == Some(false) {
            body=body.child(caption("Select heartbeat storage. Every host must already be attached; the server validates eligibility.",k).wrap());
            for sr in editor
                .inventory
                .storage
                .iter()
                .filter(|sr| {
                    editor
                        .inventory
                        .details
                        .storage
                        .get(&SrId::from(sr.uuid))
                        .is_some_and(|details| {
                            details.shared == Some(true) && details.content_type == "user"
                        })
                })
                .take(100)
            {
                let id = SrId::from(sr.uuid);
                body = body.child(
                    Button::new(format!(
                        "{} {}",
                        if editor.heartbeat.contains(&id) {
                            "✓"
                        } else {
                            "○"
                        },
                        sr.name
                    ))
                    .on_press(move |s: &mut State| {
                        let chosen = &mut s.pool_editor.as_mut().unwrap().heartbeat;
                        if !chosen.remove(&id) && chosen.len() < 8 {
                            chosen.insert(id);
                        }
                    })
                    .build(k),
                );
            }
            if let Some(expected_failures) = admin.ha_failures_to_tolerate {
                body = body.child(action(
                    k,
                    "Review enabling HA",
                    PoolOperation::EnableHa {
                        heartbeat_srs: editor.heartbeat.iter().copied().collect(),
                        expected_failures,
                    },
                    editor.heartbeat.is_empty(),
                    "pool-ha-enable",
                ));
            }
        }
        body=body.child(heading("Pool membership",k)).child(caption("Choose a member for planned coordinator handover or ejection. Both require HA to be disabled first.",k).wrap());
        for host in editor.inventory.hosts.values().take(256) {
            let id = host.id;
            body = body.child(
                Button::new(format!(
                    "{} {}{}",
                    if editor.host == Some(id) {
                        "✓"
                    } else {
                        "○"
                    },
                    host.name,
                    if admin.coordinator == Some(id) {
                        " · coordinator"
                    } else {
                        ""
                    }
                ))
                .on_press(move |s: &mut State| s.pool_editor.as_mut().unwrap().host = Some(id))
                .build(k),
            );
        }
        if let Some(host) = editor.host
            && let Some(row) = editor.inventory.hosts.get(&host)
        {
            if let Some(expected) = admin.coordinator {
                body = body.child(action(
                    k,
                    "Hand over coordinator…",
                    PoolOperation::DesignateCoordinator { expected, host },
                    host == expected || admin.ha_enabled != Some(false),
                    "pool-handover",
                ));
            }
            body = body.child(action(
                k,
                "Eject selected member…",
                PoolOperation::EjectHost {
                    host,
                    expected_name: row.name.clone(),
                },
                Some(host) == admin.coordinator || admin.ha_enabled != Some(false),
                "pool-eject",
            ));
        }
        body = body.child(heading("VM restart priorities", k)).child(
            TextInput::new(&editor.search)
                .label("Filter virtual machines")
                .on_edit(|s: &mut State| &mut s.pool_editor.as_mut().unwrap().search)
                .build(k),
        );
        let needle = editor.search.text.to_lowercase();
        let choices: Vec<_> = editor
            .inventory
            .guests()
            .filter(|vm| {
                vm.name.to_lowercase().contains(&needle) || vm.id.to_string().contains(&needle)
            })
            .collect();
        for vm in choices.iter().take(30) {
            let id = vm.id;
            body = body.child(
                Button::new(format!(
                    "{} {}",
                    if editor.vm == Some(id) { "✓" } else { "○" },
                    vm.name
                ))
                .on_press(move |s: &mut State| s.pool_editor.as_mut().unwrap().vm = Some(id))
                .build(k),
            );
        }
        if choices.len() > 30 {
            body = body.child(
                caption(
                    "Showing 30 matches. Filter by name or UUID to find another VM.",
                    k,
                )
                .wrap(),
            );
        }
        if let Some(vm) = editor.vm {
            let current = editor
                .inventory
                .details
                .vms
                .get(&vm)
                .and_then(|vm| vm.ha_restart_priority.as_deref())
                .and_then(HaRestartPriority::parse);
            body = body.child(caption(
                format!(
                    "Current priority: {}",
                    current
                        .map(HaRestartPriority::label)
                        .unwrap_or("Unknown / unsupported")
                ),
                k,
            ));
            for value in [
                HaRestartPriority::Restart,
                HaRestartPriority::BestEffort,
                HaRestartPriority::Disabled,
            ] {
                body = body.child(
                    Button::new(value.label())
                        .disabled(current.is_none() || current == Some(value))
                        .on_press(move |s: &mut State| priority(s, value))
                        .build(k),
                );
            }
        }
    }
    if let Some(error) = &editor.error {
        body = body.child(caption(error, k).wrap());
    }
    if s.demo {
        body = body.child(caption("Synthetic preview · submission disabled", k));
    }
    let mut footer = row().gap(8.0).justify_end().child(
        Button::new("Close")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.pool_editor = None)
            .build(k),
    );
    if let Some(op) = &editor.review {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| s.pool_editor.as_mut().unwrap().review = None)
                    .build(k),
            )
            .child(
                Button::new("Confirm change")
                    .variant(if confirmation(op, editor.pool).is_some() {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Outline
                    })
                    .disabled(
                        s.demo
                            || !s.can_mutate()
                            || !current(s, editor)
                            || confirmation(op, editor.pool)
                                .is_some_and(|required| editor.confirmation.text != required),
                    )
                    .on_press(apply)
                    .build(k)
                    .key("pool-confirm"),
            );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Pool administration")
        .modal(false, |s: &mut State| s.pool_editor = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 680.0))
                .h((s.window_height - 56.0).clamp(280.0, 820.0))
                .padding(20.0)
                .gap(16.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}
pub(super) fn scene(s: &mut State, review: bool) {
    crate::demo(s);
    open(s);
    if review {
        stage(s, PoolOperation::DisableHa);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_captures_target_and_compact_footer_never_submits_demo() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s, true);
            let pool = s.pool_editor.as_ref().unwrap().pool;
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
            let (x, y) = runtime.find("pool-confirm").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
            assert_eq!(runtime.state.pool_editor.as_ref().unwrap().pool, pool);
            assert!(
                describe(
                    runtime.state.pool_editor.as_ref().unwrap(),
                    &PoolOperation::DisableHa
                )
                .iter()
                .any(|s| s.contains("no longer"))
            );
        }
    }
    #[test]
    fn ejection_requires_host_uuid_and_priority_unknown_is_not_disabled() {
        let mut s = crate::init();
        scene(&mut s, false);
        let editor = s.pool_editor.as_ref().unwrap();
        let host = *editor.inventory.hosts.keys().next().unwrap();
        let op = PoolOperation::EjectHost {
            host,
            expected_name: "Fixture".into(),
        };
        assert_eq!(confirmation(&op, editor.pool), Some(host.to_string()));
        assert_eq!(HaRestartPriority::parse("future-policy"), None);
        assert_eq!(
            HaRestartPriority::parse(""),
            Some(HaRestartPriority::Disabled)
        );
        stage(&mut s, op);
        s.page = Page::Guests;
        s.rebuild();
        assert!(
            matches!(s.pool_editor.as_ref().unwrap().review,Some(PoolOperation::EjectHost{host:id,..}) if id==host)
        );
    }
}
