//! Private, opt-in development inspection of the desktop's existing connections.
use std::{
    collections::BTreeMap,
    fs, io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;

use crate::{ActionAvailability, Connection, Inventory, PowerAction, ProfileId, VmId};

const REQUEST_LIMIT: usize = 4096;
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(35);
struct RegistryState {
    connections: RwLock<BTreeMap<ProfileId, Connection>>,
    started_at_ms: u128,
}
type Registry = Arc<RegistryState>;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionClass {
    #[default]
    Vms,
    Hosts,
    Storage,
    Networks,
    Snapshots,
    Disks,
    Interfaces,
    Tasks,
    Messages,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum InspectionRequest {
    Status,
    Inventory {
        #[serde(default)]
        connection: Option<ProfileId>,
        #[serde(default)]
        vm: Option<VmId>,
        #[serde(default)]
        class: InspectionClass,
        #[serde(default)]
        object: Option<uuid::Uuid>,
        #[serde(default)]
        offset: usize,
        #[serde(default = "page_size")]
        limit: usize,
    },
    DiagnosePower {
        #[serde(default)]
        connection: Option<ProfileId>,
        vm: VmId,
    },
    History {
        #[serde(default)]
        connection: Option<ProfileId>,
        #[serde(default)]
        vm: Option<VmId>,
        #[serde(default = "page_size")]
        limit: usize,
    },
}
fn page_size() -> usize {
    100
}

pub struct DevInspection {
    registry: Registry,
    cancel: CancellationToken,
    worker: Option<std::thread::JoinHandle<()>>,
    socket: SocketFile,
}
impl DevInspection {
    /// The desktop must explicitly opt in and hold its instance lock before starting this service.
    pub fn start(state_directory: PathBuf) -> io::Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&state_directory)?;
        let owner = process_uid()?;
        let directory = fs::symlink_metadata(&state_directory)?;
        if !directory.is_dir() || directory.uid() != owner || directory.mode() & 0o777 != 0o700 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Inspection requires a user-owned state directory with mode 0700",
            ));
        }
        let path = state_directory.join("dev-inspect.sock");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(remove_stale_socket(&path, owner))?;
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        let socket = SocketFile::new(path)?;
        fs::set_permissions(&socket.path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let registry = Arc::new(RegistryState {
            connections: RwLock::new(BTreeMap::new()),
            started_at_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        });
        let cancel = CancellationToken::new();
        let worker = std::thread::Builder::new()
            .name("xenadmin-inspect".into())
            .spawn({
                let registry = registry.clone();
                let cancel = cancel.clone();
                move || {
                    runtime.block_on(async move {
                        if let Ok(listener) = UnixListener::from_std(listener) {
                            serve(listener, registry, owner, cancel).await;
                        }
                    })
                }
            })?;
        Ok(Self {
            registry,
            cancel,
            worker: Some(worker),
            socket,
        })
    }
    pub fn socket_path(&self) -> &Path {
        &self.socket.path
    }
    pub fn set_connections(&self, connections: impl IntoIterator<Item = (ProfileId, Connection)>) {
        *self
            .registry
            .connections
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = connections.into_iter().collect();
    }
}
impl Drop for DevInspection {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct SocketFile {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl SocketFile {
    fn new(path: PathBuf) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}
impl Drop for SocketFile {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn process_uid() -> io::Result<u32> {
    Ok(fs::metadata("/proc/self")?.uid())
}

async fn remove_stale_socket(path: &Path, owner: u32) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != owner
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Inspection socket path is occupied",
        ));
    }
    if !matches!(tokio::time::timeout(Duration::from_millis(250), UnixStream::connect(path)).await,
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused)
    {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "Inspection socket is already active",
        ));
    }
    let current = fs::symlink_metadata(path)?;
    if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Inspection socket path changed",
        ));
    }
    fs::remove_file(path)
}

async fn serve(listener: UnixListener, registry: Registry, owner: u32, cancel: CancellationToken) {
    let slots = Arc::new(Semaphore::new(4));
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { break };
                if !stream.peer_cred().is_ok_and(|credential| credential.uid() == owner) { continue; }
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
                let registry = registry.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = tokio::time::timeout(REQUEST_TIMEOUT, handle(stream, registry)).await;
                });
            }
        }
    }
}

