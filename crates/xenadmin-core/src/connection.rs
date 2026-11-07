use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use tokio::{
    runtime::Runtime,
    sync::{mpsc, watch},
};
use tokio_util::sync::CancellationToken;

use crate::{
    ConnectionOptions, Credentials, Error, Inventory,
    console::{self, ConsoleRequest},
    operations::{self, MutationRequest, OperationRecord, OperationUpdate},
    xapi::{Session, Store},
};

enum Request {
    Console(ConsoleRequest),
    Submit(MutationRequest),
    Control(crate::OperationId, crate::OperationControl),
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Synchronizing,
    Ready,
    Reconnecting,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    generation: u64,
    state: ConnectionState,
    inventory: Option<Arc<Inventory>>,
    error: Option<String>,
    updated_at: Option<SystemTime>,
}
#[derive(Clone, Copy, Debug)]
pub struct ObservedVm {
    pub(crate) pool: crate::PoolId,
    pub(crate) generation: u64,
    pub(crate) id: crate::VmId,
}
#[derive(Clone, Copy, Debug)]
pub struct ObservedSnapshot {
    pub(crate) pool: crate::PoolId,
    pub(crate) generation: u64,
    pub(crate) vm: crate::VmId,
    pub(crate) id: crate::SnapshotId,
}
impl std::fmt::Display for ObservedVm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.id.fmt(f)
    }
}
impl ConnectionSnapshot {
    pub fn observe_snapshot(
        &self,
        vm: crate::VmId,
        id: crate::SnapshotId,
    ) -> Result<ObservedSnapshot, Error> {
        let observed = self.observe_vm(vm)?;
        let snapshot = self
            .inventory
            .as_ref()
            .and_then(|inventory| inventory.snapshots.get(&id))
            .filter(|snapshot| snapshot.vm == Some(vm))
            .ok_or(Error::Stale)?;
        Ok(ObservedSnapshot {
            pool: observed.pool,
            generation: observed.generation,
            vm,
            id: snapshot.id,
        })
    }
    pub fn observe_vm(&self, id: crate::VmId) -> Result<ObservedVm, Error> {
        let inventory = self
            .inventory
            .as_ref()
            .filter(|_| self.is_current())
            .ok_or(Error::Stale)?;
        if !inventory.vms.contains_key(&id) {
            return Err(Error::Stale);
        }
        Ok(ObservedVm {
            pool: inventory.pool_id,
            generation: self.generation,
            id,
        })
    }
    pub fn state(&self) -> ConnectionState {
        self.state
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn inventory(&self) -> Option<&Arc<Inventory>> {
        self.inventory.as_ref()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn updated_at(&self) -> Option<SystemTime> {
        self.updated_at
    }
    pub fn is_current(&self) -> bool {
        self.state == ConnectionState::Ready
    }
}

pub struct Backend {
    runtime: Option<Runtime>,
    state_directory: PathBuf,
}
impl Backend {
    pub fn start() -> Result<Self, Error> {
        Self::with_state_directory(operations::state_directory())
    }
    pub fn with_state_directory(state_directory: PathBuf) -> Result<Self, Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("xenadmin-core")
            .build()
            .map_err(|_| Error::Runtime)?;
        Ok(Self {
            runtime: Some(runtime),
            state_directory,
        })
    }

    pub fn connect(&self, options: ConnectionOptions, credentials: Credentials) -> Connection {
        let read_only = options.read_only;
        let (sender, receiver) = watch::channel(Arc::new(ConnectionSnapshot {
            generation: 0,
            state: ConnectionState::Connecting,
            inventory: None,
            error: None,
            updated_at: None,
        }));
        let cancel = CancellationToken::new();
        let runtime = self.runtime.as_ref().expect("runtime exists before drop");
        let (requests, receiver_requests) = mpsc::channel(32);
        let (history_sender, history) = watch::channel(Arc::new(Vec::new()));
        runtime.spawn(run(
            options,
            credentials,
            sender,
            cancel.clone(),
            receiver_requests,
            self.state_directory.clone(),
            history_sender,
        ));
        Connection {
            read_only,
            receiver,
            cancel,
            runtime: runtime.handle().clone(),
            requests,
            history,
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

pub struct Connection {
    read_only: bool,
    receiver: watch::Receiver<Arc<ConnectionSnapshot>>,
    cancel: CancellationToken,
    runtime: tokio::runtime::Handle,
    requests: mpsc::Sender<Request>,
    history: watch::Receiver<Arc<Vec<OperationRecord>>>,
}
impl Connection {
    pub fn control_operation(
        &self,
        id: crate::OperationId,
        control: crate::OperationControl,
    ) -> Result<(), Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        if !self.latest().is_current() {
            return Err(Error::Stale);
        }
        self.requests
            .try_send(Request::Control(id, control))
            .map_err(|_| Error::Busy)
    }
    pub fn operation_history(&self) -> Arc<Vec<OperationRecord>> {
        self.history.borrow().clone()
    }
    pub fn submit(
        &self,
        id: crate::OperationId,
        command: crate::VmCommand,
    ) -> Result<crate::Operation, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let (observed, action) = command.normalize();
        let snapshot = self.latest();
        if !snapshot.is_current() {
            return Err(Error::Stale);
        }
        let inventory = snapshot.inventory().ok_or(Error::Stale)?;
        let vm = observed.id;
        if observed.pool != inventory.pool_id
            || observed.generation != snapshot.generation
            || !inventory.vms.contains_key(&vm)
        {
            return Err(Error::Stale);
        }
        let (operation, request) = operations::channel(
            id,
            inventory.pool_id,
            vm,
            snapshot.generation,
            action,
            self.runtime.clone(),
        );
        self.requests
            .try_send(Request::Submit(request))
            .map_err(|_| Error::Busy)?;
        Ok(operation)
    }
    pub fn open_console(&self, observed: ObservedVm) -> Result<crate::Console, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let vm = observed.id;
        let snapshot = self.latest();
        if !snapshot.is_current()
            || !snapshot
                .inventory()
                .is_some_and(|i| i.pool_id == observed.pool && i.vms.contains_key(&vm))
            || observed.generation != snapshot.generation
        {
            return Err(Error::Stale);
        }
        let (console, request) = console::channel(
            vm,
            snapshot.generation,
            self.cancel.child_token(),
            self.runtime.clone(),
        );
        self.requests
            .try_send(Request::Console(request))
            .map_err(|_| Error::Busy)?;
        Ok(console)
    }
    pub fn latest(&self) -> Arc<ConnectionSnapshot> {
        self.receiver.borrow().clone()
    }
    pub fn watch(&self) -> watch::Receiver<Arc<ConnectionSnapshot>> {
        self.receiver.clone()
    }
    pub fn disconnect(&self) {
        self.cancel.cancel();
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        let mut history = self.history.clone();
        self.runtime.spawn(async move {
            loop {
                tokio::select! {
                    result = history.changed() => { callback(); if result.is_err() { break; } }
                    result = receiver.changed() => {
                        callback();
                        if result.is_err() { break; }
                    }
                }
            }
        });
    }
    pub async fn wait_ready(&self, timeout: Duration) -> Result<Arc<Inventory>, Error> {
        let mut receiver = self.receiver.clone();
        tokio::time::timeout(timeout, async {
            loop {
                {
                    let snapshot = receiver.borrow_and_update();
                    if snapshot.is_current() {
                        return snapshot
                            .inventory
                            .clone()
                            .ok_or(Error::Protocol("ready inventory is missing"));
                    }
                    if snapshot.state == ConnectionState::Failed {
                        return Err(Error::Connection(
                            snapshot.error.clone().unwrap_or_default(),
                        ));
                    }
                    if snapshot.state == ConnectionState::Disconnected {
                        return Err(Error::Closed);
                    }
                }
                receiver.changed().await.map_err(|_| Error::Closed)?;
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
    pub async fn close(&self) {
        self.cancel.cancel();
        let mut receiver = self.receiver.clone();
        let _ = tokio::time::timeout(Duration::from_secs(4), async {
            while receiver.borrow_and_update().state != ConnectionState::Disconnected {
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn run(
    mut options: ConnectionOptions,
    credentials: Credentials,
    sender: watch::Sender<Arc<ConnectionSnapshot>>,
    cancel: CancellationToken,
    mut requests: mpsc::Receiver<Request>,
    directory: PathBuf,
    history: watch::Sender<Arc<Vec<OperationRecord>>>,
) {
    let (updates, mut operation_updates) = mpsc::channel::<OperationUpdate>(128);
    let mut records = BTreeMap::new();
    let mut generation;
    let mut last_inventory = None;
    let mut last_update = None;
    let mut backoff = 1;
    loop {
        generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        publish(
            &sender,
            generation,
            if last_inventory.is_some() {
                ConnectionState::Reconnecting
            } else {
                ConnectionState::Connecting
            },
            &last_inventory,
            None,
            last_update,
        );
        let result = tokio::select! {
            _=cancel.cancelled()=>break,
            result=Session::login(&options,&credentials)=>result,
        };
        let error = match result {
            Ok(session) => {
                let session = Arc::new(session);
                let active = cancel.child_token();
                publish(
                    &sender,
                    generation,
                    ConnectionState::Synchronizing,
                    &last_inventory,
                    None,
                    last_update,
                );
                let mut store = Store::default();
                let mut token = String::new();
                let mut recovered = false;
                let mut journal_error = None;
                let mut workers = tokio::task::JoinSet::new();
                let mut worker_counts = BTreeMap::<crate::OperationId, usize>::new();
                let start_worker =
                    |request: MutationRequest,
                     reference: Option<String>,
                     workers: &mut tokio::task::JoinSet<crate::OperationId>,
                     counts: &mut BTreeMap<crate::OperationId, usize>| {
                        let id = request.record.id;
                        *counts.entry(id).or_default() += 1;
                        let (session, directory, active, updates) = (
                            session.clone(),
                            directory.clone(),
                            active.clone(),
                            updates.clone(),
                        );
                        workers.spawn(async move {
                            request
                                .run(session, reference, directory, active, updates)
                                .await;
                            id
                        });
                    };
                let mut refresh = tokio::time::interval(Duration::from_secs(2));
                refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let events = next_events(session.clone(), token.clone());
                tokio::pin!(events);
                let error = loop {
                    let batch = tokio::select! {
                        _=cancel.cancelled()=>break None,
                        done=workers.join_next(), if !workers.is_empty()=>{
                            match done {
                                Some(Ok(id))=>{if let Some(count)=worker_counts.get_mut(&id) { *count-=1; if *count==0 {worker_counts.remove(&id);} }},
                                Some(Err(_))=>break Some(Error::Connection("An operation worker stopped unexpectedly. Reconnect to reconcile its journal.".into())),
                                None=>{},
                            }
                            continue;
                        }
                        Some(update)=operation_updates.recv()=>{
                            match update {
                                OperationUpdate::Coordinator(endpoint)=>break Some(Error::Coordinator(endpoint)),
                                OperationUpdate::Record(record)=>{merge_record(&mut records,record);publish_history(&history,&mut records);},
                            }
                            continue;
                        }
                        _=refresh.tick(), if recovered && !options.read_only=>{
                            if !directory.join("operations.sqlite3").exists() {continue;}
                            if let Some(inventory)=&last_inventory {
                                match operations::Journal::open(&directory).and_then(|journal|journal.history()) {
                                    Ok(rows)=>{
                                        journal_error=None;
                                        for record in rows.into_iter().filter(|record|record.pool==inventory.pool_id) {
                                            if !record.state.terminal() && record.has_task()
                                                && record.state!=crate::OperationState::OutcomeUnknown
                                                && !worker_counts.contains_key(&record.id) {
                                                start_worker(MutationRequest::tracking(record.clone()),None,&mut workers,&mut worker_counts);
                                            }
                                            merge_record(&mut records,record);
                                        }
                                        publish_history(&history,&mut records);
                                    },
                                    Err(error)=>{
                                        journal_error=Some(error.to_string());
                                        publish(&sender,generation,ConnectionState::Ready,&last_inventory,journal_error.clone(),last_update);
                                    },
                                }
                            }
                            continue;
                        }
                        Some(request)=requests.recv()=>{
                            match request {
                                Request::Submit(request)=>{
                                    if options.read_only {request.reject(Error::ReadOnly);}
                                    else if request.generation!=generation || token.is_empty() || !last_inventory.as_ref().is_some_and(|inventory|inventory.pool_id==request.record.pool) {
                                        request.reject(Error::Stale);
                                    } else if journal_error.is_some() {request.reject(Error::Journal);}
                                    else if !store.allows(request.record.vm,&request.record.action)
                                        && !operations::Journal::open(&directory).is_ok_and(|journal|journal.knows(&request.record)) {
                                        request.reject(Error::UnavailableAction);
                                    } else if let Some(reference)=store.vm_reference(request.record.vm) {
                                        start_worker(request,Some(reference),&mut workers,&mut worker_counts);
                                    } else {request.reject(Error::Stale);}
                                }
                                Request::Console(request)=>{
                                    if options.read_only {request.fail(Error::ReadOnly);}
                                    else if request.generation==generation && !token.is_empty() {
                                        if let (Some(reference),Some(inventory))=(store.vm_reference(request.vm),last_inventory.clone()) {
                                            tokio::spawn(request.run(session.clone(),reference,inventory,active.clone()));
                                        } else {request.fail(Error::Stale);}
                                    } else {request.fail(Error::Stale);}
                                }
                                Request::Control(id,control)=>{
                                    let result=(||{
                                        if options.read_only {return Err(Error::ReadOnly);}
                                        let pool=last_inventory.as_ref().filter(|_|!token.is_empty()).ok_or(Error::Stale)?.pool_id;
                                        let journal=operations::Journal::open(&directory)?;
                                        let record=match control {
                                            crate::OperationControl::RequestCancellation=>journal.control(pool,id,false)?,
                                            crate::OperationControl::AcknowledgeUnknown=>journal.control(pool,id,true)?,
                                            crate::OperationControl::Recheck=>journal.load(id)?.filter(|record|record.pool==pool && !record.state.terminal() && record.has_task()).ok_or(Error::UnavailableAction)?,
                                        };
                                        if !record.state.terminal() && record.has_task() && !worker_counts.contains_key(&id) {
                                            start_worker(MutationRequest::tracking(record.clone()),None,&mut workers,&mut worker_counts);
                                        }
                                        merge_record(&mut records,record);
                                        publish_history(&history,&mut records);
                                        Ok::<_,Error>(())
                                    })();
                                    if let Err(error)=result {
                                        publish(&sender,generation,ConnectionState::Ready,&last_inventory,Some(error.to_string()),last_update);
                                    }
                                }
                            }
                            continue;
                        }
                        result=&mut events=>match result {Ok(batch)=>batch,Err(error)=>break Some(error)},
                    };
                    if batch.token.is_empty() {
                        break Some(Error::Protocol("empty event continuation token"));
                    }
                    if !batch.events.is_empty() || token.is_empty() {
                        match store.apply(&batch) {
                            Ok(inventory) => {
                                if !recovered && !options.read_only {
                                    match operations::recover(&directory, inventory.pool_id) {
                                        Ok(rows) => {
                                            for record in rows {
                                                if !record.state.terminal() && record.has_task() {
                                                    start_worker(
                                                        MutationRequest::tracking(record.clone()),
                                                        None,
                                                        &mut workers,
                                                        &mut worker_counts,
                                                    );
                                                }
                                                merge_record(&mut records, record);
                                            }
                                            publish_history(&history, &mut records);
                                        }
                                        Err(error) => journal_error = Some(error.to_string()),
                                    }
                                    recovered = true;
                                }
                                last_inventory = Some(Arc::new(inventory));
                                last_update = Some(SystemTime::now());
                                backoff = 1;
                                publish(
                                    &sender,
                                    generation,
                                    ConnectionState::Ready,
                                    &last_inventory,
                                    journal_error.clone(),
                                    last_update,
                                );
                            }
                            Err(error) => break Some(error),
                        }
                    }
                    token = batch.token;
                    events.set(next_events(session.clone(), token.clone()));
                };
                active.cancel();
                let cleanup = tokio::time::sleep(Duration::from_secs(3));
                tokio::pin!(cleanup);
                while !workers.is_empty() {
                    tokio::select! {
                        _=workers.join_next()=>{},
                        Some(OperationUpdate::Record(record))=operation_updates.recv()=>{merge_record(&mut records,record);publish_history(&history,&mut records);},
                        _=&mut cleanup=>{workers.abort_all();break;},
                    }
                }
                let _ = tokio::time::timeout(Duration::from_secs(3), session.logout()).await;
                match error {
                    Some(error) => error,
                    None => break,
                }
            }
            Err(error) => error,
        };
        let retry = error.retryable();
        if let Error::Coordinator(endpoint) = &error {
            options = options.for_endpoint(endpoint.clone());
        }
        publish(
            &sender,
            generation,
            if retry {
                ConnectionState::Reconnecting
            } else {
                ConnectionState::Failed
            },
            &last_inventory,
            Some(error.to_string()),
            last_update,
        );
        if !retry {
            return;
        }
        tokio::select! {_=cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(backoff))=>{},}
        backoff = (backoff * 2).min(30);
    }
    publish(
        &sender,
        generation,
        ConnectionState::Disconnected,
        &last_inventory,
        None,
        last_update,
    );
}

fn merge_record(
    records: &mut BTreeMap<crate::OperationId, OperationRecord>,
    record: OperationRecord,
) {
    if records
        .get(&record.id)
        .is_none_or(|old| record.supersedes(old))
    {
        records.insert(record.id, record);
    }
}
fn publish_history(
    history: &watch::Sender<Arc<Vec<OperationRecord>>>,
    records: &mut BTreeMap<crate::OperationId, OperationRecord>,
) {
    while records.len() > 1500 {
        if let Some(id) = records
            .iter()
            .filter(|(_, record)| record.state.terminal())
            .min_by_key(|(_, record)| record.created_at_ms)
            .map(|(id, _)| *id)
        {
            records.remove(&id);
        } else {
            break;
        }
    }
    let mut ordered: Vec<_> = records.values().cloned().collect();
    ordered.sort_by_key(|record| record.created_at_ms);
    history.send_replace(Arc::new(ordered));
}

async fn next_events(
    session: Arc<Session>,
    token: String,
) -> Result<crate::xapi::EventBatch, Error> {
    session.events(&token).await
}

fn publish(
    sender: &watch::Sender<Arc<ConnectionSnapshot>>,
    generation: u64,
    state: ConnectionState,
    inventory: &Option<Arc<Inventory>>,
    error: Option<String>,
    updated_at: Option<SystemTime>,
) {
    sender.send_replace(Arc::new(ConnectionSnapshot {
        generation,
        state,
        inventory: inventory.clone(),
        error,
        updated_at,
    }));
}
