use super::*;

pub(super) fn operation(form: &Editor) -> VmLifecycleOperation {
    VmLifecycleOperation::Delete {
        expected_template: form.source.is_template,
        disks: form.disks.iter().copied().collect(),
        snapshots: form
            .snapshots
            .iter()
            .map(|id| SnapshotDeletionSelection {
                snapshot: *id,
                disks: form
                    .initial
                    .snapshot_deletion_disks(*id)
                    .into_iter()
                    .filter(|disk| form.snapshot_disks.contains(disk))
                    .collect(),
            })
            .collect(),
    }
}
fn consumers(inventory: &Inventory, disk: VdiId) -> Vec<String> {
    inventory
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vdi == Some(disk))
        .map(|vbd| {
            let owner = vbd
                .vm
                .and_then(|id| {
                    inventory
                        .vms
                        .get(&id)
                        .map(|vm| format!("{} ({id})", vm.name))
                })
                .or_else(|| {
                    vbd.snapshot.and_then(|id| {
                        inventory
                            .snapshots
                            .get(&id)
                            .map(|snapshot| format!("Snapshot {} ({id})", snapshot.name))
                    })
                })
                .unwrap_or_else(|| "Owner not reported".into());
            format!(
                "Consumer: {owner}; attachment {}; slot {}",
                vbd.id, vbd.userdevice
            )
        })
        .collect()
}
fn candidate_disks(form: &Editor) -> Vec<VdiId> {
    form.initial.vm_deletion_disks(form.source.id)
}
pub(super) fn editor(form: &Editor, k: &Kit) -> El {
    let mut body=col().w_full().gap(10.0).child(caption("The default removes only the VM/template record. Select individual owned disks or snapshots below to delete them too. Disk deletion is irreversible; unselected data is retained.",k).wrap())
        .child(heading("VM data disks",k));
    let candidates = candidate_disks(form);
    let all_disks: Vec<_> = form
        .initial
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vm == Some(form.source.id) && vbd.kind == "Disk")
        .filter_map(|vbd| vbd.vdi)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if all_disks.is_empty() {
        body = body.child(caption("No data disks are reported for this VM.", k));
    }
    for id in all_disks {
        if candidates.contains(&id) {
            body = body.child(
                Checkbox::new(form.disks.contains(&id))
                    .label(format!("Delete data: {}", disk_label(&form.initial, id)))
                    .on_toggle(move |s: &mut State| {
                        if let Some(form) = editor_mut(s)
                            && !form.disks.remove(&id)
                        {
                            form.disks.insert(id);
                        }
                    })
                    .build(k)
                    .key(format!("vm-delete-disk-{id}")),
            );
        } else {
            body = body.child(
                caption(
                    format!(
                        "Retain {} — ownership or exclusive use is not established.",
                        disk_label(&form.initial, id)
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
        }
        for consumer in consumers(&form.initial, id) {
            body = body.child(caption(consumer, k).wrap_anywhere());
        }
    }
    body = body.child(heading("Snapshots", k));
    let snapshots: Vec<_> = form.initial.snapshots.for_vm(form.source.id).collect();
    if snapshots.is_empty() {
        body = body.child(caption("No snapshots are reported for this VM.", k));
    }
    for snapshot in snapshots {
        let id = snapshot.id;
        let eligible = snapshot.power_state == PowerState::Halted
            && snapshot
                .allowed_actions
                .contains(&xenadmin_core::SnapshotAction::Delete);
        body = body.child(
            Checkbox::new(form.snapshots.contains(&id))
                .label(format!("Delete snapshot: {} ({id})", snapshot.name))
                .disabled(!eligible)
                .on_toggle(move |s: &mut State| {
                    if let Some(form) = editor_mut(s)
                        && !form.snapshots.remove(&id)
                    {
                        form.snapshots.insert(id);
                    }
                })
                .build(k)
                .key(format!("vm-delete-snapshot-{id}")),
        );
        if !eligible {
            body=body.child(caption("Retained. Memory checkpoints require the separate snapshot deletion flow; other snapshots must advertise deletion permission.",k).wrap());
        }
        if form.snapshots.contains(&id) {
            for disk in form.initial.snapshot_deletion_disks(id) {
                body = body.child(
                    Checkbox::new(form.snapshot_disks.contains(&disk))
                        .label(format!(
                            "Delete this snapshot's owned data: {}",
                            disk_label(&form.initial, disk)
                        ))
                        .on_toggle(move |s: &mut State| {
                            if let Some(form) = editor_mut(s)
                                && !form.snapshot_disks.remove(&disk)
                            {
                                form.snapshot_disks.insert(disk);
                            }
                        })
                        .build(k)
                        .key(format!("vm-delete-snapshot-disk-{disk}")),
                );
            }
        }
    }
    body.child(caption("Shared or unowned data is never included automatically. If another operator changes ownership or consumers, submission is refused. Completed deletions cannot be rolled back after a later failure.",k).wrap())
}
pub(super) fn review(form: &Editor) -> Vec<String> {
    let mut lines = vec![format!(
        "Permanently remove {} “{}” ({}) from the pool inventory.",
        if form.source.is_template {
            "template"
        } else {
            "VM"
        },
        form.source.name,
        form.source.id
    )];
    lines.push("Its disk and network attachment records are also removed. Retained disks remain in storage.".into());
    if form.disks.is_empty() && form.snapshots.is_empty() {
        lines.push(
            "Record-only deletion: no disk data or snapshot records were selected for deletion."
                .into(),
        );
    }
    for id in &form.disks {
        lines.push(format!(
            "DELETE DISK DATA: {}",
            disk_label(&form.initial, *id)
        ));
        lines.extend(consumers(&form.initial, *id));
    }
    for snapshot in form.initial.snapshots.for_vm(form.source.id) {
        if form.snapshots.contains(&snapshot.id) {
            lines.push(format!(
                "DELETE SNAPSHOT: {} ({})",
                snapshot.name, snapshot.id
            ));
            for disk in form
                .initial
                .snapshot_deletion_disks(snapshot.id)
                .into_iter()
                .filter(|id| form.snapshot_disks.contains(id))
            {
                lines.push(format!(
                    "DELETE SNAPSHOT DATA: {}",
                    disk_label(&form.initial, disk)
                ));
                lines.extend(consumers(&form.initial, disk));
            }
        } else {
            lines.push(format!(
                "Retain snapshot: {} ({})",
                snapshot.name, snapshot.id
            ));
        }
    }
    for id in form
        .initial
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vm == Some(form.source.id) && vbd.kind == "Disk")
        .filter_map(|vbd| vbd.vdi)
        .collect::<BTreeSet<_>>()
    {
        if !form.disks.contains(&id) {
            lines.push(format!(
                "Retain disk data: {}",
                disk_label(&form.initial, id)
            ));
        }
    }
    lines.push("Only the exact reviewed UUIDs are eligible for deletion. New snapshots or disks are not silently added. Selected data deletion cannot be undone. Partial or uncertain results remain in VM changes after the source disappears.".into());
    lines
}
