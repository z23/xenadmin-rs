use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
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
    ManagementCandidate(Box<ManagementCandidate>),
    ManagementBondReview(Box<crate::management_bond_review::Request>),
    ManagementIpv6Review(Box<crate::management_ipv6_review::Request>),
    Console(ConsoleRequest),
    Metrics(crate::metrics::MetricsRequest),
    StorageDiscovery(crate::storage_discovery::DiscoveryRequest),
    SecurityDiscovery(crate::security_discovery::Request),
    HostDataSources(crate::host_monitoring::Request),
    HostSupport(crate::host_support::Request),
    SnapshotPolicySupport(crate::snapshot_policy_support::Request),
    GuestBoot(crate::guest_boot::Request),
    SnapshotPreview(crate::snapshot_preview::Request),
    JoinPeer(
        crate::ObservedResource,
        tokio::sync::oneshot::Sender<Result<JoinPeer, Error>>,
    ),
    Diagnose(crate::diagnostics::DiagnosticRequest),
    Submit(Box<MutationRequest>),
    ResourceSubmit(Box<crate::resources::ResourceRequest>),
    ResourceControl(crate::OperationId, crate::OperationControl),
    Control(crate::OperationId, crate::OperationControl),
    RememberMembers(Vec<crate::Endpoint>),
}

pub(crate) struct ManagementRoute {
    sender: mpsc::Sender<Request>,
    generation: u64,
}
struct ManagementCandidate {
    id: crate::OperationId,
    pool: crate::PoolId,
    generation: u64,
    reply: tokio::sync::oneshot::Sender<Result<(), Error>>,
}
impl ManagementRoute {
    pub async fn register(&self, id: crate::OperationId, pool: crate::PoolId) -> Result<(), Error> {
        let (reply, receive) = tokio::sync::oneshot::channel();
        self.sender
            .try_send(Request::ManagementCandidate(Box::new(
                ManagementCandidate {
                    id,
                    pool,
                    generation: self.generation,
                    reply,
                },
            )))
            .map_err(|_| Error::Busy)?;
        tokio::time::timeout(Duration::from_secs(10), receive)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Closed)?
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStage {
    Login,
    Inventory,
    EventStream,
}
impl ConnectionStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "Connecting and authenticating",
            Self::Inventory => "Synchronizing inventory",
            Self::EventStream => "Maintaining inventory events",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionFailure {
    pub stage: ConnectionStage,
    pub certificate_endpoint: Option<crate::Endpoint>,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    permissions: crate::SessionPermissions,
    generation: u64,
    state: ConnectionState,
    inventory: Option<Arc<Inventory>>,
    error: Option<String>,
    failure: Option<ConnectionFailure>,
    route: Option<String>,
    updated_at: Option<SystemTime>,
    endpoint: Option<crate::Endpoint>,
    verified_members: Vec<crate::Endpoint>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedVm {
    pub(crate) pool: crate::PoolId,
    pub(crate) generation: u64,
    pub(crate) id: crate::VmId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub fn session_permissions(&self) -> crate::SessionPermissions {
        if self.is_current() {
            self.permissions.clone()
        } else {
            crate::SessionPermissions::default()
        }
    }
    pub fn method_permission(&self, method: &str) -> crate::PermissionDecision {
        self.session_permissions().method(method)
    }
    pub fn power_permission(&self, action: crate::PowerAction) -> crate::PermissionDecision {
        self.session_permissions().power(action)
    }
    pub fn connection_route(&self) -> Option<&str> {
        self.route.as_deref()
    }
    pub fn failure(&self) -> Option<&ConnectionFailure> {
        self.failure.as_ref()
    }
    pub fn observe_metrics(
        &self,
        target: crate::MetricTarget,
    ) -> Result<crate::ObservedMetrics, Error> {
        let inventory = self
            .inventory
            .as_ref()
            .filter(|_| self.is_current())
            .ok_or(Error::Stale)?;
        let present = match target {
            crate::MetricTarget::Vm(id) => inventory
                .vms
                .get(&id)
                .is_some_and(|vm| !vm.is_template && !vm.is_control_domain),
            crate::MetricTarget::Host(id) => inventory.hosts.contains_key(&id),
        };
        if !present {
            return Err(Error::Stale);
        }
        Ok(crate::ObservedMetrics {
            pool: inventory.pool_id,
            generation: self.generation,
            target,
        })
    }

    pub fn observe_resource(
        &self,
        target: crate::ResourceTarget,
    ) -> Result<crate::ObservedResource, Error> {
        let inventory = self
            .inventory
            .as_ref()
            .filter(|_| self.is_current())
            .ok_or(Error::Stale)?;
        let present = match target {
            crate::ResourceTarget::Disk(id) => inventory.details.vdis.contains_key(&id),
            crate::ResourceTarget::Vm(id) => inventory.vms.contains_key(&id),
            crate::ResourceTarget::Pif(id) => inventory.details.pifs.contains_key(&id),
            crate::ResourceTarget::Vlan(id) => inventory.details.vlans.contains_key(&id),
            crate::ResourceTarget::Tunnel(id) => {
                inventory.details.network_fabric.tunnels.contains_key(&id)
            }
            crate::ResourceTarget::Sriov(id) => {
                inventory.details.network_fabric.sriovs.contains_key(&id)
            }
            crate::ResourceTarget::Bond(id) => inventory.details.bonds.contains_key(&id),
            crate::ResourceTarget::SnapshotPolicy(id) => {
                inventory.details.snapshot_policies.contains_key(&id)
            }
            crate::ResourceTarget::Pool(id) => id == inventory.pool_id,
            crate::ResourceTarget::Host(id) => inventory.hosts.contains_key(&id),
            crate::ResourceTarget::Task(id) => inventory.details.tasks.contains_key(&id),
            crate::ResourceTarget::Message(id) => inventory.details.messages.contains_key(&id),
            crate::ResourceTarget::Appliance(id) => inventory.details.appliances.contains_key(&id),
            crate::ResourceTarget::Pbd(id) => inventory.details.pbds.contains_key(&id),
            crate::ResourceTarget::Storage(id) => {
                inventory.storage.iter().any(|sr| sr.uuid == id.uuid())
            }
            crate::ResourceTarget::Network(id) => inventory
                .networks
                .iter()
                .any(|network| network.uuid == id.uuid()),
        };
        if !present {
            return Err(Error::Stale);
        }
        Ok(crate::ObservedResource {
            pool: inventory.pool_id,
            generation: self.generation,
            target,
        })
    }

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
    pub fn active_endpoint(&self) -> Option<&crate::Endpoint> {
        self.endpoint.as_ref()
    }
    pub fn verified_members(&self) -> &[crate::Endpoint] {
        &self.verified_members
    }
    pub fn is_current(&self) -> bool {
        self.state == ConnectionState::Ready
    }
}

struct ConnectionLease {
    cancel: CancellationToken,
    canonical_cancel: Mutex<Option<CancellationToken>>,
}
impl ConnectionLease {
    fn disconnect(&self) {
        if let Some(cancel) = self
            .canonical_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            cancel.cancel();
        }
        self.cancel.cancel();
    }
}
impl Drop for ConnectionLease {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
#[derive(Clone)]
struct OwnerRoute {
    receiver: watch::Receiver<Arc<ConnectionSnapshot>>,
    history: watch::Receiver<Arc<Vec<OperationRecord>>>,
    resource_history: watch::Receiver<Arc<Vec<crate::ResourceOperationRecord>>>,
    requests: mpsc::Sender<Request>,
}
struct PoolOwner {
    aliases: Vec<Weak<ConnectionLease>>,
    authority: [u8; 32],
    lease: Weak<ConnectionLease>,
    route: OwnerRoute,
}
struct OwnerContext {
    authority_salt: [u8; 16],
    directory: PathBuf,
    registry: Arc<Mutex<BTreeMap<crate::PoolId, PoolOwner>>>,
    authority: [u8; 32],
    lease: Weak<ConnectionLease>,
    route: OwnerRoute,
}
pub(crate) struct RootCredentialUpdate {
    pub operation: crate::OperationId,
    pub pool: crate::PoolId,
    pub generation: u64,
    pub credentials: Credentials,
    pub reply: tokio::sync::oneshot::Sender<Result<(), Error>>,
}
struct AliasTarget {
    lease: Arc<ConnectionLease>,
    route: OwnerRoute,
}
impl OwnerContext {
    fn rotate_root_credentials(
        &mut self,
        directory: &std::path::Path,
        generation: u64,
        options: &ConnectionOptions,
        credentials: &mut Credentials,
        update: &RootCredentialUpdate,
    ) -> Result<(), Error> {
        let snapshot = self.route.receiver.borrow();
        let owner_lease = self
            .lease
            .upgrade()
            .filter(|lease| !lease.cancel.is_cancelled())
            .ok_or(Error::Closed)?;
        if generation != update.generation
            || !snapshot.is_current()
            || snapshot
                .inventory()
                .is_none_or(|i| i.pool_id != update.pool)
            || credentials.username != "root"
            || update.credentials.username != "root"
        {
            return Err(Error::Stale);
        }
        let authority = options.authority_key(&update.credentials, &self.authority_salt);
        operations::Journal::open(directory)?.root_password_credential_guard(
            update.operation,
            update.pool,
            || {
                let mut owners = self.registry.lock().unwrap_or_else(|p| p.into_inner());
                let owner = owners
                    .get_mut(&update.pool)
                    .filter(|o| Weak::ptr_eq(&o.lease, &self.lease))
                    .ok_or(Error::Stale)?;
                if owner.authority != self.authority || owner_lease.cancel.is_cancelled() {
                    return Err(Error::Stale);
                }
                for alias in owner.aliases.drain(..).filter_map(|alias| alias.upgrade()) {
                    alias.cancel.cancel();
                }
                *credentials =
                    Credentials::new("root".into(), update.credentials.password.to_string());
                owner.authority = authority;
                self.authority = authority;
                Ok(())
            },
        )
    }

