use super::*;
use xenadmin_core::{ObservedSnapshot, ObservedVm, SnapshotAction, SnapshotName, VmCommand};

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
    },
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
}
pub(super) fn open(s: &mut State) {
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
        });
        Ok::<_, xenadmin_core::Error>(())
    })();
    s.error = result.err().map(|error| error.to_string());
}
fn submit(s: &mut State) {
    let command = match &s.snapshot_dialog {
        Some(Dialog::Create {
            target: Target::Live(vm),
            ..
        }) => match SnapshotName::new(&s.snapshot_name.text) {
            Ok(name) => {
                if s.snapshot_memory {
                    VmCommand::CreateCheckpoint { vm: *vm, name }
                } else {
                    VmCommand::CreateSnapshot { vm: *vm, name }
                }
            }
            Err(error) => {
                s.error = Some(error.to_string());
                return;
            }
        },
        Some(Dialog::Restore {
            target: Target::Live(snapshot),
            ..
        }) => VmCommand::RevertSnapshot {
            snapshot: *snapshot,
        },
        _ => return,
    };
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
    let selected = s
        .snapshot_table
        .selected()
        .next()
        .and_then(|id| s.snapshot_rows.iter().find(|r| r.id.to_string() == id));
    let can_create =
        current(s) && vm.is_some_and(|vm| vm.can_snapshot || memory_available(s, vm.id));
    let can_restore = current(s)
        && vm.is_some_and(|vm| vm.power_state == PowerState::Halted)
        && selected
            .is_some_and(|snapshot| snapshot.allowed_actions.contains(&SnapshotAction::Revert));
    let mut content = col().w_full().h_full().min_h(0.0).padding(20.0).gap(12.0)
        .child(row().w_full().justify_between().center_y().child(heading("Snapshots",k)).child(Button::new("Back to inventory").variant(ButtonVariant::Ghost).on_press(|s:&mut State|s.snapshot_view=None).build(k).key("back-from-snapshots")))
        .child(heading(vm.map(|vm|vm.name.as_str()).unwrap_or("VM unavailable"),k))
        .child(caption("Disk snapshots capture VM configuration and virtual disks. Choose Include guest memory for a memory checkpoint when supported. Application quiescing is not available on XCP-ng 8.1 and later.",k).wrap())
        .child(row().gap(8.0)
            .child(Button::new("Create snapshot").variant(ButtonVariant::Outline).disabled(!s.demo && !can_create).on_press(begin_create).build(k).key("create-snapshot"))
            .child(Button::new("Restore snapshot").variant(ButtonVariant::Outline).disabled(selected.is_none() || (!s.demo && !can_restore)).on_press(begin_restore).build(k).key("restore-snapshot")));
    if s.demo {
        content = content.child(
            caption(
                "DEMO • Synthetic snapshots. Confirming host actions is disabled.",
                k,
            )
            .wrap(),
        );
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
        content = content.child(caption("Shut down the VM before restoring a snapshot.", k).wrap());
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
        content = content.child(
            table
                .cell(|s: &State, i, column| {
                    let snapshot = &s.snapshot_rows[i];
                    if column == "name" {
                        Cell::text(&snapshot.name)
                    } else {
                        Cell::text(snapshot.created_at.as_deref().unwrap_or("Unknown")).muted()
                    }
                })
                .zebra(true)
                .build(s, k)
                .grow(1.0)
                .min_h(0.0),
        );
    }
    if let Some(selected) = selected {
        content = content.child(
            caption(
                format!(
                    "{} • {}\nUUID: {}",
                    selected.name,
                    selected.created_at.as_deref().unwrap_or("Unknown date"),
                    selected.id
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
    }
    content
}

fn memory_available(s: &State, id: VmId) -> bool {
    s.inventory
        .as_ref()
        .and_then(|i| i.details.vms.get(&id))
        .and_then(|v| v.allowed_operations.as_ref())
        .is_some_and(|ops| ops.iter().any(|o| o == "checkpoint"))
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(dialog) = &s.snapshot_dialog else {
        return el();
    };
    let mut body = col()
        .w((s.window_width - 40.0).clamp(280.0, 480.0))
        .max_h(600.0)
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
                .child(TextInput::new(&s.snapshot_name).label("Snapshot name").placeholder("Snapshot name").on_edit(|s:&mut State|&mut s.snapshot_name).build(k).key("snapshot-name"));
            let selected_mode_available = if s.snapshot_memory {
                memory_available(s, *id)
            } else {
                s.inventory
                    .as_ref()
                    .and_then(|i| i.vms.get(id))
                    .is_some_and(|vm| vm.can_snapshot)
            };
            let valid = SnapshotName::new(&s.snapshot_name.text).is_ok();
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
                    caption(xenadmin_core::Error::InvalidSnapshotName.to_string(), k)
                        .wrap()
                        .role("alert"),
                );
            }
            (
                "Create disk snapshot",
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
        } => {
            body=body.child(heading("Restore this snapshot?",k))
                .child(caption(format!("VM: {vm}\nVM UUID: {id}\nSnapshot: {snapshot}\nSnapshot UUID: {snapshot_id}"),k).wrap_anywhere())
                .child(caption("This replaces the VM's configuration and disk contents. Changes made since the snapshot will be lost. The VM must be shut down, and it will not start automatically after restore.",k).wrap());
            (
                "Restore snapshot",
                "Restore snapshot",
                s.inventory.as_ref().is_some_and(|i| {
                    i.vms
                        .get(id)
                        .is_some_and(|vm| vm.power_state == PowerState::Halted)
                        && i.snapshots.get(snapshot_id).is_some_and(|snap| {
                            snap.allowed_actions.contains(&SnapshotAction::Revert)
                        })
                }),
                matches!(target,Target::Live(observed) if s.snapshot.as_ref().and_then(|s|s.observe_snapshot(*id,*snapshot_id).ok())==Some(*observed)),
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
                    .on_press(|s: &mut State| s.snapshot_dialog = None)
                    .build(k)
                    .key("cancel-snapshot"),
            )
            .child(
                Button::new(confirm)
                    .variant(if matches!(dialog, Dialog::Restore { .. }) {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Outline
                    })
                    .disabled(!valid || !live || !current(s))
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
        .modal(false, |s: &mut State| s.snapshot_dialog = None)
        .child(body)
}

#[cfg(test)]
mod tests {
    use super::*;
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
