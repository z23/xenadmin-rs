#[path = "disk_backend.rs"]
pub(crate) mod disk;
#[path = "disk_journal.rs"]
mod disk_journal;
#[path = "host_power_backend.rs"]
mod host_power_backend;
#[path = "host_power_journal.rs"]
mod host_power_journal;
#[path = "management_network_backend.rs"]
pub(crate) mod management;
#[path = "management_ipv6_backend.rs"]
pub(crate) mod management_ipv6;
#[path = "management_ipv6_journal.rs"]
mod management_ipv6_journal;
#[path = "root_password_journal.rs"]
mod root_password_journal;
#[path = "snapshot_capture_journal.rs"]
mod snapshot_capture_journal;
#[path = "storage_auth_backend.rs"]
mod storage_auth_backend;
#[path = "storage_auth_journal.rs"]
mod storage_auth_journal;
#[path = "storage_graph_backend.rs"]
pub(crate) mod storage_graph;
#[path = "storage_graph_journal.rs"]
mod storage_graph_journal;
#[path = "storage_maintenance_backend.rs"]
pub(crate) mod storage_maintenance;
#[path = "storage_maintenance_journal.rs"]
mod storage_maintenance_journal;
use crate::{
    BondId, Error, HostId, MessageId, NetworkId, NetworkOperation, OperationId, OperationState,
    OperationStep, PbdId, PifId, PoolId, SrId, StorageOperation, TaskId, VlanId,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::SystemTime};
use tokio::sync::watch;

