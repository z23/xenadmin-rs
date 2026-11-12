use super::*;
use xenadmin_core::{
    BatchVmWorkflow, DiskPlacement, NetworkId, NicPlacement, ObservedResource, ResourceCommand,
    ResourceTarget, SrId, VmMigration, VmWorkflow, VmWorkflowBatch,
};

pub(super) struct Editor {
    pool: PoolId,
    name: String,
    observed: Option<ObservedResource>,
    initial: Arc<Inventory>,
    choices: Vec<Choice>,
    destination: Option<HostId>,
    transfer: Option<NetworkId>,
    expanded: Option<usize>,
    review: Option<Review>,
    error: Option<String>,
}
struct Choice {
    source: Vm,
    affinity: Option<HostId>,
    disks: Vec<DiskPlacement>,
    networks: Vec<NicPlacement>,
    refusal: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
struct Review {
    batch: VmWorkflowBatch,
    rows: Vec<Vec<String>>,
}
fn edit(s: &mut State) -> &mut Editor {
    s.bulk_move.as_mut().expect("move editor")
}
pub(super) fn selected(s: &State) -> Result<Vec<Vm>, String> {
    if s.page != Page::Guests {
        return Err("Select ordinary VM rows".into());
    }
    let inventory = s
        .inventory
        .as_ref()
        .ok_or("Current inventory is unavailable")?;
    let rows = s
        .rows
        .iter()
        .filter(|r| s.table.is_selected(&r.id))
        .collect::<Vec<_>>();
    if !(1..=32).contains(&rows.len()) {
        return Err("Select between 1 and 32 visible VMs".into());
    }
    rows.into_iter()
        .map(|r| {
            r.vm.and_then(|id| inventory.vms.get(&id))
                .cloned()
                .ok_or_else(|| "A selected VM is missing".into())
        })
        .collect()
}
fn choice(i: &Inventory, source: Vm) -> Choice {
    let mut refusal = None;
    if source.is_template
        || source.is_control_domain
        || !matches!(source.power_state, PowerState::Running | PowerState::Halted)
    {
        refusal = Some("Only ordinary running or halted VMs can move".into());
    }
    let details = i.details.vms.get(&source.id);
    if details.is_none_or(|d| !d.affinity_known) {
        refusal = Some("VM details or home-host identity are unavailable".into());
    }
    let ids = i
        .details
        .vbds
        .values()
        .filter(|v| v.vm == Some(source.id) && v.kind == "Disk")
        .map(|v| v.vdi)
        .collect::<Vec<_>>();
    let disks = ids
        .into_iter()
        .map(|id| {
            let id = id?;
            let sr = i.details.vdis.get(&id)?.sr?;
            Some(DiskPlacement {
                vdi: id,
                expected_sr: sr,
                destination_sr: sr,
            })
        })
        .collect::<Option<Vec<_>>>();
    let networks = i
        .details
        .vifs
        .values()
        .filter(|n| n.vm == Some(source.id))
        .map(|n| {
            Some(NicPlacement {
                vif: n.id,
                expected_network: n.network?,
                destination_network: n.network?,
            })
        })
        .collect::<Option<Vec<_>>>();
    if disks.is_none() || networks.is_none() {
        refusal = Some("A disk or network mapping is incomplete".into());
    }
    Choice {
        affinity: details.and_then(|d| d.affinity),
        source,
        disks: disks.unwrap_or_default(),
        networks: networks.unwrap_or_default(),
        refusal,
    }
}
pub(super) fn open(s: &mut State) {
    let selected = match selected(s) {
        Ok(rows) => rows,
        Err(e) => {
            s.error = Some(e);
            return;
        }
    };
    let i = s.inventory.as_ref().unwrap();
    s.bulk_move = Some(Editor {
        pool: i.pool_id,
        name: i.pool_name.clone(),
        observed: s
            .snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(i.pool_id)).ok()),
        initial: i.clone(),
        choices: selected.into_iter().map(|vm| choice(i, vm)).collect(),
        destination: None,
        transfer: None,
        expanded: None,
        review: None,
        error: None,
    });
}
fn current(s: &State, e: &Editor) -> bool {
    !s.demo
        && s.can_mutate()
        && e.observed.is_some()
        && s.snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}
