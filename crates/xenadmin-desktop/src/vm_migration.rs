use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{DiskPlacement, NetworkId, NicPlacement, ObservedVm, SrId, VmMigration};

pub(super) struct Editor {
    target: Option<ObservedVm>,
    pool: PoolId,
    source: Vm,
    initial: Arc<Inventory>,
    destination: Option<HostId>,
    disks: Vec<DiskPlacement>,
    networks: Vec<NicPlacement>,
    transfer: Option<NetworkId>,
    stage: u8,
    review: Option<VmMigration>,
    error: Option<String>,
}
fn close(s: &mut State) {
    s.vm_migration = None;
}
fn current(s: &State, f: &Editor) -> bool {
    s.inventory.as_ref().is_some_and(|i| {
        i.pool_id == f.pool
            && i.vms
                .get(&f.source.id)
                .is_some_and(|v| !v.is_template && !v.is_control_domain)
    }) && (s.demo
        || f.target.is_some_and(|o| {
            s.snapshot
                .as_ref()
                .and_then(|p| p.observe_vm(f.source.id).ok())
                == Some(o)
        }))
}
pub(super) fn open(s: &mut State, id: VmId) {
    if !s.demo && !s.can_mutate() {
        return;
    }
    let Some(i) = &s.inventory else { return };
    let Some(vm) = i.vms.get(&id).filter(|v| {
        !v.is_control_domain
            && !v.is_template
            && matches!(v.power_state, PowerState::Running | PowerState::Halted)
    }) else {
        return;
    };
    let target = s.snapshot.as_ref().and_then(|p| p.observe_vm(id).ok());
    if !s.demo && target.is_none() {
        return;
    }
    let diskids = i
        .details
        .vbds
        .values()
        .filter(|v| v.vm == Some(id) && v.kind == "Disk")
        .filter_map(|v| v.vdi)
        .collect::<BTreeSet<_>>();
    let disks = diskids
        .into_iter()
        .filter_map(|id| {
            i.details.vdis.get(&id).and_then(|d| {
                d.sr.map(|sr| DiskPlacement {
                    vdi: id,
                    expected_sr: sr,
                    destination_sr: sr,
                })
            })
        })
        .collect();
    let networks = i
        .details
        .vifs
        .values()
        .filter(|n| n.vm == Some(id))
        .filter_map(|n| {
            n.network.map(|network| NicPlacement {
                vif: n.id,
                expected_network: network,
                destination_network: network,
            })
        })
        .collect();
    s.vm_migration = Some(Box::new(Editor {
        target,
        pool: i.pool_id,
        source: vm.clone(),
        initial: i.clone(),
        destination: None,
        disks,
        networks,
        transfer: None,
        stage: 0,
        review: None,
        error: None,
    }));
    s.show_details = false;
}
pub(super) fn button(s: &State, id: VmId, k: &Kit) -> El {
    let Some(vm) = s
        .inventory
        .as_ref()
        .and_then(|i| i.vms.get(&id))
        .filter(|v| !v.is_template && !v.is_control_domain)
    else {
        return el();
    };
    let supported_state = matches!(vm.power_state, PowerState::Running | PowerState::Halted);
    let control = Button::new("Migrate / move VM")
        .variant(ButtonVariant::Outline)
        .disabled(!supported_state || (!s.demo && !s.can_mutate()))
        .on_press(move |s: &mut State| open(s, id))
        .build(k)
        .key("open-vm-migration");
    if supported_state {
        control
    } else {
        col().gap(6.0).child(control).child(caption("Migration is available for running or halted VMs. Resume or shut down this guest first.",k).wrap())
    }
}

