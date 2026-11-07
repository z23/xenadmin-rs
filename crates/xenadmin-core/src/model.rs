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
    pub id: VmId,
    pub name: String,
    pub description: String,
    pub power_state: PowerState,
    pub host: Option<HostId>,
    pub vcpus: u64,
    pub memory_bytes: u64,
    pub is_template: bool,
    pub is_snapshot: bool,
    pub is_control_domain: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Storage {
    pub uuid: Uuid,
    pub name: String,
    pub kind: String,
    pub capacity_bytes: u64,
    pub used_bytes: u64,
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
    pub storage: Vec<Storage>,
    pub networks: Vec<Network>,
    pub revision: u64,
}

impl Inventory {
    pub fn guests(&self) -> impl Iterator<Item = &Vm> {
        self.vms
            .values()
            .filter(|vm| !vm.is_control_domain && !vm.is_template && !vm.is_snapshot)
    }
}
