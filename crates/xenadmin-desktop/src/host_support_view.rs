use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    HostArchiveOperation, HostId, HostSupportOverview, HostSupportQuery, ObservedResource,
    OperationId, ResourceTarget, TransferAction, TransferFile, TransferRequest,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Backup,
    Support,
    Restore,
}
pub(super) struct Panel {
    nonce: OperationId,
    epoch: u64,
    host: HostId,
    name: String,
    version: String,
    observed: Option<ObservedResource>,
    pool: Option<ObservedResource>,
    query: Option<HostSupportQuery>,
    overview: Option<HostSupportOverview>,
    mode: Mode,
    confirmation: TextEdit,
    inspection_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    selected: BTreeSet<String>,
    pending: bool,
    result: Option<std::sync::mpsc::Receiver<Result<TransferFile, String>>>,
    file: Option<TransferFile>,
    review: Option<TransferAction>,
    deletion: Option<xenadmin_core::HostCrashdump>,
    dump_page: usize,
    error: Option<String>,
}
impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(c) = &self.inspection_cancel {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
fn cancel_inspection(p: &mut Panel) {
    if let Some(c) = p.inspection_cancel.take() {
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    if s.page != Page::Hosts && console_view::host(s).is_none() {
        return el();
    }
    Button::new("Host backup and support…")
        .on_press(open)
        .build(k)
        .key("host-support-open")
}
fn open(s: &mut State) {
    let Some(ResourceTarget::Host(host)) = resource_admin::selected_target(s) else {
        return;
    };
    let Some(i) = s.inventory.as_ref() else {
        return;
    };
    let Some(h) = i.hosts.get(&host) else { return };
    let observed = s
        .snapshot
        .as_ref()
        .and_then(|v| v.observe_resource(ResourceTarget::Host(host)).ok());
    let pool = s
        .snapshot
        .as_ref()
        .and_then(|v| v.observe_resource(ResourceTarget::Pool(i.pool_id)).ok());
    s.host_support = Some(Panel {
        nonce: OperationId::new(),
        epoch: 0,
        host,
        name: h.name.clone(),
        version: h.version.clone(),
        observed,
        pool,
        query: None,
        overview: None,
        mode: Mode::Backup,
        confirmation: TextEdit::new(""),
        inspection_cancel: None,
        selected: BTreeSet::new(),
        pending: false,
        result: None,
        file: None,
        review: None,
        deletion: None,
        dump_page: 0,
        error: None,
    });
    refresh(s);
}
fn refresh(s: &mut State) {
    let Some(p) = &mut s.host_support else { return };
    cancel_inspection(p);
    p.confirmation = TextEdit::new("");
    p.epoch += 1;
    p.file = None;
    p.review = None;
    p.deletion = None;
    p.pending = false;
    p.result = None;
    p.overview = None;
    p.dump_page = 0;
    p.selected.clear();
    p.error = None;
    if s.demo {
        return;
    }
    let result = p.observed.ok_or(xenadmin_core::Error::Stale).and_then(|o| {
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .host_support(o)
    });
    match result {
        Ok(q) => {
            if let Some(w) = &s.waker {
                q.notify_on_change(w.clone());
            }
            p.query = Some(q);
        }
        Err(e) => p.error = Some(e.to_string()),
    }
}
fn current(s: &State, p: &Panel) -> bool {
    s.inventory
        .as_ref()
        .is_some_and(|i| i.hosts.get(&p.host).is_some_and(|h| h.version == p.version))
        && (s.demo
            || p.observed.is_some_and(|o| {
                s.snapshot
                    .as_ref()
                    .and_then(|v| v.observe_resource(o.target()).ok())
                    == Some(o)
            }) && p.pool.is_some_and(|o| {
                s.snapshot
                    .as_ref()
                    .and_then(|v| v.observe_resource(o.target()).ok())
                    == Some(o)
            }))
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(p) = &mut s.host_support else {
        return false;
    };
    let mut changed = false;
    if let Some(result) = p.query.as_ref().and_then(HostSupportQuery::latest) {
        p.query = None;
        changed = true;
        match result {
            Ok(v) if v.host == p.host && v.version == p.version => {
                p.selected = v
                    .capabilities
                    .iter()
                    .filter(|c| c.default_selected)
                    .map(|c| c.key.clone())
                    .collect();
                p.overview = Some(v);
            }
            Ok(_) => {
                p.error =
                    Some("Host identity or version changed; close and reopen this review".into())
            }
            Err(e) => p.error = Some(e),
        }
    }
    if let Some(result) = p.result.as_ref().and_then(|r| match r.try_recv() {
        Ok(v) => Some(v),
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
        Err(_) => Some(Err("File selection stopped".into())),
    }) {
        p.pending = false;
        p.result = None;
        changed = true;
        match result {
            Ok(v) => p.file = Some(v),
            Err(e) => p.error = Some(e),
        }
    }
    changed
}
fn action(p: &Panel) -> Option<TransferAction> {
    let operation = if p.mode == Mode::Restore {
        HostArchiveOperation::StageRestore {
            archive: p.file.as_ref()?.host_backup()?.clone(),
        }
    } else if p.mode == Mode::Support {
        let overview = p.overview.as_ref()?;
        if overview.capabilities_error.is_some()
            || p.selected.is_empty()
            || p.selected
                .iter()
                .any(|s| !overview.capabilities.iter().any(|c| &c.key == s))
        {
            return None;
        }
        HostArchiveOperation::Support {
            entries: p.selected.iter().cloned().collect(),
        }
    } else {
        HostArchiveOperation::Backup
    };
    Some(TransferAction::Host {
        host: p.host,
        expected_version: p.version.clone(),
        operation,
    })
}
fn mode(s: &mut State, mode: Mode) {
    if let Some(p) = &mut s.host_support {
        cancel_inspection(p);
        p.mode = mode;
        p.confirmation = TextEdit::new("");
        p.epoch += 1;
        p.pending = false;
        p.result = None;
        p.file = None;
        p.review = None;
        p.deletion = None;
        p.error = None;
    }
}
fn choose(s: &mut State, tx: &mut Tx) {
    let Some(p) = &mut s.host_support else { return };
    if p.pending {
        return;
    }
    cancel_inspection(p);
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    p.inspection_cancel = Some(cancel.clone());
    let nonce = p.nonce;
    let epoch = p.epoch;
    let mode = p.mode;
    let host = p.host;
    let version = p.version.clone();
    p.pending = true;
    p.file = None;
    p.review = None;
    p.error = None;
    p.confirmation = TextEdit::new("");
    let callback =
        move |s: &mut State, result: Result<Option<std::path::PathBuf>, String>, _: &mut Tx| {
            let Some(p) = s
                .host_support
                .as_mut()
                .filter(|p| p.nonce == nonce && p.epoch == epoch)
            else {
                return;
            };
            match result {
                Ok(Some(path)) => {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    let wake = s.waker.clone();
                    if std::thread::Builder::new()
                        .name("host-archive-file".into())
                        .spawn(move || {
                            let result = if mode == Mode::Restore {
                                TransferFile::host_backup_source(path, host, &version, &cancel)
                            } else {
                                TransferFile::destination(path)
                            };
                            let _ = sender.send(result.map_err(|e| e.to_string()));
                            if let Some(w) = wake {
                                w();
                            }
                        })
                        .is_ok()
                    {
                        p.result = Some(receiver);
                    } else {
                        p.pending = false;
                        p.error = Some("Cannot inspect the archive file".into());
                    }
                }
                Ok(None) => p.pending = false,
                Err(e) => {
                    p.pending = false;
                    p.error = Some(e);
                }
            }
        };
    if mode == Mode::Restore {
        tx.open_file(
            rune::FileDialogOptions {
                title: "Select a trusted backup from this host and version".into(),
                ..Default::default()
            }
            .filter("Host filesystem backup", &["gz", "tgz"]),
            move |s: &mut State, r, tx| callback(s, r.map_err(|e| e.to_string()), tx),
        );
    } else {
        tx.save_file(
            rune::FileDialogOptions {
                title: "Save a new private host archive".into(),
                file_name: Some(
                    if mode == Mode::Support {
                        "host-support.tar"
                    } else {
                        "host-backup.tar.gz"
                    }
                    .into(),
                ),
                ..Default::default()
            },
            move |s: &mut State, r, tx| callback(s, r.map_err(|e| e.to_string()), tx),
        );
    }
}
fn review(s: &mut State) {
    let valid = s.host_support.as_ref().is_some_and(|p| current(s, p));
    if valid
        && let Some(p) = &mut s.host_support
        && p.file.is_some()
    {
        p.review = action(p);
    }
}
fn submit(s: &mut State) {
    let result = (|| {
        let p = s.host_support.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if s.demo || !current(s, p) {
            return Err(xenadmin_core::Error::Stale);
        }
        let action = p
            .review
            .clone()
            .filter(|a| Some(a.clone()) == action(p))
            .ok_or(xenadmin_core::Error::Stale)?;
        if action.is_host_restore()
            && (!s.can_mutate() || p.confirmation.text.as_ref() != p.host.to_string())
        {
            return Err(xenadmin_core::Error::ReadOnly);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .transfer(
                OperationId::new(),
                p.pool.ok_or(xenadmin_core::Error::Stale)?,
                TransferRequest {
                    action,
                    file: p.file.clone().ok_or(xenadmin_core::Error::TransferFile)?,
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            s.host_support = None;
            transfers::history(s);
        }
        Err(e) => {
            if let Some(p) = &mut s.host_support {
                p.error = Some(e.to_string());
            }
        }
    }
}
fn review_deletion(s: &mut State, dump: xenadmin_core::HostCrashdump) {
    let Some(p) = &mut s.host_support else {
        return;
    };
    if !p
        .overview
        .as_ref()
        .is_some_and(|o| o.crashdumps_error.is_none() && o.crashdumps.contains(&dump))
    {
        return;
    }
    cancel_inspection(p);
    p.epoch += 1;
    p.pending = false;
    p.result = None;
    p.file = None;
    p.review = None;
    p.error = None;
    p.confirmation = TextEdit::new("");
    p.deletion = Some(dump);
}
fn submit_deletion(s: &mut State) {
    let result = (|| {
        let p = s.host_support.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let dump = p.deletion.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        if s.demo
            || s.resource_operations.len() >= 64
            || !s.can_mutate()
            || !current(s, p)
            || p.confirmation.text.as_ref() != dump.uuid.to_string()
        {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                xenadmin_core::ResourceCommand::DeleteHostCrashdump {
                    host: p.observed.ok_or(xenadmin_core::Error::Stale)?,
                    dump: dump.clone(),
                },
            )
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            s.host_support = None;
            activity::open(s);
        }
        Err(e) => {
            if let Some(p) = &mut s.host_support {
                p.error = Some(e.to_string());
            }
        }
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.host_support else {
        return el();
    };
    let mut body = col()
        .gap(12.0)
        .child(heading("Host backup and support", k))
        .child(caption(format!("{} · {}\nHost {}", p.name, p.version, p.host), k).wrap_anywhere());
    if !current(s, p) {
        body=body.child(caption("The captured connection or host version changed. Close and reopen before continuing.",k).color(k.c.danger.icon).wrap());
    }
    if let Some(dump) = &p.deletion {
        body=body.child(heading("Delete retained host crashdump",k))
            .child(caption(format!("Crashdump {}\n{} · {}",dump.uuid,dump.timestamp,bytes(dump.size)),k).wrap_anywhere())
            .child(caption("This asks the captured host to permanently remove this retained crashdump directory and its XAPI record. It can remove raw memory and logs needed for investigation. It does not create an export, affect another dump or remove previously downloaded support files. There is no undo.",k).wrap())
            .child(caption("A support log bundle does not include the raw memory dump. Raw dumps cannot be downloaded here. Retain any evidence you need before deleting.",k).wrap())
            .child(caption("A lost or failed response remains uncertain even if the record disappears. Recheck never repeats deletion. Successful verification establishes API completion and record absence, not an independent inspection of the host filesystem.",k).wrap())
            .child(caption("Type the crashdump UUID to confirm permanent deletion",k).wrap())
            .child(TextInput::new(&p.confirmation).label("Confirm crashdump UUID").on_edit(|s:&mut State|&mut s.host_support.as_mut().unwrap().confirmation).build(k).key("host-crashdump-confirmation"));
    } else if let Some(review) = &p.review {
        body = body.child(heading(review.label(), k)).child(
            caption(
                format!(
                    "{}: {}",
                    if p.mode == Mode::Restore {
                        "Source archive"
                    } else {
                        "Save as"
                    },
                    p.file.as_ref().map(TransferFile::name).unwrap_or_default()
                ),
                k,
            )
            .wrap_anywhere(),
        );
        if p.mode == Mode::Restore {
            body=body.child(caption("This reformats the host's configured BACKUP_PARTITION and replaces its contents with the selected filesystem backup. The API selects that partition; its current device path is not available to this client. Running host configuration is not restored, and no reboot or pool database restoration is performed.",k).wrap()).child(caption("Use only a backup you trust. Identity, checksum and archive structure checks do not establish that arbitrary system files are authentic or safe to install.",k).wrap());
            if let TransferAction::Host {
                operation: HostArchiveOperation::StageRestore { archive: a },
                ..
            } = review
            {
                body = body.child(
                    caption(
                        format!(
                            "{} entries · {} logical contents\nSHA-256 {}",
                            a.entries,
                            bytes(a.logical_bytes),
                            a.sha256
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>()
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
            }
            body = body
                .child(
                    caption(
                        "Type the host UUID to confirm backup partition replacement",
                        k,
                    )
                    .wrap(),
                )
                .child(
                    TextInput::new(&p.confirmation)
                        .label("Type the host UUID to confirm backup partition replacement")
                        .on_edit(|s: &mut State| &mut s.host_support.as_mut().unwrap().confirmation)
                        .build(k)
                        .key("host-restore-confirmation"),
                );
        } else {
            body=body.child(caption("Creates a new private file (0600). An existing file is never overwritten. The selected host's task and archive format are verified before the file is published.",k).wrap());
        }
        if let TransferAction::Host {
            operation: HostArchiveOperation::Support { entries },
            ..
        } = review
        {
            body = body.child(
                caption(format!("Requested categories: {}", entries.join(", ")), k).wrap_anywhere(),
            );
        }
    } else {
        body = body.child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new("Filesystem backup")
                        .disabled(p.mode == Mode::Backup)
                        .on_press(|s: &mut State| mode(s, Mode::Backup))
                        .build(k)
                        .key("host-support-backup"),
                )
                .child(
                    Button::new("Support bundle")
                        .disabled(p.mode == Mode::Support)
                        .on_press(|s: &mut State| mode(s, Mode::Support))
                        .build(k)
                        .key("host-support-bundle"),
                )
                .child(
                    Button::new("Stage recovery…")
                        .disabled(p.mode == Mode::Restore || !s.demo && !s.can_mutate())
                        .on_press(|s: &mut State| mode(s, Mode::Restore))
                        .build(k)
                        .key("host-support-restore"),
                ),
        );
        if p.mode == Mode::Support {
            body=body.child(caption("Support bundles contain selected diagnostics and may include private names, addresses and logs. They are saved locally; nothing is uploaded to a support service.",k).wrap());
            if p.query.is_some() {
                body = body.child(caption("Reading advertised diagnostic categories…", k));
            }
            if let Some(v) = &p.overview {
                if let Some(e) = &v.capabilities_error {
                    body = body.child(caption(e, k).color(k.c.danger.icon).wrap());
                }
                for capability in &v.capabilities {
                    let key = capability.key.clone();
                    let selected = p.selected.contains(&key);
                    body = body
                        .child(
                            Button::new(format!("{} {}", if selected { "✓" } else { "○" }, key))
                                .on_press(move |s: &mut State| {
                                    if let Some(p) = &mut s.host_support {
                                        if !p.selected.remove(&key) {
                                            p.selected.insert(key.clone());
                                        }
                                        p.review = None;
                                    }
                                })
                                .build(k)
                                .key(format!("host-support-cap-{}", capability.key)),
                        )
                        .child(
                            caption(
                                format!(
                                    "Private data: {} · maximum {} · up to {} seconds",
                                    capability.privacy,
                                    capability
                                        .maximum_bytes
                                        .map(bytes)
                                        .unwrap_or_else(|| "unknown size".into()),
                                    capability
                                        .maximum_seconds
                                        .map(|n| n.to_string())
                                        .unwrap_or_else(|| "unknown".into())
                                ),
                                k,
                            )
                            .wrap(),
                        );
                }
                body=body.child(caption("Crashdump logs exclude the raw memory dump (coredump.bin). Raw dump download is not supplied by this host API.",k).wrap());
                if let Some(e) = &v.crashdumps_error {
                    body = body
                        .child(caption(format!("Crashdump catalogue unavailable: {e}"), k).wrap());
                } else {
                    body = body.child(caption(
                        format!("{} crashdump records", v.crashdumps.len()),
                        k,
                    ));
                    let pages = v.crashdumps.len().div_ceil(32).max(1);
                    let page = p.dump_page.min(pages - 1);
                    if pages > 1 {
                        body = body.child(
                            row()
                                .gap(8.0)
                                .child(
                                    Button::new("Previous dumps")
                                        .disabled(page == 0)
                                        .on_press(|s: &mut State| {
                                            if let Some(p) = &mut s.host_support {
                                                p.dump_page = p.dump_page.saturating_sub(1);
                                            }
                                        })
                                        .build(k)
                                        .key("host-crashdump-previous"),
                                )
                                .child(caption(format!("Page {} of {}", page + 1, pages), k))
                                .child(
                                    Button::new("Next dumps")
                                        .disabled(page + 1 == pages)
                                        .on_press(move |s: &mut State| {
                                            if let Some(p) = &mut s.host_support {
                                                p.dump_page = (page + 1).min(pages - 1);
                                            }
                                        })
                                        .build(k)
                                        .key("host-crashdump-next"),
                                ),
                        );
                    }
                    for dump in v.crashdumps.iter().skip(page * 32).take(32) {
                        body = body
                            .child(
                                caption(
                                    format!(
                                        "{} · {} · {}",
                                        dump.uuid,
                                        dump.timestamp,
                                        bytes(dump.size)
                                    ),
                                    k,
                                )
                                .wrap_anywhere(),
                            )
                            .child(
                                Button::new("Review deletion…")
                                    .disabled(!s.demo && (!s.can_mutate() || !current(s, p)))
                                    .on_press({
                                        let dump = dump.clone();
                                        move |s: &mut State| review_deletion(s, dump.clone())
                                    })
                                    .build(k)
                                    .key(format!("host-crashdump-delete-{}", dump.uuid)),
                            );
                    }
                }
            }
        } else if p.mode == Mode::Restore {
            body=body.child(caption("Select a trusted gzip-tar filesystem backup from this exact host and product version. The archive is inspected without extraction. Staging will replace the configured backup partition for later installer-assisted recovery; it does not restore the running host.",k).wrap());
        } else {
            body=body.child(caption("Downloads the host's dom0 root filesystem, including sensitive system files and a fresh pool database copy. VM disks are not included. This archive is intended for installer-assisted recovery; it is not a running-host configuration restore.",k).wrap());
        }
        body = body.child(
            Button::new(if p.pending {
                "Inspecting archive file…"
            } else if p.mode == Mode::Restore {
                "Choose trusted backup…"
            } else {
                "Choose new archive file…"
            })
            .disabled(p.pending)
            .on_press_tx(choose)
            .build(k)
            .key("host-support-file"),
        );
        if let Some(f) = &p.file {
            body = body.child(
                caption(
                    format!(
                        "{}: {}",
                        if p.mode == Mode::Restore {
                            "Source"
                        } else {
                            "Destination"
                        },
                        f.name()
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
        }
    }
    if let Some(e) = &p.error {
        body = body.child(caption(e, k).color(k.c.danger.icon).wrap().role("alert"));
    }
    if s.demo {
        body = body.child(caption("Synthetic preview · no host requests are submitted", k).wrap());
    }
    let footer = row()
        .gap(8.0)
        .flex_wrap()
        .child(
            Button::new("Close")
                .on_press(|s: &mut State| s.host_support = None)
                .build(k)
                .key("host-support-close"),
        )
        .child(
            Button::new(if p.deletion.is_some() {
                "Activity history"
            } else {
                "Transfer history"
            })
            .on_press(|s: &mut State| {
                let deletion = s
                    .host_support
                    .as_ref()
                    .is_some_and(|p| p.deletion.is_some());
                s.host_support = None;
                if deletion {
                    activity::open(s);
                } else {
                    transfers::history(s);
                }
            })
            .build(k),
        )
        .child(if let Some(dump) = &p.deletion {
            Button::new("Permanently delete crashdump")
                .variant(ButtonVariant::Danger)
                .disabled(
                    s.demo
                        || !s.can_mutate()
                        || !current(s, p)
                        || s.resource_operations.len() >= 64
                        || p.confirmation.text.as_ref() != dump.uuid.to_string(),
                )
                .on_press(submit_deletion)
                .build(k)
                .key("host-crashdump-submit")
        } else if p.review.is_some() {
            Button::new(if p.mode == Mode::Restore {
                "Replace backup partition"
            } else {
                "Download private archive"
            })
            .variant(if p.mode == Mode::Restore {
                ButtonVariant::Danger
            } else {
                ButtonVariant::Outline
            })
            .disabled(
                s.demo
                    || !current(s, p)
                    || p.mode == Mode::Restore
                        && (!s.can_mutate() || p.confirmation.text.as_ref() != p.host.to_string()),
            )
            .on_press(submit)
            .build(k)
            .key("host-support-submit")
        } else {
            Button::new(if p.mode == Mode::Restore {
                "Review recovery staging"
            } else {
                "Review download"
            })
            .disabled(!current(s, p) || p.file.is_none() || action(p).is_none())
            .on_press(review)
            .build(k)
            .key("host-support-review")
        });
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Host backup and support")
        .modal(false, |s: &mut State| s.host_support = None)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(270.0, 760.0))
                .h((s.window_height - 40.0).clamp(260.0, 850.0))
                .fill(k.c.bg_base)
                .radius(10.0)
                .padding(18.0)
                .gap(12.0)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    s.page = Page::Hosts;
    let host = *s.inventory.as_ref().unwrap().hosts.keys().next().unwrap();
    s.rebuild();
    s.table.select(host.to_string());
    open(s);
    if let Some(p) = &mut s.host_support {
        p.mode = Mode::Support;
        p.selected.insert("host-crashdump-logs".into());
        p.overview = Some(HostSupportOverview {
            host: p.host,
            version: p.version.clone(),
            capabilities: vec![xenadmin_core::HostSupportCapability {
                key: "host-crashdump-logs".into(),
                privacy: "yes".into(),
                minimum_bytes: None,
                maximum_bytes: None,
                maximum_seconds: Some(30),
                default_selected: false,
            }],
            capabilities_error: None,
            crashdumps: vec![],
            crashdumps_error: None,
        });
    }
}

pub(super) fn restore_scene(s: &mut State) {
    scene(s);
    mode(s, Mode::Restore);
    if let Some(p) = &mut s.host_support {
        let archive = xenadmin_core::HostBackupSummary {
            host: p.host,
            version: p.version.clone(),
            entries: 12042,
            logical_bytes: 3 * 1024 * 1024 * 1024,
            sha256: [0x42; 32],
        };
        p.file =
            TransferFile::destination(std::env::temp_dir().join("example-host-backup.tar.gz")).ok();
        p.review = Some(TransferAction::Host {
            host: p.host,
            expected_version: p.version.clone(),
            operation: HostArchiveOperation::StageRestore { archive },
        });
    }
}

pub(super) fn crashdump_scene(s: &mut State) {
    scene(s);
    let dump = xenadmin_core::HostCrashdump {
        uuid: uuid::Uuid::from_u128(400).into(),
        timestamp: "20261108T00:00:00Z".into(),
        size: 8 * 1024 * 1024 * 1024,
    };
    if let Some(p) = &mut s.host_support {
        p.overview.as_mut().unwrap().crashdumps = vec![dump.clone()];
    }
    review_deletion(s, dump);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
        let mut s = init();
        scene(&mut s);
        s.window_width = size.0;
        s.window_height = size.1;
        let mut r = rune::Runtime::new(
            s,
            view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        settle(&mut r);
        r
    }
    fn settle(r: &mut rune::Runtime<State>) {
        for _ in 0..30 {
            r.frame(1.0 / 60.0);
        }
    }
    fn click(r: &mut rune::Runtime<State>, key: &str) {
        r.update(|_, tx| tx.scroll_into_view(key));
        settle(r);
        let (x, y) = r.find(key).unwrap_or_else(|| panic!("Missing {key}"));
        assert!(x >= 0.0 && x <= r.state.window_width && y >= 0.0 && y <= r.state.window_height);
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        settle(r);
    }
    fn select_file(r: &mut rune::Runtime<State>, path: std::path::PathBuf) {
        click(r, "host-support-file");
        assert_eq!(
            r.file_dialog().unwrap().0,
            if r.state.host_support.as_ref().unwrap().mode == Mode::Restore {
                rune::FileDialogKind::Open
            } else {
                rune::FileDialogKind::Save
            }
        );
        r.resolve_file_dialog(Some(path)).unwrap();
        for _ in 0..100 {
            r.update(|s, _| {
                poll(s);
            });
            settle(r);
            if !r.state.host_support.as_ref().unwrap().pending {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn host_archive_review_captures_target_and_keeps_compact_footer_visible() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut r = runtime(size);
            let captured = r.state.host_support.as_ref().unwrap().host;
            let d = tempfile::tempdir().unwrap();
            select_file(&mut r, d.path().join("private.tar"));
            click(&mut r, "host-support-review");
            let expected = TransferAction::Host {
                host: captured,
                expected_version: r.state.host_support.as_ref().unwrap().version.clone(),
                operation: HostArchiveOperation::Support {
                    entries: vec!["host-crashdump-logs".into()],
                },
            };
            assert_eq!(
                r.state.host_support.as_ref().unwrap().review,
                Some(expected)
            );
            let other = r
                .state
                .inventory
                .as_ref()
                .unwrap()
                .hosts
                .keys()
                .find(|id| **id != captured)
                .copied();
            if let Some(other) = other {
                r.update(|s, _| s.table.select(other.to_string()));
            }
            assert_eq!(r.state.host_support.as_ref().unwrap().host, captured);
            click(&mut r, "host-support-submit");
            assert!(r.state.resource_operations.is_empty());
            assert!(!d.path().join("private.tar").exists());
            click(&mut r, "host-support-close");
            assert!(r.state.host_support.is_none());
        }
    }
    #[test]
    fn crashdump_delete_review_captures_exact_dump_and_keeps_footer_visible() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut r = runtime(size);
            r.update(|s, _| crashdump_scene(s));
            settle(&mut r);
            let p = r.state.host_support.as_ref().unwrap();
            let captured_host = p.host;
            let captured_dump = p.deletion.clone().unwrap();
            assert!(p.file.is_none() && p.review.is_none());
            click(&mut r, "host-crashdump-submit");
            assert!(r.state.resource_operations.is_empty());
            r.update(|s, _| {
                s.table.select(uuid::Uuid::from_u128(999).to_string());
                s.host_support.as_mut().unwrap().confirmation =
                    TextEdit::new(captured_dump.uuid.to_string());
            });
            settle(&mut r);
            let p = r.state.host_support.as_ref().unwrap();
            assert_eq!(p.host, captured_host);
            assert_eq!(p.deletion.as_ref(), Some(&captured_dump));
            click(&mut r, "host-crashdump-submit");
            assert!(
                r.state.resource_operations.is_empty(),
                "synthetic reviews cannot mutate"
            );
            r.update(|s, _| mode(s, Mode::Backup));
            settle(&mut r);
            assert!(r.state.host_support.as_ref().unwrap().deletion.is_none());
            assert!(
                r.state
                    .host_support
                    .as_ref()
                    .unwrap()
                    .confirmation
                    .text
                    .is_empty()
            );
        }
    }
    #[test]
    fn crashdump_catalogue_pages_make_every_captured_candidate_reachable() {
        let mut r = runtime((513.0, 652.0));
        r.update(|s, _| {
            s.host_support
                .as_mut()
                .unwrap()
                .overview
                .as_mut()
                .unwrap()
                .crashdumps = (1..=65)
                .map(|id| xenadmin_core::HostCrashdump {
                    uuid: uuid::Uuid::from_u128(id).into(),
                    timestamp: "20261108T00:00:00Z".into(),
                    size: 4096,
                })
                .collect();
        });
        settle(&mut r);
        click(&mut r, "host-crashdump-next");
        assert_eq!(r.state.host_support.as_ref().unwrap().dump_page, 1);
        click(&mut r, "host-crashdump-next");
        assert_eq!(r.state.host_support.as_ref().unwrap().dump_page, 2);
        let last = uuid::Uuid::from_u128(65);
        click(&mut r, &format!("host-crashdump-delete-{last}"));
        assert_eq!(
            r.state
                .host_support
                .as_ref()
                .unwrap()
                .deletion
                .as_ref()
                .unwrap()
                .uuid
                .uuid(),
            last
        );
    }
    #[test]
    fn crashdump_delete_refuses_unadvertised_review_or_changed_host() {
        let mut r = runtime((513.0, 652.0));
        let invented = xenadmin_core::HostCrashdump {
            uuid: uuid::Uuid::from_u128(88).into(),
            timestamp: "20261108T00:00:00Z".into(),
            size: 4096,
        };
        r.update(|s, _| review_deletion(s, invented.clone()));
        assert!(r.state.host_support.as_ref().unwrap().deletion.is_none());
        r.update(|s, _| crashdump_scene(s));
        let host = r.state.host_support.as_ref().unwrap().host;
        r.update(|s, _| {
            Arc::make_mut(s.inventory.as_mut().unwrap())
                .hosts
                .remove(&host);
        });
        assert!(!current(&r.state, r.state.host_support.as_ref().unwrap()));
        click(&mut r, "host-crashdump-submit");
        assert!(r.state.resource_operations.is_empty());
    }
    #[test]
    fn stale_native_file_callback_cannot_set_a_reopened_host_review() {
        let mut r = runtime((513.0, 652.0));
        let old = r.state.host_support.as_ref().unwrap().nonce;
        click(&mut r, "host-support-file");
        r.update(|s, _| {
            s.host_support = None;
            scene(s);
        });
        assert_ne!(r.state.host_support.as_ref().unwrap().nonce, old);
        let d = tempfile::tempdir().unwrap();
        r.resolve_file_dialog(Some(d.path().join("old.tar")))
            .unwrap();
        settle(&mut r);
        assert!(r.state.host_support.as_ref().unwrap().file.is_none());
        assert!(!r.state.host_support.as_ref().unwrap().pending);
    }
    #[test]
    fn unknown_capabilities_and_changed_host_version_cannot_be_reviewed() {
        let mut r = runtime((513.0, 652.0));
        r.update(|s, _| {
            let p = s.host_support.as_mut().unwrap();
            p.overview.as_mut().unwrap().capabilities_error = Some("Unavailable".into());
        });
        assert!(action(r.state.host_support.as_ref().unwrap()).is_none());
        let host = r.state.host_support.as_ref().unwrap().host;
        r.update(|s, _| {
            Arc::make_mut(s.inventory.as_mut().unwrap())
                .hosts
                .get_mut(&host)
                .unwrap()
                .version = "changed".into();
            mode(s, Mode::Backup);
        });
        assert!(!current(&r.state, r.state.host_support.as_ref().unwrap()));
    }
    fn backup(path: &std::path::Path, host: HostId, version: &str) {
        use std::io::Write;
        let mut archive = tar::Builder::new(Vec::new());
        let data = format!("INSTALLATION_UUID='{host}'\nPRODUCT_VERSION='{version}'\n");
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o600);
        h.set_cksum();
        archive
            .append_data(&mut h, "etc/xensource-inventory", data.as_bytes())
            .unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&archive.into_inner().unwrap()).unwrap();
        std::fs::write(path, gzip.finish().unwrap()).unwrap();
    }
    #[test]
    fn restore_native_open_inspects_captured_host_and_requires_exact_confirmation() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut r = runtime(size);
            click(&mut r, "host-support-restore");
            let p = r.state.host_support.as_ref().unwrap();
            let host = p.host;
            let version = p.version.clone();
            let d = tempfile::tempdir().unwrap();
            let source = d.path().join("trusted.tar.gz");
            backup(&source, host, &version);
            select_file(&mut r, source.clone());
            let p = r.state.host_support.as_ref().unwrap();
            assert!(p.error.is_none(), "{:?}", p.error);
            assert_eq!(p.file.as_ref().unwrap().host_backup().unwrap().host, host);
            click(&mut r, "host-support-review");
            assert!(
                matches!(&r.state.host_support.as_ref().unwrap().review,Some(TransferAction::Host{operation:HostArchiveOperation::StageRestore{archive},..}) if archive.host==host)
            );
            click(&mut r, "host-support-submit");
            assert!(r.state.resource_operations.is_empty());
            r.update(|s, _| {
                s.host_support.as_mut().unwrap().confirmation = TextEdit::new(host.to_string())
            });
            click(&mut r, "host-support-submit");
            assert!(
                r.state.resource_operations.is_empty(),
                "synthetic review never submits"
            );
            assert!(source.exists());
            click(&mut r, "host-support-close");
        }
    }
    #[test]
    fn restore_rejects_wrong_host_and_cancels_obsolete_inspections() {
        let mut r = runtime((513.0, 652.0));
        click(&mut r, "host-support-restore");
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("wrong.tar.gz");
        backup(&path, HostId::from(uuid::Uuid::from_u128(999)), "8.3.0");
        select_file(&mut r, path.clone());
        let p = r.state.host_support.as_ref().unwrap();
        assert!(p.error.is_some());
        assert!(p.file.is_none());
        assert!(action(p).is_none());
        click(&mut r, "host-support-file");
        let cancel = r
            .state
            .host_support
            .as_ref()
            .unwrap()
            .inspection_cancel
            .clone()
            .unwrap();
        r.update(|s, _| mode(s, Mode::Backup));
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        r.resolve_file_dialog(Some(path)).unwrap();
        settle(&mut r);
        let p = r.state.host_support.as_ref().unwrap();
        assert!(p.file.is_none());
        assert!(!p.pending);
        assert_eq!(p.mode, Mode::Backup);
    }
}
