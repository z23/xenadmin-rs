use super::*;
use std::{path::PathBuf, sync::mpsc};
use xenadmin_core::{
    BatchVmWorkflow, ObservedResource, OvfPackageFormat, ResourceTarget, TransferAction,
    TransferFile, VmExportFormat, VmWorkflow, VmWorkflowBatch, XvaCompression,
};

pub(super) struct Editor {
    id: uuid::Uuid,
    pool: PoolId,
    pool_name: String,
    observed: Option<ObservedResource>,
    targets: Vec<Vm>,
    format: VmExportFormat,
    files: Vec<Option<(PathBuf, TransferFile)>>,
    pending: Option<usize>,
    result: Option<mpsc::Receiver<Result<(PathBuf, TransferFile), String>>>,
    review: Option<Review>,
    error: Option<String>,
}
#[derive(Clone, PartialEq)]
struct Review {
    batch: VmWorkflowBatch,
    outputs: Vec<(VmId, TransferFile)>,
    paths: Vec<String>,
}
fn selected(s: &State) -> Result<Vec<Vm>, String> {
    if !matches!(s.page, Page::Guests | Page::Templates) {
        return Err("Select ordinary VMs or custom templates".into());
    }
    let i = s
        .inventory
        .as_ref()
        .ok_or("Current inventory is unavailable")?;
    let rows = s
        .rows
        .iter()
        .filter(|r| s.table.is_selected(&r.id))
        .collect::<Vec<_>>();
    if !(1..=32).contains(&rows.len()) {
        return Err("Select 1–32 visible VMs or custom templates".into());
    }
    rows.into_iter()
        .map(|r| {
            r.vm.and_then(|id| i.vms.get(&id))
                .filter(|v| !v.is_control_domain)
                .cloned()
                .ok_or_else(|| {
                    "A selected row is no longer an ordinary VM or custom template".into()
                })
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
    let i = s.inventory.as_ref().unwrap();
    s.bulk_export = Some(Editor {
        id: uuid::Uuid::new_v4(),
        pool: i.pool_id,
        pool_name: i.pool_name.clone(),
        observed: s
            .snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(i.pool_id)).ok()),
        files: vec![None; targets.len()],
        targets,
        format: VmExportFormat::Xva {
            compression: XvaCompression::Uncompressed,
        },
        pending: None,
        result: None,
        review: None,
        error: None,
    });
}
fn edit(s: &mut State) -> &mut Editor {
    s.bulk_export.as_mut().expect("export editor")
}
fn current(s: &State, e: &Editor) -> bool {
    !s.demo
        && s.connection.is_some()
        && e.observed.is_some()
        && s.snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(e.pool)).ok())
            == e.observed
}
fn action(vm: &Vm, format: &VmExportFormat) -> TransferAction {
    let source = xenadmin_core::ExportSource::Vm {
        vm: vm.id,
        expected_template: vm.is_template,
    };
    match format {
        VmExportFormat::Xva { compression } => TransferAction::ExportXva {
            source,
            compression: *compression,
        },
        VmExportFormat::Ovf { format } => TransferAction::ExportOvf {
            source,
            format: *format,
        },
    }
}
fn proposal(s: &State, e: &Editor) -> Result<Review, String> {
    let i = s
        .inventory
        .as_ref()
        .filter(|i| i.pool_id == e.pool)
        .ok_or("Captured pool is no longer current")?;
    let mut items = Vec::new();
    let mut outputs = Vec::new();
    let mut paths = Vec::new();
    for (index, captured) in e.targets.iter().enumerate() {
        let v = i
            .vms
            .get(&captured.id)
            .filter(|v| {
                !v.is_control_domain
                    && v.is_template == captured.is_template
                    && v.power_state == captured.power_state
            })
            .ok_or_else(|| {
                format!(
                    "{} ({}): identity or power state changed",
                    captured.name, captured.id
                )
            })?;
        let status = action(v, &e.format).availability(i);
        if !status.is_available() {
            return Err(format!("{} ({}): {}", v.name, v.id, status.explanation()));
        }
        let (path, file) = e.files[index]
            .as_ref()
            .ok_or_else(|| format!("Choose the output for {} ({})", v.name, v.id))?;
        if paths
            .iter()
            .any(|p: &String| PathBuf::from(p).starts_with(path) || path.starts_with(p))
        {
            return Err(
                "Output paths overlap; choose a distinct new file or directory for every target"
                    .into(),
            );
        }
        paths.push(path.to_string_lossy().into_owned());
        outputs.push((v.id, file.clone()));
        items.push(BatchVmWorkflow {
            vm: v.id,
            expected_state: v.power_state.clone(),
            workflow: VmWorkflow::Export {
                expected_template: v.is_template,
                format: e.format.clone(),
            },
        });
    }
    let batch = VmWorkflowBatch { items };
    let permission = session_permissions_view::assess(s, &batch.permission_requirements());
    if permission.decision().is_denied() {
        return Err(permission.explanation());
    }
    Ok(Review {
        batch,
        outputs,
        paths,
    })
}
fn change_format(s: &mut State, format: VmExportFormat) {
    let e = edit(s);
    e.id = uuid::Uuid::new_v4();
    e.format = format;
    e.files.fill(None);
    e.pending = None;
    e.result = None;
    e.review = None;
    e.error = None;
}
fn choose(s: &mut State, tx: &mut Tx, index: usize) {
    let e = edit(s);
    if e.pending.is_some() {
        return;
    }
    let id = e.id;
    let format = e.format.clone();
    let vm = e.targets[index].id;
    e.pending = Some(index);
    e.error = None;
    e.review = None;
    e.files[index] = None;
    let extension = match &format {
        VmExportFormat::Xva { compression } => compression.extension(),
        VmExportFormat::Ovf {
            format: OvfPackageFormat::Ova,
        } => "ova",
        VmExportFormat::Ovf { .. } => "ovf-package",
    };
    tx.save_file(
        rune::FileDialogOptions {
            title: "Choose a new export file or package directory".into(),
            file_name: Some(format!("{vm}.{extension}")),
            ..Default::default()
        },
        move |s: &mut State, result, _| {
            let Some(e) = s.bulk_export.as_mut().filter(|e| {
                e.id == id
                    && e.format == format
                    && e.targets.get(index).is_some_and(|v| v.id == vm)
                    && e.pending == Some(index)
            }) else {
                return;
            };
            match result {
                Ok(Some(path)) => {
                    let (tx, rx) = mpsc::channel();
                    let wake = s.waker.clone();
                    let job = std::thread::Builder::new()
                        .name("batch-export-destination".into())
                        .spawn(move || {
                            let value = TransferFile::destination(&path)
                                .map(|file| (path, file))
                                .map_err(|e| e.to_string());
                            let _ = tx.send(value);
                            if let Some(w) = wake {
                                w()
                            }
                        });
                    if job.is_ok() {
                        e.result = Some(rx)
                    } else {
                        e.pending = None;
                        e.error = Some("Cannot inspect the selected destination".into())
                    }
                }
                Ok(None) => e.pending = None,
                Err(error) => {
                    e.pending = None;
                    e.error = Some(error.to_string())
                }
            }
        },
    );
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(e) = &mut s.bulk_export else {
        return false;
    };
    let Some(rx) = &e.result else { return false };
    let result = match rx.try_recv() {
        Ok(v) => v,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("Destination inspection stopped".into()),
    };
    e.result = None;
    let Some(index) = e.pending.take() else {
        return true;
    };
    match result {
        Ok(file) => e.files[index] = Some(file),
        Err(error) => e.error = Some(error),
    }
    true
}
fn review(s: &mut State) {
    let Some(e) = &s.bulk_export else { return };
    match proposal(s, e) {
        Ok(r) => {
            edit(s).review = Some(r);
            edit(s).error = None
        }
        Err(e) => edit(s).error = Some(e),
    }
}
fn submit(s: &mut State) {
    let result = (|| {
        let e = s.bulk_export.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let r = e.review.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, e)
            || proposal(s, e).ok().as_ref() != Some(r)
            || s.resource_operations.len() >= 64
        {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_vm_workflow_batch(
                xenadmin_core::OperationId::new(),
                e.observed.unwrap(),
                r.batch.clone(),
                r.outputs.clone(),
            )
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone())
            }
            s.resource_operations.push(op);
            s.bulk_export = None;
            resource_admin::open_pool(s, resource_admin::Kind::History)
        }
        Err(error) => edit(s).error = Some(error.to_string()),
    }
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    Button::new("Export selection…")
        .variant(ButtonVariant::Ghost)
        .disabled(selected(s).is_err())
        .on_press(open)
        .build(k)
        .key("batch-export-open")
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.bulk_export.as_ref().unwrap();
    let proposed = proposal(s, e);
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(10.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            caption(
                format!(
                    "{} · {}\n{} captured targets",
                    e.pool_name,
                    e.pool,
                    e.targets.len()
                ),
                k,
            )
            .wrap_anywhere(),
        );
    if e.review.is_none() {
        let mut choices = row().w_full().flex_wrap().gap(6.0);
        for (index, label, format) in formats() {
            let selected = e.format == format;
            choices = choices.child(
                Button::new(label)
                    .variant(if selected {
                        ButtonVariant::Outline
                    } else {
                        ButtonVariant::Ghost
                    })
                    .disabled(e.pending.is_some())
                    .on_press(move |s: &mut State| change_format(s, format.clone()))
                    .build(k)
                    .key(format!("batch-export-format-{index}")),
            );
        }
        body = body.child(choices);
    }
    for (index, vm) in e.targets.iter().enumerate() {
        body = body
            .child(heading(format!("{} · {}", index + 1, vm.name), k))
            .child(
                caption(
                    format!(
                        "{} · {:?}{}",
                        vm.id,
                        vm.power_state,
                        if vm.is_template { " · template" } else { "" }
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
        if let Some((path, _)) = &e.files[index] {
            body = body.child(caption(format!("Output: {}", path.display()), k).wrap_anywhere());
        }
        if e.review.is_none() {
            body = body.child(
                Button::new(if e.files[index].is_some() {
                    "Choose another destination"
                } else {
                    "Choose destination"
                })
                .disabled(e.pending.is_some())
                .on_press_tx(move |s: &mut State, tx: &mut Tx| choose(s, tx, index))
                .build(k)
                .key(format!("batch-export-choose-{index}")),
            );
        }
    }
    body=body.child(caption("Every source and destination is checked before the first transfer. Exports run serially without stopping or changing guests. Stop finishes the current archive and skips the remainder. Recovery reads existing evidence; it never starts another transfer or publishes a new output. Existing files and directories are never overwritten.",k).wrap());
    body=body.child(caption("XVA preserves native guest configuration and disks; full XVA does not include snapshot ancestry. OVF/OVA uses the supported ordinary HVM subset. Memory checkpoints and snapshots are exported separately. Read-only connections may export, subject to host permission.",k).wrap());
    if let Some(r) = &e.review {
        body = body
            .child(caption(format!("Format: {}", format_name(&e.format)), k).wrap())
            .child(session_permissions_view::review_notice(
                &session_permissions_view::assess(s, &r.batch.permission_requirements()),
                k,
            ));
    }
    if let Err(reason) = &proposed {
        body = body.child(
            caption(reason, k)
                .wrap()
                .role("alert")
                .color(k.c.danger.icon),
        );
    }
    if let Some(error) = &e.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .role("alert")
                .color(k.c.danger.icon),
        );
    }
    if e.pending.is_some() {
        body = body.child(caption("Waiting for destination selection or inspection…", k).wrap());
    }
    if s.demo {
        body = body.child(caption("Synthetic preview; transfers are disabled.", k).wrap());
    } else if !current(s, e) {
        body = body.child(
            caption(
                "The source connection changed. Reopen this review from current inventory.",
                k,
            )
            .wrap(),
        );
    }
    let mut footer = row().w_full().flex_wrap().gap(8.0).child(
        Button::new("Cancel")
            .on_press(|s: &mut State| s.bulk_export = None)
            .build(k)
            .key("batch-export-close"),
    );
    if let Some(r) = &e.review {
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| edit(s).review = None)
                    .build(k)
                    .key("batch-export-back"),
            )
            .child(
                Button::new("Export reviewed VMs")
                    .disabled(
                        !current(s, e)
                            || proposed.as_ref().ok() != Some(r)
                            || s.resource_operations.len() >= 64,
                    )
                    .on_press(submit)
                    .build(k)
                    .key("batch-export-submit"),
            );
    } else {
        footer = footer.child(
            Button::new("Review exports")
                .disabled(proposed.is_err() || e.pending.is_some())
                .on_press(review)
                .build(k)
                .key("batch-export-review"),
        );
    }
    let panel = col()
        .w((s.window_width - 24.0).clamp(280.0, 840.0))
        .h((s.window_height - 24.0).clamp(300.0, 900.0))
        .padding(16.0)
        .gap(10.0)
        .fill(k.c.bg_base)
        .radius(12.0)
        .child(heading(
            if e.review.is_some() {
                "Review selected VM exports"
            } else {
                "Export selected VMs"
            },
            k,
        ))
        .child(body)
        .child(footer);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Selected VM exports")
        .modal(false, |s: &mut State| s.bulk_export = None)
        .child(panel)
}
fn formats() -> Vec<(usize, &'static str, VmExportFormat)> {
    vec![
        (
            0,
            "XVA",
            VmExportFormat::Xva {
                compression: XvaCompression::Uncompressed,
            },
        ),
        (
            1,
            "XVA gzip",
            VmExportFormat::Xva {
                compression: XvaCompression::Gzip,
            },
        ),
        (
            2,
            "XVA zstd",
            VmExportFormat::Xva {
                compression: XvaCompression::Zstd,
            },
        ),
        (
            3,
            "OVF directory",
            VmExportFormat::Ovf {
                format: OvfPackageFormat::Ovf,
            },
        ),
        (
            4,
            "OVA",
            VmExportFormat::Ovf {
                format: OvfPackageFormat::Ova,
            },
        ),
    ]
}
fn format_name(format: &VmExportFormat) -> &'static str {
    formats()
        .into_iter()
        .find(|(_, _, f)| f == format)
        .unwrap()
        .1
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let ids = s
        .inventory
        .as_ref()
        .unwrap()
        .vms
        .keys()
        .copied()
        .take(2)
        .collect::<Vec<_>>();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    for id in &ids {
        let vm = i.vms.get_mut(id).unwrap();
        vm.power_state = PowerState::Halted;
        vm.is_template = false;
        let d = i.details.vms.entry(*id).or_default();
        d.allowed_operations = Some(vec!["export".into()]);
        d.current_operations.clear();
        d.blocked_operations.clear();
    }
    s.page = Page::Guests;
    s.rebuild();
    s.table.select(ids[0].to_string());
    open(s);
    let second = s.inventory.as_ref().unwrap().vms[&ids[1]].clone();
    let e = edit(s);
    e.targets.push(second);
    e.files.push(None);
    let e = edit(s);
    for (index, vm) in e.targets.iter().enumerate() {
        let path = std::env::temp_dir().join(format!("xenadmin-demo-{}-{}.xva", e.id, vm.id));
        e.files[index] = Some((path.clone(), TransferFile::destination(path).unwrap()));
    }
    review(s);
}

#[cfg(test)]
#[path = "bulk_export_tests.rs"]
mod tests;
