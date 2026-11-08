use super::*;
use std::sync::mpsc;
use xenadmin_core::{
    ExportSource, ObservedResource, OperationControl, OperationId, OperationState, ResourceAction,
    ResourceTarget, SrId, TransferAction, TransferFile, TransferRequest,
};
#[derive(Default)]
pub(super) struct Panel {
    pub show: bool,
    serial: u64,
    source: Option<ExportSource>,
    source_name: String,
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
    s.transfers.show = false;
    s.transfers.serial += 1;
    s.transfers.pending = false;
    s.transfers.result = None;
}
fn begin(s: &mut State, source: Option<ExportSource>) {
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
                ExportSource::Snapshot { snapshot, .. } => {
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
        source: source.clone(),
    };
    let available = s
        .inventory
        .as_ref()
        .is_some_and(|i| action.availability(i).is_available());
    Button::new("Export XVA")
        .variant(ButtonVariant::Outline)
        .disabled(!s.demo && !available)
        .on_press(move |s: &mut State| begin(s, Some(source.clone())))
        .build(k)
        .key("export-xva")
}
pub(super) fn open(s: &mut State) {
    begin(s, None);
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
fn action(p: &Panel) -> Option<TransferAction> {
    p.source
        .clone()
        .map(|source| TransferAction::ExportXva { source })
        .or_else(|| p.sr.map(|sr| TransferAction::ImportXva { sr }))
}
fn choose(s: &mut State, tx: &mut Tx) {
    if s.transfers.pending {
        return;
    }
    let serial = s.transfers.serial;
    let export = s.transfers.source.is_some();
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
                            } else {
                                TransferFile::source(path)
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
    let options = rune::FileDialogOptions {
        title: if export {
            "Save a new XVA archive"
        } else {
            "Import an XVA archive"
        }
        .into(),
        file_name: export.then(|| "guest.xva".into()),
        ..Default::default()
    }
    .filter("XVA archive", &["xva"]);
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
    if !p.review
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
                        "Archive transfers"
                    } else if p.source.is_some() {
                        "Export XVA"
                    } else {
                        "Import XVA"
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
                        .variant(ButtonVariant::Ghost)
                        .on_press(close)
                        .build(k),
                ),
        );
    if p.history {
        body = body.child(
            Button::new("Import XVA")
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
            for id in &r.transfer_created.networks {
                card =
                    card.child(caption(format!("Retained network UUID: {id}"), k).wrap_anywhere())
            }
            if !r.state.terminal() {
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
            if r.state == OperationState::OutcomeUnknown {
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
        if !current(s, p) {
            body = body.child(
                caption(
                    "Connection changed. Reopen this review using current inventory.",
                    k,
                )
                .wrap(),
            )
        }
        if let Some(source) = &p.source {
            body=body.child(caption(match source{ExportSource::Vm{vm,expected_template}=>format!("{}: {}\nUUID: {vm}",if *expected_template{"Template"}else{"Virtual machine"},p.source_name),ExportSource::Snapshot{snapshot,parent}=>format!("Disk snapshot: {}\nSnapshot UUID: {snapshot}\nParent VM UUID: {parent}",p.source_name)},k).wrap_anywhere()).child(caption("Save to a new filename. Existing files are never overwritten. The final archive appears only after the complete stream and server task are verified. Exports contain guest disk data.",k).wrap());
        } else {
            body=body.child(caption("Import ordinary halted VMs or custom templates from a plain or gzip XVA archive. New guest identities are created; existing guests are not replaced. Guests remain halted. XAPI matches networks by name or creates networks; review NICs before starting a guest.",k).wrap());
            if let Some(i) = &s.inventory {
                for sr in &i.storage {
                    let id = SrId::from(sr.uuid);
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
            } else if p.source.is_some() {
                "Choose destination"
            } else {
                "Choose XVA file"
            })
            .disabled(p.pending)
            .on_press_tx(choose)
            .build(k)
            .key("transfer-choose"),
        );
        if let Some(file) = &p.file {
            body = body.child(heading(file.name(), k));
            if let Some(a) = file.archive() {
                body=body.child(caption(format!("{} guests · {} data disks · {} virtual bytes · {} networks · {} archive bytes",a.guests.len(),a.data_disks,a.virtual_bytes,a.networks,file.size().unwrap_or(0)),k).wrap());
                for g in &a.guests {
                    body = body.child(
                        caption(
                            format!(
                                "{} · {}",
                                g.name,
                                if g.template { "Template" } else { "VM" }
                            ),
                            k,
                        )
                        .wrap(),
                    );
                }
            }
        }
        let available = action(p).and_then(|a| s.inventory.as_ref().map(|i| a.availability(i)));
        if let Some(a) = &available
            && !a.is_available()
        {
            body = body.child(caption(a.explanation(), k).wrap());
        }
        if p.review {
            body=body.child(caption("This review uses the selected file identity and destination. An interrupted import may retain partial resources; their confirmed UUIDs remain in history. Transfers reserve pool changes until the outcome is resolved.",k).wrap()).child(Button::new("Begin transfer").disabled(s.demo||!current(s,p)||!available.is_some_and(|a|a.is_available())||p.file.is_none()||(p.source.is_none()&&!s.can_mutate())).on_press(apply).build(k).key("transfer-apply"));
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
#[cfg(test)]
mod tests {
    use super::*;
    fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
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
    fn click(rt: &mut rune::Runtime<State>, key: &str) {
        rt.update(|_, tx| tx.scroll_into_view(key));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find(key).unwrap();
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
}