fn operation(f: &Editor) -> Result<VmMigration, String> {
    let o = VmMigration {
        destination: f.destination.ok_or("Select a destination host.")?,
        expected_host: f.source.host,
        expected_affinity: f
            .initial
            .details
            .vms
            .get(&f.source.id)
            .and_then(|v| v.affinity),
        live: f.source.power_state == PowerState::Running,
        disks: f.disks.clone(),
        networks: f.networks.clone(),
        transfer_network: f.transfer,
    };
    o.validate().map_err(|e| e.to_string())?;
    Ok(o)
}
fn next(s: &mut State) {
    let Some(f) = s.vm_migration.as_mut() else {
        return;
    };
    if f.stage == 0 {
        if f.destination.is_some() {
            f.stage = 1;
            f.error = None;
        } else {
            f.error = Some("Select a destination host.".into())
        }
    } else {
        match operation(f) {
            Ok(o) => {
                f.review = Some(o);
                f.stage = 2;
                f.error = None;
            }
            Err(e) => f.error = Some(e),
        }
    }
}
fn apply(s: &mut State) {
    let Some(f) = s.vm_migration.as_ref() else {
        return;
    };
    let Some(target) = f.target else { return };
    let Some(o) = &f.review else { return };
    if !s.can_mutate()
        || !current(s, f)
        || !s
            .inventory
            .as_ref()
            .is_some_and(|i| o.availability(i, f.source.id).is_available())
    {
        return;
    }
    s.submit_command(xenadmin_core::VmCommand::Migrate {
        vm: target,
        operation: o.clone(),
    });
    if s.error.is_none() {
        close(s);
    }
}
fn host_label(i: &Inventory, id: HostId) -> String {
    i.hosts
        .get(&id)
        .map_or_else(|| id.to_string(), |h| format!("{} ({id})", h.name))
}
fn sr_label(i: &Inventory, id: SrId) -> String {
    i.storage
        .iter()
        .find(|d| d.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |d| format!("{} ({id})", d.name))
}
fn net_label(i: &Inventory, id: NetworkId) -> String {
    i.networks
        .iter()
        .find(|d| d.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |d| format!("{} ({id})", d.name))
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(f) = s.vm_migration.as_ref() else {
        return el();
    };
    let is_current = current(s, f);
    let mut body = col()
        .w((s.window_width - 36.0).clamp(270.0, 760.0))
        .max_h((s.window_height - 50.0).clamp(260.0, 920.0))
        .padding(22.0)
        .gap(14.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(heading(
            if f.source.power_state == PowerState::Running {
                "Migrate running VM"
            } else {
                "Move halted VM"
            },
            k,
        ))
        .child(caption(format!("{}\nVM UUID: {}", f.source.name, f.source.id), k).wrap_anywhere());
    if !is_current {
        body=body.child(caption("The source connection changed. Close this review and reopen it from current inventory.",k).wrap());
    }
    if let Some(error) = &f.error {
        body = body.child(caption(error, k).wrap());
    }
    match f.stage {
        0 => {
            body=body.child(heading("1 of 3 · Destination host",k)).child(caption("Running VMs migrate live. Halted VMs remain halted and the selected host becomes their home host. Host compatibility is checked again before dispatch.",k).wrap());
            for host in f.initial.hosts.values() {
                let id = host.id;
                body = body.child(
                    Button::new(format!(
                        "{}{}",
                        if f.destination == Some(id) {
                            "✓ "
                        } else {
                            ""
                        },
                        host.name
                    ))
                    .variant(ButtonVariant::Outline)
                    .on_press(move |s: &mut State| {
                        if let Some(f) = s.vm_migration.as_mut() {
                            f.destination = Some(id);
                        }
                    })
                    .build(k)
                    .key(format!("migration-host-{id}")),
                );
            }
        }
        1 => {
            body=body.child(heading("2 of 3 · Storage and networks",k)).child(caption("Keep current placement for a shared-storage host move. To move only disks, choose the current host and change their storage repositories. CD media stays in its current repository and must be accessible to the destination.",k).wrap());
            for (index, disk) in f.disks.iter().enumerate() {
                let name = f
                    .initial
                    .details
                    .vdis
                    .get(&disk.vdi)
                    .map_or("Disk", |d| d.name.as_str());
                body = body.child(heading(format!("{name} · {}", disk.vdi), k));
                for sr in &f.initial.storage {
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
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            if let Some(f) = s.vm_migration.as_mut() {
                                f.disks[index].destination_sr = id;
                            }
                        })
                        .build(k)
                        .key(format!("migration-disk-{index}-{id}")),
                    );
                }
            }
            if f.disks.is_empty() {
                body=body.child(caption("No data disks in this captured view. Missing device inventory prevents applying a migration.",k).wrap());
            }
            for (index, nic) in f.networks.iter().enumerate() {
                body = body.child(heading(format!("Network interface {}", nic.vif), k));
                for network in &f.initial.networks {
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
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            if let Some(f) = s.vm_migration.as_mut() {
                                f.networks[index].destination_network = id;
                            }
                        })
                        .build(k)
                        .key(format!("migration-nic-{index}-{id}")),
                    );
                }
            }
            body=body.child(heading("Transfer network",k)).child(caption("Required for disk/network relocation and halted moves. Choose a network with a configured IP on the destination host. A host-only live move uses XAPI's ordinary migration transport.",k).wrap());
            for network in &f.initial.networks {
                let id = NetworkId::from(network.uuid);
                body = body.child(
                    Button::new(format!(
                        "{}{}",
                        if f.transfer == Some(id) { "✓ " } else { "" },
                        network.name
                    ))
                    .variant(ButtonVariant::Outline)
                    .on_press(move |s: &mut State| {
                        if let Some(f) = s.vm_migration.as_mut() {
                            f.transfer = Some(id);
                        }
                    })
                    .build(k)
                    .key(format!("migration-transfer-{id}")),
                );
            }
            body=body.child(caption("Storage-transport moves with snapshot chains, shared disks or CBT are not supported yet. Cross-pool migration is separate pending work.",k).wrap());
        }
        _ => {
            body = body.child(heading("3 of 3 · Review migration", k));
            if let Some(o) = &f.review {
                body = body.child(
                    caption(
                        format!(
                            "Destination: {}\nMode: {}",
                            host_label(&f.initial, o.destination),
                            if o.live {
                                "Live migration; preserve running state"
                            } else {
                                "Remain halted; change home host after verified move"
                            }
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                for d in &o.disks {
                    body = body.child(
                        caption(
                            format!(
                                "Disk {}\n{} → {}",
                                d.vdi,
                                sr_label(&f.initial, d.expected_sr),
                                sr_label(&f.initial, d.destination_sr)
                            ),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                }
                for n in &o.networks {
                    body = body.child(
                        caption(
                            format!(
                                "NIC {}\n{} → {}",
                                n.vif,
                                net_label(&f.initial, n.expected_network),
                                net_label(&f.initial, n.destination_network)
                            ),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                }
                if o.uses_storage_transport()
                    && let Some(n) = o.transfer_network
                {
                    body = body.child(
                        caption(format!("Transfer network: {}", net_label(&f.initial, n)), k)
                            .wrap_anywhere(),
                    );
                }
                body=body.child(caption("VM, attachment and interface UUIDs are preserved. Moving disks may replace their UUIDs; verified new disks are recorded in operation history. A failed or interrupted move can leave partial placement. Migration moves the selected disks. Leftover resources are retained for review, and interrupted migrations are never automatically repeated.",k).wrap());
                if let Some(i) = &s.inventory {
                    let status = o.availability(i, f.source.id);
                    if !status.is_available() {
                        body = body.child(caption(status.explanation(), k).wrap());
                    }
                }
            }
        }
    }
    let mut buttons = row().w_full().gap(8.0).child(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(close)
            .build(k),
    );
    if f.stage > 0 {
        buttons = buttons.child(
            Button::new("Back")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| {
                    if let Some(f) = s.vm_migration.as_mut() {
                        f.stage -= 1;
                        f.review = None;
                    }
                })
                .build(k)
                .key("migration-back"),
        );
    }
    if f.stage == 2 {
        buttons = buttons.child(
            Button::new("Apply migration")
                .variant(ButtonVariant::Outline)
                .disabled(
                    !s.can_mutate()
                        || !is_current
                        || !f.review.as_ref().is_some_and(|o| {
                            s.inventory
                                .as_ref()
                                .is_some_and(|i| o.availability(i, f.source.id).is_available())
                        }),
                )
                .on_press(apply)
                .build(k)
                .key("migration-apply"),
        );
    } else {
        buttons = buttons.child(
            Button::new(if f.stage == 0 {
                "Next"
            } else {
                "Review migration"
            })
            .variant(ButtonVariant::Outline)
            .disabled(!is_current || (!s.demo && !s.can_mutate()))
            .on_press(next)
            .build(k)
            .key("migration-next"),
        );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("VM migration")
        .modal(false, close)
        .key("migration-dialog")
        .child(body.child(buttons))
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let id = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let hosts = i.hosts.keys().copied().collect::<Vec<_>>();
    i.snapshots = Default::default();
    i.vms.get_mut(&id).unwrap().power_state = PowerState::Running;
    i.vms.get_mut(&id).unwrap().host = Some(hosts[0]);
    i.details.vms.get_mut(&id).unwrap().allowed_operations =
        Some(vec!["pool_migrate".into(), "migrate_send".into()]);
    for class in ["pbd", "pif"] {
        i.details
            .classes
            .insert(class.into(), xenadmin_core::ClassAvailability::Available);
    }
    for (index, host) in hosts.iter().enumerate() {
        for (offset, sr) in i.storage.iter().enumerate() {
            let p = xenadmin_core::PbdId::from(uuid::Uuid::from_u128(
                900 + index as u128 * 100 + offset as u128,
            ));
            i.details.pbds.insert(
                p,
                xenadmin_core::StorageAttachment {
                    location: None,
                    requires_credentials: false,
                    multipathed: None,
                    paths: Vec::new(),
                    id: p,
                    host: Some(*host),
                    sr: Some(SrId::from(sr.uuid)),
                    currently_attached: Some(true),
                },
            );
        }
        for (offset, n) in i.networks.iter().enumerate() {
            let p = xenadmin_core::PifId::from(uuid::Uuid::from_u128(
                9000 + index as u128 * 100 + offset as u128,
            ));
            i.details.pifs.insert(
                p,
                xenadmin_core::PhysicalInterface {
                    id: p,
                    host: Some(*host),
                    network: Some(NetworkId::from(n.uuid)),
                    device: "eth0".into(),
                    mac: "02:00:00:00:00:01".into(),
                    mtu: Some(1500),
                    vlan: Some(-1),
                    management: Some(true),
                    physical: Some(true),
                    currently_attached: Some(true),
                    ip_configuration_mode: "Static".into(),
                    ip: format!("192.0.2.{}", index + 10),
                    netmask: "255.255.255.0".into(),
                    gateway: String::new(),
                    dns: String::new(),
                    ipv6: vec![],
                    ipv6_configuration_mode: "None".into(),
                    ipv6_gateway: String::new(),
                },
            );
        }
    }
    s.table.select(id.to_string());
    open(s, id);
    let f = s.vm_migration.as_mut().unwrap();
    f.destination = Some(hosts[1]);
    f.transfer = f.initial.networks.first().map(|n| NetworkId::from(n.uuid));
    f.stage = 1;
    next(s);
}
#[cfg(test)]
#[path = "vm_migration_tests.rs"]
mod tests;
