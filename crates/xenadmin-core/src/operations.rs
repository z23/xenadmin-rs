#[path = "vm_workflow_batch_journal.rs"]
mod workflow_batch_journal;
#[path = "vm_workflow_children.rs"]
pub(crate) mod workflow_children;
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

#[path = "device_lifecycle_operations.rs"]
mod device_lifecycle_operations;
#[path = "migration_evidence.rs"]
mod migration_evidence;
#[path = "migration_operations.rs"]
mod migration_operations;
#[path = "migration_tracking.rs"]
mod migration_tracking;
#[path = "snapshot_operations.rs"]
mod snapshot_operations;
#[path = "snapshot_workflow_operations.rs"]
mod snapshot_workflow_operations;
#[path = "vm_lifecycle_operations.rs"]
mod vm_lifecycle_operations;
#[path = "vm_recovery_operations.rs"]
mod vm_recovery_operations;

#[derive(Clone, Debug)]
pub struct SnapshotName(String);
impl SnapshotName {
    pub fn new(value: &str) -> Result<Self, Error> {
        if value.chars().any(char::is_control) {
            return Err(Error::InvalidSnapshotName);
        }
        let value = value.trim();
        if value.is_empty() || value.chars().count() > 255 {
            return Err(Error::InvalidSnapshotName);
        }
        Ok(Self(value.into()))
    }
}

pub enum VmCommand {
    Recovery {
        vm: crate::ObservedVm,
        operation: crate::VmRecoveryOperation,
    },
    SnapshotWorkflow {
        vm: crate::ObservedVm,
        operation: crate::SnapshotWorkflow,
    },
    Migrate {
        vm: crate::ObservedVm,
        operation: crate::VmMigration,
    },
    Lifecycle {
        source: crate::ObservedVm,
        operation: crate::VmLifecycleOperation,
    },
    Device(crate::DeviceCommand),
    PowerOnHost {
        vm: crate::ObservedVm,
        action: PowerAction,
        host: crate::HostId,
    },
    Configure {
        vm: crate::ObservedVm,
        patch: crate::VmSettingsPatch,
    },
    Power {
        vm: crate::ObservedVm,
        action: PowerAction,
    },
    CreateCheckpoint {
        vm: crate::ObservedVm,
        name: SnapshotName,
    },
    CreateSnapshot {
        vm: crate::ObservedVm,
        name: SnapshotName,
    },
    RevertSnapshot {
        snapshot: crate::ObservedSnapshot,
    },
    CloneSnapshot {
        snapshot: crate::ObservedSnapshot,
        name: SnapshotName,
        as_template: bool,
    },
    DeleteSnapshot {
        snapshot: crate::ObservedSnapshot,
        disks: Vec<crate::VdiId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OperationAction {
    Recovery(crate::VmRecoveryOperation),
    Power(PowerAction),
    Snapshot(SnapshotOperation),
    Settings(crate::VmSettingsPatch),
    Placement(PlacementOperation),
    Device(crate::DeviceOperation),
    Lifecycle(crate::VmLifecycleOperation),
    Migration(crate::VmMigration),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementOperation {
    pub placement: PowerAction,
    pub host: crate::HostId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SnapshotOperation {
    Workflow {
        operation: crate::SnapshotWorkflow,
    },
    Checkpoint {
        name: String,
    },
    Create {
        name: String,
    },
    Revert {
        snapshot: crate::SnapshotId,
    },
    Clone {
        snapshot: crate::SnapshotId,
        name: String,
        as_template: bool,
    },
    Delete {
        snapshot: crate::SnapshotId,
        disks: Vec<crate::VdiId>,
    },
}
impl OperationAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Recovery(operation) => operation.label(),
            Self::Power(action) => action.label(),
            Self::Lifecycle(operation) => operation.label(),
            Self::Migration(operation) => operation.label(),
            Self::Settings(_) => "Edit VM properties",
            Self::Device(operation) => operation.label(),
            Self::Placement(p) => {
                if p.placement == PowerAction::Resume {
                    "Resume on host"
                } else {
                    "Start on host"
                }
            }
            Self::Snapshot(SnapshotOperation::Workflow { operation }) => operation.label(),
            Self::Snapshot(SnapshotOperation::Create { .. }) => "Create snapshot",
            Self::Snapshot(SnapshotOperation::Checkpoint { .. }) => "Create memory snapshot",
            Self::Snapshot(SnapshotOperation::Revert { .. }) => "Restore snapshot",
            Self::Snapshot(SnapshotOperation::Clone {
                as_template: true, ..
            }) => "Create template from snapshot",
            Self::Snapshot(SnapshotOperation::Clone {
                as_template: false, ..
            }) => "Create VM from snapshot",
            Self::Snapshot(SnapshotOperation::Delete { .. }) => "Delete snapshot",
        }
    }
    fn resources(&self, vm: VmId) -> Vec<String> {
        let mut ids = vec![vm.to_string()];
        if let Self::Snapshot(SnapshotOperation::Workflow { operation }) = self
            && let Some(id) = operation.snapshot()
        {
            ids.push(id.to_string());
        }
        if let Self::Snapshot(
            SnapshotOperation::Revert { snapshot }
            | SnapshotOperation::Clone { snapshot, .. }
            | SnapshotOperation::Delete { snapshot, .. },
        ) = self
        {
            ids.push(snapshot.to_string());
        }
        if let Self::Snapshot(SnapshotOperation::Delete { disks, .. }) = self {
            ids.extend(disks.iter().map(|id| format!("vdi:{id}")));
        }
        if let Self::Placement(p) = self {
            ids.push(format!("host:{}", p.host));
        }
        if let Self::Device(operation) = self {
            ids.extend(operation.resources());
        }
        if let Self::Migration(operation) = self {
            ids.extend(operation.resources());
        }
        if let Self::Lifecycle(operation) = self {
            ids.extend(operation.resources());
        }
        ids
    }
}
impl VmCommand {
    pub(crate) fn normalize(self) -> (crate::ObservedVm, OperationAction) {
        match self {
            Self::Recovery { vm, operation } => (vm, OperationAction::Recovery(operation)),
            Self::SnapshotWorkflow { vm, operation } => (
                vm,
                OperationAction::Snapshot(SnapshotOperation::Workflow { operation }),
            ),
            Self::Migrate { vm, operation } => (vm, OperationAction::Migration(operation)),
            Self::Lifecycle { source, operation } => {
                (source, OperationAction::Lifecycle(operation))
            }
            Self::Device(command) => (command.vm, OperationAction::Device(command.operation)),
            Self::Power { vm, action } => (vm, OperationAction::Power(action)),
            Self::PowerOnHost { vm, action, host } => (
                vm,
                OperationAction::Placement(PlacementOperation {
                    placement: action,
                    host,
                }),
            ),
            Self::Configure { vm, patch } => (vm, OperationAction::Settings(patch)),
            Self::CreateCheckpoint { vm, name } => (
                vm,
                OperationAction::Snapshot(SnapshotOperation::Checkpoint { name: name.0 }),
            ),
            Self::CreateSnapshot { vm, name } => (
                vm,
                OperationAction::Snapshot(SnapshotOperation::Create { name: name.0 }),
            ),
            Self::RevertSnapshot { snapshot } => (
                crate::ObservedVm {
                    pool: snapshot.pool,
                    generation: snapshot.generation,
                    id: snapshot.vm,
                },
                OperationAction::Snapshot(SnapshotOperation::Revert {
                    snapshot: snapshot.id,
                }),
            ),
            Self::CloneSnapshot {
                snapshot,
                name,
                as_template,
            } => (
                crate::ObservedVm {
                    pool: snapshot.pool,
                    generation: snapshot.generation,
                    id: snapshot.vm,
                },
                OperationAction::Snapshot(SnapshotOperation::Clone {
                    snapshot: snapshot.id,
                    name: name.0,
                    as_template,
                }),
            ),
            Self::DeleteSnapshot {
                snapshot,
                mut disks,
            } => {
                disks.sort();
                disks.dedup();
                (
                    crate::ObservedVm {
                        pool: snapshot.pool,
                        generation: snapshot.generation,
                        id: snapshot.vm,
                    },
                    OperationAction::Snapshot(SnapshotOperation::Delete {
                        snapshot: snapshot.id,
                        disks,
                    }),
                )
            }
        }
    }
}

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
    Resume,
    Unpause,
    Pause,
    Suspend,
    CleanShutdown,
    Reboot,
    HardShutdown,
    HardReboot,
}
impl PowerAction {
    pub const ALL: [Self; 9] = [
        Self::Start,
        Self::Resume,
        Self::Unpause,
        Self::Pause,
        Self::Suspend,
        Self::CleanShutdown,
        Self::Reboot,
        Self::HardShutdown,
        Self::HardReboot,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Resume => "Resume",
            Self::Unpause => "Unpause",
            Self::Pause => "Pause",
            Self::Suspend => "Suspend",
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
            Self::Resume => "resume",
            Self::Unpause => "unpause",
            Self::Pause => "pause",
            Self::Suspend => "suspend",
            Self::CleanShutdown => "clean_shutdown",
            Self::Reboot => "clean_reboot",
            Self::HardShutdown => "hard_shutdown",
            Self::HardReboot => "hard_reboot",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    Preparing,
    Verifying,
    Submitting,
    Dispatching,
    Tracking,
    CancellationRequested,
    Succeeded,
    PartiallyCompleted,
    Failed,
    Cancelled,
    OutcomeUnknown,
    AcknowledgedUnknown,
    Rejected,
}
impl OperationState {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::PartiallyCompleted
                | Self::Failed
                | Self::Cancelled
                | Self::Rejected
                | Self::AcknowledgedUnknown
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationRecord {
    #[serde(default)]
    pub batch_parent: Option<OperationId>,
    pub id: OperationId,
    pub pool: PoolId,
    pub vm: VmId,
    pub action: OperationAction,
    pub state: OperationState,
    pub progress: Option<f64>,
    pub detail: String,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(skip)]
    cancel_dispatched: bool,
    #[serde(default)]
    pub can_cancel: bool,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    pub local_error: Option<String>,
    #[serde(default, skip_serializing)]
    pub(crate) task: Option<String>,
    #[serde(skip)]
    result_reference: Option<String>,
    #[serde(default)]
    pub created_snapshot: Option<crate::SnapshotId>,
    #[serde(default)]
    pub created_vm: Option<crate::VmId>,
    #[serde(skip)]
    snapshot_deletion: Option<crate::snapshots::DeletionPlan>,
    #[serde(skip)]
    snapshot_workflow: Option<Box<crate::snapshot_workflow::Plan>>,
    #[serde(skip)]
    recovery_plan: Option<Box<crate::vm_recovery::Plan>>,
    #[serde(skip)]
    snapshot_clone_baseline: Option<Vec<VmId>>,
    #[serde(skip)]
    device_plan: Option<crate::device_lifecycle::Plan>,
    #[serde(skip)]
    lifecycle_plan: Option<Box<crate::vm_lifecycle_backend::Plan>>,
    #[serde(skip)]
    migration_plan: Option<Box<crate::migration_backend::Plan>>,
    #[serde(default)]
    pub created_devices: Vec<crate::CreatedDevice>,
    #[serde(default)]
    pub steps: Vec<crate::OperationStep>,
}
impl OperationRecord {
    pub(crate) fn verified_snapshot_creation(&self, snapshot: crate::SnapshotId) -> bool {
        use crate::snapshot_workflow::Step;
        let OperationAction::Snapshot(SnapshotOperation::Workflow { operation }) = &self.action
        else {
            return false;
        };
        let Some(plan) = &self.snapshot_workflow else {
            return false;
        };
        self.state == OperationState::Succeeded
            && !self.cancel_requested
            && self.created_snapshot == Some(snapshot)
            && matches!(operation, crate::SnapshotWorkflow::Create { .. })
            && plan.operation == *operation
            && plan.created == Some(snapshot)
            && !plan.baseline.iter().any(|id| id.uuid() == snapshot.uuid())
            && plan.steps == [Step::Create, Step::Description]
            && plan.evidence.len() == 2
            && plan.evidence.iter().all(|e| e.completed)
            && plan.evidence[0]
                .task
                .as_ref()
                .is_some_and(|r| crate::snapshots::reference(&json!(r)).is_ok())
            && plan.evidence[0]
                .result
                .as_ref()
                .is_some_and(|r| crate::snapshots::reference(&json!(r)).is_ok())
            && self.steps.len() == 2
            && self
                .steps
                .iter()
                .all(|s| s.state == crate::StepState::Verified)
    }
    fn valid_recovery_plan(&self) -> bool {
        match (&self.action, &self.recovery_plan) {
            (OperationAction::Recovery(_), Some(plan)) => plan.valid(self),
            (OperationAction::Recovery(_), None) => {
                self.steps.is_empty()
                    && matches!(
                        self.state,
                        OperationState::Submitting
                            | OperationState::Rejected
                            | OperationState::Failed
                            | OperationState::Cancelled
                    )
            }
            (_, plan) => plan.is_none(),
        }
    }
    pub fn recovery_boot_restoration(&self) -> Option<crate::VmRecoveryOperation> {
        let OperationAction::Recovery(crate::VmRecoveryOperation::Boot { configuration }) =
            &self.action
        else {
            return None;
        };
        let plan = self.recovery_plan.as_ref()?;
        if !self.state.terminal()
            || configuration.order.as_deref() == Some("dn")
            || plan.steps.first() != Some(&crate::vm_recovery::Step::Override)
            || !plan.evidence.first().is_some_and(|e| e.completed)
            || !plan.steps.iter().zip(&self.steps).any(|(kind, public)| {
                *kind == crate::vm_recovery::Step::Restore
                    && matches!(public.state, crate::StepState::Pending)
            })
        {
            return None;
        }
        Some(crate::VmRecoveryOperation::RestoreBoot {
            source: self.id,
            configuration: configuration.clone(),
        })
    }
    pub(crate) fn reserves_pool_creation(&self) -> bool {
        matches!(
            &self.action,
            OperationAction::Lifecycle(
                crate::VmLifecycleOperation::Create { .. }
                    | crate::VmLifecycleOperation::Clone { .. }
            )
        )
    }

