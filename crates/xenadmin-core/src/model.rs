use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! identity {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn parse(value: &str) -> Result<Self, uuid::Error> {
                Uuid::parse_str(value).map(Self)
            }

            pub fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

identity!(PoolId);
identity!(HostId);
identity!(VmId);
identity!(SnapshotId);

#[path = "model_inventory.rs"]
mod inventory;
pub use inventory::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerState {
    Running,
    Halted,
    Paused,
    Suspended,
    Unknown(String),
}

impl fmt::Display for PowerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => f.write_str("Running"),
            Self::Halted => f.write_str("Halted"),
            Self::Paused => f.write_str("Paused"),
            Self::Suspended => f.write_str("Suspended"),
            Self::Unknown(value) => write!(f, "Unknown ({value})"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Host {
    pub id: HostId,
    pub name: String,
    pub address: String,
    pub enabled: bool,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vm {
    #[serde(default)]
    pub allowed_actions: Vec<crate::PowerAction>,
    #[serde(default)]
    pub can_snapshot: bool,
    pub id: VmId,
    pub name: String,
    pub description: String,
    pub power_state: PowerState,
    pub host: Option<HostId>,
    pub vcpus: u64,
    pub memory_bytes: u64,
    pub is_template: bool,
    pub is_control_domain: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotAction {
    Revert,
    Delete,
    Clone,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub vm: Option<VmId>,
    pub name: String,
    pub description: String,
    pub created_at: Option<String>,
    pub power_state: PowerState,
    pub allowed_actions: Vec<SnapshotAction>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(
    from = "BTreeMap<SnapshotId, Snapshot>",
    into = "BTreeMap<SnapshotId, Snapshot>"
)]
pub struct SnapshotInventory {
    by_id: BTreeMap<SnapshotId, Snapshot>,
    by_vm: BTreeMap<VmId, Vec<SnapshotId>>,
}
impl From<BTreeMap<SnapshotId, Snapshot>> for SnapshotInventory {
    fn from(records: BTreeMap<SnapshotId, Snapshot>) -> Self {
        let mut inventory = Self::default();
        for snapshot in records.into_values() {
            inventory.insert(snapshot);
        }
        inventory
    }
}
impl From<SnapshotInventory> for BTreeMap<SnapshotId, Snapshot> {
    fn from(inventory: SnapshotInventory) -> Self {
        inventory.by_id
    }
}
impl SnapshotInventory {
    pub fn insert(&mut self, snapshot: Snapshot) {
        if let Some(previous) = self.by_id.remove(&snapshot.id)
            && let Some(parent) = previous.vm
            && let Some(siblings) = self.by_vm.get_mut(&parent)
        {
            siblings.retain(|id| *id != previous.id);
            if siblings.is_empty() {
                self.by_vm.remove(&parent);
            }
        }
        if let Some(parent) = snapshot.vm {
            self.by_vm.entry(parent).or_default().push(snapshot.id);
        }
        self.by_id.insert(snapshot.id, snapshot);
    }
    pub fn get(&self, id: &SnapshotId) -> Option<&Snapshot> {
        self.by_id.get(id)
    }
    pub fn for_vm(&self, vm: VmId) -> impl Iterator<Item = &Snapshot> {
        self.by_vm
            .get(&vm)
            .into_iter()
            .flatten()
            .map(|id| &self.by_id[id])
    }
    pub fn iter(&self) -> impl Iterator<Item = &Snapshot> {
        self.by_id.values()
    }
    pub fn len(&self) -> usize {
        self.by_id.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Storage {
    pub uuid: Uuid,
    pub name: String,
    pub kind: String,
    /// Unknown or unreported capacity, including XAPI's -1 sentinel, is None.
    pub capacity_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Network {
    pub uuid: Uuid,
    pub name: String,
    pub bridge: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    pub pool_id: PoolId,
    pub pool_name: String,
    pub coordinator: Option<HostId>,
    pub hosts: BTreeMap<HostId, Host>,
    pub vms: BTreeMap<VmId, Vm>,
    #[serde(default)]
    pub snapshots: SnapshotInventory,
    pub storage: Vec<Storage>,
    pub networks: Vec<Network>,
    #[serde(default)]
    pub details: InventoryDetails,
    pub revision: u64,
}

impl Inventory {
    pub fn guests(&self) -> impl Iterator<Item = &Vm> {
        self.vms
            .values()
            .filter(|vm| !vm.is_control_domain && !vm.is_template)
    }
}