async fn read_line(reader: impl AsyncRead + Unpin, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    BufReader::new(reader.take((limit + 1) as u64))
        .read_until(b'\n', &mut bytes)
        .await?;
    if bytes.len() > limit || !bytes.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Inspection message exceeds its bound or lacks newline",
        ));
    }
    Ok(bytes)
}
async fn handle(mut stream: UnixStream, registry: Registry) -> io::Result<()> {
    let response = match read_line(&mut stream, REQUEST_LIMIT).await {
        Ok(line) => match serde_json::from_slice::<InspectionRequest>(&line) {
            Ok(request) => match dispatch(&registry, request).await {
                Ok(result) => json!({"version": 1, "ok": true, "result": result}),
                Err(error) => failure(&error),
            },
            Err(_) => failure("Invalid inspection request"),
        },
        Err(_) => failure("Invalid inspection message"),
    };
    let mut encoded = serde_json::to_vec(&response)?;
    if encoded.len() >= RESPONSE_LIMIT {
        encoded = serde_json::to_vec(&failure("Response too large; narrow the query"))?;
    }
    encoded.push(b'\n');
    stream.write_all(&encoded).await
}
fn failure(error: &str) -> Value {
    json!({"version": 1, "ok": false, "error": error})
}

/// Connects only to a user-owned private Unix socket; never opens a host connection.
pub async fn inspect_live(path: &Path, request: &InspectionRequest) -> io::Result<Value> {
    let metadata = fs::symlink_metadata(path)?;
    let owner = process_uid()?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != owner
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Inspection socket must be user-owned with mode 0600",
        ));
    }
    tokio::time::timeout(REQUEST_TIMEOUT + Duration::from_secs(2), async {
        let mut stream = UnixStream::connect(path).await?;
        if stream.peer_cred()?.uid() != owner {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Inspection peer has another owner",
            ));
        }
        let mut encoded = serde_json::to_vec(request)?;
        encoded.push(b'\n');
        if encoded.len() > REQUEST_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Inspection request too large",
            ));
        }
        stream.write_all(&encoded).await?;
        let response: Value = serde_json::from_slice(&read_line(stream, RESPONSE_LIMIT).await?)?;
        if response["version"] != 1 || response["ok"] != true {
            return Err(io::Error::other(
                response["error"]
                    .as_str()
                    .unwrap_or("Invalid inspection response"),
            ));
        }
        Ok(response["result"].clone())
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Inspection request timed out"))?
}