    pub(crate) fn resources(&self) -> Vec<String> {
        let mut resources = self.action.resources(self.vm);
        resources.extend(self.created_vm.iter().map(ToString::to_string));
        resources.extend(self.created_snapshot.iter().map(ToString::to_string));
        resources.extend(self.created_devices.iter().map(|device| match device {
            crate::CreatedDevice::Disk(id) => format!("vdi:{id}"),
            crate::CreatedDevice::Attachment(id) => format!("vbd:{id}"),
            crate::CreatedDevice::Nic(id) => format!("vif:{id}"),
        }));
        resources.sort();
        resources.dedup();
        resources
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn can_recheck(&self) -> bool {
        if self.batch_parent.is_some() {
            return false;
        }
        self.has_task()
            || matches!(
                &self.action,
                OperationAction::Lifecycle(_)
                    | OperationAction::Migration(_)
                    | OperationAction::Recovery(_)
            )
            || matches!(&self.action,OperationAction::Device(operation) if operation.is_lifecycle())
            || matches!(
                self.action,
                OperationAction::Settings(_)
                    | OperationAction::Snapshot(
                        SnapshotOperation::Delete { .. } | SnapshotOperation::Workflow { .. }
                    )
            )
    }
    pub fn has_task(&self) -> bool {
        self.task.is_some()
    }
    pub fn supersedes(&self, other: &Self) -> bool {
        (
            self.revision,
            self.local_error.is_some(),
            self.revision == 0 && self.state == OperationState::Rejected,
        ) > (
            other.revision,
            other.local_error.is_some(),
            other.revision == 0 && other.state == OperationState::Rejected,
        )
    }
}
#[derive(Clone, Copy, Debug)]
pub enum OperationControl {
    RequestCancellation,
    Recheck,
    AcknowledgeUnknown,
}

#[derive(Clone)]
pub struct Operation {
    receiver: watch::Receiver<Arc<OperationRecord>>,
    runtime: tokio::runtime::Handle,
}
impl Operation {
    pub fn latest(&self) -> Arc<OperationRecord> {
        self.receiver.borrow().clone()
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

pub(crate) struct MutationRequest {
    pub(crate) batch_permit: Option<crate::vm_workflow_batch::ChildPermit>,
    pub generation: u64,
    pub record: OperationRecord,
    sender: watch::Sender<Arc<OperationRecord>>,
    updates: Option<mpsc::Sender<OperationUpdate>>,
}
pub(crate) enum OperationUpdate {
    RootCredentials(Box<crate::connection::RootCredentialUpdate>),
    Record(Box<OperationRecord>),
    Resource(Box<crate::ResourceOperationRecord>),
    Coordinator(crate::Endpoint),
}
struct DispatchPermit(OperationAction);
impl DispatchPermit {
    async fn send(
        self,
        session: &Session,
        reference: String,
        host_reference: Option<String>,
        device: Option<crate::devices::PreparedDevice>,
        cancel: &CancellationToken,
    ) -> Result<serde_json::Value, Error> {
        let mut params = vec![json!(reference)];
        let method = match self.0 {
            OperationAction::Recovery(_) => return Err(Error::UnavailableAction),
            OperationAction::Settings(_) => return Err(Error::InvalidSettings),
            OperationAction::Migration(_) => return Err(Error::InvalidMigration),
            OperationAction::Lifecycle(_) => return Err(Error::InvalidVmLifecycle),
            OperationAction::Device(_) => {
                return device
                    .ok_or(Error::InvalidDeviceOperation)?
                    .send(session, cancel)
                    .await;
            }
            OperationAction::Placement(p) => {
                params.extend([
                    json!(host_reference.ok_or(Error::Stale)?),
                    json!(false),
                    json!(false),
                ]);
                match p.placement {
                    PowerAction::Start => "Async.VM.start_on".into(),
                    PowerAction::Resume => "Async.VM.resume_on".into(),
                    _ => return Err(Error::UnavailableAction),
                }
            }
            OperationAction::Power(action) => {
                if matches!(action, PowerAction::Start | PowerAction::Resume) {
                    params.extend([json!(false), json!(false)]);
                }
                format!("Async.VM.{}", action.wire())
            }
            OperationAction::Snapshot(SnapshotOperation::Checkpoint { name }) => {
                params.push(json!(name));
                "Async.VM.checkpoint".into()
            }
            OperationAction::Snapshot(SnapshotOperation::Create { name }) => {
                params.push(json!(name));
                "Async.VM.snapshot".into()
            }
            OperationAction::Snapshot(SnapshotOperation::Revert { .. }) => "Async.VM.revert".into(),
            OperationAction::Snapshot(SnapshotOperation::Clone { name, .. }) => {
                params.push(json!(name));
                "Async.VM.clone".into()
            }
            OperationAction::Snapshot(
                SnapshotOperation::Delete { .. } | SnapshotOperation::Workflow { .. },
            ) => {
                return Err(Error::UnavailableAction);
            }
        };
        tokio::select! { _=cancel.cancelled()=>Err(Error::Closed), result=session.call(&method,params)=>result }
    }
}
pub(crate) fn channel(
    id: OperationId,
    pool: PoolId,
    vm: VmId,
    generation: u64,
    action: OperationAction,
    runtime: tokio::runtime::Handle,
) -> (Operation, MutationRequest) {
    let record = OperationRecord {
        batch_parent: None,
        id,
        pool,
        vm,
        action,
        state: OperationState::Submitting,
        progress: None,
        detail: "Waiting for admission".into(),
        created_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_millis() as u64)
            .unwrap_or(0),
        revision: 0,
        local_error: None,
        cancel_requested: false,
        cancel_dispatched: false,
        can_cancel: false,
        task: None,
        result_reference: None,
        created_snapshot: None,
        created_vm: None,
        snapshot_deletion: None,
        snapshot_workflow: None,
        recovery_plan: None,
        snapshot_clone_baseline: None,
        device_plan: None,
        lifecycle_plan: None,
        migration_plan: None,
        created_devices: Vec::new(),
        steps: Vec::new(),
    };
    let (sender, receiver) = watch::channel(Arc::new(record.clone()));
    (
        Operation { receiver, runtime },
        MutationRequest {
            batch_permit: None,
            generation,
            record,
            sender,
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

fn journal_sqlite_failure(stage: &'static str, error: rusqlite::Error) -> Error {
    let code = error.sqlite_error().map(|error| error.extended_code);
    eprintln!("xenadmin journal initialization failed: stage={stage} sqlite_code={code:?}");
    Error::Journal
}
fn journal_file_failure(stage: &'static str, error: std::io::Error) -> Error {
    eprintln!(
        "xenadmin journal initialization failed: stage={stage} os_code={:?}",
        error.raw_os_error()
    );
    Error::Journal
}
static JOURNAL_INITIALIZATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) struct Journal(pub(crate) rusqlite::Connection);
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
        let _initializing = JOURNAL_INITIALIZATION.lock().map_err(|_| Error::Journal)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|error| journal_file_failure("directory", error))?;
        let path = directory.join("operations.sqlite3");
        // Closing a non-SQLite fd for an existing database releases this process's
        // POSIX locks, including locks owned by other live SQLite connections.
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(created) => drop(created),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(journal_file_failure("private-file", error)),
        }
        let connection = rusqlite::Connection::open(path)
            .map_err(|error| journal_sqlite_failure("open", error))?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|error| journal_sqlite_failure("busy-timeout", error))?;
        let version: u64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| journal_sqlite_failure("read-version", error))?;
        if version > 4 {
            return Err(Error::JournalVersion);
        }
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|error| journal_sqlite_failure("read-journal-mode", error))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            let selected: String = connection
                .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                .map_err(|error| journal_sqlite_failure("enable-wal", error))?;
            if !selected.eq_ignore_ascii_case("wal") {
                return Err(Error::Journal);
            }
        }
        connection
            .execute_batch("PRAGMA synchronous=FULL;")
            .map_err(|error| journal_sqlite_failure("full-durability", error))?;
        if version < 4 {
            let transaction = rusqlite::Transaction::new_unchecked(
                &connection,
                rusqlite::TransactionBehavior::Immediate,
            )
            .map_err(|error| journal_sqlite_failure("migration-begin", error))?;
            let locked_version: u64 = transaction
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .map_err(|error| journal_sqlite_failure("migration-read-version", error))?;
            if locked_version > 4 {
                return Err(Error::JournalVersion);
            }
            transaction.execute_batch("CREATE TABLE IF NOT EXISTS operations (id TEXT PRIMARY KEY, pool TEXT NOT NULL, vm TEXT NOT NULL, payload TEXT NOT NULL, task TEXT, terminal INTEGER NOT NULL, revision INTEGER NOT NULL DEFAULT 0);").map_err(|error| journal_sqlite_failure("migration-create-vm-table",error))?;
            let has_revision:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('operations') WHERE name='revision')",[],|row|row.get(0)).map_err(|error| journal_sqlite_failure("migration-inspect-columns",error))?;
            if !has_revision {
                transaction
                    .execute_batch(
                        "ALTER TABLE operations ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;",
                    )
                    .map_err(|error| journal_sqlite_failure("migration-add-revision", error))?;
            }
            transaction
                .execute_batch("CREATE TABLE IF NOT EXISTS resource_operations (id TEXT PRIMARY KEY, pool TEXT NOT NULL, payload TEXT NOT NULL, task TEXT, result_reference TEXT, terminal INTEGER NOT NULL, revision INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS workflow_batch_children (parent TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal >= 0 AND ordinal < 32), child TEXT NOT NULL UNIQUE, kind TEXT NOT NULL CHECK(kind IN ('vm','resource')), PRIMARY KEY(parent,ordinal)); PRAGMA user_version=4;")
                .map_err(|error| journal_sqlite_failure("migration-create-resource-table",error))?;
            transaction
                .commit()
                .map_err(|error| journal_sqlite_failure("migration-commit", error))?;
        }
        Ok(Self(connection))
    }
    fn encode_record(record: &OperationRecord) -> Result<String, Error> {
        if !record.valid_recovery_plan()
            || !crate::migration_backend::valid_record(record, record.migration_plan.as_deref())
            || !crate::migration_backend::valid_receipts(record, record.migration_plan.as_deref())
        {
            return Err(Error::Journal);
        }
        let mut value = serde_json::to_value(record).map_err(|_| Error::Journal)?;
        let object = value.as_object_mut().ok_or(Error::Journal)?;
        macro_rules! evidence { ($($field:ident),+ $(,)?) => {$(object.insert(stringify!($field).into(),serde_json::to_value(&record.$field).map_err(|_|Error::Journal)?);)+}; }
        evidence!(
            cancel_dispatched,
            result_reference,
            snapshot_deletion,
            snapshot_workflow,
            recovery_plan,
            snapshot_clone_baseline,
            device_plan,
            lifecycle_plan,
            migration_plan
        );
        serde_json::to_string(&value).map_err(|_| Error::Journal)
    }
    fn decode_record(payload: &str) -> Result<OperationRecord, Error> {
        fn take<T: serde::de::DeserializeOwned + Default>(
            payload: &mut serde_json::Value,
            key: &str,
        ) -> Result<T, Error> {
            payload
                .as_object_mut()
                .ok_or(Error::Journal)?
                .remove(key)
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Journal)
                .map(Option::unwrap_or_default)
        }
        let mut value: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| Error::Journal)?;
        let cancel_dispatched = take(&mut value, "cancel_dispatched")?;
        let result_reference = take(&mut value, "result_reference")?;
        let snapshot_deletion = take(&mut value, "snapshot_deletion")?;
        let snapshot_workflow = take(&mut value, "snapshot_workflow")?;
        let recovery_plan = take(&mut value, "recovery_plan")?;
        let snapshot_clone_baseline = take(&mut value, "snapshot_clone_baseline")?;
        let device_plan = take(&mut value, "device_plan")?;
        let lifecycle_plan = take(&mut value, "lifecycle_plan")?;
        let migration_plan = take(&mut value, "migration_plan")?;
        let mut record: OperationRecord =
            serde_json::from_value(value).map_err(|_| Error::Journal)?;
        record.cancel_dispatched = cancel_dispatched;
        record.result_reference = result_reference;
        record.snapshot_deletion = snapshot_deletion;
        record.snapshot_workflow = snapshot_workflow;
        record.recovery_plan = recovery_plan;
        if !record.valid_recovery_plan() {
            return Err(Error::Journal);
        }
        record.snapshot_clone_baseline = snapshot_clone_baseline;
        record.device_plan = device_plan;
        record.lifecycle_plan = lifecycle_plan;
        record.migration_plan = migration_plan;
        if record.state == OperationState::OutcomeUnknown
            && let Some(plan) = record.migration_plan.as_mut()
        {
            plan.interrupted = true;
        }
        if !crate::migration_backend::valid_record(&record, record.migration_plan.as_deref()) {
            return Err(Error::Journal);
        }
        Ok(record)
    }
    pub(crate) fn load(&self, id: OperationId) -> Result<Option<OperationRecord>, Error> {
        let value: Option<(String, Option<String>, u64)> = self
            .0
            .query_row(
                "SELECT payload,task,revision FROM operations WHERE id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| Error::Journal)?;
        value
            .map(|(payload, task, revision)| {
                let mut record = Self::decode_record(&payload)?;
                record.task = task;
                record.revision = revision;
                if !crate::migration_backend::valid_receipts(
                    &record,
                    record.migration_plan.as_deref(),
                ) {
                    return Err(Error::Journal);
                }
                self.workflow_validate_child(&record)?;
                Ok(record)
            })
            .transpose()
    }
    fn admit(&mut self, record: &mut OperationRecord) -> Result<bool, Error> {
        self.admit_owned(record, None)
    }
    fn admit_owned(
        &mut self,
        record: &mut OperationRecord,
        permit: Option<&crate::vm_workflow_batch::ChildPermit>,
    ) -> Result<bool, Error> {
        self.0
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| Error::Journal)?;
        let result = (|| {
            if self.resource_load(record.id)?.is_some() {
                return Err(Error::OperationConflict);
            }
            let parent = self.workflow_admit_child(record, permit)?;
            if let Some(existing) = self.load(record.id)? {
                if existing.pool != record.pool
                    || existing.vm != record.vm
                    || existing.action != record.action
                {
                    return Err(Error::OperationConflict);
                }
                return Ok(false);
            }
            let busy = self.resource_pool_busy_except(record.pool, parent)?;
            if busy {
                return Err(Error::ConflictingPoolOperation);
            }
            let resources = record.resources();
            let mut query = self
                .0
                .prepare("SELECT payload FROM operations WHERE terminal=0 AND pool=?1")
                .map_err(|_| Error::Journal)?;
            let rows = query
                .query_map([record.pool.to_string()], |row| row.get::<_, String>(0))
                .map_err(|_| Error::Journal)?;
            for payload in rows {
                let existing = Self::decode_record(&payload.map_err(|_| Error::Journal)?)?;
                self.workflow_validate_child(&existing)?;
                if record.reserves_pool_creation() || existing.reserves_pool_creation() {
                    return Err(Error::PoolCreationConflict);
                }
                if existing.resources().iter().any(|id| resources.contains(id)) {
                    return Err(Error::ConflictingOperation);
                }
            }
            drop(query);
            let total: i64 = self
                .0
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM operations WHERE terminal=0)+(SELECT COUNT(*) FROM resource_operations WHERE terminal=0)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| Error::Journal)?;
            if total >= 1000 {
                return Err(Error::JournalFull);
            }
            if let Some(permit) = permit {
                self.workflow_mark_admitted(record, permit)?;
                self.save_validated(record)?;
            } else {
                self.save(record)?;
            }
            self.workflow_prune()?;
            self.0.execute("DELETE FROM operations WHERE terminal=1 AND id NOT IN (SELECT child FROM workflow_batch_children) AND rowid NOT IN (SELECT rowid FROM operations WHERE terminal=1 ORDER BY rowid DESC LIMIT 500)", []).map_err(|_| Error::Journal)?;
            Ok(true)
        })();
        if result.is_ok() {
            self.0.execute_batch("COMMIT").map_err(|_| Error::Journal)?;
        } else {
            let _ = self.0.execute_batch("ROLLBACK");
        }
        result
    }
    fn save(&self, record: &mut OperationRecord) -> Result<(), Error> {
        self.workflow_validate_child(record)?;
        self.save_validated(record)
    }
    fn save_validated(&self, record: &mut OperationRecord) -> Result<(), Error> {
        if record.state == OperationState::OutcomeUnknown
            && let Some(plan) = record.migration_plan.as_mut()
        {
            plan.interrupted = true;
        }
        if let Some(previous) = self.load(record.id)? {
            if previous.batch_parent != record.batch_parent
                || previous.pool != record.pool
                || previous.vm != record.vm
                || previous.action != record.action
            {
                return Err(Error::OperationConflict);
            }
            if (previous.state.terminal() && previous.state != record.state)
                || (previous.task.is_some() && previous.task != record.task)
                || (previous.cancel_requested && !record.cancel_requested)
                || (previous.cancel_dispatched && !record.cancel_dispatched)
                || (previous.result_reference.is_some()
                    && previous.result_reference != record.result_reference)
                || (previous.created_snapshot.is_some()
                    && previous.created_snapshot != record.created_snapshot)
                || (previous.created_vm.is_some() && previous.created_vm != record.created_vm)
                || previous.lifecycle_plan.as_ref().is_some_and(|old| {
                    !record
                        .lifecycle_plan
                        .as_ref()
                        .is_some_and(|new| new.extends(old))
                })
                || previous.migration_plan.as_ref().is_some_and(|old| {
                    record
                        .migration_plan
                        .as_ref()
                        .is_none_or(|new| !new.extends(old))
                })
                || previous.snapshot_workflow.as_ref().is_some_and(|old| {
                    !record
                        .snapshot_workflow
                        .as_ref()
                        .is_some_and(|new| new.extends(old))
                })
                || previous.recovery_plan.as_ref().is_some_and(|old| {
                    !record
                        .recovery_plan
                        .as_ref()
                        .is_some_and(|new| new.extends(old))
                })
                || (previous.snapshot_deletion.is_some()
                    && previous.snapshot_deletion != record.snapshot_deletion)
                || (previous.snapshot_clone_baseline.is_some()
                    && previous.snapshot_clone_baseline != record.snapshot_clone_baseline)
                || !record
                    .created_devices
                    .starts_with(&previous.created_devices)
            {
                return Err(Error::JournalChanged);
            }
        }
        let changed = self.0.execute("INSERT INTO operations(id,pool,vm,payload,task,terminal,revision) VALUES(?1,?2,?3,?4,?5,?6,?7+1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,task=excluded.task,terminal=excluded.terminal,revision=excluded.revision WHERE operations.revision=?7 AND (operations.terminal=0 OR excluded.terminal=1)", rusqlite::params![record.id.to_string(), record.pool.to_string(), record.vm.to_string(), Self::encode_record(record)?, record.task, record.state.terminal(), record.revision]).map_err(|_| Error::Journal)?;
        if changed != 1 {
            return Err(Error::JournalChanged);
        }
        record.revision += 1;
        Ok(())
    }
    fn mark_dispatching(&self, record: &mut OperationRecord) -> Result<DispatchPermit, Error> {
        if record.state != OperationState::Submitting
            || record.task.is_some()
            || record.cancel_requested
        {
            return Err(Error::JournalChanged);
        }
        record.state = OperationState::Dispatching;
        record.detail = "Submitting to XAPI".into();
        self.save(record)?;
        Ok(DispatchPermit(record.action.clone()))
    }
    fn attach_task(&self, evidence: &OperationRecord) -> Result<OperationRecord, Error> {
        let Some(task) = &evidence.task else {
            return self.load(evidence.id)?.ok_or(Error::Journal);
        };
        for _ in 0..16 {
            let mut current = self.load(evidence.id)?.ok_or(Error::Journal)?;
            if current.pool != evidence.pool
                || current.vm != evidence.vm
                || current.action != evidence.action
            {
                return Err(Error::OperationConflict);
            }
            if let Some(existing) = &current.task {
                if existing != task {
                    return Err(Error::Protocol("conflicting task-reference evidence"));
                }
                return Ok(current);
            }
            if current.state == OperationState::Submitting {
                return Err(Error::JournalChanged);
            }
            current.task = Some(task.clone());
            if !current.state.terminal() && !self.workflow_child_stopped(current.id)? {
                current.state = OperationState::Tracking;
                current.detail = "Host accepted the operation; tracking its task".into();
            }
            match self.save(&mut current) {
                Ok(()) => return Ok(current),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    pub(crate) fn control(
        &self,
        pool: PoolId,
        id: OperationId,
        acknowledge: bool,
    ) -> Result<OperationRecord, Error> {
        if self.workflow_owner(id)?.is_some() {
            return Err(Error::UnavailableAction);
        }
        for _ in 0..16 {
            let mut record = self
                .load(id)?
                .filter(|record| record.pool == pool)
                .ok_or(Error::OperationConflict)?;
            if acknowledge {
                if record.state != OperationState::OutcomeUnknown {
                    return Err(Error::UnavailableAction);
                }
                record.state = OperationState::AcknowledgedUnknown;
                record.can_cancel = false;
                record.detail = "Outcome remains unverified. Operator acknowledged it and released the local VM reservation.".into();
            } else {
                if record.state.terminal() || !record.can_recheck() {
                    return Err(Error::UnavailableAction);
                }
                record.cancel_requested = true;
                record.detail = if record.task.is_none() {"Cancellation requested; remaining steps will stop after the in-flight step resolves"} else {"Cancellation requested; waiting for the host task"}.into();
            }
            match self.save(&mut record) {
                Ok(()) => return Ok(record),
                Err(Error::JournalChanged) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::JournalChanged)
    }
    pub(crate) fn history(&self) -> Result<Vec<OperationRecord>, Error> {
        let mut query = self
            .0
            .prepare("SELECT payload,task,revision FROM operations ORDER BY rowid DESC LIMIT 1500")
            .map_err(|_| Error::Journal)?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            })
            .map_err(|_| Error::Journal)?;
        rows.map(|row| {
            let (payload, task, revision) = row.map_err(|_| Error::Journal)?;
            let mut record = Self::decode_record(&payload)?;
            record.task = task;
            record.revision = revision;
            if !crate::migration_backend::valid_receipts(&record, record.migration_plan.as_deref())
            {
                return Err(Error::Journal);
            }
            self.workflow_validate_child(&record)?;
            Ok(record)
        })
        .collect()
    }
}

pub(crate) fn recover(directory: &Path, pool: PoolId) -> Result<Vec<OperationRecord>, Error> {
    if !directory.join("operations.sqlite3").exists() {
        return Ok(Vec::new());
    }
    let journal = Journal::open(directory)?;
    let mut recovered = Vec::new();
    for mut record in journal
        .history()?
        .into_iter()
        .filter(|record| record.pool == pool)
    {
        if record.batch_parent.is_some() {
            recovered.push(record);
            continue;
        }
        if !record.state.terminal() && record.migration_plan.is_some() {
            record.state = OperationState::OutcomeUnknown;
            record.can_cancel = false;
            record.detail = "Interrupted migration; recovery reads saved evidence without new receiver tokens or remaining changes".into();
            match journal.save(&mut record) {
                Ok(()) => {}
                Err(Error::JournalChanged) => {
                    record = journal.load(record.id)?.ok_or(Error::Journal)?;
                }
                Err(error) => return Err(error),
            }
        } else if !record.state.terminal() && record.task.is_none() {
            record.state = if record.state == OperationState::Preparing
                && record
                    .steps
                    .iter()
                    .any(|s| s.state == crate::StepState::Verified)
            {
                OperationState::PartiallyCompleted
            } else if matches!(
                record.state,
                OperationState::Submitting | OperationState::Preparing
            ) {
                OperationState::Failed
            } else {
                OperationState::OutcomeUnknown
            };
            record.detail = if matches!(record.action, OperationAction::Snapshot(SnapshotOperation::Workflow { .. })) {
                "Interrupted snapshot workflow. Recheck saved step evidence and inspect the host; remaining steps will not be submitted automatically."
            } else {
                "Interrupted operation without a saved task reference. Inspect the host before acknowledging its unknown outcome."
            }.into();
            match journal.save(&mut record) {
                Ok(()) => {}
                Err(Error::JournalChanged) => {
                    record = journal.load(record.id)?.ok_or(Error::Journal)?;
                }
                Err(error) => return Err(error),
            }
        }
        recovered.push(record);
    }
    Ok(recovered)
}

impl MutationRequest {
    pub(crate) fn tracking(record: OperationRecord) -> Self {
        let (sender, _) = watch::channel(Arc::new(record.clone()));
        Self {
            generation: 0,
            batch_permit: None,
            record,
            sender,
            updates: None,
        }
    }
    pub(crate) fn reject(mut self, error: Error) {
        self.record.state = OperationState::Rejected;
        self.record.detail = error.to_string();
        self.sender.send_replace(Arc::new(self.record));
    }
    async fn emit(&self) {
        self.sender.send_replace(Arc::new(self.record.clone()));
        if let Some(updates) = &self.updates {
            let _ = updates
                .send(OperationUpdate::Record(Box::new(self.record.clone())))
                .await;
        }
    }
    fn emit_dispatch(&self) {
        self.sender.send_replace(Arc::new(self.record.clone()));
        if let Some(updates) = &self.updates {
            let _ = updates.try_send(OperationUpdate::Record(Box::new(self.record.clone())));
        }
    }
    async fn publish(&mut self, journal: &mut Journal) -> Result<(), Error> {
        journal.save(&mut self.record)?;
        self.emit().await;
        Ok(())
    }
    pub(crate) async fn run(
        self,
        session: Arc<Session>,
        reference: Option<String>,
        directory: PathBuf,
        connection_cancel: CancellationToken,
        updates: mpsc::Sender<OperationUpdate>,
    ) {
        self.run_session(&session, reference, directory, connection_cancel, updates)
            .await;
    }
    pub(crate) async fn run_session(
        mut self,
        session: &Session,
        reference: Option<String>,
        directory: PathBuf,
        connection_cancel: CancellationToken,
        updates: mpsc::Sender<OperationUpdate>,
    ) {
        self.updates = Some(updates);
        let result = if let Some(reference) = reference {
            self.execute(session, reference, &directory, &connection_cancel)
                .await
        } else {
            match Journal::open(&directory) {
                Ok(mut journal)
                    if matches!(
                        self.record.action,
                        OperationAction::Snapshot(SnapshotOperation::Workflow { .. })
                    ) =>
                {
                    self.recheck_snapshot_workflow(session, &mut journal).await
                }
                Ok(mut journal) if matches!(self.record.action, OperationAction::Recovery(_)) => {
                    self.recheck_recovery(session, &mut journal).await
                }
                Ok(mut journal) if matches!(self.record.action, OperationAction::Settings(_)) => {
                    self.recheck_settings(session, &mut journal).await
                }
                Ok(mut journal)
                    if matches!(
                        self.record.action,
                        OperationAction::Snapshot(SnapshotOperation::Delete { .. })
                    ) =>
                {
                    self.recheck_snapshot_delete(session, &mut journal).await
                }
                Ok(mut journal) if matches!(&self.record.action,OperationAction::Device(operation) if operation.is_lifecycle()) => {
                    self.recheck_device_lifecycle(session, &mut journal).await
                }
                Ok(mut journal) if matches!(&self.record.action, OperationAction::Migration(_)) => {
                    self.recheck_migration(session, &mut journal, &connection_cancel)
                        .await
                }
                Ok(mut journal) if matches!(&self.record.action, OperationAction::Lifecycle(_)) => {
                    self.recheck_vm_lifecycle(session, &mut journal).await
                }
                Ok(mut journal) => {
                    self.track(session, &mut journal, &connection_cancel, false)
                        .await
                }
                Err(error) => Err(error),
            }
        };
        if let Err(error) = result {
            if let Error::Coordinator(endpoint) = &error
                && let Some(updates) = &self.updates
            {
                let _ = updates
                    .send(OperationUpdate::Coordinator(endpoint.clone()))
                    .await;
            }
            let journal = Journal::open(&directory);
            if let Ok(journal) = &journal {
                if self.record.task.is_some()
                    && let Ok(current) = journal.attach_task(&self.record)
                    && current.revision != self.record.revision
                {
                    self.record = current;
                    self.emit().await;
                    return;
                }
                if let Ok(Some(current)) = journal.load(self.record.id)
                    && (current.revision != self.record.revision || current.state.terminal())
                {
                    self.record = current;
                    self.emit().await;
                    return;
                }
            }
            self.record.state = if self.record.state == OperationState::Preparing
                && self
                    .record
                    .steps
                    .iter()
                    .any(|s| s.state == crate::StepState::Verified)
            {
                OperationState::PartiallyCompleted
            } else if matches!(
                self.record.state,
                OperationState::Submitting | OperationState::Preparing
            ) {
                OperationState::Rejected
            } else if self.record.state == OperationState::Dispatching
                && matches!(error, Error::Api(_) | Error::Coordinator(_))
            {
                OperationState::Failed
            } else {
                OperationState::OutcomeUnknown
            };
            self.record.detail = error.to_string();
            if self.record.state == OperationState::OutcomeUnknown {
                for step in &mut self.record.steps {
                    if step.state == crate::StepState::Dispatching {
                        step.state = crate::StepState::OutcomeUnknown;
                        step.detail = error.to_string();
                    }
                }
            }
            self.record.can_cancel = false;
            if let Ok(journal) = journal {
                match journal.save(&mut self.record) {
                    Ok(()) => {}
                    Err(Error::JournalChanged) => {
                        if let Ok(Some(current)) = journal.load(self.record.id) {
                            self.record = current;
                        }
                    }
                    Err(_) => {
                        self.record.state = OperationState::OutcomeUnknown;
                        self.record.local_error = Some(Error::Journal.to_string());
                    }
                }
            } else {
                self.record.state = OperationState::OutcomeUnknown;
                self.record.local_error = Some(Error::Journal.to_string());
            }
            self.emit().await;
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
        let admission = if let Some(permit) = &self.batch_permit {
            journal.admit_owned(&mut self.record, Some(permit))
        } else {
            journal.admit(&mut self.record)
        };
        let new = match admission {
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
        if connection_cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        self.record
            .action
            .permission_requirements()
            .guard(session.permissions())?;
        if matches!(&self.record.action, OperationAction::Recovery(_)) {
            return self
                .execute_recovery(session, &mut journal, connection_cancel)
                .await;
        }
        if matches!(
            &self.record.action,
            OperationAction::Snapshot(SnapshotOperation::Workflow { .. })
        ) {
            return self
                .execute_snapshot_workflow(session, &mut journal, connection_cancel)
                .await;
        }
        if matches!(&self.record.action, OperationAction::Migration(_)) {
            return self
                .execute_migration(session, &mut journal, connection_cancel)
                .await;
        }
        if matches!(&self.record.action, OperationAction::Lifecycle(_)) {
            return self
                .execute_vm_lifecycle(session, &mut journal, connection_cancel)
                .await;
        }
        if matches!(
            self.record.action,
            OperationAction::Snapshot(SnapshotOperation::Delete { .. })
        ) {
            return self
                .execute_snapshot_delete(session, &mut journal, connection_cancel)
                .await;
        }
        if matches!(&self.record.action,OperationAction::Device(operation) if operation.is_lifecycle())
        {
            return self
                .execute_device_lifecycle(session, &mut journal, connection_cancel)
                .await;
        }
        let device = if matches!(self.record.action, OperationAction::Device(_)) {
            Some(
                tokio::select! {_=connection_cancel.cancelled()=>return Err(Error::Closed),result=crate::devices::prepare(session,&self.record)=>result?},
            )
        } else {
            None
        };
        let (reference, host_reference) = if device.is_some() {
            (reference, None)
        } else if let OperationAction::Placement(placement) = &self.record.action {
            let (vm, host) = tokio::select! {
                _=connection_cancel.cancelled()=>return Err(Error::Closed),
                result=session.prepare_placement(self.record.vm,placement.host,placement.placement)=>result?,
            };
            (vm, Some(host))
        } else {
            let reference = tokio::select! {
                _=connection_cancel.cancelled()=>return Err(Error::Closed),
                result=crate::xapi::prepare_mutation(session, &self.record, reference)=>result?,
            };
            (reference, None)
        };
        if connection_cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        if let OperationAction::Settings(patch) = self.record.action.clone() {
            return self
                .execute_settings(session, &reference, patch, &mut journal, connection_cancel)
                .await;
        }
        if matches!(
            self.record.action,
            OperationAction::Snapshot(SnapshotOperation::Clone { .. })
        ) {
            self.record.snapshot_clone_baseline = Some(tokio::select! {
                _=connection_cancel.cancelled()=>return Err(Error::Closed),
                result=crate::snapshots::clone_baseline(session)=>result?,
            });
        }
        let permit = journal.mark_dispatching(&mut self.record)?;
        self.emit().await;
        let task = permit
            .send(
                session,
                reference,
                host_reference,
                device,
                connection_cancel,
            )
            .await?;
        self.record.task = Some(
            task.as_str()
                .filter(|task| task.starts_with("OpaqueRef:") && *task != "OpaqueRef:NULL")
                .ok_or(Error::Protocol("invalid task reference"))?
                .into(),
        );
        self.record = journal.attach_task(&self.record)?;
        self.emit().await;
        self.track(session, &mut journal, connection_cancel, true)
            .await
    }
    async fn persist_steps(&mut self, journal: &mut Journal) -> Result<(), Error> {
        for _ in 0..16 {
            match self.publish(journal).await {
                Ok(()) => return Ok(()),
                Err(Error::JournalChanged) => {
                    let current = journal.load(self.record.id)?.ok_or(Error::Journal)?;
                    if current.state.terminal()
                        || current.state == OperationState::OutcomeUnknown
                        || journal.workflow_child_stopped(current.id)?
                    {
                        self.record = current;
                        return Err(Error::JournalChanged);
                    }
                    self.record.revision = current.revision;
                    self.record.cancel_requested |= current.cancel_requested;
                }
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
    async fn execute_settings(
        &mut self,
        session: &Session,
        reference: &str,
        patch: crate::VmSettingsPatch,
        journal: &mut Journal,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let steps = patch.steps();
        self.record.steps = steps
            .iter()
            .map(|s| crate::OperationStep {
                label: s.label.clone(),
                state: crate::StepState::Pending,
                detail: String::new(),
            })
            .collect();
        self.record.can_cancel = true;
        self.record.state = OperationState::Preparing;
        self.persist_steps(journal).await?;
        for (index, step) in steps.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(Error::Closed);
            }
            let before = session
                .call("VM.get_record", vec![json!(reference)])
                .await?;
            let before = patch.resolve_record(session, before).await?;
            if before["uuid"].as_str() != Some(self.record.vm.to_string().as_str()) {
                return Err(Error::Stale);
            }
            if let Err(error) = patch
                .target_guard(&before)
                .and_then(|_| step.preflight(&before))
            {
                self.record.steps[index].state = crate::StepState::Failed;
                self.record.steps[index].detail = error.to_string();
                self.record.state = if index == 0 {
                    OperationState::Rejected
                } else {
                    OperationState::PartiallyCompleted
                };
                self.record.detail = error.to_string();
                self.record.can_cancel = false;
                return self.persist_steps(journal).await;
            }
            if !self.begin_step(journal, index).await? {
                return Ok(());
            }
            let result = tokio::select! { _=cancel.cancelled()=>Err(Error::Closed), r=step.send(session,reference,&before)=>r };
            if let Err(error) = result {
                if let Error::Coordinator(endpoint) = &error
                    && let Some(updates) = &self.updates
                {
                    let _ = updates
                        .send(OperationUpdate::Coordinator(endpoint.clone()))
                        .await;
                }
                let refused = matches!(error, Error::Api(_) | Error::Coordinator(_));
                self.record.steps[index].state = if refused {
                    crate::StepState::Failed
                } else {
                    crate::StepState::OutcomeUnknown
                };
                self.record.steps[index].detail = error.to_string();
                self.record.state = if !refused {
                    OperationState::OutcomeUnknown
                } else if index == 0 {
                    OperationState::Failed
                } else {
                    OperationState::PartiallyCompleted
                };
                self.record.detail = error.to_string();
                self.record.can_cancel = false;
                self.persist_steps(journal).await?;
                return Ok(());
            }
            self.record.state = OperationState::Verifying;
            self.persist_steps(journal).await?;
            let after = session
                .call("VM.get_record", vec![json!(reference)])
                .await?;
            let after = patch.resolve_record(session, after).await?;
            if after["uuid"].as_str() != Some(self.record.vm.to_string().as_str())
                || patch.target_guard(&after).is_err()
                || !step.verified(&after)
            {
                return Err(Error::SettingsConflict);
            }
            self.record.steps[index].state = crate::StepState::Verified;
            self.record.steps[index].detail = "Saved value verified on host".into();
            self.record.state = OperationState::Preparing;
            self.record.progress = Some((index + 1) as f64 / steps.len() as f64);
            self.persist_steps(journal).await?;
        }
        self.record.state = OperationState::Succeeded;
        self.record.detail = "VM properties saved and verified".into();
        self.record.progress = Some(1.0);
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }
    async fn recheck_settings(
        &mut self,
        session: &Session,
        journal: &mut Journal,
    ) -> Result<(), Error> {
        let OperationAction::Settings(patch) = &self.record.action else {
            return Err(Error::InvalidSettings);
        };
        let steps = patch.steps();
        let reference = session
            .call("VM.get_by_uuid", vec![json!(self.record.vm.to_string())])
            .await?;
        let current = session.call("VM.get_record", vec![reference]).await?;
        let current = patch.resolve_record(session, current).await?;
        if current["uuid"].as_str() != Some(self.record.vm.to_string().as_str()) {
            return Err(Error::Stale);
        };
        patch.target_guard(&current)?;
        let complete = steps.iter().all(|step| step.verified(&current));
        if complete {
            self.record.steps = steps
                .iter()
                .map(|s| crate::OperationStep {
                    label: s.label.clone(),
                    state: crate::StepState::Verified,
                    detail: "Desired value observed during reconciliation".into(),
                })
                .collect();
            self.record.state = OperationState::Succeeded;
            self.record.progress = Some(1.0);
            self.record.detail =
                "All desired property values are present; no writes were repeated".into();
        } else {
            self.record.state = OperationState::OutcomeUnknown;
            self.record.detail = "Some desired values are absent or changed. Inspect properties before acknowledging; no writes were repeated.".into();
        }
        self.record.can_cancel = false;
        self.persist_steps(journal).await
    }
    async fn track(
        &mut self,
        session: &Session,
        journal: &mut Journal,
        connection_cancel: &CancellationToken,
        allow_post_task_writes: bool,
    ) -> Result<(), Error> {
        if matches!(self.record.action, OperationAction::Migration(_)) {
            return self
                .track_migration(session, journal, connection_cancel, !allow_post_task_writes)
                .await;
        }
        let reference = self
            .record
            .task
            .clone()
            .ok_or(Error::Protocol("task reference is unavailable"))?;
        let mut retry_delay = 1;
        loop {
            self.record = journal.load(self.record.id)?.ok_or(Error::Journal)?;
            if self.record.state.terminal() {
                self.emit().await;
                return Ok(());
            }
            if self.record.task.as_ref() != Some(&reference) {
                return Err(Error::Protocol("task identity changed"));
            }
            let task = tokio::select! {
                _=connection_cancel.cancelled()=>return Err(Error::Closed),
                result=async { if self.record.result_reference.is_some() {Ok(json!({"status":"success","progress":1.0}))} else {session.call("task.get_record",vec![json!(&reference)]).await} }=>result,
            };
            let task = match task {
                Ok(task) => {
                    retry_delay = 1;
                    task
                }
                Err(error @ Error::Coordinator(_)) => return Err(error),
                Err(error) if error.retryable() => {
                    self.record.state = OperationState::Tracking;
                    self.record.detail = format!("Cannot query the host task; retrying: {error}");
                    match self.publish(journal).await {
                        Ok(()) | Err(Error::JournalChanged) => {}
                        Err(error) => return Err(error),
                    }
                    tokio::select! {_=connection_cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(retry_delay))=>{}}
                    retry_delay = (retry_delay * 2).min(10);
                    continue;
                }
                Err(error) => return Err(error),
            };
            self.record.progress = task["progress"]
                .as_f64()
                .or_else(|| {
                    task["progress"]
                        .as_str()
                        .and_then(|value| value.parse().ok())
                })
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 1.0));
            self.record.can_cancel = task["allowed_operations"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == "cancel"));
            self.record.state = if self.record.cancel_requested {
                OperationState::CancellationRequested
            } else {
                OperationState::Tracking
            };
            match task["status"].as_str() {
                Some("success") => {
                    if matches!(
                        self.record.action,
                        OperationAction::Snapshot(SnapshotOperation::Clone { .. })
                    ) {
                        if self.record.result_reference.is_none() {
                            self.record.result_reference =
                                Some(crate::xapi::snapshot_task_result(&task)?);
                            self.record.detail =
                                "Clone task completed; verifying its identity".into();
                            match self.publish(journal).await {
                                Ok(()) => {}
                                Err(Error::JournalChanged) => continue,
                                Err(error) => return Err(error),
                            }
                        }
                        self.complete_snapshot_clone(
                            session,
                            journal,
                            connection_cancel,
                            allow_post_task_writes,
                        )
                        .await?;
                        if matches!(
                            self.record.state,
                            OperationState::OutcomeUnknown | OperationState::PartiallyCompleted
                        ) {
                            return Ok(());
                        }
                    }
                    if matches!(
                        self.record.action,
                        OperationAction::Snapshot(
                            SnapshotOperation::Create { .. } | SnapshotOperation::Checkpoint { .. }
                        )
                    ) {
                        if self.record.result_reference.is_none() {
                            self.record.result_reference =
                                Some(crate::xapi::snapshot_task_result(&task)?);
                            self.record.detail =
                                "Snapshot task completed; verifying its identity".into();
                            match self.publish(journal).await {
                                Ok(()) => {}
                                Err(Error::JournalChanged) => continue,
                                Err(error) => return Err(error),
                            }
                        }
                        let result = tokio::select! {
                            _=connection_cancel.cancelled()=>return Err(Error::Closed),
                            result=crate::xapi::verify_created_snapshot(session, self.record.vm, self.record.result_reference.as_deref().ok_or(Error::Protocol("snapshot result missing"))?)=>result,
                        };
                        match result {
                            Ok(id) => self.record.created_snapshot = Some(id),
                            Err(error)
                                if error.retryable() && !matches!(error, Error::Coordinator(_)) =>
                            {
                                self.record.detail = format!(
                                    "Snapshot task completed; identity verification will retry: {error}"
                                );
                                match self.publish(journal).await {
                                    Ok(()) | Err(Error::JournalChanged) => {}
                                    Err(error) => return Err(error),
                                }
                                tokio::select! {_=connection_cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
                                continue;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    self.record.state = OperationState::Succeeded;
                    self.record.detail = match self.record.action {
                        OperationAction::Snapshot(SnapshotOperation::Create { .. } | SnapshotOperation::Checkpoint { .. }) => "Snapshot created and identity verified".into(),
                        OperationAction::Snapshot(SnapshotOperation::Revert { .. }) => "Snapshot restored. Inspect the VM state before starting or resuming it.".into(),
                        OperationAction::Snapshot(SnapshotOperation::Clone {as_template:true,..}) => "Template created from snapshot and identity verified".into(),
                        OperationAction::Snapshot(SnapshotOperation::Clone {as_template:false,..}) => "VM created from snapshot and identity verified; it remains halted".into(),
                        _=>"Host task completed".into(),
                    };
                    self.record.progress = Some(1.0);
                }
                Some("failure") => {
                    self.record.state = OperationState::Failed;
                    let code = task["error_info"]
                        .as_array()
                        .and_then(|values| values.first())
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("UNKNOWN_API_ERROR");
                    self.record.detail =
                        format!("Host task failed: {}", crate::xapi::safe_error_code(code));
                }
                Some("cancelled") => {
                    self.record.state = OperationState::Cancelled;
                    self.record.detail = "Host confirmed task cancellation".into();
                }
                Some("pending" | "cancelling") => {}
                _ => return Err(Error::Protocol("unknown task status")),
            }
            if self.record.state.terminal() {
                self.record.can_cancel = false;
                match self.publish(journal).await {
                    Ok(()) => {}
                    Err(Error::JournalChanged) => continue,
                    Err(error) => return Err(error),
                }
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    session.call("task.destroy", vec![json!(&reference)]),
                )
                .await;
                return Ok(());
            }
            if self.record.cancel_requested
                && !self.record.cancel_dispatched
                && self.record.can_cancel
            {
                self.record.cancel_dispatched = true;
                self.record.detail = "Waiting for the host to confirm cancellation".into();
                match self.publish(journal).await {
                    Ok(()) => {}
                    Err(Error::JournalChanged) => continue,
                    Err(error) => return Err(error),
                }
                if let Err(error) = session.call("task.cancel", vec![json!(&reference)]).await {
                    if matches!(error, Error::Coordinator(_)) {
                        return Err(error);
                    }
                    self.record.detail = format!(
                        "Cancellation was not confirmed: {error}. Still tracking the original task."
                    );
                }
            } else if self.record.cancel_requested && !self.record.can_cancel {
                self.record.detail = "Cancellation requested, but the host task does not currently advertise cancellation".into();
            }
            match self.publish(journal).await {
                Ok(()) | Err(Error::JournalChanged) => {}
                Err(error) => return Err(error),
            }
            tokio::select! {_=connection_cancel.cancelled()=>return Err(Error::Closed),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_names_are_validated_before_any_submission() {
        for value in ["", "   ", "bad\nname", "bad\0name", &"x".repeat(256)] {
            assert!(SnapshotName::new(value).is_err());
        }
        assert_eq!(
            SnapshotName::new("  Before upgrade  ").unwrap().0,
            "Before upgrade"
        );
        assert!(SnapshotName::new(&"é".repeat(255)).is_ok());
    }
    #[test]
    fn snapshot_reservations_cover_both_parent_and_snapshot_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(directory.path()).unwrap();
        let mut first = record();
        first.state = OperationState::Submitting;
        let snapshot = crate::SnapshotId::parse("33333333-3333-3333-3333-333333333333").unwrap();
        first.action = OperationAction::Snapshot(SnapshotOperation::Revert { snapshot });
        journal.admit(&mut first).unwrap();
        let mut power = record();
        power.state = OperationState::Submitting;
        assert!(matches!(
            journal.admit(&mut power),
            Err(Error::ConflictingOperation)
        ));
        let mut other_parent = record();
        other_parent.state = OperationState::Submitting;
        other_parent.vm = VmId::parse("44444444-4444-4444-4444-444444444444").unwrap();
        other_parent.action = first.action.clone();
        assert!(matches!(
            journal.admit(&mut other_parent),
            Err(Error::ConflictingOperation)
        ));
        other_parent.action = OperationAction::Power(PowerAction::Start);
        assert!(
            journal.admit(&mut other_parent).unwrap(),
            "unrelated VMs remain independent"
        );
    }
    fn record() -> OperationRecord {
        OperationRecord {
            batch_parent: None,
            id: OperationId::new(),
            pool: PoolId::parse("11111111-1111-1111-1111-111111111111").unwrap(),
            vm: VmId::parse("22222222-2222-2222-2222-222222222222").unwrap(),
            action: OperationAction::Power(PowerAction::Start),
            state: OperationState::Dispatching,
            progress: None,
            detail: "test".into(),
            created_at_ms: 0,
            revision: 0,
            local_error: None,
            task: None,
            result_reference: None,
            created_snapshot: None,
            created_vm: None,
            snapshot_deletion: None,
            snapshot_workflow: None,
            recovery_plan: None,
            snapshot_clone_baseline: None,
            device_plan: None,
            lifecycle_plan: None,
            migration_plan: None,
            created_devices: Vec::new(),
            steps: Vec::new(),
            cancel_requested: false,
            cancel_dispatched: false,
            can_cancel: false,
        }
    }
    #[test]
    fn journal_deduplicates_ids_and_reserves_unresolved_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let mut record = record();
        assert!(journal.admit(&mut record).unwrap());
        assert!(!journal.admit(&mut record).unwrap());
        let mut another = record.clone();
        another.id = OperationId::new();
        assert!(matches!(
            journal.admit(&mut another),
            Err(Error::ConflictingOperation)
        ));
        another.id = record.id;
        another.action = OperationAction::Power(PowerAction::Reboot);
        assert!(matches!(
            journal.admit(&mut another),
            Err(Error::OperationConflict)
        ));
        drop(journal);
        let journal = Journal::open(dir.path()).unwrap();
        assert_eq!(
            journal.load(record.id).unwrap().unwrap().state,
            OperationState::Dispatching
        );
    }
    #[test]
    fn stale_recovery_cannot_release_a_dispatched_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let mut submitted = record();
        submitted.state = OperationState::Submitting;
        journal.admit(&mut submitted).unwrap();
        let mut recovery = journal.load(submitted.id).unwrap().unwrap();
        let mut worker = recovery.clone();
        worker.state = OperationState::Dispatching;
        journal.save(&mut worker).unwrap();
        recovery.state = OperationState::Failed;
        assert!(
            journal.save(&mut recovery).is_err(),
            "stale recovery must lose its compare-and-swap"
        );
        assert_eq!(
            journal.load(submitted.id).unwrap().unwrap().state,
            OperationState::Dispatching
        );
        submitted.id = OperationId::new();
        assert!(
            journal.admit(&mut submitted).is_err(),
            "the dispatched operation still reserves the VM"
        );
    }
    #[test]
    fn recovery_before_dispatch_revokes_the_stale_workers_permission() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let mut initial = record();
        initial.state = OperationState::Submitting;
        journal.admit(&mut initial).unwrap();
        let mut worker = initial.clone();
        assert_eq!(
            recover(dir.path(), initial.pool).unwrap()[0].state,
            OperationState::Failed
        );
        assert!(matches!(
            journal.mark_dispatching(&mut worker),
            Err(Error::JournalChanged)
        ));
        assert_eq!(
            journal.load(initial.id).unwrap().unwrap().state,
            OperationState::Failed
        );
    }
    #[test]
    fn late_task_evidence_survives_recovery_and_keeps_the_vm_reserved() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let mut initial = record();
        initial.state = OperationState::Submitting;
        journal.admit(&mut initial).unwrap();
        let _permit = journal.mark_dispatching(&mut initial).unwrap();
        assert_eq!(
            recover(dir.path(), initial.pool).unwrap()[0].state,
            OperationState::OutcomeUnknown
        );
        initial.task = Some("OpaqueRef:late-task".into());
        let tracked = journal.attach_task(&initial).unwrap();
        assert_eq!(tracked.task.as_deref(), Some("OpaqueRef:late-task"));
        assert_eq!(tracked.state, OperationState::Tracking);
        assert!(tracked.revision() > initial.revision());
        initial.id = OperationId::new();
        initial.revision = 0;
        assert!(matches!(
            journal.admit(&mut initial),
            Err(Error::ConflictingOperation)
        ));
    }
    #[test]
    fn cancellation_is_durable_and_unknown_acknowledgement_does_not_claim_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        let mut initial = record();
        initial.state = OperationState::Submitting;
        journal.admit(&mut initial).unwrap();
        let _permit = journal.mark_dispatching(&mut initial).unwrap();
        initial.task = Some("OpaqueRef:task".into());
        let mut tracked = journal.attach_task(&initial).unwrap();
        let requested = journal.control(initial.pool, initial.id, false).unwrap();
        assert!(requested.cancel_requested);
        assert!(matches!(
            journal.save(&mut tracked),
            Err(Error::JournalChanged)
        ));
        let mut current = recover(dir.path(), initial.pool).unwrap().remove(0);
        assert!(current.cancel_requested);
        current.state = OperationState::OutcomeUnknown;
        journal.save(&mut current).unwrap();
        let acknowledged = journal.control(initial.pool, initial.id, true).unwrap();
        assert_eq!(acknowledged.state, OperationState::AcknowledgedUnknown);
        assert_eq!(acknowledged.task.as_deref(), Some("OpaqueRef:task"));
        initial.id = OperationId::new();
        initial.revision = 0;
        initial.task = None;
        initial.state = OperationState::Submitting;
        assert!(journal.admit(&mut initial).unwrap());
    }
    #[test]
    fn opening_another_journal_preserves_existing_sqlite_process_locks() {
        let directory = tempfile::tempdir().unwrap();
        let first = Journal::open(directory.path()).unwrap();
        first.0.execute_batch("BEGIN IMMEDIATE").unwrap();
        fn protected(path: &Path) -> bool {
            std::process::Command::new("python3").arg("-c").arg("import fcntl, os, sys\nfd = os.open(sys.argv[1], os.O_RDWR)\ntry:\n fcntl.lockf(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)\nexcept BlockingIOError:\n sys.exit(0)\nsys.exit(1)").arg(path).status().unwrap().success()
        }
        let path = directory.path().join("operations.sqlite3");
        assert!(
            protected(&path),
            "fixture must start with a real SQLite process lock"
        );
        let second = Journal::open(directory.path()).unwrap();
        assert!(
            protected(&path),
            "opening history must not release another connection's SQLite process lock"
        );
        assert!(second.history().unwrap().is_empty());
        first.0.execute_batch("ROLLBACK").unwrap();
    }
    #[test]
    fn concurrent_journal_initialization_admission_and_history_keep_private_durable_records() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let gate = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let directory = directory.path();
                let gate = &gate;
                scope.spawn(move || {
                    gate.wait();
                    for iteration in 0..16 {
                        let mut journal = Journal::open(directory).unwrap();
                        if worker < 4 {
                            let mut operation = record();
                            operation.id = OperationId::new();
                            operation.vm = VmId::from(uuid::Uuid::new_v4());
                            operation.state = OperationState::Submitting;
                            assert!(journal.admit(&mut operation).unwrap());
                            operation.state = OperationState::Succeeded;
                            operation.detail = format!("fixture {worker}/{iteration}");
                            journal.save(&mut operation).unwrap();
                        } else {
                            for row in journal.history().unwrap() {
                                assert!(matches!(
                                    row.state,
                                    OperationState::Submitting | OperationState::Succeeded
                                ));
                            }
                        }
                    }
                });
            }
        });
        let journal = Journal::open(directory.path()).unwrap();
        let records = journal.history().unwrap();
        assert_eq!(records.len(), 64);
        assert!(records.iter().all(|r| r.state == OperationState::Succeeded));
        assert_eq!(
            std::fs::metadata(directory.path().join("operations.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let integrity: String = journal
            .0
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }
    #[test]
    fn future_journal_formats_are_rejected_without_downgrading() {
        let dir = tempfile::tempdir().unwrap();
        let database = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
        database.execute_batch("PRAGMA user_version=100;").unwrap();
        assert!(matches!(
            Journal::open(dir.path()),
            Err(Error::JournalVersion)
        ));
        assert_eq!(
            database
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            100
        );
    }
    #[test]
    fn legacy_journal_migration_preserves_the_task_and_unresolved_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let initial = record();
        let database = rusqlite::Connection::open(dir.path().join("operations.sqlite3")).unwrap();
        database.execute_batch("CREATE TABLE operations (id TEXT PRIMARY KEY,pool TEXT NOT NULL,vm TEXT NOT NULL,payload TEXT NOT NULL,task TEXT,terminal INTEGER NOT NULL)").unwrap();
        let payload = json!({"id":initial.id,"pool":initial.pool,"vm":initial.vm,"action":"Start","state":"Tracking","progress":0.5,"detail":"Legacy task"});
        database
            .execute(
                "INSERT INTO operations VALUES(?1,?2,?3,?4,?5,0)",
                rusqlite::params![
                    initial.id.to_string(),
                    initial.pool.to_string(),
                    initial.vm.to_string(),
                    payload.to_string(),
                    "OpaqueRef:legacy-task"
                ],
            )
            .unwrap();
        drop(database);
        let mut journal = Journal::open(dir.path()).unwrap();
        let migrated = journal.load(initial.id).unwrap().unwrap();
        assert_eq!(migrated.task.as_deref(), Some("OpaqueRef:legacy-task"));
        assert_eq!(migrated.revision(), 0);
        assert!(!migrated.cancel_requested);
        let mut another = record();
        assert!(matches!(
            journal.admit(&mut another),
            Err(Error::ConflictingOperation)
        ));
    }
    #[test]
    fn recovery_returns_large_history_without_a_bounded_channel_barrier() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        for _ in 0..200 {
            let mut row = record();
            row.state = OperationState::Succeeded;
            journal.admit(&mut row).unwrap();
        }
        assert_eq!(recover(dir.path(), record().pool).unwrap().len(), 200);
    }
}

