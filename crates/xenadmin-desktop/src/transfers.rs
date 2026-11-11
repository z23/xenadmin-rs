use super::*;
#[cfg(test)]
#[path = "checkpoint_transfer_tests.rs"]
mod checkpoint_tests;
#[cfg(test)]
#[path = "compression_transfer_tests.rs"]
mod compression_tests;
#[cfg(test)]
#[path = "ovf_transfer_tests.rs"]
mod ovf_tests;
#[path = "ovf_transfer_ui.rs"]
mod ovf_ui;

use std::sync::mpsc;
use xenadmin_core::{
    DiskImageFormat, ExportSource, ObservedResource, OperationControl, OperationId, OperationState,
    OvfPackageFormat, ResourceAction, ResourceTarget, SrId, TransferAction, TransferFile,
    TransferRequest, VdiId,
};
#[derive(Default)]
pub(super) struct Panel {
    pub show: bool,
    serial: u64,
    source: Option<ExportSource>,
    source_name: String,
    disk_export: Option<(VdiId, SrId, u64)>,
    disk_picker: bool,
    ovf_import: bool,
    package_export: Option<OvfPackageFormat>,
    compression: xenadmin_core::XvaCompression,
    ovf_preview: Option<xenadmin_core::OvfSummary>,
    networks: std::collections::BTreeMap<String, xenadmin_core::NetworkId>,
    inspection_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    format: Option<DiskImageFormat>,
    disk_name: TextEdit,
    pool: Option<ObservedResource>,
    pool_id: Option<PoolId>,
    sr: Option<SrId>,
    file: Option<TransferFile>,
    pending: bool,
    result: Option<mpsc::Receiver<Result<TransferFile, String>>>,
    error: Option<String>,
    review: bool,
    history: bool,
    acknowledge: Option<OperationId>,
}
fn close(s: &mut State) {
    cancel_inspection(s);
    s.transfers.show = false;
    s.transfers.serial += 1;
    s.transfers.pending = false;
    s.transfers.result = None;
}
fn begin(s: &mut State, source: Option<ExportSource>) {
    cancel_inspection(s);
    let serial = s.transfers.serial + 1;
    let pool_id = s.inventory.as_ref().map(|i| i.pool_id);
    let pool = pool_id.and_then(|id| {
        s.snapshot
            .as_ref()?
            .observe_resource(ResourceTarget::Pool(id))
            .ok()
    });
    let source_name = source
        .as_ref()
        .and_then(|source| {
            let i = s.inventory.as_ref()?;
            match source {
                ExportSource::Vm { vm, .. } => i.vms.get(vm).map(|v| v.name.clone()),
                ExportSource::Snapshot { snapshot, .. }
                | ExportSource::Checkpoint { snapshot, .. } => {
                    i.snapshots.get(snapshot).map(|v| v.name.clone())
                }
            }
        })
        .unwrap_or_default();
    s.transfers = Panel {
        show: true,
        serial,
        source,
        source_name,
        pool,
        pool_id,
        ..Default::default()
    };
}
pub(super) fn button(s: &State, id: VmId, k: &Kit) -> El {
    let Some(vm) = s
        .inventory
        .as_ref()
        .and_then(|i| i.vms.get(&id))
        .filter(|v| !v.is_control_domain)
    else {
        return el();
    };
    let source = ExportSource::Vm {
        vm: id,
        expected_template: vm.is_template,
    };
    export_button(s, source, k)
}
pub(super) fn export_button(s: &State, source: ExportSource, k: &Kit) -> El {
    let action = TransferAction::ExportXva {
        compression: Default::default(),
        source: source.clone(),
    };
    let available = s
        .inventory
        .as_ref()
        .is_some_and(|i| action.availability(i).is_available());
    Button::new("Export…")
        .disabled(!s.demo && !available)
        .on_press(move |s: &mut State| begin(s, Some(source.clone())))
        .build(k)
        .key("export-xva")
}
pub(super) fn history(s: &mut State) {
    begin(s, None);
    s.transfers.history = true;
}
pub(super) fn open(s: &mut State) {
    begin(s, None);
}
pub(super) fn import_converted(s: &mut State, path: std::path::PathBuf, format: DiskImageFormat) {
    begin(s, None);
    s.transfers.format = Some(format);
    s.transfers.disk_name = TextEdit::new("Converted disk");
    s.transfers.pending = true;
    let (sender, receiver) = mpsc::channel();
    let wake = s.waker.clone();
    match std::thread::Builder::new()
        .name("converted-disk-review".into())
        .spawn(move || {
            let result = TransferFile::disk_source(path, format).map_err(|e| e.to_string());
            let _ = sender.send(result);
            if let Some(wake) = wake {
                wake();
            }
        }) {
        Ok(_) => s.transfers.result = Some(receiver),
        Err(_) => {
            s.transfers.pending = false;
            s.transfers.error = Some("Cannot inspect converted disk".into());
        }
    }
}
fn current(s: &State, p: &Panel) -> bool {
    s.inventory
        .as_ref()
        .is_some_and(|i| Some(i.pool_id) == p.pool_id)
        && (s.demo
            || p.pool.is_some_and(|p| {
                s.snapshot
                    .as_ref()
                    .and_then(|v| v.observe_resource(p.target()).ok())
                    == Some(p)
            }))
}
fn exporting(p: &Panel) -> bool {
    p.source.is_some() || p.disk_export.is_some()
}
fn action(p: &Panel) -> Option<TransferAction> {
    if let Some((vdi, expected_sr, expected_size)) = p.disk_export {
        return Some(TransferAction::ExportDisk {
            vdi,
            expected_sr,
            expected_size,
            format: p.format.unwrap_or(DiskImageFormat::Vhd),
        });
    }
    if let Some(source) = p.source.clone() {
        return Some(match p.package_export {
            Some(format) => TransferAction::ExportOvf { source, format },
            None => TransferAction::ExportXva {
                source,
                compression: p.compression,
            },
        });
    }
    if p.ovf_import {
        let summary = p.file.as_ref()?.ovf()?;
        if summary
            .networks
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            != p.networks.keys().cloned().collect()
        {
            return None;
        }
        return p.sr.map(|sr| TransferAction::ImportOvf {
            sr,
            networks: p.networks.clone(),
        });
    }
    p.sr.map(|sr| {
        if let Some(format) = p.format {
            TransferAction::ImportDisk {
                sr,
                name: p.disk_name.text.to_string(),
                format,
            }
        } else {
            if p.file
                .as_ref()
                .and_then(|f| f.archive())
                .is_some_and(|a| a.memory_images() > 0)
            {
                TransferAction::ImportCheckpointXva { sr }
            } else {
                TransferAction::ImportXva { sr }
            }
        }
    })
}
pub(super) fn disk_button(s: &State, disk: &xenadmin_core::VirtualDisk, k: &Kit) -> El {
    let Some(sr) = disk.sr else { return el() };
    let Some(size) = disk.virtual_size else {
        return el();
    };
    let id = disk.id;
    let name = disk.name.clone();
    let Some(inventory) = &s.inventory else {
        return el();
    };
    let available = TransferAction::ExportDisk {
        vdi: id,
        expected_sr: sr,
        expected_size: size,
        format: DiskImageFormat::Raw,
    }
    .availability(inventory)
    .is_available();
    Button::new("Export disk image")
        .disabled(!s.demo && !available)
        .on_press(move |s: &mut State| {
            begin(s, None);
            s.transfers.disk_export = Some((id, sr, size));
            s.transfers.format = Some(if size > 2040 * 1024 * 1024 * 1024 {
                DiskImageFormat::Raw
            } else {
                DiskImageFormat::Vhd
            });
            s.transfers.source_name = name.clone();
        })
        .build(k)
        .key(format!("export-disk-{id}"))
}
fn cancel_inspection(s: &mut State) {
    if let Some(cancel) = s.transfers.inspection_cancel.take() {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
fn set_format(s: &mut State, format: Option<DiskImageFormat>) {
    cancel_inspection(s);
    s.transfers.ovf_import = false;
    s.transfers.package_export = None;
    s.transfers.compression = Default::default();
    s.transfers.networks.clear();
    s.transfers.format = format;
    s.transfers.file = None;
    s.transfers.pending = false;
    s.transfers.result = None;
    s.transfers.review = false;
    s.transfers.serial += 1;
}
fn choose(s: &mut State, tx: &mut Tx) {
    if s.transfers.pending {
        return;
    }
    let serial = s.transfers.serial;
    let export = exporting(&s.transfers);
    let format = s.transfers.format;
    let ovf = s.transfers.ovf_import;
    let package = s.transfers.package_export;
    cancel_inspection(s);
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    s.transfers.inspection_cancel = Some(cancel.clone());
    s.transfers.networks.clear();
    s.transfers.pending = true;
    s.transfers.error = None;
    s.transfers.file = None;
    s.transfers.review = false;
    let callback =
        move |s: &mut State, result: Result<Option<std::path::PathBuf>, String>, _: &mut Tx| {
            if s.transfers.serial != serial || !s.transfers.show {
                return;
            }
            match result {
                Ok(Some(path)) => {
                    let (sender, receiver) = mpsc::channel();
                    let wake = s.waker.clone();
                    let spawned = std::thread::Builder::new()
                        .name("archive-file-review".into())
                        .spawn(move || {
                            let result = if export {
                                TransferFile::destination(path)
                            } else if ovf {
                                TransferFile::ovf_source_with_cancel(path, &cancel)
                            } else if let Some(format) = format {
                                TransferFile::disk_source(path, format)
                            } else {
                                TransferFile::source_with_cancel(path, &cancel)
                            }
                            .map_err(|e| e.to_string());
                            let _ = sender.send(result);
                            if let Some(w) = wake {
                                w();
                            }
                        });
                    if spawned.is_ok() {
                        s.transfers.result = Some(receiver)
                    } else {
                        s.transfers.pending = false;
                        s.transfers.error = Some("Cannot start file inspection".into())
                    }
                }
                Ok(None) => s.transfers.pending = false,
                Err(e) => {
                    s.transfers.pending = false;
                    s.transfers.error = Some(e)
                }
            }
        };
    let folder = package == Some(OvfPackageFormat::Ovf);
    let extension = if package == Some(OvfPackageFormat::Ova) {
        "ova"
    } else {
        format.map_or(
            s.transfers.compression.extension(),
            DiskImageFormat::extension,
        )
    };
    let mut options = rune::FileDialogOptions {
        title: if folder {
            "Choose a new OVF package directory name"
        } else if export {
            "Save a new disk or archive image"
        } else {
            "Select a disk or archive image"
        }
        .into(),
        file_name: export.then(|| {
            if folder {
                "guest-ovf".into()
            } else {
                format!("guest.{extension}")
            }
        }),
        ..Default::default()
    };
    if !folder {
        options = options.filter(
            "Disk or archive image",
            &if ovf {
                vec!["ovf", "ova"]
            } else if !export && format.is_none() {
                vec!["xva", "gz", "zst", "zstd"]
            } else {
                vec![extension]
            },
        );
    }
    if export {
        tx.save_file(options, move |s: &mut State, r, tx| {
            callback(s, r.map_err(|e| e.to_string()), tx)
        });
    } else {
        tx.open_file(options, move |s: &mut State, r, tx| {
            callback(s, r.map_err(|e| e.to_string()), tx)
        });
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(rx) = &s.transfers.result else {
        return false;
    };
    let value = match rx.try_recv() {
        Ok(v) => v,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("File inspection stopped".into()),
    };
    s.transfers.pending = false;
    s.transfers.result = None;
    match value {
        Ok(file) => s.transfers.file = Some(file),
        Err(e) => s.transfers.error = Some(e),
    }
    true
}
fn apply(s: &mut State) {
    let p = &s.transfers;
    let Some(action) = action(p) else { return };
    let Some(pool) = p.pool else { return };
    let Some(file) = p.file.clone() else { return };
    if s.resource_operations.len() >= 64
        || !p.review
        || !current(s, p)
        || s.demo
        || (!action.is_export() && !s.can_mutate())
        || !s
            .inventory
            .as_ref()
            .is_some_and(|i| action.availability(i).is_available())
    {
        return;
    }
    let permission = session_permissions_view::assess(s, &action.permission_requirements());
    if permission.decision().is_denied() {
        s.transfers.error = Some(permission.explanation());
        return;
    }
    let result = s
        .connection
        .as_ref()
        .ok_or(xenadmin_core::Error::Closed)
        .and_then(|c| c.transfer(OperationId::new(), pool, TransferRequest { action, file }));
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone())
            }
            s.resource_operations.push(op);
            s.transfers.history = true;
            s.transfers.review = false;
            s.transfers.file = None;
        }
        Err(e) => s.transfers.error = Some(e.to_string()),
    }
}
fn control(s: &mut State, id: OperationId, action: OperationControl) {
    if let Some(c) = &s.connection
        && let Err(e) = c.control_resource(id, action)
    {
        s.transfers.error = Some(e.to_string())
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let p = &s.transfers;
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
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(heading(
                    if p.history {
                        "Transfers"
                    } else if p.disk_export.is_some() {
                        "Export disk image"
                    } else if exporting(p) {
                        "Export VM or template"
                    } else {
                        "Import image"
                    },
                    k,
                ))
                .child(
                    Button::new("History")
                        .on_press(|s: &mut State| s.transfers.history = true)
                        .build(k),
                )
                .child(
                    Button::new("Close")
                        .variant(ButtonVariant::Outline)
                        .on_press(close)
                        .build(k),
                ),
        );
    if p.history {
        body = body.child(
            Button::new("Import image")
                .disabled(!s.demo && !s.can_mutate())
                .on_press(open)
                .build(k),
        );
        for r in s
            .resource_rows
            .iter()
            .rev()
            .filter(|r| matches!(r.action, ResourceAction::Transfer(_)))
        {
            let id = r.id;
            let allowed = s.connection.as_ref().is_some_and(|c| {
                !c.is_read_only() || matches!(&r.action,ResourceAction::Transfer(a)if a.is_export())
            }) && s.snapshot.as_ref().is_some_and(|v| v.is_current())
                && s.inventory.as_ref().is_some_and(|i| i.pool_id == r.pool);
            let mut card = col()
                .gap(8.0)
                .padding(12.0)
                .child(heading(format!("{} · {:?}", r.action.label(), r.state), k))
                .child(caption(format!("Operation {} · pool {}", r.id, r.pool), k).wrap_anywhere())
                .child(caption(&r.detail, k).wrap());
            if let Some(t) = &r.transfer {
                card = card.child(
                    caption(
                        format!(
                            "{} bytes{} · {}",
                            t.bytes,
                            t.total.map(|n| format!(" of {n}")).unwrap_or_default(),
                            t.phase
                        ),
                        k,
                    )
                    .wrap(),
                );
            }
            for id in &r.transfer_created.vms {
                card = card.child(caption(format!("Retained guest UUID: {id}"), k).wrap_anywhere())
            }
            for id in &r.transfer_created.disks {
                card = card.child(caption(format!("Retained disk UUID: {id}"), k).wrap_anywhere())
            }
            for id in &r.transfer_created.suspend_images {
                card = card.child(
                    caption(format!("Retained guest memory image UUID: {id}"), k).wrap_anywhere(),
                );
            }
            for id in &r.transfer_created.networks {
                card =
                    card.child(caption(format!("Retained network UUID: {id}"), k).wrap_anywhere())
            }
            if let Some(parent) = r.batch_parent {
                card = card.child(caption(format!("Controlled by batch {parent}. Recheck, stop or acknowledge the batch in pool operation history."), k).wrap_anywhere());
            }
            if r.batch_parent.is_none() && !r.state.terminal() {
                card = card.child(
                    row()
                        .gap(8.0)
                        .flex_wrap()
                        .child(
                            Button::new("Recheck")
                                .disabled(!allowed)
                                .on_press(move |s: &mut State| {
                                    control(s, id, OperationControl::Recheck)
                                })
                                .build(k),
                        )
                        .child(
                            Button::new("Stop transfer")
                                .disabled(!allowed || r.cancel_requested)
                                .on_press(move |s: &mut State| {
                                    control(s, id, OperationControl::RequestCancellation)
                                })
                                .build(k),
                        ),
                );
            }
            for id in &r.transfer_created.vbds {
                card = card.child(
                    caption(format!("Retained disk attachment UUID: {id}"), k).wrap_anywhere(),
                );
            }
            for id in &r.transfer_created.vifs {
                card = card.child(caption(format!("Retained NIC UUID: {id}"), k).wrap_anywhere());
            }
            for step in &r.steps {
                card = card.child(caption(format!("{} · {:?}", step.label, step.state), k).wrap());
            }
            if r.batch_parent.is_none() && r.state == OperationState::OutcomeUnknown {
                card=card.child(caption("Stopping the stream does not prove server cleanup. Recheck reads existing evidence and never sends archive bytes again.",k).wrap());
                if p.acknowledge == Some(id) {
                    card=card.child(caption("Acknowledge only after inspecting the pool and any retained resources. This releases the local reservation while leaving the transfer outcome unverified; it does not delete resources or cancel server work.",k).wrap()).child(Button::new("Release reservation").variant(ButtonVariant::Danger).disabled(!allowed).on_press(move|s:&mut State|control(s,id,OperationControl::AcknowledgeUnknown)).build(k).key(format!("transfer-release-{id}")));
                } else {
                    card = card.child(
                        Button::new("Acknowledge after inspection")
                            .disabled(!allowed)
                            .on_press(move |s: &mut State| s.transfers.acknowledge = Some(id))
                            .build(k)
                            .key(format!("transfer-acknowledge-{id}")),
                    );
                }
            }
            body = body.child(card);
        }
    } else {
        if let Some(pool) = p.pool_id {
            let name = s
                .inventory
                .as_ref()
                .filter(|i| i.pool_id == pool)
                .map_or("Captured pool", |i| i.pool_name.as_str());
            body = body.child(caption(format!("Pool: {name} · {pool}"), k).wrap_anywhere());
        }
        if !current(s, p) {
            body = body.child(
                caption(
                    "Connection changed. Reopen this review using current inventory.",
                    k,
                )
                .wrap(),
            )
        }
        if p.source.is_none() && !p.review && !exporting(p) {
            body = body.child(
                Button::new(if p.disk_picker {
                    "Hide export disk choices"
                } else {
                    "Choose existing disk to export"
                })
                .on_press(|s: &mut State| s.transfers.disk_picker = !s.transfers.disk_picker)
                .build(k)
                .key("transfer-disk-picker"),
            );
        }
        if p.disk_picker && p.source.is_none() && !p.review && !exporting(p) {
            body = body.child(heading("Export an existing disk", k));
            if let Some(i) = &s.inventory {
                let mut disks = i.details.vdis.values().collect::<Vec<_>>();
                disks.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                for d in disks {
                    if let (Some(sr), Some(size)) = (d.sr, d.virtual_size)
                        && (TransferAction::ExportDisk {
                            vdi: d.id,
                            expected_sr: sr,
                            expected_size: size,
                            format: DiskImageFormat::Raw,
                        })
                        .availability(i)
                        .is_available()
                    {
                        body = body.child(
                            row()
                                .gap(8.0)
                                .flex_wrap()
                                .child(caption(format!("{} · {}", d.name, d.id), k).wrap_anywhere())
                                .child(disk_button(s, d, k)),
                        );
                    }
                }
            }
            body = body.child(heading("Import an image", k));
        }
        if p.source.is_some() {
            let mut formats = row().gap(8.0).flex_wrap();
            for (format, label, key) in [
                (None, "XVA", "xva"),
                (Some(OvfPackageFormat::Ova), "OVA file", "ova"),
                (Some(OvfPackageFormat::Ovf), "OVF directory", "ovf"),
            ] {
                if format.is_some() && matches!(p.source, Some(ExportSource::Checkpoint { .. })) {
                    continue;
                }
                formats = formats.child(
                    Button::new(format!(
                        "{}{label}",
                        if p.package_export == format {
                            "✓ "
                        } else {
                            ""
                        }
                    ))
                    .disabled(p.review)
                    .on_press(move |s: &mut State| {
                        set_format(s, None);
                        s.transfers.package_export = format;
                    })
                    .build(k)
                    .key(format!("transfer-package-{key}")),
                );
            }
            body = body.child(formats);
            if p.package_export.is_none() {
                let mut choices = row().gap(8.0).flex_wrap();
                for compression in xenadmin_core::XvaCompression::ALL {
                    choices = choices.child(
                        Button::new(format!(
                            "{}{}",
                            if p.compression == compression {
                                "✓ "
                            } else {
                                ""
                            },
                            compression.label()
                        ))
                        .disabled(p.review)
                        .on_press(move |s: &mut State| {
                            set_format(s, None);
                            s.transfers.compression = compression;
                        })
                        .build(k)
                        .key(format!("transfer-compression-{}", compression.label())),
                    );
                }
                body = body.child(heading("XVA compression", k)).child(choices)
                    .child(caption(format!("Output format: {} · .{}", p.compression.label(), p.compression.extension()), k).wrap())
                    .child(caption("Compressed exports require host support. The downloaded format and complete archive are verified before the new file is published.", k).wrap());
            }
            if let Some(ExportSource::Checkpoint {
                memory,
                preserve_memory,
                ..
            }) = &p.source
            {
                body = body.child(
                    caption(
                        format!(
                            "Checkpoint memory image: {} · {} bytes · storage {}",
                            memory.vdi, memory.virtual_size, memory.sr
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                for (preserve, label) in [(false, "Disks only"), (true, "Include guest memory")] {
                    body = body.child(
                        Button::new(format!(
                            "{}{label}",
                            if *preserve_memory == preserve {
                                "✓ "
                            } else {
                                ""
                            }
                        ))
                        .disabled(p.review)
                        .on_press(move |s: &mut State| {
                            if let Some(ExportSource::Checkpoint {
                                preserve_memory, ..
                            }) = &mut s.transfers.source
                            {
                                *preserve_memory = preserve;
                            }
                            s.transfers.review = false;
                        })
                        .build(k)
                        .key(if preserve {
                            "checkpoint-preserve-memory"
                        } else {
                            "checkpoint-disks-only"
                        }),
                    );
                }
                body = body.child(caption(if *preserve_memory { "The archive includes guest RAM, which may contain credentials and other sensitive data. Import creates a suspended guest; CPU, hypervisor and guest compatibility must be checked before a separate resume action." } else { "Only configuration and disks are exported. Import creates a halted guest; the checkpoint's memory state is omitted." }, k).wrap());
                body = body.child(caption("Full XVA export creates one independent guest archive. Snapshot ancestry is not included by this endpoint.", k).wrap());
            }
            if let Some(format) = p.package_export {
                body = body.child(caption(match format {
                    OvfPackageFormat::Ova => "Create one OVA file containing the descriptor and VHD disks.",
                    OvfPackageFormat::Ovf => "Create a new private directory containing the OVF descriptor, manifest and VHD disks. Choose a new directory name in the save dialog; existing directories are not reused.",
                }, k).wrap()).child(caption("OVF/OVA export supports ordinary halted BIOS/UEFI guests with supported disk and NIC settings. Mounted installation media and unsupported configuration must be resolved before export. Package disks contain guest data.", k).wrap());
            }
        }
        if p.source.is_none() {
            let mut formats = row().gap(8.0).flex_wrap();
            if p.disk_export.is_none() {
                formats = formats.child(
                    Button::new("XVA")
                        .disabled(p.review)
                        .on_press(|s: &mut State| set_format(s, None))
                        .build(k)
                        .key("transfer-format-xva"),
                );
            }
            if p.disk_export.is_none() {
                formats = formats.child(
                    Button::new(if p.ovf_import {
                        "✓ OVF / OVA"
                    } else {
                        "OVF / OVA"
                    })
                    .disabled(p.review)
                    .on_press(|s: &mut State| {
                        set_format(s, None);
                        s.transfers.ovf_import = true;
                    })
                    .build(k)
                    .key("transfer-format-ovf"),
                );
            }
            for (format, label) in [(DiskImageFormat::Vhd, "VHD"), (DiskImageFormat::Raw, "Raw")] {
                formats = formats.child(
                    Button::new(format!(
                        "{}{label}",
                        if p.format == Some(format) { "✓ " } else { "" }
                    ))
                    .disabled(p.review)
                    .on_press(move |s: &mut State| set_format(s, Some(format)))
                    .build(k)
                    .key(format!("transfer-format-{}", format.extension())),
                );
            }
            body = body.child(formats);
        }
        if let Some((vdi, sr, size)) = p.disk_export {
            body=body.child(caption(format!("Disk: {}\nUUID: {vdi}\nStorage: {sr}\nVirtual size: {size} bytes",p.source_name),k).wrap_anywhere()).child(caption("All consumers must remain halted. For a stable point in time, export a disk snapshot. The private file is published only after full transfer and format verification.",k).wrap());
        } else if let Some(source) = &p.source {
            body=body.child(caption(match source{ExportSource::Vm{vm,expected_template}=>format!("{}: {}\nUUID: {vm}",if *expected_template{"Template"}else{"Virtual machine"},p.source_name),ExportSource::Snapshot{snapshot,parent}=>format!("Disk snapshot: {}\nSnapshot UUID: {snapshot}\nParent VM UUID: {parent}",p.source_name),ExportSource::Checkpoint{snapshot,parent,..}=>format!("Memory checkpoint: {}\nSnapshot UUID: {snapshot}\nParent VM UUID: {parent}",p.source_name)},k).wrap_anywhere()).child(caption("Save to a new filename. Existing files are never overwritten. The final output appears only after all required transfers and verification complete. Exports contain guest disk data.",k).wrap());
        } else {
            if p.ovf_import {
                body = body.child(caption("Import ordinary BIOS or UEFI guests from OVF or OVA packages containing raw or complete VHD disks. Map each package network explicitly. New guest, disk and NIC identities are created; MAC addresses are generated. Guests are not started. Convert VMDK disks separately; automatic appliance remapping and unsupported device extensions are not available.", k).wrap());
            } else if p.format.is_some() {
                body=body.child(caption("Create a new unattached data disk from a complete raw or fixed/dynamic VHD image. Existing disks are not overwritten. Differencing VHD, VHDX and QCOW2 are not supported here.",k).wrap());
                body = body.child(if p.review {
                    caption(format!("New disk name: {}", p.disk_name.text), k).wrap()
                } else {
                    TextInput::new(&p.disk_name)
                        .label("New disk name")
                        .on_edit(|s: &mut State| &mut s.transfers.disk_name)
                        .build(k)
                        .key("transfer-disk-name")
                });
            } else {
                body=body.child(caption("Import halted guests or memory checkpoints from a plain, gzip or zstd XVA archive. New guest identities are created; existing guests are not replaced. Disk-only guests remain halted; memory checkpoints remain suspended and are never resumed automatically. XAPI matches networks by name or creates networks; review NICs before starting a guest.",k).wrap());
            }
            if let Some(i) = &s.inventory {
                for sr in &i.storage {
                    let id = SrId::from(sr.uuid);
                    if p.review && p.sr != Some(id) {
                        continue;
                    }
                    let a = TransferAction::ImportXva { sr: id };
                    if a.availability(i).is_available() {
                        body = body.child(
                            Button::new(format!(
                                "{}{} · {}",
                                if p.sr == Some(id) { "✓ " } else { "" },
                                sr.name,
                                id
                            ))
                            .disabled(p.review)
                            .on_press(move |s: &mut State| s.transfers.sr = Some(id))
                            .build(k)
                            .key(format!("transfer-sr-{id}")),
                        );
                    }
                }
            }
        }
        body = body.child(
            Button::new(if p.pending {
                "Inspecting file…"
            } else if exporting(p) {
                "Choose destination"
            } else {
                "Choose image file"
            })
            .disabled(p.pending)
            .on_press_tx(choose)
            .build(k)
            .key("transfer-choose"),
        );
        if let Some(file) = &p.file {
            body = body.child(heading(file.name(), k));
            if let Some(d) = file.disk_image() {
                body = body.child(
                    caption(
                        format!(
                            "{:?} · {} virtual bytes · {} file bytes · {}",
                            d.format,
                            d.virtual_size,
                            file.size().unwrap_or(0),
                            if d.sparse {
                                "Dynamic image"
                            } else {
                                "Fixed size image"
                            }
                        ),
                        k,
                    )
                    .wrap(),
                );
            }
            if let Some(summary) = file.ovf() {
                body = body.child(ovf_ui::summary(s, summary, k));
            }
            if let Some(a) = file.archive() {
                body = body.child(
                    caption(
                        format!(
                            "Archive compression: {}",
                            a.compression.map_or("Unknown", |c| c.label())
                        ),
                        k,
                    )
                    .wrap(),
                );
                body=body.child(caption(format!("{} guests · {} data disks · {} virtual bytes · {} networks · {} archive bytes",a.guests.len(),a.data_disks,a.virtual_bytes,a.networks,file.size().unwrap_or(0)),k).wrap());
                if a.memory_images() > 0 {
                    body = body.child(caption(format!("{} guest memory image(s) fully checked. Import will create suspended guests. Memory may contain secrets; review target CPU/hypervisor compatibility and NICs before a separate resume action.", a.memory_images()), k).wrap());
                }
                for g in &a.guests {
                    body = body.child(
                        caption(
                            format!(
                                "{} · {}",
                                g.name,
                                if g.memory.is_some() {
                                    "Suspended checkpoint"
                                } else if g.template {
                                    "Template"
                                } else {
                                    "Halted VM"
                                }
                            ),
                            k,
                        )
                        .wrap(),
                    );
                }
            }
        }
        if s.demo
            && let Some(summary) = &p.ovf_preview
        {
            body = body
                .child(heading("application.ova · synthetic preview", k))
                .child(ovf_ui::summary(s, summary, k));
        }
        let available = action(p).and_then(|a| s.inventory.as_ref().map(|i| a.availability(i)));
        if let Some(a) = &available
            && !a.is_available()
        {
            body = body.child(caption(a.explanation(), k).wrap());
        }
        if p.review {
            let permission = action(p)
                .map(|a| session_permissions_view::assess(s, &a.permission_requirements()));
            if let Some(assessment) = &permission {
                body = body.child(session_permissions_view::review_notice(assessment, k));
            }
            body=body.child(caption(if exporting(p) { "This review uses the captured source and new destination. Interrupted exports retain their evidence in history. Transfers reserve pool changes until the outcome is resolved." } else { "This review uses the selected file identity and destination. An interrupted import may retain partial resources; their confirmed UUIDs remain in history. Transfers reserve pool changes until the outcome is resolved." },k).wrap()).child(Button::new("Begin transfer").disabled(s.resource_operations.len()>=64||permission.as_ref().is_some_and(|p|p.decision().is_denied())||s.demo||!current(s,p)||!available.is_some_and(|a|a.is_available())||p.file.is_none()||(!exporting(p)&&!s.can_mutate())).on_press(apply).build(k).key("transfer-apply"));
        } else {
            body = body.child(
                Button::new("Review transfer")
                    .disabled(
                        p.file.is_none()
                            || !available.is_some_and(|a| a.is_available())
                            || !current(s, p),
                    )
                    .on_press(|s: &mut State| s.transfers.review = true)
                    .build(k)
                    .key("transfer-review"),
            );
        }
    }
    if let Some(e) = &p.error {
        body = body.child(caption(e, k).wrap());
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Archive transfer")
        .modal(false, close)
        .key("transfer-dialog")
        .child(body)
}

pub(super) fn scene(s: &mut State) {
    demo(s);
    let id = s
        .inventory
        .as_ref()
        .unwrap()
        .vms
        .values()
        .find(|v| !v.is_control_domain && !v.is_template)
        .unwrap()
        .id;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    i.vms.get_mut(&id).unwrap().power_state = PowerState::Halted;
    i.details.vms.get_mut(&id).unwrap().allowed_operations = Some(vec!["export".into()]);
    begin(
        s,
        Some(ExportSource::Vm {
            vm: id,
            expected_template: false,
        }),
    );
}
pub(super) fn compression_scene(s: &mut State) {
    scene(s);
    s.transfers.compression = xenadmin_core::XvaCompression::Zstd;
}
pub(super) fn checkpoint_scene(s: &mut State) {
    demo(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let parent = *i.vms.keys().next().unwrap();
    let snapshot = xenadmin_core::SnapshotId::from(uuid::Uuid::from_u128(0x9901));
    let memory = VdiId::from(uuid::Uuid::from_u128(0x9902));
    let mut disk = i.details.vdis.values().next().unwrap().clone();
    disk.id = memory;
    disk.name = "Checkpoint memory".into();
    disk.kind = "suspend".into();
    disk.virtual_size = Some(512 * 1024 * 1024);
    disk.managed = Some(true);
    disk.missing = Some(false);
    i.details.vdis.insert(memory, disk);
    i.details.snapshots.insert(
        snapshot,
        xenadmin_core::SnapshotDetails {
            memory_vdi: Some(memory),
            memory_vdi_known: true,
            can_export: Some(true),
            ..Default::default()
        },
    );
    i.snapshots.insert(xenadmin_core::Snapshot {
        id: snapshot,
        vm: Some(parent),
        name: "Application memory checkpoint".into(),
        description: "Synthetic checkpoint for archive review".into(),
        created_at: None,
        power_state: PowerState::Suspended,
        allowed_actions: vec![],
    });
    let source = ExportSource::checkpoint(i, snapshot, parent, false).unwrap();
    begin(s, Some(source));
}
pub(super) fn ovf_scene(s: &mut State, export: bool) {
    ovf_ui::scene(s, export);
}
pub(super) fn disk_scene(s: &mut State) {
    demo(s);
    begin(s, None);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let d = i.details.vdis.values().next().unwrap();
    s.transfers.disk_export = Some((d.id, d.sr.unwrap(), d.virtual_size.unwrap()));
    s.transfers.source_name = d.name.clone();
    s.transfers.format = Some(DiskImageFormat::Vhd);
    for vm in i.vms.values_mut() {
        vm.power_state = PowerState::Halted;
    }
    for vbd in i.details.vbds.values_mut() {
        vbd.currently_attached = Some(false);
    }
    s.rebuild();
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
        let mut s = init();
        scene(&mut s);
        s.window_width = size.0;
        s.window_height = size.1;
        let mut rt = rune::Runtime::new(
            s,
            view,
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
    pub(super) fn click(rt: &mut rune::Runtime<State>, key: &str) {
        rt.update(|_, tx| tx.scroll_into_view(key));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("Missing control {key}"));
        assert!(x >= 0.0 && x <= rt.state.window_width && y >= 0.0 && y <= rt.state.window_height);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }
    #[test]
    fn native_save_captures_source_and_reviews_without_demo_writes() {
        for size in [(480.0, 800.0), (1440.0, 900.0)] {
            let mut rt = runtime(size);
            let source = rt.state.transfers.source.clone();
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("export.xva");
            click(&mut rt, "transfer-choose");
            assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Save);
            rt.resolve_file_dialog(Some(path.clone())).unwrap();
            rt.frame(0.0);
            for _ in 0..500 {
                if poll(&mut rt.state) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(rt.state.transfers.file.is_some());
            assert_eq!(rt.state.transfers.source, source);
            click(&mut rt, "transfer-review");
            assert!(rt.state.transfers.review);
            click(&mut rt, "transfer-apply");
            assert!(rt.state.resource_operations.is_empty());
            assert!(!path.exists());
        }
    }
    #[test]
    fn cancelled_or_stale_file_dialog_does_not_replace_review() {
        let mut rt = runtime((640.0, 800.0));
        click(&mut rt, "transfer-choose");
        rt.resolve_file_dialog(None).unwrap();
        rt.frame(0.0);
        assert!(!rt.state.transfers.pending);
        click(&mut rt, "transfer-choose");
        close(&mut rt.state);
        let d = tempfile::tempdir().unwrap();
        rt.resolve_file_dialog(Some(d.path().join("stale.xva")))
            .unwrap();
        rt.frame(0.0);
        assert!(rt.state.transfers.file.is_none());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }
    #[test]
    fn offline_unknown_history_preserves_evidence_and_cannot_release_guard() {
        let mut rt = runtime((480.0, 800.0));
        let pool = rt.state.inventory.as_ref().unwrap().pool_id;
        let id = OperationId::new();
        let record:xenadmin_core::ResourceOperationRecord=serde_json::from_value(serde_json::json!({"id":id,"pool":pool,"target":{"Pool":pool},"action":{"Transfer":{"ImportXva":{"sr":uuid::Uuid::from_u128(4)}}},"state":"OutcomeUnknown","progress":null,"detail":"Host import failed; cleanup unverified","created_at_ms":1,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"transfer":{"bytes":1024,"total":2048,"stream_complete":false,"phase":"Stopped"},"transfer_created":{"vms":[],"disks":[uuid::Uuid::from_u128(22)],"networks":[]}})).unwrap();
        rt.state.resource_rows = vec![record];
        rt.state.inventory = None;
        rt.state.snapshot = None;
        rt.state.transfers.history = true;
        click(&mut rt, &format!("transfer-acknowledge-{id}"));
        assert!(rt.state.transfers.acknowledge.is_none());
        assert_eq!(
            rt.state.resource_rows[0].state,
            OperationState::OutcomeUnknown
        );
        assert_eq!(rt.state.resource_rows[0].transfer_created.disks.len(), 1);
    }
    #[test]
    fn switched_pool_invalidates_review_and_import_uses_open_dialog() {
        let mut rt = runtime((640.0, 800.0));
        assert!(current(&rt.state, &rt.state.transfers));
        Arc::make_mut(rt.state.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::new_v4().into();
        assert!(!current(&rt.state, &rt.state.transfers));
        open(&mut rt.state);
        click(&mut rt, "transfer-choose");
        assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Open);
        rt.resolve_file_dialog(None).unwrap();
    }
    pub(super) fn disk_scene(rt: &mut rune::Runtime<State>) -> VdiId {
        open(&mut rt.state);
        let i = Arc::make_mut(rt.state.inventory.as_mut().unwrap());
        let sr = SrId::from(i.storage[0].uuid);
        let pbd = xenadmin_core::PbdId::from(uuid::Uuid::from_u128(0x9991));
        i.details.pbds.insert(
            pbd,
            xenadmin_core::StorageAttachment {
                id: pbd,
                location: None,
                requires_credentials: false,
                multipathed: None,
                paths: Vec::new(),
                host: i.hosts.keys().next().copied(),
                sr: Some(sr),
                currently_attached: Some(true),
            },
        );
        for vm in i.vms.values_mut() {
            vm.power_state = PowerState::Halted;
        }
        for vbd in i.details.vbds.values_mut() {
            vbd.currently_attached = Some(false);
        }
        *i.details.vdis.keys().next().unwrap()
    }
    #[test]
    fn native_raw_import_reviews_name_capacity_and_exact_storage_without_overwrite() {
        for size in [(480.0, 800.0), (1440.0, 900.0)] {
            let mut rt = runtime(size);
            disk_scene(&mut rt);
            click(&mut rt, "transfer-format-raw");
            let sr = SrId::from(rt.state.inventory.as_ref().unwrap().storage[0].uuid);
            click(&mut rt, &format!("transfer-sr-{sr}"));
            rt.state.transfers.disk_name = TextEdit::new("Imported data");
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("data.raw");
            std::fs::write(&path, vec![7; 4096]).unwrap();
            click(&mut rt, "transfer-choose");
            assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Open);
            rt.resolve_file_dialog(Some(path.clone())).unwrap();
            rt.frame(0.0);
            for _ in 0..500 {
                if poll(&mut rt.state) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert_eq!(
                rt.state
                    .transfers
                    .file
                    .as_ref()
                    .unwrap()
                    .disk_image()
                    .unwrap()
                    .virtual_size,
                4096
            );
            click(&mut rt, "transfer-review");
            assert!(rt.state.transfers.review);
            assert!(rt.find("transfer-disk-name").is_none());
            assert_eq!(
                action(&rt.state.transfers),
                Some(TransferAction::ImportDisk {
                    sr,
                    name: "Imported data".into(),
                    format: DiskImageFormat::Raw
                })
            );
            click(&mut rt, "transfer-apply");
            assert!(rt.state.resource_operations.is_empty());
            assert_eq!(std::fs::read(path).unwrap(), vec![7; 4096]);
        }
    }
    #[test]
    fn disk_export_captures_identity_and_rejects_capacity_or_attachment_changes() {
        let mut rt = runtime((480.0, 800.0));
        let id = disk_scene(&mut rt);
        click(&mut rt, "transfer-disk-picker");
        click(&mut rt, &format!("export-disk-{id}"));
        let a = action(&rt.state.transfers).unwrap();
        assert!(a.is_export());
        assert!(
            a.availability(rt.state.inventory.as_ref().unwrap())
                .is_available()
        );
        click(&mut rt, "transfer-choose");
        assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Save);
        rt.resolve_file_dialog(None).unwrap();
        rt.frame(0.0);
        let i = Arc::make_mut(rt.state.inventory.as_mut().unwrap());
        i.details.vdis.get_mut(&id).unwrap().virtual_size = Some(512);
        assert!(!a.availability(i).is_available());
        assert_eq!(action(&rt.state.transfers), Some(a));
    }
    #[test]
    fn format_change_invalidates_selected_file_and_blank_disk_name_cannot_apply() {
        let mut rt = runtime((640.0, 800.0));
        disk_scene(&mut rt);
        let sr = SrId::from(rt.state.inventory.as_ref().unwrap().storage[0].uuid);
        rt.state.transfers.sr = Some(sr);
        set_format(&mut rt.state, Some(DiskImageFormat::Raw));
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("data.raw");
        std::fs::write(&path, vec![0; 512]).unwrap();
        rt.state.transfers.file =
            Some(TransferFile::disk_source(path, DiskImageFormat::Raw).unwrap());
        assert!(
            !action(&rt.state.transfers)
                .unwrap()
                .availability(rt.state.inventory.as_ref().unwrap())
                .is_available()
        );
        set_format(&mut rt.state, Some(DiskImageFormat::Vhd));
        assert!(rt.state.transfers.file.is_none());
        assert!(!rt.state.transfers.review);
    }
}
