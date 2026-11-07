use crate::{Error, PoolId, VmId, xapi::Session};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OperationId(Uuid);
impl OperationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Display for OperationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerAction {
    Start,
    CleanShutdown,
    Reboot,
    HardShutdown,
    HardReboot,
}
impl PowerAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::CleanShutdown => "Shut down",
            Self::Reboot => "Reboot",
            Self::HardShutdown => "Force shut down",
            Self::HardReboot => "Force reboot",
        }
    }
    pub fn is_force(self) -> bool {
        matches!(self, Self::HardShutdown | Self::HardReboot)
    }
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::CleanShutdown => "clean_shutdown",
            Self::Reboot => "clean_reboot",
            Self::HardShutdown => "hard_shutdown",
            Self::HardReboot => "hard_reboot",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    Submitting,
    Dispatching,
    Tracking,
    CancellationRequested,
    Succeeded,
    Failed,
    Cancelled,
    OutcomeUnknown,
    Rejected,
}
impl OperationState {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Rejected
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationRecord {
    pub id: OperationId,
    pub pool: PoolId,
    pub vm: VmId,
    pub action: PowerAction,
    pub state: OperationState,
    pub progress: Option<f64>,
    pub detail: String,
    #[serde(default, skip_serializing)]
    pub(crate) task: Option<String>,
}

pub struct Operation {
    receiver: watch::Receiver<Arc<OperationRecord>>,
    cancel: CancellationToken,
    runtime: tokio::runtime::Handle,
}
impl Operation {
    pub fn latest(&self) -> Arc<OperationRecord> {
        self.receiver.borrow().clone()
    }
    pub fn request_cancel(&self) {
        self.cancel.cancel();
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            while receiver.changed().await.is_ok() {
                callback();
            }
        });
    }
}

pub(crate) struct PowerRequest {
    pub generation: u64,
    pub record: OperationRecord,
    sender: watch::Sender<Arc<OperationRecord>>,
    cancel: CancellationToken,
    updates: Option<mpsc::Sender<OperationRecord>>,
}
pub(crate) fn channel(
    id: OperationId,
    pool: PoolId,
    vm: VmId,
    generation: u64,
    action: PowerAction,
    runtime: tokio::runtime::Handle,
) -> (Operation, PowerRequest) {
    let record = OperationRecord {
        id,
        pool,
        vm,
        action,
        state: OperationState::Submitting,
        progress: None,
        detail: "Waiting for admission".into(),
        task: None,
    };
    let (sender, receiver) = watch::channel(Arc::new(record.clone()));
    let cancel = CancellationToken::new();
    (
        Operation {
            receiver,
            cancel: cancel.clone(),
            runtime,
        },
        PowerRequest {
            generation,
            record,
            sender,
            cancel,
            updates: None,
        },
    )
}

pub(crate) fn state_directory() -> PathBuf {
    std::env::var_os("XENADMIN_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME").map(|p| PathBuf::from(p).join("xenadmin-rs"))
        })
        .or_else(|| {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state/xenadmin-rs"))
        })
        .unwrap_or_else(|| PathBuf::from(".xenadmin-rs-state"))
}

