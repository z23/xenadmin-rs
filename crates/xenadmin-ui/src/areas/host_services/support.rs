//! Host backup, status report, recovery staging and crashdump deletion:
//! the logic of the Rune module `host_support_view.rs`, carried over.
//!
//! One panel for one server. Rune's panel switched between three modes with
//! a row of buttons; here each mode is a command and a dialog of its own,
//! so the mode is fixed when the panel opens (`mode()` remains: it is what
//! Back from a review runs, and it resets what the Rune mode switch reset).
//!
//! What must not be lost (map, hazards 15 to 24):
//!
//! - a file prompt answer is accepted only for the same `nonce` and
//!   `epoch`; `mode`, `refresh` and `review_deletion` bump the epoch and
//!   cancel a running inspection, and so does dropping the panel;
//! - a submission needs the review to equal a freshly computed `action`;
//!   toggling a category clears the review;
//! - downloads work on a read-only connection; only staging needs write
//!   access and the typed host UUID;
//! - `current` compares the product version as well as both observations,
//!   and a query result is checked against host and version again;
//! - the default categories replace the selection when the overview comes;
//! - a crashdump under review must be in the captured overview, compared by
//!   full value; deletion is refused at 64 resource operation handles of
//!   the session list, as written in Rune.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use xenadmin_core::{
    Error, HostArchiveOperation, HostCrashdump, HostId, HostSupportOverview, HostSupportQuery,
    ObservedResource, OperationId, PoolId, ResourceCommand, ResourceTarget, TransferAction,
    TransferFile, TransferRequest,
};

use crate::prelude::*;

/// What the dialog says once the server it was opened for has moved on.
pub const STALE: &str =
    "The captured connection or host version changed. Close and reopen before continuing.";

/// Crashdumps shown at once.
pub const DUMPS_PER_PAGE: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Backup,
    Support,
    Restore,
}

pub struct Panel {
    pub nonce: OperationId,
    pub epoch: u64,
    /// The pool of the server: the panel follows it, not the scope session.
    pub pool_id: PoolId,
    pub host: HostId,
    pub name: String,
    pub version: String,
    pub observed: Option<ObservedResource>,
    pub pool: Option<ObservedResource>,
    pub query: Option<HostSupportQuery>,
    pub overview: Option<HostSupportOverview>,
    pub mode: Mode,
    pub confirmation: String,
    pub inspection_cancel: Option<Arc<AtomicBool>>,
    pub selected: BTreeSet<String>,
    pub pending: bool,
    pub result: Option<mpsc::Receiver<Result<TransferFile, String>>>,
    pub file: Option<TransferFile>,
    pub review: Option<TransferAction>,
    pub deletion: Option<HostCrashdump>,
    pub dump_page: usize,
    pub error: Option<String>,
}

impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(c) = &self.inspection_cancel {
            c.store(true, Ordering::Relaxed);
        }
    }
}

fn cancel_inspection(p: &mut Panel) {
    if let Some(c) = p.inspection_cancel.take() {
        c.store(true, Ordering::Relaxed);
    }
}

impl Panel {
    /// The panel for a server of a pool whose inventory is known. `None`:
    /// the object is no such server.
    pub fn capture(m: &Model, object: &ObjectRef, mode: Mode) -> Option<Self> {
        if object.kind != ObjectKind::Host {
            return None;
        }
        let pool_id = object.pool?;
        let host = object.host_id()?;
        let i = m.inventory_for(pool_id)?;
        let h = i.hosts.get(&host)?;
        let snapshot = m.snapshot_for(pool_id);
        let observe = |target| {
            snapshot
                .as_ref()
                .and_then(|v| v.observe_resource(target).ok())
        };
        Some(Self {
            nonce: OperationId::new(),
            epoch: 0,
            pool_id,
            host,
            name: h.name.clone(),
            version: h.version.clone(),
            observed: observe(ResourceTarget::Host(host)),
            pool: observe(ResourceTarget::Pool(i.pool_id)),
            query: None,
            overview: None,
            mode,
            confirmation: String::new(),
            inspection_cancel: None,
            selected: BTreeSet::new(),
            pending: false,
            result: None,
            file: None,
            review: None,
            deletion: None,
            dump_page: 0,
            error: None,
        })
    }
}

/// Why a panel just captured could never reach a submission, whatever the
/// user chose: what Rune showed inside the dialog right after opening (the
/// error of its first `refresh`), and its disabled "Stage recovery…".
pub fn blocked(m: &Model, p: &Panel) -> Option<String> {
    if synthetic(m, p) {
        return None;
    }
    if m.connection_for(p.pool_id).is_none() {
        return Some(Error::Closed.to_string());
    }
    if p.observed.is_none() || p.pool.is_none() {
        return Some(Error::Stale.to_string());
    }
    (p.mode == Mode::Restore && !m.can_mutate_pool(p.pool_id))
        .then(|| super::commands::NOT_WRITABLE.to_owned())
}