fn select(registry: &Registry, id: Option<ProfileId>) -> Result<(ProfileId, Connection), String> {
    let connections = registry
        .connections
        .read()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(id) = id {
        return connections
            .get(&id)
            .cloned()
            .map(|connection| (id, connection))
            .ok_or_else(|| "Connection is not registered in the running desktop".into());
    }
    if connections.len() != 1 {
        return Err("Choose a connection UUID from the status response".into());
    }
    let (id, connection) = connections.first_key_value().expect("one connection");
    Ok((*id, connection.clone()))
}
fn context(id: ProfileId, connection: &Connection, snapshot: &crate::ConnectionSnapshot) -> Value {
    json!({"connection": id, "state": format!("{:?}", snapshot.state()),
        "generation": snapshot.generation(), "current": snapshot.is_current(),
        "read_only": connection.is_read_only(), "has_error": snapshot.error().is_some(),
        "error": snapshot.error().map(bounded),
        "updated_age_ms": snapshot.updated_at().and_then(|at| SystemTime::now().duration_since(at).ok()).map(|age| age.as_millis()),
        "pool": snapshot.inventory().map(|inventory| inventory.pool_id),
        "revision": snapshot.inventory().map(|inventory| inventory.revision)})
}
fn operation_value(record: &crate::OperationRecord) -> Value {
    json!({"id": record.id, "vm": record.vm, "action": record.action.label(), "state": record.state,
        "progress": record.progress, "created_at_ms": record.created_at_ms,
        "created_vm": record.created_vm, "created_snapshot": record.created_snapshot,
        "created_devices": record.created_devices.iter().take(64).collect::<Vec<_>>(),
        "detail": bounded(&record.detail), "local_error": record.local_error.as_deref().map(bounded),
        "steps": record.steps.iter().take(64).map(|step| json!({"label":bounded(&step.label),"state":step.state,"detail":bounded(&step.detail)})).collect::<Vec<_>>(),
        "has_local_error": record.local_error.is_some(), "can_cancel": record.can_cancel})
}
fn resource_operation_value(record: &crate::ResourceOperationRecord) -> Value {
    json!({"id":record.id,"pool":record.pool,"recovery_pool":record.recovery_pool(),"target":record.target,"action":record.action.label(),"state":record.state,
        "progress":record.progress,"created_at_ms":record.created_at_ms,"created":record.created,
        "transfer":record.transfer,"transfer_created":record.transfer_created,"disk_size":record.disk_size,
        "power_batch":match &record.action {crate::ResourceAction::PowerBatch(selection)=>Some(json!({"action":selection.action,"vms":selection.vms.iter().take(32).map(|item|json!({"vm":item.vm,"expected_state":bounded(&item.expected_state.to_string())})).collect::<Vec<_>>()})),_=>None},
        "storage_secrets":record.storage_secrets,
        "host_power_secret":record.host_power_secret,
        "root_password":record.root_password,
        "detail":bounded(&record.detail),"local_error":record.local_error.as_deref().map(bounded),
        "can_cancel":record.can_cancel,"cancel_requested":record.cancel_requested,
        "steps":record.steps.iter().take(64).map(|step|json!({"label":bounded(&step.label),"state":step.state,"detail":bounded(&step.detail)})).collect::<Vec<_>>()})
}
fn bounded(value: &str) -> String {
    let text = value.chars().take(512).collect::<String>();
    text.split_inclusive(char::is_whitespace)
        .map(|part| {
            if let Some((prefix, _)) = part.split_once("OpaqueRef:") {
                format!("{prefix}[reference] ")
            } else {
                part.to_owned()
            }
        })
        .collect()
}
fn diagnostic_value(mut availability: ActionAvailability) -> Value {
    match &mut availability {
        ActionAvailability::Blocked { reason } => *reason = bounded(reason),
        ActionAvailability::Busy { operations } => {
            operations.truncate(64);
            for operation in operations {
                operation.task = None;
                operation.operation = bounded(&operation.operation);
            }
        }
        ActionAvailability::Rejected { code } => *code = bounded(code),
        _ => {}
    }
    json!({"availability": availability, "explanation": bounded(&availability.explanation())})
}
fn vm_value(inventory: &Inventory, vm: &crate::Vm) -> Value {
    let details = inventory.details.vms.get(&vm.id);
    json!({"uuid": vm.id, "name": bounded(&vm.name), "power_state": bounded(&vm.power_state.to_string()),
        "host": vm.host, "vcpus": vm.vcpus, "memory_limit": vm.memory_bytes,
        "allowed_operations": details.and_then(|details| details.allowed_operations.as_ref()).map(|ops| ops.iter().take(64).map(|op| bounded(op)).collect::<Vec<_>>()),
        "current_operations": details.map(|details| details.current_operations.iter().take(64).map(|op| bounded(&op.operation)).collect::<Vec<_>>()),
        "blocked_operations": details.map(|details| details.blocked_operations.iter().take(64).map(|(op,reason)| (bounded(op), bounded(reason))).collect::<BTreeMap<_,_>>())})
}

fn page<T>(
    items: impl Iterator<Item = T>,
    offset: usize,
    limit: usize,
    project: impl Fn(T) -> Value,
) -> Value {
    let mut rows = Vec::new();
    let mut total = 0;
    for item in items {
        if total >= offset && rows.len() < limit {
            rows.push(project(item));
        }
        total += 1;
    }
    json!({"objects": rows, "total": total, "offset": offset})
}

