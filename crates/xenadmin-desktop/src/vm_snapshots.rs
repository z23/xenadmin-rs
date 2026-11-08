use super::*;
use xenadmin_core::{
    ClassAvailability, ObservedSnapshot, ObservedVm, PowerAction, SnapshotAction, SnapshotId,
    SnapshotName, SnapshotWorkflow, VdiId, VmCommand,
};

#[derive(Clone, Copy)]
pub(super) enum Target<T> {
    Live(T),
    Demo,
}
pub(super) enum Dialog {
    Create {
        target: Target<ObservedVm>,
        vm: String,
        id: VmId,
    },
    Restore {
        target: Target<ObservedSnapshot>,
        vm: String,
        id: VmId,
        snapshot: String,
        snapshot_id: xenadmin_core::SnapshotId,
        vm_target: Target<ObservedVm>,
        expected_state: PowerState,
        checkpoint: bool,
        shutdown: bool,
        start_after: bool,
    },
    Edit {
        source: SnapshotReview,
    },
    Clone {
        source: SnapshotReview,
        as_template: bool,
    },
    Delete {
        source: SnapshotReview,
        disks: Vec<ReviewedDisk>,
        retained: Vec<ReviewedDisk>,
        disk_inventory_available: bool,
        delete_disks: bool,
        memory_checkpoint: bool,
    },
}
pub(super) struct SnapshotReview {
    target: Target<ObservedSnapshot>,
    pool: PoolId,
    vm: String,
    id: VmId,
    snapshot: String,
    snapshot_id: SnapshotId,
    power_state: PowerState,
    description: String,
    vm_target: Target<ObservedVm>,
}
pub(super) struct ReviewedDisk {
    id: VdiId,
    name: String,
    size: Option<u64>,
}
impl SnapshotReview {
    fn capture(s: &State) -> Result<Self, xenadmin_core::Error> {
        let id = s.snapshot_view.ok_or(xenadmin_core::Error::Stale)?;
        let inventory = s.inventory.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let vm = inventory.vms.get(&id).ok_or(xenadmin_core::Error::Stale)?;
        let snapshot = selected_snapshot(s).ok_or(xenadmin_core::Error::Stale)?;
        if snapshot.vm != Some(id) {
            return Err(xenadmin_core::Error::Stale);
        }
        let target = if s.demo {
            Target::Demo
        } else {
            Target::Live(
                s.snapshot
                    .as_ref()
                    .ok_or(xenadmin_core::Error::Stale)?
                    .observe_snapshot(id, snapshot.id)?,
            )
        };
        Ok(Self {
            target,
            pool: inventory.pool_id,
            vm: vm.name.clone(),
            id,
            snapshot: snapshot.name.clone(),
            snapshot_id: snapshot.id,
            power_state: snapshot.power_state.clone(),
            description: snapshot.description.clone(),
            vm_target: if s.demo {
                Target::Demo
            } else {
                Target::Live(
                    s.snapshot
                        .as_ref()
                        .ok_or(xenadmin_core::Error::Stale)?
                        .observe_vm(id)?,
                )
            },
        })
    }
    fn live(&self, s: &State) -> bool {
        matches!(self.target, Target::Live(observed) if s.snapshot.as_ref()
            .and_then(|snapshot| snapshot.observe_snapshot(self.id, self.snapshot_id).ok())
            == Some(observed))
    }
    fn permits(&self, s: &State, action: SnapshotAction) -> bool {
        s.inventory
            .as_ref()
            .filter(|inventory| inventory.pool_id == self.pool)
            .and_then(|inventory| inventory.snapshots.get(&self.snapshot_id))
            .is_some_and(|snapshot| {
                snapshot.vm == Some(self.id)
                    && snapshot.power_state == self.power_state
                    && snapshot.allowed_actions.contains(&action)
            })
    }
    fn summary(&self, k: &Kit) -> El {
        caption(
            format!(
                "VM: {}\nVM UUID: {}\nSnapshot: {}\nSnapshot UUID: {}",
                self.vm, self.id, self.snapshot, self.snapshot_id
            ),
            k,
        )
        .wrap_anywhere()
    }
}
pub(super) fn selected_snapshot(s: &State) -> Option<&xenadmin_core::Snapshot> {
    s.snapshot_table.selected().next().and_then(|id| {
        s.snapshot_rows
            .iter()
            .find(|snapshot| snapshot.id.to_string() == id)
    })
}
fn begin_edit(s: &mut State) {
    match SnapshotReview::capture(s) {
        Ok(source) => {
            s.snapshot_name = TextEdit::new(&source.snapshot);
            s.snapshot_description = TextEdit::new(&source.description);
            s.snapshot_dialog = Some(Dialog::Edit { source });
            s.error = None;
        }
        Err(error) => s.error = Some(error.to_string()),
    }
}
fn begin_clone(s: &mut State, as_template: bool) {
    match SnapshotReview::capture(s) {
        Ok(source) => {
            s.snapshot_name = TextEdit::new("");
            s.snapshot_dialog = Some(Dialog::Clone {
                source,
                as_template,
            });
            s.error = None;
        }
        Err(error) => s.error = Some(error.to_string()),
    }
}
fn begin_delete(s: &mut State) {
    let result = (|| {
        let source = SnapshotReview::capture(s)?;
        let inventory = s.inventory.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let disk_inventory_available = ["vbd", "vdi"].iter().all(|class| {
            inventory.details.classes.get(*class) == Some(&ClassAvailability::Available)
        });
        let candidates = inventory.snapshot_deletion_disks(source.snapshot_id);
        let mut disks = Vec::new();
        let mut retained = Vec::new();
        let attached: std::collections::BTreeSet<_> = inventory
            .details
            .vbds
            .values()
            .filter(|vbd| vbd.snapshot == Some(source.snapshot_id))
            .filter_map(|vbd| vbd.vdi)
            .collect();
        for id in attached {
            let record = inventory.details.vdis.get(&id);
            let disk = ReviewedDisk {
                id,
                name: record
                    .map_or("Disk details unavailable", |disk| disk.name.as_str())
                    .into(),
                size: record.and_then(|disk| disk.virtual_size),
            };
            if disk_inventory_available && candidates.contains(&id) {
                disks.push(disk);
            } else {
                retained.push(disk);
            }
        }
        let memory_checkpoint = inventory
            .snapshots
            .get(&source.snapshot_id)
            .is_some_and(|snapshot| snapshot.power_state == PowerState::Suspended);
        s.snapshot_dialog = Some(Dialog::Delete {
            source,
            disks,
            retained,
            disk_inventory_available,
            delete_disks: false,
            memory_checkpoint,
        });
        Ok::<_, xenadmin_core::Error>(())
    })();
    s.error = result.err().map(|error| error.to_string());
}