pub(crate) struct Journal(rusqlite::Connection);
impl Journal {
    pub(crate) fn knows(&self, record: &OperationRecord) -> bool {
        self.load(record.id).ok().flatten().is_some_and(|existing| {
            existing.pool == record.pool
                && existing.vm == record.vm
                && existing.action == record.action
        })
    }
    pub(crate) fn open(directory: &Path) -> Result<Self, Error> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|_| Error::Journal)?;
        let path = directory.join("operations.sqlite3");
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|_| Error::Journal)?;
        let connection = rusqlite::Connection::open(path).map_err(|_| Error::Journal)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|_| Error::Journal)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS operations (id TEXT PRIMARY KEY, pool TEXT NOT NULL, vm TEXT NOT NULL, payload TEXT NOT NULL, task TEXT, terminal INTEGER NOT NULL);").map_err(|_| Error::Journal)?;
        Ok(Self(connection))
    }
    fn load(&self, id: OperationId) -> Result<Option<OperationRecord>, Error> {
        let value: Option<(String, Option<String>)> = self
            .0
            .query_row(
                "SELECT payload,task FROM operations WHERE id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| Error::Journal)?;
        value
            .map(|(payload, task)| {
                let mut record: OperationRecord =
                    serde_json::from_str(&payload).map_err(|_| Error::Journal)?;
                record.task = task;
                Ok(record)
            })
            .transpose()
    }
    fn admit(&mut self, record: &OperationRecord) -> Result<bool, Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            if let Some(existing) = self.load(record.id)? {
                if existing.pool != record.pool
                    || existing.vm != record.vm
                    || existing.action != record.action
                {
                    return Err(Error::OperationConflict);
                }
                return Ok(false);
            }
            let pending: i64 = self
                .0
                .query_row(
                    "SELECT COUNT(*) FROM operations WHERE terminal=0 AND pool=?1 AND vm=?2",
                    rusqlite::params![record.pool.to_string(), record.vm.to_string()],
                    |row| row.get(0),
                )
                .map_err(|_| Error::Journal)?;
            let total: i64 = self
                .0
                .query_row(
                    "SELECT COUNT(*) FROM operations WHERE terminal=0",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| Error::Journal)?;
            if pending > 0 || total >= 1000 {
                return Err(Error::Busy);
            }
            self.save(record)?;
            self.0.execute("DELETE FROM operations WHERE terminal=1 AND rowid NOT IN (SELECT rowid FROM operations WHERE terminal=1 ORDER BY rowid DESC LIMIT 500)", []).map_err(|_| Error::Journal)?;
            Ok(true)
        })();
        if result.is_ok() {
            self.0.execute_batch("COMMIT").map_err(|_| Error::Journal)?;
        } else {
            let _ = self.0.execute_batch("ROLLBACK");
        }
        result
    }
    fn save(&self, record: &OperationRecord) -> Result<(), Error> {
        self.0.execute("INSERT INTO operations(id,pool,vm,payload,task,terminal) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,task=excluded.task,terminal=excluded.terminal WHERE operations.terminal=0 OR excluded.terminal=1", rusqlite::params![record.id.to_string(), record.pool.to_string(), record.vm.to_string(), serde_json::to_string(record).map_err(|_| Error::Journal)?, record.task, record.state.terminal()]).map_err(|_| Error::Journal)?;
        Ok(())
    }
    pub(crate) fn history(&self) -> Result<Vec<OperationRecord>, Error> {
        let mut query = self
            .0
            .prepare("SELECT payload,task FROM operations ORDER BY rowid DESC LIMIT 1500")
            .map_err(|_| Error::Journal)?;
        let rows = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|_| Error::Journal)?;
        rows.map(|row| {
            let (payload, task) = row.map_err(|_| Error::Journal)?;
            let mut record: OperationRecord =
                serde_json::from_str(&payload).map_err(|_| Error::Journal)?;
            record.task = task;
            Ok(record)
        })
        .collect()
    }
}

pub(crate) async fn recover(
    session: Arc<Session>,
    directory: PathBuf,
    pool: PoolId,
    cancel: CancellationToken,
    updates: mpsc::Sender<OperationRecord>,
) {
    if !directory.join("operations.sqlite3").exists() {
        return;
    }
    let Ok(journal) = Journal::open(&directory) else {
        return;
    };
    let Ok(records) = journal.history() else {
        return;
    };
    for mut record in records.into_iter().filter(|record| record.pool == pool) {
        if !record.state.terminal() && record.task.is_none() {
            record.state = if record.state == OperationState::Submitting {
                OperationState::Failed
            } else {
                OperationState::OutcomeUnknown
            };
            record.detail="Interrupted operation. No saved task reference is available; inspect the host before taking further action.".into();
            let _ = journal.save(&record);
        }
        let _ = updates.send(record.clone()).await;
        if !record.state.terminal() && record.task.is_some() {
            let (sender, _) = watch::channel(Arc::new(record.clone()));
            let mut request = PowerRequest {
                generation: 0,
                record,
                sender,
                cancel: CancellationToken::new(),
                updates: Some(updates.clone()),
            };
            let (session, directory, cancel) = (session.clone(), directory.clone(), cancel.clone());
            tokio::spawn(async move {
                let Ok(mut journal) = Journal::open(&directory) else {
                    return;
                };
                request.record.state = OperationState::Tracking;
                request.record.detail =
                    "Reconciling an existing host task; the mutation is not repeated".into();
                if request.publish(&mut journal).await.is_err() {
                    return;
                }
                if let Err(error) = request.track(&session, &mut journal, &cancel).await {
                    request.record.state = OperationState::OutcomeUnknown;
                    request.record.detail = error.to_string();
                    let _ = request.publish(&mut journal).await;
                }
            });
        }
    }
}