fn inventory_page(
    inventory: &Inventory,
    class: InspectionClass,
    object: Option<uuid::Uuid>,
    vm: Option<VmId>,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let selected = |id| object.is_none_or(|wanted| wanted == id);
    if vm.is_some()
        && !matches!(
            class,
            InspectionClass::Vms
                | InspectionClass::Snapshots
                | InspectionClass::Disks
                | InspectionClass::Interfaces
                | InspectionClass::Messages
        )
    {
        return Err("VM filtering is unavailable for this class; use an object UUID".into());
    }
    let details = &inventory.details;
    let mut result = match class {
        InspectionClass::Vms => page(
            inventory
                .vms
                .values()
                .filter(|record| selected(record.id.uuid()) && vm.is_none_or(|id| id == record.id)),
            offset,
            limit,
            |record| vm_value(inventory, record),
        ),
        InspectionClass::Hosts => page(
            inventory
                .hosts
                .values()
                .filter(|record| selected(record.id.uuid())),
            offset,
            limit,
            |record| {
                let detail = details.hosts.get(&record.id);
                let metrics = detail
                    .and_then(|detail| detail.metrics)
                    .and_then(|id| details.host_metrics.get(&id));
                json!({"uuid":record.id,"name":bounded(&record.name),"address":bounded(&record.address),"enabled":record.enabled,"version":bounded(&record.version),
                "hostname":detail.map(|detail| bounded(&detail.hostname)),"metrics":metrics.map(|metrics| json!({"live":metrics.live,"memory_total":metrics.memory_total,"memory_free":metrics.memory_free,"last_updated":metrics.last_updated.as_deref().map(bounded)}))})
            },
        ),
        InspectionClass::Storage => page(
            inventory
                .storage
                .iter()
                .filter(|record| selected(record.uuid)),
            offset,
            limit,
            |record| {
                let id = crate::SrId::from(record.uuid);
                json!({"uuid":record.uuid,"name":bounded(&record.name),"kind":bounded(&record.kind),"capacity_bytes":record.capacity_bytes,"used_bytes":record.used_bytes,
                "attachments":details.pbds.values().filter(|attachment| attachment.sr == Some(id)).take(64).collect::<Vec<_>>()})
            },
        ),
        InspectionClass::Networks => page(
            inventory
                .networks
                .iter()
                .filter(|record| selected(record.uuid)),
            offset,
            limit,
            |record| {
                let detail = details.networks.get(&crate::NetworkId::from(record.uuid));
                json!({"uuid":record.uuid,"name":bounded(&record.name),"bridge":bounded(&record.bridge),"mtu":detail.and_then(|detail| detail.mtu),"managed":detail.and_then(|detail| detail.managed)})
            },
        ),
        InspectionClass::Snapshots => page(
            inventory.snapshots.iter().filter(|record| {
                selected(record.id.uuid()) && vm.is_none_or(|id| record.vm == Some(id))
            }),
            offset,
            limit,
            |record| {
                json!({"uuid":record.id,"vm":record.vm,"name":bounded(&record.name),"created_at":record.created_at.as_deref().map(bounded),"power_state":bounded(&record.power_state.to_string()),
                "allowed_actions":record.allowed_actions,"disks":details.vbds.values().filter(|device| device.snapshot==Some(record.id)).take(64).map(|device| json!({"vbd":device.id,"vdi":device.vdi,"owned":device.owned})).collect::<Vec<_>>()})
            },
        ),
        InspectionClass::Disks => page(
            details.vdis.values().filter(|record| {
                selected(record.id.uuid())
                    && vm.is_none_or(|id| {
                        details
                            .vbds
                            .values()
                            .any(|device| device.vm == Some(id) && device.vdi == Some(record.id))
                    })
            }),
            offset,
            limit,
            |record| {
                json!({"uuid":record.id,"name":bounded(&record.name),"sr":record.sr,"virtual_size":record.virtual_size,"physical_utilisation":record.physical_utilisation,
                "kind":bounded(&record.kind),"read_only":record.read_only,"sharable":record.sharable,"missing":record.missing,"is_snapshot":record.is_snapshot,"snapshot_of":record.snapshot_of,
                "attachments":details.vbds.values().filter(|device| device.vdi==Some(record.id)).take(64).map(|device| json!({"uuid":device.id,"vm":device.vm,"snapshot":device.snapshot,"device":bounded(&device.device),"mode":bounded(&device.mode),"attached":device.currently_attached,"owned":device.owned})).collect::<Vec<_>>()})
            },
        ),
        InspectionClass::Interfaces => {
            enum Interface<'a> {
                Virtual(&'a crate::VirtualInterface),
                Physical(&'a crate::PhysicalInterface),
            }
            let items = details
                .vifs
                .values()
                .filter(|record| {
                    selected(record.id.uuid()) && vm.is_none_or(|id| record.vm == Some(id))
                })
                .map(Interface::Virtual)
                .chain(
                    details
                        .pifs
                        .values()
                        .filter(|record| vm.is_none() && selected(record.id.uuid()))
                        .map(Interface::Physical),
                );
            page(items, offset, limit, |record| match record {
                Interface::Virtual(record) => {
                    json!({"uuid":record.id,"kind":"virtual","vm":record.vm,"network":record.network,"device":bounded(&record.device),"mac":bounded(&record.mac),"mtu":record.mtu,"attached":record.currently_attached})
                }
                Interface::Physical(record) => {
                    json!({"uuid":record.id,"kind":"physical","host":record.host,"network":record.network,"device":bounded(&record.device),"mac":bounded(&record.mac),"mtu":record.mtu,"vlan":record.vlan,"management":record.management,"attached":record.currently_attached,"ip":bounded(&record.ip),"mode":bounded(&record.ip_configuration_mode)})
                }
            })
        }
        InspectionClass::Tasks => page(
            details
                .tasks
                .values()
                .filter(|record| selected(record.id.uuid())),
            offset,
            limit,
            |record| json!({"uuid":record.id,"name":bounded(&record.name),"status":bounded(&record.status),"progress":record.progress,"created":record.created.as_deref().map(bounded),"finished":record.finished.as_deref().map(bounded),"error_code":record.error_code.as_deref().map(bounded)}),
        ),
        InspectionClass::Messages => page(
            details.messages.values().filter(|record| {
                selected(record.id.uuid())
                    && vm.is_none_or(|id| record.object_uuid == Some(id.uuid()))
            }),
            offset,
            limit,
            |record| json!({"uuid":record.id,"name":bounded(&record.name),"priority":record.priority,"object_class":bounded(&record.object_class),"object_uuid":record.object_uuid,"timestamp":record.timestamp.as_deref().map(bounded),"body":bounded(&record.body)}),
        ),
    };
    result["class"] = json!(class);
    result["class_availability"] = json!(details.classes);
    Ok(result)
}
async fn dispatch(registry: &Registry, request: InspectionRequest) -> Result<Value, String> {
    match request {
        InspectionRequest::Status => {
            let connections = registry
                .connections
                .read()
                .unwrap_or_else(|poison| poison.into_inner());
            Ok(
                json!({"connections": connections.iter().take(128).map(|(id,connection)| context(*id,connection,&connection.latest())).collect::<Vec<_>>(),
                "total": connections.len(), "application_version": env!("CARGO_PKG_VERSION"),
                "process_id": std::process::id(), "started_at_ms": registry.started_at_ms}),
            )
        }
        InspectionRequest::Inventory {
            connection,
            vm,
            class,
            object,
            offset,
            limit,
        } => {
            if !(1..=100).contains(&limit) {
                return Err("Limit must be between 1 and 100".into());
            }
            let (id, connection) = select(registry, connection)?;
            let snapshot = connection.latest();
            let inventory = snapshot.inventory().ok_or("No inventory is available")?;
            let mut page = inventory_page(inventory, class, object, vm, offset, limit)?;
            page["connection"] = context(id, &connection, &snapshot);
            Ok(page)
        }
        InspectionRequest::History {
            connection,
            vm,
            limit,
        } => {
            if !(1..=100).contains(&limit) {
                return Err("Limit must be between 1 and 100".into());
            }
            let (id, connection) = select(registry, connection)?;
            let history = connection.operation_history();
            let rows = history
                .iter()
                .rev()
                .filter(|record| vm.is_none_or(|id| record.vm == id))
                .take(limit)
                .map(operation_value)
                .collect::<Vec<_>>();
            Ok(
                json!({"connection": context(id,&connection,&connection.latest()), "operations": rows,
                 "resource_operations": connection.resource_history().iter().rev().filter(|record| vm.is_none_or(|id|matches!(&record.action,crate::ResourceAction::PowerBatch(selection) if selection.vms.iter().any(|item|item.vm==id)))).take(limit).map(resource_operation_value).collect::<Vec<_>>()}),
            )
        }
        InspectionRequest::DiagnosePower { connection, vm } => {
            let (id, connection) = select(registry, connection)?;
            let snapshot = connection.latest();
            let target = snapshot.observe_vm(vm).map_err(|error| error.to_string())?;
            let inventory = snapshot
                .inventory()
                .ok_or("Current inventory unavailable")?;
            let record = inventory
                .guests()
                .find(|record| record.id == vm)
                .ok_or("Select a guest VM")?;
            let observed = vm_value(inventory, record);
            let checks = PowerAction::ALL
                .into_iter()
                .map(|action| {
                    connection
                        .diagnose_power(target, action)
                        .map(|check| (action, check))
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                if connection.latest().observe_vm(vm).ok() != Some(target) {
                    return Err("Connection or VM became stale during diagnostics".into());
                }
                if checks.iter().all(|(_, check)| check.latest().is_some())
                    || tokio::time::Instant::now() >= deadline
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let mut complete = true;
            let actions = checks.into_iter().map(|(action,check)| match check.latest() {
                Some(Ok(availability)) => {
                    complete &= !matches!(availability,ActionAvailability::Unknown|ActionAvailability::Stale);
                    let mut result = diagnostic_value(availability);
                    result["action"] = json!(action);
                    result["label"] = json!(action.label());
                    result
                }
                Some(Err(error)) => { complete = false; json!({"action":action,"label":action.label(),"error":bounded(&error)}) }
                None => { complete = false; json!({"action":action,"label":action.label(),"error":"Action check timed out"}) }
            }).collect::<Vec<_>>();
            let completion = connection.latest();
            if completion.observe_vm(vm).ok() != Some(target) {
                return Err("Connection or VM became stale during diagnostics".into());
            }
            Ok(
                json!({"connection": context(id,&connection,&snapshot), "completion": context(id,&connection,&completion), "observed": observed, "complete":complete,"power_actions":actions}),
            )
        }
    }
}

#[cfg(test)]
#[path = "inspection_tests.rs"]
mod tests;
