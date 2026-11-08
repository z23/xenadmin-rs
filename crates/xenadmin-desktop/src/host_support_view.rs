use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    HostArchiveOperation, HostId, HostSupportOverview, HostSupportQuery, ObservedResource,
    OperationId, ResourceTarget, TransferAction, TransferFile, TransferRequest,
};

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
    support: bool,
    selected: BTreeSet<String>,
    pending: bool,
    result: Option<std::sync::mpsc::Receiver<Result<TransferFile, String>>>,
    file: Option<TransferFile>,
    review: Option<TransferAction>,
    error: Option<String>,
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
        support: false,
        selected: BTreeSet::new(),
        pending: false,
        result: None,
        file: None,
        review: None,
        error: None,
    });
    refresh(s);
}
fn refresh(s: &mut State) {
    let Some(p) = &mut s.host_support else { return };
    p.epoch += 1;
    p.file = None;
    p.review = None;
    p.pending = false;
    p.result = None;
    p.overview = None;
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
    let operation = if p.support {
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
fn mode(s: &mut State, support: bool) {
    if let Some(p) = &mut s.host_support {
        p.support = support;
        p.epoch += 1;
        p.pending = false;
        p.result = None;
        p.file = None;
        p.review = None;
        p.error = None;
    }
}
fn choose(s: &mut State, tx: &mut Tx) {
    let Some(p) = &mut s.host_support else { return };
    if p.pending {
        return;
    }
    let nonce = p.nonce;
    let epoch = p.epoch;
    let support = p.support;
    p.pending = true;
    p.file = None;
    p.review = None;
    p.error = None;
    tx.save_file(
        rune::FileDialogOptions {
            title: "Save a new private host archive".into(),
            file_name: Some(
                if support {
                    "host-support.tar"
                } else {
                    "host-backup.tar.gz"
                }
                .into(),
            ),
            ..Default::default()
        },
        move |s: &mut State, result, _| {
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
                            let _ = sender
                                .send(TransferFile::destination(path).map_err(|e| e.to_string()));
                            if let Some(w) = wake {
                                w();
                            }
                        })
                        .is_ok()
                    {
                        p.result = Some(receiver);
                    } else {
                        p.pending = false;
                        p.error = Some("Cannot inspect the destination".into());
                    }
                }
                Ok(None) => p.pending = false,
                Err(e) => {
                    p.pending = false;
                    p.error = Some(e.to_string());
                }
            }
        },
    );
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
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.host_support else {
        return el();
    };
    let mut body = col()
        .gap(12.0)
        .child(heading("Host backup and support", k))
        .child(caption(format!("{} · {}\nHost {}", p.name, p.version, p.host), k).wrap_anywhere());
    if !current(s, p) {
        body=body.child(caption("The captured connection or host version changed. Close and reopen before downloading.",k).color(k.c.danger.icon).wrap());
    }
    if let Some(review) = &p.review {
        body=body.child(heading(review.label(),k)).child(caption(format!("Save as {}",p.file.as_ref().map(TransferFile::name).unwrap_or_default()),k).wrap_anywhere()).child(caption("Creates a new private file (0600). An existing file is never overwritten. The selected host's task and archive format are verified before the file is published.",k).wrap());
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
                        .disabled(!p.support)
                        .on_press(|s: &mut State| mode(s, false))
                        .build(k)
                        .key("host-support-backup"),
                )
                .child(
                    Button::new("Support bundle")
                        .disabled(p.support)
                        .on_press(|s: &mut State| mode(s, true))
                        .build(k)
                        .key("host-support-bundle"),
                ),
        );
        if p.support {
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
                    for dump in v.crashdumps.iter().take(32) {
                        body = body.child(
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
                        );
                    }
                }
            }
        } else {
            body=body.child(caption("Downloads the host's dom0 root filesystem, including sensitive system files and a fresh pool database copy. VM disks are not included. This archive is intended for installer-assisted recovery; it is not a running-host configuration restore.",k).wrap());
        }
        body = body.child(
            Button::new(if p.pending {
                "Selecting destination…"
            } else {
                "Choose new archive file…"
            })
            .disabled(p.pending)
            .on_press_tx(choose)
            .build(k)
            .key("host-support-file"),
        );
        if let Some(f) = &p.file {
            body = body.child(caption(format!("Destination: {}", f.name()), k).wrap_anywhere());
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
            Button::new("Transfer history")
                .on_press(|s: &mut State| {
                    s.host_support = None;
                    transfers::history(s);
                })
                .build(k),
        )
        .child(if p.review.is_some() {
            Button::new("Download private archive")
                .disabled(s.demo || !current(s, p))
                .on_press(submit)
                .build(k)
                .key("host-support-submit")
        } else {
            Button::new("Review download")
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
        p.support = true;
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
        assert_eq!(r.file_dialog().unwrap().0, rune::FileDialogKind::Save);
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
            mode(s, false);
        });
        assert!(!current(&r.state, r.state.host_support.as_ref().unwrap()));
    }
}
