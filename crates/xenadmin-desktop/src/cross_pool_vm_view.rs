use super::*;

fn label_sr(i: &Inventory, id: SrId) -> String {
    i.storage
        .iter()
        .find(|r| r.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |r| format!("{} ({id})", r.name))
}
fn label_net(i: &Inventory, id: NetworkId) -> String {
    i.networks
        .iter()
        .find(|r| r.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |r| format!("{} ({id})", r.name))
}
fn selector(
    label: String,
    key: String,
    selected: bool,
    k: &Kit,
    change: impl Fn(&mut State) + 'static,
) -> El {
    Button::new(format!("{}{label}", if selected { "✓ " } else { "" }))
        .variant(ButtonVariant::Outline)
        .on_press(change)
        .build(k)
        .key(key)
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let mut body = col()
        .w((s.window_width - 36.0).clamp(270.0, 780.0))
        .max_h((s.window_height - 50.0).clamp(260.0, 920.0))
        .padding(22.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(heading("Cross-pool VM transfers", k));
    if let Some(e) = &s.cross_pool.editor {
        body = body.child(
            caption(
                format!(
                    "{}\nSource pool: {}\nVM: {}",
                    e.vm.name, e.source.inventory.pool_name, e.vm.id
                ),
                k,
            )
            .wrap_anywhere(),
        );
        if !current(s, e) {
            body = body.child(
                caption(
                    "A captured connection changed. Close and reopen this review.",
                    k,
                )
                .wrap(),
            );
        }
        if let Some(error) = &e.error {
            body = body.child(caption(error, k).wrap());
        }
        match e.stage {
            0 => {
                body = body.child(heading("1 of 3 · Destination and mode", k));
                if e.vm.power_state == PowerState::Halted {
                    for mode in [CrossPoolMode::MoveHalted, CrossPoolMode::CopyHalted] {
                        body = body.child(selector(
                            mode.label().into(),
                            format!("cross-mode-{mode:?}"),
                            e.mode == mode,
                            k,
                            move |s| {
                                if let Some(e) = &mut s.cross_pool.editor {
                                    e.mode = mode;
                                }
                            },
                        ));
                    }
                } else {
                    body=body.child(caption("Live move preserves the running state. Cross-pool copy requires a halted guest.",k).wrap());
                }
                if e.peers.is_empty() {
                    body=body.child(caption("Connect another pool with write access using Connections, then reopen this transfer.",k).wrap());
                }
                for (index, p) in e.peers.iter().enumerate() {
                    body = body.child(selector(
                        format!("{} · {}", p.inventory.pool_name, p.inventory.pool_id),
                        format!("cross-pool-{index}"),
                        e.destination == Some(index),
                        k,
                        move |s| choose(s, index),
                    ));
                }
                if let Some(d) = destination(e) {
                    for host in d.inventory.hosts.values().filter(|h| h.enabled) {
                        let id = host.id;
                        body = body.child(selector(
                            format!("Host: {} ({id})", host.name),
                            format!("cross-host-{id}"),
                            e.host == Some(id),
                            k,
                            move |s| {
                                if let Some(e) = &mut s.cross_pool.editor {
                                    e.host = Some(id);
                                    e.transfer = None;
                                }
                            },
                        ));
                    }
                }
            }
            1 => {
                body = body.child(heading("2 of 3 · Storage, NICs and transfer address", k));
                if let Some(d) = destination(e) {
                    let i = &d.inventory;
                    for (index, disk) in e.disks.iter().enumerate() {
                        body = body.child(heading(format!("Disk {}", disk.vdi), k));
                        for sr in i.storage.iter().filter(|sr| {
                            i.details
                                .storage
                                .get(&SrId::from(sr.uuid))
                                .is_some_and(|r| r.content_type == "user")
                                && i.details.pbds.values().any(|p| {
                                    p.sr == Some(SrId::from(sr.uuid))
                                        && p.host == e.host
                                        && p.currently_attached == Some(true)
                                })
                        }) {
                            let id = SrId::from(sr.uuid);
                            body = body.child(selector(
                                sr.name.clone(),
                                format!("cross-disk-{index}-{id}"),
                                disk.destination_sr == id,
                                k,
                                move |s| {
                                    if let Some(e) = &mut s.cross_pool.editor {
                                        e.disks[index].destination_sr = id;
                                    }
                                },
                            ));
                        }
                    }
                    for (index, nic) in e.networks.iter().enumerate() {
                        body = body.child(heading(format!("NIC {}", nic.vif), k));
                        for n in &i.networks {
                            let id = NetworkId::from(n.uuid);
                            body = body.child(selector(
                                n.name.clone(),
                                format!("cross-nic-{index}-{id}"),
                                nic.destination_network == id,
                                k,
                                move |s| {
                                    if let Some(e) = &mut s.cross_pool.editor {
                                        e.networks[index].destination_network = id;
                                    }
                                },
                            ));
                        }
                    }
                    body = body.child(heading("Destination transfer address", k));
                    for p in i
                        .details
                        .pifs
                        .values()
                        .filter(|p| p.host == e.host && p.currently_attached == Some(true))
                    {
                        if let Some(network) = p.network {
                            for address in std::iter::once(p.ip.clone())
                                .chain(
                                    p.ipv6
                                        .iter()
                                        .map(|a| a.split('/').next().unwrap_or_default().into()),
                                )
                                .filter(|a| {
                                    a.parse::<std::net::IpAddr>()
                                        .is_ok_and(|a| !a.is_unspecified() && !a.is_loopback())
                                })
                            {
                                let value = (network, address.clone());
                                let selected = e.transfer.as_ref() == Some(&value);
                                body = body.child(selector(
                                    format!("{} · {address}", label_net(i, network)),
                                    format!("cross-address-{network}-{address}"),
                                    selected,
                                    k,
                                    move |s| {
                                        if let Some(e) = &mut s.cross_pool.editor {
                                            e.transfer = Some(value.clone());
                                        }
                                    },
                                ));
                            }
                        }
                    }
                }
                body=body.child(caption("This workflow accepts ordinary guests with exclusive owned data disks and empty CD drives. Snapshot chains, shared disks, mounted media, HA protection and vApps require separate preparation.",k).wrap());
            }
            _ => {
                body = body.child(heading("3 of 3 · Review both pools", k));
                if let Some(o) = &e.review {
                    let d = destination(e).unwrap();
                    body = body.child(
                        caption(
                            format!(
                                "{}\nDestination: {} ({})\nHost: {}\nHost-to-host address: {}",
                                o.mode.label(),
                                d.inventory.pool_name,
                                o.destination_pool,
                                o.destination_host,
                                o.transfer_address
                            ),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                    for disk in &o.disks {
                        body = body.child(
                            caption(
                                format!(
                                    "Disk {}\n{} → {}",
                                    disk.vdi,
                                    label_sr(&e.source.inventory, disk.expected_sr),
                                    label_sr(&d.inventory, disk.destination_sr)
                                ),
                                k,
                            )
                            .wrap_anywhere(),
                        );
                    }
                    for nic in &o.networks {
                        body = body.child(
                            caption(
                                format!(
                                    "NIC {}\n{} → {}",
                                    nic.vif,
                                    label_net(&e.source.inventory, nic.expected_network),
                                    label_net(&d.inventory, nic.destination_network)
                                ),
                                k,
                            )
                            .wrap_anywhere(),
                        );
                    }
                    body=body.child(caption(if o.mode.copy(){"Copy creates new VM, disk and device identities and new MAC addresses. The source stays halted. The destination is not started."}else{"XAPI moves the VM and removes the exclusive source disks. VM/device UUIDs and MAC addresses are retained; destination disk UUIDs may change."},k).wrap());
                    body=body.child(caption("Host-to-host transport follows the hosts' migration policy and may use unencrypted HTTP or TLS without certificate verification. Desktop certificate trust does not secure this transfer. Use a trusted migration network. The application does not force TLS or silently retry with a different policy.",k).wrap().color(k.c.danger.icon));
                    body = body.child(selector(
                        "I reviewed the host-to-host transport policy".into(),
                        "cross-transport-reviewed".into(),
                        e.transport_reviewed,
                        k,
                        |s| {
                            if let Some(e) = &mut s.cross_pool.editor {
                                e.transport_reviewed = !e.transport_reviewed;
                                if let Some(o) = &mut e.review {
                                    o.host_transport_reviewed = e.transport_reviewed;
                                }
                            }
                        },
                    ));
                    body=body.child(caption("Both pools are reserved until completion or explicit acknowledgement. An interrupted transfer can leave partial source or destination resources. Recheck reads existing evidence; it never repeats a transfer. Verified destination UUIDs remain in history.",k).wrap());
                    if o.mode.live() {
                        body=body.child(caption("The source console closes when submitted. After verified completion, use Open destination console from transfer history.",k).wrap());
                    }
                    let status = availability(s, e);
                    if !status.is_available() {
                        body = body.child(caption(status.explanation(), k).wrap());
                    }
                }
            }
        }
        if e.stage > 0 {
            body = body.child(
                Button::new("Back")
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut State| {
                        if let Some(e) = &mut s.cross_pool.editor {
                            e.stage -= 1;
                            e.review = None;
                            e.transport_reviewed = false;
                        }
                    })
                    .build(k)
                    .key("cross-back"),
            );
        }
        body = body.child(if e.stage == 2 {
            Button::new("Apply reviewed transfer")
                .disabled(!s.can_mutate() || !current(s, e) || !availability(s, e).is_available())
                .on_press(apply)
                .build(k)
                .key("cross-apply")
        } else {
            Button::new("Next")
                .disabled(!current(s, e))
                .on_press(next)
                .build(k)
                .key("cross-next")
        });
    } else {
        body = history_view(s, body, k);
    }
    if let Some(notice) = &s.cross_pool.notice {
        body = body.child(caption(notice, k).wrap());
    }
    body = body.child(
        Button::new("Close")
            .variant(ButtonVariant::Outline)
            .on_press(close)
            .build(k),
    );
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Cross-pool VM transfers")
        .modal(false, close)
        .child(body)
}
fn history_view(s: &State, mut body: El, k: &Kit) -> El {
    body=body.child(caption("Reconnect both pools to recheck an unresolved transfer or batch. Missing or uncertain task receipts never trigger another transfer, and a recheck never starts a VM that a batch had not started.",k).wrap());
    // Single transfers, batches of moves, and the moves a batch has started. A move that
    // belongs to a batch is listed for what it created; it is rechecked, stopped and
    // acknowledged through its batch.
    for r in s
        .resource_rows
        .iter()
        .rev()
        .filter(|r| r.action.spans_pools())
        .take(100)
    {
        let id = r.id;
        let Some(peer) = r.peer_pool() else {
            continue;
        };
        let batch = matches!(r.action, ResourceAction::VmWorkflowBatch(_));
        body = body.child(heading(format!("{} · {:?}", r.action.label(), r.state), k));
        body = body.child(
            caption(
                match &r.action {
                    ResourceAction::CrossPoolVm(operation) => format!(
                        "Source pool: {}\nSource VM: {}\nDestination pool: {peer}\n{}{}",
                        r.pool,
                        operation.vm,
                        r.detail,
                        r.batch_parent
                            .map(|parent| format!("\nPart of batch {parent}"))
                            .unwrap_or_default()
                    ),
                    _ => format!(
                        "Source pool: {}\nDestination pool: {peer}\n{}",
                        r.pool, r.detail
                    ),
                },
                k,
            )
            .wrap_anywhere(),
        );
        for result in &r.workflow_batch {
            body = body.child(
                caption(
                    format!(
                        "VM {} · {}\n{}",
                        result.vm,
                        result
                            .state
                            .as_ref()
                            .map_or_else(|| "Not started".into(), |s| format!("{s:?}")),
                        result.detail
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
        }
        for (kind, ids) in [
            (
                "VM",
                r.transfer_created
                    .vms
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            ),
            (
                "Disk",
                r.transfer_created
                    .disks
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
            (
                "Disk attachment",
                r.transfer_created
                    .vbds
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
            (
                "NIC",
                r.transfer_created
                    .vifs
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
        ] {
            for id in ids {
                body = body.child(caption(format!("Destination {kind}: {id}"), k).wrap_anywhere());
            }
        }
        for step in &r.steps {
            body = body.child(caption(format!("{} · {:?}", step.label, step.state), k).wrap());
        }
        if !r.state.terminal() && r.batch_parent.is_none() {
            body = body.child(
                Button::new("Recheck both pools")
                    .disabled(!can_recheck(s, id) || r.state != OperationState::OutcomeUnknown)
                    .on_press(move |s: &mut State| control(s, id, OperationControl::Recheck))
                    .build(k)
                    .key(format!("cross-recheck-{id}")),
            );
            if r.can_cancel && !r.cancel_requested {
                body = body.child(
                    Button::new(if batch {
                        "Finish current VM and stop"
                    } else {
                        "Request host cancellation"
                    })
                    .disabled(
                        profiles_view::resource_control_connection(
                            s,
                            id,
                            OperationControl::RequestCancellation,
                        )
                        .is_none(),
                    )
                    .on_press(move |s: &mut State| {
                        control(s, id, OperationControl::RequestCancellation)
                    })
                    .build(k)
                    .key(format!("cross-stop-{id}")),
                );
            }
            if r.state == OperationState::OutcomeUnknown {
                body = body.child(
                    Button::new(if batch {
                        "Acknowledge unresolved batch…"
                    } else {
                        "Acknowledge unresolved transfer…"
                    })
                    .on_press(move |s: &mut State| s.cross_pool.acknowledge = Some(id))
                    .build(k),
                );
            }
        } else if r.state == OperationState::Succeeded && !batch {
            let console = matches!(&r.action,ResourceAction::CrossPoolVm(o) if o.mode.live());
            body = body.child(
                Button::new(if console {
                    "Open destination console"
                } else {
                    "Show destination VM"
                })
                .disabled(destination_target(s, r).is_none())
                .on_press(move |s: &mut State| show_destination(s, id))
                .build(k)
                .key(format!("cross-destination-{id}")),
            );
        }
        if s.cross_pool.acknowledge == Some(id) {
            body=body.child(caption("Acknowledgement releases BOTH pool reservations without proving completion or removing any resource. Inspect both pools and the host task before continuing.",k).wrap()).child(Button::new("Acknowledge and release both pools").disabled(profiles_view::resource_control_connection(s,id,OperationControl::AcknowledgeUnknown).is_none()).on_press(move|s:&mut State|control(s,id,OperationControl::AcknowledgeUnknown)).build(k));
        }
    }
    body
}