impl PowerRequest {
    pub(crate) fn reject(mut self, error: Error) {
        self.record.state = OperationState::Rejected;
        self.record.detail = error.to_string();
        self.sender.send_replace(Arc::new(self.record));
    }
    async fn publish(&self, journal: &mut Journal) -> Result<(), Error> {
        journal.save(&self.record)?;
        self.sender.send_replace(Arc::new(self.record.clone()));
        if let Some(updates) = &self.updates {
            let _ = updates.send(self.record.clone()).await;
        }
        Ok(())
    }
    pub(crate) async fn run(
        mut self,
        session: Arc<Session>,
        reference: String,
        directory: PathBuf,
        connection_cancel: CancellationToken,
        updates: mpsc::Sender<OperationRecord>,
    ) {
        self.updates = Some(updates);
        let result = self
            .execute(&session, reference, &directory, &connection_cancel)
            .await;
        if let Err(error) = result {
            if self.record.state.terminal() {
                return;
            }
            self.record.state = if self.record.state == OperationState::Submitting {
                OperationState::Rejected
            } else if self.record.state == OperationState::Dispatching
                && matches!(error, Error::Api(_))
            {
                OperationState::Failed
            } else {
                OperationState::OutcomeUnknown
            };
            self.record.detail = error.to_string();
            if let Ok(journal) = Journal::open(&directory) {
                let _ = journal.save(&self.record);
            }
            self.sender.send_replace(Arc::new(self.record));
            let record = self.sender.borrow().as_ref().clone();
            if let Some(updates) = &self.updates {
                let _ = updates.send(record).await;
            }
        }
    }
    async fn execute(
        &mut self,
        session: &Session,
        reference: String,
        directory: &Path,
        connection_cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let mut journal = Journal::open(directory)?;
        let new = match journal.admit(&self.record) {
            Ok(new) => new,
            Err(error) => {
                self.record.state = OperationState::Rejected;
                self.record.detail = error.to_string();
                self.sender.send_replace(Arc::new(self.record.clone()));
                return Ok(());
            }
        };
        if !new {
            loop {
                self.record = journal.load(self.record.id)?.ok_or(Error::Journal)?;
                self.sender.send_replace(Arc::new(self.record.clone()));
                if self.record.state.terminal()
                    || self.record.state == OperationState::OutcomeUnknown
                {
                    return Ok(());
                }
                tokio::select! { _=connection_cancel.cancelled()=>return Ok(()), _=tokio::time::sleep(Duration::from_millis(250))=>{} }
            }
        }
        self.record.state = OperationState::Dispatching;
        self.record.detail = "Submitting to XAPI".into();
        self.publish(&mut journal).await?;
        let mut params = vec![json!(reference)];
        if self.record.action == PowerAction::Start {
            params.extend([json!(false), json!(false)]);
        }
        let method = format!("Async.VM.{}", self.record.action.wire());
        let task = tokio::select! {
            _=connection_cancel.cancelled()=>return Err(Error::Closed),
            result=session.call(&method,params)=>result?,
        };
        self.record.task = Some(
            task.as_str()
                .filter(|t| t.starts_with("OpaqueRef:"))
                .ok_or(Error::Protocol("invalid task reference"))?
                .into(),
        );
        self.record.state = OperationState::Tracking;
        self.record.detail = "Host task is running".into();
        self.publish(&mut journal).await?;
        self.track(session, &mut journal, connection_cancel).await
    }
    async fn track(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        connection_cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let reference = self
            .record
            .task
            .clone()
            .ok_or(Error::Protocol("task reference is unavailable"))?;
        let mut cancellation_sent = false;
        loop {
            let task = tokio::select! {_=connection_cancel.cancelled()=>return Err(Error::Closed), result=session.call("task.get_record",vec![json!(reference)])=>result?};
            self.record.progress = task["progress"]
                .as_f64()
                .or_else(|| task["progress"].as_str().and_then(|s| s.parse().ok()))
                .filter(|p| p.is_finite())
                .map(|p| p.clamp(0.0, 1.0));
            match task["status"].as_str() {
                Some("success") => {
                    self.record.state = OperationState::Succeeded;
                    self.record.detail = "Host task completed".into();
                    self.record.progress = Some(1.0);
                    self.publish(journal).await?;
                    return Ok(());
                }
                Some("failure") => {
                    self.record.state = OperationState::Failed;
                    self.record.detail =
                        "Host task failed. Inspect the host task log for details.".into();
                    self.publish(journal).await?;
                    return Ok(());
                }
                Some("cancelled") => {
                    self.record.state = OperationState::Cancelled;
                    self.record.detail = "Host confirmed task cancellation".into();
                    self.publish(journal).await?;
                    return Ok(());
                }
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("unknown task status")),
            }
            if self.cancel.is_cancelled() && !cancellation_sent {
                cancellation_sent = true;
                if task["allowed_operations"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == "cancel"))
                {
                    session.call("task.cancel", vec![json!(reference)]).await?;
                    self.record.state = OperationState::CancellationRequested;
                    self.record.detail = "Waiting for the host to confirm cancellation".into();
                } else {
                    self.record.detail = "This host task does not advertise cancellation".into();
                }
            }
            self.publish(journal).await?;
            tokio::select! {_=connection_cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> OperationRecord {
        OperationRecord {
            id: OperationId::new(),
            pool: PoolId::parse("11111111-1111-1111-1111-111111111111").unwrap(),
            vm: VmId::parse("22222222-2222-2222-2222-222222222222").unwrap(),
            action: PowerAction::Start,
            state: OperationState::Dispatching,
            progress: None,
            detail: "test".into(),
            task: None,
        }
    }
    #[test]
    fn journal_deduplicates_ids_and_reserves_unresolved_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let record = record();
        assert!(journal.admit(&record).unwrap());
        assert!(!journal.admit(&record).unwrap());
        let mut another = record.clone();
        another.id = OperationId::new();
        assert!(matches!(journal.admit(&another), Err(Error::Busy)));
        another.id = record.id;
        another.action = PowerAction::Reboot;
        assert!(matches!(
            journal.admit(&another),
            Err(Error::OperationConflict)
        ));
        drop(journal);
        let journal = Journal::open(dir.path()).unwrap();
        assert_eq!(
            journal.load(record.id).unwrap().unwrap().state,
            OperationState::Dispatching
        );
    }
}