    fn claim(&self, pool: crate::PoolId) -> Result<Option<AliasTarget>, Error> {
        let mut owners = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        owners.retain(|_, owner| {
            owner
                .lease
                .upgrade()
                .is_some_and(|lease| !lease.cancel.is_cancelled())
                && !matches!(
                    owner.route.receiver.borrow().state(),
                    ConnectionState::Failed | ConnectionState::Disconnected
                )
        });
        if let Some(owner) = owners.get_mut(&pool) {
            if Weak::ptr_eq(&owner.lease, &self.lease) {
                return Ok(None);
            }
            if let Some(lease) = owner.lease.upgrade() {
                if owner.authority != self.authority {
                    return Err(Error::Connection("This pool is already connected with different credentials, trust or access settings. Disconnect it before reconnecting with the new settings.".into()));
                }
                owner.aliases.retain(|alias| alias.strong_count() > 0);
                if !owner
                    .aliases
                    .iter()
                    .any(|alias| Weak::ptr_eq(alias, &self.lease))
                {
                    owner.aliases.push(self.lease.clone());
                }
                return Ok(Some(AliasTarget {
                    lease,
                    route: owner.route.clone(),
                }));
            }
        }
        owners.insert(
            pool,
            PoolOwner {
                aliases: Vec::new(),
                authority: self.authority,
                lease: self.lease.clone(),
                route: self.route.clone(),
            },
        );
        Ok(None)
    }
}

pub struct Backend {
    runtime: Option<Runtime>,
    state_directory: PathBuf,
    registry: Arc<Mutex<BTreeMap<crate::PoolId, PoolOwner>>>,
    authority_salt: [u8; 16],
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
            registry: Arc::new(Mutex::new(BTreeMap::new())),
            authority_salt: *uuid::Uuid::new_v4().as_bytes(),
        })
    }

    pub fn connect(&self, options: ConnectionOptions, credentials: Credentials) -> Connection {
        let read_only = options.read_only;
        let authority = options.authority_key(&credentials, &self.authority_salt);
        let (sender, receiver) = watch::channel(Arc::new(ConnectionSnapshot {
            permissions: crate::SessionPermissions::default(),
            generation: 0,
            state: ConnectionState::Connecting,
            inventory: None,
            error: None,
            failure: None,
            route: options.connection_route(&options.endpoint).ok(),
            updated_at: None,
            endpoint: None,
            verified_members: Vec::new(),
        }));
        let cancel = CancellationToken::new();
        let lease = Arc::new(ConnectionLease {
            cancel: cancel.clone(),
            canonical_cancel: Mutex::new(None),
        });
        let runtime = self.runtime.as_ref().expect("runtime exists before drop");
        let (requests, receiver_requests) = mpsc::channel(32);
        let (history_sender, history) = watch::channel(Arc::new(Vec::new()));
        let (resource_sender, resource_history) = watch::channel(Arc::new(Vec::new()));
        runtime.spawn(run(
            options,
            credentials,
            sender,
            cancel.clone(),
            receiver_requests,
            OwnerContext {
                authority_salt: self.authority_salt,
                directory: self.state_directory.clone(),
                registry: self.registry.clone(),
                authority,
                lease: Arc::downgrade(&lease),
                route: OwnerRoute {
                    receiver: receiver.clone(),
                    history: history.clone(),
                    resource_history: resource_history.clone(),
                    requests: requests.clone(),
                },
            },
            (history_sender, resource_sender),
        ));
        Connection {
            backend_scope: self.authority_salt,
            lease,
            read_only,
            receiver,
            cancel,
            runtime: runtime.handle().clone(),
            requests,
            history,
            resource_history,
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

#[derive(Clone)]
pub struct Connection {
    backend_scope: [u8; 16],
    lease: Arc<ConnectionLease>,
    read_only: bool,
    receiver: watch::Receiver<Arc<ConnectionSnapshot>>,
    cancel: CancellationToken,
    runtime: tokio::runtime::Handle,
    requests: mpsc::Sender<Request>,
    history: watch::Receiver<Arc<Vec<OperationRecord>>>,
    resource_history: watch::Receiver<Arc<Vec<crate::ResourceOperationRecord>>>,
}
impl Connection {
    pub fn snapshot_preview(
        &self,
        observed: ObservedSnapshot,
    ) -> Result<crate::SnapshotPreviewQuery, Error> {
        if self.latest().observe_snapshot(observed.vm, observed.id)? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) = crate::snapshot_preview::channel(observed, self.runtime.clone());
        self.requests
            .try_send(Request::SnapshotPreview(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn guest_boot_status(
        &self,
        observed: ObservedVm,
    ) -> Result<crate::GuestBootStatusQuery, Error> {
        if self.latest().observe_vm(observed.id)? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) = crate::guest_boot::channel(observed, self.runtime.clone());
        self.requests
            .try_send(Request::GuestBoot(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn snapshot_policy_support(
        &self,
        observed: crate::ObservedResource,
    ) -> Result<crate::SnapshotPolicySupportQuery, Error> {
        if !matches!(observed.target, crate::ResourceTarget::Pool(_))
            || self.latest().observe_resource(observed.target)? != observed
        {
            return Err(Error::Stale);
        }
        let (handle, request) =
            crate::snapshot_policy_support::channel(observed, self.runtime.clone());
        self.requests
            .try_send(Request::SnapshotPolicySupport(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn review_management_bond(
        &self,
        observed: crate::ObservedResource,
        operation: crate::ManagementNetworkOperation,
    ) -> Result<crate::ManagementBondReviewQuery, Error> {
        if self.latest().observe_resource(observed.target())? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) =
            crate::management_bond_review::channel(observed, operation, self.runtime.clone())?;
        self.requests
            .try_send(Request::ManagementBondReview(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn review_management_ipv6_bond(
        &self,
        observed: crate::ObservedResource,
        operation: crate::ManagementIpv6Bond,
    ) -> Result<crate::ManagementIpv6BondReviewQuery, Error> {
        if self.latest().observe_resource(observed.target())? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) =
            crate::management_ipv6_review::channel(observed, operation, self.runtime.clone())?;
        self.requests
            .try_send(Request::ManagementIpv6Review(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn discover_storage(
        &self,
        observed: crate::ObservedResource,
        query: crate::StorageQuery,
    ) -> Result<crate::StorageDiscovery, Error> {
        query.validate()?;
        if !matches!(observed.target, crate::ResourceTarget::Pool(_))
            || self.latest().observe_resource(observed.target)? != observed
        {
            return Err(Error::Stale);
        }
        let (handle, request) =
            crate::storage_discovery::channel(observed, query, self.runtime.clone());
        self.requests
            .try_send(Request::StorageDiscovery(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn discover_authenticated_storage(
        &self,
        observed: crate::ObservedResource,
        query: crate::StorageQuery,
        credentials: crate::StorageCredentials,
    ) -> Result<crate::StorageDiscovery, Error> {
        query.validate()?;
        crate::storage_discovery::validate_credentials(&query, &credentials)?;
        if !matches!(observed.target, crate::ResourceTarget::Pool(_))
            || self.latest().observe_resource(observed.target)? != observed
        {
            return Err(Error::Stale);
        }
        let (handle, mut request) =
            crate::storage_discovery::channel(observed, query, self.runtime.clone());
        request.credentials = Some(credentials);
        self.requests
            .try_send(Request::StorageDiscovery(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn subscribe_metrics(
        &self,
        observed: crate::ObservedMetrics,
        options: crate::MetricsOptions,
    ) -> Result<crate::Metrics, Error> {
        options.validate()?;
        if self.latest().observe_metrics(observed.target)? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) = crate::metrics::channel(observed, options, self.runtime.clone());
        self.requests
            .try_send(Request::Metrics(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }

    pub fn host_data_sources(
        &self,
        observed: crate::ObservedResource,
    ) -> Result<crate::HostDataSourceQuery, Error> {
        if self.latest().observe_resource(observed.target())? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) = crate::host_monitoring::channel(observed, self.runtime.clone())?;
        self.requests
            .try_send(Request::HostDataSources(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn host_support(
        &self,
        observed: crate::ObservedResource,
    ) -> Result<crate::HostSupportQuery, Error> {
        if self.latest().observe_resource(observed.target())? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) = crate::host_support::channel(observed, self.runtime.clone())?;
        self.requests
            .try_send(Request::HostSupport(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }

    pub fn inspect_pool_security(
        &self,
        observed: crate::ObservedResource,
        query: crate::PoolSecurityQuery,
    ) -> Result<crate::PoolSecurityDiscovery, Error> {
        if self.latest().observe_resource(observed.target())? != observed {
            return Err(Error::Stale);
        }
        let (handle, request) =
            crate::security_discovery::channel(observed, query, self.runtime.clone())?;
        self.requests
            .try_send(Request::SecurityDiscovery(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }

    pub fn resource_history(&self) -> Arc<Vec<crate::ResourceOperationRecord>> {
        self.resource_history.borrow().clone()
    }
    pub fn submit_resource(
        &self,
        id: crate::OperationId,
        command: crate::ResourceCommand,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let (observed, action, inputs) = command.submission();
        // A join and a batch of cross-pool moves need a second connection, which only
        // their own submissions carry.
        if crate::pool_join::destination(&action).is_some() || action.spans_pools() {
            return Err(Error::InvalidResourceCommand);
        }
        if let crate::ResourceAction::Pool(crate::PoolOperation::Security(op)) = &action
            && (op.secret_required() != inputs.is_some()
                || inputs.as_ref().is_some_and(|v| !v.valid_for(op)))
        {
            return Err(Error::InvalidResourceCommand);
        }
        if self.latest().observe_resource(observed.target)? != observed {
            return Err(Error::Stale);
        }
        action.validate(observed.target)?;
        let (operation, mut request) =
            crate::resources::channel(id, observed, action, self.runtime.clone());
        request.sensitive = inputs;
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(operation)
    }
    pub fn submit_vm_workflow_batch(
        &self,
        id: crate::OperationId,
        pool: crate::ObservedResource,
        selection: crate::VmWorkflowBatch,
        outputs: Vec<(crate::VmId, crate::TransferFile)>,
    ) -> Result<crate::ResourceOperation, Error> {
        // Several snapshots of one VM share that VM, so their outputs are named by snapshot.
        // Moves to another pool are submitted with that pool's connection.
        if selection.is_snapshot_export() || selection.cross_pool_destination().is_some() {
            return Err(Error::InvalidResourceCommand);
        }
        let outputs = outputs.into_iter().map(|(vm, file)| (vm.uuid(), file));
        self.submit_workflow_batch(id, pool, selection, outputs.collect())
    }
    /// Submits a reviewed batch of snapshot and checkpoint exports, each output named by
    /// the snapshot it is for.
    pub fn submit_snapshot_export_batch(
        &self,
        id: crate::OperationId,
        pool: crate::ObservedResource,
        selection: crate::VmWorkflowBatch,
        outputs: Vec<(crate::SnapshotId, crate::TransferFile)>,
    ) -> Result<crate::ResourceOperation, Error> {
        if !selection.is_snapshot_export() {
            return Err(Error::InvalidResourceCommand);
        }
        let outputs = outputs
            .into_iter()
            .map(|(snapshot, file)| (snapshot.uuid(), file));
        self.submit_workflow_batch(id, pool, selection, outputs.collect())
    }
    fn submit_workflow_batch(
        &self,
        id: crate::OperationId,
        pool: crate::ObservedResource,
        selection: crate::VmWorkflowBatch,
        outputs: Vec<(uuid::Uuid, crate::TransferFile)>,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only && !selection.is_export() {
            return Err(Error::ReadOnly);
        }
        if self.latest().observe_resource(pool.target)? != pool {
            return Err(Error::Stale);
        }
        selection.validate(pool.target)?;
        let outputs = crate::vm_workflow_export::bind(&selection, outputs)?;
        let (handle, mut request) = crate::resources::channel(
            id,
            pool,
            crate::ResourceAction::VmWorkflowBatch(selection),
            self.runtime.clone(),
        );
        request
            .record
            .workflow_batch_plan
            .as_mut()
            .ok_or(Error::Journal)?
            .outputs = outputs;
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn upload_snapshot_preview(
        &self,
        id: crate::OperationId,
        snapshot: crate::ObservedSnapshot,
        source: crate::OperationId,
        image: crate::SnapshotPreviewImage,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let latest = self.latest();
        if latest.observe_snapshot(snapshot.vm, snapshot.id)? != snapshot {
            return Err(Error::Stale);
        }
        let parent = latest.observe_resource(crate::ResourceTarget::Vm(snapshot.vm))?;
        let action =
            crate::ResourceAction::SnapshotPreviewUpload(image.descriptor(source, snapshot.id));
        action.validate(parent.target)?;
        let (handle, mut request) =
            crate::resources::channel(id, parent, action, self.runtime.clone());
        request.snapshot_preview_image = Some(image);
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn change_pool_root_password(
        &self,
        id: crate::OperationId,
        pool: crate::ObservedResource,
        expected: crate::PoolRootPasswordChange,
        password: crate::RootPassword,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        if self.latest().observe_resource(pool.target)? != pool {
            return Err(Error::Stale);
        }
        let action = crate::ResourceAction::RootPassword(expected);
        action.validate(pool.target)?;
        let (handle, mut request) =
            crate::resources::channel(id, pool, action, self.runtime.clone());
        request.root_password = Some(password);
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    pub fn configure_host_power(
        &self,
        id: crate::OperationId,
        host: crate::ObservedResource,
        expected: crate::HostPowerConfiguration,
        settings: crate::HostPowerSettings,
        password: Option<crate::HostPowerPassword>,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        if self.latest().observe_resource(host.target)? != host {
            return Err(Error::Stale);
        }
        if settings.needs_password() != password.is_some() {
            return Err(Error::InvalidResourceCommand);
        }
        let action = crate::ResourceAction::HostPower(crate::HostPowerOperation::Configure {
            expected,
            settings,
        });
        action.validate(host.target)?;
        let (handle, mut request) =
            crate::resources::channel(id, host, action, self.runtime.clone());
        request.host_power_password = password;
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    /// Provision authenticated storage without persisting clear credentials.
    pub fn submit_authenticated_storage(
        &self,
        id: crate::OperationId,
        target: crate::ObservedResource,
        operation: crate::StorageOperation,
        credentials: crate::StorageCredentials,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        if self.latest().observe_resource(target.target)? != target {
            return Err(Error::Stale);
        }
        credentials.validate(&operation)?;
        let action = crate::ResourceAction::AuthenticatedStorage {
            operation,
            authentication: credentials.authentication,
        };
        action.validate(target.target)?;
        let (handle, mut request) =
            crate::resources::channel(id, target, action, self.runtime.clone());
        request.storage_credentials = Some(credentials);
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
    /// Transfer a selected archive or disk image through fixed authenticated endpoints.
    pub fn prepare_cloud_init(
        &self,
        id: crate::OperationId,
        vm: ObservedVm,
        sr: crate::SrId,
        device: String,
        media: crate::CloudInitMedia,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let latest = self.latest();
        if latest.observe_vm(vm.id)? != vm || media.vm != vm.id {
            return Err(Error::Stale);
        }
        let target = latest.observe_resource(crate::ResourceTarget::Vm(vm.id))?;
        let config = crate::CloudInitConfiguration {
            sr,
            device,
            configuration_id: media.id,
        };
        let inventory = latest.inventory.as_ref().ok_or(Error::Stale)?;
        if !config.availability(inventory, vm.id).is_available() {
            return Err(Error::UnavailableAction);
        }
        let (operation, mut pending) = crate::resources::channel(
            id,
            target,
            crate::ResourceAction::CloudInit(config.clone()),
            self.runtime.clone(),
        );
        pending.record.transfer = Some(Default::default());
        pending.record.transfer_plan =
            Some(Box::new(crate::transfer::Plan::new(media.file.clone())));
        pending.record.cloud_init_plan = Some(Box::new(crate::cloud_init::Plan {
            operation: id,
            pool: vm.pool,
            vm: vm.id,
            configuration: config,
            sha256: media.sha256,
            interrupted: false,
            upload_verified: false,
            attachment: None,
        }));
        pending.cloud_media = Some(media);
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(pending)))
            .map_err(|_| Error::Busy)?;
        Ok(operation)
    }
    pub fn transfer(
        &self,
        id: crate::OperationId,
        pool: crate::ObservedResource,
        request: crate::TransferRequest,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only && !request.action.is_export() {
            return Err(Error::ReadOnly);
        }
        if self.latest().observe_resource(pool.target)? != pool {
            return Err(Error::Stale);
        }
        if !matches!(pool.target, crate::ResourceTarget::Pool(_)) {
            return Err(Error::TransferConflict);
        }
        if request.action.is_export() != request.file.identity.is_none() {
            return Err(Error::TransferFile);
        }
        match &request.action {
            crate::TransferAction::Host {
                host,
                expected_version,
                operation: crate::HostArchiveOperation::StageRestore { archive },
            } if request.file.host_backup.as_ref() != Some(archive)
                || archive.host != *host
                || archive.version != *expected_version
                || request.file.manifest.is_some()
                || request.file.disk.is_some()
                || request.file.ovf.is_some() =>
            {
                return Err(Error::TransferFile);
            }
            crate::TransferAction::ImportOvf { networks, .. }
                if request.file.ovf.as_ref().is_none_or(|o| {
                    o.summary
                        .networks
                        .iter()
                        .cloned()
                        .collect::<std::collections::BTreeSet<_>>()
                        != networks.keys().cloned().collect()
                }) =>
            {
                return Err(Error::TransferFile);
            }
            crate::TransferAction::ImportXva { .. }
            | crate::TransferAction::ImportCheckpointXva { .. }
                if request.file.manifest.as_ref().is_none_or(|a| {
                    (a.memory_images() > 0)
                        != matches!(
                            request.action,
                            crate::TransferAction::ImportCheckpointXva { .. }
                        )
                }) || request.file.disk.is_some()
                    || request.file.ovf.is_some() =>
            {
                return Err(Error::TransferFile);
            }
            crate::TransferAction::ImportDisk { format, .. }
                if request
                    .file
                    .disk
                    .as_ref()
                    .is_none_or(|d| d.format != *format)
                    || request.file.manifest.is_some()
                    || request.file.ovf.is_some() =>
            {
                return Err(Error::TransferFile);
            }
            _ => {}
        }
        let (operation, pending) =
            crate::resources::transfer_channel(id, pool, request, self.runtime.clone());
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(pending)))
            .map_err(|_| Error::Busy)?;
        Ok(operation)
    }
    pub fn control_resource(
        &self,
        id: crate::OperationId,
        control: crate::OperationControl,
    ) -> Result<(), Error> {
        if matches!(control, crate::OperationControl::Recheck)
            && self
                .resource_history()
                .iter()
                .any(|r| r.id == id && r.action.spans_pools())
        {
            return Err(Error::Protocol(
                "Use the cross-pool transfer view to recheck with both connected pools",
            ));
        }
        if self.read_only
            && !self
                .resource_history()
                .iter()
                .any(|r| r.id == id && r.action.read_only_export())
        {
            return Err(Error::ReadOnly);
        }
        if !self.latest().is_current() {
            return Err(Error::Stale);
        }
        self.requests
            .try_send(Request::ResourceControl(id, control))
            .map_err(|_| Error::Busy)
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn diagnose_power(
        &self,
        target: ObservedVm,
        action: crate::PowerAction,
    ) -> Result<crate::ActionDiagnostic, Error> {
        let snapshot = self.latest();
        if !snapshot.is_current()
            || snapshot.generation != target.generation
            || snapshot
                .inventory()
                .is_none_or(|i| i.pool_id != target.pool)
        {
            return Err(Error::Stale);
        }
        let (handle, request) = crate::diagnostics::channel(target, action, self.runtime.clone());
        self.requests
            .try_send(Request::Diagnose(request))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
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
            .try_send(Request::Submit(Box::new(request)))
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
            || !snapshot.inventory().is_some_and(|i| {
                i.pool_id == observed.pool
                    && i.vms
                        .get(&vm)
                        .is_some_and(|v| !v.is_template && !v.is_control_domain)
            })
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
    pub fn open_host_console(
        &self,
        observed: crate::ObservedResource,
        control_domain: crate::VmId,
    ) -> Result<crate::Console, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let crate::ResourceTarget::Host(host) = observed.target else {
            return Err(Error::Stale);
        };
        let snapshot = self.latest();
        let inventory = snapshot
            .inventory()
            .filter(|i| {
                snapshot.is_current()
                    && snapshot.generation == observed.generation
                    && i.pool_id == observed.pool
            })
            .ok_or(Error::Stale)?;
        let vm = inventory
            .details
            .hosts
            .get(&host)
            .and_then(|h| h.control_domain)
            .and_then(|id| inventory.vms.get(&id))
            .filter(|vm| {
                vm.id == control_domain
                    && vm.is_control_domain
                    && !vm.is_template
                    && vm.host == Some(host)
                    && vm.power_state == crate::PowerState::Running
            })
            .ok_or(Error::NoConsole)?;
        let (console, mut request) = console::channel(
            vm.id,
            snapshot.generation,
            self.cancel.child_token(),
            self.runtime.clone(),
        );
        request.host = Some(host);
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
        self.lease.disconnect();
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        let mut history = self.history.clone();
        let mut resource_history = self.resource_history.clone();
        self.runtime.spawn(async move {
            loop {
                tokio::select! {
                    result = resource_history.changed() => { callback(); if result.is_err() { break; } }
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
        self.lease.disconnect();
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
type HistorySenders = (
    watch::Sender<Arc<Vec<OperationRecord>>>,
    watch::Sender<Arc<Vec<crate::ResourceOperationRecord>>>,
);

async fn run(
    mut options: ConnectionOptions,
    mut credentials: Credentials,
    sender: watch::Sender<Arc<ConnectionSnapshot>>,
    cancel: CancellationToken,
    mut requests: mpsc::Receiver<Request>,
    mut context: OwnerContext,
    histories: HistorySenders,
) {
    let (history, resource_history) = histories;
    let directory = context.directory.clone();
    let mut management_candidates =
        BTreeMap::<String, (crate::HostId, crate::CertificateFingerprint)>::new();
    if let Some(pool) = options.expected_pool
        && directory.join("operations.sqlite3").exists()
        && let Ok(rows) =
            operations::Journal::open(&directory).and_then(|j| j.resource_history(pool))
    {
        for row in rows.into_iter().filter(|row| {
            !row.state.terminal()
                || row.state == crate::OperationState::PartiallyCompleted
                    && row
                        .management_ipv6_bond
                        .as_ref()
                        .is_some_and(|v| v.retains_reservation)
        }) {
            if let Some(p) = row.management_ipv6_bond_plan.filter(|p| p.registered) {
                for endpoint in [p.temporary_endpoint, p.final_endpoint] {
                    if !options.fallback_endpoints.contains(&endpoint)
                        && options.fallback_endpoints.len() < 128
                    {
                        options.fallback_endpoints.push(endpoint.clone());
                    }
                    management_candidates
                        .insert(endpoint.to_string(), (p.host, p.certificate.clone()));
                }
            }
            if let Some(plan) = row.management_network_plan.filter(|plan| plan.registered) {
                if !options.fallback_endpoints.contains(&plan.recovery)
                    && options.fallback_endpoints.len() < 128
                {
                    options.fallback_endpoints.push(plan.recovery.clone());
                }
                management_candidates
                    .insert(plan.recovery.to_string(), (plan.host, plan.certificate));
            }
        }
    }
    let mut expected_pool = options.expected_pool;
    let mut protocol_failures = 0u8;
    let (updates, mut operation_updates) = mpsc::channel::<OperationUpdate>(128);
    let mut records = BTreeMap::new();
    let mut resource_records = BTreeMap::new();
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
        let mut failure_stage = ConnectionStage::Login;
        let result = tokio::select! {
            _=cancel.cancelled()=>break,
            result=async {
                if let Some((_,certificate))=management_candidates.get(options.endpoint.as_str()) {Session::login_bound(&options,&credentials,certificate).await}
                else {Session::login(&options,&credentials).await}
            }=>result,
        };
        let error = match result {
            Ok(session) => {
                failure_stage = ConnectionStage::Inventory;
                let session = Arc::new(session);
                let mut authenticated = (*sender.borrow().clone()).clone();
                authenticated.permissions = session.permissions().clone();
                sender.send_replace(Arc::new(authenticated));
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
                let mut diagnostics = tokio::task::JoinSet::new();
                let mut discoveries = tokio::task::JoinSet::new();
                let mut metrics = tokio::task::JoinSet::new();
                let mut resource_workers = tokio::task::JoinSet::new();
                let mut resource_counts = BTreeMap::<crate::OperationId, usize>::new();
                let mut resource_rechecks = BTreeSet::new();
                let mut vm_rechecks = BTreeSet::new();
                let mut cross_pool_rechecks =
                    BTreeMap::<crate::OperationId, crate::resources::ResourceRequest>::new();
                let worker_options = options.clone();
                let management_route = context.route.requests.clone();
                let start_resource =
                    |mut request: crate::resources::ResourceRequest,
                     workers: &mut tokio::task::JoinSet<crate::OperationId>,
                     counts: &mut BTreeMap<crate::OperationId, usize>,
                     credentials: &Credentials| {
                        if matches!(
                            request.record.action,
                            crate::ResourceAction::ManagementNetwork(_)
                                | crate::ResourceAction::ManagementIpv6Bond(_)
                        ) {
                            request.management = Some(crate::management_network::Inputs {
                                options: worker_options.clone(),
                                credentials: Credentials::new(
                                    credentials.username.clone(),
                                    credentials.password.to_string(),
                                ),
                                route: ManagementRoute {
                                    sender: management_route.clone(),
                                    generation,
                                },
                            });
                        }
                        *counts.entry(request.record.id).or_default() += 1;
                        workers.spawn(request.run(
                            session.clone(),
                            directory.clone(),
                            active.clone(),
                            updates.clone(),
                        ));
                    };
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
                // The inventory is kept current by events alone. As a check on that, the
                // host records are read directly now and then and compared with it. An
                // event carries its object's whole record, so a host that differs at two
                // checks running with no event for it applied in between is one whose
                // events are not arriving or not being applied, and the whole inventory is
                // read again (lab finding 64). A host that differs twice while its events
                // do arrive is only changing, and is left alone. The read runs beside the
                // loop and only its result is handled here, so the inventory keeps its one
                // writer.
                let check_every = options.inventory_check_interval;
                let mut inventory_check = tokio::time::interval_at(
                    tokio::time::Instant::now() + check_every.max(Duration::from_millis(1)),
                    check_every.max(Duration::from_millis(1)),
                );
                inventory_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut host_reads =
                    tokio::task::JoinSet::<Result<serde_json::Value, Error>>::new();
                // For each host found different at the last check, how many batches with
                // an event for it had been applied by then; and that count for every host.
                let mut drifting = std::collections::BTreeMap::<String, u64>::new();
                let mut host_events = std::collections::BTreeMap::<String, u64>::new();
                let mut rebuilding = false;
                let events = next_events(session.clone(), token.clone());
                tokio::pin!(events);
                let error = loop {
                    let mut batch = tokio::select! {
                        _=cancel.cancelled()=>break None,
                        _=inventory_check.tick(), if !check_every.is_zero() && !token.is_empty() && host_reads.is_empty()=>{
                            let session = session.clone();
                            host_reads.spawn(async move { session.call("host.get_all_records", vec![]).await });
                            continue;
                        }
                        read=host_reads.join_next(), if !host_reads.is_empty()=>{
                            // A read that failed says nothing: the event stream reports
                            // a connection that is going away by itself.
                            if let Some(Ok(Ok(fresh))) = read
                                && !token.is_empty()
                            {
                                let seen = |host: &String| host_events.get(host).copied().unwrap_or(0);
                                let now = store.host_drift(&fresh);
                                if now.iter().any(|host| drifting.get(host) == Some(&seen(host))) {
                                    if crate::xapi::rpc_trace_enabled() {
                                        eprintln!("xenadmin inventory check: host records differed from the inventory at two checks running; reading the inventory again");
                                    }
                                    drifting.clear();
                                    rebuilding = true;
                                    token.clear();
                                    events.set(next_events(session.clone(), String::new()));
                                    publish(&sender,generation,session_state(&token),&last_inventory,journal_error.clone(),last_update);
                                } else {
                                    drifting = now.into_iter().map(|host| { let count = seen(&host); (host, count) }).collect();
                                }
                            }
                            continue;
                        }
                        _=diagnostics.join_next(), if !diagnostics.is_empty()=>{continue;}
                        _=discoveries.join_next(), if !discoveries.is_empty()=>{continue;}
                        _=metrics.join_next(), if !metrics.is_empty()=>{continue;}
                        done=resource_workers.join_next(), if !resource_workers.is_empty()=>{
                            match done {
                                Some(Ok(id))=>{
                                    if let Some(count)=resource_counts.get_mut(&id) {*count-=1;if *count==0 {resource_counts.remove(&id);}}
                                    if !resource_counts.contains_key(&id)
                                        && let Some(request) = cross_pool_rechecks.remove(&id) {
                                        start_resource(request,&mut resource_workers,&mut resource_counts,&credentials);
                                    }
                                    if !resource_counts.contains_key(&id)&&resource_rechecks.remove(&id) {
                                        match operations::Journal::open(&directory).and_then(|journal|journal.resource_load(id)) {
                                            Ok(Some(record)) if !record.state.terminal() && !record.action.spans_pools()=>start_resource(crate::resources::ResourceRequest::tracking(record),&mut resource_workers,&mut resource_counts,&credentials),
                                            Err(error)=>publish(&sender,generation,session_state(&token),&last_inventory,Some(error.to_string()),last_update),
                                            _=>{},
                                        }
                                    }
                                },
                                Some(Err(_))=>break Some(Error::Connection("An infrastructure operation worker stopped unexpectedly. Reconnect to reconcile its journal.".into())),None=>{},
                            }continue;
                        }
                        done=workers.join_next(), if !workers.is_empty()=>{
                            match done {
                                Some(Ok(id))=>{
                                    if let Some(count)=worker_counts.get_mut(&id) { *count-=1; if *count==0 {worker_counts.remove(&id);} }
                                    if !worker_counts.contains_key(&id) && vm_rechecks.remove(&id) {
                                        match operations::Journal::open(&directory).and_then(|journal|journal.load(id)) {
                                            Ok(Some(record)) if record.batch_parent.is_none() && !record.state.terminal() && record.can_recheck()=>start_worker(MutationRequest::tracking(record),None,&mut workers,&mut worker_counts),
                                            Err(error)=>publish(&sender,generation,session_state(&token),&last_inventory,Some(error.to_string()),last_update),
                                            _=>{},
                                        }
                                    }
                                },
                                Some(Err(_))=>break Some(Error::Connection("An operation worker stopped unexpectedly. Reconnect to reconcile its journal.".into())),
                                None=>{},
                            }
                            continue;
                        }
                        Some(update)=operation_updates.recv()=>{
                            match update {
                                OperationUpdate::RootCredentials(update)=>{
                                    let result=context.rotate_root_credentials(&directory,generation,&options,&mut credentials,&update);
                                    let _=update.reply.send(result);
                                },
                                OperationUpdate::Coordinator(endpoint)=>break Some(Error::Coordinator(endpoint)),
                                OperationUpdate::Resource(record)=>{merge_resource_record(&mut resource_records,*record);publish_resource_history(&resource_history,&mut resource_records);},
                                OperationUpdate::Record(record)=>{merge_record(&mut records,*record);publish_history(&history,&mut records);},
                            }
                            continue;
                        }
                        _=refresh.tick(), if recovered && !options.read_only=>{
                            if !directory.join("operations.sqlite3").exists() {continue;}
                            if let Some(inventory)=&last_inventory {
                                match operations::Journal::open(&directory).and_then(|journal|journal.resource_history(inventory.pool_id)) {
                                    Ok(rows)=>{for record in rows {merge_resource_record(&mut resource_records,record);}publish_resource_history(&resource_history,&mut resource_records);},
                                    Err(error)=>{journal_error=Some(error.to_string());publish(&sender,generation,session_state(&token),&last_inventory,journal_error.clone(),last_update);continue;}
                                }
                                match operations::Journal::open(&directory).and_then(|journal|journal.history()) {
                                    Ok(rows)=>{
                                        journal_error=None;
                                        for record in rows.into_iter().filter(|record|record.pool==inventory.pool_id) {
                                            if record.batch_parent.is_none() && !record.state.terminal() && record.has_task()
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
                                        publish(&sender,generation,session_state(&token),&last_inventory,journal_error.clone(),last_update);
                                    },
                                }
                            }
                            continue;
                        }
                        Some(request)=requests.recv()=>{
                            match request {
                                Request::ResourceSubmit(request)=>{
                                    if operations::Journal::open(&directory).and_then(|j|j.workflow_owner(request.record.id)).map_or(true,|p|p.is_some()) {request.reject(Error::OperationConflict);}
                                    else if options.read_only && !request.record.action.read_only_export() {request.reject(Error::ReadOnly);}
                                    else if matches!(request.record.action,crate::ResourceAction::RootPassword(_)) && credentials.username != "root" {request.reject(Error::HostPrerequisite("Reconnect as local root before changing the pool root password"));}
                                    else if request.generation!=generation || token.is_empty() || !last_inventory.as_ref().is_some_and(|i|i.pool_id==request.record.pool) {request.reject(Error::Stale);}
                                    else if journal_error.is_some() {request.reject(Error::Journal);}
                                    else if request.record.action.spans_pools() && request.record.revision()>0 && resource_counts.contains_key(&request.record.id) {
                                        match cross_pool_rechecks.entry(request.record.id) {
                                            std::collections::btree_map::Entry::Vacant(entry)=>{request.queued_recheck();entry.insert(*request);},
                                            std::collections::btree_map::Entry::Occupied(_)=>request.reject(Error::Busy),
                                        }
                                    }
                                    else if resource_workers.len()>=8 {request.reject(Error::Busy);}
                                    else {start_resource(*request,&mut resource_workers,&mut resource_counts,&credentials);}
                                }
                                Request::ResourceControl(id,control)=>{
                                    let result=(|| {
                                        let pool=last_inventory.as_ref().filter(|_|!token.is_empty()).ok_or(Error::Stale)?.pool_id;
                                        if !resource_counts.contains_key(&id)&&resource_workers.len()>=8 {return Err(Error::Busy);}
                                        let journal=operations::Journal::open(&directory)?;
                                        if options.read_only && !journal.resource_load(id)?.is_some_and(|r|r.pool==pool&&r.action.read_only_export()){return Err(Error::ReadOnly)}
                                        let record=journal.resource_control(pool,id,control)?;
                                        if !record.state.terminal() && !record.action.spans_pools() {
                                            if !resource_counts.contains_key(&id) {start_resource(crate::resources::ResourceRequest::tracking(record.clone()),&mut resource_workers,&mut resource_counts,&credentials);}
                                            else if record.state==crate::OperationState::OutcomeUnknown {resource_rechecks.insert(id);}
                                        }
                                        merge_resource_record(&mut resource_records,record);publish_resource_history(&resource_history,&mut resource_records);Ok::<_,Error>(())
                                    })();
                                    if let Err(error)=result {publish(&sender,generation,session_state(&token),&last_inventory,Some(error.to_string()),last_update);}
                                }
                                Request::Submit(request)=>{
                                    if operations::Journal::open(&directory).and_then(|j|j.workflow_owner(request.record.id)).map_or(true,|p|p.is_some()) {request.reject(Error::OperationConflict);}
                                    else if options.read_only {request.reject(Error::ReadOnly);}
                                    else if request.generation!=generation || token.is_empty() || !last_inventory.as_ref().is_some_and(|inventory|inventory.pool_id==request.record.pool) {
                                        request.reject(Error::Stale);
                                    } else if journal_error.is_some() {request.reject(Error::Journal);}
                                    else if matches!(&request.record.action, crate::OperationAction::Recovery(operation) if operation.migration_unaccepted()) {
                                        // Refused in its own words, before the fresh check
                                        // that would only call it unavailable.
                                        request.reject(Error::ForceDuringMigration);
                                    }
                                    else if !store.allows(request.record.vm,&request.record.action)
                                        && !operations::Journal::open(&directory).is_ok_and(|journal|journal.knows(&request.record)) {
                                        request.reject(Error::UnavailableAction);
                                    } else if let Some(reference)=store.vm_reference(request.record.vm) {
                                        start_worker(*request,Some(reference),&mut workers,&mut worker_counts);
                                    } else {request.reject(Error::Stale);}
                                }
                                Request::JoinPeer(observed, reply)=>{
                                    let value=if options.read_only {Err(Error::ReadOnly)} else if context.route.receiver.borrow().observe_resource(observed.target()).is_ok_and(|v|v==observed) {
                                        Ok(JoinPeer{session:session.clone(),receiver:context.route.receiver.clone(),cancel:active.clone()})
                                    } else {Err(Error::Stale)};
                                    let _=reply.send(value);
                                }
                                Request::ManagementBondReview(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::ManagementIpv6Review(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::HostDataSources(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::HostSupport(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::SecurityDiscovery(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::GuestBoot(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::SnapshotPreview(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::SnapshotPolicySupport(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::StorageDiscovery(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|i|i.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if discoveries.len()>=4 {request.fail(Error::Busy);}
                                    else {discoveries.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone()));}
                                }
                                Request::Metrics(request)=>{
                                    if request.observed.generation!=generation||token.is_empty()||!last_inventory.as_ref().is_some_and(|inventory|inventory.pool_id==request.observed.pool) {request.fail(Error::Stale);}
                                    else if metrics.len()>=4 {request.fail(Error::Busy);}
                                    else {metrics.spawn(request.run(session.clone(),context.route.receiver.clone(),active.clone(),updates.clone()));}
                                }
                                Request::Diagnose(request)=>{
                                    if request.target.generation!=generation || token.is_empty() || !last_inventory.as_ref().is_some_and(|i|i.pool_id==request.target.pool) {request.fail(Error::Stale);}
                                    else if diagnostics.len()>=16 {request.fail(Error::Busy);}
                                    else { diagnostics.spawn(request.run(session.clone(),active.clone())); }
                                }
                                Request::Console(request)=>{
                                    if options.read_only {request.fail(Error::ReadOnly);}
                                    else if request.generation==generation && !token.is_empty() {
                                        if let (Some(reference),Some(inventory))=(store.vm_reference(request.vm),last_inventory.clone()) {
                                            tokio::spawn(request.run(session.clone(),reference,inventory,active.clone()));
                                        } else {request.fail(Error::Stale);}
                                    } else {request.fail(Error::Stale);}
                                }
                                Request::ManagementCandidate(request)=>{
                                    let result=(|| {
                                        if request.generation!=generation || token.is_empty() || last_inventory.as_ref().is_none_or(|i|i.pool_id!=request.pool) {return Err(Error::Stale);}
                                        let record=operations::Journal::open(&directory)?.resource_load(request.id)?.ok_or(Error::Journal)?;
                                        if record.pool!=request.pool || record.state.terminal() || record.interrupted {return Err(Error::Stale);}
                                        let candidates=if let Some(plan)=record.management_network_plan{vec![(plan.recovery,plan.host,plan.certificate)]}else if let Some(plan)=record.management_ipv6_bond_plan{vec![(plan.temporary_endpoint,plan.host,plan.certificate.clone()),(plan.final_endpoint,plan.host,plan.certificate)]}else{return Err(Error::Journal);};
                                        for (endpoint,host,certificate) in candidates{if !options.fallback_endpoints.contains(&endpoint){if options.fallback_endpoints.len()>=128{return Err(Error::Busy);}options.fallback_endpoints.push(endpoint.clone());}management_candidates.insert(endpoint.to_string(),(host,certificate));}
                                        Ok(())
                                    })();
                                    let _=request.reply.send(result);
                                }
                                Request::RememberMembers(endpoints)=>{
                                    let mut snapshot = (*sender.borrow().clone()).clone();
                                    for endpoint in endpoints.into_iter().take(128) {
                                        if snapshot.verified_members.len() < 128 && !snapshot.verified_members.contains(&endpoint) {
                                            snapshot.verified_members.push(endpoint.clone());
                                        }
                                        if options.fallback_endpoints.len() < 128 && !options.fallback_endpoints.contains(&endpoint) {
                                            options.fallback_endpoints.push(endpoint);
                                        }
                                    }
                                    sender.send_replace(Arc::new(snapshot));
                                }
                                Request::Control(id,control)=>{
                                    let result=(||{
                                        if options.read_only {return Err(Error::ReadOnly);}
                                        let pool=last_inventory.as_ref().filter(|_|!token.is_empty()).ok_or(Error::Stale)?.pool_id;
                                        let journal=operations::Journal::open(&directory)?;
                                        if journal.workflow_owner(id)?.is_some() {return Err(Error::UnavailableAction);}
                                        let record=match control {
                                            crate::OperationControl::RequestCancellation=>journal.control(pool,id,false)?,
                                            crate::OperationControl::AcknowledgeUnknown=>journal.control(pool,id,true)?,
                                            crate::OperationControl::Recheck=>journal.load(id)?.filter(|record|record.pool==pool && !record.state.terminal() && record.can_recheck()).ok_or(Error::UnavailableAction)?,
                                        };
                                        if !record.state.terminal() && record.can_recheck() {
                                            if !worker_counts.contains_key(&id) {
                                                start_worker(MutationRequest::tracking(record.clone()),None,&mut workers,&mut worker_counts);
                                            } else if matches!(control,crate::OperationControl::Recheck) {
                                                vm_rechecks.insert(id);
                                            }
                                        }
                                        merge_record(&mut records,record);
                                        publish_history(&history,&mut records);
                                        Ok::<_,Error>(())
                                    })();
                                    if let Err(error)=result {
                                        publish(&sender,generation,session_state(&token),&last_inventory,Some(error.to_string()),last_update);
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
                    if rebuilding && token.is_empty() {
                        batch.rebuild();
                        rebuilding = false;
                    }
                    for host in batch.touched("host") {
                        *host_events.entry(host.to_owned()).or_default() += 1;
                    }
                    if !batch.events.is_empty() || token.is_empty() {
                        match store.apply(&batch) {
                            Ok(inventory) => {
                                if expected_pool.is_some_and(|pool| pool != inventory.pool_id) {
                                    break Some(Error::Connection("The endpoint now belongs to a different pool. Create a separate profile; this connection will not change its target pool.".into()));
                                }
                                if let Some((host, _)) =
                                    management_candidates.get(options.endpoint.as_str())
                                {
                                    let candidate = url::Url::parse(options.endpoint.as_str())
                                        .ok()
                                        .and_then(|v| v.host_str().map(str::to_owned));
                                    if inventory.hosts.get(host).is_none_or(|h| {
                                        Some(h.address.trim_matches(['[', ']']).to_owned())
                                            != candidate
                                                .map(|c| c.trim_matches(['[', ']']).to_owned())
                                    }) {
                                        break Some(Error::Protocol(
                                            "Recovery endpoint host address identity differs from the reviewed host",
                                        ));
                                    }
                                }
                                expected_pool = Some(inventory.pool_id);
                                {
                                    let mut snapshot = (*sender.borrow().clone()).clone();
                                    snapshot.endpoint = Some(session.options().endpoint.clone());
                                    snapshot.route = session
                                        .options()
                                        .connection_route(&session.options().endpoint)
                                        .ok();
                                    // Every pool member whose certificate this connection can
                                    // already verify is a remembered endpoint for failover,
                                    // so a coordinator loss can be followed to a surviving
                                    // member that takes over (lab finding 35).
                                    let pins = options.host_certificates();
                                    let members = inventory
                                        .hosts
                                        .values()
                                        .filter_map(|h| {
                                            let address = h.address.trim_matches(['[', ']']);
                                            if address.is_empty() {
                                                return None;
                                            }
                                            let text = if address.contains(':') {
                                                format!("https://[{address}]/")
                                            } else {
                                                format!("https://{address}/")
                                            };
                                            let endpoint = crate::Endpoint::parse(&text).ok()?;
                                            (options.accept_invalid_certificates
                                                || pins.contains_key(&endpoint.to_string()))
                                            .then_some(endpoint)
                                        })
                                        .take(128)
                                        .collect::<Vec<_>>();
                                    for endpoint in [&options.endpoint, &session.options().endpoint]
                                        .into_iter()
                                        .chain(members.iter())
                                    {
                                        if snapshot.verified_members.len() < 128
                                            && !snapshot.verified_members.contains(endpoint)
                                        {
                                            snapshot.verified_members.push(endpoint.clone());
                                        }
                                        if options.fallback_endpoints.len() < 128
                                            && !options.fallback_endpoints.contains(endpoint)
                                        {
                                            options.fallback_endpoints.push(endpoint.clone());
                                        }
                                    }
                                    sender.send_replace(Arc::new(snapshot));
                                }
                                match context.claim(inventory.pool_id) {
                                    Ok(Some(target)) => {
                                        let endpoints = sender.borrow().verified_members.clone();
                                        let _ = target
                                            .route
                                            .requests
                                            .try_send(Request::RememberMembers(endpoints));
                                        active.cancel();
                                        let _ = tokio::time::timeout(
                                            Duration::from_secs(3),
                                            session.logout(),
                                        )
                                        .await;
                                        if let Some(lease) = context.lease.upgrade() {
                                            *lease
                                                .canonical_cancel
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner()) =
                                                Some(target.lease.cancel.clone());
                                        }
                                        forward_alias(
                                            target,
                                            sender,
                                            (history, resource_history),
                                            requests,
                                            cancel,
                                        )
                                        .await;
                                        return;
                                    }
                                    Ok(None) => {}
                                    Err(error) => break Some(error),
                                }
                                if !recovered {
                                    if !options.read_only {
                                        match operations::recover(&directory, inventory.pool_id) {
                                            Ok(rows) => {
                                                for record in rows {
                                                    if record.batch_parent.is_none()
                                                        && !record.state.terminal()
                                                        && record.has_task()
                                                    {
                                                        start_worker(
                                                            MutationRequest::tracking(
                                                                record.clone(),
                                                            ),
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
                                    }
                                    let recovery = if directory.join("operations.sqlite3").exists()
                                    {
                                        operations::Journal::open(&directory).and_then(|journal| {
                                            if options.read_only {
                                                journal.resource_recover_exports(inventory.pool_id)
                                            } else {
                                                journal.resource_recover(inventory.pool_id)
                                            }
                                        })
                                    } else {
                                        Ok(Vec::new())
                                    };
                                    match recovery {
                                        Ok(rows) => {
                                            for record in rows {
                                                if record.batch_parent.is_none()
                                                    && !record.state.terminal()
                                                    && !record.action.spans_pools()
                                                    && !matches!(
                                                        record.action,
                                                        crate::ResourceAction::HostPower(_)
                                                            | crate::ResourceAction::RootPassword(
                                                                _
                                                            )
                                                    )
                                                    && resource_workers.len() < 8
                                                {
                                                    start_resource(
                                                        crate::resources::ResourceRequest::tracking(
                                                            record.clone(),
                                                        ),
                                                        &mut resource_workers,
                                                        &mut resource_counts,
                                                        &credentials,
                                                    );
                                                }
                                                merge_resource_record(
                                                    &mut resource_records,
                                                    record,
                                                );
                                            }
                                            publish_resource_history(
                                                &resource_history,
                                                &mut resource_records,
                                            );
                                        }
                                        Err(error) => journal_error = Some(error.to_string()),
                                    }
                                    recovered = true;
                                }
                                failure_stage = ConnectionStage::EventStream;
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
                    if !token.is_empty() {
                        protocol_failures = 0;
                    }
                    token = batch.token;
                    events.set(next_events(session.clone(), token.clone()));
                };
                // Revoke observations before workers finish retiring this generation.
                publish(
                    &sender,
                    generation,
                    ConnectionState::Reconnecting,
                    &last_inventory,
                    error.as_ref().map(ToString::to_string),
                    last_update,
                );
                active.cancel();
                for request in cross_pool_rechecks.into_values() {
                    request.reject(Error::Closed);
                }
                let cleanup = tokio::time::sleep(Duration::from_secs(3));
                tokio::pin!(cleanup);
                while !workers.is_empty() || !resource_workers.is_empty() || !metrics.is_empty() {
                    tokio::select! {
                        _=workers.join_next(), if !workers.is_empty()=>{},
                        _=resource_workers.join_next(), if !resource_workers.is_empty()=>{},
                        _=metrics.join_next(), if !metrics.is_empty()=>{},
                        Some(update)=operation_updates.recv()=>{match update {
                            OperationUpdate::Record(record)=>{merge_record(&mut records,*record);publish_history(&history,&mut records);},
                            OperationUpdate::Resource(record)=>{merge_resource_record(&mut resource_records,*record);publish_resource_history(&resource_history,&mut resource_records);},
                            OperationUpdate::RootCredentials(update)=>{let _=update.reply.send(Err(Error::Closed));},
                            OperationUpdate::Coordinator(_)=>{},
                        }},
                        _=&mut cleanup=>{workers.abort_all();resource_workers.abort_all();metrics.abort_all();break;},
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
        let protocol_fault = matches!(error, Error::Protocol(_) | Error::InventoryNumber { .. });
        if protocol_fault {
            protocol_failures = protocol_failures.saturating_add(1);
        }
        let retry = error.retryable() || (protocol_fault && protocol_failures <= 3);
        if let Error::Coordinator(endpoint) = &error {
            options = options.for_endpoint(endpoint.clone());
        } else if matches!(
            error,
            Error::Transport | Error::Timeout | Error::Http(502..=504)
        ) && expected_pool.is_some()
            && !options.fallback_endpoints.is_empty()
        {
            let next = options
                .fallback_endpoints
                .iter()
                .position(|endpoint| endpoint == &options.endpoint)
                .map_or(0, |position| {
                    (position + 1) % options.fallback_endpoints.len()
                });
            options = options.for_endpoint(options.fallback_endpoints[next].clone());
        }
        let previous = sender.borrow().clone();
        sender.send_replace(Arc::new(ConnectionSnapshot {
            permissions: crate::SessionPermissions::default(),
            generation,
            state: if retry {
                ConnectionState::Reconnecting
            } else {
                ConnectionState::Failed
            },
            inventory: last_inventory.clone(),
            error: Some(error.to_string()),
            failure: Some(ConnectionFailure {
                stage: failure_stage,
                certificate_endpoint: match &error {
                    Error::EndpointCertificate(endpoint) => Some(endpoint.clone()),
                    _ => None,
                },
                message: error.to_string(),
            }),
            updated_at: last_update,
            route: previous.route.clone(),
            endpoint: previous.endpoint.clone(),
            verified_members: previous.verified_members.clone(),
        }));
        if !retry {
            return;
        }
        tokio::select! {_=cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(backoff) + Duration::from_millis((generation.wrapping_mul(137)) % 251))=>{},}
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

async fn forward_alias(
    mut target: AliasTarget,
    sender: watch::Sender<Arc<ConnectionSnapshot>>,
    histories: HistorySenders,
    mut requests: mpsc::Receiver<Request>,
    cancel: CancellationToken,
) {
    let (history, resource_history) = histories;
    resource_history.send_replace(target.route.resource_history.borrow().clone());
    sender.send_replace(target.route.receiver.borrow().clone());
    history.send_replace(target.route.history.borrow().clone());
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            result = target.route.receiver.changed() => {
                sender.send_replace(target.route.receiver.borrow().clone());
                if result.is_err() { break; }
            }
            result = target.route.resource_history.changed() => {
                resource_history.send_replace(target.route.resource_history.borrow().clone());
                if result.is_err() {break;}
            }
            result = target.route.history.changed() => {
                history.send_replace(target.route.history.borrow().clone());
                if result.is_err() { break; }
            }
            request = requests.recv() => {
                let Some(request) = request else { break; };
                if cancel.is_cancelled() { break; }
                if let Err(error) = target.route.requests.try_send(request) {
                    match error.into_inner() {
                        Request::Submit(request) => request.reject(Error::Busy),
                        Request::ResourceSubmit(request) => request.reject(Error::Busy),
                        Request::Console(request) => request.fail(Error::Busy),
                        Request::Metrics(request) => request.fail(Error::Busy),
                        Request::StorageDiscovery(request) => request.fail(Error::Busy),
                        Request::SecurityDiscovery(request) => request.fail(Error::Busy),
                        Request::HostDataSources(request) => request.fail(Error::Busy),
                        Request::HostSupport(request) => request.fail(Error::Busy),
                        Request::ManagementBondReview(request) => request.fail(Error::Busy),
                        Request::ManagementIpv6Review(request)=>request.fail(Error::Busy),
                        Request::SnapshotPolicySupport(request) => request.fail(Error::Busy),
                        Request::GuestBoot(request) => request.fail(Error::Busy),
                        Request::SnapshotPreview(request) => request.fail(Error::Busy),
                        Request::JoinPeer(_,reply)=>{let _=reply.send(Err(Error::Busy));},
                        Request::Diagnose(request) => request.fail(Error::Busy),
                        Request::ManagementCandidate(request)=>{let _=request.reply.send(Err(Error::Busy));},
                        Request::RememberMembers(_) => {},
                        Request::Control(_, _) | Request::ResourceControl(_,_) => {
                            let mut snapshot = (*sender.borrow().clone()).clone();
                            snapshot.error = Some(Error::Busy.to_string());
                            sender.send_replace(Arc::new(snapshot));
                        }
                    }
                }
            }
        }
    }
    let previous = sender.borrow().clone();
    publish(
        &sender,
        previous.generation,
        ConnectionState::Disconnected,
        &previous.inventory,
        None,
        previous.updated_at,
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

/// The state of a session inside its event loop: synchronizing until the first inventory
/// of its generation has been applied, ready after it. A refusal reported from the loop must
/// carry this state: reporting Ready before that inventory would present the previous
/// generation's inventory as current.
fn session_state(token: &str) -> ConnectionState {
    if token.is_empty() {
        ConnectionState::Synchronizing
    } else {
        ConnectionState::Ready
    }
}

fn publish(
    sender: &watch::Sender<Arc<ConnectionSnapshot>>,
    generation: u64,
    state: ConnectionState,
    inventory: &Option<Arc<Inventory>>,
    error: Option<String>,
    updated_at: Option<SystemTime>,
) {
    let previous = sender.borrow().clone();
    sender.send_replace(Arc::new(ConnectionSnapshot {
        permissions: if generation == previous.generation {
            previous.permissions.clone()
        } else {
            crate::SessionPermissions::default()
        },
        generation,
        state,
        inventory: inventory.clone(),
        error,
        failure: None,
        route: previous.route.clone(),
        updated_at,
        endpoint: previous.endpoint.clone(),
        verified_members: previous.verified_members.clone(),
    }));
}

fn merge_resource_record(
    records: &mut BTreeMap<crate::OperationId, crate::ResourceOperationRecord>,
    record: crate::ResourceOperationRecord,
) {
    if records
        .get(&record.id)
        .is_none_or(|old| record.supersedes(old))
    {
        records.insert(record.id, record);
    }
}
fn publish_resource_history(
    history: &watch::Sender<Arc<Vec<crate::ResourceOperationRecord>>>,
    records: &mut BTreeMap<crate::OperationId, crate::ResourceOperationRecord>,
) {
    while records.len() > 1500 {
        if let Some(id) = records
            .iter()
            .filter(|(_, r)| r.state.terminal())
            .min_by_key(|(_, r)| r.created_at_ms)
            .map(|(id, _)| *id)
        {
            records.remove(&id);
        } else {
            break;
        }
    }
    let mut ordered: Vec<_> = records.values().cloned().collect();
    ordered.sort_by_key(|r| r.created_at_ms);
    history.send_replace(Arc::new(ordered));
}

pub(crate) struct JoinPeer {
    pub session: Arc<Session>,
    pub receiver: watch::Receiver<Arc<ConnectionSnapshot>>,
    pub cancel: CancellationToken,
}
#[derive(Clone)]
pub(crate) struct JoinPeerHandle {
    requests: mpsc::Sender<Request>,
    pub cancel: CancellationToken,
}
impl JoinPeerHandle {
    pub async fn get(&self, observed: crate::ObservedResource) -> Result<JoinPeer, Error> {
        let (send, recv) = tokio::sync::oneshot::channel();
        self.requests
            .try_send(Request::JoinPeer(observed, send))
            .map_err(|_| Error::Busy)?;
        tokio::select! {_=self.cancel.cancelled()=>Err(Error::Closed),result=tokio::time::timeout(Duration::from_secs(45),recv)=>result.map_err(|_|Error::Timeout)?.map_err(|_|Error::Closed)?}
    }
}
impl Connection {
    pub(crate) fn submit_pool_join(
        &self,
        id: crate::OperationId,
        selection: crate::PoolJoinSelection,
        destination: &Connection,
        credentials: Credentials,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only || destination.read_only {
            return Err(Error::ReadOnly);
        }
        if self.backend_scope != destination.backend_scope
            || !crate::pool_join::credentials_valid(&credentials)
        {
            return Err(Error::InvalidResourceCommand);
        }
        if self.latest().observe_resource(selection.source.target())? != selection.source
            || destination
                .latest()
                .observe_resource(selection.destination.target())?
                != selection.destination
        {
            return Err(Error::Stale);
        }
        let action = selection.action()?;
        let (handle, mut request) =
            crate::resources::channel(id, selection.source, action, self.runtime.clone());
        request.join = Some(crate::pool_join::Inputs {
            destination: JoinPeerHandle {
                requests: destination.requests.clone(),
                cancel: destination.cancel.clone(),
            },
            observation: selection.destination,
            credentials,
        });
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }

    pub(crate) fn submit_cross_pool(
        &self,
        id: crate::OperationId,
        selection: crate::CrossPoolVmSelection,
        destination: &Connection,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only || destination.read_only {
            return Err(Error::ReadOnly);
        }
        if self.backend_scope != destination.backend_scope
            || selection.source.id != selection.operation.vm
            || selection.destination.target
                != crate::ResourceTarget::Pool(selection.operation.destination_pool)
        {
            return Err(Error::InvalidMigration);
        }
        if self.latest().observe_vm(selection.source.id)? != selection.source
            || destination
                .latest()
                .observe_resource(selection.destination.target)?
                != selection.destination
        {
            return Err(Error::Stale);
        }
        selection.operation.validate(selection.source.pool)?;
        let observed = crate::ObservedResource {
            pool: selection.source.pool,
            generation: selection.source.generation,
            target: crate::ResourceTarget::Pool(selection.source.pool),
        };
        let (handle, mut request) = crate::resources::channel(
            id,
            observed,
            crate::ResourceAction::CrossPoolVm(selection.operation),
            self.runtime.clone(),
        );
        request.cross_pool = Some(crate::cross_pool::Inputs {
            destination: JoinPeerHandle {
                requests: destination.requests.clone(),
                cancel: destination.cancel.clone(),
            },
            observation: selection.destination,
        });
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }

    /// Submits a reviewed batch of cross-pool moves under the single move's own rules:
    /// both connections writable, current and owned by one backend. The destination
    /// connection is handed to the batch and to each of its children; nothing of it is
    /// stored.
    pub(crate) fn submit_cross_pool_batch(
        &self,
        id: crate::OperationId,
        selection: crate::cross_pool::CrossPoolBatchSelection,
        destination: &Connection,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only || destination.read_only {
            return Err(Error::ReadOnly);
        }
        let Some(destination_pool) = selection.batch.cross_pool_destination() else {
            return Err(Error::InvalidMigration);
        };
        if self.backend_scope != destination.backend_scope
            || selection.source.target != crate::ResourceTarget::Pool(selection.source.pool)
            || selection.destination.target != crate::ResourceTarget::Pool(destination_pool)
        {
            return Err(Error::InvalidMigration);
        }
        if self.latest().observe_resource(selection.source.target)? != selection.source
            || destination
                .latest()
                .observe_resource(selection.destination.target)?
                != selection.destination
        {
            return Err(Error::Stale);
        }
        selection.batch.validate(selection.source.target)?;
        let (handle, mut request) = crate::resources::channel(
            id,
            selection.source,
            crate::ResourceAction::VmWorkflowBatch(selection.batch),
            self.runtime.clone(),
        );
        request.cross_pool = Some(crate::cross_pool::Inputs {
            destination: JoinPeerHandle {
                requests: destination.requests.clone(),
                cancel: destination.cancel.clone(),
            },
            observation: selection.destination,
        });
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }

    /// Rechecks a single cross-pool transfer or a batch of cross-pool moves. A move that
    /// is a child of a batch is read through its batch only.
    pub(crate) fn recheck_cross_pool(
        &self,
        id: crate::OperationId,
        destination: &Connection,
    ) -> Result<crate::ResourceOperation, Error> {
        if self.read_only || destination.read_only {
            return Err(Error::ReadOnly);
        }
        if self.backend_scope != destination.backend_scope {
            return Err(Error::InvalidMigration);
        }
        let record = self
            .resource_history()
            .iter()
            .find(|r| r.id == id && !r.state.terminal())
            .cloned()
            .ok_or(Error::OperationConflict)?;
        let Some(destination_pool) = crate::cross_pool::destination(&record.action) else {
            return Err(Error::InvalidMigration);
        };
        if record.batch_parent.is_some() {
            return Err(Error::UnavailableAction);
        }
        let observed = self
            .latest()
            .observe_resource(crate::ResourceTarget::Pool(record.pool))?;
        let peer = destination
            .latest()
            .observe_resource(crate::ResourceTarget::Pool(destination_pool))?;
        let (handle, mut request) =
            crate::resources::channel(id, observed, record.action.clone(), self.runtime.clone());
        request.record = record;
        request.mark_recheck();
        request.cross_pool = Some(crate::cross_pool::Inputs {
            destination: JoinPeerHandle {
                requests: destination.requests.clone(),
                cancel: destination.cancel.clone(),
            },
            observation: peer,
        });
        self.requests
            .try_send(Request::ResourceSubmit(Box::new(request)))
            .map_err(|_| Error::Busy)?;
        Ok(handle)
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;
    #[test]
    fn publication_discards_permissions_on_generation_change_or_stale_state() {
        let permissions = crate::SessionPermissions::decode(
            &serde_json::json!({"is_local_superuser":false,"rbac_permissions":[]}),
        );
        let (sender, _receiver) = watch::channel(Arc::new(ConnectionSnapshot {
            permissions,
            generation: 4,
            state: ConnectionState::Reconnecting,
            inventory: None,
            error: None,
            failure: None,
            route: None,
            updated_at: None,
            endpoint: None,
            verified_members: vec![],
        }));
        // A stale connection must not expose its previous session's denial.
        assert!(matches!(
            sender.borrow().power_permission(crate::PowerAction::Start),
            crate::PermissionDecision::Unknown(_)
        ));
        publish(&sender, 5, ConnectionState::Connecting, &None, None, None);
        assert_eq!(
            sender.borrow().permissions,
            crate::SessionPermissions::default()
        );
    }
}