#[path = "crashdump_operations.rs"]
pub(crate) mod crashdump;
#[path = "host_operations.rs"]
pub(crate) mod host;
#[path = "resource_journal.rs"]
pub(crate) mod journal;
#[path = "network_operations.rs"]
mod network;
#[path = "network_fabric_backend.rs"]
pub(crate) mod network_fabric;
#[path = "network_service_backend.rs"]
mod network_service;
#[path = "snapshot_policy_operations.rs"]
pub(crate) mod snapshot_policy;
#[path = "network_topology_backend.rs"]
pub(crate) mod topology;
#[path = "resource_worker.rs"]
mod worker;
#[path = "resource_xapi.rs"]
mod xapi;
pub(crate) use worker::ResourceRequest;
#[path = "alarm_operations.rs"]
mod alarms;
#[path = "appliance_operations.rs"]
mod appliance;
#[path = "organization_operations.rs"]
mod organization;
#[path = "pool_operations.rs"]
mod pool;
#[path = "security_operations.rs"]
mod security;
#[path = "storage_operations.rs"]
mod storage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceTarget {
    Disk(crate::VdiId),
    Vm(crate::VmId),
    SnapshotPolicy(crate::SnapshotPolicyId),
    Pool(PoolId),
    Host(HostId),
    Storage(SrId),
    Network(NetworkId),
    Pif(PifId),
    Vlan(VlanId),
    Bond(BondId),
    Tunnel(crate::TunnelId),
    Sriov(crate::NetworkSriovId),
    Task(TaskId),
    Message(MessageId),
    Pbd(PbdId),
    Appliance(crate::ApplianceId),
}
impl ResourceTarget {
    pub(crate) fn class(self) -> &'static str {
        match self {
            Self::Disk(_) => "VDI",
            Self::Vm(_) => "VM",
            Self::SnapshotPolicy(_) => "VMSS",
            Self::Pool(_) => "pool",
            Self::Host(_) => "host",
            Self::Storage(_) => "SR",
            Self::Network(_) => "network",
            Self::Pif(_) => "PIF",
            Self::Vlan(_) => "VLAN",
            Self::Bond(_) => "Bond",
            Self::Tunnel(_) => "Tunnel",
            Self::Sriov(_) => "Network_sriov",
            Self::Task(_) => "task",
            Self::Message(_) => "message",
            Self::Pbd(_) => "PBD",
            Self::Appliance(_) => "VM_appliance",
        }
    }
    pub fn uuid(self) -> uuid::Uuid {
        match self {
            Self::Disk(id) => id.uuid(),
            Self::Vm(id) => id.uuid(),
            Self::SnapshotPolicy(id) => id.uuid(),
            Self::Pool(id) => id.uuid(),
            Self::Host(id) => id.uuid(),
            Self::Storage(id) => id.uuid(),
            Self::Network(id) => id.uuid(),
            Self::Pif(id) => id.uuid(),
            Self::Vlan(id) => id.uuid(),
            Self::Bond(id) => id.uuid(),
            Self::Tunnel(id) => id.uuid(),
            Self::Sriov(id) => id.uuid(),
            Self::Task(id) => id.uuid(),
            Self::Message(id) => id.uuid(),
            Self::Pbd(id) => id.uuid(),
            Self::Appliance(id) => id.uuid(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedResource {
    pub(crate) pool: PoolId,
    pub(crate) generation: u64,
    pub(crate) target: ResourceTarget,
}
impl ObservedResource {
    pub fn target(self) -> ResourceTarget {
        self.target
    }
}

pub enum ResourceCommand {
    StorageGraph {
        target: ObservedResource,
        operation: crate::StorageGraph,
    },
    StorageMaintenance {
        target: ObservedResource,
        operation: crate::StorageMaintenance,
    },
    Alarm {
        target: ObservedResource,
        operation: crate::AlarmOperation,
    },
    DeleteHostCrashdump {
        host: ObservedResource,
        dump: crate::HostCrashdump,
    },
    Disk {
        target: ObservedResource,
        operation: crate::DiskOperation,
    },
    ManagementIpv6Bond {
        host: ObservedResource,
        operation: Box<crate::ManagementIpv6Bond>,
    },
    ManagementNetwork {
        host: ObservedResource,
        operation: Box<crate::ManagementNetworkOperation>,
    },
    HostPower {
        host: ObservedResource,
        operation: crate::HostPowerOperation,
    },
    NetworkTopology {
        target: ObservedResource,
        operation: crate::NetworkTopologyOperation,
    },
    PropertiesBatch {
        pool: ObservedResource,
        selection: crate::VmPropertiesBatch,
    },
    VmWorkflowBatch {
        pool: ObservedResource,
        selection: crate::VmWorkflowBatch,
    },
    PowerBatch {
        pool: ObservedResource,
        selection: crate::VmPowerBatch,
    },
    CleanupStorageCredentials {
        pool: ObservedResource,
        operation: OperationId,
        secrets: Vec<crate::SecretId>,
    },
    SnapshotPolicy {
        pool: ObservedResource,
        operation: crate::SnapshotPolicyOperation,
    },
    Appliance {
        pool: ObservedResource,
        operation: crate::ApplianceOperation,
    },
    PoolSensitive {
        target: ObservedResource,
        change: crate::PoolSensitiveChange,
    },
    Pool {
        target: ObservedResource,
        operation: crate::PoolOperation,
    },
    Host {
        target: ObservedResource,
        operation: crate::HostOperation,
    },
    Organization {
        pool: ObservedResource,
        patch: crate::OrganizationPatch,
    },
    Network {
        target: ObservedResource,
        operation: NetworkOperation,
    },
    CancelTask {
        task: ObservedResource,
    },
    DismissMessage {
        message: ObservedResource,
        expected_digest: [u8; 32],
    },
    Storage {
        target: ObservedResource,
        operation: StorageOperation,
    },
    RenameMetadata {
        target: ObservedResource,
        expected_name: String,
        expected_description: String,
        name: String,
        description: String,
    },
    CreateInternalNetwork {
        pool: ObservedResource,
        name: String,
        description: String,
        mtu: u64,
    },
    ConfigureInternalNetwork {
        network: ObservedResource,
        expected_mtu: u64,
        mtu: u64,
    },
    DeleteInternalNetwork {
        network: ObservedResource,
        expected_name: String,
    },
    ScanStorage {
        sr: ObservedResource,
    },
    SetHostEnabled {
        host: ObservedResource,
        expected_enabled: bool,
        enabled: bool,
    },
    SetDefaultStorage {
        pool: ObservedResource,
        sr: SrId,
        expected_default: Option<SrId>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceAction {
    SnapshotPreviewUpload(crate::SnapshotPreviewUpload),
    StorageGraph(crate::StorageGraph),
    StorageMaintenance(crate::StorageMaintenance),
    Alarm(crate::AlarmOperation),
    DeleteHostCrashdump(crate::HostCrashdump),
    Disk(crate::DiskOperation),
    RootPassword(crate::PoolRootPasswordChange),
    ManagementIpv6Bond(Box<crate::ManagementIpv6Bond>),
    ManagementNetwork(Box<crate::ManagementNetworkOperation>),
    HostPower(crate::HostPowerOperation),
    NetworkTopology(crate::NetworkTopologyOperation),
    VmWorkflowBatch(crate::VmWorkflowBatch),
    PowerBatch(crate::VmPowerBatch),
    PropertiesBatch(crate::VmPropertiesBatch),
    CrossPoolVm(crate::CrossPoolVmOperation),
    AuthenticatedStorage {
        operation: StorageOperation,
        authentication: crate::StorageAuthentication,
    },
    CleanupStorageCredentials {
        operation: OperationId,
        secrets: Vec<crate::SecretId>,
    },
    SnapshotPolicy(crate::SnapshotPolicyOperation),
    Appliance(crate::ApplianceOperation),
    Pool(crate::PoolOperation),
    Host(crate::HostOperation),
    Transfer(crate::TransferAction),
    CloudInit(crate::CloudInitConfiguration),
    Organization(crate::OrganizationPatch),
    Network(NetworkOperation),
    CancelTask,
    DismissMessage {
        expected_digest: [u8; 32],
    },
    Storage(StorageOperation),
    RenameMetadata {
        expected_name: String,
        expected_description: String,
        name: String,
        description: String,
    },
    CreateInternalNetwork {
        name: String,
        description: String,
        mtu: u64,
    },
    ConfigureInternalNetwork {
        expected_mtu: u64,
        mtu: u64,
    },
    DeleteInternalNetwork {
        expected_name: String,
    },
    ScanStorage,
    SetHostEnabled {
        expected_enabled: bool,
        enabled: bool,
    },
    SetDefaultStorage {
        sr: SrId,
        expected_default: Option<SrId>,
    },
}
impl ResourceAction {
    pub fn permission_requirements(&self, target: ResourceTarget) -> crate::PermissionRequirements {
        use crate::PermissionRequirements as Required;
        match self {
            Self::VmWorkflowBatch(selection) => selection.permission_requirements(),
            Self::Appliance(op) => op.permission_requirements(),
            Self::SnapshotPolicy(op) => op.permission_requirements(),
            Self::Organization(patch) => patch.permission_requirements(),
            Self::RootPassword(_) => Required::writes(["session.change_password"]),
            Self::CleanupStorageCredentials { .. } => Required::writes(["secret.destroy"]),
            Self::HostPower(op) => op.permission_requirements(),
            Self::PowerBatch(selection) => {
                Required::writes([format!("VM.{}", selection.action.wire())])
            }
            Self::NetworkTopology(op) => op.permission_requirements(),
            Self::CloudInit(op) => op.permission_requirements(),
            Self::Transfer(action) => action.permission_requirements(),
            Self::Alarm(op) => Required::writes([match op {
                crate::AlarmOperation::Refresh { .. } => "host.call_plugin".into(),
                _ => format!("{}.set_other_config", target.class()),
            }]),
            Self::DeleteHostCrashdump(_) => Required::writes(["host_crashdump.destroy"]),
            Self::StorageGraph(op) => op.permission_requirements(),
            Self::StorageMaintenance(op) => op.permission_requirements(),
            Self::Disk(op) => op.permission_requirements(),
            Self::PropertiesBatch(selection) => selection.permission_requirements(),
            Self::SnapshotPreviewUpload(_) => {
                Required::writes(["VM.create_new_blob"]).with_host_dependent_steps()
            }
            Self::Storage(op) => op.permission_requirements(),
            Self::AuthenticatedStorage { operation, .. } => {
                operation.authenticated_permission_requirements()
            }
            Self::Network(op) => op.permission_requirements(),
            Self::ManagementIpv6Bond(op)
                if op.purpose == crate::ManagementIpv6Next::LeavePartial =>
            {
                Required::writes(std::iter::empty::<&str>())
            }
            Self::ManagementIpv6Bond(op) => Required::writes([
                "host.management_reconfigure",
                "PIF.reconfigure_ipv6",
                if op.operation.creates() {
                    "Bond.create"
                } else {
                    "Bond.destroy"
                },
            ]),
            Self::ManagementNetwork(op) => Required::writes([match op.as_ref() {
                crate::ManagementNetworkOperation::Bond { operation, .. } => {
                    if operation.creates() {
                        "Bond.create"
                    } else {
                        "Bond.destroy"
                    }
                }
                crate::ManagementNetworkOperation::Move { .. } => "host.management_reconfigure",
                crate::ManagementNetworkOperation::ConfigureIpv4 { .. } => "PIF.reconfigure_ip",
                crate::ManagementNetworkOperation::ConfigureIpv6 { .. } => "PIF.reconfigure_ipv6",
                crate::ManagementNetworkOperation::ConfigureDynamic { mode, .. } => match mode {
                    crate::ManagementDynamicMode::Ipv4Dhcp => "PIF.reconfigure_ip",
                    _ => "PIF.reconfigure_ipv6",
                },
            }]),
            Self::Host(op) => op.permission_requirements(),
            Self::Pool(op) => op.permission_requirements(),
            Self::CancelTask => Required::writes(["task.cancel"]),
            Self::DismissMessage { .. } => Required::writes(["message.destroy"]),
            Self::RenameMetadata {
                expected_name,
                expected_description,
                name,
                description,
            } => {
                let mut methods = Vec::new();
                if expected_name != name {
                    methods.push(format!("{}.set_name_label", target.class()));
                }
                if expected_description != description {
                    methods.push(format!("{}.set_name_description", target.class()));
                }
                Required::writes(methods)
            }
            Self::CreateInternalNetwork { .. } => Required::writes(["network.create"]),
            Self::ConfigureInternalNetwork { .. } => Required::writes(["network.set_MTU"]),
            Self::DeleteInternalNetwork { .. } => Required::writes(["network.destroy"]),
            Self::ScanStorage => Required::writes(["SR.scan"]),
            Self::SetHostEnabled { enabled, .. } => Required::writes([if *enabled {
                "host.enable"
            } else {
                "host.disable"
            }]),
            Self::SetDefaultStorage { .. } => Required::writes(["pool.set_default_SR"]),
            _ => Required::writes(Vec::<String>::new()).with_host_dependent_steps(),
        }
    }

    pub fn read_only_export(&self) -> bool {
        matches!(self, Self::Transfer(action) if action.is_export())
            || matches!(self, Self::VmWorkflowBatch(selection) if selection.is_export())
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Alarm(op) => op.label(),
            Self::DeleteHostCrashdump(_) => "Delete retained host crashdump",
            Self::StorageGraph(op) => op.label(),
            Self::StorageMaintenance(op) => op.label(),
            Self::Disk(op) => op.label(),
            Self::SnapshotPreviewUpload(_) => "Attach snapshot preview",
            Self::RootPassword(_) => "Change pool root password",
            Self::HostPower(operation) => operation.label(),
            Self::VmWorkflowBatch(selection) => {
                match selection.items.first().map(|i| &i.workflow) {
                    Some(crate::VmWorkflow::Move(_)) => "Move selected VMs",
                    Some(crate::VmWorkflow::Export { .. }) => "Export selected VMs",
                    _ => "Delete selected VMs",
                }
            }
            Self::PowerBatch(_) => "Power actions on selected VMs",
            Self::PropertiesBatch(_) => "Edit selected VM properties",
            Self::CrossPoolVm(operation) => operation.mode.label(),
            Self::AuthenticatedStorage { operation, .. } => operation.label(),
            Self::CleanupStorageCredentials { .. } => "Clean up retained storage credentials",
            Self::SnapshotPolicy(operation) => operation.label(),
            Self::Appliance(operation) => operation.label(),
            Self::Pool(operation) => operation.label(),
            Self::Host(operation) => operation.label(),
            Self::Organization(_) => "Edit organization metadata",
            Self::ManagementIpv6Bond(operation) => operation.label(),
            Self::ManagementNetwork(operation) => operation.label(),
            Self::NetworkTopology(operation) => operation.label(),
            Self::Network(operation) => operation.label(),
            Self::Transfer(action) => action.label(),
            Self::CloudInit(_) => "Prepare cloud-init drive",
            Self::CancelTask => "Request task cancellation",
            Self::DismissMessage { .. } => "Dismiss server message",
            Self::Storage(operation) => operation.label(),
            Self::RenameMetadata { .. } => "Edit name and description",
            Self::CreateInternalNetwork { .. } => "Create internal network",
            Self::ConfigureInternalNetwork { .. } => "Set internal network MTU",
            Self::DeleteInternalNetwork { .. } => "Delete internal network",
            Self::ScanStorage => "Scan storage",
            Self::SetHostEnabled { enabled: true, .. } => "Enable host",
            Self::SetHostEnabled { enabled: false, .. } => "Disable host",
            Self::SetDefaultStorage { .. } => "Set default storage",
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let name = |value: &str| {
            !value.trim().is_empty()
                && value.chars().count() <= 255
                && !value.chars().any(char::is_control)
        };
        let description = |value: &str| value.len() <= 16_384 && !value.contains('\0');
        let mtu = |value: u64| (1..=65_535).contains(&value);
        let valid = match self {
            Self::SnapshotPreviewUpload(upload) => upload.valid(target),
            Self::Alarm(op) => return op.validate(target),
            Self::DeleteHostCrashdump(dump) => return crashdump::validate_command(target, dump),
            Self::StorageGraph(op) => return op.validate(target),
            Self::StorageMaintenance(op) => return op.validate(target),
            Self::Disk(op) => return op.validate(target),
            Self::RootPassword(change) => {
                matches!(target, ResourceTarget::Pool(_)) && change.valid()
            }
            Self::HostPower(operation) => {
                matches!(target, ResourceTarget::Host(_)) && operation.valid()
            }
            Self::VmWorkflowBatch(selection) => return selection.validate(target),
            Self::PowerBatch(selection) => return selection.validate(target),
            Self::PropertiesBatch(selection) => return selection.validate(target),
            Self::CrossPoolVm(operation) => {
                let ResourceTarget::Pool(pool) = target else {
                    return Err(Error::InvalidResourceCommand);
                };
                return operation.validate(pool);
            }
            Self::AuthenticatedStorage {
                operation,
                authentication,
            } => {
                operation.validate(target)?;
                authentication.valid_for(operation)
            }
            Self::CleanupStorageCredentials { secrets, .. } => {
                matches!(target, ResourceTarget::Pool(_))
                    && !secrets.is_empty()
                    && secrets.len() <= 2
                    && secrets
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == secrets.len()
            }
            Self::SnapshotPolicy(operation) => return operation.validate(target),
            Self::Appliance(operation) => return operation.validate(target),
            Self::Pool(operation) => return operation.validate(target),
            Self::Host(operation) => return operation.validate(target),
            Self::Organization(patch) => {
                if !matches!(target, ResourceTarget::Pool(_)) {
                    return Err(Error::InvalidResourceCommand);
                }
                return patch.validate();
            }
            Self::ManagementIpv6Bond(operation) => return operation.validate(target),
            Self::ManagementNetwork(operation) => return operation.validate(target),
            Self::NetworkTopology(operation) => return operation.validate(target),
            Self::Network(operation) => return operation.validate(target),
            Self::Transfer(_) => matches!(target, ResourceTarget::Pool(_)),
            Self::CloudInit(config) => matches!(target, ResourceTarget::Vm(_)) && config.valid(),
            Self::CancelTask => matches!(target, ResourceTarget::Task(_)),
            Self::DismissMessage { .. } => matches!(target, ResourceTarget::Message(_)),
            Self::Storage(operation) => return operation.validate(target),
            Self::RenameMetadata {
                expected_name,
                expected_description,
                name: value,
                description: desc,
            } => {
                matches!(
                    target,
                    ResourceTarget::Pool(_)
                        | ResourceTarget::Host(_)
                        | ResourceTarget::Storage(_)
                        | ResourceTarget::Network(_)
                ) && name(value)
                    && description(desc)
                    && description(expected_name)
                    && description(expected_description)
                    && (value != expected_name || desc != expected_description)
            }
            Self::CreateInternalNetwork {
                name: value,
                description: desc,
                mtu: value_mtu,
            } => {
                matches!(target, ResourceTarget::Pool(_))
                    && name(value)
                    && description(desc)
                    && mtu(*value_mtu)
            }
            Self::ConfigureInternalNetwork {
                expected_mtu,
                mtu: value,
            } => {
                matches!(target, ResourceTarget::Network(_))
                    && mtu(*expected_mtu)
                    && mtu(*value)
                    && expected_mtu != value
            }
            Self::DeleteInternalNetwork { expected_name } => {
                matches!(target, ResourceTarget::Network(_)) && description(expected_name)
            }
            Self::ScanStorage => matches!(target, ResourceTarget::Storage(_)),
            Self::SetHostEnabled {
                expected_enabled,
                enabled,
            } => matches!(target, ResourceTarget::Host(_)) && expected_enabled != enabled,
            Self::SetDefaultStorage {
                sr,
                expected_default,
            } => matches!(target, ResourceTarget::Pool(_)) && Some(*sr) != *expected_default,
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
}
impl ResourceCommand {
    pub(crate) fn submission(
        self,
    ) -> (
        ObservedResource,
        ResourceAction,
        Option<crate::pool_security::SensitiveInputs>,
    ) {
        match self {
            Self::PoolSensitive { target, change } => {
                let (target, action, inputs) = change.split(target);
                (target, action, Some(inputs))
            }
            normal => {
                let (target, action) = normal.normalize();
                (target, action, None)
            }
        }
    }
    fn normalize(self) -> (ObservedResource, ResourceAction) {
        match self {
            Self::Alarm { target, operation } => (target, ResourceAction::Alarm(operation)),
            Self::DeleteHostCrashdump { host, dump } => {
                (host, ResourceAction::DeleteHostCrashdump(dump))
            }
            Self::StorageGraph { target, operation } => {
                (target, ResourceAction::StorageGraph(operation))
            }
            Self::StorageMaintenance { target, operation } => {
                (target, ResourceAction::StorageMaintenance(operation))
            }
            Self::Disk { target, operation } => (target, ResourceAction::Disk(operation)),
            Self::HostPower { host, operation } => (host, ResourceAction::HostPower(operation)),
            Self::VmWorkflowBatch { pool, selection } => {
                (pool, ResourceAction::VmWorkflowBatch(selection))
            }
            Self::PowerBatch { pool, selection } => (pool, ResourceAction::PowerBatch(selection)),
            Self::PropertiesBatch { pool, selection } => {
                (pool, ResourceAction::PropertiesBatch(selection))
            }
            Self::CleanupStorageCredentials {
                pool,
                operation,
                secrets,
            } => (
                pool,
                ResourceAction::CleanupStorageCredentials { operation, secrets },
            ),
            Self::PoolSensitive { .. } => {
                unreachable!("sensitive commands split before normalization")
            }
            Self::SnapshotPolicy { pool, operation } => {
                (pool, ResourceAction::SnapshotPolicy(operation))
            }
            Self::Appliance { pool, operation } => (pool, ResourceAction::Appliance(operation)),
            Self::Pool { target, operation } => (target, ResourceAction::Pool(operation)),
            Self::Host { target, operation } => (target, ResourceAction::Host(operation)),
            Self::Organization { pool, patch } => (pool, ResourceAction::Organization(patch)),
            Self::ManagementIpv6Bond { host, operation } => {
                (host, ResourceAction::ManagementIpv6Bond(operation))
            }
            Self::ManagementNetwork { host, operation } => {
                (host, ResourceAction::ManagementNetwork(operation))
            }
            Self::NetworkTopology { target, operation } => {
                (target, ResourceAction::NetworkTopology(operation))
            }
            Self::Network { target, operation } => (target, ResourceAction::Network(operation)),
            Self::CancelTask { task } => (task, ResourceAction::CancelTask),
            Self::DismissMessage {
                message,
                expected_digest,
            } => (message, ResourceAction::DismissMessage { expected_digest }),
            Self::Storage { target, operation } => (target, ResourceAction::Storage(operation)),
            Self::RenameMetadata {
                target,
                expected_name,
                expected_description,
                name,
                description,
            } => (
                target,
                ResourceAction::RenameMetadata {
                    expected_name,
                    expected_description,
                    name,
                    description,
                },
            ),
            Self::CreateInternalNetwork {
                pool,
                name,
                description,
                mtu,
            } => (
                pool,
                ResourceAction::CreateInternalNetwork {
                    name,
                    description,
                    mtu,
                },
            ),
            Self::ConfigureInternalNetwork {
                network,
                expected_mtu,
                mtu,
            } => (
                network,
                ResourceAction::ConfigureInternalNetwork { expected_mtu, mtu },
            ),
            Self::DeleteInternalNetwork {
                network,
                expected_name,
            } => (
                network,
                ResourceAction::DeleteInternalNetwork { expected_name },
            ),
            Self::ScanStorage { sr } => (sr, ResourceAction::ScanStorage),
            Self::SetHostEnabled {
                host,
                expected_enabled,
                enabled,
            } => (
                host,
                ResourceAction::SetHostEnabled {
                    expected_enabled,
                    enabled,
                },
            ),
            Self::SetDefaultStorage {
                pool,
                sr,
                expected_default,
            } => (
                pool,
                ResourceAction::SetDefaultStorage {
                    sr,
                    expected_default,
                },
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceOperationRecord {
    #[serde(default)]
    pub batch_parent: Option<OperationId>,
    #[serde(default)]
    pub management_ipv6_bond: Option<crate::ManagementIpv6BondEvidence>,
    #[serde(skip)]
    pub(crate) management_ipv6_bond_plan: Option<Box<crate::management_ipv6_bond::Plan>>,
    #[serde(skip)]
    pub(crate) storage_graph_plan: Option<Box<crate::storage_graph::Plan>>,
    #[serde(default)]
    pub storage_graph: Vec<crate::StorageGraphResult>,
    #[serde(skip)]
    pub(crate) workflow_batch_plan: Option<Box<crate::vm_workflow_batch::Plan>>,
    #[serde(default)]
    pub workflow_batch: Vec<crate::VmWorkflowResult>,
    #[serde(skip)]
    pub(crate) storage_maintenance_plan: Option<Box<crate::storage_maintenance::Plan>>,
    #[serde(skip)]
    pub(crate) crashdump_deletion: Option<Box<crashdump::Plan>>,
    #[serde(skip)]
    pub(crate) snapshot_preview_plan: Option<Box<crate::snapshot_capture::Plan>>,
    #[serde(default)]
    pub(crate) alarm_refresh_receipts: Vec<HostId>,
    #[serde(skip)]
    pub(crate) disk_plan: Option<Box<crate::disk_administration::Plan>>,
    #[serde(default)]
    pub disk_size: Option<u64>,
    #[serde(default)]
    pub management_network: Option<crate::ManagementNetworkEvidence>,
    #[serde(skip)]
    pub(crate) management_network_plan: Option<Box<crate::management_network::Plan>>,
    #[serde(default)]
    pub topology_changes: crate::NetworkTopologyChanges,
    #[serde(skip)]
    pub(crate) network_topology_plan: Option<Box<crate::network_topology::Plan>>,
    #[serde(default)]
    pub(crate) interrupted: bool,
    pub id: OperationId,
    pub pool: PoolId,
    pub target: ResourceTarget,
    pub action: ResourceAction,
    pub state: OperationState,
    pub progress: Option<f64>,
    pub detail: String,
    pub created_at_ms: u64,
    pub can_cancel: bool,
    pub cancel_requested: bool,
    pub steps: Vec<OperationStep>,
    pub created: Option<ResourceTarget>,
    #[serde(default)]
    pub transfer: Option<crate::TransferProgress>,
    #[serde(default)]
    pub transfer_created: crate::TransferCreated,
    #[serde(skip)]
    pub(crate) transfer_plan: Option<Box<crate::transfer::Plan>>,
    #[serde(skip)]
    pub(crate) cloud_init_plan: Option<Box<crate::cloud_init::Plan>>,
    #[serde(skip)]
    pub local_error: Option<String>,
    #[serde(skip)]
    pub(crate) revision: u64,
    #[serde(skip)]
    pub(crate) task: Option<String>,
    #[serde(skip)]
    pub(crate) result_reference: Option<String>,
    #[serde(default)]
    pub(crate) cancel_dispatched: bool,
    #[serde(default)]
    pub(crate) baseline_networks: Option<Vec<NetworkId>>,
    #[serde(skip)]
    pub(crate) network_fabric_plan: Option<Box<network_fabric::Plan>>,
    #[serde(default)]
    pub network_fabric_created: Option<crate::NetworkFabricEvidence>,
    #[serde(default)]
    pub(crate) network_baseline: Option<crate::network_types::NetworkCreationBaseline>,
    #[serde(default)]
    pub(crate) storage_baseline: Option<crate::storage::StorageBaseline>,
    #[serde(default)]
    pub(crate) storage_trim_succeeded: bool,
    #[serde(default)]
    pub(crate) pool_completed: bool,
    #[serde(default)]
    pub(crate) appliance_completed: bool,
    #[serde(default)]
    pub(crate) appliance_baseline: Option<Vec<crate::ApplianceId>>,
    #[serde(skip)]
    pub(crate) host_plan: Option<Box<host::Plan>>,
    #[serde(skip)]
    pub(crate) root_password_plan: Option<Box<crate::root_password::Plan>>,
    #[serde(default)]
    pub root_password: Option<crate::RootPasswordEvidence>,
    #[serde(skip)]
    pub(crate) host_power_plan: Option<Box<crate::host_power::Plan>>,
    #[serde(default)]
    pub host_power_secret: Option<crate::HostPowerSecretEvidence>,
    #[serde(skip)]
    pub(crate) power_batch_plan: Option<Box<crate::power_batch::Plan>>,
    #[serde(skip)]
    pub(crate) properties_batch_plan: Option<Box<crate::properties_batch::Plan>>,
    #[serde(skip)]
    pub(crate) storage_auth_plan: Option<Box<crate::storage_auth::Plan>>,
    #[serde(skip)]
    pub(crate) storage_secret_cleanup: Option<Box<crate::storage_auth::CleanupPlan>>,
    #[serde(default)]
    pub storage_secrets: Vec<crate::StorageSecretEvidence>,
    #[serde(skip)]
    pub(crate) snapshot_policy_evidence: Option<Box<snapshot_policy::Evidence>>,
    #[serde(default)]
    pub(crate) subject_baseline: Option<Vec<crate::SubjectId>>,
    #[serde(default)]
    pub(crate) join_baseline: Option<Vec<HostId>>,
    #[serde(skip)]
    pub(crate) cross_pool_plan: Option<Box<crate::cross_pool_backend::Plan>>,
}
impl ResourceOperationRecord {
    pub fn was_interrupted(&self) -> bool {
        self.interrupted
    }
    pub fn applies_to_pool(&self, pool: PoolId) -> bool {
        self.pool == pool || self.peer_pool() == Some(pool)
    }
    pub fn peer_pool(&self) -> Option<PoolId> {
        crate::pool_join::destination(&self.action)
            .or_else(|| crate::cross_pool::destination(&self.action))
    }
    pub fn recovery_pool(&self) -> PoolId {
        crate::pool_join::destination(&self.action).unwrap_or(self.pool)
    }
    pub(crate) fn pool_scopes(&self) -> Vec<PoolId> {
        let mut pools = vec![self.pool];
        if let Some(pool) = self.peer_pool()
            && pool != self.pool
        {
            pools.push(pool);
        }
        pools
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn can_recheck(&self) -> bool {
        self.batch_parent.is_none() && !self.state.terminal()
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
pub struct ResourceOperation {
    receiver: watch::Receiver<Arc<ResourceOperationRecord>>,
    runtime: tokio::runtime::Handle,
}
impl ResourceOperation {
    pub fn latest(&self) -> Arc<ResourceOperationRecord> {
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
pub(crate) fn channel(
    id: OperationId,
    target: ObservedResource,
    action: ResourceAction,
    runtime: tokio::runtime::Handle,
) -> (ResourceOperation, ResourceRequest) {
    let mut record = ResourceOperationRecord {
        batch_parent: None,
        workflow_batch_plan: None,
        workflow_batch: Vec::new(),
        snapshot_preview_plan: None,
        alarm_refresh_receipts: Vec::new(),
        crashdump_deletion: None,
        storage_graph_plan: None,
        storage_graph: Vec::new(),
        storage_maintenance_plan: None,
        disk_plan: None,
        disk_size: None,
        management_ipv6_bond: None,
        management_ipv6_bond_plan: None,
        management_network: None,
        management_network_plan: None,
        topology_changes: Default::default(),
        network_topology_plan: None,
        interrupted: false,
        id,
        pool: target.pool,
        target: target.target,
        action,
        state: OperationState::Preparing,
        progress: None,
        detail: "Waiting for admission".into(),
        created_at_ms: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        can_cancel: false,
        cancel_requested: false,
        steps: Vec::new(),
        created: None,
        transfer: None,
        transfer_created: Default::default(),
        transfer_plan: None,
        cloud_init_plan: None,
        local_error: None,
        revision: 0,
        task: None,
        result_reference: None,
        cancel_dispatched: false,
        baseline_networks: None,
        network_fabric_plan: None,
        network_fabric_created: None,
        network_baseline: None,
        storage_baseline: None,
        storage_trim_succeeded: false,
        pool_completed: false,
        appliance_completed: false,
        appliance_baseline: None,
        host_plan: None,
        root_password_plan: None,
        root_password: None,
        host_power_plan: None,
        host_power_secret: None,
        power_batch_plan: None,
        properties_batch_plan: None,
        storage_auth_plan: None,
        storage_secret_cleanup: None,
        storage_secrets: Vec::new(),
        snapshot_policy_evidence: None,
        subject_baseline: None,
        join_baseline: None,
        cross_pool_plan: None,
    };
    if let ResourceAction::VmWorkflowBatch(selection) = &record.action {
        let plan = crate::vm_workflow_batch::Plan::new(selection.clone());
        record.workflow_batch = plan.results();
        record.steps = selection
            .items
            .iter()
            .map(|item| OperationStep {
                label: format!("{} {}", item.workflow.label(), item.vm),
                state: crate::StepState::Pending,
                detail: "Not started".into(),
            })
            .collect();
        record.workflow_batch_plan = Some(Box::new(plan));
    }
    let (sender, receiver) = watch::channel(Arc::new(record.clone()));
    (
        ResourceOperation { receiver, runtime },
        ResourceRequest::new(target.generation, record, sender),
    )
}

pub(crate) fn transfer_channel(
    id: OperationId,
    target: ObservedResource,
    request: crate::TransferRequest,
    runtime: tokio::runtime::Handle,
) -> (ResourceOperation, ResourceRequest) {
    let (operation, mut pending) = channel(
        id,
        target,
        ResourceAction::Transfer(request.action),
        runtime,
    );
    pending.record.transfer = Some(Default::default());
    pending.record.transfer_plan = Some(Box::new(crate::transfer::Plan::new(request.file)));
    (operation, pending)
}

#[cfg(test)]
#[test]
fn operation_record_keeps_mutually_exclusive_workflows_off_the_stack() {
    // Nested batch/child dispatch decodes several records on the default worker stack.
    // Private plans belong on the heap even when most workflows are absent.
    let size = std::mem::size_of::<ResourceOperationRecord>();
    eprintln!("Resource operation record: {size} bytes");
    assert!(size <= 4096);
}