/// Opens the panel in one mode. `false`: the object is no known server.
pub fn open(m: &mut Model, object: &ObjectRef, mode: Mode) -> bool {
    let Some(panel) = Panel::capture(m, object, mode) else {
        return false;
    };
    m.areas.host_services.support = Some(panel);
    refresh(m);
    true
}

pub fn close(m: &mut Model) {
    m.areas.host_services.support = None;
}

pub fn synthetic(m: &Model, p: &Panel) -> bool {
    m.is_synthetic_pool(p.pool_id)
}

/// Resets the panel and, for the status report, asks the server which
/// diagnostic categories it offers and which crashdumps it keeps. Rune asked
/// in every mode because its panel could switch; the backup and the staging
/// dialog show nothing of the answer, so they do not ask.
pub fn refresh(m: &mut Model) {
    let Some(p) = &m.areas.host_services.support else {
        return;
    };
    let synthetic = synthetic(m, p);
    let connection = m.connection_for(p.pool_id);
    let waker = m.waker.clone();
    let Some(p) = &mut m.areas.host_services.support else {
        return;
    };
    cancel_inspection(p);
    p.confirmation.clear();
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
    if synthetic || p.mode != Mode::Support {
        return;
    }
    let result = p
        .observed
        .ok_or(Error::Stale)
        .and_then(|o| connection.as_ref().ok_or(Error::Closed)?.host_support(o));
    match result {
        Ok(q) => {
            if let Some(w) = waker {
                q.notify_on_change(w);
            }
            p.query = Some(q);
        }
        Err(e) => p.error = Some(e.to_string()),
    }
}

pub fn current(m: &Model, p: &Panel) -> bool {
    let fresh = |o: ObservedResource| {
        m.snapshot_for(p.pool_id)
            .and_then(|v| v.observe_resource(o.target()).ok())
            == Some(o)
    };
    m.inventory_for(p.pool_id)
        .is_some_and(|i| i.hosts.get(&p.host).is_some_and(|h| h.version == p.version))
        && (synthetic(m, p) || p.observed.is_some_and(fresh) && p.pool.is_some_and(fresh))
}

