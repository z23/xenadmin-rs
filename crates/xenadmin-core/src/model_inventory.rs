use super::*;

identity!(SrId);
identity!(NetworkId);
identity!(VdiId);
identity!(VbdId);
identity!(VifId);
identity!(PifId);
identity!(PbdId);
identity!(BondId);
identity!(VlanId);
identity!(GuestMetricsId);
identity!(VmMetricsId);
identity!(HostMetricsId);
identity!(TaskId);
identity!(MessageId);
identity!(ApplianceId);
identity!(SnapshotPolicyId);
identity!(StorageManagerId);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassAvailability {
    #[default]
    Unknown,
    Available,
    Unsupported,
    PermissionDenied,
    InvalidData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningOperation {
    pub task: Option<TaskId>,
    pub operation: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionAvailability {
    Available,
    Busy { operations: Vec<RunningOperation> },
    Blocked { reason: String },
    PermissionDenied,
    Unsupported,
    Rejected { code: String },
    Prerequisite { reason: String },
    Stale,
    Unknown,
    Unavailable,
}

impl ActionAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub fn explanation(&self) -> String {
        match self {
            Self::Available => "Available".into(),
            Self::Busy { operations } => format!(
                "The host reports {} running operation(s): {}",
                operations.len(),
                operations
                    .iter()
                    .map(|op| op.operation.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Blocked { reason } => format!("Administratively blocked: {reason}"),
            Self::PermissionDenied => "Your account is not permitted to perform this action".into(),
            Self::Unsupported => "This action is not supported by this host".into(),
            Self::Rejected { code } => format!("The host rejected this action: {code}"),
            Self::Prerequisite { reason } => reason.clone(),
            Self::Stale => {
                "Reconnect or wait for current inventory before using this action".into()
            }
            Self::Unknown => "Action availability was not provided or could not be decoded".into(),
            Self::Unavailable => "The host does not currently allow this action".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VmDetails {
    #[serde(default)]
    pub domain_type: Option<String>,
    #[serde(default)]
    pub cores_per_socket: Option<String>,
    #[serde(default)]
    pub topology_known: bool,
    #[serde(default)]
    pub cpu_weight: Option<String>,
    #[serde(default)]
    pub cpu_weight_known: bool,
    #[serde(default)]
    pub shadow_multiplier: Option<String>,
    #[serde(default)]
    pub is_default_template: Option<bool>,
    pub allowed_operations: Option<Vec<String>>,
    pub current_operations: Vec<RunningOperation>,
    pub blocked_operations: BTreeMap<String, String>,
    pub vcpus_max: Option<u64>,
    pub vcpus_at_startup: Option<u64>,
    pub memory_static_min: Option<u64>,
    pub memory_static_max: Option<u64>,
    pub memory_dynamic_min: Option<u64>,
    pub memory_dynamic_max: Option<u64>,
    pub memory_target: Option<u64>,
    pub boot_order: Option<String>,
    pub boot_policy: Option<String>,
    pub firmware: Option<String>,
    pub affinity: Option<HostId>,
    #[serde(default)]
    pub affinity_known: bool,
    pub guest_metrics: Option<GuestMetricsId>,
    pub metrics: Option<VmMetricsId>,
    pub suspend_vdi: Option<VdiId>,
    pub appliance: Option<ApplianceId>,
    #[serde(default)]
    pub appliance_known: bool,
    #[serde(default)]
    pub appliance_order: Option<crate::ApplianceOrder>,
    pub snapshot_policy: Option<SnapshotPolicyId>,
    pub tags: Vec<String>,
    pub ha_restart_priority: Option<String>,
    pub auto_power_on: Option<bool>,
}

impl VmDetails {
    pub fn power_availability(
        &self,
        action: crate::PowerAction,
        current: bool,
    ) -> ActionAvailability {
        if !current {
            return ActionAvailability::Stale;
        }
        if let Some(reason) = self.blocked_operations.get(action.wire()) {
            return ActionAvailability::Blocked {
                reason: reason.clone(),
            };
        }
        let Some(allowed) = &self.allowed_operations else {
            return ActionAvailability::Unknown;
        };
        if allowed.iter().any(|operation| operation == action.wire()) {
            return ActionAvailability::Available;
        }
        if !self.current_operations.is_empty() {
            return ActionAvailability::Busy {
                operations: self.current_operations.clone(),
            };
        }
        ActionAvailability::Unavailable
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostDetails {
    #[serde(default)]
    pub allowed_operations: Option<Vec<String>>,
    #[serde(default)]
    pub current_operations: Option<Vec<RunningOperation>>,
    #[serde(default)]
    pub maintenance_mode: Option<bool>,
    #[serde(default)]
    pub boot_time: Option<String>,
    #[serde(default)]
    pub agent_start_time: Option<String>,
    #[serde(default)]
    pub control_domain: Option<VmId>,
    #[serde(default)]
    pub power_on_mode: Option<String>,
    #[serde(default)]
    pub power_on_config_empty: Option<bool>,
    #[serde(default)]
    pub syslog_destination: Option<String>,
    #[serde(default)]
    pub syslog_known: bool,
    #[serde(default)]
    pub multipathing: Option<bool>,
    pub description: String,
    pub hostname: String,
    pub metrics: Option<HostMetricsId>,
    pub software_version: BTreeMap<String, String>,
    pub cpu_info: BTreeMap<String, String>,
    pub capabilities: Vec<String>,
    pub tags: Vec<String>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageDetails {
    pub id: SrId,
    pub description: String,
    pub content_type: String,
    pub shared: Option<bool>,
    pub virtual_allocation: Option<u64>,
    pub allowed_operations: Option<Vec<String>>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkDetails {
    #[serde(default)]
    pub automatic: Option<String>,
    #[serde(default)]
    pub automatic_known: bool,
    pub id: NetworkId,
    pub description: String,
    pub mtu: Option<u64>,
    pub managed: Option<bool>,
    pub default_locking_mode: Option<String>,
    pub allowed_operations: Option<Vec<String>>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VirtualDisk {
    pub id: VdiId,
    pub name: String,
    pub description: String,
    pub sr: Option<SrId>,
    pub virtual_size: Option<u64>,
    pub physical_utilisation: Option<u64>,
    pub kind: String,
    pub read_only: Option<bool>,
    pub sharable: Option<bool>,
    pub managed: Option<bool>,
    pub missing: Option<bool>,
    pub is_snapshot: Option<bool>,
    pub snapshot_of: Option<VdiId>,
    pub cbt_enabled: Option<bool>,
    pub allowed_operations: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VirtualBlockDevice {
    #[serde(default)]
    pub qos: Option<crate::DeviceQos>,
    #[serde(default)]
    pub qos_supported_algorithms: Option<Vec<String>>,
    pub id: VbdId,
    pub vm: Option<VmId>,
    pub snapshot: Option<SnapshotId>,
    pub vdi: Option<VdiId>,
    pub device: String,
    pub userdevice: String,
    pub kind: String,
    pub mode: String,
    pub currently_attached: Option<bool>,
    pub bootable: Option<bool>,
    pub empty: Option<bool>,
    #[serde(default)]
    pub owned: bool,
    pub allowed_operations: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VirtualInterface {
    #[serde(default)]
    pub qos_known: bool,
    #[serde(default)]
    pub qos_supported_algorithms: Option<Vec<String>>,
    pub id: VifId,
    pub vm: Option<VmId>,
    pub network: Option<NetworkId>,
    pub device: String,
    pub mac: String,
    pub mtu: Option<u64>,
    pub currently_attached: Option<bool>,
    pub qos_algorithm: String,
    pub qos_parameters: BTreeMap<String, String>,
    pub allowed_operations: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PhysicalInterface {
    pub id: PifId,
    pub host: Option<HostId>,
    pub network: Option<NetworkId>,
    pub device: String,
    pub mac: String,
    pub mtu: Option<u64>,
    pub vlan: Option<i64>,
    pub management: Option<bool>,
    pub physical: Option<bool>,
    pub currently_attached: Option<bool>,
    pub ip_configuration_mode: String,
    pub ip: String,
    pub netmask: String,
    pub gateway: String,
    pub dns: String,
    pub ipv6: Vec<String>,
    pub ipv6_configuration_mode: String,
    pub ipv6_gateway: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoragePathCount {
    pub device: String,
    pub active: u32,
    pub maximum: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageAttachment {
    pub id: PbdId,
    #[serde(default)]
    pub location: Option<crate::StorageLocation>,
    #[serde(default)]
    pub requires_credentials: bool,
    #[serde(default)]
    pub multipathed: Option<bool>,
    #[serde(default)]
    pub paths: Vec<StoragePathCount>,
    pub host: Option<HostId>,
    pub sr: Option<SrId>,
    pub currently_attached: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkBond {
    #[serde(default)]
    pub primary_slave: Option<PifId>,
    #[serde(default)]
    pub hashing_algorithm: Option<String>,
    #[serde(default)]
    pub hashing_algorithm_known: bool,
    pub id: BondId,
    pub master: Option<PifId>,
    pub slaves: Vec<PifId>,
    pub mode: String,
    pub links_up: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vlan {
    pub id: VlanId,
    pub tagged_pif: Option<PifId>,
    pub untagged_pif: Option<PifId>,
    pub tag: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuestMetrics {
    pub id: GuestMetricsId,
    pub os_version: BTreeMap<String, String>,
    pub tools_version: BTreeMap<String, String>,
    pub networks: BTreeMap<String, String>,
    pub live: Option<bool>,
    pub last_updated: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VmMetrics {
    pub id: VmMetricsId,
    pub memory_actual: Option<u64>,
    pub vcpus_number: Option<u64>,
    pub start_time: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostMetrics {
    pub id: HostMetricsId,
    pub live: Option<bool>,
    pub memory_total: Option<u64>,
    pub memory_free: Option<u64>,
    pub last_updated: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerTask {
    pub id: TaskId,
    pub name: String,
    pub description: String,
    pub status: String,
    pub progress: Option<f64>,
    pub created: Option<String>,
    pub finished: Option<String>,
    pub error_code: Option<String>,
    pub allowed_operations: Option<Vec<String>>,
    #[serde(default)]
    pub resident_host: Option<HostId>,
    #[serde(default)]
    pub parent: Option<TaskId>,
    #[serde(default)]
    pub subtasks: Vec<TaskId>,
    #[serde(default)]
    pub result_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerMessage {
    pub id: MessageId,
    pub name: String,
    pub priority: Option<i64>,
    pub object_class: String,
    pub object_uuid: Option<Uuid>,
    pub timestamp: Option<String>,
    pub body: String,
}
impl ServerMessage {
    /// Binds a dismissal review to the displayed content without journaling its body.
    pub fn review_digest(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let fields = (
            &self.id,
            &self.name,
            &self.priority,
            &self.object_class,
            &self.object_uuid,
            &self.timestamp,
            &self.body,
        );
        Sha256::digest(serde_json::to_vec(&fields).expect("message fields serialize")).into()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VmAppliance {
    #[serde(default)]
    pub vms_known: bool,
    pub id: ApplianceId,
    pub name: String,
    pub description: String,
    pub vms: Vec<VmId>,
    pub allowed_operations: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotPolicy {
    #[serde(default)]
    pub vms_known: bool,
    #[serde(default)]
    pub last_run_time: Option<String>,
    pub id: SnapshotPolicyId,
    pub name: String,
    pub description: String,
    pub enabled: Option<bool>,
    pub kind: String,
    pub frequency: String,
    pub retained_snapshots: Option<u64>,
    pub schedule: BTreeMap<String, String>,
    pub vms: Vec<VmId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageManager {
    pub id: StorageManagerId,
    pub kind: String,
    pub name: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolDetails {
    #[serde(default)]
    pub administration: crate::PoolAdministration,
    #[serde(default)]
    pub auto_poweron: Option<bool>,
    #[serde(default)]
    pub ha_enabled: Option<bool>,
    #[serde(default)]
    pub igmp_snooping_enabled: Option<bool>,
    pub id: PoolId,
    pub description: String,
    pub default_sr: Option<SrId>,
    pub default_sr_known: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InventoryDetails {
    #[serde(default)]
    pub organization: crate::OrganizationInventory,
    #[serde(default)]
    pub pool: Option<PoolDetails>,
    pub classes: BTreeMap<String, ClassAvailability>,
    pub vms: BTreeMap<VmId, VmDetails>,
    pub hosts: BTreeMap<HostId, HostDetails>,
    pub storage: BTreeMap<SrId, StorageDetails>,
    pub networks: BTreeMap<NetworkId, NetworkDetails>,
    pub vdis: BTreeMap<VdiId, VirtualDisk>,
    pub vbds: BTreeMap<VbdId, VirtualBlockDevice>,
    pub vifs: BTreeMap<VifId, VirtualInterface>,
    pub pifs: BTreeMap<PifId, PhysicalInterface>,
    pub pbds: BTreeMap<PbdId, StorageAttachment>,
    pub bonds: BTreeMap<BondId, NetworkBond>,
    pub vlans: BTreeMap<VlanId, Vlan>,
    pub guest_metrics: BTreeMap<GuestMetricsId, GuestMetrics>,
    pub vm_metrics: BTreeMap<VmMetricsId, VmMetrics>,
    pub host_metrics: BTreeMap<HostMetricsId, HostMetrics>,
    pub tasks: BTreeMap<TaskId, ServerTask>,
    pub messages: BTreeMap<MessageId, ServerMessage>,
    pub appliances: BTreeMap<ApplianceId, VmAppliance>,
    pub snapshot_policies: BTreeMap<SnapshotPolicyId, SnapshotPolicy>,
    pub storage_managers: BTreeMap<StorageManagerId, StorageManager>,
    pub vm_disks: BTreeMap<VmId, Vec<VbdId>>,
    pub snapshot_disks: BTreeMap<SnapshotId, Vec<VbdId>>,
    pub vm_interfaces: BTreeMap<VmId, Vec<VifId>>,
    pub sr_disks: BTreeMap<SrId, Vec<VdiId>>,
    pub vdi_attachments: BTreeMap<VdiId, Vec<VbdId>>,
    pub host_interfaces: BTreeMap<HostId, Vec<PifId>>,
    pub network_interfaces: BTreeMap<NetworkId, Vec<VifId>>,
    pub network_uplinks: BTreeMap<NetworkId, Vec<PifId>>,
    pub sr_attachments: BTreeMap<SrId, Vec<PbdId>>,
}