#[cfg(test)]
#[path = "resource_journal_tests.rs"]
mod resource_journal_tests;

#[cfg(test)]
#[path = "vm_journal_privacy_tests.rs"]
mod journal_privacy_tests;

#[cfg(test)]
#[path = "guest_tools_tests.rs"]
mod guest_tools_tests;

impl OperationRecord {
    pub fn owned_disk_candidates(&self) -> Vec<crate::VdiId> {
        self.created_devices
            .iter()
            .filter_map(|d| match d {
                crate::CreatedDevice::Disk(id) if self.owned_disk_marker(*id).is_some() => {
                    Some(*id)
                }
                _ => None,
            })
            .collect()
    }
    pub(crate) fn owned_disk_marker(
        &self,
        disk: crate::VdiId,
    ) -> Option<Option<(&'static str, String)>> {
        if !self.state.terminal()
            || !self
                .created_devices
                .contains(&crate::CreatedDevice::Disk(disk))
        {
            return None;
        }
        if self.device_plan.as_ref().is_some_and(|p| {
            p.steps.iter().zip(&p.evidence).any(|(step, e)| {
                matches!(step, crate::device_lifecycle::Step::CreateDisk { .. })
                    && e.as_ref().is_some_and(|e| {
                        e.created == Some(crate::CreatedDevice::Disk(disk))
                            && e.result_reference.is_some()
                            && !e.baseline.contains(&disk.uuid())
                    })
            })
        }) {
            return Some(Some(("xenadmin-rs-operation", self.id.to_string())));
        }
        if self.lifecycle_plan.as_ref().is_some_and(|p| {
            p.created.is_some()
                && p.created_devices
                    .contains(&crate::CreatedDevice::Disk(disk))
                && p.baseline
                    .get("VDI")
                    .is_some_and(|ids| !ids.contains(&disk.uuid()))
                && p.evidence
                    .first()
                    .is_some_and(|e| e.completed && e.result.is_some())
        }) {
            return Some(None);
        }
        None
    }
}
#[cfg(test)]
#[path = "snapshot_capture_tests.rs"]
mod snapshot_capture_tests;

#[cfg(test)]
#[path = "vm_workflow_batch_tests.rs"]
mod vm_workflow_batch_tests;