pub(super) fn refresh(s: &mut State) {
    s.snapshot_rows = s
        .snapshot_view
        .and_then(|vm| {
            s.inventory
                .as_ref()
                .map(|i| i.snapshots.for_vm(vm).cloned().collect())
        })
        .unwrap_or_default();
    s.snapshot_rows
        .sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    s.snapshot_ancestry = s
        .snapshot_view
        .and_then(|vm| s.inventory.as_ref().map(|i| i.snapshot_ancestry(vm)))
        .unwrap_or_default();
    if s.snapshot_tree {
        let positions: std::collections::BTreeMap<_, _> = s
            .snapshot_ancestry
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id, index))
            .collect();
        s.snapshot_rows
            .sort_by_key(|row| positions.get(&row.id).copied().unwrap_or(usize::MAX));
    }
}
pub(super) fn open(s: &mut State) {
    s.metrics = None;
    let Some(vm) = s.selected().and_then(|r| r.vm) else {
        return;
    };
    s.console = None;
    s.devices = None;
    s.snapshot_view = Some(vm);
    s.snapshot_table = TableState::new().key("snapshots-table");
    s.show_details = false;
    refresh(s);
}
fn current(s: &State) -> bool {
    s.can_mutate()
}
pub(super) fn begin_create(s: &mut State) {
    if s.snapshot_capture.busy() {
        return;
    }
    s.snapshot_capture.opt_in = false;
    s.snapshot_capture.message = None;
    let result = (|| {
        let vm = s.snapshot_view.ok_or(xenadmin_core::Error::Stale)?;
        let name = s
            .inventory
            .as_ref()
            .and_then(|i| i.vms.get(&vm))
            .ok_or(xenadmin_core::Error::Stale)?
            .name
            .clone();
        let target = if s.demo {
            Target::Demo
        } else {
            Target::Live(
                s.snapshot
                    .as_ref()
                    .ok_or(xenadmin_core::Error::Stale)?
                    .observe_vm(vm)?,
            )
        };
        s.snapshot_name = TextEdit::new("");
        s.snapshot_description = TextEdit::new("");
        s.snapshot_memory = !s
            .inventory
            .as_ref()
            .and_then(|i| i.vms.get(&vm))
            .is_some_and(|vm| vm.can_snapshot)
            && memory_available(s, vm);
        s.snapshot_dialog = Some(Dialog::Create {
            target,
            vm: name,
            id: vm,
        });
        Ok::<_, xenadmin_core::Error>(())
    })();
    s.error = result.err().map(|error| error.to_string());
}
pub(super) fn edit_scene(s: &mut State) {
    demo(s);
    s.table.select("30000000-0000-0000-0000-000000000000");
    open(s);
    s.snapshot_table
        .select("60000000-0000-0000-0000-000000000000");
    begin_edit(s);
}
pub(super) fn restore_scene(s: &mut State) {
    demo(s);
    let id = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let guest = Arc::make_mut(s.inventory.as_mut().unwrap())
        .vms
        .get_mut(&id)
        .unwrap();
    guest.power_state = PowerState::Running;
    guest.allowed_actions.push(PowerAction::CleanShutdown);
    s.table.select(id.to_string());
    open(s);
    s.snapshot_table
        .select("60000000-0000-0000-0000-000000000000");
    begin_restore(s);
}
pub(super) fn begin_restore(s: &mut State) {
    let result = (|| {
        let vm = s.snapshot_view.ok_or(xenadmin_core::Error::Stale)?;
        let guest = s
            .inventory
            .as_ref()
            .and_then(|i| i.vms.get(&vm))
            .ok_or(xenadmin_core::Error::Stale)?;
        let selected = s
            .snapshot_table
            .selected()
            .next()
            .and_then(|id| s.snapshot_rows.iter().find(|r| r.id.to_string() == id))
            .ok_or(xenadmin_core::Error::Stale)?;
        let target = if s.demo {
            Target::Demo
        } else {
            Target::Live(
                s.snapshot
                    .as_ref()
                    .ok_or(xenadmin_core::Error::Stale)?
                    .observe_snapshot(vm, selected.id)?,
            )
        };
        s.snapshot_dialog = Some(Dialog::Restore {
            target,
            vm: guest.name.clone(),
            id: vm,
            snapshot: selected.name.clone(),
            snapshot_id: selected.id,
            vm_target: if s.demo {
                Target::Demo
            } else {
                Target::Live(
                    s.snapshot
                        .as_ref()
                        .ok_or(xenadmin_core::Error::Stale)?
                        .observe_vm(vm)?,
                )
            },
            expected_state: guest.power_state.clone(),
            checkpoint: selected.power_state == PowerState::Suspended,
            shutdown: false,
            start_after: false,
        });
        Ok::<_, xenadmin_core::Error>(())
    })();
    s.error = result.err().map(|error| error.to_string());
}
fn submit(s: &mut State) {
    if s.snapshot_capture.busy() {
        return;
    }
    let command = match &s.snapshot_dialog {
        Some(Dialog::Create {
            target: Target::Live(vm),
            ..
        }) => VmCommand::SnapshotWorkflow {
            vm: *vm,
            operation: SnapshotWorkflow::Create {
                name: s.snapshot_name.text.trim().into(),
                description: s.snapshot_description.text.clone(),
                memory: s.snapshot_memory,
            },
        },
        Some(Dialog::Restore {
            vm_target: Target::Live(vm),
            snapshot_id,
            expected_state,
            checkpoint,
            shutdown,
            start_after,
            ..
        }) => VmCommand::SnapshotWorkflow {
            vm: *vm,
            operation: SnapshotWorkflow::Restore {
                snapshot: *snapshot_id,
                expected_state: expected_state.clone(),
                checkpoint: *checkpoint,
                shutdown: *shutdown,
                start_after: *start_after,
            },
        },
        Some(Dialog::Edit {
            source:
                SnapshotReview {
                    vm_target: Target::Live(vm),
                    snapshot_id,
                    snapshot,
                    description,
                    ..
                },
        }) => VmCommand::SnapshotWorkflow {
            vm: *vm,
            operation: SnapshotWorkflow::Edit {
                snapshot: *snapshot_id,
                expected_name: snapshot.clone(),
                expected_description: description.clone(),
                name: s.snapshot_name.text.trim().into(),
                description: s.snapshot_description.text.clone(),
            },
        },
        Some(Dialog::Clone {
            source:
                SnapshotReview {
                    target: Target::Live(snapshot),
                    ..
                },
            as_template,
        }) => match SnapshotName::new(&s.snapshot_name.text) {
            Ok(name) => VmCommand::CloneSnapshot {
                snapshot: *snapshot,
                name,
                as_template: *as_template,
            },
            Err(error) => {
                s.error = Some(error.to_string());
                return;
            }
        },
        Some(Dialog::Delete {
            source:
                SnapshotReview {
                    target: Target::Live(snapshot),
                    ..
                },
            disks,
            delete_disks,
            ..
        }) => VmCommand::DeleteSnapshot {
            snapshot: *snapshot,
            disks: if *delete_disks {
                disks.iter().map(|disk| disk.id).collect()
            } else {
                Vec::new()
            },
        },
        _ => return,
    };
    if s.snapshot_capture.opt_in
        && let Some(Dialog::Create { id, .. }) = s.snapshot_dialog
    {
        snapshot_capture::begin(s, command, id);
        return;
    }
    s.submit_command(command);
    if s.error.is_none() {
        s.snapshot_dialog = None;
        s.snapshot_name = TextEdit::new("");
        s.snapshot_memory = false;
    }
}