pub fn poll(m: &mut Model) -> bool {
    let Some(p) = &mut m.areas.host_services.support else {
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
                    Some("Host identity or version changed; close and reopen this review".into());
            }
            Err(e) => p.error = Some(e),
        }
    }
    if let Some(result) = p.result.as_ref().and_then(|r| match r.try_recv() {
        Ok(v) => Some(v),
        Err(mpsc::TryRecvError::Empty) => None,
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

pub fn action(p: &Panel) -> Option<TransferAction> {
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

/// The Rune mode switch. The overview, the chosen categories and the page
/// of the crashdump list are kept; everything of a file or a review goes.
pub fn mode(m: &mut Model, mode: Mode) {
    if let Some(p) = &mut m.areas.host_services.support {
        cancel_inspection(p);
        p.mode = mode;
        p.confirmation.clear();
        p.epoch += 1;
        p.pending = false;
        p.result = None;
        p.file = None;
        p.review = None;
        p.deletion = None;
        p.error = None;
    }
}

/// Back from a review (Rune had none; hazard 23 says what it must reset):
/// the mode switch to the mode the panel is in.
pub fn back(m: &mut Model) {
    if let Some(mode_now) = m.areas.host_services.support.as_ref().map(|p| p.mode) {
        mode(m, mode_now);
    }
}

/// Selects or deselects one diagnostic category. The review goes.
pub fn toggle(m: &mut Model, key: &str) {
    if let Some(p) = &mut m.areas.host_services.support {
        if !p.selected.remove(key) {
            p.selected.insert(key.to_owned());
        }
        p.review = None;
    }
}

/// Asks for the archive file: the backup to stage (an existing file, which
/// is then inspected on a worker thread without being extracted) or the new
/// file of a download.
pub fn choose(m: &mut Model) {
    let Some(p) = &mut m.areas.host_services.support else {
        return;
    };
    if p.pending {
        return;
    }
    cancel_inspection(p);
    let cancel = Arc::new(AtomicBool::new(false));
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
    p.confirmation.clear();
    // The answer comes in a later update: the panel may be closed, reopened
    // or in another state by then, and then the answer is dropped.
    let same = move |m: &mut Model| -> bool {
        m.areas
            .host_services
            .support
            .as_ref()
            .is_some_and(|p| p.nonce == nonce && p.epoch == epoch)
    };
    let reply = move |m: &mut Model, path: Option<PathBuf>| {
        if !same(m) {
            return;
        }
        let wake = m.waker.clone();
        let Some(p) = &mut m.areas.host_services.support else {
            return;
        };
        let Some(path) = path else {
            p.pending = false;
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = cancel.clone();
        let version = version.clone();
        let spawned = std::thread::Builder::new()
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
            });
        if spawned.is_ok() {
            p.result = Some(receiver);
        } else {
            p.pending = false;
            p.error = Some("Cannot inspect the archive file".into());
        }
    };
    let failed = move |m: &mut Model, error: &str| {
        if !same(m) {
            return;
        }
        if let Some(p) = &mut m.areas.host_services.support {
            p.pending = false;
            p.error = Some(error.to_owned());
        }
    };
    let request = if mode == Mode::Restore {
        FileRequest::open("Select a trusted backup from this host and version", reply)
            .filter("Host filesystem backup", ["gz", "tgz"])
    } else {
        FileRequest::save(
            "Save a new private host archive",
            if mode == Mode::Support {
                "host-support.tar"
            } else {
                "host-backup.tar.gz"
            },
            reply,
        )
    };
    m.choose_file(request.on_failure(failed));
}

pub fn review(m: &mut Model) {
    let valid = m
        .areas
        .host_services
        .support
        .as_ref()
        .is_some_and(|p| current(m, p));
    if valid
        && let Some(p) = &mut m.areas.host_services.support
        && p.file.is_some()
    {
        p.review = action(p);
    }
}

pub fn submit(m: &mut Model) {
    let result = (|| {
        let p = m.areas.host_services.support.as_ref().ok_or(Error::Stale)?;
        if synthetic(m, p) || !current(m, p) {
            return Err(Error::Stale);
        }
        let action = p
            .review
            .clone()
            .filter(|a| Some(a.clone()) == action(p))
            .ok_or(Error::Stale)?;
        if action.is_host_restore()
            && (!m.can_mutate_pool(p.pool_id) || p.confirmation != p.host.to_string())
        {
            return Err(Error::ReadOnly);
        }
        m.connection_for(p.pool_id).ok_or(Error::Closed)?.transfer(
            OperationId::new(),
            p.pool.ok_or(Error::Stale)?,
            TransferRequest {
                action,
                file: p.file.clone().ok_or(Error::TransferFile)?,
            },
        )
    })();
    match result {
        Ok(op) => {
            m.track_resource(op);
            close(m);
            crate::areas::transfers::history(m);
        }
        Err(e) => {
            if let Some(p) = &mut m.areas.host_services.support {
                p.error = Some(e.to_string());
            }
        }
    }
}

pub fn review_deletion(m: &mut Model, dump: HostCrashdump) {
    let Some(p) = &mut m.areas.host_services.support else {
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
    p.confirmation.clear();
    p.deletion = Some(dump);
}

pub fn submit_deletion(m: &mut Model) {
    let result = (|| {
        let p = m.areas.host_services.support.as_ref().ok_or(Error::Stale)?;
        let dump = p.deletion.as_ref().ok_or(Error::Stale)?;
        if synthetic(m, p)
            || m.resource_operations.len() >= 64
            || !m.can_mutate_pool(p.pool_id)
            || !current(m, p)
            || p.confirmation != dump.uuid.to_string()
        {
            return Err(Error::Stale);
        }
        m.connection_for(p.pool_id)
            .ok_or(Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::DeleteHostCrashdump {
                    host: p.observed.ok_or(Error::Stale)?,
                    dump: dump.clone(),
                },
            )
    })();
    match result {
        Ok(op) => {
            m.track_resource(op);
            close(m);
            crate::areas::events::show(m);
        }
        Err(e) => {
            if let Some(p) = &mut m.areas.host_services.support {
                p.error = Some(e.to_string());
            }
        }
    }
}

/// The history button of the footer: the panel closes and the Events tab
/// shows what was started from it (Rune: the transfer history, or the
/// activity history from a deletion review).
pub fn history(m: &mut Model) {
    let deletion = m
        .areas
        .host_services
        .support
        .as_ref()
        .is_some_and(|p| p.deletion.is_some());
    close(m);
    if deletion {
        crate::areas::events::show(m);
    } else {
        crate::areas::transfers::history(m);
    }
}

/// The page of the crashdump list that is showing, and how many there are.
pub fn dump_pages(p: &Panel) -> (usize, usize) {
    let count = p.overview.as_ref().map_or(0, |v| v.crashdumps.len());
    let pages = count.div_ceil(DUMPS_PER_PAGE).max(1);
    (p.dump_page.min(pages - 1), pages)
}

pub fn previous_dumps(m: &mut Model) {
    if let Some(p) = &mut m.areas.host_services.support {
        p.dump_page = p.dump_page.saturating_sub(1);
    }
}

pub fn next_dumps(m: &mut Model) {
    if let Some(p) = &mut m.areas.host_services.support {
        let (page, pages) = dump_pages(p);
        p.dump_page = (page + 1).min(pages - 1);
    }
}