fn build_review(s: &State, e: &Editor) -> Result<Review, String> {
    let i = s
        .inventory
        .as_ref()
        .filter(|i| i.pool_id == e.pool)
        .ok_or("The captured pool is no longer current")?;
    let destination = e
        .destination
        .ok_or("Choose a destination host for the selection")?;
    let mut rows = Vec::new();
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for c in &e.choices {
        let result: Result<_, String> = (|| {
            if let Some(error) = &c.refusal {
                return Err(error.clone());
            }
            if i.details
                .vms
                .get(&c.source.id)
                .is_none_or(|d| !d.affinity_known)
            {
                return Err("Current home-host identity is unavailable".into());
            }
            let vm = i
                .vms
                .get(&c.source.id)
                .filter(|vm| {
                    !vm.is_template
                        && !vm.is_control_domain
                        && vm.power_state == c.source.power_state
                        && vm.host == c.source.host
                })
                .ok_or("VM identity, state or resident host changed")?;
            let operation = VmMigration {
                destination,
                expected_host: c.source.host,
                expected_affinity: c.affinity,
                live: c.source.power_state == PowerState::Running,
                disks: c.disks.clone(),
                networks: c.networks.clone(),
                transfer_network: e.transfer,
            };
            let availability = operation.availability(i, vm.id);
            if !availability.is_available() {
                return Err(availability.explanation());
            }
            let mut lines = vec![
                format!("{} ({})", vm.name, vm.id),
                format!(
                    "{} → {}",
                    if operation.live {
                        "Live migration; preserve running state"
                    } else {
                        "Remain halted; set home host after verified move"
                    },
                    vm_migration::host_label(i, destination)
                ),
            ];
            for disk in &operation.disks {
                lines.push(format!(
                    "Disk {}: {} → {}",
                    disk.vdi,
                    vm_migration::sr_label(i, disk.expected_sr),
                    vm_migration::sr_label(i, disk.destination_sr)
                ));
            }
            for nic in &operation.networks {
                lines.push(format!(
                    "NIC {}: {} → {}",
                    nic.vif,
                    vm_migration::net_label(i, nic.expected_network),
                    vm_migration::net_label(i, nic.destination_network)
                ));
            }
            if operation.uses_storage_transport() {
                lines.push(format!(
                    "Transfer network: {}",
                    vm_migration::net_label(i, e.transfer.ok_or("A transfer network is required")?)
                ));
            }
            Ok((
                BatchVmWorkflow {
                    vm: vm.id,
                    expected_state: vm.power_state.clone(),
                    workflow: VmWorkflow::Move(operation),
                },
                lines,
            ))
        })();
        match result {
            Ok((item, lines)) => {
                items.push(item);
                rows.push(lines);
            }
            Err(error) => errors.push(format!("{} ({}): {error}", c.source.name, c.source.id)),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let batch = VmWorkflowBatch { items };
    let permission = session_permissions_view::assess(s, &batch.permission_requirements());
    if permission.decision().is_denied() {
        return Err(permission.explanation());
    }
    Ok(Review { batch, rows })
}
fn review(s: &mut State) {
    let Some(e) = &s.bulk_move else {
        return;
    };
    match build_review(s, e) {
        Ok(r) => {
            edit(s).review = Some(r);
            edit(s).error = None;
        }
        Err(error) => edit(s).error = Some(error),
    }
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s.bulk_move.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let r = e.review.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, e)
            || build_review(s, e).ok().as_ref() != Some(r)
            || s.resource_operations.len() >= 64
        {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                xenadmin_core::OperationId::new(),
                ResourceCommand::VmWorkflowBatch {
                    pool: e.observed.ok_or(xenadmin_core::Error::Stale)?,
                    selection: r.batch.clone(),
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            s.bulk_move = None;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        }
        Err(error) => edit(s).error = Some(error.to_string()),
    }
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    Button::new("Move selection…")
        .disabled(selected(s).is_err())
        .on_press(open)
        .build(k)
        .key("batch-move-open")
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.bulk_move.as_ref().unwrap();
    let mut body = col().w_full().gap(10.0).child(
        caption(
            format!("{} · {}\n{} captured VMs", e.name, e.pool, e.choices.len()),
            k,
        )
        .wrap_anywhere(),
    );
    if let Some(review) = &e.review {
        for (index, lines) in review.rows.iter().enumerate() {
            body = body.child(heading(format!("Target {}", index + 1), k));
            for line in lines {
                body = body.child(caption(line, k).wrap_anywhere());
            }
        }
        body=body.child(caption("Every VM is checked before the first move. VMs move serially; Stop finishes the current VM workflow and skips the rest. Failed moves can leave partial placement or replacement disks. Recovery never starts an unsent target, repeats a move or acquires another receiver token. This review covers this pool only.",k).wrap());
    } else {
        body = body.child(heading("Destination host", k));
        for host in e.initial.hosts.values() {
            let id = host.id;
            body = body.child(
                Button::new(format!(
                    "{}{}",
                    if e.destination == Some(id) {
                        "✓ "
                    } else {
                        ""
                    },
                    host.name
                ))
                .disabled(!host.enabled)
                .on_press(move |s: &mut State| edit(s).destination = Some(id))
                .build(k)
                .key(format!("batch-move-host-{id}")),
            );
        }
        body=body.child(heading("Transfer network",k)).child(caption("Required for halted moves or disk/network relocation. Ordinary live host moves keep the host's normal migration transport.",k).wrap());
        for network in &e.initial.networks {
            let id = NetworkId::from(network.uuid);
            body = body.child(
                Button::new(format!(
                    "{}{}",
                    if e.transfer == Some(id) { "✓ " } else { "" },
                    network.name
                ))
                .on_press(move |s: &mut State| edit(s).transfer = Some(id))
                .build(k)
                .key(format!("batch-move-transfer-{id}")),
            );
        }
        body = body.child(heading("Captured targets and device mappings", k));
        for (index, c) in e.choices.iter().enumerate() {
            body = body.child(
                caption(
                    format!(
                        "{} ({}) · {}",
                        c.source.name, c.source.id, c.source.power_state
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
            if let Some(error) = &c.refusal {
                body = body.child(caption(error, k).wrap().color(k.c.danger.icon));
            }
            body = body.child(
                Button::new(if e.expanded == Some(index) {
                    "Close device mappings"
                } else {
                    "Review / edit device mappings"
                })
                .on_press(move |s: &mut State| {
                    let e = edit(s);
                    e.expanded = if e.expanded == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                })
                .build(k)
                .key(format!("batch-move-mappings-{index}")),
            );
            if e.expanded != Some(index) {
                continue;
            }
            for (disk_index, disk) in c.disks.iter().enumerate() {
                body = body.child(
                    caption(
                        format!(
                            "Disk {} · current {}",
                            disk.vdi,
                            vm_migration::sr_label(&e.initial, disk.expected_sr)
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                for sr in &e.initial.storage {
                    let id = SrId::from(sr.uuid);
                    body = body.child(
                        Button::new(format!(
                            "{}{}",
                            if disk.destination_sr == id {
                                "✓ "
                            } else {
                                ""
                            },
                            sr.name
                        ))
                        .on_press(move |s: &mut State| {
                            edit(s).choices[index].disks[disk_index].destination_sr = id
                        })
                        .build(k)
                        .key(format!("batch-move-disk-{index}-{disk_index}-{id}")),
                    );
                }
            }
            for (nic_index, nic) in c.networks.iter().enumerate() {
                body = body.child(
                    caption(
                        format!(
                            "NIC {} · current {}",
                            nic.vif,
                            vm_migration::net_label(&e.initial, nic.expected_network)
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                for network in &e.initial.networks {
                    let id = NetworkId::from(network.uuid);
                    body = body.child(
                        Button::new(format!(
                            "{}{}",
                            if nic.destination_network == id {
                                "✓ "
                            } else {
                                ""
                            },
                            network.name
                        ))
                        .on_press(move |s: &mut State| {
                            edit(s).choices[index].networks[nic_index].destination_network = id
                        })
                        .build(k)
                        .key(format!("batch-move-nic-{index}-{nic_index}-{id}")),
                    );
                }
            }
            body=body.child(caption("CD media remains in its current repository. Shared disks, CBT or snapshot chains may prevent storage relocation; the full selection is refused when any target is unsupported.",k).wrap());
        }
    }
    let proposal = build_review(s, e);
    if let Some(review) = &e.review {
        body = body.child(session_permissions_view::review_notice(
            &session_permissions_view::assess(s, &review.batch.permission_requirements()),
            k,
        ));
    }
    if let Err(reason) = &proposal {
        body = body.child(
            caption(reason, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    if let Some(error) = &e.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    if !s.demo && !current(s, e) {
        body = body.child(
            caption(
                "The connection changed or inventory is stale. Reopen this review.",
                k,
            )
            .wrap(),
        );
    }
    if s.demo {
        body = body.child(caption("Synthetic preview; submission is disabled.", k).wrap());
    }
    let mut footer = row().w_full().gap(8.0).flex_wrap().child(
        Button::new("Cancel")
            .on_press(|s: &mut State| s.bulk_move = None)
            .build(k)
            .key("batch-move-close"),
    );
    if let Some(r) = &e.review {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| edit(s).review = None)
                    .build(k)
                    .key("batch-move-back"),
            )
            .child(
                Button::new("Move reviewed VMs")
                    .disabled(
                        !current(s, e)
                            || proposal.as_ref().ok() != Some(r)
                            || s.resource_operations.len() >= 64,
                    )
                    .on_press(submit)
                    .build(k)
                    .key("batch-move-submit"),
            );
    } else {
        footer = footer.child(
            Button::new("Review moves")
                .disabled(proposal.is_err())
                .on_press(review)
                .build(k)
                .key("batch-move-review"),
        );
    }
    let panel = col()
        .w((s.window_width - 24.0).clamp(280.0, 820.0))
        .h((s.window_height - 24.0).clamp(300.0, 900.0))
        .padding(16.0)
        .gap(10.0)
        .fill(k.c.bg_base)
        .radius(12.0)
        .child(heading(
            if e.review.is_some() {
                "Review selected VM moves"
            } else {
                "Move selected VMs"
            },
            k,
        ))
        .child(
            body.grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .key("batch-move-body"),
        )
        .child(footer);
    el().absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .fill(rune::Color::BLACK.alpha(0.55))
        .role("dialog")
        .label("Move selected VMs")
        .modal(false, |s: &mut State| s.bulk_move = None)
        .child(panel)
}
pub(super) fn scene(s: &mut State) {
    vm_migration::scene(s);
    s.vm_migration = None;
    let id = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let second = VmId::parse("30000000-0000-0000-0000-000000000001").unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let host = i.vms[&id].host;
    i.vms.get_mut(&second).unwrap().power_state = PowerState::Running;
    i.vms.get_mut(&second).unwrap().host = host;
    i.details.vms.get_mut(&second).unwrap().allowed_operations =
        Some(vec!["pool_migrate".into(), "migrate_send".into()]);
    for vm in [id, second] {
        i.details.vms.get_mut(&vm).unwrap().affinity_known = true;
    }
    s.rebuild();
    s.table.select(id.to_string());
    open(s);
    let e = edit(s);
    e.choices
        .push(choice(&e.initial, e.initial.vms[&second].clone()));
    e.destination = e.initial.hosts.keys().copied().find(|h| Some(*h) != host);
    e.transfer = e.initial.networks.first().map(|n| NetworkId::from(n.uuid));
    review(s);
}
#[cfg(test)]
#[path = "bulk_move_tests.rs"]
mod tests;