pub(super) fn view(s: &State, k: &Kit) -> El {
    let vm = s
        .snapshot_view
        .and_then(|id| s.inventory.as_ref()?.vms.get(&id));
    let selected = selected_snapshot(s);
    let can_create =
        current(s) && vm.is_some_and(|vm| vm.can_snapshot || memory_available(s, vm.id));
    let can_restore = current(s)
        && vm.is_some_and(|vm| {
            vm.power_state == PowerState::Halted
                || (vm.power_state == PowerState::Running
                    && vm.allowed_actions.contains(&PowerAction::CleanShutdown))
        })
        && selected
            .is_some_and(|snapshot| snapshot.allowed_actions.contains(&SnapshotAction::Revert));
    let can_clone = current(s)
        && selected
            .is_some_and(|snapshot| snapshot.allowed_actions.contains(&SnapshotAction::Clone));
    let can_delete = current(s)
        && selected.is_some_and(|snapshot| {
            matches!(
                snapshot.power_state,
                PowerState::Halted | PowerState::Suspended
            ) && snapshot.allowed_actions.contains(&SnapshotAction::Delete)
        });
    let mut content = col().w_full().h_full().min_h(0.0).padding(20.0).gap(12.0)
        .child(row().w_full().justify_between().center_y().child(heading("Snapshots",k)).child(Button::new("Back to inventory").variant(ButtonVariant::Ghost).on_press(|s:&mut State|s.snapshot_view=None).build(k).key("back-from-snapshots")))
        .child(heading(vm.map(|vm|vm.name.as_str()).unwrap_or("VM unavailable"),k))
        .child(caption(if s.window_width < 700.0 { "Select a snapshot to restore, copy or delete." } else { "Disk snapshots capture VM configuration and virtual disks. Choose Include guest memory for a memory checkpoint when supported. Application quiescing is not available on XCP-ng 8.1 and later." },k).wrap())
        .child(row().gap(8.0).flex_wrap()
            .child(Button::new("Create snapshot").variant(ButtonVariant::Outline).disabled(!s.demo && !can_create).on_press(begin_create).build(k).key("create-snapshot"))
            .child(Button::new("Restore snapshot").variant(ButtonVariant::Outline).disabled(selected.is_none() || (!s.demo && !can_restore)).on_press(begin_restore).build(k).key("restore-snapshot"))
            .child(Button::new("Edit properties").variant(ButtonVariant::Outline).disabled(selected.is_none() || (!s.demo && !current(s))).on_press(begin_edit).build(k).key("edit-snapshot"))
            .child(Button::new("Create VM").variant(ButtonVariant::Outline).disabled(selected.is_none() || (!s.demo && !can_clone)).on_press(|s: &mut State| begin_clone(s, false)).build(k).key("clone-snapshot-vm"))
            .child(Button::new("Create template").variant(ButtonVariant::Outline).disabled(selected.is_none() || (!s.demo && !can_clone)).on_press(|s: &mut State| begin_clone(s, true)).build(k).key("clone-snapshot-template"))
            .child(Button::new("Inspect snapshot").variant(ButtonVariant::Outline).disabled(selected.is_none()).on_press(snapshot_inspection::open).build(k).key("inspect-snapshot"))
            .child(Button::new(if s.snapshot_tree {"Show list"}else{"Show ancestry"}).variant(ButtonVariant::Ghost).on_press(|s:&mut State|{s.snapshot_tree = !s.snapshot_tree;refresh(s);}).build(k).key("snapshot-tree-toggle"))
            .child(Button::new("Activity").variant(ButtonVariant::Ghost).on_press(activity::open).build(k).key("snapshot-activity"))
            .child(Button::new("Delete snapshot").variant(ButtonVariant::Danger).disabled(selected.is_none() || (!s.demo && !can_delete)).on_press(begin_delete).build(k).key("delete-snapshot")));
    if let Some(snapshot) = selected
        && let Some(parent) = snapshot.vm
    {
        content = content.child(transfers::export_button(
            s,
            xenadmin_core::ExportSource::Snapshot {
                snapshot: snapshot.id,
                parent,
            },
            k,
        ));
    }
    if let Some(message) = &s.snapshot_capture.message {
        content = content.child(caption(message, k).wrap().role("status"));
    }
    if s.demo {
        content = content.child(
            caption(
                "DEMO • Synthetic snapshots. Confirming host actions is disabled.",
                k,
            )
            .wrap(),
        );
    } else if s
        .connection
        .as_ref()
        .is_some_and(|connection| connection.is_read_only())
    {
        content = content
            .child(caption("Read-only connection: snapshot changes are disabled.", k).wrap());
    } else if !current(s) {
        content = content.child(
            caption(
                "Inventory is stale. Snapshot actions require a current connection.",
                k,
            )
            .wrap(),
        );
    }
    if vm.is_some_and(|vm| vm.power_state != PowerState::Halted) {
        content = content.child(caption("Restore can cleanly shut down a running VM when explicitly selected. Paused or suspended VMs must be shut down separately.", k).wrap());
    }
    if !s.demo && current(s) && selected.is_some() {
        if !can_clone {
            content = content.child(caption("The host does not currently allow cloning the selected snapshot into a VM or template.", k).wrap());
        }
        if !can_delete {
            content = content.child(caption("The selected snapshot cannot currently be deleted. Deletion requires a halted snapshot or a memory checkpoint the host permits stopping.", k).wrap());
        }
    }
    if let Some(error) = s
        .error
        .as_deref()
        .or_else(|| s.snapshot.as_ref().and_then(|snapshot| snapshot.error()))
    {
        content = content.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    if s.snapshot_rows.is_empty() {
        content = content.child(
            col()
                .grow(1.0)
                .w_full()
                .center()
                .child(caption("No snapshots for this VM.", k)),
        );
    } else {
        let table = Table::new(
            |s: &State| &s.snapshot_table,
            |s: &mut State| &mut s.snapshot_table,
            |s: &State| s.snapshot_rows.len(),
            |s: &State, i| Cow::Owned(s.snapshot_rows[i].id.to_string()),
        )
        .name("VM snapshots")
        .column(TableColumn::new("name", "Name").flex(2.0).min(150.0));
        let table = if s.window_width >= 700.0 {
            table.column(
                TableColumn::new("created", "Created (host time)")
                    .flex(1.0)
                    .min(150.0),
            )
        } else {
            table
        };
        let table = table
            .cell(|s: &State, i, column| {
                let snapshot = &s.snapshot_rows[i];
                if column == "name" {
                    if s.snapshot_tree {
                        let ancestry = s
                            .snapshot_ancestry
                            .get(i)
                            .filter(|row| row.id == snapshot.id);
                        Cell::text(format!(
                            "{}{}{}{}",
                            "  ".repeat(ancestry.map_or(0, |row| row.depth.min(12))),
                            if ancestry.is_some_and(|row| row.depth > 0) {
                                "└ "
                            } else {
                                ""
                            },
                            snapshot.name,
                            if ancestry.is_some_and(|row| row.current_parent) {
                                " · current base"
                            } else if ancestry.is_some_and(|row| row.issue.is_some()) {
                                " · ancestry incomplete"
                            } else {
                                ""
                            }
                        ))
                    } else {
                        Cell::text(&snapshot.name)
                    }
                } else {
                    Cell::text(snapshot.created_at.as_deref().unwrap_or("Unknown")).muted()
                }
            })
            .zebra(true)
            .build(s, k);
        content = content.child(if s.window_width < 700.0 {
            table.h(180.0).min_h(180.0)
        } else {
            table.grow(1.0).min_h(0.0)
        });
    }
    if let Some(selected) = selected {
        content = content.child(
            caption(
                format!(
                    "{} • {}\nUUID: {}\n{}",
                    selected.name,
                    selected.created_at.as_deref().unwrap_or("Unknown date"),
                    selected.id,
                    selected.description
                ),
                k,
            )
            .wrap_anywhere(),
        );
    }
    for operation in s
        .operation_rows
        .iter()
        .rev()
        .filter(|row| {
            Some(row.vm) == s.snapshot_view
                && s.inventory.as_ref().is_some_and(|i| i.pool_id == row.pool)
        })
        .take(2)
    {
        content = content.child(
            caption(
                format!(
                    "{} • {:?}\n{}",
                    operation.action.label(),
                    operation.state,
                    operation.detail
                ),
                k,
            )
            .wrap(),
        );
        if matches!(
            &operation.action,
            xenadmin_core::OperationAction::Snapshot(
                xenadmin_core::SnapshotOperation::Workflow { .. }
            )
        ) {
            let id = operation.id;
            if operation.can_cancel && !operation.cancel_requested && !operation.state.terminal() {
                content = content.child(
                    Button::new("Stop remaining steps")
                        .variant(ButtonVariant::Ghost)
                        .disabled(!current(s))
                        .on_press(move |s: &mut State| {
                            s.control_operation(
                                id,
                                xenadmin_core::OperationControl::RequestCancellation,
                            )
                        })
                        .build(k),
                );
            }
            if operation.can_recheck()
                && operation.state == xenadmin_core::OperationState::OutcomeUnknown
            {
                content = content.child(
                    Button::new("Recheck snapshot outcome")
                        .variant(ButtonVariant::Outline)
                        .disabled(!current(s))
                        .on_press(move |s: &mut State| {
                            s.control_operation(id, xenadmin_core::OperationControl::Recheck)
                        })
                        .build(k),
                );
            }
        }
        if let Some(id) = operation.created_snapshot {
            content =
                content.child(caption(format!("Created snapshot UUID: {id}"), k).wrap_anywhere());
        }
        if let Some(id) = operation.created_vm {
            content =
                content.child(caption(format!("Created object UUID: {id}"), k).wrap_anywhere());
            if let Some(created) = s
                .inventory
                .as_ref()
                .and_then(|inventory| inventory.vms.get(&id))
            {
                let is_template = created.is_template;
                content = content.child(
                    Button::new(if is_template {
                        "Show created template"
                    } else {
                        "Show created VM"
                    })
                    .variant(ButtonVariant::Outline)
                    .on_press(move |s: &mut State| {
                        s.snapshot_view = None;
                        s.host_filter = None;
                        s.search = TextEdit::new("");
                        s.page = if is_template {
                            Page::Templates
                        } else {
                            Page::Guests
                        };
                        s.filter_rows();
                        s.table.select(id.to_string());
                    })
                    .build(k),
                );
            }
        }
    }
    if s.window_width < 700.0 {
        content.scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
    } else {
        content
    }
}

fn memory_available(s: &State, id: VmId) -> bool {
    s.inventory
        .as_ref()
        .and_then(|i| i.details.vms.get(&id))
        .is_some_and(|vm| {
            !vm.blocked_operations.contains_key("checkpoint")
                && vm.allowed_operations.as_ref().is_some_and(|operations| {
                    operations.iter().any(|operation| operation == "checkpoint")
                })
        })
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(dialog) = &s.snapshot_dialog else {
        return el();
    };
    let mut body = col()
        .w((s.window_width - 40.0).clamp(280.0, 480.0))
        .max_h((s.window_height - 40.0).clamp(280.0, 650.0))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .padding(24.0)
        .gap(14.0)
        .radius(10.0)
        .fill(k.c.bg_base);
    let (title, confirm, valid, live) = match dialog {
        Dialog::Create { target, vm, id } => {
            body=body.child(heading("Create snapshot",k))
                .child(caption(format!("VM: {vm}\nUUID: {id}"),k).wrap_anywhere())
                 .child(Checkbox::new(s.snapshot_memory).label("Include guest memory").disabled(!s.demo && !memory_available(s,*id)).on_toggle(|s:&mut State|s.snapshot_memory = !s.snapshot_memory).build(k))
                .child(caption(if s.snapshot_memory {"Captures disks and guest memory. The VM may pause while memory is saved."}else{"Guest memory is not included. For application-consistent data, quiesce applications or shut down the guest first."},k).wrap())
                .child(TextInput::new(&s.snapshot_name).label("Snapshot name").placeholder("Snapshot name").on_edit(|s:&mut State|&mut s.snapshot_name).build(k).key("snapshot-name"))
                .child(caption("Application quiescing is not available on XCP-ng 8.1 and later.", k).wrap());
            body = body.child(Checkbox::new(s.snapshot_capture.opt_in).label("Save a console preview with the snapshot")
                .disabled(s.snapshot_capture.busy() || (!s.demo && !snapshot_capture::available(s,*id)))
                .on_toggle(|s:&mut State|s.snapshot_capture.opt_in = !s.snapshot_capture.opt_in).build(k).key("snapshot-capture-option"))
                .child(caption("Optional: captures the console before creation and stores the image on the pool. It may contain sensitive guest information. Snapshot and preview outcomes are reported separately.",k).wrap());
            if s.snapshot_capture.opt_in
                && let Some(message) = &s.snapshot_capture.message
            {
                body = body.child(caption(message, k).wrap().role("status"));
            }
            let selected_mode_available = if s.snapshot_memory {
                memory_available(s, *id)
            } else {
                s.inventory
                    .as_ref()
                    .and_then(|i| i.vms.get(id))
                    .is_some_and(|vm| vm.can_snapshot)
            };
            body = body.child(caption("Description", k)).child(
                TextInput::new(&s.snapshot_description)
                    .label("Snapshot description")
                    .placeholder("Optional description")
                    .on_edit(|s: &mut State| &mut s.snapshot_description)
                    .build(k)
                    .key("snapshot-description"),
            );
            let valid = SnapshotWorkflow::Create {
                name: s.snapshot_name.text.trim().into(),
                description: s.snapshot_description.text.clone(),
                memory: s.snapshot_memory,
            }
            .validate()
            .is_ok();
            if !s.demo && !selected_mode_available {
                body = body.child(
                    caption(
                        "The host does not currently allow the selected snapshot mode.",
                        k,
                    )
                    .wrap()
                    .role("status"),
                );
            }
            if !valid && !s.snapshot_name.text.is_empty() {
                body = body.child(
                    caption(xenadmin_core::Error::InvalidSnapshotMetadata.to_string(), k)
                        .wrap()
                        .role("alert"),
                );
            }
            (
                "Create snapshot",
                "Create snapshot",
                valid && (s.demo || selected_mode_available),
                matches!(target,Target::Live(observed) if s.snapshot.as_ref().and_then(|s|s.observe_vm(*id).ok())==Some(*observed)),
            )
        }
        Dialog::Restore {
            target,
            vm,
            id,
            snapshot,
            snapshot_id,
            expected_state,
            checkpoint,
            shutdown,
            start_after,
            ..
        } => {
            body=body.child(heading("Restore this snapshot?",k))
                .child(caption(format!("VM: {vm}\nVM UUID: {id}\nSnapshot: {snapshot}\nSnapshot UUID: {snapshot_id}"),k).wrap_anywhere())
                .child(caption("This replaces the VM's configuration and disk contents. Changes made since the snapshot will be lost.",k).wrap());
            if *expected_state == PowerState::Running {
                body = body.child(
                    Checkbox::new(*shutdown)
                        .label("Cleanly shut down this running VM before restore")
                        .on_toggle(|s: &mut State| {
                            if let Some(Dialog::Restore { shutdown, .. }) = &mut s.snapshot_dialog {
                                *shutdown = !*shutdown
                            }
                        })
                        .build(k)
                        .key("restore-shutdown"),
                );
            }
            body=body.child(Checkbox::new(*start_after).label(if *checkpoint {"Resume restored memory after restore"} else {"Start VM after restore"}).on_toggle(|s:&mut State|if let Some(Dialog::Restore{start_after,..})=&mut s.snapshot_dialog {*start_after = !*start_after}).build(k).key("restore-start"))
                .child(caption(if *checkpoint {"Restores the saved memory as a separate VM memory image. Without Resume, the VM remains suspended."}else{"Without Start, the restored VM remains halted."},k).wrap())
                .child(caption("Shutdown, restore and startup are tracked separately. A failure stops later steps; interrupted writes are never automatically repeated. Stop remaining steps does not interrupt a running host task.",k).wrap());
            let operation = SnapshotWorkflow::Restore {
                snapshot: *snapshot_id,
                expected_state: expected_state.clone(),
                checkpoint: *checkpoint,
                shutdown: *shutdown,
                start_after: *start_after,
            };
            let valid = operation.validate().is_ok()
                && s.inventory.as_ref().is_some_and(|i| {
                    i.vms.get(id).is_some_and(|vm| {
                        vm.power_state == *expected_state
                            && (!shutdown
                                || vm.allowed_actions.contains(&PowerAction::CleanShutdown))
                    }) && i.snapshots.get(snapshot_id).is_some_and(|snap| {
                        snap.vm == Some(*id)
                            && snap.power_state
                                == if *checkpoint {
                                    PowerState::Suspended
                                } else {
                                    PowerState::Halted
                                }
                            && snap.allowed_actions.contains(&SnapshotAction::Revert)
                    })
                });
            if !valid {
                body=body.child(caption("Review the current VM state and explicitly select a permitted clean shutdown before restoring a running VM.",k).wrap().role("status"));
            }
            (
                "Restore snapshot",
                "Restore snapshot",
                valid,
                matches!(target,Target::Live(observed) if s.snapshot.as_ref().and_then(|s|s.observe_snapshot(*id,*snapshot_id).ok())==Some(*observed)),
            )
        }
        Dialog::Edit { source } => {
            body=body.child(heading("Edit snapshot properties",k)).child(source.summary(k))
                .child(caption("Name",k)).child(TextInput::new(&s.snapshot_name).label("Snapshot name").on_edit(|s:&mut State| &mut s.snapshot_name).build(k).key("snapshot-name"))
                .child(caption("Description",k)).child(TextInput::new(&s.snapshot_description).label("Snapshot description").on_edit(|s:&mut State| &mut s.snapshot_description).build(k).key("snapshot-description"))
                .child(caption("Name and description are saved in separate verified steps. Concurrent metadata changes stop the update.",k).wrap());
            let valid = SnapshotWorkflow::Edit {
                snapshot: source.snapshot_id,
                expected_name: source.snapshot.clone(),
                expected_description: source.description.clone(),
                name: s.snapshot_name.text.trim().into(),
                description: s.snapshot_description.text.clone(),
            }
            .validate()
            .is_ok()
                && s.inventory.as_ref().is_some_and(|i| {
                    i.pool_id == source.pool
                        && i.snapshots.get(&source.snapshot_id).is_some_and(|snap| {
                            snap.vm == Some(source.id)
                                && snap.name == source.snapshot
                                && snap.description == source.description
                        })
                });
            (
                "Edit snapshot properties",
                "Save properties",
                valid,
                source.live(s),
            )
        }
        Dialog::Clone {
            source,
            as_template,
        } => {
            let title = if *as_template {
                "Create template from snapshot"
            } else {
                "Create VM from snapshot"
            };
            body = body
                .child(heading(title, k))
                .child(source.summary(k))
                .child(caption(
                    if *as_template {
                        "Creates a separate template from this snapshot's configuration and disks in the existing storage. The source VM and snapshot are retained."
                    } else {
                        "Creates a separate VM from this snapshot's configuration and disks in the existing storage. The source VM and snapshot are retained. The new VM will not start automatically."
                    },
                    k,
                ).wrap())
                .child(TextInput::new(&s.snapshot_name)
                    .label(if *as_template { "Template name" } else { "VM name" })
                    .placeholder(if *as_template { "Template name" } else { "VM name" })
                    .on_edit(|s: &mut State| &mut s.snapshot_name)
                    .build(k)
                    .key("snapshot-clone-name"));
            let valid_name = SnapshotName::new(&s.snapshot_name.text).is_ok();
            if !valid_name && !s.snapshot_name.text.is_empty() {
                body = body.child(
                    caption(
                        "Enter a name of 1–255 characters without control characters.",
                        k,
                    )
                    .wrap()
                    .role("alert"),
                );
            }
            let permitted = source.permits(s, SnapshotAction::Clone);
            if !s.demo && !permitted {
                body = body.child(caption("The snapshot changed or the host no longer permits cloning it. Close this review and select the snapshot again.", k).wrap().role("status"));
            }
            (
                title,
                if *as_template {
                    "Create template"
                } else {
                    "Create VM"
                },
                valid_name && (s.demo || permitted),
                source.live(s),
            )
        }
        Dialog::Delete {
            source,
            disks,
            retained,
            disk_inventory_available,
            delete_disks,
            memory_checkpoint,
        } => {
            body = body
                .child(heading("Delete this snapshot?", k))
                .child(source.summary(k))
                .child(caption("This permanently removes this recovery point. The source VM is not deleted or shut down. Deleting a snapshot cannot be undone.", k).wrap());
            if *memory_checkpoint {
                body = body.child(caption("The saved memory checkpoint is stopped before its snapshot record is deleted. Its saved memory state is discarded.", k).wrap());
            }
            if !disk_inventory_available {
                body = body.child(caption("Disk inventory is unavailable or incomplete. Optional data-disk cleanup is disabled. Saved checkpoint memory, if present, is still discarded.", k).wrap().role("status"));
            } else if disks.is_empty() {
                body = body.child(caption("No exclusively owned snapshot disks are eligible for cleanup. Associated virtual disks will be retained.", k).wrap());
            } else {
                body = body.child(
                    Checkbox::new(*delete_disks)
                        .label(format!(
                            "Also delete {} exclusively owned snapshot disk(s)",
                            disks.len()
                        ))
                        .on_toggle(|s: &mut State| {
                            if let Some(Dialog::Delete { delete_disks, .. }) =
                                &mut s.snapshot_dialog
                            {
                                *delete_disks = !*delete_disks;
                            }
                        })
                        .build(k)
                        .key("snapshot-delete-disks"),
                );
                body = body.child(caption(if *delete_disks {
                    "These reviewed disks will be deleted after the snapshot record. Ownership is checked again before deletion."
                } else {
                    "These disks will be retained unless disk cleanup is selected:"
                }, k).wrap());
                for disk in disks {
                    body = body.child(
                        caption(
                            format!(
                                "{} • {}\nUUID: {}",
                                disk.name,
                                storage_bytes(disk.size),
                                disk.id
                            ),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                }
            }
            if !retained.is_empty() {
                body = body.child(heading("Disks retained", k))
                    .child(caption("Shared disks, CD/DVD media, disks without snapshot ownership, and disks with incomplete ownership data are retained.", k).wrap());
                for disk in retained {
                    body = body.child(
                        caption(
                            format!(
                                "{} • {}\nUUID: {}",
                                disk.name,
                                storage_bytes(disk.size),
                                disk.id
                            ),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                }
            }
            let ownership_current = !delete_disks
                || s.inventory.as_ref().is_some_and(|inventory| {
                    inventory.pool_id == source.pool && {
                        let eligible = inventory.snapshot_deletion_disks(source.snapshot_id);
                        disks.iter().all(|disk| eligible.contains(&disk.id))
                    }
                });
            if !ownership_current {
                body = body.child(caption("Disk ownership changed after this review. Close it and review the snapshot again before deleting disks.", k).wrap().role("alert"));
            }
            let permitted = source.permits(s, SnapshotAction::Delete)
                && matches!(
                    source.power_state,
                    PowerState::Halted | PowerState::Suspended
                );
            if !s.demo && !permitted {
                body = body.child(caption("The snapshot changed or the host no longer permits deleting it. Close this review and select the snapshot again.", k).wrap().role("status"));
            }
            body = body.child(caption("Deletion runs in tracked steps. If a later step fails, the operation history identifies completed work and remaining disks. An interrupted response may leave the outcome unknown; recheck it before retrying.", k).wrap());
            (
                "Delete snapshot",
                "Delete snapshot",
                (s.demo || permitted) && ownership_current,
                source.live(s),
            )
        }
    };
    if s.demo {
        body = body.child(caption("Demo preview • no host changes can be submitted.", k).wrap());
    }
    if let Some(error) = &s.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    body = body.child(
        row()
            .gap(8.0)
            .justify_end()
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_press(snapshot_capture::cancel_dialog)
                    .build(k)
                    .key("cancel-snapshot"),
            )
            .child(
                Button::new(confirm)
                    .variant(
                        if matches!(dialog, Dialog::Restore { .. } | Dialog::Delete { .. }) {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Outline
                        },
                    )
                    .disabled(!valid || !live || !current(s) || s.snapshot_capture.busy())
                    .on_press(submit)
                    .build(k)
                    .key("confirm-snapshot"),
            ),
    );
    el().key("snapshot-dialog")
        .role("dialog")
        .label(title)
        .w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .modal(false, snapshot_capture::cancel_dialog)
        .child(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot_state() -> State {
        let mut state = init();
        demo(&mut state);
        state.table.select("30000000-0000-0000-0000-000000000000");
        open(&mut state);
        state
            .snapshot_table
            .select("60000000-0000-0000-0000-000000000000");
        state
    }
    fn runtime(mut state: State, size: (f32, f32)) -> rune::Runtime<State> {
        state.window_width = size.0;
        state.window_height = size.1;
        let mut rt = rune::Runtime::new(
            state,
            super::super::view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }
    fn click(rt: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("missing control: {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }
    #[test]
    fn snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut rt = runtime(snapshot_state(), size);
            click(&mut rt, "create-snapshot");
            click(&mut rt, "snapshot-name");
            rt.key(rune::Key::Text("Before upgrade".into()).into());
            rt.frame(0.0);
            rt.update(|_, tx| tx.scroll_into_view("snapshot-description"));
            rt.frame(0.0);
            click(&mut rt, "snapshot-description");
            rt.key(rune::Key::Text("Reviewed rollback point".into()).into());
            rt.frame(0.0);
            assert_eq!(
                rt.state.snapshot_description.text,
                "Reviewed rollback point"
            );
            assert!(
                rt.accessibility_json()
                    .to_string()
                    .contains("Application quiescing is not available on XCP-ng 8.1 and later")
            );
            rt.update(|_, tx| tx.scroll_into_view("confirm-snapshot"));
            rt.frame(0.0);
            click(&mut rt, "confirm-snapshot");
            assert!(rt.state.operations.is_empty());
            assert!(rt.state.snapshot_dialog.is_some());
        }
    }
    #[test]
    fn restore_requires_explicit_shutdown_and_start_choices_and_captures_identity() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut state = snapshot_state();
            let id = state.snapshot_view.unwrap();
            let guest = Arc::make_mut(state.inventory.as_mut().unwrap())
                .vms
                .get_mut(&id)
                .unwrap();
            guest.power_state = PowerState::Running;
            guest.allowed_actions.push(PowerAction::CleanShutdown);
            let mut rt = runtime(state, size);
            click(&mut rt, "restore-snapshot");
            assert!(matches!(
                &rt.state.snapshot_dialog,
                Some(Dialog::Restore {
                    expected_state: PowerState::Running,
                    shutdown: false,
                    start_after: false,
                    ..
                })
            ));
            click(&mut rt, "restore-shutdown");
            click(&mut rt, "restore-start");
            assert!(
                matches!(&rt.state.snapshot_dialog,Some(Dialog::Restore{shutdown:true,start_after:true,snapshot,..}) if snapshot=="Before upgrade")
            );
            rt.update(|s, _| {
                s.snapshot_table
                    .select("60000000-0000-0000-0000-000000000001")
            });
            rt.frame(0.0);
            assert!(
                matches!(&rt.state.snapshot_dialog,Some(Dialog::Restore{snapshot,..}) if snapshot=="Before upgrade")
            );
            rt.update(|_, tx| tx.scroll_into_view("confirm-snapshot"));
            rt.frame(0.0);
            click(&mut rt, "confirm-snapshot");
            assert!(rt.state.operations.is_empty());
        }
    }
    #[test]
    fn snapshot_editor_keeps_reviewed_metadata_when_selection_changes() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut rt = runtime(snapshot_state(), size);
            click(&mut rt, "edit-snapshot");
            assert_eq!(rt.state.snapshot_name.text, "Before upgrade");
            click(&mut rt, "snapshot-description");
            rt.key(rune::Key::Text("Extra context".into()).into());
            rt.frame(0.0);
            rt.update(|s, _| {
                s.snapshot_table
                    .select("60000000-0000-0000-0000-000000000001")
            });
            rt.frame(0.0);
            assert!(
                matches!(&rt.state.snapshot_dialog,Some(Dialog::Edit{source}) if source.snapshot=="Before upgrade")
            );
            assert!(rt.state.snapshot_description.text.contains("Extra context"));
            rt.update(|_, tx| tx.scroll_into_view("confirm-snapshot"));
            rt.frame(0.0);
            click(&mut rt, "confirm-snapshot");
            assert!(rt.state.operations.is_empty());
        }
    }
    #[test]
    fn checkpoint_restore_offers_resume_and_retains_saved_memory() {
        let mut state = snapshot_state();
        let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
        let id = SnapshotId::parse("60000000-0000-0000-0000-000000000000").unwrap();
        let mut snap = inventory.snapshots.get(&id).unwrap().clone();
        snap.power_state = PowerState::Suspended;
        inventory.snapshots.insert(snap);
        refresh(&mut state);
        let mut rt = runtime(state, (513.0, 652.0));
        click(&mut rt, "restore-snapshot");
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Resume restored memory after restore")
        );
        assert!(matches!(
            &rt.state.snapshot_dialog,
            Some(Dialog::Restore {
                checkpoint: true,
                start_after: false,
                ..
            })
        ));
        click(&mut rt, "restore-start");
        assert!(matches!(
            &rt.state.snapshot_dialog,
            Some(Dialog::Restore {
                start_after: true,
                ..
            })
        ));
    }
    #[test]
    fn cloning_reviews_the_captured_snapshot_in_both_modes_and_stays_safe_in_demo() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            for as_template in [false, true] {
                let mut rt = runtime(snapshot_state(), size);
                click(
                    &mut rt,
                    if as_template {
                        "clone-snapshot-template"
                    } else {
                        "clone-snapshot-vm"
                    },
                );
                assert!(
                    matches!(rt.state.snapshot_dialog, Some(Dialog::Clone { as_template: mode, .. }) if mode == as_template)
                );
                click(&mut rt, "snapshot-clone-name");
                rt.key(rune::Key::Text("Recovery copy".into()).into());
                rt.frame(0.0);
                rt.state
                    .snapshot_table
                    .select("60000000-0000-0000-0000-000000000001");
                rt.frame(0.0);
                assert!(
                    matches!(&rt.state.snapshot_dialog, Some(Dialog::Clone { source, .. }) if source.snapshot == "Before upgrade")
                );
                let accessibility = rt.accessibility_json().to_string();
                assert!(accessibility.contains(if as_template {
                    "Create template from snapshot"
                } else {
                    "Create VM from snapshot"
                }));
                assert!(accessibility.contains("Recovery copy"));
                click(&mut rt, "confirm-snapshot");
                assert!(rt.state.operations.is_empty());
                assert!(rt.state.snapshot_dialog.is_some());
            }
        }
    }
    #[test]
    fn deletion_reviews_only_captured_owned_disks_and_lists_retained_disks() {
        let mut state = snapshot_state();
        let snapshot = SnapshotId::parse("60000000-0000-0000-0000-000000000000").unwrap();
        let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
        let prototype_disk = inventory.details.vdis.values().next().unwrap().clone();
        let prototype_attachment = inventory
            .details
            .vbds
            .values()
            .find(|vbd| vbd.kind == "Disk")
            .unwrap()
            .clone();
        let owned_id = VdiId::parse("70000000-0000-0000-0000-000000000001").unwrap();
        for (i, shared) in [(1, false), (2, true)] {
            let id = VdiId::parse(&format!("70000000-0000-0000-0000-{i:012}")).unwrap();
            let mut disk = prototype_disk.clone();
            disk.id = id;
            disk.name = if shared {
                "Retained shared disk"
            } else {
                "Owned recovery disk"
            }
            .into();
            disk.is_snapshot = Some(true);
            disk.sharable = Some(shared);
            disk.missing = Some(false);
            inventory.details.vdis.insert(id, disk);
            let mut attachment = prototype_attachment.clone();
            attachment.id =
                xenadmin_core::VbdId::parse(&format!("80000000-0000-0000-0000-{i:012}")).unwrap();
            attachment.vm = None;
            attachment.snapshot = Some(snapshot);
            attachment.vdi = Some(id);
            attachment.owned = true;
            inventory
                .details
                .vdi_attachments
                .insert(id, vec![attachment.id]);
            inventory.details.vbds.insert(attachment.id, attachment);
        }
        let mut rt = runtime(state, (1440.0, 900.0));
        click(&mut rt, "delete-snapshot");
        assert!(
            matches!(&rt.state.snapshot_dialog, Some(Dialog::Delete { disks, retained, delete_disks: false, .. })
            if disks.len() == 1 && disks[0].id == owned_id && retained.len() == 1)
        );
        let tree = rt.accessibility_json().to_string();
        assert!(tree.contains("Owned recovery disk"));
        assert!(tree.contains("Retained shared disk"));
        click(&mut rt, "snapshot-delete-disks");
        assert!(matches!(
            &rt.state.snapshot_dialog,
            Some(Dialog::Delete {
                delete_disks: true,
                ..
            })
        ));
        rt.state
            .snapshot_table
            .select("60000000-0000-0000-0000-000000000001");
        assert!(
            matches!(&rt.state.snapshot_dialog, Some(Dialog::Delete { source, disks, .. })
            if source.snapshot_id == snapshot && disks.len() == 1 && disks[0].id == owned_id)
        );
        rt.update(|state, _| {
            Arc::make_mut(state.inventory.as_mut().unwrap())
                .details
                .vdis
                .get_mut(&owned_id)
                .unwrap()
                .sharable = Some(true);
        });
        rt.frame(0.0);
        rt.wheel(720.0, 450.0, -600.0);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Disk ownership changed after this review")
        );
        click(&mut rt, "confirm-snapshot");
        assert!(rt.state.operations.is_empty());
        assert!(rt.state.snapshot_dialog.is_some());
    }
    #[test]
    fn deletion_does_not_offer_cleanup_without_complete_disk_inventory() {
        let mut state = snapshot_state();
        Arc::make_mut(state.inventory.as_mut().unwrap())
            .details
            .classes
            .insert("vdi".into(), ClassAvailability::PermissionDenied);
        begin_delete(&mut state);
        let mut rt = runtime(state, (513.0, 652.0));
        assert!(rt.find("snapshot-delete-disks").is_none());
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("disk cleanup is disabled")
        );
        submit(&mut rt.state);
        assert!(rt.state.operations.is_empty());
    }
    #[test]
    fn explicit_checkpoint_block_overrides_advertised_memory_mode() {
        let mut state = snapshot_state();
        let vm = state.snapshot_view.unwrap();
        let details = Arc::make_mut(state.inventory.as_mut().unwrap())
            .details
            .vms
            .get_mut(&vm)
            .unwrap();
        details.allowed_operations = Some(vec!["checkpoint".into()]);
        assert!(memory_available(&state, vm));
        Arc::make_mut(state.inventory.as_mut().unwrap())
            .details
            .vms
            .get_mut(&vm)
            .unwrap()
            .blocked_operations
            .insert("checkpoint".into(), "Protected".into());
        assert!(!memory_available(&state, vm));
    }

    #[test]
    fn snapshot_review_requires_the_same_pool_parent_state_and_exact_capability() {
        let mut state = snapshot_state();
        let source = SnapshotReview::capture(&state).unwrap();
        assert!(!source.permits(&state, SnapshotAction::Clone));
        let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
        let mut snapshot = inventory
            .snapshots
            .get(&source.snapshot_id)
            .unwrap()
            .clone();
        snapshot.allowed_actions.push(SnapshotAction::Clone);
        inventory.snapshots.insert(snapshot.clone());
        assert!(source.permits(&state, SnapshotAction::Clone));
        assert!(!source.permits(&state, SnapshotAction::Delete));
        let valid_snapshot = snapshot.clone();
        snapshot.vm = Some(VmId::parse("30000000-0000-0000-0000-000000000001").unwrap());
        Arc::make_mut(state.inventory.as_mut().unwrap())
            .snapshots
            .insert(snapshot.clone());
        assert!(!source.permits(&state, SnapshotAction::Clone));
        snapshot = valid_snapshot;
        snapshot.power_state = PowerState::Suspended;
        Arc::make_mut(state.inventory.as_mut().unwrap())
            .snapshots
            .insert(snapshot);
        assert!(!source.permits(&state, SnapshotAction::Clone));
        let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
        let mut snapshot = inventory
            .snapshots
            .get(&source.snapshot_id)
            .unwrap()
            .clone();
        snapshot.power_state = PowerState::Halted;
        inventory.snapshots.insert(snapshot);
        inventory.pool_id = PoolId::parse("20000000-0000-0000-0000-000000000099").unwrap();
        assert!(!source.permits(&state, SnapshotAction::Clone));
        assert!(!source.live(&state));
    }

    #[test]
    fn snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut state = init();
            demo(&mut state);
            let vm = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
            state.table.select(vm.to_string());
            let mut rt = rune::Runtime::new(
                state,
                super::super::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            if size.0 < 1200.0 {
                let (x, y) = rt.find("show-details").unwrap();
                rt.pointer_down(x, y);
                rt.pointer_up(x, y);
                rt.frame(0.0);
                for _ in 0..30 {
                    rt.frame(1.0 / 60.0);
                }
            }
            let (x, y) = rt.find("open-snapshots").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert_eq!(rt.state.snapshot_view, Some(vm));
            assert_eq!(rt.state.snapshot_rows.len(), 3);
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let (x, y) = rt.find("create-snapshot").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.find("snapshot-dialog").is_some());
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let (x, y) = rt.find("snapshot-name").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            rt.key(rune::Key::Text("Before upgrade".into()).into());
            rt.frame(0.0);
            let (x, y) = rt.find("confirm-snapshot").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.operations.is_empty());
            assert!(
                rt.state.snapshot_dialog.is_some(),
                "demo confirmation stays disabled"
            );
            let (x, y) = rt.find("cancel-snapshot").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            assert!(rt.state.snapshot_dialog.is_none());
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let (x, y) = rt.find("60000000-0000-0000-0000-000000000000").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            let (x, y) = rt.find("restore-snapshot").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
            let tree = rt.accessibility_json().to_string();
            assert!(
                matches!(rt.state.snapshot_dialog, Some(Dialog::Restore { .. })),
                "restore review was not opened"
            );
            assert!(tree.contains("Before upgrade"));
            assert!(tree.contains("Changes made since the snapshot will be lost"));
            // Changing the selection after opening review must not retarget the command.
            rt.state
                .snapshot_table
                .select(rt.state.snapshot_rows[1].id.to_string());
            rt.frame(0.0);
            assert!(
                matches!(rt.state.snapshot_dialog,Some(Dialog::Restore{snapshot:ref name,..}) if name=="Before upgrade")
            );
        }
    }
}
